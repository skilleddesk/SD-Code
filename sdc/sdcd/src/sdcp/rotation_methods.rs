//! Settings → Agents (Phase 3): the Rotation Chain editor's two calls, and the manual **Escalate** button.
//!
//! * `rotation.get` / `rotation.set` read and write `rotation.toml` in the data directory (`data::rotation`; validated, atomic,
//!   a backup beside it) and add what a person needs to choose by: whether a model is local, and a cost hint from the catalogue.
//! * `rotation.escalate` - when a local step failed, the person may send **that one step** to the next model of their chain. It asks
//!   in two calls: `confirm: false` returns exactly what would be sent (a minimal, redacted capsule) and what it may cost; only
//!   `confirm: true` sends, after writing what is sent to `.sdc/audit/sent.jsonl`. The chat's own model is never changed, so the next
//!   step is the local model again. A `local-only` project cannot escalate at all.
//!
//! The model-selection contract (`guard::modelgate`) is why this is manual: a Rotation Chain row is only ever called on the person's click.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::*;
use crate::data::rotation::{self, Global, Row, Rotation};
use crate::rotation::policy::RelayPolicy;

/// A typical step, for a price hint only: 6,000 tokens in, 1,000 out.
const STEP_IN: f64 = 6_000.0;
const STEP_OUT: f64 = 1_000.0;
/// What the capsule may carry: at most this many files, this many characters each and in all.
const CAPSULE_FILES: usize = 3;
const FILE_CHARS: usize = 6_000;
const CAPSULE_CHARS: usize = 20_000;

/// How many escalations the current failed step has had, per chat: the first escalation goes to the first entry, a second one (the
/// first also failed) to the second, and so on. A regular local turn starts the count again.
fn counts() -> &'static Mutex<HashMap<String, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

    COUNTS.get_or_init(Default::default)
}

/// A regular (local) turn began in this chat: the next failure starts from the first chain entry again.
pub(super) fn note_regular_turn(session_id: &str) {
    if let Ok(mut map) = counts().lock() {
        map.remove(session_id);
    }
}

/// The price hint of one `provider:model`: dollars per million tokens in and out, and for a typical step. `known: false` for a model
/// with no listed price (never a made-up zero); a local model is free.
pub fn cost_hint(model: &str) -> Value {
    let (provider, name) = model.split_once(':').unwrap_or(("", model));

    match crate::trust::cost::price_of(Some(provider).filter(|provider| !provider.is_empty()), name) {
        Some((input, output)) => json!({ "known": true, "inPer1M": input, "outPer1M": output, "perStep": (input * STEP_IN + output * STEP_OUT) / 1_000_000.0 }),
        None => json!({ "known": false }),
    }
}

fn row_json(index: usize, row: &Row) -> Value {
    json!({
        "index": index,
        "model": row.model,
        "enabled": row.enabled,
        "local": rotation::is_local(&row.model),
        "allowedSteps": row.allowed_steps,
        "maxCostPerStep": row.max_cost_per_step,
        "maxRepairRetries": row.max_repair_retries,
        "cost": cost_hint(&row.model),
    })
}

fn view(rotation: &Rotation, warning: Option<String>, home: &std::path::Path) -> Value {
    json!({
        "rows": rotation.chain.iter().enumerate().map(|(index, row)| row_json(index, row)).collect::<Vec<_>>(),
        "global": {
            "dailyUsd": rotation.global.daily_usd,
            "monthlyUsd": rotation.global.monthly_usd,
            "askBeforeCloud": rotation.global.ask_before_cloud,
            "offloadLimit": rotation.global.offload_limit,
        },
        "relay": relay_json(&rotation.relay),
        "warning": warning,
        "path": rotation::path(home).display().to_string(),
    })
}

/// `7d`, `24h`, `30m` or an RFC 3339 time, as the RFC 3339 time it means (now minus the period).
pub fn parse_since(text: &str, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    let text = text.trim();

    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(time.with_timezone(&chrono::Utc).to_rfc3339());
    }

    let (number, unit) = text.split_at(text.len().checked_sub(1)?);
    let count: i64 = number.parse().ok().filter(|count| *count > 0)?;
    let span = match unit {
        "d" => chrono::Duration::days(count),
        "h" => chrono::Duration::hours(count),
        "m" => chrono::Duration::minutes(count),
        _ => return None,
    };

    Some((now - span).to_rfc3339())
}

fn relay_json(relay: &RelayPolicy) -> Value {
    json!({
        "auto": relay.auto,
        "localAttempts": relay.local_attempts,
        "localTtftS": relay.local_ttft_s,
        "maxCloudStepsPerTurn": relay.max_cloud_steps_per_turn,
        "maxUsdPerTurn": relay.max_usd_per_turn,
        "showChip": relay.show_chip,
        "preroute": relay.preroute,
        "learn": relay.learn,
        "callTimeoutS": relay.call_timeout_s,
    })
}

/// The relay part of a `rotation.set`: every field the call names replaces the stored one, every field it leaves out stays as it was.
fn parse_relay(value: &Value, stored: &RelayPolicy) -> RelayPolicy {
    RelayPolicy {
        auto: value["auto"].as_bool().unwrap_or(stored.auto),
        local_attempts: value["localAttempts"].as_u64().map_or(stored.local_attempts, |n| n.min(u32::MAX as u64) as u32),
        local_ttft_s: value["localTtftS"].as_u64().map_or(stored.local_ttft_s, |n| n.min(u32::MAX as u64) as u32),
        max_cloud_steps_per_turn: value["maxCloudStepsPerTurn"].as_u64().map_or(stored.max_cloud_steps_per_turn, |n| n.min(u32::MAX as u64) as u32),
        max_usd_per_turn: value["maxUsdPerTurn"].as_f64().unwrap_or(stored.max_usd_per_turn),
        show_chip: value["showChip"].as_bool().unwrap_or(stored.show_chip),
        preroute: value["preroute"].as_bool().unwrap_or(stored.preroute),
        learn: value["learn"].as_bool().unwrap_or(stored.learn),
        call_timeout_s: value["callTimeoutS"].as_u64().map_or(stored.call_timeout_s, |n| n.min(u32::MAX as u64) as u32),
    }
}

/// The rotation a `rotation.set` call describes. `stored_relay` is what the file holds now: a call that does not mention `relay` leaves it alone.
fn parse(envelope: &Envelope, stored_relay: &RelayPolicy) -> Result<Rotation, ErrorObject> {
    let rows = envelope.params.get("rows").and_then(Value::as_array).ok_or_else(|| ErrorObject::bad_request("`rows` is required"))?;
    let mut chain = Vec::new();

    for row in rows {
        chain.push(Row {
            model: row["model"].as_str().unwrap_or_default().trim().to_string(),
            enabled: row["enabled"].as_bool().unwrap_or(true),
            allowed_steps: row["allowedSteps"].as_array().map(|steps| steps.iter().filter_map(|step| step.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            max_cost_per_step: row["maxCostPerStep"].as_f64().unwrap_or(0.0),
            max_repair_retries: row["maxRepairRetries"].as_u64().unwrap_or(0) as u32,
        });
    }

    let global = envelope.params.get("global").cloned().unwrap_or(Value::Null);
    let defaults = Global::default();

    Ok(Rotation {
        chain,
        global: Global {
            daily_usd: global["dailyUsd"].as_f64().unwrap_or(defaults.daily_usd),
            monthly_usd: global["monthlyUsd"].as_f64().unwrap_or(defaults.monthly_usd),
            ask_before_cloud: global["askBeforeCloud"].as_bool().unwrap_or(defaults.ask_before_cloud),
            offload_limit: global["offloadLimit"].as_u64().unwrap_or(0) as u32,
        },
        relay: parse_relay(envelope.params.get("relay").unwrap_or(&Value::Null), stored_relay),
    })
}

/// What escalation sends: the goal, the exact error, and the few files that matter - each passed through the secret guard and the
/// redaction vault first. Answers the text and the rules that fired.
pub fn build_capsule(session_id: &str, goal: &str, error: &str, files: &[(String, String)]) -> (String, Vec<String>) {
    let mut fired = Vec::new();
    let mut clean = |label: &str, text: &str| -> String {
        let found = crate::trust::secretguard::scan("", text);
        let masked = if found.is_empty() { text.to_string() } else { crate::trust::secretguard::mask(text, &found) };
        let (scrubbed, count) = crate::v3::redact::scrub(session_id, &masked);

        if !found.is_empty() || count > 0 {
            fired.push(format!("{label}:{}", found.len() + count));
        }

        scrubbed
    };
    let mut text = format!("A step on a local model failed and the person asked you to take over this one step.\n\nGoal:\n{}\n\nThe exact error:\n{}\n", clean("goal", goal.trim()), clean("error", error.trim()));

    for (path, content) in files.iter().take(CAPSULE_FILES) {
        let body: String = content.chars().take(FILE_CHARS).collect();

        text.push_str(&format!("\nRelevant file {path}:\n```\n{}\n```\n", clean(path, &body)));
    }

    text.push_str("\nFix this step only. Do not touch unrelated files.\n");

    (text.chars().take(CAPSULE_CHARS).collect(), fired)
}

impl Daemon {
    pub(super) fn rotation_get(&self) -> Result<Value, ErrorObject> {
        let home = crate::data::home_or_error().map_err(ErrorObject::internal)?;
        let (rotation, warning) = rotation::load(&home);

        Ok(view(&rotation, warning, &home))
    }

    pub(super) fn rotation_set(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let home = crate::data::home_or_error().map_err(ErrorObject::internal)?;
        let (stored, _) = rotation::load(&home);
        let wanted = parse(envelope, &stored.relay)?;

        rotation::save(&home, &wanted).map_err(ErrorObject::bad_request)?;

        let (saved, warning) = rotation::load(&home);

        Ok(view(&saved, warning, &home))
    }

    /// `relay.report` (M1-7): how much of the work the local model did and how much went to the chain, from the `RelaySummary` events of the log.
    /// `sessionId` narrows it to one chat, `since` (`7d`, `24h`, or an RFC 3339 time) to a period.
    pub(super) fn relay_report(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let since = match envelope.opt_str("since") {
            Some(text) => Some(parse_since(&text, chrono::Utc::now()).ok_or_else(|| ErrorObject::bad_request("`since` is a period like 7d or 24h, or an RFC 3339 time"))?),
            None => None,
        };
        let summaries = self.store().events_of_type("RelaySummary", since.as_deref(), envelope.opt_str("sessionId").as_deref()).map_err(ErrorObject::internal)?;
        let values: Vec<Value> = summaries.into_iter().map(|stored| stored.event).collect();

        Ok(crate::rotation::ledger::report(&values))
    }

    /// The two-step Escalate (see the module's note).
    pub(super) fn rotation_escalate(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let session_id = envelope.require_str("sessionId")?;
        let confirm = envelope.opt_bool("confirm");
        let root = self.store().session_project_root(&session_id).map_err(ErrorObject::internal)?;
        let policy = crate::trust::policy::Policy::load(root.as_deref(), None);

        if policy.privacy_local() {
            return Err(ErrorObject::permission_denied("This project is local-only (.sdc/policy.toml): nothing may leave this computer, so a step cannot be escalated."));
        }

        let home = crate::data::home_or_error().map_err(ErrorObject::internal)?;
        let (rotation, _) = rotation::load(&home);
        let rows: Vec<&Row> = rotation.usable().into_iter().filter(|row| !rotation::is_local(&row.model)).collect();
        let done = counts().lock().map(|map| map.get(&session_id).copied().unwrap_or(0)).unwrap_or(0);
        let row = rows.get(done).ok_or_else(|| ErrorObject::not_found(if rows.is_empty() { "Your Rotation Chain has no enabled cloud model. Add one in Settings → Agents." } else { "Every model in your chain has already been asked for this step." }))?;
        let turns = self.store().turns(&session_id).map_err(ErrorObject::internal)?;
        let failed = turns.iter().rev().find(|turn| turn["state"] != "rewound").ok_or_else(|| ErrorObject::not_found("This chat has no step to escalate yet."))?;

        if failed["engine"] != "ollama" {
            return Err(ErrorObject::bad_request("Escalate is for a step that ran on your local model; this chat's last step did not."));
        }

        let turn_id = failed["turnId"].as_str().unwrap_or_default().to_string();
        let events = self.store().turn_events(&turn_id).map_err(ErrorObject::internal)?;
        let error = events
            .iter()
            .rev()
            .find(|event| event.event["type"] == "ErrorRaised")
            .map(|event| format!("{}: {}", event.event["title"].as_str().unwrap_or_default(), event.event["explanation"].as_str().unwrap_or_default()))
            .unwrap_or_else(|| failed["answer"].as_str().unwrap_or_default().chars().rev().take(1_500).collect::<Vec<_>>().into_iter().rev().collect());
        let mut paths: Vec<String> = Vec::new();

        for event in &events {
            if let Some(path) = event.event["target"].as_str().filter(|path| !path.is_empty() && !path.starts_with('/') && !path.contains("..")) {
                if !paths.iter().any(|known| known == path) {
                    paths.push(path.to_string());
                }
            }
        }

        let files: Vec<(String, String)> = paths
            .iter()
            .filter(|path| policy.secret(path, root.as_deref()).is_none() && crate::trust::secretguard::sensitive_name(path).is_none())
            .filter_map(|path| root.as_deref().and_then(|root| std::fs::read_to_string(std::path::Path::new(root).join(path)).ok()).map(|content| (path.clone(), content)))
            .take(CAPSULE_FILES)
            .collect();
        let (capsule, redactions) = build_capsule(&session_id, failed["prompt"].as_str().unwrap_or_default(), &error, &files);
        let (provider, model) = row.model.split_once(':').unwrap_or(("", &row.model));
        let tokens = crate::tokens::estimate(&capsule);
        let cost = cost_hint(&row.model);
        let estimate = cost["known"].as_bool().filter(|known| *known).map(|_| (cost["inPer1M"].as_f64().unwrap_or(0.0) * tokens as f64 + cost["outPer1M"].as_f64().unwrap_or(0.0) * 2_000.0) / 1_000_000.0);

        if !confirm {
            return Ok(json!({
                "sent": false,
                "model": row.model,
                "position": done + 1,
                "of": rows.len(),
                "capsule": capsule,
                "files": files.iter().map(|(path, _)| path).collect::<Vec<_>>(),
                "redactions": redactions,
                "tokens": tokens,
                "estimateUsd": estimate,
                "costKnown": estimate.is_some(),
            }));
        }

        /* Written to the audit log first: a send that happened and was never recorded is the failure an audit may not have. */
        if let Some(root) = root.as_deref() {
            crate::data::sent::append(
                std::path::Path::new(root),
                &crate::data::sent::Sent {
                    at: chrono::Utc::now().to_rfc3339(),
                    model: row.model.clone(),
                    step: "escalate".into(),
                    turn: turn_id.clone(),
                    session: session_id.clone(),
                    tokens: tokens as usize,
                    redactions: redactions.clone(),
                    hash: crate::data::sent::hash(&capsule),
                    left_the_machine: true,
                },
            )
            .map_err(ErrorObject::internal)?;
        }

        self.count("escalate");

        if let Ok(mut map) = counts().lock() {
            map.insert(session_id.clone(), done + 1);
        }

        /* One turn on the chain model, started by this click. The chat's own model setting is not touched, so the step after it is the
           local model again; the turn's own gate pins it to this one model. */
        let mut params = serde_json::Map::new();

        params.insert("sessionId".into(), json!(session_id));
        /* A subscription provider runs through its own CLI; every other provider is an API model. */
        params.insert("engine".into(), json!(match provider { "claude" => "claude_code", "openai" => "codex", "gemini" => "gemini", _ => "native_api" }));
        params.insert("provider".into(), json!(provider));
        params.insert("model".into(), json!(model));
        params.insert("prompt".into(), json!(capsule));
        params.insert("agent".into(), json!(true));
        params.insert("autonomy".into(), json!(envelope.opt_str("autonomy").unwrap_or_else(|| "simple".into())));
        params.insert("escalation".into(), json!(true));

        let started = self.engine_start(&Envelope { v: crate::SDCP_VERSION.to_string(), id: "escalate".into(), method: "engine.start".into(), params, host_id: None }, out)?;

        Ok(json!({ "sent": true, "model": row.model, "turnId": started["turnId"], "position": done + 1, "of": rows.len(), "tokens": tokens, "redactions": redactions }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capsule_carries_the_goal_the_error_and_files_with_every_secret_taken_out() {
        let fake = "FAKEvalue9Zq4Xw7Lm2Pt8";
        let (capsule, fired) = build_capsule(
            "chat-capsule",
            "make the login work",
            "TypeError: cannot read properties of undefined (reading 'token')",
            &[("src/auth.js".to_string(), format!("const password = \"{fake}\";\nexport function login() {{}}\n"))],
        );

        assert!(capsule.contains("make the login work") && capsule.contains("TypeError: cannot read properties"), "{capsule}");
        assert!(capsule.contains("export function login()"));
        assert!(!capsule.contains(fake), "the secret never goes into a capsule: {capsule}");
        assert!(fired.iter().any(|rule| rule.starts_with("src/auth.js")), "{fired:?}");
    }

    #[test]
    fn a_capsule_is_small_and_names_only_a_few_files() {
        let files: Vec<(String, String)> = (0..8).map(|n| (format!("f{n}.rs"), "x".repeat(50_000))).collect();
        let (capsule, _) = build_capsule("chat-capsule-2", "g", "e", &files);

        assert!(capsule.chars().count() <= CAPSULE_CHARS);
        assert!(!capsule.contains("f4.rs"), "at most three files");
    }

    #[test]
    fn a_price_hint_is_a_number_for_a_listed_model_free_for_a_local_one_and_unknown_otherwise() {
        assert_eq!(cost_hint("ollama:qwen3.5:9b")["perStep"], 0.0);
        assert_eq!(cost_hint("nobody:no-such-model")["known"], false);
    }

    #[test]
    fn escalation_goes_down_the_chain_in_order_and_a_regular_turn_starts_it_again() {
        note_regular_turn("chat-order");
        counts().lock().unwrap().insert("chat-order".into(), 1);

        assert_eq!(counts().lock().unwrap().get("chat-order"), Some(&1));

        note_regular_turn("chat-order");

        assert_eq!(counts().lock().unwrap().get("chat-order"), None);
    }

    #[test]
    fn a_set_that_does_not_mention_the_relay_leaves_it_alone_and_one_that_does_changes_only_what_it_names() {
        let stored = RelayPolicy { auto: true, max_usd_per_turn: 1.0, ..Default::default() };
        let plain = Envelope { v: "0.1".into(), id: "1".into(), method: "rotation.set".into(), params: serde_json::from_value(json!({ "rows": [] })).unwrap(), host_id: None };

        assert_eq!(parse(&plain, &stored).unwrap().relay, stored);

        let named = Envelope { params: serde_json::from_value(json!({ "rows": [], "relay": { "maxCloudStepsPerTurn": 3, "showChip": false } })).unwrap(), ..plain };
        let relay = parse(&named, &stored).unwrap().relay;

        assert_eq!((relay.auto, relay.max_usd_per_turn, relay.max_cloud_steps_per_turn, relay.show_chip), (true, 1.0, 3, false));
        assert_eq!(relay_json(&relay)["maxCloudStepsPerTurn"], 3);
    }

    #[test]
    fn a_period_becomes_a_time_and_nonsense_is_refused() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z").unwrap().with_timezone(&chrono::Utc);

        assert_eq!(parse_since("7d", now).as_deref(), Some("2026-10-13T12:00:00+00:00"));
        assert_eq!(parse_since("24h", now).as_deref(), Some("2026-10-19T12:00:00+00:00"));
        assert_eq!(parse_since("2026-10-01T00:00:00Z", now).as_deref(), Some("2026-10-01T00:00:00+00:00"));

        for bad in ["", "d", "0d", "-3d", "7x", "soon", "7"] {
            assert_eq!(parse_since(bad, now), None, "{bad:?}");
        }
    }
}
