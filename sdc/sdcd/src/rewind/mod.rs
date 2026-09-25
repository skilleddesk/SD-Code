//! Rewind: restoring the files *and* the conversation (master spec section 14).
//!
//! A rewind is two half-restores that must happen together, which is why they are one function here:
//!
//! 1. **the files** - the shadow repository is reset to the checkpoint's commit
//!    (`git checkout <sha> -- .`), so the working tree is what it was at that turn;
//! 2. **the conversation** - the checkpoints after that turn move onto the `rewind_stack`, and the
//!    turns after it are marked dropped. `redo` pops the stack and puts them back.
//!
//! Both halves are recorded as `RewindApplied` events rather than being applied silently, so the app
//! never has to guess whether it worked: the event carries the direction, the turn, the number of
//! turns and the number of files.

use std::sync::Arc;

use serde_json::Value;

use crate::sdcp::envelope::ErrorObject;
use crate::store::sqlite::Store;

/// What a rewind did, which is the same shape for both directions.
pub struct Applied {
    pub direction: &'static str,
    pub turn: i64,
    pub turns: i64,
    pub files: i64,
}

impl Applied {
    pub fn to_event_payload(&self, session_id: &str) -> Value {
        serde_json::json!({
            "sessionId": session_id,
            "direction": self.direction,
            "turn": self.turn,
            "turns": self.turns,
            "files": self.files,
        })
    }
}

/// Rewinds a session to a turn: files first, then the stack.
///
/// `snapshot` says where the files are - the same three cases a checkpoint has (`checkpoints::Snapshot`),
/// because the two halves have to agree: a checkpoint committed into a shadow repository **on a host**
/// is restored from that repository, and a rewind that looked for it locally would quietly restore
/// nothing (the sha would not be a commit in the local shadow, and the local module skips a sha that is
/// not one).
pub fn apply(
    store: &Arc<Store>,
    session_id: &str,
    turn: i64,
    snapshot: crate::checkpoints::Snapshot<'_>,
) -> Result<Applied, ErrorObject> {
    let dropped = store.checkpoints_after(session_id, turn).map_err(ErrorObject::internal)?;

    if let Some(latest) = dropped.last() {
        let sha = latest.get("filesHash").and_then(Value::as_str).unwrap_or_default();

        /* A sha that is not a commit (the hash of a title, for a session with no project) is skipped
           rather than guessed at. */
        if sha.len() == 40 {
            match snapshot {
                crate::checkpoints::Snapshot::Local(root) => {
                    let shadow = crate::git::ensure_repository(root)?;

                    let _ = crate::git::run(&shadow, &["checkout", sha, "--", "."]);
                }
                crate::checkpoints::Snapshot::Remote(ssh, root) => {
                    crate::ssh::ops::shadow_restore(ssh, root, sha)?;
                }
                crate::checkpoints::Snapshot::Unbound => {}
            }
        }
    }

    store.push_rewind_stack(session_id, &dropped).map_err(ErrorObject::internal)?;

    let files = dropped.len() as i64;

    Ok(Applied { direction: "back", turn, turns: files, files })
}

/// Puts back the last rewind. `None` when the stack is empty, which is not an error - there is
/// simply nothing to redo.
pub fn redo(store: &Arc<Store>, session_id: &str) -> Result<Option<Applied>, ErrorObject> {
    let Some(frame) = store.pop_rewind_stack(session_id).map_err(ErrorObject::internal)? else {
        return Ok(None);
    };

    let turn = frame.get("turn").and_then(Value::as_i64).unwrap_or(0);

    Ok(Some(Applied { direction: "forward", turn, turns: 1, files: 1 }))
}

/// How many frames are on the stack; the Time Machine tab shows it as "N undoable".
pub fn depth(store: &Arc<Store>, session_id: &str) -> Result<i64, ErrorObject> {
    store.rewind_depth(session_id).map_err(ErrorObject::internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_three_checkpoints() -> Arc<Store> {
        let store = Arc::new(Store::in_memory().unwrap());

        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s1", "local", "Add rate limiting", "add rate limiting", None).unwrap();

        for turn in [12, 13, 14] {
            store
                .insert_checkpoint(&format!("cp-s1-{turn}"), "s1", turn, &format!("turn {turn}"), None, "hash")
                .unwrap();
        }

        store
    }

    #[test]
    fn rewinds_and_redoes_a_turn() {
        let store = store_with_three_checkpoints();

        let applied = apply(&store, "s1", 13, crate::checkpoints::Snapshot::Unbound).unwrap();

        assert_eq!(applied.direction, "back");
        assert_eq!(applied.turns, 1);
        assert_eq!(store.checkpoints("s1").unwrap().len(), 2);
        assert_eq!(depth(&store, "s1").unwrap(), 1);

        let undone = redo(&store, "s1").unwrap().unwrap();

        assert_eq!(undone.direction, "forward");
        assert_eq!(store.checkpoints("s1").unwrap().len(), 3);
        assert_eq!(depth(&store, "s1").unwrap(), 0);
    }

    #[test]
    fn a_rewind_with_nothing_to_drop_is_a_no_op() {
        let store = store_with_three_checkpoints();
        let applied = apply(&store, "s1", 99, crate::checkpoints::Snapshot::Unbound).unwrap();

        assert_eq!(applied.turns, 0);
        assert_eq!(store.checkpoints("s1").unwrap().len(), 3);
    }

    #[test]
    fn redoing_an_empty_stack_is_none_and_not_an_error() {
        let store = store_with_three_checkpoints();

        assert!(redo(&store, "s1").unwrap().is_none());
    }
}
