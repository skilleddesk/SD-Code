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
    /// The `RewindApplied` event. It used to be the same fields **without** the event's `type`, so the
    /// window never recognised a rewind: its checkpoints stayed listed, Redo never appeared, and the tree
    /// kept showing the files the rewind had just changed.
    pub fn to_event_payload(&self, session_id: &str) -> Value {
        crate::sdcp::events::event::rewind_applied(session_id, self.direction, self.turn, self.turns, self.files)
    }
}

/// Rewinds a chat to a checkpoint: the folder becomes what that checkpoint recorded, and the checkpoint
/// and every later one leave the list for the redo stack.
///
/// `turn` is the checkpoint's own ordinal - the number the Time Machine, the checkpoint rail and Ctrl+Z
/// send. v4 changed what it means. It used to hide only the checkpoints *after* it and restore the newest
/// of those - so the one you clicked was never the one you got, the newest checkpoint could not be chosen
/// at all, and the file half never ran locally (see `git::restore`).
///
/// Before the folder changes, its current state is committed to the shadow repository, and that commit
/// is part of the frame: it is what a redo puts back.
pub fn apply(
    store: &Arc<Store>,
    session_id: &str,
    turn: i64,
    snapshot: crate::checkpoints::Snapshot<'_>,
    host_id: Option<&str>,
) -> Result<Applied, ErrorObject> {
    let rows = store.checkpoints_from(session_id, turn).map_err(ErrorObject::internal)?;

    let Some(target) = rows.first() else {
        return Ok(Applied { direction: "back", turn, turns: 0, files: 0 });
    };

    let sha = target["filesHash"].as_str().unwrap_or_default().to_string();
    let restorable = sha.len() == 40 && sha.chars().all(|character| character.is_ascii_hexdigit());
    let now = if restorable { commit_now(&snapshot)? } else { None };

    if restorable {
        journaled(store, session_id, &snapshot, host_id, &sha, now.as_deref(), "all", || restore(&snapshot, &sha))?;
    }

    /* The conversation half (TM-1): the turn that wrote this checkpoint and every later one leave the
       history the next engine is given, so it does not build on work that is no longer on disk. Their
       states travel in the frame, so a redo puts each back exactly as it was. */
    let dropped = store.turns_after_checkpoint(session_id, target).map_err(ErrorObject::internal)?;

    for (id, _) in &dropped {
        store.set_turn_state(id, "rewound").map_err(ErrorObject::internal)?;
    }

    let frame = serde_json::json!({
        "rows": rows,
        "nowSha": now,
        "turns": dropped.iter().map(|(id, state)| serde_json::json!({ "id": id, "state": state })).collect::<Vec<_>>(),
    });
    let hidden: Vec<String> = rows.iter().filter_map(|row| row["id"].as_str().map(str::to_string)).collect();
    let target_id = target["id"].as_str().unwrap_or_default().to_string();

    store
        .push_rewind_frame(session_id, turn, &target_id, &frame, &hidden)
        .map_err(ErrorObject::internal)?;

    Ok(Applied {
        direction: "back",
        turn,
        turns: hidden.len() as i64,
        files: i64::from(restorable),
    })
}

/// Puts back the last rewind: the folder as it was just before it, its checkpoints, and the turns it
/// took out of the conversation. `None` when the stack is empty, which is not an error - there is simply
/// nothing to redo.
pub fn redo(
    store: &Arc<Store>,
    session_id: &str,
    snapshot: crate::checkpoints::Snapshot<'_>,
    host_id: Option<&str>,
) -> Result<Option<Applied>, ErrorObject> {
    let Some((turn, frame)) = store.pop_rewind_frame(session_id).map_err(ErrorObject::internal)? else {
        return Ok(None);
    };

    let rows = frame["rows"].as_array().map(Vec::len).unwrap_or(0) as i64;
    let restored = match frame["nowSha"].as_str() {
        Some(sha) if sha.len() == 40 => {
            journaled(store, session_id, &snapshot, host_id, sha, None, "all", || restore(&snapshot, sha))?;
            1
        }
        _ => 0,
    };

    for dropped in frame["turns"].as_array().cloned().unwrap_or_default() {
        if let (Some(id), Some(state)) = (dropped["id"].as_str(), dropped["state"].as_str()) {
            store.set_turn_state(id, state).map_err(ErrorObject::internal)?;
        }
    }

    Ok(Some(Applied { direction: "forward", turn, turns: rows, files: restored }))
}

/// Runs a restore inside the journal (TM-2): the row is written **before** the folder moves and closed
/// after, so a daemon that stops in the middle finds the open row at its next start and finishes the
/// restore (`recover`) instead of leaving a folder that is half one checkpoint and half another.
#[allow(clippy::too_many_arguments)]
pub fn journaled(
    store: &Arc<Store>,
    session_id: &str,
    snapshot: &crate::checkpoints::Snapshot<'_>,
    host_id: Option<&str>,
    target: &str,
    before: Option<&str>,
    scope: &str,
    work: impl FnOnce() -> Result<(), ErrorObject>,
) -> Result<(), ErrorObject> {
    let root = match snapshot {
        crate::checkpoints::Snapshot::Local(root) => root.display().to_string(),
        crate::checkpoints::Snapshot::Remote(_, root) => root.to_string(),
        crate::checkpoints::Snapshot::Unbound => return work(),
    };
    let entry = store.journal_open(session_id, &root, host_id, target, before, scope).map_err(ErrorObject::internal)?;
    let result = work();

    let _ = store.journal_close(entry, if result.is_ok() { "done" } else { "failed" });

    result
}

/// Finishes the restores a stopped daemon left open. A whole-folder restore is re-run to its target -
/// `read-tree -u --reset` lands on the same tree however far the first attempt got - and a single-file
/// restore likewise. Local folders only: a host's restore is re-run the next time its chat is opened, so
/// the report names them. Returns one sentence per journal entry.
pub fn recover(store: &Arc<Store>) -> Vec<String> {
    let mut sentences = Vec::new();

    for entry in store.open_journals().unwrap_or_default() {
        let id = entry["id"].as_i64().unwrap_or(0);
        let root = entry["root"].as_str().unwrap_or_default();
        let target = entry["target"].as_str().unwrap_or_default();
        let scope = entry["scope"].as_str().unwrap_or("all");
        let host = entry["hostId"].as_str().filter(|host| *host != "local");

        if host.is_some() {
            let _ = store.journal_close(id, "interrupted-remote");
            sentences.push(format!(
                "A restore of {root} on a VPS was cut off when SDC stopped. Open its chat and rewind again to finish it."
            ));
            continue;
        }

        let path = std::path::Path::new(root);
        let result = if scope == "all" {
            crate::git::restore(path, target)
        } else {
            crate::git::restore_file(path, target, scope).map(|_| ())
        };

        match result {
            Ok(()) => {
                let _ = store.journal_close(id, "recovered");
                sentences.push(format!("A restore of {root} was cut off when SDC stopped; it has been finished."));
            }
            Err(error) => {
                let _ = store.journal_close(id, "failed");
                sentences.push(format!("A restore of {root} was cut off and could not be finished: {}", error.message));
            }
        }
    }

    sentences
}

/// The folder's current state as a shadow commit, so a redo has something to return to.
pub(crate) fn commit_now(snapshot: &crate::checkpoints::Snapshot<'_>) -> Result<Option<String>, ErrorObject> {
    Ok(match snapshot {
        crate::checkpoints::Snapshot::Local(root) => Some(crate::git::checkpoint(root, "sdcd: before a rewind")?),
        crate::checkpoints::Snapshot::Remote(ssh, root) => {
            Some(crate::ssh::ops::shadow_checkpoint(ssh, root, "sdcd: before a rewind")?)
        }
        crate::checkpoints::Snapshot::Unbound => None,
    })
}

fn restore(snapshot: &crate::checkpoints::Snapshot<'_>, sha: &str) -> Result<(), ErrorObject> {
    match snapshot {
        crate::checkpoints::Snapshot::Local(root) => crate::git::restore(root, sha),
        crate::checkpoints::Snapshot::Remote(ssh, root) => crate::ssh::ops::shadow_restore(ssh, root, sha),
        crate::checkpoints::Snapshot::Unbound => Ok(()),
    }
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

    /// A rewind *to* checkpoint 13 hides 13 and 14 (the folder is now what 13 recorded), and a redo puts
    /// both back **with their own titles and hashes** - the old redo re-inserted `'restored'` rows.
    #[test]
    fn rewinds_to_the_chosen_checkpoint_and_redoes_it_whole() {
        let store = store_with_three_checkpoints();

        let applied = apply(&store, "s1", 13, crate::checkpoints::Snapshot::Unbound, None).unwrap();

        assert_eq!(applied.direction, "back");
        assert_eq!(applied.turns, 2);
        assert_eq!(store.checkpoints("s1").unwrap().len(), 1);
        assert_eq!(depth(&store, "s1").unwrap(), 1);

        let undone = redo(&store, "s1", crate::checkpoints::Snapshot::Unbound, None).unwrap().unwrap();

        assert_eq!(undone.direction, "forward");
        assert_eq!(depth(&store, "s1").unwrap(), 0);

        let back = store.checkpoints("s1").unwrap();

        assert_eq!(back.len(), 3);
        assert!(back.iter().any(|row| row["title"] == "turn 13" && row["filesHash"] == "hash"), "{back:?}");
    }

    /// The file half, on a real folder: changed files go back, a deleted file returns, a file made after
    /// the checkpoint is removed - and a redo brings the later state back exactly.
    #[test]
    fn a_rewind_restores_the_folder_exactly_and_a_redo_brings_it_back() {
        let root = std::env::temp_dir().join(format!("sdc-rewind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        std::fs::write(root.join("gone.txt"), "keep me\n").unwrap();

        let Ok(sha) = crate::git::checkpoint(&root, "before the change") else {
            /* No git on this machine: the doctor says so; nothing to measure here. */
            return;
        };
        let store = store_with_three_checkpoints();

        store.insert_checkpoint("cp-s1-20", "s1", 20, "Before Edit a.txt", None, &sha).unwrap();

        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        std::fs::write(root.join("new.txt"), "made later\n").unwrap();

        let applied = apply(&store, "s1", 20, crate::checkpoints::Snapshot::Local(&root), None).unwrap();

        assert_eq!(applied.files, 1);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap().trim_end(), "one");
        assert!(root.join("gone.txt").exists(), "a deleted file comes back");
        assert!(!root.join("new.txt").exists(), "a file made after the checkpoint is removed");

        redo(&store, "s1", crate::checkpoints::Snapshot::Local(&root), None).unwrap().unwrap();

        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap().trim_end(), "two");
        assert!(root.join("new.txt").exists());
        assert!(!root.join("gone.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_event_is_a_rewind_applied_the_window_recognises() {
        let payload = Applied { direction: "back", turn: 7, turns: 2, files: 1 }.to_event_payload("s1");

        assert_eq!(payload["type"], "RewindApplied");
        assert_eq!(payload["sessionId"], "s1");
        assert_eq!(payload["turn"], 7);
    }

    #[test]
    fn a_rewind_with_nothing_to_drop_is_a_no_op() {
        let store = store_with_three_checkpoints();
        let applied = apply(&store, "s1", 99, crate::checkpoints::Snapshot::Unbound, None).unwrap();

        assert_eq!(applied.turns, 0);
        assert_eq!(store.checkpoints("s1").unwrap().len(), 3);
    }

    #[test]
    fn redoing_an_empty_stack_is_none_and_not_an_error() {
        let store = store_with_three_checkpoints();

        assert!(redo(&store, "s1", crate::checkpoints::Snapshot::Unbound, None).unwrap().is_none());
    }
}
