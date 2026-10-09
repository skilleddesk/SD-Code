//! The append-only event log (master spec sections 3.3 and 5.4).
//!
//! Append-only is a property of the *API*, not a promise in a comment: there is `append`, there is
//! `since`, and there is nothing else. No update, no delete, no addressing an event by position. A
//! correction is a new event, which is what makes the app's time travel (`applyEvents(empty, log)`)
//! the same operation as its normal rendering.
//!
//! One exception since 1.0, and it changes no fold: a finished turn's streamed pieces are joined into one
//! event per run (`compaction`), so the replay at every start is not hundreds of thousands of single words.
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
    /// Events the database refused, oldest first, written before the next one (0.16.1).
    ///
    /// A failed write used to be dropped (`let _ = store.store_event(..)`): the subscribers had the
    /// event, the disk did not, and after a restart the log handed the same seq to a different event -
    /// history rewritten under the window. Now a refused event waits here and is written, in order,
    /// as soon as the database takes writes again.
    unsaved: Mutex<Vec<StoredEvent>>,
}

impl EventLog {
    /// Hydrates from the store, so a restarted daemon continues the same sequence.
    pub fn hydrate(store: Arc<Store>) -> Result<Self> {
        compact_completed(&store);

        let entries = store.recent_events(0)?;
        let next = entries.last().map(|entry| entry.seq + 1).unwrap_or(1);

        Ok(Self { next: AtomicI64::new(next), entries: Mutex::new(entries), store: Some(store), unsaved: Mutex::new(Vec::new()) })
    }

    /// An empty log, for a test that does not want a database.
    pub fn empty() -> Self {
        Self { next: AtomicI64::new(1), entries: Mutex::new(Vec::new()), store: None, unsaved: Mutex::new(Vec::new()) }
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
            self.persist(store, &entry);

            /* The Trust Kernel's ledger is fed here, the one path every event takes, so no feature can
               act without leaving its hash-chained row (trust::ledger). */
            crate::trust::ledger::record(store, &entry);
        }

        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry.clone());
        }

        entry
    }

    /// Writes this event - after any the database refused before it, so the disk keeps the log's order.
    fn persist(&self, store: &Store, entry: &StoredEvent) {
        let Ok(mut unsaved) = self.unsaved.lock() else {
            return;
        };

        unsaved.push(entry.clone());

        let mut written = 0;

        for waiting in unsaved.iter() {
            match store.store_event(waiting) {
                Ok(()) => written += 1,
                Err(error) => {
                    /* Said once per refused event, not once per retry. */
                    if waiting.seq == entry.seq {
                        eprintln!("sdcd: event {} is not on disk yet ({error}); it is kept and written with the next one", entry.seq);
                    }

                    break;
                }
            }
        }

        unsaved.drain(..written);
    }

    /// How many events are waiting to be written - `0` on a healthy disk.
    pub fn unsaved(&self) -> usize {
        self.unsaved.lock().map(|unsaved| unsaved.len()).unwrap_or(0)
    }

    /// Everything after `since`, oldest first - what `event.list` answers with.
    pub fn since(&self, since: i64) -> Vec<StoredEvent> {
        self.entries
            .lock()
            .map(|entries| entries.iter().filter(|entry| entry.seq > since).cloned().collect())
            .unwrap_or_default()
    }

    /// Folds a finished turn's streamed pieces into one event per run (1.0) - see [`compaction`]. In the
    /// database and in the projection `event.list` answers from, together.
    pub fn compact_turn(&self, turn_id: &str) {
        let Some(store) = &self.store else {
            return;
        };
        let Ok(events) = store.turn_events(turn_id) else {
            return;
        };
        let (updates, deletes) = compaction(&events);

        if deletes.is_empty() || store.apply_compaction(&updates, &deletes).is_err() {
            return;
        }

        if let Ok(mut entries) = self.entries.lock() {
            let gone: std::collections::HashSet<i64> = deletes.iter().copied().collect();
            let merged: std::collections::HashMap<i64, &Value> = updates.iter().map(|(seq, payload)| (*seq, payload)).collect();

            entries.retain(|entry| !gone.contains(&entry.seq));

            for entry in entries.iter_mut() {
                if let Some(payload) = merged.get(&entry.seq) {
                    entry.event = (*payload).clone();
                }
            }
        }
    }

    /// How many events the log holds.
    pub fn len(&self) -> usize {
        self.entries.lock().map(|entries| entries.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The setting that remembers how far the compaction pass has looked.
const COMPACTED_THROUGH: &str = "events.compactedThrough";

/// Compacts every turn that completed since the last pass (1.0), and gives the space back once a lot went.
fn compact_completed(store: &Store) {
    let through: i64 = store.setting(COMPACTED_THROUGH).ok().flatten().and_then(|value| value.parse().ok()).unwrap_or(0);
    let Ok(turns) = store.turns_completed_after(through) else {
        return;
    };
    let mut removed = 0;

    for turn in turns {
        let Ok(events) = store.turn_events(&turn) else {
            continue;
        };
        let (updates, deletes) = compaction(&events);

        if !deletes.is_empty() && store.apply_compaction(&updates, &deletes).is_ok() {
            removed += deletes.len();
        }
    }

    if let Ok(Some(last)) = store.recent_events(0).map(|events| events.last().map(|event| event.seq)) {
        let _ = store.set_setting(COMPACTED_THROUGH, &last.to_string());
    }

    if removed > 10_000 {
        let _ = store.vacuum();
    }
}

/// What a finished turn's log becomes (1.0): every run of streamed pieces - `ThinkingDelta`s, `TurnDelta`s,
/// a tool's `ToolCallOutput` lines of one level - is one event carrying them all, at the run's first `seq`.
///
/// The app replays the whole log at every start and folds it event by event. The owner's log held 302,317
/// events, 207,007 of them one word of thinking each and 64,924 one line of output: opening SDC took a long
/// time and the sidebar spun meanwhile. The fold of a run is the fold of its joined text - the reducer appends
/// deltas, and an output entry shows its lines as they were - so the window draws the same turn from a
/// fraction of the events. A run is broken by any other event of the same turn, so the order of everything
/// the turn did is kept. Answers the merged payloads by `seq`, and the `seq`s to delete.
pub fn compaction(events: &[StoredEvent]) -> (Vec<(i64, Value)>, Vec<i64>) {
    let mut updates: Vec<(i64, Value)> = Vec::new();
    let mut deletes: Vec<i64> = Vec::new();
    /* The open run: its first event's seq, its payload, and how many it has absorbed. */
    let mut open: Option<(i64, Value, usize)> = None;
    let key = |event: &Value| -> Option<(String, String)> {
        match event["type"].as_str()? {
            kind @ ("ThinkingDelta" | "TurnDelta") => Some((kind.to_string(), String::new())),
            "ToolCallOutput" => Some(("ToolCallOutput".to_string(), format!("{}\u{1}{}", event["callId"].as_str()?, event["level"].as_str()?))),
            _ => None,
        }
    };
    let close = |open: &mut Option<(i64, Value, usize)>, updates: &mut Vec<(i64, Value)>| {
        if let Some((seq, payload, absorbed)) = open.take() {
            if absorbed > 0 {
                updates.push((seq, payload));
            }
        }
    };

    for entry in events {
        let this = key(&entry.event);
        let joins = match (&open, &this) {
            (Some((_, payload, _)), Some(this)) => key(payload).as_ref() == Some(this),
            _ => false,
        };

        if joins {
            if let Some((_, payload, absorbed)) = open.as_mut() {
                let (field, separator) = if this.as_ref().is_some_and(|(kind, _)| kind == "ToolCallOutput") { ("text", "\n") } else { ("delta", "") };
                let joined = format!("{}{separator}{}", payload[field].as_str().unwrap_or_default(), entry.event[field].as_str().unwrap_or_default());

                payload[field] = Value::String(joined);
                *absorbed += 1;
            }

            deletes.push(entry.seq);
            continue;
        }

        close(&mut open, &mut updates);

        if this.is_some() {
            open = Some((entry.seq, entry.event.clone(), 0));
        }
    }

    close(&mut open, &mut updates);

    /* A `ContextUpdated` replaces the session's gauge whole, so of a finished turn's only the last one counts. */
    let gauges: Vec<i64> = events.iter().filter(|entry| entry.event["type"] == "ContextUpdated").map(|entry| entry.seq).collect();

    deletes.extend(gauges.iter().take(gauges.len().saturating_sub(1)));
    deletes.sort_unstable();

    (updates, deletes)
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
    /// The catalogue was refreshed from the providers' own endpoints (0.9.0). Carries counts, not
    /// rows: a window that cares asks `models.list`, which answers from the fresh cache.
    pub fn models_updated(providers: &[String], models: usize) -> Value {
        base("ModelsUpdated", json!({ "providers": providers, "models": models }))
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

    /// A tool call the model is still writing (0.14.2) - the live view before `ToolCallStarted`.
    pub fn tool_call_drafting(turn_id: &str, call_id: &str, name: &str, target: &str, chars: u64, preview: &str) -> Value {
        base(
            "ToolCallDrafting",
            json!({ "turnId": turn_id, "callId": call_id, "name": name, "target": target, "chars": chars, "preview": preview }),
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

    /// The agent's checklist (v4): every step with its status, replacing the last one sent for the turn.
    /// A `/research` turn's numbered sources (0.16.1) - the list under its answer.
    pub fn research_sources(turn_id: &str, session_id: &str, sources: Value) -> Value {
        base("ResearchSources", json!({ "turnId": turn_id, "sessionId": session_id, "sources": sources }))
    }

    pub fn plan_updated(turn_id: &str, steps: Value) -> Value {
        base("PlanUpdated", json!({ "turnId": turn_id, "steps": steps }))
    }

    /// Words the person sent into a running turn, as the model received them (0.12.5).
    pub fn turn_steered(turn_id: &str, text: &str) -> Value {
        base("TurnSteered", json!({ "turnId": turn_id, "text": text }))
    }

    /// How full the model's context is for a turn (0.13): what it is sent against what it holds, whether
    /// older turns were folded, and whether a CLI continues its own conversation.
    pub fn context_updated(session_id: &str, turn_id: &str, used: u64, window: u64, compacted: bool, resumed: bool) -> Value {
        base(
            "ContextUpdated",
            json!({
                "sessionId": session_id,
                "turnId": turn_id,
                "usedTokens": used,
                "windowTokens": window,
                "percent": (used * 100).checked_div(window).unwrap_or(0).min(100),
                "compacted": compacted,
                "resumed": resumed,
            }),
        )
    }

    /// The agent asks the person something and waits (0.13, `ask_user`); `question.answer` replies.
    pub fn question_asked(session_id: &str, turn_id: &str, question_id: &str, question: &str, options: &[String]) -> Value {
        base(
            "QuestionAsked",
            json!({
                "sessionId": session_id,
                "turnId": turn_id,
                "questionId": question_id,
                "question": question,
                "options": options,
            }),
        )
    }

    /// A question was answered (or the turn stopped asking it): the card closes.
    pub fn question_answered(turn_id: &str, question_id: &str, answer: &str) -> Value {
        base("QuestionAnswered", json!({ "turnId": turn_id, "questionId": question_id, "answer": answer }))
    }

    /// A verify run as it stands (v4): its checks, its review, and whether it passed - whole each time.
    pub fn verify_updated(fields: Value) -> Value {
        base("VerifyUpdated", fields)
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

    /* ---- The Trust Kernel (0.12) ---------------------------------------------------------------- */

    /// What a turn cost, and where the number came from (`measured`, `priced`, `local`, `subscription`,
    /// `unpriced`, `none`). Pushed once, when the turn ends.
    #[allow(clippy::too_many_arguments)]
    pub fn cost_updated(
        session_id: &str,
        turn_id: &str,
        engine: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        cost_source: &str,
        estimate_usd: Option<f64>,
        saved_usd: Option<f64>,
    ) -> Value {
        base(
            "CostUpdated",
            json!({
                "sessionId": session_id,
                "turnId": turn_id,
                "engine": engine,
                "model": model,
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
                "costUsd": cost_usd,
                "costSource": cost_source,
                "estimateUsd": estimate_usd,
                "savedUsd": saved_usd,
            }),
        )
    }

    /// A policy rule was hit: which rule, on what, what SDC did about it, and the sentence for the person.
    pub fn policy_violation(session_id: &str, turn_id: Option<&str>, rule: &str, target: &str, action: &str, sentence: &str) -> Value {
        base(
            "PolicyViolation",
            json!({ "sessionId": session_id, "turnId": turn_id, "rule": rule, "target": target, "action": action, "sentence": sentence }),
        )
    }

    /// The governor stopped a turn: `kind` is `budget` or `runaway`.
    pub fn budget_stop(session_id: &str, turn_id: &str, kind: &str, sentence: &str) -> Value {
        base("BudgetStop", json!({ "sessionId": session_id, "turnId": turn_id, "kind": kind, "sentence": sentence }))
    }

    /// Something a browser did through SDC Anywhere (0.17): unlock, Kill, a refused decision, a revoked
    /// device. Audited, so the ledger says which device did it.
    pub fn remote_activity(what: &str, detail: Value) -> Value {
        base("RemoteActivity", json!({ "what": what, "detail": detail }))
    }

    /// The kill switch: everything that was stopped, and the checkpoints that keep the state it stopped in.
    pub fn kill_switch(stopped: Value, checkpoints: Value) -> Value {
        base("KillSwitch", json!({ "stopped": stopped, "checkpoints": checkpoints }))
    }

    /// A turn's Trust score, with the reasons that make it up.
    pub fn trust_scored(session_id: &str, turn_id: &str, score: i64, level: &str, reasons: Value) -> Value {
        base(
            "TrustScored",
            json!({ "sessionId": session_id, "turnId": turn_id, "score": score, "level": level, "reasons": reasons }),
        )
    }

    /// A checkpoint row changed after it was saved: a label, or the mark that something after it cannot
    /// be undone. Carries the whole row, like `CheckpointSaved`.
    pub fn checkpoint_updated(session_id: &str, checkpoint: Value) -> Value {
        base("CheckpointUpdated", json!({ "sessionId": session_id, "checkpoint": checkpoint }))
    }

    /// One file was put back as a checkpoint had it.
    pub fn file_restored(session_id: &str, checkpoint_id: &str, path: &str, outcome: &str, undo: Option<&str>) -> Value {
        base(
            "FileRestored",
            json!({ "sessionId": session_id, "checkpointId": checkpoint_id, "path": path, "outcome": outcome, "undoCheckpointId": undo }),
        )
    }

    /// The person confirmed (or corrected) what the Intent Engine understood.
    pub fn intent_confirmed(session_id: Option<&str>, intent_id: &str, corrections: i64) -> Value {
        base("IntentConfirmed", json!({ "sessionId": session_id, "intentId": intent_id, "corrections": corrections }))
    }

    /// A deploy as it stands now, whole each time (like `VerifyUpdated`).
    pub fn deploy_updated(fields: Value) -> Value {
        base("DeployUpdated", fields)
    }

    /// A site's latest health report and Ops score.
    pub fn health_updated(fields: Value) -> Value {
        base("HealthUpdated", fields)
    }

    /// A site crossed a line a person should hear about, in their language.
    pub fn health_alert(site_id: &str, name: &str, level: &str, sentence: &str) -> Value {
        base("HealthAlert", json!({ "siteId": site_id, "name": name, "level": level, "sentence": sentence }))
    }

    /// The Night Guardian acted on its own (only ever a rollback to the last good deploy), or prepared a
    /// fix that waits for approval.
    pub fn guardian_action(site_id: &str, action: &str, sentence: &str, deploy_id: Option<&str>) -> Value {
        base("GuardianAction", json!({ "siteId": site_id, "action": action, "sentence": sentence, "deployId": deploy_id }))
    }

    /// Someone approved or declined something that waited for them.
    pub fn approval_recorded(approval: Value) -> Value {
        base("ApprovalRecorded", json!({ "approval": approval }))
    }
}

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use serde_json::json;

    fn entry(seq: i64, event: Value) -> StoredEvent {
        StoredEvent { seq, ts: format!("t{seq}"), session_id: Some("s".into()), turn_id: Some("t".into()), event }
    }

    /// 1.0: runs of streamed pieces become one event at the run's first seq; anything else breaks a run, so
    /// the order of what the turn did is kept, and an output run is one tool's lines of one level.
    #[test]
    fn a_finished_turns_streamed_pieces_become_one_event_per_run() {
        let events = vec![
            entry(1, json!({ "type": "TurnStarted" })),
            entry(2, json!({ "type": "ThinkingDelta", "delta": "Let " })),
            entry(3, json!({ "type": "ThinkingDelta", "delta": "me " })),
            entry(4, json!({ "type": "ThinkingDelta", "delta": "look." })),
            entry(5, json!({ "type": "TurnDelta", "delta": "Read" })),
            entry(6, json!({ "type": "TurnDelta", "delta": "ing." })),
            entry(7, json!({ "type": "ToolCallStarted", "callId": "c1" })),
            entry(8, json!({ "type": "ToolCallOutput", "callId": "c1", "level": "dim", "text": "a" })),
            entry(9, json!({ "type": "ToolCallOutput", "callId": "c1", "level": "dim", "text": "b" })),
            entry(10, json!({ "type": "ToolCallOutput", "callId": "c1", "level": "fail", "text": "exit 1" })),
            entry(11, json!({ "type": "ToolCallCompleted", "callId": "c1" })),
            entry(12, json!({ "type": "ThinkingDelta", "delta": "Again" })),
            entry(13, json!({ "type": "ContextUpdated", "usedTokens": 1 })),
            entry(14, json!({ "type": "ContextUpdated", "usedTokens": 2 })),
            entry(15, json!({ "type": "TurnCompleted" })),
        ];
        let (updates, deletes) = compaction(&events);

        assert_eq!(deletes, vec![3, 4, 6, 9, 13], "only the last gauge of the turn stays");
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0], (2, json!({ "type": "ThinkingDelta", "delta": "Let me look." })));
        assert_eq!(updates[1], (5, json!({ "type": "TurnDelta", "delta": "Reading." })));
        assert_eq!(updates[2], (8, json!({ "type": "ToolCallOutput", "callId": "c1", "level": "dim", "text": "a\nb" })));

        /* Nothing to join: nothing changes. */
        let (updates, deletes) = compaction(&events[..2]);

        assert!(updates.is_empty() && deletes.is_empty());
    }

    /// The database and the projection `event.list` answers from agree after a compaction, and a restarted
    /// log continues after the highest seq.
    #[test]
    fn a_compacted_turn_is_the_same_in_the_database_and_in_event_list() {
        let store = Arc::new(Store::in_memory().unwrap());
        let log = EventLog::hydrate(store.clone()).unwrap();

        log.append(json!({ "type": "TurnStarted" }), Some("s".into()), Some("t".into()));

        for word in ["a", "b", "c"] {
            log.append(json!({ "type": "ThinkingDelta", "delta": word }), Some("s".into()), Some("t".into()));
        }

        log.append(json!({ "type": "TurnCompleted" }), Some("s".into()), Some("t".into()));
        log.compact_turn("t");

        let listed = log.since(0);

        assert_eq!(listed.len(), 3);
        assert_eq!(listed[1].event["delta"], "abc");
        assert_eq!(store.recent_events(0).unwrap().iter().map(|entry| entry.event.clone()).collect::<Vec<_>>(), listed.iter().map(|entry| entry.event.clone()).collect::<Vec<_>>());
        assert_eq!(EventLog::hydrate(store).unwrap().seq(), 5);
    }
}
