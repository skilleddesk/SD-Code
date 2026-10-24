//! The vault: content-addressed files under `.sdc/vault/` that NAVARATNA keeps outside the model's context - the whole prompt and its
//! unit map (N2-1), the full log behind an error slice (N3-3), the sections of a long document (N4-4). A model asks for a part of it
//! by id (`spec_read`); nothing in it is ever summarised or dropped.
//!
//! This is the small version of MEGHNA's M7-2 Spec Vault, which the code does not have: only what the NAVARATNA tasks need.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub const DIR: &str = ".sdc/vault";

/// The id of a content: the first 16 hex characters of its SHA-256 (the whole hash is kept in the file name).
pub fn id_of(content: &[u8]) -> String {
    hex::encode(Sha256::digest(content))[..16].to_string()
}

fn kind_ok(kind: &str) -> bool {
    !kind.is_empty() && kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn path_of(root: &Path, kind: &str, id: &str) -> PathBuf {
    root.join(DIR).join(kind).join(format!("{id}.txt"))
}

/// Stores `content` under `kind` (`spec`, `log`, `section`, ...) and answers its id. The same content is stored once.
pub fn put(root: &Path, kind: &str, content: &str) -> Result<String, String> {
    if !kind_ok(kind) {
        return Err(format!("not a vault kind: {kind}"));
    }

    let id = id_of(content.as_bytes());
    let path = path_of(root, kind, &id);

    if path.exists() {
        return Ok(id);
    }

    std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|error| error.to_string())?;

    let temporary = path.with_extension("tmp");

    std::fs::write(&temporary, content).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    Ok(id)
}

/// The content stored under `kind` and `id`, in full.
pub fn get(root: &Path, kind: &str, id: &str) -> Option<String> {
    if !kind_ok(kind) || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    std::fs::read_to_string(path_of(root, kind, id)).ok()
}

/// `spec_read`: lines `from..=to` (1-based) of a stored text - how a model fetches one part of a big prompt or log.
pub fn read_lines(root: &Path, kind: &str, id: &str, from: usize, to: usize) -> Option<String> {
    let text = get(root, kind, id)?;

    Some(text.lines().enumerate().filter(|(n, _)| (from.max(1)..=to).contains(&(n + 1))).map(|(_, line)| line).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_goes_in_comes_out_whole_and_is_stored_once() {
        let dir = tempfile::tempdir().unwrap();
        let a = put(dir.path(), "log", "line1\nline2\nline3\n").unwrap();
        let b = put(dir.path(), "log", "line1\nline2\nline3\n").unwrap();

        assert_eq!(a, b);
        assert_eq!(get(dir.path(), "log", &a).unwrap(), "line1\nline2\nline3\n");
        assert_eq!(read_lines(dir.path(), "log", &a, 2, 3).unwrap(), "line2\nline3");
        assert_eq!(get(dir.path(), "spec", &a), None, "another kind is another place");
    }

    #[test]
    fn a_kind_or_id_that_could_leave_the_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();

        assert!(put(dir.path(), "../x", "a").is_err());
        assert_eq!(get(dir.path(), "log", "../../etc/passwd"), None);
    }
}
