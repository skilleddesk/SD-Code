//! **Content-addressed checkpoints** (v3 W7a, switch `v3.cas`, on): the copy of a file as it was before SDC
//! changed it, kept under `.sdc/objects/` by its own SHA-256 - so a turn can be taken back in a project that is
//! not a git repository at all, and inside one without touching its history. `.sdc/objects/turns/<turn>.json`
//! lists what a turn changed and the blob the old content went to; `undo_turn` puts them all back.
//!
//! The shadow-git checkpoints and the VPS snapshots of v1 stay as they are: this is the layer that works everywhere
//! and needs nothing installed, and it is what the rewind stack reads when the git path is not available.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DIR: &str = ".sdc/objects";

/// One file's pre-image in a turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pre {
    pub path: String,
    /// The blob holding the content as it was; `None` when the file did not exist before (undo deletes it).
    pub blob: Option<String>,
}

/// The content's address: SHA-256 of the bytes, hex.
pub fn address(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Keeps bytes and answers their address.
pub fn save(root: &Path, bytes: &[u8]) -> Option<String> {
    let id = address(bytes);
    let path = root.join(DIR).join(&id[..2]).join(&id[2..]);

    if path.exists() {
        return Some(id);
    }

    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&path, bytes).ok()?;
    Some(id)
}

/// The bytes at an address.
pub fn load(root: &Path, id: &str) -> Option<Vec<u8>> {
    (id.len() > 4).then(|| std::fs::read(root.join(DIR).join(&id[..2]).join(&id[2..])).ok()).flatten()
}

fn turn_file(root: &Path, turn_id: &str) -> PathBuf {
    root.join(DIR).join("turns").join(format!("{}.json", turn_id.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')))
}

/// Records one file of a turn, before it was changed. `before` is `None` when the file is about to be created.
pub fn record(root: &Path, turn_id: &str, path: &str, before: Option<&[u8]>) -> Option<()> {
    let blob = match before {
        Some(bytes) => Some(save(root, bytes)?),
        None => None,
    };
    let file = turn_file(root, turn_id);
    let mut pres: Vec<Pre> = std::fs::read_to_string(&file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();

    /* A file changed twice in one turn keeps its first pre-image - the one that undoes the turn. */
    if !pres.iter().any(|pre| pre.path == path) {
        pres.push(Pre { path: path.to_string(), blob });
    }

    std::fs::create_dir_all(file.parent()?).ok()?;
    std::fs::write(&file, serde_json::to_string_pretty(&pres).ok()?).ok()
}

/// What a turn changed.
pub fn pre_images(root: &Path, turn_id: &str) -> Vec<Pre> {
    std::fs::read_to_string(turn_file(root, turn_id)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Puts a turn's files back as they were, answering how many were restored (a file the turn created is removed).
pub fn undo_turn(root: &Path, turn_id: &str) -> usize {
    let mut restored = 0;

    for pre in pre_images(root, turn_id) {
        let target = root.join(&pre.path);

        let done = match pre.blob.as_deref().and_then(|id| load(root, id)) {
            Some(bytes) => std::fs::write(&target, bytes).is_ok(),
            None => !target.exists() || std::fs::remove_file(&target).is_ok(),
        };

        if done {
            restored += 1;
        }
    }

    restored
}

/// How much disk the store takes, and how many blobs it holds - for Settings.
pub fn size(root: &Path) -> (u64, usize) {
    let mut bytes = 0;
    let mut count = 0;
    let mut stack = vec![root.join(DIR)];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = entry.metadata() {
                bytes += meta.len();
                count += 1;
            }
        }
    }

    (bytes, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_is_taken_back_without_git() {
        let root = std::env::temp_dir().join(format!("sdc-cas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/orders.ts"), "the old body\n").unwrap();

        /* A turn changes one file and creates another. */
        record(&root, "turn-1", "src/orders.ts", Some(b"the old body\n")).unwrap();
        record(&root, "turn-1", "src/new.ts", None).unwrap();
        std::fs::write(root.join("src/orders.ts"), "the new body\n").unwrap();
        std::fs::write(root.join("src/new.ts"), "brand new\n").unwrap();

        /* The same file twice in one turn keeps its first pre-image. */
        record(&root, "turn-1", "src/orders.ts", Some(b"the new body\n")).unwrap();
        assert_eq!(pre_images(&root, "turn-1").len(), 2);

        assert_eq!(undo_turn(&root, "turn-1"), 2);
        assert_eq!(std::fs::read_to_string(root.join("src/orders.ts")).unwrap(), "the old body\n");
        assert!(!root.join("src/new.ts").exists(), "a file the turn created is removed");

        let (bytes, count) = size(&root);

        assert!(bytes > 0 && count >= 2, "{bytes} bytes in {count} files");
        assert_eq!(address(b"x"), address(b"x"), "the same content is the same address");
        assert_ne!(address(b"x"), address(b"y"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
