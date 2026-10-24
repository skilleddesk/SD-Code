use super::*;
use crate::navaratna::errslice::{Found, Tool};

fn found(message: &str) -> Found {
    Found { tool: Tool::Rustc, message: message.into(), file: Some("src/a.rs".into()), line: Some(7), expected: None, actual: None, total: 1 }
}

fn fix(turn: &str) -> Fix {
    Fix {
        signature: signature(&found("error[E0308]: mismatched types"), "fn", "src/a.rs"),
        sliced_error: "rustc error: mismatched types at src/a.rs:7".into(),
        final_diff: "-    let n: i32 = \"a\";\n+    let n: i32 = 1;".into(),
        turn: turn.into(),
        tests_passed: true,
        touched: vec!["src/a.rs".into()],
    }
}

fn no_secret_paths(_: &str) -> bool {
    false
}

#[test]
fn a_signature_is_error_class_symbol_kind_and_language() {
    assert_eq!(signature(&found("error[E0308]: mismatched types"), "fn", "src/a.rs"), "E0308|fn|rust");
    assert_eq!(signature(&Found { tool: Tool::Tsc, ..found("error TS2322: Type 'string' is not assignable") }, "method", "a.ts"), "TS2322|method|typescript");
    assert_eq!(signature(&Found { tool: Tool::Pytest, ..found("AssertionError: assert 3 == 4") }, "def", "t.py"), "AssertionError|def|python");
    assert_eq!(error_class(&found("undefined: widget")), "undefined: widget");
}

#[test]
fn a_repeated_fixture_problem_gets_its_example_after_two_witnesses_and_the_hit_is_counted() {
    crate::v3::forcing(&["nr.golden_trace"], || {
        let dir = tempfile::tempdir().unwrap();
        let sig = fix("t1").signature;

        assert_eq!(record(dir.path(), &fix("t1"), &no_secret_paths, false), Recorded::Stored { witnesses: 1 });
        assert_eq!(lookup(dir.path(), &sig), None, "one witness is a guess");
        assert_eq!(record(dir.path(), &fix("t1"), &no_secret_paths, false), Recorded::Stored { witnesses: 1 }, "the same turn is not a second witness");
        assert_eq!(record(dir.path(), &fix("t2"), &no_secret_paths, false), Recorded::Stored { witnesses: 2 });

        let example = lookup(dir.path(), &sig).expect("two turns saw it");

        assert!(example.render().contains("mismatched types") && example.render().contains("+    let n: i32 = 1;"));
        assert_eq!(lookup(dir.path(), "E0599|fn|rust"), None, "another signature, no example");
        assert_eq!(hit_count(dir.path()), 1);
        lookup(dir.path(), &sig);
        assert_eq!(hit_count(dir.path()), 2);
    });
}

#[test]
fn at_most_one_example_is_handed_out() {
    crate::v3::forcing(&["nr.golden_trace"], || {
        let dir = tempfile::tempdir().unwrap();
        let other = Fix { final_diff: "-    old()\n+    new_way()".into(), ..fix("t1") };

        for turn in ["t1", "t2"] {
            record(dir.path(), &fix(turn), &no_secret_paths, false);
            record(dir.path(), &Fix { turn: turn.into(), ..other.clone() }, &no_secret_paths, false);
        }

        assert!(lookup(dir.path(), &fix("t1").signature).is_some());
        assert_eq!(hit_count(dir.path()), 1, "one example, one hit");
    });
}

#[test]
fn an_example_followed_by_a_failure_twice_is_dropped() {
    crate::v3::forcing(&["nr.golden_trace"], || {
        let dir = tempfile::tempdir().unwrap();
        let sig = fix("t1").signature;

        record(dir.path(), &fix("t1"), &no_secret_paths, false);
        record(dir.path(), &fix("t2"), &no_secret_paths, false);

        let example = lookup(dir.path(), &sig).unwrap();

        assert!(!followed_by_failure(dir.path(), example.id), "one strike");
        assert!(lookup(dir.path(), &sig).is_some(), "still offered after one");
        assert!(followed_by_failure(dir.path(), example.id), "two strikes");
        assert_eq!(lookup(dir.path(), &sig), None, "dropped");
    });
}

#[test]
fn a_failed_fix_a_secret_and_a_local_only_secret_path_are_never_stored() {
    crate::v3::forcing(&["nr.golden_trace"], || {
        let dir = tempfile::tempdir().unwrap();
        let fake = format!("sk-{}", "a1B2c3D4".repeat(6));

        assert!(matches!(record(dir.path(), &Fix { tests_passed: false, ..fix("t1") }, &no_secret_paths, false), Recorded::Refused(why) if why.contains("passed every test")));
        assert!(matches!(record(dir.path(), &Fix { final_diff: format!("+let key = \"{fake}\";"), ..fix("t1") }, &no_secret_paths, false), Recorded::Refused(why) if why.contains("secret")));
        assert!(matches!(record(dir.path(), &Fix { sliced_error: "token ‹secret:3› rejected".into(), ..fix("t1") }, &no_secret_paths, false), Recorded::Refused(_)));
        assert!(matches!(record(dir.path(), &Fix { touched: vec![".env".into()], ..fix("t1") }, &|p| p == ".env", true), Recorded::Refused(why) if why.contains("secret path")));
        assert!(matches!(record(dir.path(), &Fix { touched: vec![".env".into()], ..fix("t1") }, &|p| p == ".env", false), Recorded::Stored { .. }), "a project that is not local-only may keep it");

        let stored = std::fs::read(dir.path().join(FILE)).unwrap_or_default();

        assert!(!String::from_utf8_lossy(&stored).contains(&fake), "no secret in the database file");
    });
}

#[test]
fn off_by_default_nothing_is_stored_or_offered() {
    let dir = tempfile::tempdir().unwrap();

    assert!(matches!(record(dir.path(), &fix("t1"), &no_secret_paths, false), Recorded::Refused(_)));
    assert_eq!(lookup(dir.path(), &fix("t1").signature), None);
    assert!(!dir.path().join(FILE).exists());
}
