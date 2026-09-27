//! Scores: a Trust score for every turn and an Agency Ops Score for every site (Trust Kernel, part 6).
//!
//! One engine, two subjects. Both are **derived** - from the event log for a turn, from the latest
//! health report for a site - so a score can always be recomputed and always comes with its reasons.
//! A number with no reasons would be the kind of claim principle P4 forbids; every point taken off or
//! given here is a sentence the window shows next to it.
//!
//! A turn that has not been verified is capped: "nobody checked" is not a high-trust state, however
//! small the change (the plan's UNPROVEN).

use serde_json::{json, Value};

use crate::sdcp::events::StoredEvent;
use crate::trust::policy::Policy;

/// What a turn did, read back from its events.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TurnFacts {
    pub files_changed: Vec<String>,
    pub protected_touched: Vec<String>,
    pub commands: usize,
    pub risky_commands: usize,
    pub violations: usize,
    pub checkpoint: bool,
    pub failed: bool,
    pub stopped: bool,
    /// `None` when Verify has not run for the turn.
    pub verify_pass: Option<bool>,
    pub checks_run: usize,
    pub review_verdict: Option<String>,
    pub secrets: usize,
    pub sast_high: usize,
    pub sast_other: usize,
}

/// The facts of one turn, from the log.
pub fn facts(events: &[StoredEvent], turn_id: &str, policy: &Policy, root: Option<&str>) -> TurnFacts {
    let mut facts = TurnFacts::default();

    for entry in events.iter().filter(|entry| entry.turn_id.as_deref() == Some(turn_id) || entry.event["turnId"] == turn_id) {
        let event = &entry.event;

        match event["type"].as_str().unwrap_or_default() {
            "ToolCallStarted" => {
                let target = event["target"].as_str().unwrap_or_default();

                match event["tool"].as_str() {
                    Some("edit") => {
                        if !target.is_empty() && !facts.files_changed.iter().any(|file| file == target) {
                            facts.files_changed.push(target.to_string());
                        }

                        if let Some(pattern) = policy.protected(target, root) {
                            if !facts.protected_touched.contains(&pattern) {
                                facts.protected_touched.push(pattern);
                            }
                        }
                    }
                    Some("run") => {
                        facts.commands += 1;

                        if crate::agent::gate::looks_dangerous(target) || policy.always_asks(target).is_some() {
                            facts.risky_commands += 1;
                        }
                    }
                    _ => {}
                }
            }
            "CheckpointSaved" => facts.checkpoint = true,
            "PolicyViolation" => facts.violations += 1,
            "ErrorRaised" => facts.failed = true,
            "BudgetStop" | "KillSwitch" => facts.stopped = true,
            "TurnCompleted" if event["summary"] == "Interrupted" || event["summary"] == "Force killed" => facts.stopped = true,
            "VerifyUpdated" if event["state"] == "done" => {
                facts.verify_pass = match event["verdict"].as_str() {
                    Some("PASS") => Some(true),
                    Some("FAIL") => Some(false),
                    Some(_) => None,
                    None => event["pass"].as_bool(),
                };
                facts.checks_run = event["checks"].as_array().map(|checks| checks.iter().filter(|check| check["status"] == "pass" || check["status"] == "fail").count()).unwrap_or(0);
                facts.review_verdict = event["review"]["verdict"].as_str().map(str::to_string);
                facts.secrets = event["scans"]["secrets"].as_array().map(Vec::len).unwrap_or(0);
                facts.sast_high = event["scans"]["sast"].as_array().map(|all| all.iter().filter(|finding| finding["severity"] == "high").count()).unwrap_or(0);
                facts.sast_other = event["scans"]["sast"].as_array().map(|all| all.iter().filter(|finding| finding["severity"] != "high").count()).unwrap_or(0);
            }
            _ => {}
        }
    }

    facts
}

fn take(score: &mut i64, reasons: &mut Vec<Value>, points: i64, text: String) {
    if points > 0 {
        *score -= points;
        reasons.push(reason(text, -points));
    }
}

fn reason(text: impl Into<String>, delta: i64) -> Value {
    json!({ "text": text.into(), "delta": delta })
}

/// The turn's score, 0-100, its level, and every reason with the points it moved.
pub fn turn(facts: &TurnFacts, max_files: usize) -> (i64, &'static str, Vec<Value>) {
    let mut score: i64 = 100;
    let mut reasons = Vec::new();

    if !facts.protected_touched.is_empty() {
        take(&mut score, &mut reasons, 30, format!("Changed a protected path ({})", facts.protected_touched.join(", ")));
    }

    if facts.violations > 0 {
        take(&mut score, &mut reasons, 20 * facts.violations.min(2) as i64, format!("{} policy violation(s)", facts.violations));
    }

    if facts.secrets > 0 {
        take(&mut score, &mut reasons, 35, format!("{} secret(s) added to the code", facts.secrets));
    }

    if facts.sast_high > 0 {
        take(&mut score, &mut reasons, 15 * facts.sast_high.min(3) as i64, format!("{} high-risk code pattern(s)", facts.sast_high));
    }

    if facts.sast_other > 0 {
        take(&mut score, &mut reasons, 5 * facts.sast_other.min(3) as i64, format!("{} lower-risk code pattern(s)", facts.sast_other));
    }

    if facts.risky_commands > 0 {
        take(&mut score, &mut reasons, 8 * facts.risky_commands.min(3) as i64, format!("{} command(s) that reach beyond the folder", facts.risky_commands));
    }

    let files = facts.files_changed.len();

    if max_files > 0 && files > max_files {
        take(&mut score, &mut reasons, 15, format!("{files} files changed, over the limit of {max_files}"));
    } else if files > 10 {
        take(&mut score, &mut reasons, 5, format!("A wide change: {files} files"));
    }

    if facts.failed {
        take(&mut score, &mut reasons, 15, "The turn ended with an error".to_string());
    }

    if files > 0 && !facts.checkpoint {
        take(&mut score, &mut reasons, 20, "No rollback point before the change".to_string());
    } else if facts.checkpoint {
        reasons.push(reason("A rollback point exists", 0));
    }

    match facts.verify_pass {
        Some(true) => reasons.push(reason(format!("Verified: {} check(s) passed{}", facts.checks_run, if facts.review_verdict.as_deref() == Some("pass") { ", and a second AI found no issues" } else { "" }), 0)),
        Some(false) => take(&mut score, &mut reasons, 25, "Verify did not pass".to_string()),
        None if files > 0 || facts.commands > 0 => {
            /* Unproven: nothing wrong was found because nothing was looked at. */
            let cap = 70;

            if score > cap {
                reasons.push(reason("Not verified yet: run Verify to prove it", cap - score));
                score = cap;
            } else {
                reasons.push(reason("Not verified yet", 0));
            }
        }
        None => reasons.push(reason("Read-only: nothing was changed", 0)),
    }

    if facts.review_verdict.as_deref() == Some("issues") {
        take(&mut score, &mut reasons, 10, "The reviewer found issues".to_string());
    }

    let score = score.clamp(0, 100);
    let level = if score >= 80 {
        "high"
    } else if score >= 50 {
        "medium"
    } else {
        "low"
    };

    (score, level, reasons)
}

/// A site's Agency Ops Score from its latest health report, its deploys and its month.
pub fn ops(report: &Value, last_deploy_state: Option<&str>, violations_30d: usize) -> (i64, &'static str, Vec<Value>) {
    let mut score: i64 = 100;
    let mut reasons = Vec::new();

    match report["http"]["ok"].as_bool() {
        Some(false) => take(&mut score, &mut reasons, 40, format!("The site is not answering as expected ({})", report["http"]["detail"].as_str().unwrap_or("down"))),
        Some(true) => reasons.push(reason(format!("Up · {} ms", report["http"]["ms"].as_u64().unwrap_or(0)), 0)),
        None => take(&mut score, &mut reasons, 10, "Not checked yet".to_string()),
    }

    if let Some(ms) = report["http"]["ms"].as_u64() {
        if ms > 3000 {
            take(&mut score, &mut reasons, 10, format!("Slow: {ms} ms"));
        }
    }

    if let Some(days) = report["ssl"]["days"].as_i64() {
        if days < 0 {
            take(&mut score, &mut reasons, 30, "The SSL certificate has expired".to_string());
        } else if days < 7 {
            take(&mut score, &mut reasons, 20, format!("SSL expires in {days} days"));
        } else if days < 21 {
            take(&mut score, &mut reasons, 8, format!("SSL expires in {days} days"));
        }
    }

    if let Some(percent) = report["disk"]["percent"].as_i64() {
        if percent >= 95 {
            take(&mut score, &mut reasons, 25, format!("Disk {percent}% full"));
        } else if percent >= 85 {
            take(&mut score, &mut reasons, 10, format!("Disk {percent}% full"));
        }
    }

    match report["backup"]["ageHours"].as_f64() {
        Some(hours) if hours > 24.0 * 7.0 => take(&mut score, &mut reasons, 20, format!("Newest backup is {:.0} days old", hours / 24.0)),
        Some(hours) if hours > 48.0 => take(&mut score, &mut reasons, 8, format!("Newest backup is {:.0} hours old", hours)),
        Some(_) => reasons.push(reason("A recent backup exists", 0)),
        None if report["backup"]["configured"] == false => take(&mut score, &mut reasons, 15, "No backup folder is set for this site".to_string()),
        None => {}
    }

    if let Some(errors) = report["errors"]["count"].as_u64() {
        if errors > 50 {
            take(&mut score, &mut reasons, 10, format!("{errors} error lines in the log recently"));
        }
    }

    if matches!(last_deploy_state, Some("failed" | "rollback_failed")) {
        take(&mut score, &mut reasons, 15, "The last deploy failed".to_string());
    }

    if violations_30d > 0 {
        take(&mut score, &mut reasons, 5 * violations_30d.min(4) as i64, format!("{violations_30d} policy violation(s) in 30 days"));
    }

    let score = score.clamp(0, 100);
    let level = if score >= 80 {
        "healthy"
    } else if score >= 50 {
        "watch"
    } else {
        "at-risk"
    };

    (score, level, reasons)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(turn: &str, event: Value) -> StoredEvent {
        StoredEvent { seq: 1, ts: String::new(), session_id: Some("s1".into()), turn_id: Some(turn.into()), event }
    }

    #[test]
    fn an_unverified_change_is_capped_and_a_verified_one_is_not() {
        let policy = Policy::default();
        let mut log = vec![
            event("t1", json!({ "type": "CheckpointSaved" })),
            event("t1", json!({ "type": "ToolCallStarted", "tool": "edit", "target": "src/a.ts" })),
        ];
        let (score, level, reasons) = turn(&facts(&log, "t1", &policy, None), 25);

        assert_eq!(score, 70);
        assert_eq!(level, "medium");
        assert!(reasons.iter().any(|reason| reason["text"].as_str().unwrap().contains("Not verified")));

        log.push(event("t1", json!({ "type": "VerifyUpdated", "state": "done", "pass": true, "checks": [{"status": "pass"}], "review": { "verdict": "pass" }, "scans": { "secrets": [], "sast": [] } })));

        let (score, level, _) = turn(&facts(&log, "t1", &policy, None), 25);

        assert_eq!((score, level), (100, "high"));
    }

    #[test]
    fn protected_paths_secrets_and_no_rollback_point_cost_the_most() {
        let policy = Policy::default();
        let log = vec![
            event("t2", json!({ "type": "ToolCallStarted", "tool": "edit", "target": "wp-config.php" })),
            event("t2", json!({ "type": "VerifyUpdated", "state": "done", "pass": false, "checks": [], "review": {}, "scans": { "secrets": [{}], "sast": [] } })),
        ];
        let (score, level, _) = turn(&facts(&log, "t2", &policy, None), 25);

        assert_eq!(level, "low");
        assert!(score < 20, "{score}");
    }

    #[test]
    fn a_read_only_turn_is_fully_trusted() {
        let (score, _, reasons) = turn(&TurnFacts::default(), 25);

        assert_eq!(score, 100);
        assert!(reasons[0]["text"].as_str().unwrap().contains("Read-only"));
    }

    #[test]
    fn a_site_that_is_down_with_an_old_backup_is_at_risk() {
        let report = json!({
            "http": { "ok": false, "detail": "HTTP 502" },
            "ssl": { "days": 5 },
            "disk": { "percent": 91 },
            "backup": { "ageHours": 400.0 },
        });
        let (score, level, reasons) = ops(&report, Some("failed"), 1);

        assert_eq!(level, "at-risk");
        assert!(score < 20);
        assert!(reasons.len() >= 5);

        let healthy = json!({ "http": { "ok": true, "ms": 180 }, "ssl": { "days": 80 }, "disk": { "percent": 40 }, "backup": { "ageHours": 5.0 } });

        assert_eq!(ops(&healthy, Some("success"), 0).1, "healthy");
    }
}
