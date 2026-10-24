use super::*;

/// A project of two files and one anchor test; the "runner" interprets the tests it is given.
fn base() -> Tree {
    Tree::from([
        ("src/calc.py".to_string(), "def add(a, b):\n    return a + b\n\ndef double(x):\n    raise NotImplementedError\n".to_string()),
        ("tests/test_anchor.py".to_string(), "def test_add(): assert add(1, 2) == 3\n".to_string()),
    ])
}

fn anchors() -> Anchors {
    Anchors { ids: BTreeSet::from(["tests/test_anchor.py::test_add".to_string()]), files: BTreeSet::from(["tests/test_anchor.py".to_string()]) }
}

fn step() -> Step {
    Step { id: "s1".into(), tag: Tag::Code, done_when: "double(3) returns 6 and double(0) returns 0".into(), spec: "implement double(x)".into() }
}

/// Scripted parties. The runner decides a test's result from the code in the tree: `double` is implemented when the text has `return x * 2`
/// (right) or `return x + 3` (wrong).
struct Fake {
    tests: Vec<TestFile>,
    code: Vec<(String, String)>,
    saw_in_code_call: Vec<String>,
    runs: usize,
}

impl Parties for Fake {
    fn write_tests(&mut self, _done_when: &str) -> Vec<TestFile> {
        self.tests.clone()
    }

    fn write_code(&mut self, spec: &str) -> Vec<(String, String)> {
        self.saw_in_code_call.push(spec.to_string());
        self.code.clone()
    }

    fn run(&mut self, tree: &Tree, ids: &[String]) -> BTreeMap<String, bool> {
        self.runs += 1;

        let calc = tree.get("src/calc.py").cloned().unwrap_or_default();
        let double = |x: i64| -> Option<i64> {
            if calc.contains("return x * 2") {
                Some(x * 2)
            } else if calc.contains("return x + 3") {
                Some(x + 3)
            } else {
                None
            }
        };

        ids.iter()
            .map(|id| {
                let passed = match id.as_str() {
                    "tests/test_anchor.py::test_add" => !calc.contains("def add(a, b):\n    return 0") && tree.get("tests/test_anchor.py").is_some_and(|t| t.contains("assert add(1, 2) == 3")),
                    "tests/test_double.py::test_three" => double(3) == Some(6),
                    "tests/test_double.py::test_zero" => double(0) == Some(0),
                    "tests/test_double.py::test_vacuous" => true,
                    "tests/test_double.py::test_wrong" => double(2) == Some(5),
                    _ => false,
                };

                (id.clone(), passed)
            })
            .collect()
    }
}

fn test_file(names: &[&str]) -> TestFile {
    TestFile { path: "tests/test_double.py".into(), content: "# tests".into(), names: names.iter().map(|n| format!("tests/test_double.py::{n}")).collect() }
}

fn fake(names: &[&str], code: &str) -> Fake {
    Fake { tests: vec![test_file(names)], code: vec![("src/calc.py".into(), code.into())], saw_in_code_call: Vec::new(), runs: 0 }
}

const GOOD: &str = "def add(a, b):\n    return a + b\n\ndef double(x):\n    return x * 2\n";

#[test]
fn good_code_with_real_tests_ends_green_and_every_anchor_is_untouched() {
    crate::v3::forcing(&["nr.test_first"], || {
        let mut parties = fake(&["test_three", "test_zero"], GOOD);
        let outcome = run_step(&step(), &base(), &anchors(), &mut parties);
        let Outcome::Done { tree, new_tests, discarded_vacuous } = outcome else { panic!("{outcome:?}") };

        assert_eq!(new_tests.len(), 2);
        assert!(discarded_vacuous.is_empty());
        assert_eq!(tree["tests/test_anchor.py"], base()["tests/test_anchor.py"]);
        assert!(tree["src/calc.py"].contains("x * 2") && tree.contains_key("tests/test_double.py"));
        assert_eq!(parties.runs, 2, "once on the stub, once after the code");
    });
}

#[test]
fn the_code_call_is_given_the_spec_and_never_the_tests() {
    crate::v3::forcing(&["nr.test_first"], || {
        let mut parties = fake(&["test_three"], GOOD);

        run_step(&step(), &base(), &anchors(), &mut parties);

        assert_eq!(parties.saw_in_code_call, vec!["implement double(x)".to_string()]);
    });
}

#[test]
fn a_test_that_passes_on_the_stub_is_vacuous_and_is_discarded() {
    crate::v3::forcing(&["nr.test_first"], || {
        let mut parties = fake(&["test_three", "test_vacuous"], GOOD);
        let Outcome::Done { new_tests, discarded_vacuous, .. } = run_step(&step(), &base(), &anchors(), &mut parties) else { panic!() };

        assert_eq!(new_tests, vec!["tests/test_double.py::test_three".to_string()]);
        assert_eq!(discarded_vacuous, vec!["tests/test_double.py::test_vacuous".to_string()]);

        let mut all_vacuous = fake(&["test_vacuous"], GOOD);

        assert!(matches!(run_step(&step(), &base(), &anchors(), &mut all_vacuous), Outcome::Skipped(reason) if reason.contains("passed on the stub")));
    });
}

#[test]
fn code_that_edits_or_deletes_an_anchor_test_is_refused_and_the_tree_is_untouched() {
    crate::v3::forcing(&["nr.test_first"], || {
        let mut parties = fake(&["test_three"], GOOD);

        parties.code.push(("tests/test_anchor.py".into(), "def test_add(): assert True\n".into()));

        let outcome = run_step(&step(), &base(), &anchors(), &mut parties);

        assert!(matches!(&outcome, Outcome::AnchorViolation { detail } if detail.contains("tests/test_anchor.py")), "{outcome:?}");

        /* deleting is writing nothing: an empty replacement of an anchor file is still a write to it */
        let mut deleting = fake(&["test_three"], GOOD);

        deleting.code.push(("tests/test_anchor.py".into(), String::new()));

        assert!(matches!(run_step(&step(), &base(), &anchors(), &mut deleting), Outcome::AnchorViolation { .. }));
    });
}

#[test]
fn a_model_written_test_file_cannot_replace_an_anchor_file() {
    crate::v3::forcing(&["nr.test_first"], || {
        let mut parties = fake(&["test_three"], GOOD);

        parties.tests.push(TestFile { path: "tests/test_anchor.py".into(), content: "def test_add(): pass\n".into(), names: vec!["tests/test_anchor.py::test_add".into()] });

        let Outcome::Done { tree, .. } = run_step(&step(), &base(), &anchors(), &mut parties) else { panic!() };

        assert_eq!(tree["tests/test_anchor.py"], base()["tests/test_anchor.py"], "the replacement was dropped");
    });
}

#[test]
fn code_that_breaks_an_anchor_is_a_violation() {
    crate::v3::forcing(&["nr.test_first"], || {
        let mut parties = fake(&["test_three"], "def add(a, b):\n    return 0\n\ndef double(x):\n    return x * 2\n");
        let outcome = run_step(&step(), &base(), &anchors(), &mut parties);

        assert!(matches!(&outcome, Outcome::AnchorViolation { detail } if detail.contains("no longer pass")), "{outcome:?}");
    });
}

#[test]
fn a_wrong_test_that_keeps_failing_is_flagged_for_review_and_not_deleted() {
    crate::v3::forcing(&["nr.test_first"], || {
        let mut parties = fake(&["test_three", "test_wrong"], GOOD);
        let outcome = run_step(&step(), &base(), &anchors(), &mut parties);
        let Outcome::Flagged { tree, failing, .. } = outcome else { panic!("{outcome:?}") };

        assert_eq!(failing, vec!["tests/test_double.py::test_wrong".to_string()]);
        assert!(tree.contains_key("tests/test_double.py"), "the test file is kept for the person to read");
    });
}

#[test]
fn at_most_five_tests_and_none_at_all_means_skipped() {
    crate::v3::forcing(&["nr.test_first"], || {
        let mut many = fake(&["test_three", "test_zero", "test_wrong", "a", "b", "c", "d"], GOOD);
        let Outcome::Flagged { failing, .. } = run_step(&step(), &base(), &anchors(), &mut many) else { panic!() };

        assert!(failing.len() <= MAX_TESTS);

        let mut none = Fake { tests: vec![], ..fake(&[], GOOD) };

        assert!(matches!(run_step(&step(), &base(), &anchors(), &mut none), Outcome::Skipped(_)));
    });
}

#[test]
fn trivial_steps_off_and_no_done_when_skip_test_first() {
    let mut parties = fake(&["test_three"], GOOD);

    assert!(matches!(run_step(&step(), &base(), &anchors(), &mut parties), Outcome::Skipped(reason) if reason.contains("off")), "off by default");

    crate::v3::forcing(&["nr.test_first"], || {
        assert!(matches!(run_step(&Step { tag: Tag::Trivial, ..step() }, &base(), &anchors(), &mut parties), Outcome::Skipped(_)));
        assert!(matches!(run_step(&Step { done_when: " ".into(), ..step() }, &base(), &anchors(), &mut parties), Outcome::Skipped(_)));
    });

    assert_eq!(parties.runs, 0, "no test was run for any of them");
}
