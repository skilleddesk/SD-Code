//! **N4-3 - the parse/lint edit gate** (switch `nr.edit_gate`, on).
//!
//! The Step Runtime already refuses an edit that newly breaks a file's syntax, but lets the *same text sent twice* through, so a grammar's
//! false alarm cannot deadlock an agent. For the languages whose grammar is reliable that bypass is closed here: a broken edit never
//! reaches the disk. A repair of a file that was already broken is still allowed, and a language without a trusted grammar keeps the old
//! rule. A fast linter, when the project configures one, is asked after the parse and may refuse too.

use crate::graph::parse::Lang;

/// Languages whose parse errors are trusted without an escape hatch. PHP (templates) and shell (heredocs) are left out on purpose.
pub fn strict_language(path: &str) -> bool {
    Lang::of(path).is_some_and(|lang| matches!(lang, Lang::Rust | Lang::Python | Lang::JavaScript | Lang::TypeScript | Lang::Tsx | Lang::Go | Lang::Json))
}

/// May the same refused text be written when it is sent again? Only where the gate is not strict, or the switch is off.
pub fn may_insist(path: &str) -> bool {
    !(crate::v3::on("nr.edit_gate") && strict_language(path))
}

/// A fast linter: path and new text in, `Some(message)` to refuse.
pub type Linter<'a> = &'a dyn Fn(&str, &str) -> Option<String>;

/// The fast linter's verdict on a new file: `Some(message)` refuses. The linter is whatever the caller configured (`ruff`, `eslint --quiet`,
/// ...); with none, nothing is said.
pub fn lint(path: &str, new: &str, linter: Option<Linter<'_>>) -> Option<String> {
    linter.and_then(|run| run(path, new))
}

/// Python's grammar accepts `def f():` followed by a line at the same indent as an empty body; that is an IndentationError at run time.
/// `Some(line)` is the first block with nothing in it.
pub fn empty_python_block(path: &str, text: &str) -> Option<usize> {
    if Lang::of(path) != Some(Lang::Python) {
        return None;
    }

    let tree = crate::graph::parse::parse(Lang::Python, text)?;
    let mut stack = vec![tree.root_node()];

    while let Some(node) = stack.pop() {
        if node.kind() == "block" && node.start_byte() == node.end_byte() {
            return Some(node.start_position().row + 1);
        }

        let mut cursor = node.walk();

        stack.extend(node.children(&mut cursor));
    }

    None
}

/// The whole gate for one file in memory: parse first, then the linter. `Ok(())` lets the edit through.
pub fn check(path: &str, old: Option<&str>, new: &str, linter: Option<Linter<'_>>) -> Result<(), String> {
    if let Some(problem) = crate::runtime::step::gate(path, old, new) {
        return Err(problem);
    }

    match lint(path, new, linter) {
        Some(problem) => Err(format!("{path}: {problem}")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BROKEN: [(&str, &str); 5] = [
        ("a.rs", "fn main() {\n    let x = 1;\n"),
        ("a.py", "def f():\nreturn 1\n"),
        ("a.py", "def f():\n    x = 'unterminated\n    return x\n"),
        ("a.ts", "export function f( {\n  return 1;\n}\n"),
        ("a.json", "{ \"a\": 1,, }"),
    ];

    #[test]
    fn broken_braces_bad_indentation_and_unterminated_strings_are_all_refused() {
        for (path, text) in BROKEN {
            assert!(check(path, None, text, None).is_err(), "{path}: {text:?} passed the gate");
        }

        assert!(check("a.rs", Some("fn a() {}"), "fn a() { let b = 1; }", None).is_ok());
    }

    #[test]
    fn the_send_it_again_bypass_is_closed_for_trusted_grammars_and_kept_elsewhere() {
        crate::v3::forcing(&["nr.edit_gate"], || {
            assert!(!may_insist("src/a.rs") && !may_insist("a.py") && !may_insist("a.ts") && !may_insist("a.json"));
            assert!(may_insist("page.php") && may_insist("run.sh") && may_insist("notes.md"));
        });
        crate::v3::ablating(&["nr.edit_gate"], || assert!(may_insist("src/a.rs"), "with the switch off the old rule stands"));
    }

    #[test]
    fn a_broken_file_edit_never_reaches_the_disk_even_when_sent_twice() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();

        crate::v3::forcing(&["nr.edit_gate"], || {
            for _ in 0..3 {
                let edit = vec![("a.rs".to_string(), Some("fn a() {\n".to_string()))];

                assert!(crate::runtime::step::apply(dir.path(), "tx1", &edit).is_err());
            }

            assert_eq!(std::fs::read_to_string(dir.path().join("a.rs")).unwrap(), "fn a() {}\n");
        });
    }

    #[test]
    fn a_configured_linter_can_refuse_after_a_clean_parse() {
        let linter = |_: &str, text: &str| text.contains("TODO").then(|| "TODO left in".to_string());

        assert!(check("a.rs", None, "fn a() { /* TODO */ }", Some(&linter)).is_err());
        assert!(check("a.rs", None, "fn a() {}", Some(&linter)).is_ok());
    }
}
