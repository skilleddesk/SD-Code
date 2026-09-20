//! The Session Bridge - switching engines mid-turn without losing the conversation
//! (master spec section 16.5).
//!
//! What has to survive a switch is not the *tokens* the old engine had produced; it is the
//! **conversation**: the prompts and the answers so far, in order. `history_for()` builds exactly that
//! list from the turns the store already holds, and `frame()` records the switch as a
//! `SessionBridged` event so the app can put a banner on the turn instead of silently changing who is
//! answering.
//!
//! `snapshot()` is the third piece: the state a switch has to carry, in the shape the app's confirm
//! dialog shows before the user agrees to it - engine, model, the turn that is running and how far it
//! had got.

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;
use crate::store::sqlite::Store;

/// The conversation so far, as the message list the new engine is handed.
///
/// Each entry is one message: the turn's `prompt` then its `answer`, oldest first. A turn that never
/// produced an answer contributes its prompt only - a half-finished turn is still context, and
/// dropping it would change the question the new engine is being asked.
pub fn history_for(store: &Store, session_id: &str) -> Result<Vec<String>, ErrorObject> {
    let turns = store.turns(session_id).map_err(ErrorObject::internal)?;
    let mut history = Vec::new();

    for turn in turns {
        if let Some(prompt) = turn.get("prompt").and_then(Value::as_str) {
            if !prompt.is_empty() {
                history.push(prompt.to_string());
            }
        }

        if let Some(answer) = turn.get("answer").and_then(Value::as_str) {
            if !answer.is_empty() {
                history.push(answer.to_string());
            }
        }
    }

    Ok(history)
}

/// The `SessionBridged` payload.
pub fn frame(session_id: &str, turn_id: &str, from: &str, to: &str, model: &str, reason: Option<&str>) -> Value {
    json!({
        "sessionId": session_id,
        "turnId": turn_id,
        "from": from,
        "to": to,
        "model": model,
        "reason": reason,
    })
}

/// The state a switch carries, for the confirmation dialog of spec section 16.5.
pub fn snapshot(store: &Store, session_id: &str, turn_id: &str, engine: &str, model: &str) -> Result<Value, ErrorObject> {
    let turns = store.turns(session_id).map_err(ErrorObject::internal)?;
    let running = turns.iter().find(|turn| turn.get("turnId").and_then(Value::as_str) == Some(turn_id));
    let answered = running
        .and_then(|turn| turn.get("answer").and_then(Value::as_str))
        .map(str::len)
        .unwrap_or(0);

    Ok(json!({
        "sessionId": session_id,
        "turnId": turn_id,
        "engine": engine,
        "model": model,
        "contextMessages": turns.len() * 2,
        "answeredChars": answered,
        "replay": true,
    }))
}

/// The engines a switch may move to: everything except the one that is running.
pub fn candidates(engine_ids: &[&'static str], current: &str) -> Vec<String> {
    engine_ids.iter().filter(|id| **id != current).map(|id| id.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_a_turn() -> Store {
        let store = Store::in_memory().unwrap();

        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s1", "local", "Add rate limiting", "add rate limiting to the login route").unwrap();
        store.start_turn("turn-1", "s1", 1, "claude_code", "sonnet", "Balanced", "add rate limiting").unwrap();
        store.finish_turn("turn-1", "Added the limiter to the login route.", "Done", "done").unwrap();

        store
    }

    #[test]
    fn history_replays_prompts_and_answers_oldest_first() {
        let store = store_with_a_turn();
        let history = history_for(&store, "s1").unwrap();

        assert_eq!(history.len(), 2);
        assert_eq!(history[0], "add rate limiting");
        assert!(history[1].starts_with("Added the limiter"));
    }

    #[test]
    fn history_of_a_session_with_no_turns_is_empty() {
        let store = Store::in_memory().unwrap();

        assert!(history_for(&store, "nothing").unwrap().is_empty());
    }

    #[test]
    fn a_snapshot_reports_what_would_be_replayed() {
        let store = store_with_a_turn();
        let snapshot = snapshot(&store, "s1", "turn-1", "claude_code", "sonnet").unwrap();

        assert_eq!(snapshot["turnId"], json!("turn-1"));
        assert_eq!(snapshot["contextMessages"], json!(2));
        assert_eq!(snapshot["replay"], json!(true));
        assert!(snapshot["answeredChars"].as_i64().unwrap() > 0);
    }

    #[test]
    fn the_frame_is_the_event_the_banner_reads() {
        let frame = frame("s1", "turn-1", "claude_code", "codex", "default", Some("switched mid-turn"));

        assert_eq!(frame["from"], json!("claude_code"));
        assert_eq!(frame["to"], json!("codex"));
        assert_eq!(frame["reason"], json!("switched mid-turn"));
    }

    #[test]
    fn a_switch_never_offers_the_engine_it_is_already_on() {
        assert_eq!(
            candidates(&["claude_code", "codex", "gemini"], "codex"),
            vec!["claude_code".to_string(), "gemini".to_string()]
        );
    }
}
