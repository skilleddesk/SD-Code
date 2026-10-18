//! **The Step Transaction (P0-14, switch `edit.transaction`)**: a step's edits reach the real tree all at once or not at
//! all.
//!
//! A step that changes five files and fails on the fourth must not leave the project with three of them changed. So the
//! edits are made in a **shadow copy** of the project; Verify runs in the shadow (in the sandbox); and only if it is
//! green are they carried across to the real tree - by a protocol whose every interruption, including `kill -9` at any
//! instant, leaves either the old tree or the new one and never a mixture:
//!
//! 1. **Check the base.** Each file the step touches must still be what the step started from. If the person (or another
//!    step) changed it meanwhile, the transaction fails closed with [`TxError::Conflict`]: it will not overwrite work it
//!    never saw.
//! 2. **Write the journal first.** `journal.json` lists every file with a copy of its *pre-image* in `pre/`, and is
//!    `fsync`'d before the real tree is touched: from this moment recovery has everything it needs to undo.
//! 3. **Apply.** Each file is written beside its destination and `rename`d into place (atomic per file), or removed.
//! 4. **Mark it done.** The journal's state becomes `committed`, and the shadow is removed.
//!
//! [`recover`] runs at start and finishes whatever a crash left: a `committing` journal is **rolled back** from the
//! pre-images (even if every file had been applied - the step is then simply to do again, which is safe, where rolling
//! forward would trust a state nobody verified), a `committed` one is only cleaned up, and a transaction that never got
//! as far as a journal is just a shadow to delete.
//!
//! Each successful commit is a Time Machine point ([`commit_with_point`]).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where transactions live, under the project.
pub const DIR: &str = ".sdc/tx";
/// Folders that are linked into the shadow rather than copied: dependencies and build output a check needs and a step
/// does not change. (A link, so a check that writes `target/` writes the real one - the sandbox's rule is the project.)
const LINKED: [&str; 6] = ["node_modules", "target", ".venv", "venv", "vendor", "__pycache__"];
/// Never copied: the shadow's own bookkeeping and version control.
const SKIPPED: [&str; 3] = [".git", ".sdc", "._"];
const MAX_COPY_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum TxError {
    /// The path is absolute, climbs out of the project, or is SDC's own.
    BadPath(String),
    /// A file changed since the step began; nothing was written.
    Conflict(Vec<String>),
    Io(String),
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxError::BadPath(path) => write!(f, "not a path inside the project: {path}"),
            TxError::Conflict(paths) => write!(f, "changed since this step began, so nothing was written: {}", paths.join(", ")),
            TxError::Io(message) => write!(f, "{message}"),
        }
    }
}

impl From<std::io::Error> for TxError {
    fn from(error: std::io::Error) -> Self {
        TxError::Io(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Write(Vec<u8>),
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Committed,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub id: String,
    pub outcome: Outcome,
    pub changed: Vec<String>,
    /// The Time Machine point made for a commit (a checkpoint id), when the caller made one.
    pub point: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    path: String,
    /// The hash of the file before the step, `None` when the step creates it.
    pre: Option<String>,
    /// The hash after, `None` when the step deletes it.
    post: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    id: String,
    state: String,
    entries: Vec<Entry>,
}

pub struct Transaction {
    root: PathBuf,
    id: String,
    dir: PathBuf,
    shadow: PathBuf,
    /// The hash of each touched file in the real tree when the step began (`None`: it did not exist).
    base: BTreeMap<String, Option<String>>,
    edits: BTreeMap<String, Edit>,
    /// A light transaction has no shadow copy: the edits are held in memory and nothing runs in a shadow (the Step Runtime's own
    /// per-edit gate is the verifier). The commit protocol - base check, journal, atomic renames, recovery - is the same.
    light: bool,
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn hash_of(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| hash(&bytes))
}

/// A project-relative path with `/` separators, or an error. No `..`, no root, nothing under `.sdc` or `.git`.
fn clean(path: &str) -> Result<String, TxError> {
    let candidate = Path::new(path);
    let mut parts = Vec::new();

    for component in candidate.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return Err(TxError::BadPath(path.to_string())),
        }
    }

    if parts.is_empty() || parts.iter().any(|part| part == ".sdc" || part == ".git") {
        return Err(TxError::BadPath(path.to_string()));
    }

    Ok(parts.join("/"))
}

fn sync_dir(path: &Path) {
    if let Ok(dir) = std::fs::File::open(path) {
        let _ = dir.sync_all();
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;

    file.write_all(bytes)?;
    file.sync_all()
}

/// The shadow: a real copy of the project's sources, with the dependency folders linked in.
fn copy_shadow(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;

    for entry in std::fs::read_dir(from)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        if SKIPPED.iter().any(|skipped| name.starts_with(skipped)) {
            continue;
        }

        let (source, dest) = (entry.path(), to.join(&name));
        let kind = entry.file_type()?;

        if kind.is_dir() && LINKED.contains(&name.as_str()) {
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(&source, &dest);
            #[cfg(windows)]
            let _ = std::os::windows::fs::symlink_dir(&source, &dest);
        } else if kind.is_dir() {
            copy_shadow(&source, &dest)?;
        } else if kind.is_file() && entry.metadata().is_ok_and(|meta| meta.len() <= MAX_COPY_BYTES) {
            std::fs::copy(&source, &dest)?;
        }
    }

    Ok(())
}

impl Transaction {
    /// Starts a transaction: makes the shadow copy. `id` names it (a step id).
    pub fn begin(root: &Path, id: &str) -> Result<Self, TxError> {
        let root = std::fs::canonicalize(root)?;
        let safe_id: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
        let dir = root.join(DIR).join(&safe_id);

        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let shadow = dir.join("shadow");

        copy_shadow(&root, &shadow)?;

        Ok(Self { root, id: safe_id, dir, shadow, base: BTreeMap::new(), edits: BTreeMap::new(), light: false })
    }

    /// A transaction without the shadow copy (P1-2): for a step whose verifier is a check of the edits themselves - the syntax
    /// gate - and not a command run in a copy of the project. It costs no copy of the tree, which is what lets every agent edit
    /// go through a transaction.
    pub fn begin_light(root: &Path, id: &str) -> Result<Self, TxError> {
        let root = std::fs::canonicalize(root)?;
        let safe_id: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
        let dir = root.join(DIR).join(&safe_id);

        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        Ok(Self { root, id: safe_id, shadow: dir.join("shadow"), dir, base: BTreeMap::new(), edits: BTreeMap::new(), light: true })
    }

    /// Where Verify runs: the project as it will be if the step is committed.
    pub fn shadow(&self) -> &Path {
        &self.shadow
    }

    fn remember(&mut self, rel: &str) {
        if !self.base.contains_key(rel) {
            self.base.insert(rel.to_string(), hash_of(&self.root.join(rel)));
        }
    }

    /// Writes a file in the shadow (and records it for the commit).
    pub fn write(&mut self, path: &str, content: &[u8]) -> Result<(), TxError> {
        let rel = clean(path)?;
        let target = self.shadow.join(&rel);

        self.remember(&rel);

        if !self.light {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(target, content)?;
        }

        self.edits.insert(rel, Edit::Write(content.to_vec()));

        Ok(())
    }

    /// Removes a file in the shadow (and records it for the commit).
    pub fn delete(&mut self, path: &str) -> Result<(), TxError> {
        let rel = clean(path)?;

        self.remember(&rel);

        if !self.light {
            let _ = std::fs::remove_file(self.shadow.join(&rel));
        }

        self.edits.insert(rel, Edit::Delete);

        Ok(())
    }

    pub fn changed(&self) -> Vec<String> {
        self.edits.keys().cloned().collect()
    }

    /// Ends the transaction. `verified` is Verify's verdict on the shadow: only `true` reaches the real tree.
    pub fn finish(self, verified: bool) -> Result<Receipt, TxError> {
        self.finish_with(verified, |_| None)
    }

    /// As [`Transaction::finish`], and `point` is called after a successful commit to make the Time Machine point; what it
    /// returns is carried on the receipt.
    pub fn finish_with(self, verified: bool, point: impl FnOnce(&Receipt) -> Option<String>) -> Result<Receipt, TxError> {
        let changed = self.changed();

        if !verified || self.edits.is_empty() {
            let _ = std::fs::remove_dir_all(&self.dir);

            return Ok(Receipt { id: self.id, outcome: Outcome::RolledBack, changed, point: None });
        }

        /* 1. The real files must still be what the step started from. */
        let conflicts: Vec<String> = self.base.iter().filter(|(rel, base)| hash_of(&self.root.join(rel.as_str())) != **base).map(|(rel, _)| rel.clone()).collect();

        if !conflicts.is_empty() {
            let _ = std::fs::remove_dir_all(&self.dir);

            return Err(TxError::Conflict(conflicts));
        }

        /* 2. The journal, with every pre-image, on disk before the tree is touched. */
        let pre_dir = self.dir.join("pre");

        std::fs::create_dir_all(&pre_dir)?;

        let mut entries = Vec::new();

        for (index, (rel, edit)) in self.edits.iter().enumerate() {
            let real = self.root.join(rel);
            let existing = std::fs::read(&real).ok();

            if let Some(bytes) = &existing {
                write_synced(&pre_dir.join(index.to_string()), bytes)?;
            }

            failpoint("pre-image", index);

            entries.push(Entry { path: rel.clone(), pre: existing.as_deref().map(hash), post: match edit {
                Edit::Write(bytes) => Some(hash(bytes)),
                Edit::Delete => None,
            } });
        }

        let mut journal = Journal { id: self.id.clone(), state: "committing".into(), entries };

        write_journal(&self.dir, &journal)?;
        sync_dir(&self.dir);
        failpoint("journal", 0);

        /* 3. Apply, one atomic rename at a time. */
        for (index, (rel, edit)) in self.edits.iter().enumerate() {
            let real = self.root.join(rel);

            match edit {
                Edit::Write(bytes) => {
                    if let Some(parent) = real.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    let beside = real.with_extension(format!("sdc-tx-{}", self.id));

                    write_synced(&beside, bytes)?;
                    std::fs::rename(&beside, &real)?;
                }
                Edit::Delete => {
                    let _ = std::fs::remove_file(&real);
                }
            }

            failpoint("apply", index + 1);
        }

        /* 4. Done. */
        journal.state = "committed".into();
        write_journal(&self.dir, &journal)?;
        failpoint("committed", 0);

        let mut receipt = Receipt { id: self.id.clone(), outcome: Outcome::Committed, changed, point: None };

        receipt.point = point(&receipt);

        let _ = std::fs::remove_dir_all(&self.dir);

        Ok(receipt)
    }
}

fn write_journal(dir: &Path, journal: &Journal) -> std::io::Result<()> {
    let temporary = dir.join("journal.json.tmp");

    write_synced(&temporary, &serde_json::to_vec_pretty(journal).unwrap_or_default())?;
    std::fs::rename(temporary, dir.join("journal.json"))?;
    sync_dir(dir);

    Ok(())
}

/// A deliberate crash, for the test that kills a commit: `SDC_TX_FAILPOINT=apply:3` aborts the process (no destructors, no
/// flushing - the same instant a `kill -9` would pick) after the third file has been applied. Inert without the variable.
fn failpoint(stage: &str, index: usize) {
    if let Ok(wanted) = std::env::var("SDC_TX_FAILPOINT") {
        if wanted == format!("{stage}:{index}") {
            std::process::abort();
        }
    }
}

/// What recovery did with one transaction.
#[derive(Debug, PartialEq, Eq)]
pub enum Recovered {
    /// A half-applied commit, undone from its pre-images: these paths are back as they were.
    RolledBack(Vec<String>),
    /// A commit that had finished; only its leftovers were removed.
    CleanedUp,
    /// A transaction that never reached a journal: only a shadow to delete.
    Discarded,
}

/// Finishes whatever a crash left under `root/.sdc/tx`. Safe to run when there is nothing to do; run it at start.
pub fn recover(root: &Path) -> Vec<(String, Recovered)> {
    let mut done = Vec::new();
    let Ok(entries) = std::fs::read_dir(root.join(DIR)) else {
        return done;
    };

    for entry in entries.flatten() {
        let dir = entry.path();

        if !dir.is_dir() {
            continue;
        }

        let id = entry.file_name().to_string_lossy().to_string();
        let journal: Option<Journal> = std::fs::read(dir.join("journal.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let result = match journal {
            None => Recovered::Discarded,
            Some(journal) if journal.state == "committed" => Recovered::CleanedUp,
            Some(journal) => {
                let mut restored = Vec::new();

                for (index, entry) in journal.entries.iter().enumerate() {
                    let real = root.join(&entry.path);

                    match &entry.pre {
                        /* It existed: put the old bytes back (atomically). */
                        Some(_) => {
                            if let Ok(bytes) = std::fs::read(dir.join("pre").join(index.to_string())) {
                                let beside = real.with_extension("sdc-tx-restore");

                                if write_synced(&beside, &bytes).is_ok() && std::fs::rename(&beside, &real).is_ok() {
                                    restored.push(entry.path.clone());
                                }
                            }
                        }
                        /* The step created it: remove it. */
                        None => {
                            if real.exists() && std::fs::remove_file(&real).is_ok() {
                                restored.push(entry.path.clone());
                            }
                        }
                    }
                }

                Recovered::RolledBack(restored)
            }
        };

        let _ = std::fs::remove_dir_all(&dir);

        done.push((id, result));
    }

    done
}

/// A commit that is also a Time Machine point: the project is checkpointed (the shadow git of `crate::git`) with the step
/// named in the message, and the checkpoint's id is on the receipt.
pub fn commit_with_point(tx: Transaction, verified: bool, label: &str) -> Result<Receipt, TxError> {
    let root = tx.root.clone();

    tx.finish_with(verified, |receipt| crate::git::checkpoint(&root, &format!("{label} ({} file(s), {})", receipt.changed.len(), receipt.id)).ok())
}

#[cfg(test)]
mod tests;
