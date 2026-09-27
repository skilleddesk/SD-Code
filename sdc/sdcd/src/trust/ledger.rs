//! The audit ledger: an append-only, hash-chained record of everything a turn did (Trust Kernel, part 1).
//!
//! The event log already holds every event, and it is append-only by API. What it cannot do is *prove*
//! it: a row edited in the database file looks exactly like a row that was always so. The ledger keeps a
//! second record of the events that matter - who ran what, on which model, which command, which file,
//! which test, which review, what it cost, who approved - and chains each row to the one before it with
//! SHA-256, so an edit or a deletion anywhere breaks the chain at that row and `audit.verify` names it.
//!
//! It is fed from one place, [`EventLog::append`](crate::sdcp::events::EventLog::append): every event any
//! part of the daemon pushes passes through it, so no feature can act without leaving its row - the
//! kernel is not a set of calls a feature remembers to make.

use serde_json::{json, Value};

use crate::sdcp::events::StoredEvent;
use crate::store::sqlite::Store;
use crate::store::trust::{audit_hash, AuditRow, GENESIS};

/// Whether an event of this type belongs in the ledger. Stream fragments (deltas, tool output lines) do
/// not: they are the words of a turn, and the ledger records what it *did*.
pub fn audited(event: &Value) -> bool {
    let kind = event["type"].as_str().unwrap_or_default();

    match kind {
        "TurnStarted" | "TurnCompleted" | "ToolCallStarted" | "ToolCallCompleted" | "PermissionRequested"
        | "PermissionResolved" | "CheckpointSaved" | "RewindApplied" | "ErrorRaised" | "SessionOpened" | "SessionClosed"
        | "HostRemoved" | "CostUpdated" | "PolicyViolation" | "KillSwitch" | "TrustScored" | "FileRestored"
        | "CheckpointLabeled" | "IntentConfirmed" | "ApprovalRecorded" | "HealthAlert" | "BudgetStop" | "GuardianAction" => true,
        "VerifyUpdated" => event["state"] == "done",
        "DeployUpdated" => matches!(event["state"].as_str(), Some("success" | "failed" | "rolled_back" | "rollback_failed")),
        "ProviderStatus" => matches!(event["status"].as_str(), Some("connected" | "removed")),
        _ => false,
    }
}

/// Who did it: the engine for a turn's own actions, the person for their decisions, SDC for its own.
fn actor(store: &Store, event: &Value, turn_id: Option<&str>) -> String {
    let person = store
        .setting("team.current")
        .ok()
        .flatten()
        .filter(|name| !name.trim().is_empty())
        .map(|name| format!("person:{name}"))
        .unwrap_or_else(|| "person".to_string());

    match event["type"].as_str().unwrap_or_default() {
        "PermissionResolved" | "RewindApplied" | "CheckpointLabeled" | "IntentConfirmed" | "ApprovalRecorded" | "FileRestored"
        | "SessionOpened" | "SessionClosed" | "HostRemoved" | "KillSwitch" => person,
        "TurnStarted" => format!(
            "{}:{}",
            event["engine"].as_str().unwrap_or("engine"),
            event["model"].as_str().unwrap_or("")
        ),
        "ToolCallStarted" | "ToolCallCompleted" | "TurnCompleted" => turn_id
            .and_then(|turn| store.turn_row(turn).ok().flatten())
            .map(|row| format!("{}:{}", row["engine"].as_str().unwrap_or(""), row["model"].as_str().unwrap_or("")))
            .unwrap_or_else(|| "engine".to_string()),
        _ => "sdc".to_string(),
    }
}

/// The one-line summary and the detail a row keeps. Everything a reader of the ledger needs to know what
/// happened is here; the full event stays in the event log.
fn describe(event: &Value) -> (String, Value) {
    let text = |key: &str| event[key].as_str().unwrap_or_default().to_string();

    match event["type"].as_str().unwrap_or_default() {
        "TurnStarted" => (
            format!("Turn on {} · {}", text("engine"), text("model")),
            json!({ "prompt": text("prompt"), "tier": text("tier"), "reading": event["reading"] }),
        ),
        "ToolCallStarted" => (
            format!("{} {}", text("name"), text("target")),
            json!({ "tool": text("tool"), "callId": text("callId"), "target": text("target") }),
        ),
        "ToolCallCompleted" => (
            format!("{} · {}", text("status"), text("meta")),
            json!({
                "callId": text("callId"),
                "status": text("status"),
                "diffLines": event["diff"].as_array().map(Vec::len).unwrap_or(0),
            }),
        ),
        "PermissionRequested" => (
            format!("Asked: {} {} [{}]", text("title"), text("target"), text("risk")),
            json!({ "permissionId": text("permissionId"), "action": text("action"), "risk": text("risk"), "target": text("target") }),
        ),
        "PermissionResolved" => (format!("Decided: {}", text("decision")), json!({ "permissionId": text("permissionId") })),
        "CheckpointSaved" => {
            let checkpoint = &event["checkpoint"];

            (
                format!("Checkpoint {}", checkpoint["title"].as_str().unwrap_or_default()),
                json!({ "id": checkpoint["id"], "filesHash": checkpoint["filesHash"] }),
            )
        }
        "RewindApplied" => (
            format!("Rewind {} to turn {}", text("direction"), event["turn"]),
            json!({ "turns": event["turns"], "files": event["files"] }),
        ),
        "VerifyUpdated" => {
            let checks: Vec<Value> = event["checks"]
                .as_array()
                .map(|checks| checks.iter().map(|check| json!({ "name": check["name"], "status": check["status"] })).collect())
                .unwrap_or_default();

            (
                format!(
                    "Verify {}",
                    match event["pass"].as_bool() {
                        Some(true) => "passed",
                        Some(false) => "did not pass",
                        None => "finished",
                    }
                ),
                json!({
                    "verdict": event["verdict"],
                    "checks": checks,
                    "review": { "engine": event["review"]["engine"], "model": event["review"]["model"], "verdict": event["review"]["verdict"] },
                    "scans": event["scans"],
                }),
            )
        }
        "ErrorRaised" => (text("title"), json!({ "explanation": text("explanation"), "source": event["source"] })),
        "TurnCompleted" => (format!("Turn ended: {}", text("summary")), json!({ "meta": text("meta"), "pass": event["pass"] })),
        "CostUpdated" => (
            format!("Cost ${:.4} ({})", event["costUsd"].as_f64().unwrap_or(0.0), text("costSource")),
            json!({
                "inputTokens": event["inputTokens"],
                "outputTokens": event["outputTokens"],
                "costUsd": event["costUsd"],
                "costSource": event["costSource"],
            }),
        ),
        "PolicyViolation" => (format!("Policy: {}", text("rule")), json!({ "target": text("target"), "action": text("action"), "sentence": text("sentence") })),
        "KillSwitch" => (format!("Kill switch: {} stopped", event["stopped"].as_array().map(Vec::len).unwrap_or(0)), json!({ "stopped": event["stopped"] })),
        "TrustScored" => (format!("Trust score {} ({})", event["score"], text("level")), json!({ "reasons": event["reasons"] })),
        "DeployUpdated" => (format!("Deploy {} {}", text("siteId"), text("state")), json!({ "deployId": text("deployId"), "kind": text("kind") })),
        _ => (event["type"].as_str().unwrap_or("Event").to_string(), event.clone()),
    }
}

/// Records one event in the ledger when it is one the ledger keeps. Called for every appended event.
pub fn record(store: &Store, entry: &StoredEvent) -> Option<AuditRow> {
    if !audited(&entry.event) {
        return None;
    }

    let turn_id = entry.turn_id.as_deref().or_else(|| entry.event["turnId"].as_str());
    let session_id = entry.session_id.as_deref().or_else(|| entry.event["sessionId"].as_str());
    let (summary, detail) = describe(&entry.event);
    let kind = entry.event["type"].as_str().unwrap_or("Event");
    let who = actor(store, &entry.event, turn_id);

    store.append_audit(session_id, turn_id, &who, kind, &summary, &detail).ok()
}

/// Walks the whole chain and says whether it is intact - and, when it is not, the first row that breaks it.
pub fn verify(store: &Store) -> Value {
    let rows = match store.audit_chain() {
        Ok(rows) => rows,
        Err(error) => return json!({ "intact": false, "entries": 0, "brokenAt": Value::Null, "reason": error.to_string() }),
    };
    let mut prev = GENESIS.to_string();

    for row in &rows {
        let expected = audit_hash(
            &prev,
            row.seq,
            &row.ts,
            row.session_id.as_deref(),
            row.turn_id.as_deref(),
            &row.actor,
            &row.kind,
            &row.summary,
            &row.detail,
        );

        if row.prev_hash != prev {
            return json!({
                "intact": false,
                "entries": rows.len(),
                "brokenAt": row.seq,
                "reason": format!("row {} does not follow the row before it: a row was removed or inserted", row.seq),
            });
        }

        if row.hash != expected {
            return json!({
                "intact": false,
                "entries": rows.len(),
                "brokenAt": row.seq,
                "reason": format!("row {} was changed after it was written", row.seq),
            });
        }

        prev = row.hash.clone();
    }

    json!({ "intact": true, "entries": rows.len(), "brokenAt": Value::Null, "head": prev })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(event: Value, session: Option<&str>, turn: Option<&str>) -> StoredEvent {
        StoredEvent {
            seq: 1,
            ts: chrono::Utc::now().to_rfc3339(),
            session_id: session.map(str::to_string),
            turn_id: turn.map(str::to_string),
            event,
        }
    }

    #[test]
    fn deltas_are_not_audited_and_actions_are() {
        assert!(!audited(&json!({ "type": "TurnDelta" })));
        assert!(!audited(&json!({ "type": "VerifyUpdated", "state": "running" })));
        assert!(audited(&json!({ "type": "VerifyUpdated", "state": "done" })));
        assert!(audited(&json!({ "type": "ToolCallStarted" })));
    }

    #[test]
    fn a_tampered_row_is_found_and_named() {
        let store = Store::in_memory().unwrap();

        for index in 0..4 {
            record(
                &store,
                &entry(json!({ "type": "ToolCallStarted", "name": "Run", "target": format!("npm test {index}"), "tool": "run" }), Some("s1"), Some("t1")),
            )
            .unwrap();
        }

        assert_eq!(verify(&store)["intact"], true);
        assert_eq!(verify(&store)["entries"], 4);

        store.tamper_audit(3, "Run rm -rf /").unwrap();

        let report = verify(&store);

        assert_eq!(report["intact"], false);
        assert_eq!(report["brokenAt"], 3);
    }

    #[test]
    fn a_permission_decision_is_the_persons() {
        let store = Store::in_memory().unwrap();
        let row = record(&store, &entry(json!({ "type": "PermissionResolved", "permissionId": "p1", "decision": "deny" }), None, None)).unwrap();

        assert_eq!(row.actor, "person");
        assert_eq!(row.summary, "Decided: deny");
    }
}
