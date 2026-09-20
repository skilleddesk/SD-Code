//! Safe file operations (master spec sections 5.4 and 17.2).
//!
//! Every read and every write an engine asks for goes through this module, and it does two things:
//!
//! * **blocked patterns.** `.env` (and `.env.*`), `*.pem`, `*.key`, `id_rsa`, `id_ed25519`,
//!   `credentials`, `.npmrc`, `.netrc` and the daemon's own data directory are never read and never
//!   written - not "warned about", refused. It is the same list the Safety tab of spec section 9.11
//!   shows, in one function here, so the two cannot drift.
//! * **hashing.** Every read and write returns the file's SHA-256, which is the `filesHash` a
//!   checkpoint stores and what makes "did this turn change anything?" answerable.

use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::sdcp::envelope::ErrorObject;

/// Exact file names that are never touched, compared lower-case.
pub const BLOCKED_EXACT: &[&str] = &[".env", ".npmrc", ".netrc", "id_rsa", "id_ed25519", "credentials"];

/// Suffixes that are never touched.
pub const BLOCKED_SUFFIXES: &[&str] = &[".pem", ".key", ".p12", ".pfx"];

/// Why a path is blocked, or `None` when it is fine. Separate from `guard` so a test can assert the
/// *reason*, which is what the error message shows a user.
pub fn blocked_reason(path: &Path) -> Option<String> {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_lowercase();

    if BLOCKED_EXACT.contains(&name.as_str()) || name.starts_with(".env") {
        return Some(format!("`{name}` holds secrets"));
    }

    if BLOCKED_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
        return Some(format!("`{name}` is a private key or certificate"));
    }

    if crate::paths::is_internal(path) {
        return Some("it is inside the daemon's own data directory".to_string());
    }

    None
}

/// Refuses a blocked path. Every fs method calls this first.
pub fn guard(path: &Path) -> Result<(), ErrorObject> {
    match blocked_reason(path) {
        Some(reason) => Err(ErrorObject::blocked(&format!(
            "{} was refused because {reason}",
            path.display()
        ))),
        None => Ok(()),
    }
}

/// The SHA-256 of some bytes, hex-encoded.
pub fn hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();

    hasher.update(bytes);

    hex::encode(hasher.finalize())
}

/// Reads a text file. Returns `(text, sha256)`.
pub fn read(path: &Path) -> Result<(String, String), ErrorObject> {
    guard(path)?;

    let bytes = std::fs::read(path)
        .map_err(|error| ErrorObject::not_found(format!("{}: {error}", path.display())))?;
    let text = String::from_utf8_lossy(&bytes).to_string();

    Ok((text, hash(&bytes)))
}

/// Writes a text file, creating its parent directories. Returns the new `sha256`.
pub fn write(path: &Path, text: &str) -> Result<String, ErrorObject> {
    guard(path)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ErrorObject::internal(format!("{}: {error}", parent.display())))?;
    }

    std::fs::write(path, text)
        .map_err(|error| ErrorObject::internal(format!("{}: {error}", path.display())))?;

    Ok(hash(text.as_bytes()))
}

/// `(size, dir, sha256)` for one path.
pub fn stat(path: &Path) -> Result<Value, ErrorObject> {
    guard(path)?;

    let metadata = std::fs::metadata(path)
        .map_err(|error| ErrorObject::not_found(format!("{}: {error}", path.display())))?;
    let sha256 = if metadata.is_file() { read(path)?.1 } else { String::new() };

    Ok(serde_json::json!({
        "path": path.display().to_string(),
        "size": metadata.len(),
        "dir": metadata.is_dir(),
        "sha256": sha256,
    }))
}

/// A directory listing, one level deep.
///
/// A blocked name is filtered out of the listing as well as out of a read: a listing that *shows*
/// `.env` but refuses to open it invites a support ticket, and "never send these to providers" is
/// about the name reaching a model as much as the contents.
pub fn list(path: &Path) -> Result<Vec<Value>, ErrorObject> {
    guard(path)?;

    let entries = std::fs::read_dir(path)
        .map_err(|error| ErrorObject::not_found(format!("{}: {error}", path.display())))?;
    let mut rows = Vec::new();

    for entry in entries.flatten() {
        let entry_path: PathBuf = entry.path();
        let metadata = entry.metadata().ok();

        if blocked_reason(&entry_path).is_some() {
            continue;
        }

        rows.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "path": entry_path.display().to_string(),
            "dir": metadata.as_ref().map(|meta| meta.is_dir()).unwrap_or(false),
            "size": metadata.as_ref().map(|meta| meta.len()).unwrap_or(0),
        }));
    }

    rows.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));

    Ok(rows)
}

/// A recursive text search, capped: at most `limit` hits and at most six levels down.
///
/// It is deliberately small. `ripgrep` is what the doctor checks for and what an engine should shell
/// out to for a real search; this exists so `fs.search` answers correctly on a box where `rg` is
/// missing - which is exactly the case the doctor's warn row is about.
pub fn search(
    root: &Path,
    query: &str,
    glob: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, ErrorObject> {
    guard(root)?;

    let mut hits = Vec::new();

    walk(root, query, glob, 6, limit, &mut hits);

    Ok(hits)
}

fn walk(root: &Path, query: &str, glob: Option<&str>, depth: usize, limit: usize, hits: &mut Vec<Value>) {
    if depth == 0 || hits.len() >= limit {
        return;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        if hits.len() >= limit {
            return;
        }

        let path = entry.path();

        if blocked_reason(&path).is_some() {
            continue;
        }

        if path.is_dir() {
            walk(&path, query, glob, depth - 1, limit, hits);
            continue;
        }

        if let Some(glob) = glob {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();

            if !name.contains(glob.trim_start_matches('*').trim_start_matches('.')) {
                continue;
            }
        }

        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };

        for (index, line) in text.lines().enumerate() {
            if line.contains(query) {
                hits.push(serde_json::json!({
                    "path": path.display().to_string(),
                    "line": index + 1,
                    "text": line.trim(),
                }));

                if hits.len() >= limit {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_secrets_and_the_daemons_own_directory() {
        assert!(blocked_reason(Path::new("/work/.env")).is_some());
        assert!(blocked_reason(Path::new("/work/.env.production")).is_some());
        assert!(blocked_reason(Path::new("/work/cert.pem")).is_some());
        assert!(blocked_reason(Path::new("/keys/id_rsa")).is_some());
        assert!(blocked_reason(Path::new("/work/src/auth.ts")).is_none());

        if let Ok(data) = crate::paths::data_dir() {
            assert!(blocked_reason(&data.join("sdc.db")).is_some());
        }
    }

    #[test]
    fn writes_reads_and_hashes_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("src/rate.ts");

        let written = write(&file, "export const limit = 5;").unwrap();
        let (text, read_back) = read(&file).unwrap();

        assert_eq!(text, "export const limit = 5;");
        assert_eq!(written, read_back);
        assert_eq!(written.len(), 64);
    }

    #[test]
    fn lists_and_searches_without_leaking_a_blocked_name() {
        let dir = tempfile::tempdir().unwrap();

        /* Written with `std::fs` on purpose: the guard refuses to *write* `.env`, and this test is
           about the listing hiding a blocked name that exists on disk. */
        std::fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        write(&dir.path().join("a.ts"), "const limiter = rateLimit();").unwrap();

        let listing = list(dir.path()).unwrap();
        let names: Vec<&str> = listing.iter().filter_map(|row| row["name"].as_str()).collect();

        assert_eq!(names, vec!["a.ts"]);

        let hits = search(dir.path(), "rateLimit", None, 10).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["line"], 1);
    }

    #[test]
    fn guard_reports_a_blocked_path_as_a_structured_error() {
        let error = guard(&crate::paths::data_dir().unwrap().join("sdc.db")).unwrap_err();

        assert_eq!(error.code, "blocked_path");
        assert!(!error.message.is_empty());
    }
}
