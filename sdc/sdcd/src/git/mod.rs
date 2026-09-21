//! The shadow git repository (master spec section 5.5 of the daemon plan).
//!
//! Every session gets a **worktree** of a repository the daemon owns, under
//! `<data>/git/<hash of the project root>`. The user's own `.git` is never touched, and never read:
//! this is the record the Time Machine of spec section 14 rewinds to.
//!
//! Why the CLI and not `libgit2`: the daemon needs five verbs (`init`, `add`, `commit`, `diff`,
//! `worktree`) and git is already on every machine the doctor checks for. A linked library would be
//! a second implementation of git's rules and a second build toolchain, which is a poor trade for
//! five commands - so `git` is spawned, and its exit code is checked rather than its stderr parsed
//! (principle P3).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::sdcp::envelope::ErrorObject;

/// Runs one git command against the shadow repository, with the *project* as its work tree.
///
/// `--git-dir` and `--work-tree` are the whole trick of a shadow repository: the objects and refs
/// live under `<data>/git/...`, the files being versioned stay exactly where the user put them, and
/// the project never grows a `.git` of its own. The user's own repository - if there is one - is
/// neither read nor written.
pub fn run(work_tree: &Path, args: &[&str]) -> Result<String, ErrorObject> {
    let shadow = ensure_repository(work_tree)?;
    let git_dir = shadow.join(".git");

    let output = Command::new("git")
        /* A commit needs an identity; the daemon's is not the user's, on purpose. */
        .args(["-c", "user.name=sdcd", "-c", "user.email=sdcd@localhost"])
        .arg(format!("--git-dir={}", git_dir.display()))
        .arg(format!("--work-tree={}", work_tree.display()))
        .args(args)
        .output()
        .map_err(|error| ErrorObject::internal(format!("git is not available: {error}")))?;

    if !output.status.success() {
        return Err(ErrorObject::internal(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Runs one git command *inside a repository directory* (no work tree) - what `init` and `worktree`
/// need.
fn run_here(cwd: &Path, args: &[&str]) -> Result<String, ErrorObject> {
    let output = Command::new("git")
        .args(["-c", "user.name=sdcd", "-c", "user.email=sdcd@localhost"])
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| ErrorObject::internal(format!("git is not available: {error}")))?;

    if !output.status.success() {
        return Err(ErrorObject::internal(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// The shadow repository for a project root: `<data>/git/<hex of the root's hash>`.
pub fn shadow_path(project_root: &Path) -> Result<PathBuf, ErrorObject> {
    let digest = crate::fs::hash(project_root.display().to_string().as_bytes());
    let path = crate::paths::shadow_git_dir()
        .map_err(ErrorObject::internal)?
        .join(&digest[..16]);

    Ok(path)
}

/// Creates (or opens) the shadow repository and returns its path.
pub fn ensure_repository(project_root: &Path) -> Result<PathBuf, ErrorObject> {
    let shadow = shadow_path(project_root)?;

    if !shadow.join(".git").exists() {
        std::fs::create_dir_all(&shadow).map_err(ErrorObject::internal)?;
        run_here(&shadow, &["init", "--quiet"])?;
        run_here(&shadow, &["commit", "--allow-empty", "--quiet", "-m", "sdcd: shadow repository"])?;
    }

    Ok(shadow)
}

/// A checkpoint: everything staged, one commit, and the commit's sha - which is what
/// `git.checkpoint` answers with and what a `CheckpointSaved` event records.
pub fn checkpoint(project_root: &Path, message: &str) -> Result<String, ErrorObject> {
    run(project_root, &["add", "-A"])?;
    run(project_root, &["commit", "--quiet", "--allow-empty", "-m", message])?;

    Ok(run(project_root, &["rev-parse", "HEAD"])?.trim().to_string())
}

/// Does the project have a git repository **of its own** (here or above it)?
///
/// `run_here` starts in the directory with no `--git-dir`, so git discovers the repository the way a person's
/// shell would - including the case where the project is a subdirectory of a repository, which is how most
/// projects are actually laid out.
fn has_own_repository(project_root: &Path) -> bool {
    run_here(project_root, &["rev-parse", "--git-dir"]).is_ok()
}

/// `(branch, dirty count)` for **the project's own repository** - `git.status`.
///
/// It used to answer for the *shadow* repository, and that was a lie of exactly the kind this build keeps
/// removing: `run` takes a `--git-dir` and a `--work-tree`, the shadow's `git init` leaves it on `master`, so
/// a project on `main` was told `master` - a branch name its owner had never seen, about a repository they had
/// never heard of, on a badge that asks "which branch is my project on?". The dirty count was the shadow's
/// index rather than the project's working tree, and a folder that is not a repository at all got a confident
/// `master · clean`.
///
/// So: the project itself, and **`("", 0)` when it is not a repository** - the window draws no badge for an
/// empty branch name, which is the honest picture of a folder git knows nothing about.
pub fn status(project_root: &Path) -> Result<(String, usize), ErrorObject> {
    if !has_own_repository(project_root) {
        return Ok((String::new(), 0));
    }

    let branch = run_here(project_root, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    let porcelain = run_here(project_root, &["status", "--porcelain"])?;
    let dirty = porcelain.lines().filter(|line| !line.trim().is_empty()).count();

    Ok((branch, dirty))
}

/// The patch between a checkpoint and the working tree. Empty when nothing changed - which is the
/// honest answer for a turn that only read files.
///
/// The project's own repository when it has one, because `git diff HEAD` *there* is "what have I changed".
/// The shadow repository is the fallback, and it is the honest answer for a folder with no git at all: a
/// checkpoint's diff is a real record of the files a turn saw, and refusing to show it would hide the only
/// history that exists.
pub fn diff(project_root: &Path, since: Option<&str>) -> Result<String, ErrorObject> {
    if has_own_repository(project_root) {
        return match since {
            Some(sha) => run_here(project_root, &["diff", sha]),
            None => run_here(project_root, &["diff", "HEAD"]),
        };
    }

    match since {
        Some(sha) => run(project_root, &["diff", sha]),
        None => run(project_root, &["diff", "HEAD"]),
    }
}

/// A per-session worktree, so two chats on the same project cannot see each other's half-finished
/// edits (spec section 5.5: "worktree per session").
pub fn worktree(project_root: &Path, session_id: &str) -> Result<PathBuf, ErrorObject> {
    let shadow = ensure_repository(project_root)?;
    let path = shadow.join("worktrees").join(session_id);

    if path.join(".git").exists() {
        return Ok(path);
    }

    std::fs::create_dir_all(path.parent().unwrap_or(&shadow)).map_err(ErrorObject::internal)?;

    let branch = format!("session-{session_id}");
    let path_arg = path.display().to_string();

    let _ = run_here(&shadow, &["worktree", "add", "-B", &branch, &path_arg]);

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository to ask about, with one commit, on a branch of its own - so a test can tell "the project's
    /// branch" from "the shadow repository's" (which is `master`, because `ensure_repository` runs a bare
    /// `git init`).
    fn repository(directory: &Path) {
        run_here(directory, &["init", "-b", "probe-branch", "--quiet"]).unwrap();
        std::fs::write(directory.join("a.txt"), "one\n").unwrap();
        run_here(directory, &["add", "-A"]).unwrap();
        run_here(directory, &["commit", "--quiet", "-m", "first"]).unwrap();
    }

    /// 0.7.9: `git.status` answers for **the project's own** repository.
    ///
    /// The bug this pins down: the old implementation ran `git rev-parse --abbrev-ref HEAD` through `run`,
    /// which passes the *shadow* repository's `--git-dir`. A project on `main` was told `master` - the shadow's
    /// branch, created by a bare `git init` - and a folder that is not a repository at all got the same
    /// confident `master · clean`.
    #[test]
    fn status_reports_the_projects_own_branch_and_respects_a_folder_without_git() {
        let dir = tempfile::tempdir().unwrap();

        /* Not a repository at all: an empty branch name, which the window draws as no badge. */
        let (branch, dirty) = status(dir.path()).unwrap();

        assert_eq!(branch, "");
        assert_eq!(dirty, 0);

        repository(dir.path());

        let (branch, dirty) = status(dir.path()).unwrap();

        assert_eq!(branch, "probe-branch");
        assert_eq!(dirty, 0);

        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();

        let (_, dirty) = status(dir.path()).unwrap();

        assert_eq!(dirty, 1, "the working tree, not the shadow's index");
    }

    /// And `get.diff` is the project's diff when there is a project repository to diff.
    #[test]
    fn diff_comes_from_the_projects_repository_when_it_has_one() {
        let dir = tempfile::tempdir().unwrap();

        repository(dir.path());
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();

        let patch = diff(dir.path(), None).unwrap();

        assert!(patch.contains("+two"), "the patch must be the project's change: {patch}");
    }

    /// These tests need `git` on `PATH`; the doctor's row says so, and a machine without git skips
    /// them rather than failing the build.
    fn git_available() -> bool {
        Command::new("git").arg("--version").output().map(|output| output.status.success()).unwrap_or(false)
    }

    #[test]
    fn checkpoints_a_shadow_repository_outside_the_project() {
        if !git_available() {
            return;
        }

        let project = tempfile::tempdir().unwrap();

        std::fs::write(project.path().join("a.ts"), "export const a = 1;").unwrap();

        let sha = checkpoint(project.path(), "sdcd: first").unwrap();

        assert_eq!(sha.len(), 40);
        assert!(shadow_path(project.path()).unwrap().join(".git").exists());
        /* The project itself must not have become a repository. */
        assert!(!project.path().join(".git").exists());

        std::fs::write(project.path().join("a.ts"), "export const a = 2;").unwrap();

        /* A folder with no repository **of its own** has no branch and no changed count (0.7.9) - the window
           draws no badge, instead of a `master · 1 changed` badge about the shadow repository, whose branch
           name nobody chose. The shadow still holds the checkpoint, and `diff` still falls back to it: that is
           the only history a non-git folder has, and hiding it would hide the checkpoints themselves. */
        let (branch, dirty) = status(project.path()).unwrap();

        assert_eq!(branch, "");
        assert_eq!(dirty, 0);
        assert!(diff(project.path(), Some(&sha)).unwrap().contains("export const a = 2;"));
    }
}
