//! A bench task (`bench/tasks/*.toml`) and its verifier (v3 W1).
//!
//! ```toml
//! id = "bugfix-01"
//! kind = "bugfix"            # find | explain | edit | bugfix | feature | devops | multilingual | fleet | repeat
//! fixture = "rust-ledger"    # bench/fixtures/<fixture>, copied to a fresh folder for every run
//! tiers = ["smoke", "medium"] # every task is in "full"
//! language = "en"
//! repeat_of = "edit-02"      # a paired repeat (I4): same work, asked again later
//! requires = ["docker"]      # skipped (and said so) when the machine lacks it
//! max_steps = 40
//! timeout_secs = 600
//! prompt = "..."
//!
//! [verify]
//! answer_all = ["regex", ...]   # every one must match the final answer (case-insensitive)
//! answer_any = ["regex", ...]   # rubric: at least `answer_min` of them must match
//! answer_min = 3
//! hidden = "bugfix-01"          # bench/tasks/hidden/<dir>, copied over the work folder after the turn
//! commands = ["cargo test -q"]  # each must exit 0 in the work folder (after `hidden`)
//! no_edits = true               # a find/explain task must not change tracked files
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Verify {
    #[serde(default)]
    pub answer_all: Vec<String>,
    #[serde(default)]
    pub answer_any: Vec<String>,
    #[serde(default)]
    pub answer_min: usize,
    #[serde(default)]
    pub hidden: Option<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub no_edits: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub kind: String,
    pub fixture: String,
    #[serde(default)]
    pub tiers: Vec<String>,
    #[serde(default = "english")]
    pub language: String,
    #[serde(default)]
    pub repeat_of: Option<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default = "default_steps")]
    pub max_steps: u64,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    pub prompt: String,
    #[serde(default)]
    pub verify: Verify,
}

fn english() -> String {
    "en".into()
}

fn default_steps() -> u64 {
    40
}

fn default_timeout() -> u64 {
    600
}

pub const KINDS: [&str; 9] = ["find", "explain", "edit", "bugfix", "feature", "devops", "multilingual", "fleet", "repeat"];

impl Task {
    pub fn parse(text: &str) -> Result<Self, String> {
        let task: Task = toml::from_str(text).map_err(|error| error.to_string())?;

        if !KINDS.contains(&task.kind.as_str()) {
            return Err(format!("{}: unknown kind `{}`", task.id, task.kind));
        }

        for pattern in task.verify.answer_all.iter().chain(&task.verify.answer_any) {
            regex::Regex::new(pattern).map_err(|error| format!("{}: bad regex `{pattern}`: {error}", task.id))?;
        }

        if task.verify.answer_min > task.verify.answer_any.len() {
            return Err(format!("{}: answer_min is larger than answer_any", task.id));
        }

        let checks = task.verify.answer_all.len() + task.verify.answer_any.len() + task.verify.commands.len();

        if checks == 0 {
            return Err(format!("{}: a task needs something to verify", task.id));
        }

        Ok(task)
    }

    /// Is the task in a tier? `full` holds the plan's 40 (every task but the paired repeats and fleet tasks, which
    /// have tiers of their own); `medium` holds the smoke tasks too.
    pub fn in_tier(&self, tier: &str) -> bool {
        match tier {
            "full" => !matches!(self.kind.as_str(), "repeat" | "fleet") || self.tiers.iter().any(|t| t == "full"),
            "medium" => self.tiers.iter().any(|t| t == "medium" || t == "smoke"),
            other => self.tiers.iter().any(|t| t == other),
        }
    }
}

/// Every task under `dir` (`*.toml`, sorted by id).
pub fn load_all(dir: &Path) -> Result<Vec<Task>, String> {
    let mut tasks = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        /* `._name` files are macOS metadata written beside every file on an exFAT drive, not tasks. */
        let metadata = entry.file_name().to_string_lossy().starts_with("._");

        if !metadata && path.extension().is_some_and(|ext| ext == "toml") {
            let text = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;

            tasks.push(Task::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?);
        }
    }

    tasks.sort_by(|a, b| a.id.cmp(&b.id));

    let mut ids: Vec<&str> = tasks.iter().map(|task| task.id.as_str()).collect();

    ids.dedup();

    if ids.len() != tasks.len() {
        return Err("two tasks share an id".into());
    }

    Ok(tasks)
}

/// The verdict of one run, with what failed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Outcome {
    pub pass: bool,
    pub failures: Vec<String>,
}

/// Checks a finished run: the final answer, the hidden tests, and that a read-only task changed nothing.
pub fn verify(task: &Task, answer: &str, work: &Path, hidden_root: &Path) -> Outcome {
    let mut failures = Vec::new();
    let matches = |pattern: &str| regex::RegexBuilder::new(pattern).case_insensitive(true).build().is_ok_and(|re| re.is_match(answer));

    for pattern in &task.verify.answer_all {
        if !matches(pattern) {
            failures.push(format!("answer lacks /{pattern}/"));
        }
    }

    if !task.verify.answer_any.is_empty() {
        let found = task.verify.answer_any.iter().filter(|pattern| matches(pattern)).count();

        if found < task.verify.answer_min {
            failures.push(format!("rubric {found}/{} (needs {})", task.verify.answer_any.len(), task.verify.answer_min));
        }
    }

    if task.verify.no_edits {
        match git(work, &["status", "--porcelain", "--untracked-files=no"]) {
            Ok(status) if status.trim().is_empty() => {}
            Ok(status) => failures.push(format!("a read-only task changed files: {}", status.trim().replace('\n', ", "))),
            Err(error) => failures.push(format!("git status failed: {error}")),
        }
    }

    if let Some(hidden) = &task.verify.hidden {
        if let Err(error) = copy_tree(&hidden_root.join(hidden), work) {
            failures.push(format!("hidden tests could not be copied: {error}"));
        }
    }

    for command in &task.verify.commands {
        match shell(work, command) {
            Ok((true, _)) => {}
            Ok((false, tail)) => failures.push(format!("`{command}` failed: {tail}")),
            Err(error) => failures.push(format!("`{command}` could not start: {error}")),
        }
    }

    Outcome { pass: failures.is_empty(), failures }
}

/// Runs a verify command through the platform shell; answers success and the last lines of its output.
pub fn shell(dir: &Path, command: &str) -> Result<(bool, String), String> {
    let output = if cfg!(windows) {
        Command::new("cmd").args(["/C", command]).current_dir(dir).output()
    } else {
        Command::new("sh").args(["-c", command]).current_dir(dir).output()
    }
    .map_err(|error| error.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let tail: Vec<&str> = text.lines().rev().take(6).collect();

    Ok((output.status.success(), tail.into_iter().rev().collect::<Vec<_>>().join(" | ")))
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git").args(args).current_dir(dir).output().map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Copies a folder's contents into another, overwriting files of the same name. Skips `target`,
/// `node_modules` and `.git`, so a fixture that was built locally copies only its sources.
pub fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    if !from.is_dir() {
        return Err(format!("{} is not a folder", from.display()));
    }

    std::fs::create_dir_all(to).map_err(|error| error.to_string())?;

    for entry in std::fs::read_dir(from).map_err(|error| error.to_string())?.flatten() {
        let name = entry.file_name();
        let skip = name == "target" || name == "node_modules" || name == ".git" || name == "__pycache__" || name.to_string_lossy().starts_with("._");

        if skip {
            continue;
        }

        let (source, dest): (PathBuf, PathBuf) = (entry.path(), to.join(&name));
        let kind = entry.file_type().map_err(|error| error.to_string())?;

        if kind.is_dir() {
            copy_tree(&source, &dest)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &dest).map_err(|error| format!("{}: {error}", source.display()))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = r#"
id = "find-01"
kind = "find"
fixture = "rust-ledger"
tiers = ["smoke"]
prompt = "Where is the balance computed?"

[verify]
answer_all = ["src/ledger\\.rs"]
answer_any = ["balance", "fn total", "credit", "debit"]
answer_min = 2
no_edits = true
"#;

    #[test]
    fn a_task_file_parses_with_its_defaults() {
        let task = Task::parse(TASK).unwrap();

        assert_eq!((task.max_steps, task.timeout_secs, task.language.as_str()), (40, 600, "en"));
        assert!(task.in_tier("smoke") && task.in_tier("medium") && task.in_tier("full"));
        assert!(!Task::parse(&TASK.replace("medium", "x").replace("\"smoke\"", "\"medium\"")).unwrap().in_tier("smoke"));
    }

    #[test]
    fn bad_task_files_are_refused_with_a_reason() {
        assert!(Task::parse(&TASK.replace("kind = \"find\"", "kind = \"dance\"")).unwrap_err().contains("unknown kind"));
        assert!(Task::parse(&TASK.replace("answer_min = 2", "answer_min = 9")).unwrap_err().contains("answer_min"));
        assert!(Task::parse(&TASK.replace("src/ledger\\\\.rs", "(")).unwrap_err().contains("bad regex"));
    }

    #[test]
    fn the_verifier_checks_the_answer_the_hidden_tests_and_read_only_tasks() {
        let base = std::env::temp_dir().join(format!("sdc-bench-verify-{}", std::process::id()));
        let (work, hidden) = (base.join("work"), base.join("hidden"));
        let _ = std::fs::remove_dir_all(&base);

        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(hidden.join("t1")).unwrap();
        std::fs::write(work.join("a.txt"), "one\n").unwrap();
        std::fs::write(hidden.join("t1").join("check.txt"), "hidden\n").unwrap();

        for args in [&["init", "-q"][..], &["add", "."], &["-c", "user.email=b@b", "-c", "user.name=b", "commit", "-qm", "base"]] {
            git(&work, args).unwrap();
        }

        let task = Task::parse(TASK).unwrap();

        assert!(verify(&task, "It is in src/ledger.rs: fn total adds each credit.", &work, &hidden).pass);

        let missing = verify(&task, "It is somewhere.", &work, &hidden);

        assert!(!missing.pass && missing.failures.len() == 2, "{:?}", missing.failures);

        std::fs::write(work.join("a.txt"), "changed\n").unwrap();
        assert!(verify(&task, "src/ledger.rs balance credit", &work, &hidden).failures[0].contains("read-only task changed"));

        let mut with_tests = task.clone();

        with_tests.verify = Verify { hidden: Some("t1".into()), commands: vec![if cfg!(windows) { "type check.txt".into() } else { "cat check.txt".into() }], ..Default::default() };
        assert!(verify(&with_tests, "", &work, &hidden).pass, "the hidden file arrives before the command runs");

        with_tests.verify.commands = vec!["exit 3".into()];
        assert!(!verify(&with_tests, "", &work, &hidden).pass);

        let _ = std::fs::remove_dir_all(&base);
    }
}
