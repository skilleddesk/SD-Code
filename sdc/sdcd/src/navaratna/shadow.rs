//! **N6-2 - the journal and the Git Shadow** (switch `nr.git_shadow`, off).
//!
//! Before a step, a hidden snapshot of the working tree is recorded under `refs/sdc/shadow/<step>` - made with a **private index file**, so the
//! user's branch, index and staged files are never touched and no visible commit appears. A failed step puts the tree back exactly (files the
//! step changed are restored, files it created are removed) and `git status` reads the same as before. A journal records each step as
//! `started`, `verified` or `done`; after a crash [`recover`] restores the tree of any step that was `started` and a restart skips every `done`
//! step. What a cloud model answered is stored keyed by `(step, target)`, so a retry replaces it and a restart never asks again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REF_PREFIX: &str = "refs/sdc/shadow/";
const DIR: &str = ".sdc/shadow";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum State {
    Started,
    Verified,
    Done,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Journal {
    pub steps: BTreeMap<String, State>,
}

/// Where the journal and the kept answers live: inside `.git` for a repository (so they are never part of the work tree being snapshotted
/// and restored), else under `.sdc/shadow`.
fn data_dir(root: &Path) -> PathBuf {
    match git(root, &["rev-parse", "--absolute-git-dir"], None) {
        Ok(dir) => PathBuf::from(dir).join("sdc-shadow"),
        Err(_) => root.join(DIR),
    }
}

fn journal_path(root: &Path) -> PathBuf {
    data_dir(root).join("journal.json")
}

impl Journal {
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(journal_path(root)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    /// Written to a temporary file and renamed: a crash leaves the old journal or the new one, never half of it.
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = journal_path(root);

        std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|error| error.to_string())?;

        let temporary = path.with_extension("tmp");

        std::fs::write(&temporary, serde_json::to_string_pretty(self).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|error| error.to_string())
    }
}

fn git(root: &Path, args: &[&str], index: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new("git");

    /* A snapshot must give back the very bytes it took. On Windows git would otherwise convert line endings on the way in and out (autocrlf), so a
       restored file would differ from the one that was there; the snapshot and the restore both run with the conversion off. */
    command.current_dir(root).args(["-c", "core.autocrlf=false", "-c", "core.safecrlf=false", "-c", "core.eol=lf"]).args(args).env("GIT_OPTIONAL_LOCKS", "0");

    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }

    let output = command.output().map_err(|error| format!("git could not be started: {error}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim_end_matches('\n').to_string())
    } else {
        Err(format!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&output.stderr).trim()))
    }
}

fn safe_step(step: &str) -> Result<(), String> {
    if !step.is_empty() && step.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && !step.contains("..") {
        Ok(())
    } else {
        Err(format!("not a step id: {step}"))
    }
}

/// A private index file inside `.git` (never the user's).
fn private_index(root: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = PathBuf::from(git(root, &["rev-parse", "--absolute-git-dir"], None)?);

    Ok(dir.join(format!("sdc-shadow-{name}-{}.index", std::process::id())))
}

/// The tree object of the working tree as it is now: tracked and untracked files that are not ignored.
fn worktree_tree(root: &Path) -> Result<String, String> {
    let index = private_index(root, "snap")?;
    let _ = std::fs::remove_file(&index);
    let result = git(root, &["add", "-A", "--", ".", ":(exclude).sdc"], Some(&index)).and_then(|_| git(root, &["write-tree"], Some(&index)));

    let _ = std::fs::remove_file(&index);
    result
}

/// Records the snapshot of a step. Answers the commit id that `refs/sdc/shadow/<step>` now points at.
pub fn snapshot(root: &Path, step: &str) -> Result<String, String> {
    safe_step(step)?;

    let tree = worktree_tree(root)?;
    let parent = git(root, &["rev-parse", "-q", "--verify", "HEAD"], None).ok();
    let mut args = vec!["-c", "user.name=sdc-shadow", "-c", "user.email=shadow@sdc.invalid", "commit-tree", &tree, "-m"];
    let message = format!("sdc shadow snapshot of step {step}");

    args.push(&message);

    if let Some(parent) = parent.as_deref() {
        args.push("-p");
        args.push(parent);
    }

    let commit = git(root, &args, None)?;

    git(root, &["update-ref", &format!("{REF_PREFIX}{step}"), &commit], None)?;
    Ok(commit)
}

/// Puts the working tree back to the step's snapshot: its files restored, the files created since removed. The user's index and branch are
/// not touched.
pub fn restore(root: &Path, step: &str) -> Result<(), String> {
    safe_step(step)?;

    let commit = git(root, &["rev-parse", "-q", "--verify", &format!("{REF_PREFIX}{step}")], None).map_err(|_| format!("no shadow snapshot for step {step}"))?;
    let wanted: Vec<String> = git(root, &["ls-tree", "-r", "--name-only", "-z", &commit], None)?.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect();
    let now_tree = worktree_tree(root)?;
    let present: Vec<String> = git(root, &["ls-tree", "-r", "--name-only", "-z", &now_tree], None)?.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect();
    let index = private_index(root, "restore")?;
    let _ = std::fs::remove_file(&index);
    let restored = git(root, &["read-tree", &commit], Some(&index)).and_then(|_| git(root, &["checkout-index", "-a", "-f"], Some(&index)));

    let _ = std::fs::remove_file(&index);
    restored?;

    for path in present.iter().filter(|path| !wanted.contains(path) && !path.starts_with(".sdc/")) {
        let target = root.join(path);

        if target.is_file() || target.is_symlink() {
            std::fs::remove_file(&target).map_err(|error| format!("{path}: {error}"))?;
        }
    }

    Ok(())
}

pub fn drop_snapshot(root: &Path, step: &str) {
    if safe_step(step).is_ok() {
        let _ = git(root, &["update-ref", "-d", &format!("{REF_PREFIX}{step}")], None);
    }
}

/// Is `root` inside a git work tree?
pub fn is_repository(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"], None).is_ok_and(|answer| answer == "true")
}

/* ---- the run ------------------------------------------------------------------------------------------------------------------------- */

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// The journal says the step is done: it was not run again.
    Skipped,
    Done,
    /// The step failed; the tree was restored.
    Failed(String),
}

/// Runs a step under the shadow: skip it if done, snapshot, journal `started`, run, then `verified` and `done` - or restore on failure.
/// With the switch off the body just runs.
pub fn run_step(root: &Path, step: &str, body: &mut dyn FnMut() -> Result<(), String>) -> Result<Ran, String> {
    if !crate::v3::on("nr.git_shadow") || !is_repository(root) {
        return Ok(match body() {
            Ok(()) => Ran::Done,
            Err(error) => Ran::Failed(error),
        });
    }

    let mut journal = Journal::load(root);

    if journal.steps.get(step) == Some(&State::Done) {
        return Ok(Ran::Skipped);
    }

    snapshot(root, step)?;
    journal.steps.insert(step.to_string(), State::Started);
    journal.save(root)?;

    match body() {
        Ok(()) => {
            journal.steps.insert(step.to_string(), State::Verified);
            journal.save(root)?;
            journal.steps.insert(step.to_string(), State::Done);
            journal.save(root)?;
            drop_snapshot(root, step);
            Ok(Ran::Done)
        }
        Err(error) => {
            restore(root, step)?;
            journal.steps.remove(step);
            journal.save(root)?;
            drop_snapshot(root, step);
            Ok(Ran::Failed(error))
        }
    }
}

/// After a crash: the tree of every step still `started` is put back and the step is forgotten (it will run again from a clean tree). A step
/// that was `verified` but not `done` is finished: its work is whole.
pub fn recover(root: &Path) -> Vec<String> {
    if !is_repository(root) {
        return Vec::new();
    }

    let mut journal = Journal::load(root);
    let mut touched = Vec::new();

    for (step, state) in journal.steps.clone() {
        match state {
            State::Started => {
                if restore(root, &step).is_ok() {
                    journal.steps.remove(&step);
                    drop_snapshot(root, &step);
                    touched.push(step);
                }
            }
            State::Verified => {
                journal.steps.insert(step.clone(), State::Done);
                drop_snapshot(root, &step);
                touched.push(step);
            }
            State::Done => {}
        }
    }

    let _ = journal.save(root);
    touched
}

/* ---- answers kept by (step, target) --------------------------------------------------------------------------------------------------- */

fn answer_path(root: &Path, step: &str, target: &str) -> PathBuf {
    data_dir(root).join("answers").join(step).join(format!("{}.txt", super::vault::id_of(target.as_bytes())))
}

/// Keeps what a model answered for `(step, target)`. Writing again replaces it - a retry never appends.
pub fn keep_answer(root: &Path, step: &str, target: &str, answer: &str) -> Result<(), String> {
    safe_step(step)?;

    let path = answer_path(root, step, target);

    std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|error| error.to_string())?;

    let temporary = path.with_extension("tmp");

    std::fs::write(&temporary, answer).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

/// What was kept for `(step, target)`, if anything: a restart uses it instead of calling the model again.
pub fn kept_answer(root: &Path, step: &str, target: &str) -> Option<String> {
    safe_step(step).ok()?;
    std::fs::read_to_string(answer_path(root, step, target)).ok()
}

#[cfg(test)]
mod tests;
