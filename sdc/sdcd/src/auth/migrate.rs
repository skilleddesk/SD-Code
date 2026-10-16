//! **Keys out of plaintext (P0-2, switch `secrets.keychain`)**: two jobs, both of which only ever tell the person
//! *where* a key is, never the key itself.
//!
//! * [`migrate_dir`]: the file fallback (`<data>/keys/<name>`, used when no OS store was reachable) is moved into the
//!   OS keychain - written, **read back and compared**, and only then is the plaintext file deleted. Nothing happens
//!   until the person confirms (`apply = false` reports what would move).
//! * [`stray_keys`]: a text file in a project folder that holds something shaped like a key (a `keys.txt` dropped
//!   beside the code) is reported by path and rule so the person can move it; it is never uploaded or logged.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const SKIP_DIRS: [&str; 6] = [".git", "node_modules", "target", "dist", ".venv", ".sdc"];
const MAX_FILE: u64 = 64 * 1024;
const MAX_FILES: usize = 20_000;

/// Moves every key file in `dir` through `set` and `get` (the OS store, injected so the logic is testable). With
/// `apply = false` nothing is written or deleted. Per name: `moved`, `would_move`, `failed` or `empty`.
pub fn migrate_dir(dir: &Path, apply: bool, set: &mut dyn FnMut(&str, &str) -> Result<(), String>, get: &dyn Fn(&str) -> Option<String>) -> Value {
    let mut rows = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return json!({ "apply": apply, "keys": rows });
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if !path.is_file() || name.starts_with('.') {
            continue;
        }

        let secret = std::fs::read_to_string(&path).unwrap_or_default().trim().to_string();
        let state = if secret.is_empty() {
            "empty"
        } else if !apply {
            "would_move"
        } else if set(&name, &secret).is_err() || get(&name).as_deref() != Some(secret.as_str()) {
            /* A key the store did not give back is a key the file must keep. */
            "failed"
        } else if std::fs::remove_file(&path).is_err() {
            "failed"
        } else {
            "moved"
        };

        rows.push(json!({ "name": name, "state": state }));
    }

    rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

    json!({ "apply": apply, "keys": rows })
}

/// The key files of this machine's data folder into the OS keychain, when the build has one and it answers.
pub fn migrate(apply: bool) -> Value {
    if super::keychain::backend() != "os" {
        return json!({ "apply": apply, "keys": [], "skipped": "no OS key store is reachable in this build or session; the keys stay in their 0600 files" });
    }

    let Ok(dir) = crate::paths::data_dir().map(|dir| dir.join("keys")) else {
        return json!({ "apply": apply, "keys": [] });
    };

    migrate_dir(&dir, apply, &mut |name, secret| super::keychain::set(name, secret).map_err(|error| error.message), &|name| super::keychain::get(name))
}

/// The doctor's "where do your keys live" row (P0-2).
///
/// The Provider Hub's promise is that a key goes to the OS keychain and never to disk. This is the one row
/// that says whether that promise is being kept on *this* machine, and, when it is not, how many keys are
/// sitting in the protected-file fallback: `keychain::backend()` answers with the store that is actually in
/// use, so the row cannot disagree with where `set`/`get` really write.
pub fn doctor_row() -> Value {
    let fallback = crate::paths::data_dir()
        .ok()
        .map(|dir| dir.join("keys"))
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map(|entries| entries.flatten().filter(|entry| entry.path().is_file()).count())
        .unwrap_or(0);

    if super::keychain::backend() == "os" {
        json!({
            "id": "secrets",
            "label": "API keys",
            "state": "ok",
            "detail": if fallback == 0 {
                "OS keychain".to_string()
            } else {
                /* The store works and files are still there: `sdcd keys migrate --apply` is the single step. */
                format!("OS keychain · {fallback} still in the fallback file")
            },
        })
    } else {
        json!({
            "id": "secrets",
            "label": "API keys",
            "state": "warn",
            "detail": format!("{fallback} in a protected file - this computer has no key store SDC can reach"),
        })
    }
}

/// `sdcd keys migrate [--apply]` and `sdcd keys stray [DIR]` - the two questions P0-2 asks, on the command line
/// so a person can see the answer without a window. Without `--apply` nothing is written or deleted: the first
/// run reports what *would* move, which is the confirmation the plan requires.
pub fn main(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("migrate") => {
            let apply = args.iter().any(|arg| arg == "--apply");
            let report = migrate(apply);

            println!("{}", serde_json::to_string_pretty(&report).unwrap_or_else(|_| report.to_string()));

            /* A key the store did not give back is still a plaintext file; the person should not read "0". */
            match report["keys"].as_array().is_some_and(|keys| keys.iter().any(|row| row["state"] == "failed")) {
                true => 1,
                false => 0,
            }
        }
        Some("stray") => {
            let root = args.get(1).map(PathBuf::from).or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("."));
            let files = stray_keys(&root);

            println!("{}", serde_json::to_string_pretty(&json!({ "root": root.display().to_string(), "files": files })).unwrap_or_default());

            /* Exit 2 means "found something", so a hook or a shell can act on it without reading JSON. */
            match files.is_empty() {
                true => 0,
                false => 2,
            }
        }
        _ => {
            eprintln!("usage: sdcd keys migrate [--apply] | sdcd keys stray [DIR]");
            3
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if out.len() >= MAX_FILES {
            return;
        }

        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with("._") {
                walk(&path, out);
            }
        } else if !name.starts_with("._") && entry.metadata().is_ok_and(|meta| meta.len() <= MAX_FILE) {
            out.push(path);
        }
    }
}

/// Files under `root` that hold something shaped like a key: `{path, rule}` only, relative to `root`.
pub fn stray_keys(root: &Path) -> Vec<Value> {
    let mut files = Vec::new();
    let mut found = Vec::new();

    walk(root, &mut files);

    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");

        if let Some(first) = crate::trust::scan::secrets_in_text(&relative, &text).first() {
            found.push(json!({ "path": relative, "rule": first["rule"], "line": first["line"] }));
        }
    }

    found.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const KEY: &str = "sk-proj-abcdefghijklmnopqrstuvwx1234";

    #[test]
    fn a_key_is_deleted_only_after_the_store_gives_it_back() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(dir.path().join("sdc.provider.good"), format!("{KEY}\n")).unwrap();
        std::fs::write(dir.path().join("sdc.provider.lossy"), "another-secret-value").unwrap();
        std::fs::write(dir.path().join("sdc.provider.blank"), "  \n").unwrap();

        let store = RefCell::new(HashMap::new());
        let report = migrate_dir(
            dir.path(),
            true,
            &mut |name, secret| {
                /* a store that silently keeps the wrong value for one key */
                store.borrow_mut().insert(name.to_string(), if name.ends_with("lossy") { "truncated".to_string() } else { secret.to_string() });
                Ok(())
            },
            &|name| store.borrow().get(name).cloned(),
        );
        let state = |name: &str| report["keys"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap()["state"].as_str().unwrap().to_string();

        assert_eq!(state("sdc.provider.good"), "moved");
        assert!(!dir.path().join("sdc.provider.good").exists(), "plaintext gone after a verified move");
        assert_eq!(store.borrow()["sdc.provider.good"], KEY);
        assert_eq!(state("sdc.provider.lossy"), "failed");
        assert!(dir.path().join("sdc.provider.lossy").exists(), "a key the store did not give back stays on disk");
        assert_eq!(state("sdc.provider.blank"), "empty");
    }

    #[test]
    fn without_confirmation_nothing_moves() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(dir.path().join("sdc.provider.a"), KEY).unwrap();

        let report = migrate_dir(dir.path(), false, &mut |_, _| panic!("must not write"), &|_| None);

        assert_eq!(report["keys"][0]["state"], "would_move");
        assert!(dir.path().join("sdc.provider.a").exists());
    }

    #[test]
    fn a_key_file_beside_the_code_is_reported_by_path_never_by_value() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(dir.path().join("keys.txt"), format!("deepseek: {KEY}\n")).unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
        std::fs::write(dir.path().join("node_modules/x/leak.txt"), KEY).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "nothing secret here").unwrap();

        let found = stray_keys(dir.path());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["path"], "keys.txt");
        assert!(!found[0].to_string().contains(KEY), "the report never carries the key");
    }
}
