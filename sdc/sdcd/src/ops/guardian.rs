//! **The Night Guardian** (2.0 in the plan): a watch that never sleeps, with one hand tied on purpose.
//!
//! When a site that is watched goes down and stays down for two checks in a row, the Guardian:
//!
//! 1. **may roll back on its own - and only roll back** - to the version before the newest deploy, when
//!    that deploy is recent and the site (or the folder's policy, `[guardian] auto_rollback = true`) allows
//!    it. A rollback restores a state the site was already in, it is backed up first, and it is undoable
//!    (P5). The Guardian never ships new code.
//! 2. **prepares a fix and waits**: an approval item with the diagnosis (the health report, the error
//!    count, the last deploy) and a ready prompt. Nothing reaches production until a person approves -
//!    then it is an ordinary chat, with its checkpoints, its Verify and its Safe Deploy.
//! 3. **tells the person**, in their language, what it saw and what it did.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

use super::Site;
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;
use crate::DaemonState;

/// Consecutive failed checks before the Guardian acts - one bad minute is not an outage.
pub const DOWN_CHECKS: u32 = 2;
/// A deploy older than this is not "the one that broke it"; the Guardian then only alerts.
pub const RECENT_HOURS: i64 = 48;

fn failures() -> &'static Mutex<HashMap<String, u32>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

    FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Deploys the Guardian has already acted on, so an outage is handled once, not every minute.
fn handled() -> &'static Mutex<HashSet<String>> {
    static HANDLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

    HANDLED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// What the Guardian would do for a site in this state - pure, so the rules are tested without a server.
pub fn decide(down_checks: u32, enabled: bool, auto_rollback: bool, last_deploy: Option<&Value>, already: bool) -> &'static str {
    if !enabled || down_checks < DOWN_CHECKS || already {
        return "none";
    }

    let recent_success = last_deploy.is_some_and(|deploy| {
        deploy["state"] == "success"
            && deploy["kind"] != "rollback"
            && deploy["backup"]["files"]["path"].as_str().is_some_and(|path| !path.is_empty())
            && deploy["startedAt"]
                .as_str()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| (chrono::Utc::now() - at.with_timezone(&chrono::Utc)).num_hours() <= RECENT_HOURS)
    });

    if recent_success && auto_rollback {
        "rollback"
    } else {
        "alert"
    }
}

/// Reads one health report and acts when the rules say so.
pub fn observe(state: &Arc<DaemonState>, out: &Arc<dyn Notifier>, site: &Site, remote: Option<crate::ssh::Ssh>, report: &Value) {
    let down = report["http"]["ok"].as_bool() == Some(false);
    let count = {
        let mut map = failures().lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = map.entry(site.id.clone()).or_insert(0);

        *entry = if down { *entry + 1 } else { 0 };
        *entry
    };

    if !down {
        return;
    }

    let enabled = site.config["guardian"]["enabled"].as_bool().unwrap_or(false);
    let policy = crate::trust::policy::Policy::load(Some(&site.root), remote.as_ref());
    let auto_rollback = site.config["guardian"]["autoRollback"].as_bool().unwrap_or(false) || policy.auto_rollback;
    let last = state.store.deploys(Some(&site.id), 1).ok().and_then(|rows| rows.into_iter().next());
    let key = last.as_ref().and_then(|deploy| deploy["id"].as_str()).map(|id| format!("{}:{id}", site.id)).unwrap_or_else(|| format!("{}:none", site.id));
    let already = handled().lock().map(|set| set.contains(&key)).unwrap_or(false);

    match decide(count, enabled, auto_rollback, last.as_ref(), already) {
        "none" => {}
        decision => {
            if let Ok(mut set) = handled().lock() {
                set.insert(key);
            }

            let lang = state.store.setting("ui.language").ok().flatten().unwrap_or_else(|| "en".into());
            let detail = report["http"]["detail"].as_str().unwrap_or("down").to_string();

            if decision == "rollback" {
                if let Some(deploy) = &last {
                    let result = super::deploy::rollback(state, out, site, remote.clone(), deploy, "guardian");
                    let sentence = if lang.starts_with("bn") {
                        format!("{} বন্ধ ছিল ({detail}), তাই Night Guardian শেষ ভালো version-এ ফিরিয়ে দিয়েছে। নতুন কোনো কোড চালু করা হয়নি।", site.name)
                    } else {
                        format!("{} was down ({detail}), so the Night Guardian rolled it back to the last good version. No new code was deployed.", site.name)
                    };

                    out.push(event::guardian_action(&site.id, "rolled_back", &sentence, result["deployId"].as_str()), None, None);
                }
            } else {
                let sentence = if lang.starts_with("bn") {
                    format!("{} বন্ধ ({detail})। Night Guardian নিজে কিছু বদলায়নি; একটা fix-এর প্রস্তাব আপনার অনুমোদনের অপেক্ষায়।", site.name)
                } else {
                    format!("{} is down ({detail}). The Night Guardian changed nothing on its own; a fix is prepared and waits for your approval.", site.name)
                };

                out.push(event::guardian_action(&site.id, "alerted", &sentence, None), None, None);
            }

            prepare_fix(state, out, site, report, last.as_ref());
        }
    }
}

/// The fix that waits: an approval item carrying the diagnosis and the prompt a chat would start with.
fn prepare_fix(state: &Arc<DaemonState>, out: &Arc<dyn Notifier>, site: &Site, report: &Value, last: Option<&Value>) {
    let id = format!("appr-{}", uuid::Uuid::new_v4().simple());
    let prompt = format!(
        "The site {name} ({url}) went down: {detail}. The last deploy was {deploy}. Error lines in the log recently: {errors}.\n\
         Find the cause and fix it in the site's folder ({root}). Do not deploy: prepare the change, run the checks, and stop - a person will review it and ship it with Safe Deploy.",
        name = site.name,
        url = site.health_url(),
        detail = report["http"]["detail"].as_str().unwrap_or("not answering"),
        deploy = last.map(|deploy| format!("{} ({})", deploy["id"].as_str().unwrap_or("?"), deploy["state"].as_str().unwrap_or("?"))).unwrap_or_else(|| "none on record".into()),
        errors = report["errors"]["count"].as_u64().map(|count| count.to_string()).unwrap_or_else(|| "not watched".into()),
        root = site.root,
    );
    let note = json!({ "siteId": site.id, "prompt": prompt, "report": report }).to_string();

    if state.store.save_approval(&id, &format!("site:{}", site.id), "guardian-fix", "guardian", &note, None).is_ok() {
        if let Ok(Some(approval)) = state.store.approvals(Some(&format!("site:{}", site.id))).map(|rows| rows.into_iter().find(|row| row["id"] == id.as_str())) {
            out.push(event::approval_recorded(approval), None, None);
        }

        out.push(event::guardian_action(&site.id, "fix_prepared", "A fix is prepared and waits for approval.", None), None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deploy(state: &str, hours_ago: i64) -> Value {
        json!({
            "id": "dep-1",
            "kind": "production",
            "state": state,
            "startedAt": (chrono::Utc::now() - chrono::Duration::hours(hours_ago)).to_rfc3339(),
            "backup": { "files": { "path": "/b/x.tar.gz" } },
        })
    }

    #[test]
    fn it_rolls_back_only_a_recent_successful_deploy_and_only_when_allowed() {
        assert_eq!(decide(2, true, true, Some(&deploy("success", 3)), false), "rollback");
        assert_eq!(decide(2, true, false, Some(&deploy("success", 3)), false), "alert", "auto-rollback is off: alert only");
        assert_eq!(decide(2, true, true, Some(&deploy("success", 200)), false), "alert", "an old deploy is not the cause");
        assert_eq!(decide(2, true, true, Some(&deploy("failed", 1)), false), "alert");
        assert_eq!(decide(1, true, true, Some(&deploy("success", 1)), false), "none", "one bad check is not an outage");
        assert_eq!(decide(3, false, true, Some(&deploy("success", 1)), false), "none", "a site that is not guarded");
        assert_eq!(decide(3, true, true, Some(&deploy("success", 1)), true), "none", "handled once");
    }
}
