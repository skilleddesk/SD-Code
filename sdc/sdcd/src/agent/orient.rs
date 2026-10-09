//! Pace and orientation (0.20): getting the agent to the answer in fewer steps.
//!
//! Where a turn's time goes is the model's calls - one per step, three to ten seconds each - so the lever
//! that matters is *how many steps a request takes*, and two things decide that before the first one:
//!
//! * **Pace** - how much work this request deserves. A question with no change in it ("why does the login
//!   redirect?", "kivabe cache kaj kore?") used to get the same plan, narration and self-review as a
//!   refactor. [`pace_of`] reads the person's own words, locally and without a model call, and
//!   [`pace_note`] says it in one line at the front of the message: answer directly; or, for a large task,
//!   plan first.
//! * **Orientation** - the first thing every agent turn did was look around: `list_dir .`, `glob`, find out
//!   which command runs the tests. The turn's history keeps what was *said*, not what the tools returned,
//!   so every new turn started blind. [`orientation`] hands over the folder's top level, the project's own
//!   check commands and what is uncommitted, in about 150 tokens, so the first step can be the one that
//!   matters.
//!
//! Both are for the turn in flight only; the stored conversation stays the person's own words.

use std::time::Duration;

use super::search::SKIPPED_DIRS;
use super::workspace::Workspace;

/// How much work a request deserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// A question, or a greeting: answer it, with at most a lookup or two.
    Quick,
    /// Everything else: the system prompt's own way of working.
    Standard,
    /// A large or many-part task: plan first, check, review.
    Deep,
}

/// Words that ask for something to be *done* - in English, and in Banglish the way people type it. A request
/// with one of these is never Quick.
const CHANGE_CUES: &[&str] = &[
    "fix", "add", "change", "update", "create", "write", "implement", "remove", "delete", "rename", "refactor", "install", "run",
    "build", "deploy", "make", "edit", "replace", "convert", "move", "set", "generate", "optimize", "optimise", "improve", "debug",
    "test", "migrate", "rewrite", "redesign", "upgrade", "setup", "commit", "push", "merge", "revert", "undo", "apply", "patch",
    "koro", "kor", "korbo", "korte", "korba", "likho", "likhe", "banao", "banabo", "banate", "lagao", "lagabo", "thik", "dao", "deo",
    "chalao", "bad", "bosao", "bodlao", "sajao", "tule", "jog", "muche", "fele", "continue", "proceed", "retry", "chalu", "cholo",
    "করো", "কর", "করুন", "লেখো", "লিখো", "বানাও", "ঠিক", "যোগ", "চালাও", "মুছে", "বদলাও", "দাও",
];

/// Words that open a question.
const QUESTION_WORDS: &[&str] = &[
    "what", "why", "how", "where", "which", "who", "when", "is", "are", "does", "do", "can", "could", "should", "explain", "tell",
    "whats", "hows", "ki", "keno", "kivabe", "kibhabe", "kothay", "koto", "kon", "kar", "kobe", "bujhiye", "bolo", "bolun", "jiggesh",
    "কী", "কি", "কেন", "কীভাবে", "কিভাবে", "কোথায়", "কত", "কোন", "বুঝিয়ে", "বলো",
];

/// Phrases and words that say the task is big.
const DEEP_CUES: &[&str] = &[
    "refactor", "migrate", "rewrite", "redesign", "from scratch", "end to end", "end-to-end", "entire", "whole project", "every file",
    "all files", "architecture", "security audit", "full app", "complete app", "whole app", "puro project", "sob file", "sokol file",
    "notun system", "from the ground up", "পুরো প্রজেক্ট", "সব ফাইল",
];

/// The text without what is code: a `backticked` span, or a call such as `add(2, 3)`. A function called
/// `add` or `update` is something the person asks *about*, not a request to add or update anything.
fn without_code(text: &str) -> String {
    static CODE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

    CODE.get_or_init(|| regex::Regex::new(r"`[^`]*`|[A-Za-z_][\w.]*\([^)]*\)").expect("a valid pattern")).replace_all(text, " ").into_owned()
}

fn words(text: &str) -> Vec<String> {
    /* The Bengali and Devanagari blocks are whole: their vowel signs are marks, which `is_alphanumeric` would split on. */
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || matches!(c as u32, 0x0900..=0x09FF)))
        .filter(|word| !word.is_empty())
        .map(|word| word.trim_matches('\'').to_lowercase())
        .collect()
}

/// How much work `text` - the person's own words - deserves. Conservative on purpose: only a plain
/// question is Quick, and when unsure the answer is [`Pace::Standard`], which changes nothing.
pub fn pace_of(text: &str) -> Pace {
    let trimmed = text.trim();

    if trimmed.is_empty() || trimmed.starts_with('/') {
        return Pace::Standard;
    }

    let lowered = without_code(trimmed).to_lowercase();
    let all = words(&lowered);
    let count = all.len();
    let cues = all.iter().filter(|word| CHANGE_CUES.contains(&word.as_str())).count();

    if count >= 80 || cues >= 4 || DEEP_CUES.iter().any(|cue| lowered.contains(cue)) {
        return Pace::Deep;
    }

    if cues > 0 || count > 16 {
        return Pace::Standard;
    }

    /* A short line with nothing to do in it: a question, or a hello. */
    let asks = trimmed.contains('?') || trimmed.contains('？') || all.first().is_some_and(|word| QUESTION_WORDS.contains(&word.as_str()));
    let greeting = count <= 3 && all.iter().any(|word| matches!(word.as_str(), "hi" | "hello" | "hey" | "salam" | "assalamualaikum" | "thanks" | "thank" | "hlw" | "hola"));

    if asks || greeting { Pace::Quick } else { Pace::Standard }
}

/// The one line that says it, or `None` when the system prompt's own way of working is right.
pub fn pace_note(pace: Pace) -> Option<&'static str> {
    match pace {
        Pace::Quick => Some(
            "[SDC pace: a quick question. Answer it directly - from what you know, or after one or two lookups. No plan, no narration, no review: the shortest correct answer, first.]",
        ),
        Pace::Deep => Some(
            "[SDC pace: a larger task. Plan it first (update_plan), work through it step by step, run the project's checks, and review your own change before your summary.]",
        ),
        Pace::Standard => None,
    }
}

/// How many names of the top level are shown.
const MAP_NAMES: usize = 40;
/// How many uncommitted files are shown.
const MAP_CHANGES: usize = 12;

/// The orientation text from what was gathered - pure, so the rules are tested without a folder.
///
/// `entries` is the top level as `Workspace::list` names it (`src/`, `package.json (1204 bytes)`).
pub fn build_orientation(entries: &[String], hidden: i64, checks: &[String], status: &[String]) -> Option<String> {
    let names: Vec<String> = entries
        .iter()
        .map(|entry| entry.split(" (").next().unwrap_or(entry).to_string())
        .filter(|name| name.trim_end_matches('/') != ".git" && !SKIPPED_DIRS.contains(&name.trim_end_matches('/')))
        .collect();

    if names.is_empty() {
        return None;
    }

    let more = names.len().saturating_sub(MAP_NAMES) as i64 + hidden.max(0);
    let mut lines = vec![
        "[SDC: a map of the project, current as of this message - read it instead of listing the folder]".to_string(),
        format!(
            "Top level: {}{}",
            names.iter().take(MAP_NAMES).cloned().collect::<Vec<_>>().join(", "),
            if more > 0 { format!(" (+{more} more)") } else { String::new() }
        ),
    ];

    if !checks.is_empty() {
        lines.push(format!("The project's own checks: {}", checks.iter().map(|check| format!("`{check}`")).collect::<Vec<_>>().join(" · ")));
    }

    if !status.is_empty() {
        let extra = status.len().saturating_sub(MAP_CHANGES);

        lines.push(format!(
            "Uncommitted: {}{}",
            status.iter().take(MAP_CHANGES).cloned().collect::<Vec<_>>().join(" · "),
            if extra > 0 { format!(" (+{extra} more)") } else { String::new() }
        ));
    }

    Some(lines.join("\n"))
}

/// The map of the folder the turn works in. Gathered at the same time (the top level, and `git status` on
/// this machine - a host would pay another round trip for it), and `None` on any trouble: a map is a
/// help, never a reason for a turn to fail.
pub fn orientation(workspace: &Workspace) -> Option<String> {
    let status = std::thread::scope(|scope| {
        let status = (!workspace.is_remote()).then(|| scope.spawn(|| git_status(workspace)));
        let (entries, hidden) = workspace.list(".").ok()?;
        let names: Vec<String> = entries.iter().map(|entry| entry.split(" (").next().unwrap_or(entry).trim_end_matches('/').to_string()).collect();
        let posix = workspace.is_remote() || !cfg!(windows);
        let checks: Vec<String> = crate::verify::plan(&names, &|name| workspace.read(name).ok().map(|(text, _)| text), posix)
            .into_iter()
            .map(|check| check.command)
            .collect();
        let status = status.and_then(|handle| handle.join().ok()).unwrap_or_default();

        Some((entries, hidden, checks, status))
    })?;

    build_orientation(&status.0, status.1, &status.2, &status.3)
}

/// `git status --short`, as short lines; empty outside a repository, on a timeout or on an error.
fn git_status(workspace: &Workspace) -> Vec<String> {
    match workspace.run("git status --short", Duration::from_secs(3)) {
        Ok(report) if report.ok && !report.timed_out => report.stdout.lines().map(|line| line.trim().to_string()).filter(|line| !line.is_empty()).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_question_is_quick_and_a_request_is_not() {
        for question in [
            "why does the login redirect fail?",
            "what is a mutex",
            "kivabe cache kaj kore",
            "keno eta hocche?",
            "hi",
            "কেন এটা কাজ করছে না?",
            "why does add(2, 3) return -1 in this project?",
            "what does `update` do here?",
        ] {
            assert_eq!(pace_of(question), Pace::Quick, "{question}");
        }

        for request in ["fix the login redirect", "add a dark mode toggle", "login er por redirect fix koro", "run the tests", "why is it slow? fix it", "ei file ta thik koro"] {
            assert_ne!(pace_of(request), Pace::Quick, "{request}");
        }
    }

    #[test]
    fn a_large_or_many_part_task_is_deep() {
        assert_eq!(pace_of("refactor the auth module"), Pace::Deep);
        assert_eq!(pace_of("rewrite the whole project in TypeScript"), Pace::Deep);
        assert_eq!(pace_of("add a toggle, fix the header, update the tests, and write the docs"), Pace::Deep);
        assert_eq!(pace_of(&"word ".repeat(90)), Pace::Deep);
    }

    #[test]
    fn when_unsure_nothing_changes() {
        assert_eq!(pace_of(""), Pace::Standard);
        assert_eq!(pace_of("/research rust async"), Pace::Standard);
        assert_eq!(pace_of("the login page"), Pace::Standard);
        assert!(pace_note(Pace::Standard).is_none());
        assert!(pace_note(Pace::Quick).unwrap().contains("quick question"));
        assert!(pace_note(Pace::Deep).unwrap().contains("update_plan"));
    }

    #[test]
    fn the_map_names_the_top_level_the_checks_and_the_uncommitted() {
        let entries = vec!["src/".to_string(), "node_modules/".to_string(), "package.json (1204 bytes)".to_string(), ".git/".to_string()];
        let map = build_orientation(&entries, 3, &["pnpm run test".to_string()], &["M src/a.ts".to_string(), "?? b.txt".to_string()]).unwrap();

        assert!(map.contains("Top level: src/, package.json (+3 more)"), "{map}");
        assert!(!map.contains("node_modules"), "heavy folders are not the project: {map}");
        assert!(map.contains("The project's own checks: `pnpm run test`"), "{map}");
        assert!(map.contains("Uncommitted: M src/a.ts · ?? b.txt"), "{map}");
        assert!(build_orientation(&[], 0, &[], &[]).is_none());
    }

    #[test]
    fn a_real_folder_is_mapped() {
        let root = std::env::temp_dir().join(format!("sdc-orient-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("package.json"), r#"{"scripts":{"test":"vitest run"}}"#).unwrap();

        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let map = orientation(&workspace).expect("a folder with files has a map");

        assert!(map.contains("src/") && map.contains("package.json"), "{map}");
        assert!(map.contains("test"), "the project's test script is a check: {map}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
