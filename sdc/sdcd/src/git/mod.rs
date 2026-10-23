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
    Shadow::of(work_tree)?.run(args)
}

/// One shadow repository, resolved **once** for a whole operation.
///
/// Every command of an operation (`add -A` and then `commit`, or `add -A` and then `read-tree`) has to run against the
/// same repository. Resolving it per command means that a data directory which is momentarily different - it comes from
/// the environment, and a thread that moves it moves it for everyone - stages the tree in one repository and commits it in
/// another: a fresh repository then commits an *empty* tree, and a restore from that empty commit wipes the project. So an
/// operation opens one [`Shadow`] and runs all of its commands through it.
pub struct Shadow {
    pub git_dir: PathBuf,
    pub work_tree: PathBuf,
}

impl Shadow {
    /// Opens (or creates) the shadow repository of `work_tree`.
    pub fn of(work_tree: &Path) -> Result<Self, ErrorObject> {
        /* 0.22: SDC installs Git itself on first start; until it is there, say so rather than run a stub. */
        if !crate::host::tools::has_working_git() {
            return Err(ErrorObject::internal("Git is still being installed by SDC (Settings → Environment shows its progress); checkpoints start working the moment it is done."));
        }

        Ok(Self { git_dir: ensure_repository(work_tree)?.join(".git"), work_tree: work_tree.to_path_buf() })
    }

    /// Runs one git command against this repository, with the project as its work tree.
    pub fn run(&self, args: &[&str]) -> Result<String, ErrorObject> {
        Ok(String::from_utf8_lossy(&self.run_raw(args, None)?).to_string())
    }

    /// The same, answering the raw bytes and optionally feeding git's standard input - what `update-index
    /// --index-info` needs, and what keeps a path that is not valid UTF-8 intact.
    pub fn run_raw(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, ErrorObject> {
        self.run_indexed(args, input, None)
    }

    /// `run_raw` against an index file of its own (`GIT_INDEX_FILE`) instead of the repository's: a scratch index is
    /// never contended for, and it starts empty, so reading it costs nothing however large the folder is.
    fn run_indexed(&self, args: &[&str], input: Option<&[u8]>, index: Option<&Path>) -> Result<Vec<u8>, ErrorObject> {
        use std::io::Write;
        use std::process::Stdio;

        let mut command = Command::new("git");

        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }

        let mut child = command
            /* A commit needs an identity; the daemon's is not the user's, on purpose. */
            .args(["-c", "user.name=sdcd", "-c", "user.email=sdcd@localhost"])
            /* A shadow repository is written to by this daemon and nobody else, so it never needs a background
               `gc` (which would stall the next checkpoint on a slow disk), and a folder that is walked again
               and again is walked faster with git's untracked cache. */
            .args(["-c", "gc.auto=0", "-c", "core.untrackedCache=true", "-c", "core.preloadIndex=true", "-c", "core.quotePath=false"])
            .arg(format!("--git-dir={}", self.git_dir.display()))
            .arg(format!("--work-tree={}", self.work_tree.display()))
            .args(args)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| ErrorObject::internal(format!("git is not available: {error}")))?;

        if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
            /* A broken pipe here means git already failed; its exit status below says why. */
            let _ = stdin.write_all(bytes);
        }

        let output = child
            .wait_with_output()
            .map_err(|error| ErrorObject::internal(format!("git is not available: {error}")))?;

        if !output.status.success() {
            return Err(ErrorObject::internal(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        Ok(output.stdout)
    }
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
    let shadow = Shadow::of(project_root)?;
    let _index = INDEX.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    shadow.run(&["add", "-A"])?;
    shadow.run(&["commit", "--quiet", "--allow-empty", "-m", message])?;

    Ok(shadow.run(&["rev-parse", "HEAD"])?.trim().to_string())
}

static SCRATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Held while the repository's own index is being read or rewritten: git refuses two writers at once (`index.lock`),
/// and a checkpoint, a restore and the deferred checkpoint below must take turns.
static INDEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What some paths looked like **before** a change: their entries in git's index format, captured now.
///
/// A change made from the window (Save, Rename, Delete) must be undoable, so a checkpoint has to hold the files as they
/// were - but a checkpoint of the whole folder walks every file of it, and on a slow disk or a big folder that walk is
/// what a person waited for after pressing Save. Capturing only the paths the change touches takes a moment; the
/// walk then happens afterwards (`checkpoint_with`), and puts these entries back over what it finds.
pub struct PreImage {
    /// The touched paths, relative to the project and written with `/`.
    pub paths: Vec<String>,
    /// `git ls-files -s -z` of those paths, or empty when none of them existed.
    pub entries: Vec<u8>,
}

/// Captures the pre-image of `paths` (absolute, inside `project_root`). A path git does not track - ignored, outside
/// the folder, not there - simply has no entry, which is what a checkpoint holds for it too.
pub fn preimage(project_root: &Path, paths: &[&Path]) -> Result<PreImage, ErrorObject> {
    let relative: Vec<String> = paths
        .iter()
        .filter_map(|path| path.strip_prefix(project_root).ok())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .filter(|path| !path.is_empty())
        .collect();

    if relative.is_empty() {
        return Ok(PreImage { paths: Vec::new(), entries: Vec::new() });
    }

    let shadow = Shadow::of(project_root)?;
    let scratch = shadow.git_dir.join(format!(
        "index-gesture-{}-{}",
        std::process::id(),
        SCRATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    /* Only what is there to be read: `git add` refuses the whole command when one of its paths does not exist, and a
       path that does not exist has no pre-image to capture (a file about to be created, a rename's new name). */
    let present: Vec<&str> = relative
        .iter()
        .map(String::as_str)
        .filter(|path| std::fs::symlink_metadata(project_root.join(path)).is_ok())
        .collect();

    if !present.is_empty() {
        let mut add = vec!["--literal-pathspecs", "add", "-A", "--ignore-errors", "--"];

        add.extend(present);

        /* Refused for a path that is ignored; the listing below says what actually got in. */
        let _ = shadow.run_indexed(&add, None, Some(&scratch));
    }

    let mut list = vec!["--literal-pathspecs", "ls-files", "-s", "-z", "--"];

    list.extend(relative.iter().map(String::as_str));

    let entries = shadow.run_indexed(&list, None, Some(&scratch));

    let _ = std::fs::remove_file(&scratch);

    Ok(PreImage { paths: relative, entries: entries? })
}

/// A checkpoint of the folder **as it was before** the changes whose pre-images are given: the whole folder is read
/// as it is now, and every touched path is put back to the way its pre-image says.
pub fn checkpoint_with(project_root: &Path, message: &str, images: &[PreImage]) -> Result<String, ErrorObject> {
    let shadow = Shadow::of(project_root)?;
    let _index = INDEX.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    shadow.run(&["add", "-A"])?;

    for image in images.iter().filter(|image| !image.paths.is_empty()) {
        let mut remove = vec!["--literal-pathspecs", "rm", "-r", "--cached", "--ignore-unmatch", "-q", "--"];

        remove.extend(image.paths.iter().map(String::as_str));
        shadow.run_raw(&remove, None)?;

        if !image.entries.is_empty() {
            shadow.run_raw(&["update-index", "-z", "--index-info"], Some(&image.entries))?;
        }
    }

    shadow.run(&["commit", "--quiet", "--allow-empty", "-m", message])?;

    Ok(shadow.run(&["rev-parse", "HEAD"])?.trim().to_string())
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
    /* `--no-optional-locks`: a badge that is refreshed in the background must not take the repository's index lock
       away from the person's own `git commit`. */
    let porcelain = run_here(project_root, &["--no-optional-locks", "status", "--porcelain"])?;
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

/// Makes the folder **exactly** what a shadow commit recorded (v4): changed files go back, deleted
/// files come back, and files created after it are removed.
///
/// The rewind used to run `checkout <sha> -- .` with the *shadow repository* as the work tree - so it
/// looked for the shadow of the shadow, failed, and the error was discarded: a local rewind never
/// restored a single file while it answered `restoredFiles: 1`. `add -A` first, so the index knows every
/// file that exists now; `read-tree -u --reset` then moves the index and the folder to the commit,
/// removing what the commit does not have. Ignored files (`node_modules`) are not in the index and are
/// left alone; the project's own `.git` is never touched.
pub fn restore(project_root: &Path, sha: &str) -> Result<(), ErrorObject> {
    let shadow = Shadow::of(project_root)?;
    let _index = INDEX.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    shadow.run(&["add", "-A"])?;
    shadow.run(&["read-tree", "-u", "--reset", sha])?;

    Ok(())
}

/// What changed in the folder since a shadow commit, new files included (see `ssh::ops::shadow_changes`,
/// the same rule on a host). Only the shadow's index is written; the project's own repository is not.
pub fn changes_since(project_root: &Path, sha: &str) -> Result<String, ErrorObject> {
    let shadow = Shadow::of(project_root)?;

    shadow.run(&["add", "-N", "--", ":/"])?;

    shadow.run(&["diff", sha])
}

/// Which files changed since a shadow commit, new ones included: `[(status, path)]` with git's letters
/// (`A` added, `M` modified, `D` deleted, `R` renamed). The Time Machine's before/after list.
pub fn files_since(project_root: &Path, sha: &str) -> Result<Vec<(String, String)>, ErrorObject> {
    let shadow = Shadow::of(project_root)?;

    shadow.run(&["add", "-N", "--", ":/"])?;

    Ok(parse_name_status(&shadow.run(&["diff", "--name-status", sha])?))
}

/// `git diff --name-status` output as `(status, path)` pairs; a rename keeps its new path.
pub fn parse_name_status(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let status = parts.next()?.trim().chars().next()?.to_string();
            let path = parts.next_back()?.trim().to_string();

            (!path.is_empty()).then_some((status, path))
        })
        .collect()
}

/// One file's change since a shadow commit.
pub fn file_diff(project_root: &Path, sha: &str, path: &str) -> Result<String, ErrorObject> {
    let path = clean_relative(path)?;

    let shadow = Shadow::of(project_root)?;

    shadow.run(&["add", "-N", "--", &path])?;

    shadow.run(&["diff", sha, "--", &path])
}

/// Puts **one file** back as a shadow commit recorded it - the rest of the folder is not touched.
///
/// A file the commit did not have (made after the checkpoint) is removed, which is what "as it was" means
/// for it. Returns `restored` or `removed`.
pub fn restore_file(project_root: &Path, sha: &str, path: &str) -> Result<&'static str, ErrorObject> {
    let path = clean_relative(path)?;
    let shadow = Shadow::of(project_root)?;
    let _index = INDEX.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let existed = shadow.run(&["cat-file", "-e", &format!("{sha}:{path}")]).is_ok();

    if existed {
        shadow.run(&["checkout", sha, "--", &path])?;

        return Ok("restored");
    }

    let absolute = project_root.join(&path);

    if absolute.exists() {
        std::fs::remove_file(&absolute).map_err(ErrorObject::internal)?;
    }

    Ok("removed")
}

/// A path the person gave, as git wants it: relative, forward slashes, and never outside the folder.
pub fn clean_relative(path: &str) -> Result<String, ErrorObject> {
    let normal = path.replace('\\', "/").trim_start_matches("./").trim_start_matches('/').to_string();

    if normal.is_empty() || normal.split('/').any(|segment| segment == "..") {
        return Err(ErrorObject::bad_request(format!("`{path}` is not a path inside the folder")));
    }

    Ok(normal)
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

    /// P0-17's fix, pinned: a [`Shadow`] runs every command in the repository it was opened with.
    ///
    /// The bug this replaces: each command resolved the shadow repository again from the data directory, so a data
    /// directory that moved between two commands of one operation - it comes from the environment - staged the tree in one
    /// repository and committed it in another, and a fresh repository commits an *empty* tree. Restoring that commit
    /// emptied the project; the rewind test caught it as a folder that came back with no files in it.
    #[test]
    fn a_shadow_runs_its_commands_in_the_repository_it_was_opened_with() {
        let project = tempfile::tempdir().unwrap();

        std::fs::write(project.path().join("a.txt"), "one\n").unwrap();

        let repo = tempfile::tempdir().unwrap().keep();

        assert!(run_here(&repo, &["init", "--quiet"]).is_ok());

        let shadow = Shadow { git_dir: repo.join(".git"), work_tree: project.path().to_path_buf() };
        let elsewhere = tempfile::tempdir().unwrap().keep();

        assert!(run_here(&elsewhere, &["init", "--quiet"]).is_ok());

        shadow.run(&["add", "-A"]).unwrap();
        shadow.run(&["commit", "--quiet", "--allow-empty", "-m", "before"]).unwrap();

        let sha = shadow.run(&["rev-parse", "HEAD"]).unwrap().trim().to_string();

        assert!(run_here(&repo, &["--git-dir", ".git", "cat-file", "-e", &sha]).is_ok(), "the commit is in the repository the operation opened");
        assert!(run_here(&elsewhere, &["--git-dir", ".git", "cat-file", "-e", &sha]).is_err(), "and in no other one");
        assert_eq!(shadow.run(&["ls-tree", "-r", "--name-only", &sha]).unwrap().trim(), "a.txt", "the tree is the project's, not an empty one");
    }

    /// The tree a shadow commit recorded, as `path -> content`.
    fn recorded(project: &Path, sha: &str) -> std::collections::BTreeMap<String, String> {
        let shadow = Shadow::of(project).unwrap();
        let names = shadow.run(&["ls-tree", "-r", "--name-only", sha]).unwrap();

        names
            .lines()
            .map(|name| (name.to_string(), shadow.run(&["show", &format!("{sha}:{name}")]).unwrap()))
            .collect()
    }

    /// A checkpoint taken *after* a Save, with the file's pre-image put back over it, holds the file as it was.
    #[test]
    fn a_deferred_checkpoint_holds_the_file_as_it_was_before_the_save() {
        if !git_available() {
            return;
        }

        let project = tempfile::tempdir().unwrap();
        let file = project.path().join("a.txt");

        std::fs::write(&file, "before\n").unwrap();
        std::fs::write(project.path().join("b.txt"), "other\n").unwrap();

        let image = preimage(project.path(), &[file.as_path()]).unwrap();

        std::fs::write(&file, "after\n").unwrap();

        let sha = checkpoint_with(project.path(), "sdcd: save", &[image]).unwrap();
        let tree = recorded(project.path(), &sha);

        assert_eq!(tree["a.txt"], "before\n", "the touched file is as it was");
        assert_eq!(tree["b.txt"], "other\n", "the rest of the folder is as it is");
    }

    /// A deleted folder is in the checkpoint even though it was gone when the folder was read.
    #[test]
    fn a_deferred_checkpoint_still_holds_a_folder_that_was_deleted() {
        if !git_available() {
            return;
        }

        let project = tempfile::tempdir().unwrap();
        let folder = project.path().join("src");

        std::fs::create_dir_all(folder.join("deep")).unwrap();
        std::fs::write(folder.join("one.rs"), "1\n").unwrap();
        std::fs::write(folder.join("deep").join("two.rs"), "2\n").unwrap();
        std::fs::write(project.path().join("keep.txt"), "keep\n").unwrap();

        let image = preimage(project.path(), &[folder.as_path()]).unwrap();

        std::fs::remove_dir_all(&folder).unwrap();

        let sha = checkpoint_with(project.path(), "sdcd: delete", &[image]).unwrap();
        let tree = recorded(project.path(), &sha);

        assert_eq!(tree["src/one.rs"], "1\n");
        assert_eq!(tree["src/deep/two.rs"], "2\n");
        assert_eq!(tree["keep.txt"], "keep\n");
    }

    /// A new file did not exist before, so the checkpoint must not hold it - and a rename is a delete of the old name
    /// plus a create of the new one.
    #[test]
    fn a_deferred_checkpoint_leaves_out_what_the_change_created() {
        if !git_available() {
            return;
        }

        let project = tempfile::tempdir().unwrap();
        let old = project.path().join("old.txt");
        let new = project.path().join("new.txt");

        std::fs::write(&old, "content\n").unwrap();

        let image = preimage(project.path(), &[old.as_path(), new.as_path()]).unwrap();

        std::fs::rename(&old, &new).unwrap();

        let sha = checkpoint_with(project.path(), "sdcd: rename", &[image]).unwrap();
        let tree = recorded(project.path(), &sha);

        assert_eq!(tree.get("old.txt").map(String::as_str), Some("content\n"));
        assert!(!tree.contains_key("new.txt"), "the renamed-to name did not exist before");
    }

    /// Several changes in one checkpoint each get their own pre-image back.
    #[test]
    fn one_deferred_checkpoint_can_cover_several_changes() {
        if !git_available() {
            return;
        }

        let project = tempfile::tempdir().unwrap();
        let (one, two) = (project.path().join("one.txt"), project.path().join("two.txt"));

        std::fs::write(&one, "1\n").unwrap();
        std::fs::write(&two, "2\n").unwrap();

        let first = preimage(project.path(), &[one.as_path()]).unwrap();

        std::fs::write(&one, "1 changed\n").unwrap();

        let second = preimage(project.path(), &[two.as_path()]).unwrap();

        std::fs::write(&two, "2 changed\n").unwrap();

        let sha = checkpoint_with(project.path(), "sdcd: two saves", &[first, second]).unwrap();
        let tree = recorded(project.path(), &sha);

        assert_eq!(tree["one.txt"], "1\n");
        assert_eq!(tree["two.txt"], "2\n");
    }

    /// A path outside the folder has no pre-image, which is how the caller knows to take the plain checkpoint.
    #[test]
    fn a_path_outside_the_folder_has_no_preimage() {
        let project = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let image = preimage(project.path(), &[elsewhere.path().join("x.txt").as_path()]).unwrap();

        assert!(image.paths.is_empty());
    }
}
