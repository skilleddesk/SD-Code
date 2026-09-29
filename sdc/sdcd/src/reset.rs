//! Forgetting everything (0.13): the uninstaller's "Delete the application data" box, and Settings →
//! Erase all SDC data.
//!
//! The report: *"sdc install korle old jinis add hoye thakse"*. An installer carries no one's data - chats,
//! hosts and settings live in this machine's data directory, and keys in its OS keychain - so a person
//! installing SDC from GitHub on their own computer starts empty. But on *this* machine, uninstalling and
//! installing again brought everything back, because nothing ever removed the data directory or the
//! keychain entries: Tauri's own "delete app data" box removes the folder named after the bundle id,
//! and SDC's folder is `sdc`. This is the step that was missing.

use std::path::Path;

/// Every keychain entry SDC may have written: one per provider it knows, plus the custom endpoint.
fn key_names() -> Vec<String> {
    let mut ids: Vec<String> = crate::providers::registry(&[])
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_string))
        .chain(crate::providers::models::blocked().iter().map(|block| block.id.clone()))
        .chain(["custom".to_string(), "google".to_string(), "anthropic-api".to_string()])
        .collect();

    ids.sort();
    ids.dedup();
    ids.iter().map(|id| crate::providers::key_ref(id)).collect()
}

/// Removes SDC's keys from the keychain and its data directory from disk. Answers what was removed.
pub fn forget_everything(data_dir: &Path) -> Vec<String> {
    let mut removed = forget_keys();

    removed.extend(empty_folder(data_dir));

    /* The daily copies (0.15.5) are the same history: an erase asked for on purpose takes them too. */
    if !cfg!(test) {
        if let Ok(backups) = crate::paths::backup_dir() {
            removed.extend(empty_folder(&backups));
        }
    }

    removed
}

/// SDC's keys out of the OS keychain.
///
/// **Never in a test build.** A test of the erase once ran with the `keychain` feature on and deleted the
/// developer's real provider keys (2026-09-28): the keychain is the machine's, not the test's.
fn forget_keys() -> Vec<String> {
    let mut removed = Vec::new();

    if cfg!(test) {
        return removed;
    }

    for name in key_names() {
        if crate::auth::keychain::get(&name).is_some() && crate::auth::keychain::delete(&name).is_ok() {
            removed.push(format!("key {name}"));
        }
    }

    removed
}

/// Everything inside a folder, then the folder.
fn empty_folder(data_dir: &Path) -> Vec<String> {
    let mut removed = Vec::new();

    if data_dir.exists() {
        /* Every entry, then the folder: an entry held open (a log another process has) does not stop the rest. */
        if let Ok(entries) = std::fs::read_dir(data_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let gone = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };

                if gone.is_ok() {
                    removed.push(path.display().to_string());
                }
            }
        }

        let _ = std::fs::remove_dir(data_dir);
    }

    removed
}

/// The marker Settings → Erase leaves for the next start: the database cannot be deleted while it is open.
pub const MARKER: &str = "ERASE-ON-START";

/// Called before the database opens: a pending erase is carried out, keys and all.
pub fn erase_if_marked(data_dir: &Path) -> bool {
    if !data_dir.join(MARKER).exists() {
        return false;
    }

    forget_everything(data_dir);
    let _ = std::fs::create_dir_all(data_dir);

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_folder_is_emptied_and_an_unmarked_one_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("sdc-reset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("attachments")).unwrap();
        std::fs::write(dir.join("sdc.db"), "x").unwrap();

        assert!(!erase_if_marked(&dir));
        assert!(dir.join("sdc.db").exists());

        std::fs::write(dir.join(MARKER), "").unwrap();

        assert!(erase_if_marked(&dir));
        assert!(!dir.join("sdc.db").exists() && !dir.join("attachments").exists() && !dir.join(MARKER).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_provider_key_name_is_known() {
        let names = key_names();

        assert!(names.contains(&"sdc.provider.deepseek".to_string()));
        assert!(names.contains(&"sdc.provider.custom".to_string()));
    }
}
