//! **First-open secret sweep (Phase 0.4).** When a folder is opened for the first time, SDC reads it once - without any
//! model - and tells the person which files already hold what looks like a secret, with a one-click way to move a value
//! into the OS keychain. It never prints a value (a finding is a file, a line and a kind) and it never sends anything
//! anywhere; the answer goes only to the window.
//!
//! "Move to keychain" stores the value in the OS keychain under a name SDC shows, and replaces *only that value* in the file
//! with a marker. It is not a rotation: a key that was in a file (or in git history) may have been read already, and the
//! answer says so.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

use crate::trust::secretguard;

const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".next", ".venv", "venv", "vendor", "__pycache__", ".sdc", ".idea", ".gradle", "Pods", ".pnpm-store"];
const SKIP_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "ico", "pdf", "zip", "gz", "tar", "woff", "woff2", "ttf", "otf", "mp3", "mp4", "mov", "wasm", "exe", "dll", "so", "dylib", "bin", "lock", "svg", "map"];
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_FILES: usize = 6000;
const MAX_FINDINGS: usize = 200;

/// The marker a moved value leaves in its file.
pub const MARKER: &str = "__MOVED_TO_SDC_KEYCHAIN__";

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub what: String,
    /// The file's name alone says it holds a secret (and no value was recognised).
    pub by_name: bool,
}

#[derive(Debug, Default)]
pub struct Sweep {
    pub findings: Vec<Finding>,
    pub files: usize,
    pub truncated: bool,
}

fn skipped_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| SKIP_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

fn walk(root: &Path, dir: &Path, sweep: &mut Sweep) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();

    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if sweep.files >= MAX_FILES || sweep.findings.len() >= MAX_FINDINGS {
            sweep.truncated = true;

            return;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with("._") {
            continue;
        }

        let Ok(kind) = entry.file_type() else {
            continue;
        };

        if kind.is_symlink() {
            continue;
        }

        if kind.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &path, sweep);
            }

            continue;
        }

        if !kind.is_file() || skipped_file(&path) {
            continue;
        }

        sweep.files += 1;

        let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let by_name = secretguard::sensitive_name(&relative).is_some();
        let small = entry.metadata().map(|meta| meta.len() <= MAX_FILE_BYTES).unwrap_or(false);
        let text = if small { std::fs::read(&path).ok().filter(|bytes| !bytes.contains(&0)).map(|bytes| String::from_utf8_lossy(&bytes).to_string()) } else { None };
        let found = text.as_deref().map(|text| secretguard::scan(&relative, text)).unwrap_or_default();

        for item in &found {
            sweep.findings.push(Finding { file: relative.clone(), line: item.line, what: item.what.clone(), by_name: false });
        }

        if found.is_empty() && by_name {
            sweep.findings.push(Finding { file: relative, line: 0, what: "the file name says it holds a secret".into(), by_name: true });
        }
    }
}

/// Reads the folder once. No value is kept in the answer.
pub fn run(root: &Path) -> Sweep {
    let mut sweep = Sweep::default();

    walk(root, root, &mut sweep);
    sweep
}

pub fn to_json(sweep: &Sweep) -> Value {
    json!({
        "findings": sweep.findings.iter().map(|finding| json!({ "file": finding.file, "line": finding.line, "what": finding.what, "byName": finding.by_name })).collect::<Vec<_>>(),
        "files": sweep.files,
        "truncated": sweep.truncated,
    })
}

/// A path inside `root`, or an error: no `..`, no absolute path, no symlink out.
fn inside(root: &Path, file: &str) -> Result<PathBuf, String> {
    let relative = Path::new(file);

    if relative.is_absolute() || relative.components().any(|part| matches!(part, Component::ParentDir | Component::Prefix(_))) {
        return Err("that path is outside the folder".into());
    }

    let joined = root.join(relative);
    let real = joined.canonicalize().map_err(|error| error.to_string())?;
    let base = root.canonicalize().map_err(|error| error.to_string())?;

    if real.starts_with(&base) {
        Ok(real)
    } else {
        Err("that path is outside the folder".into())
    }
}

/// The keychain name for a moved value: stable per folder, file and line, never the value.
pub fn keychain_name(root: &Path, file: &str, line: usize) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    let short: String = digest.iter().take(4).map(|byte| format!("{byte:02x}")).collect();

    format!("sdc.moved.{short}.{}:{line}", file.replace('/', "."))
}

/// Moves the value on `line` of `file` into the keychain (through `store`) and leaves [`MARKER`] in its place.
/// Answers the keychain name, and what the person should still do. `store` is the keychain's `set`, passed in so the
/// tests do not touch the real one.
pub fn move_to_keychain(root: &Path, file: &str, line: usize, store: impl FnOnce(&str, &str) -> Result<(), String>) -> Result<Value, String> {
    let path = inside(root, file)?;
    let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let found = secretguard::scan(file, &text);
    let target = found.iter().find(|item| item.line == line).ok_or("no secret is on that line any more (the file changed?)")?;
    let value = &text[target.start..target.end];

    if value == MARKER {
        return Err("that value is already moved".into());
    }

    let name = keychain_name(root, file, line);

    store(&name, value)?;

    /* Written beside the file and renamed over it: a crash leaves the old file or the new one, never half of one. */
    let mut moved = String::with_capacity(text.len());

    moved.push_str(&text[..target.start]);
    moved.push_str(MARKER);
    moved.push_str(&text[target.end..]);

    let partial = path.with_extension("sdc-partial");

    std::fs::write(&partial, moved).map_err(|error| error.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if let Ok(meta) = std::fs::metadata(&path) {
            let _ = std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(meta.permissions().mode()));
        }
    }

    std::fs::rename(&partial, &path).map_err(|error| error.to_string())?;

    Ok(json!({
        "name": name,
        "file": file,
        "line": line,
        "note": "The value is in your OS keychain now and gone from the file. It is not rotated: if this file was ever committed, sent, or shared, treat the key as leaked and regenerate it on its provider's site. The app that needs it must read it from the keychain or an environment variable.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fake values only.
    const FAKE: &str = "FAKEvalue9Zq4Xw7Lm2Pt8";

    fn folder(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-sweep-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_sweep_finds_secrets_by_content_and_by_name_without_repeating_a_value() {
        let root = folder("find");

        std::fs::write(root.join(".env"), format!("API_KEY={FAKE}\nMODE=dev\n")).unwrap();
        std::fs::write(root.join("pass.txt"), "x").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() { println!(\"hi\"); }\n").unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("node_modules/x/leak.env"), format!("TOKEN={FAKE}\n")).unwrap();

        let sweep = run(&root);
        let answer = to_json(&sweep).to_string();

        assert!(sweep.findings.iter().any(|finding| finding.file == ".env" && finding.line == 1 && !finding.by_name), "{answer}");
        assert!(sweep.findings.iter().any(|finding| finding.file == "pass.txt" && finding.by_name), "{answer}");
        assert!(!sweep.findings.iter().any(|finding| finding.file == "main.rs" || finding.file.contains("node_modules")), "{answer}");
        assert!(!answer.contains(FAKE), "a value is never in the answer");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn moving_a_value_stores_it_and_leaves_only_a_marker_in_the_file() {
        let root = folder("move");

        std::fs::write(root.join(".env"), format!("A=1\nAPI_KEY={FAKE}\nB=2\n")).unwrap();

        let mut stored = None;
        let answer = move_to_keychain(&root, ".env", 2, |name, value| {
            stored = Some((name.to_string(), value.to_string()));
            Ok(())
        })
        .unwrap();
        let after = std::fs::read_to_string(root.join(".env")).unwrap();

        assert_eq!(stored.as_ref().unwrap().1, FAKE);
        assert_eq!(answer["name"].as_str().unwrap(), stored.unwrap().0);
        assert_eq!(after, format!("A=1\nAPI_KEY={MARKER}\nB=2\n"));
        assert!(!answer.to_string().contains(FAKE));
        assert!(move_to_keychain(&root, ".env", 2, |_, _| Ok(())).is_err(), "a second move finds nothing to move");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_keychain_leaves_the_file_exactly_as_it_was() {
        let root = folder("fail");
        let text = format!("API_KEY={FAKE}\n");

        std::fs::write(root.join(".env"), &text).unwrap();

        assert!(move_to_keychain(&root, ".env", 1, |_, _| Err("keychain locked".into())).is_err());
        assert_eq!(std::fs::read_to_string(root.join(".env")).unwrap(), text);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_path_outside_the_folder_is_refused() {
        let root = folder("outside");

        for file in ["../x.env", "/etc/passwd", "a/../../x"] {
            assert!(move_to_keychain(&root, file, 1, |_, _| Ok(())).is_err(), "{file}");
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}
