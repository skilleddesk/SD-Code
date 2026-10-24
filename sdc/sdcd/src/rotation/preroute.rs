//! **Pre-routing (M1-13, switch `relay.preroute`, off)**: a step plainly beyond the local model goes to the chain at once, instead of after the
//! failed attempts that would have told SDC the same.
//!
//! The judgement is a heuristic, written down as a score so a test can read it: what kind of step it is, how many files and declarations it
//! touches, how small the model's window is (S/M/L), and whether a step of this kind has needed the chain before in this project. It is **off by
//! default because it is unmeasured**: a wrong "hard" sends a step to the cloud that the local model would have done, which is exactly the cost
//! Silent Relay exists to avoid. When it is on, the receipt says which steps were pre-routed.

use std::path::Path;

use crate::sme::budget::Class;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A question about the code: reading, no change.
    Question,
    /// One thing is wrong: find it and fix it.
    Bugfix,
    /// Add or change behaviour.
    Edit,
    /// Rename, move, migrate, restructure: many places change together.
    Refactor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

/// What the heuristic reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub kind: Kind,
    pub files: usize,
    pub symbols: usize,
    pub class: Class,
    /// A step of this kind needed the chain before, in this project.
    pub relayed_before: bool,
}

/// The score: each part is a number a person can argue with.
pub fn score(facts: &Facts) -> u32 {
    let kind = match facts.kind {
        Kind::Question => 0,
        Kind::Bugfix => 1,
        Kind::Edit => 2,
        Kind::Refactor => 4,
    };
    let class = match facts.class {
        Class::Large => 0,
        Class::Medium => 1,
        Class::Small => 2,
    };

    kind + facts.files.min(8) as u32 * 2 + facts.symbols.min(8) as u32 + class + if facts.relayed_before { 3 } else { 0 }
}

/// `Hard` from 9 on, `Medium` from 5.
pub fn difficulty(facts: &Facts) -> Difficulty {
    match score(facts) {
        9.. => Difficulty::Hard,
        5..=8 => Difficulty::Medium,
        _ => Difficulty::Easy,
    }
}

/// Should the step go to the chain without a local attempt? Only a hard one, and never a question (a question changes nothing, so a
/// wrong answer costs a re-ask, not a cloud call).
pub fn plainly_beyond(facts: &Facts) -> bool {
    facts.kind != Kind::Question && difficulty(facts) == Difficulty::Hard
}

/// The kind of step a request is, from its own words.
pub fn kind_of(request: &str) -> Kind {
    let lower = request.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));

    if has(&["refactor", "rename", "migrate", "restructure", "move all", "across the", "every file", "সব ফাইল", "রিনেম"]) {
        Kind::Refactor
    } else if has(&["fix", "bug", "error", "fails", "failing", "broken", "crash", "ঠিক করো", "সমস্যা"]) {
        Kind::Bugfix
    } else if lower.trim_end().ends_with('?') || has(&["why ", "what ", "how does", "explain", "where is"]) {
        Kind::Question
    } else {
        Kind::Edit
    }
}

/// The file paths a request names, and the identifiers (snake_case or CamelCase) it mentions.
pub fn mentions(request: &str) -> (usize, usize) {
    let mut files = std::collections::BTreeSet::new();
    let mut symbols = std::collections::BTreeSet::new();

    for word in request.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '"' | '\'' | '`')) {
        let word = word.trim_matches(|c: char| matches!(c, '.' | ':' | '?' | '!'));
        let extension = word.rsplit_once('.').map(|(_, extension)| extension);

        if word.contains('/') || extension.is_some_and(|extension| (1..=5).contains(&extension.len()) && extension.chars().all(|c| c.is_ascii_alphanumeric()) && word.len() > extension.len() + 1 && !extension.chars().all(|c| c.is_ascii_digit())) {
            files.insert(word.to_string());
        } else if word.len() >= 5 && (word.contains('_') && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && word.chars().skip(1).any(|c| c.is_ascii_uppercase()) && word.chars().all(char::is_alphanumeric)) {
            symbols.insert(word.to_string());
        }
    }

    (files.len(), symbols.len())
}

/// Where a project remembers which kinds of step needed the chain.
fn history_file(root: &Path) -> std::path::PathBuf {
    root.join(".sdc").join("relay").join("kinds.txt")
}

/// The key of a step for the history: its kind and the extension of the first file it changed.
pub fn history_key(kind: Kind, changed: &[String]) -> String {
    let extension = changed.first().and_then(|path| path.rsplit_once('.')).map_or("none", |(_, extension)| extension);

    format!("{kind:?}.{extension}").to_lowercase()
}

pub fn remember_relayed(root: &Path, key: &str) {
    let file = history_file(root);

    if relayed_before(root, key) {
        return;
    }

    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Ok(mut handle) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        use std::io::Write;

        let _ = writeln!(handle, "{key}");
    }
}

pub fn relayed_before(root: &Path, key: &str) -> bool {
    std::fs::read_to_string(history_file(root)).is_ok_and(|text| text.lines().any(|line| line.trim() == key))
}

/// The facts for a request in a project, for a model of this class.
pub fn facts_for(root: &Path, request: &str, class: Class) -> Facts {
    let kind = kind_of(request);
    let (files, symbols) = mentions(request);
    /* The extension of the first file named stands in for "what would be changed" before anything is. */
    let named: Vec<String> = request.split_whitespace().filter(|word| word.contains('.')).map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '/').to_string()).collect();

    Facts { kind, files, symbols, class, relayed_before: relayed_before(root, &history_key(kind, &named)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(kind: Kind, files: usize, symbols: usize, class: Class, before: bool) -> Facts {
        Facts { kind, files, symbols, class, relayed_before: before }
    }

    #[test]
    fn the_score_is_a_sum_of_parts_a_person_can_read() {
        assert_eq!(score(&facts(Kind::Question, 0, 0, Class::Large, false)), 0);
        assert_eq!(score(&facts(Kind::Edit, 1, 1, Class::Medium, false)), 2 + 2 + 1 + 1);
        assert_eq!(score(&facts(Kind::Refactor, 3, 2, Class::Small, true)), 4 + 6 + 2 + 2 + 3);
        assert_eq!(score(&facts(Kind::Edit, 50, 50, Class::Large, false)), 2 + 16 + 8, "a long list does not run away");
    }

    #[test]
    fn only_a_plainly_hard_change_is_pre_routed_and_never_a_question() {
        assert!(!plainly_beyond(&facts(Kind::Bugfix, 1, 1, Class::Medium, false)));
        assert!(plainly_beyond(&facts(Kind::Refactor, 3, 2, Class::Small, false)));
        assert!(!plainly_beyond(&facts(Kind::Question, 8, 8, Class::Small, true)), "a question changes nothing");
        assert_eq!(difficulty(&facts(Kind::Edit, 1, 0, Class::Large, false)), Difficulty::Easy);
        assert_eq!(difficulty(&facts(Kind::Edit, 2, 0, Class::Medium, false)), Difficulty::Medium);
    }

    #[test]
    fn the_class_of_the_model_moves_the_line() {
        let same = |class| facts(Kind::Edit, 2, 1, class, false);

        assert_eq!(difficulty(&same(Class::Large)), Difficulty::Medium);
        assert_eq!(difficulty(&same(Class::Small)), Difficulty::Hard, "the same step is harder for a small window");
    }

    #[test]
    fn a_request_is_read_into_kind_files_and_symbols() {
        assert_eq!(kind_of("rename charge_customer across the billing package"), Kind::Refactor);
        assert_eq!(kind_of("fix the failing test in src/pay.py"), Kind::Bugfix);
        assert_eq!(kind_of("why does the header flicker?"), Kind::Question);
        assert_eq!(kind_of("add a dark mode toggle"), Kind::Edit);
        assert_eq!(mentions("update src/pay.py and billing/render.py so charge_customer and RefundHandler agree"), (2, 2));
        assert_eq!(mentions("make it faster"), (0, 0));
        assert_eq!(mentions("version 1.0 of the page"), (0, 0), "a number with a dot is not a file");
    }

    #[test]
    fn a_kind_that_needed_the_chain_before_is_remembered_in_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let key = history_key(Kind::Refactor, &["src/pay.py".to_string()]);

        assert_eq!(key, "refactor.py");
        assert!(!relayed_before(dir.path(), &key));

        remember_relayed(dir.path(), &key);
        remember_relayed(dir.path(), &key);

        assert!(relayed_before(dir.path(), &key));
        assert_eq!(std::fs::read_to_string(dir.path().join(".sdc/relay/kinds.txt")).unwrap().lines().count(), 1, "once");

        let seen = facts_for(dir.path(), "rename x across the code in a.py", Class::Medium);

        assert!(seen.relayed_before);
    }
}
