//! The append-only event log (master spec sections 3.3 and 5.4).
//!
//! Append-only is a property of the *API*, not a promise in a comment: there is `append`, there is
//! `since`, and there is nothing else. No update, no delete, no addressing an event by position. A
//! correction is a new event, which is what makes the app's time travel (`applyEvents(empty, log)`)
//! the same operation as its normal rendering.
//!
//! Events are carried as `serde_json::Value` built by the constructors in `event` below. Two
//! reasons, and both are about the wire being the contract:
//!
//! * the catalogue is defined once, in `protocol/sdcp.schema.json`, and this module mirrors it
//!   rather than re-declaring it in a second type system;
//! * a value that is already `{"type": "TurnDelta", â€¦}` is exactly what a notification carries, so
//!   there is no re-serialisation step where a field could be renamed by accident.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::Utc;
use serde_json::Value;

use crate::store::sqlite::Store;

/// One entry of the log, as it was persisted and as it will be replayed.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub seq: i64,
    pub ts: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub event: Value,
}

/// The log. One `Arc<EventLog>` is shared by every connection - cloning the log would be cloning the
/// sequence counter, which is exactly the bug this shape prevents.
pub struct EventLog {
    next: AtomicI64,
    /// The in-memory projection, oldest first. The database is the record; this is what `since()`
    /// answers from without a query per reconnect.
    entries: Mutex<Vec<StoredEvent>>,
    /// Where every event is persisted as it is appended. `None` only for a throwaway log.
    store: Option<Arc<Store>>,
}

impl EventLog {
    /// Hydrates from the store, so a restarted daemon continues the same sequence.
    pub fn hydrate(store: Arc<Store>) -> Result<Self> {
        let entries = store.recent_events(0)?;
        let next = entries.last().map(|entry| entry.seq + 1).unwrap_or(1);

        Ok(Self { next: AtomicI64::new(next), entries: Mutex::new(entries), store: Some(store) })
    }

    /// An empty log, for a test that does not want a database.
    pub fn empty() -> Self {
        Self { next: AtomicI64::new(1), entries: Mutex::new(Vec::new()), store: None }
    }

    /// The highest sequence number in the log; `0` before the first event.
    pub fn seq(&self) -> i64 {
        self.next.load(Ordering::SeqCst) - 1
    }
    /// Appends one event, persists it, and returns the stored entry.
    ///
    /// Persisting here rather than at the transport is deliberate: an event that a background turn
    /// pushed must be in the database even if the client that started the turn has already gone
    /// away, and a reconnecting client replays it with `event.list` (spec section 5.4).
    pub fn append(
        &self,
        event: Value,
        session_id: Option<String>,
        turn_id: Option<String>,
    ) -> StoredEvent {
        let entry = StoredEvent {
            seq: self.next.fetch_add(1, Ordering::SeqCst),
            ts: Utc::now().to_rfc3339(),
            session_id,
            turn_id,
            event,
        };

        if let Some(store) = &self.store {
            let _ = store.store_event(&entry);
        }

        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry.clone());
        }

        entry
    }

    /// Everything after `since`, oldest first - what `event.list` answers with.
    pub fn since(&self, since: i64) -> Vec<StoredEvent> {
        self.entries
            .lock()
            .map(|entries| entries.iter().filter(|entry| entry.seq > since).cloned().collect())
            .unwrap_or_default()
    }

    /// How many events the log holds.
    pub fn len(&self) -> usize {
        self.entries.lock().map(|entries| entries.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The event constructors: one per catalogue entry, so a handler never hand-writes a `type` string
/// and a field name in two places.
pub mod event {
    use serde_json::{json, Map, Value};

    fn base(kind: &str, fields: Value) -> Value {
        let mut map = Map::new();

        map.insert("type".to_string(), Value::String(kind.to_string()));

        if let Value::Object(extra) = fields {
            map.extend(extra);
        }

        Value::Object(map)
    }

    /// `HostStatus` - the host's state, and the three facts that explain it.
    ///
    /// `platform` is the machine line (`Debian 12 · x64`). `detail` is the *sentence* - what just
    /// happened to this host, in words: `copying SDC's key with that password…`, `root@vps is
    /// reachable`, or the fingerprint a host is waiting to be trusted with. Until 0.7.13 there was one
    /// parameter for both jobs and the sentences travelled in `platform`, so a card that reads the
    /// machine line read "copying SDC's key…" instead, and About's `This host` row said the same.
    /// `host_key` is the fingerprint itself when the host is **waiting to be trusted** - the string the
    /// dialog's `Trust and connect` button hands back to `host.trust`.
    pub fn host_status(
        host_id: &str,
        name: &str,
        host_type: &str,
        status: &str,
        platform: Option<&str>,
        detail: Option<&str>,
        host_key: Option<&str>,
    ) -> Value {
        base(
            "HostStatus",
            json!({
                "hostId": host_id,
                "name": name,
                "hostType": host_type,
                "status": status,
                "sdcd": crate::VERSION,
                "platform": platform,
                "detail": detail,
                "hostKey": host_key,
            }),
        )
    }
    /// `host.remove`: the host and its sessions are gone. `sessions` is the count that went with it,
    /// so the toast that follows can say what was actually thrown away.
    pub fn host_removed(host_id: &str, name: &str, sessions: i64) -> Value {
        base("HostRemoved", json!({ "hostId": host_id, "name": name, "sessions": sessions }))
    }



    pub fn provider_status(fields: Value) -> Value {
        base("ProviderStatus", fields)
    }
    pub fn registry_loaded(models: Value) -> Value {
        base("RegistryLoaded", json!({ "models": models }))
    }

    pub fn session_opened(
        session_id: &str,
        host_id: &str,
        title: &str,
        prompt: &str,
        project_id: Option<&str>,
        project_root: Option<&str>,
    ) -> Value {
        base(
            "SessionOpened",
            json!({
                "sessionId": session_id,
                "hostId": host_id,
                "title": title,
                "prompt": prompt,
                /* `null` for a chat with no folder, which is every chat the user never pointed at one -
                   and the truth, rather than an empty string a reader would have to interpret. */
                "projectId": project_id,
                "projectRoot": project_root,
            }),
        )
    }

    pub fn session_closed(session_id: &str) -> Value {
        base("SessionClosed", json!({ "sessionId": session_id }))
    }

    pub fn session_updated(fields: Value) -> Value {
        base("SessionUpdated", fields)
    }

    pub fn turn_started(
        turn_id: &str,
        session_id: &str,
        engine: &str,
        model: &str,
        tier: &str,
        prompt: &str,
    ) -> Value {
        base(
            "TurnStarted",
            json!({
                "turnId": turn_id,
                "sessionId": session_id,
                "engine": engine,
                "model": model,
                "tier": tier,
                "prompt": prompt,
            }),
        )
    }

    pub fn turn_delta(turn_id: &str, delta: &str) -> Value {
        base("TurnDelta", json!({ "turnId": turn_id, "delta": delta }))
    }

    pub fn turn_completed(turn_id: &str, summary: &str, meta: &str, pass: Option<bool>) -> Value {
        base("TurnCompleted", json!({ "turnId": turn_id, "summary": summary, "meta": meta, "pass": pass }))
    }

    pub fn thinking_delta(turn_id: &str, delta: &str) -> Value {
        base("ThinkingDelta", json!({ "turnId": turn_id, "delta": delta }))
    }

    pub fn error_raised(
        session_id: &str,
        turn_id: Option<&str>,
        title: &str,
        explanation: &str,
        source: Option<&str>,
    ) -> Value {
        base(
            "ErrorRaised",
            json!({
                "sessionId": session_id,
                "turnId": turn_id,
                "title": title,
                "explanation": explanation,
                "source": source,
                "fixable": true,
            }),
        )
    }

    pub fn tool_call_started(turn_id: &str, call_id: &str, tool: &str, name: &str, target: &str) -> Value {
        base(
            "ToolCallStarted",
            json!({ "turnId": turn_id, "callId": call_id, "tool": tool, "name": name, "target": target }),
        )
    }

    pub fn tool_call_output(turn_id: &str, call_id: &str, level: &str, text: &str) -> Value {
        base(
            "ToolCallOutput",
            json!({ "turnId": turn_id, "callId": call_id, "level": level, "text": text }),
        )
    }

    pub fn tool_call_completed(turn_id: &str, call_id: &str, status: &str, meta: &str, diff: Option<Value>) -> Value {
        base(
            "ToolCallCompleted",
            json!({ "turnId": turn_id, "callId": call_id, "status": status, "meta": meta, "diff": diff }),
        )
    }

    pub fn duel_started(duel_id: &str, session_id: &str, prompt: &str, engines: Value, panes: Value) -> Value {
        base(
            "DuelStarted",
            json!({
                "duelId": duel_id,
                "sessionId": session_id,
                "prompt": prompt,
                "engines": engines,
                "panes": panes,
            }),
        )
    }

    pub fn duel_resolved(duel_id: &str, kept: Option<&str>) -> Value {
        base("DuelResolved", json!({ "duelId": duel_id, "kept": kept }))
    }

    pub fn permission_requested(fields: Value) -> Value {
        base("PermissionRequested", fields)
    }

    pub fn permission_resolved(permission_id: &str, decision: &str) -> Value {
        base("PermissionResolved", json!({ "permissionId": permission_id, "decision": decision }))
    }

    pub fn checkpoint_saved(session_id: &str, checkpoint: Value) -> Value {
        base("CheckpointSaved", json!({ "sessionId": session_id, "checkpoint": checkpoint }))
    }

    pub fn rewind_applied(session_id: &str, direction: &str, turn: i64, turns: i64, files: i64) -> Value {
        base(
            "RewindApplied",
            json!({
                "sessionId": session_id,
                "direction": direction,
                "turn": turn,
                "turns": turns,
                "files": files,
            }),
        )
    }

    pub fn toast(message: &str, action: Option<&str>, hold_ms: Option<i64>) -> Value {
        base("Toast", json!({ "message": message, "action": action, "holdMs": hold_ms }))
    }
}
