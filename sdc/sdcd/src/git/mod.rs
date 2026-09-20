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

/// The patch between a checkpoint and the working tree. Empty when nothing changed - which is the
/// honest answer for a turn that only read files.
pub fn diff(project_root: &Path, since: Option<&str>) -> Result<String, ErrorObject> {
    match since {
        Some(sha) => run(project_root, &["diff", sha]),
        None => run(project_root, &["diff", "HEAD"]),
    }
}

/// `(branch, dirty count)` for the shadow repository - `git.status`.
pub fn status(project_root: &Path) -> Result<(String, usize), ErrorObject> {
    let branch = run(project_root, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    let porcelain = run(project_root, &["status", "--porcelain"])?;
    let dirty = porcelain.lines().filter(|line| !line.trim().is_empty()).count();

    Ok((branch, dirty))
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

        let (_, dirty) = status(project.path()).unwrap();

        assert_eq!(dirty, 1);
        assert!(diff(project.path(), Some(&sha)).unwrap().contains("export const a = 2;"));
    }
}
