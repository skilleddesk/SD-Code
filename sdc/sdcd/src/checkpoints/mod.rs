//! Checkpoints: the automatic safety net (master spec section 14).
//!
//! Every turn that mutates a file gets a checkpoint *before* the mutation (principle P5). A
//! checkpoint is three things, and all three are recorded in one row:
//!
//! * `filesHash` - the SHA-256 the file guard returned, and the shadow git commit's sha;
//! * `thumbnail` - a screenshot path when a preview was open, `null` otherwise;
//! * `rewindRef` - the pointer the rewind stack of `rewind/` pops.
//!
//! `screenshot.rs` explains why `thumbnail` is usually `null` on this build, and why that does not
//! weaken the rewind: the *files* are what a rewind restores, and the file half is real today.

pub mod screenshot;

use std::sync::Arc;

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;
use crate::store::sqlite::Store;

/// The checkpoint a turn just produced, as the `CheckpointSaved` event carries it.
pub struct Fresh {
    pub id: String,
    pub turn: i64,
    pub title: String,
    pub thumbnail: Option<String>,
    pub files_hash: String,
}

impl Fresh {
    pub fn to_event_payload(&self) -> Value {
        json!({
            "id": self.id,
            "turn": self.turn,
            "ts": chrono::Utc::now().to_rfc3339(),
            "title": self.title,
            "thumbnail": self.thumbnail,
            "filesHash": self.files_hash,
            "rewindRef": Value::Null,
        })
    }
}

/// Records a checkpoint: the shadow git commit, the row, and the same fact returned as a payload for
/// the event. `project_root` is `None` when the session has no project yet, in which case the hash is
/// of the title - a checkpoint with nothing to restore is still a point in the conversation.
pub fn create(
    store: &Arc<Store>,
    session_id: &str,
    turn: i64,
    title: &str,
    project_root: Option<&std::path::Path>,
    thumbnail: Option<String>,
) -> Result<Fresh, ErrorObject> {
    let files_hash = match project_root {
        Some(root) => crate::git::checkpoint(root, &format!("sdcd: turn {turn} — {title}"))?,
        None => crate::fs::hash(title.as_bytes()),
    };
    let id = format!("cp-{session_id}-{turn}");

    store
        .insert_checkpoint(&id, session_id, turn, title, thumbnail.as_deref(), &files_hash)
        .map_err(ErrorObject::internal)?;

    Ok(Fresh { id, turn, title: title.to_string(), thumbnail, files_hash })
}

/// Every checkpoint of a session, newest first - `checkpoint.list`'s answer.
pub fn list(store: &Arc<Store>, session_id: &str) -> Result<Vec<Value>, ErrorObject> {
    store.checkpoints(session_id).map_err(ErrorObject::internal)
}

/// The checkpoint a rewind should go to for a turn id (`turn-14` → 14).
pub fn turn_of(turn_id: &str) -> i64 {
    turn_id.trim_start_matches("turn-").parse::<i64>().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_a_checkpoint_without_a_project_root() {
        let store = Arc::new(Store::in_memory().unwrap());
        /* The schema's foreign keys are real: a checkpoint belongs to a session, and a session to a
           host. Seeding them is what makes the constraint meaningful rather than decorative. */
        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s1", "local", "Add rate limiting", "add").unwrap();

        let fresh = create(&store, "s1", 3, "Added validation", None, None).unwrap();

        assert_eq!(fresh.id, "cp-s1-3");
        assert_eq!(fresh.files_hash.len(), 64);
        assert!(fresh.thumbnail.is_none());

        let listed = list(&store, "s1").unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["turn"], json!(3));
        assert_eq!(listed[0]["filesHash"], json!(fresh.files_hash));
    }

    #[test]
    fn two_checkpoints_of_the_same_turn_replaces_rather_than_duplicates() {
        let store = Arc::new(Store::in_memory().unwrap());

        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s1", "local", "Add rate limiting", "add").unwrap();

        create(&store, "s1", 4, "first", None, None).unwrap();
        create(&store, "s1", 4, "second", None, None).unwrap();

        let listed = list(&store, "s1").unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["title"], json!("second"));
    }

    #[test]
    fn turns_are_read_back_from_their_ids() {
        assert_eq!(turn_of("turn-14"), 14);
        assert_eq!(turn_of("nonsense"), 0);
    }
}
