//! **The Handover Note (M1-5)**: what the chain model did, told to the local model that carries on.
//!
//! The note is written by **code from the diff**, not by a model and not from the chain model's own account of itself: the files that
//! changed, the declarations in them that were added, changed or removed (the Code Graph's hashes before and after), and the checks that
//! passed. Three to six lines, so it costs a small window next to nothing. The local model's next step sees it; the chat's own model setting
//! is never touched.

use std::collections::BTreeMap;

use super::executor::Verified;
use crate::graph::parse::{definitions, parse, Lang};

const MAX_SYMBOLS: usize = 6;

/// How the declarations of one file differ between two texts: `name changed`, `name added`, `name removed`. Empty for a language the graph
/// does not read, and for a file whose text did not change.
pub fn symbol_changes(path: &str, before: &str, after: &str) -> Vec<String> {
    let Some(lang) = Lang::of(path) else {
        return Vec::new();
    };
    let table = |text: &str| -> BTreeMap<String, String> {
        parse(lang, text).map(|tree| definitions(lang, &tree, text).into_iter().map(|def| (def.qualified, def.hash)).collect()).unwrap_or_default()
    };
    let (old, new) = (table(before), table(after));
    let mut found = Vec::new();

    for (name, hash) in &new {
        match old.get(name) {
            None => found.push(format!("{name} added")),
            Some(known) if known != hash => found.push(format!("{name} changed")),
            Some(_) => {}
        }
    }

    for name in old.keys().filter(|name| !new.contains_key(*name)) {
        found.push(format!("{name} removed"));
    }

    found
}

/// The note, from the changes (`(path, symbol changes)`), the goal and what the checks said.
pub fn note(model: &str, goal: &str, changes: &[(String, Vec<String>)], verified: &Verified) -> String {
    let mut lines = vec![format!("[SDC handover: {model} did the step you could not, and checked it. Carry on from here; do not redo it.]")];
    let goal: String = goal.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect();

    lines.push(format!("- step: {goal}"));

    if changes.is_empty() {
        lines.push("- changed: no files".to_string());
    } else {
        let shown: Vec<String> = changes
            .iter()
            .take(6)
            .map(|(path, symbols)| if symbols.is_empty() { path.clone() } else { format!("{path} ({})", symbols.iter().take(MAX_SYMBOLS).cloned().collect::<Vec<_>>().join(", ")) })
            .collect();
        let more = changes.len().saturating_sub(6);

        lines.push(format!("- changed: {}{}", shown.join("; "), if more > 0 { format!("; and {more} more") } else { String::new() }));
    }

    lines.push(match verified {
        Verified::Green(checks) if checks.is_empty() => "- checks: passed".to_string(),
        Verified::Green(checks) => format!("- checks passed: {}", checks.iter().map(|check| format!("`{check}`")).collect::<Vec<_>>().join(", ")),
        Verified::NoChecks => "- checks: this folder has none, so the change is not verified".to_string(),
        Verified::Red(_) => "- checks: failing".to_string(),
    });

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_are_compared_by_their_hash() {
        let before = "def charge(amount):\n    return amount * 2\n\ndef refund(amount):\n    return -amount\n";
        let after = "def charge(amount):\n    return round(amount * 2)\n\ndef round_cents(x):\n    return x\n";
        let changes = symbol_changes("pay.py", before, after);

        assert_eq!(changes, ["charge changed", "round_cents added", "refund removed"]);
        assert!(symbol_changes("pay.py", before, before).is_empty(), "nothing changed");
        assert!(symbol_changes("notes.txt", "a", "b").is_empty(), "a language the graph does not read says nothing");
        assert!(symbol_changes("pay.py", "def charge(amount):\n    return 1\n", "def charge(amount):\n    return 1\n\n\n").is_empty(), "blank lines after a declaration are not a change of it");
    }

    #[test]
    fn the_note_names_the_files_the_symbols_and_the_checks_in_a_few_lines() {
        let changes = vec![("src/pay.py".to_string(), vec!["charge changed".to_string(), "round_cents added".to_string()]), ("README.md".to_string(), vec![])];
        let note = note("deepseek:deepseek-chat", "make   charge\nround to cents", &changes, &Verified::Green(vec!["python -m pytest -q".into()]));
        let lines: Vec<&str> = note.lines().collect();

        assert!(lines.len() <= 6, "{note}");
        assert!(lines[0].contains("deepseek:deepseek-chat") && lines[0].contains("do not redo it"));
        assert_eq!(lines[1], "- step: make charge round to cents", "whitespace is folded");
        assert_eq!(lines[2], "- changed: src/pay.py (charge changed, round_cents added); README.md");
        assert_eq!(lines[3], "- checks passed: `python -m pytest -q`");
    }

    #[test]
    fn a_change_no_check_could_confirm_is_said_to_be_unverified() {
        let note = note("a:b", "goal", &[], &Verified::NoChecks);

        assert!(note.contains("changed: no files") && note.contains("not verified"), "{note}");
    }

    #[test]
    fn a_long_list_is_cut_and_counted() {
        let changes: Vec<(String, Vec<String>)> = (0..9).map(|n| (format!("f{n}.rs"), vec![])).collect();
        let note = note("a:b", "goal", &changes, &Verified::Green(vec![]));

        assert!(note.contains("f5.rs") && !note.contains("f6.rs") && note.contains("and 3 more"), "{note}");
    }
}
