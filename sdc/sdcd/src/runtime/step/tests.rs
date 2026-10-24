use super::*;

fn project(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sdc-step-{name}-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn a() -> u32 { 1 }\n").unwrap();
    std::fs::write(root.join("src/b.py"), "def b():\n    return 2\n").unwrap();
    root
}

fn edit(path: &str, text: &str) -> (String, Option<String>) {
    (path.to_string(), Some(text.to_string()))
}

#[test]
fn a_step_of_valid_edits_is_committed_whole_and_leaves_no_journal_behind() {
    let root = project("commit");
    let changed = apply(&root, "t1-1", &[edit("src/a.rs", "fn a() -> u32 { 10 }\n"), edit("src/c.rs", "fn c() {}\n"), ("src/b.py".into(), None)]).unwrap();

    assert_eq!(changed.len(), 3);
    assert_eq!(std::fs::read_to_string(root.join("src/a.rs")).unwrap(), "fn a() -> u32 { 10 }\n");
    assert!(root.join("src/c.rs").exists() && !root.join("src/b.py").exists());
    assert!(!root.join(".sdc/tx/t1-1").exists(), "the transaction cleaned up after itself");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn one_file_that_would_not_parse_refuses_the_whole_step_and_nothing_is_written() {
    let root = project("refuse");
    let refused = apply(&root, "t2-1", &[edit("src/a.rs", "fn a() -> u32 { 10 }\n"), edit("src/c.rs", "fn broken( {\n")]).unwrap_err();

    assert!(refused.0.contains("src/c.rs") && refused.0.contains("syntax error at line 1") && refused.0.contains("nothing in this step was changed"), "{}", refused.0);
    assert_eq!(std::fs::read_to_string(root.join("src/a.rs")).unwrap(), "fn a() -> u32 { 1 }\n", "the valid file of the same step was not written either");
    assert!(!root.join("src/c.rs").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_file_that_was_already_broken_may_be_written_and_an_unknown_language_is_never_judged() {
    let root = project("lenient");

    std::fs::write(root.join("src/a.rs"), "fn a( {\n").unwrap();
    apply(&root, "t3-1", &[edit("src/a.rs", "fn a( { still broken\n")]).unwrap();
    apply(&root, "t3-2", &[edit("notes.txt", "fn ( not code {\n"), edit("data.unknownext", "}}}{{{")]).unwrap();

    assert!(root.join("notes.txt").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn sending_the_same_text_again_after_a_refusal_writes_it_so_a_grammar_gap_cannot_deadlock() {
    let root = project("insist");
    let text = "fn odd( {\n";

    /* with `nr.edit_gate` off, the rule of 1.0: the same text, sent again, is written */
    crate::v3::ablating(&["nr.edit_gate"], || {
        assert!(apply(&root, "t4-1", &[edit("src/odd.rs", text)]).is_err());
        assert!(!root.join("src/odd.rs").exists());
        assert!(apply(&root, "t4-2", &[edit("src/odd.rs", text)]).is_ok(), "the same text, sent again, is written");
        assert!(apply(&root, "t4-3", &[edit("src/odd2.rs", "fn also( {\n")]).is_err(), "but a different text is judged afresh");
    });

    /* with it on, that is still so for a language whose grammar is not trusted (PHP templates), and closed for Rust */
    crate::v3::forcing(&["nr.edit_gate"], || {
        let php = "<?php function odd( {\n";

        assert!(apply(&root, "t4-4", &[edit("src/odd.php", php)]).is_err());
        assert!(apply(&root, "t4-5", &[edit("src/odd.php", php)]).is_ok());
        assert!(apply(&root, "t4-6", &[edit("src/odd3.rs", text)]).is_err());
        assert!(apply(&root, "t4-7", &[edit("src/odd3.rs", text)]).is_err(), "a broken Rust edit is refused every time");
        assert!(!root.join("src/odd3.rs").exists());
    });

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn json_and_python_are_gated_too() {
    let root = project("langs");

    assert!(apply(&root, "t5-1", &[edit("package.json", "{\"name\": \"x\",}")]).is_err());
    assert!(apply(&root, "t5-2", &[edit("package.json", "{\"name\": \"x\"}")]).is_ok());
    assert!(apply(&root, "t5-3", &[edit("src/b.py", "def b(:\n    pass\n")]).is_err());
    assert_eq!(std::fs::read_to_string(root.join("src/b.py")).unwrap(), "def b():\n    return 2\n");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_gate_decision_is_a_pure_function_of_old_and_new() {
    assert_eq!(gate("a.rs", Some("fn a() {}"), "fn a() { 1 }"), None);
    assert!(gate("a.rs", Some("fn a() {}"), "fn a( {").is_some());
    assert_eq!(gate("a.rs", Some("fn a( {"), "fn a( { x"), None, "already broken: let a repair through");
    assert_eq!(gate("a.txt", None, "fn ( {"), None);
    assert_eq!(gate("a.sh", None, "if then fi ("), None, "Bash is not judged");
}
