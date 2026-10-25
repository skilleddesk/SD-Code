use std::collections::{BTreeMap, VecDeque};

use super::*;
use crate::navaratna::spec;

/// A model that answers from a queue, whatever it is asked, and keeps every request.
#[derive(Default)]
struct Scripted {
    replies: VecDeque<Result<String, String>>,
    seen: Vec<Request>,
}

impl Scripted {
    fn with(replies: &[&str]) -> Self {
        Self { replies: replies.iter().map(|reply| Ok(reply.to_string())).collect(), seen: Vec::new() }
    }
}

impl Engine for Scripted {
    fn complete(&mut self, request: &Request) -> Result<String, String> {
        self.seen.push(request.clone());
        self.replies.pop_front().unwrap_or_else(|| Err("the script ran out".into()))
    }

    fn name(&self) -> String {
        "ollama:fake".into()
    }
}

/// A project in memory. A check passes when every `(path, text)` rule is satisfied; `escalation` is what the relay would answer.
struct Fake {
    root: PathBuf,
    files: BTreeMap<String, String>,
    rules: Vec<(String, String)>,
    has_checks: bool,
    escalation: Option<Check>,
    test_runner: bool,
    log: Vec<String>,
    applied: Vec<String>,
    initial: BTreeMap<String, String>,
}

impl Fake {
    fn new(root: &std::path::Path) -> Self {
        let files = BTreeMap::from([
            ("src/shop.py".to_string(), "def total(items):\n    return 0\n".to_string()),
            ("src/cart.py".to_string(), "def count(items):\n    return 0\n".to_string()),
            ("tests/test_old.py".to_string(), "def test_old():\n    assert True\n".to_string()),
        ]);

        Self { root: root.to_path_buf(), initial: files.clone(), files, rules: Vec::new(), has_checks: true, escalation: None, test_runner: false, log: Vec::new(), applied: Vec::new() }
    }

    fn must(mut self, path: &str, text: &str) -> Self {
        self.rules.push((path.to_string(), text.to_string()));
        self
    }
}

const RUSTC_RED: &str = "error[E0308]: mismatched types\n --> src/shop.py:2:5\n  |\nerror: could not compile `demo` due to 1 previous error\n";

impl Project for Fake {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn read(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }

    fn files(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    fn apply(&mut self, step: &str, writes: &[(String, String)]) -> Result<(), String> {
        for (path, text) in writes {
            if let Some(problem) = crate::runtime::step::gate(path, self.files.get(path).map(String::as_str), text) {
                return Err(problem);
            }
        }

        for (path, text) in writes {
            self.files.insert(path.clone(), text.clone());
        }

        self.applied.push(step.to_string());
        Ok(())
    }

    fn put(&mut self, writes: &[(String, Option<String>)]) {
        for (path, text) in writes {
            match text {
                Some(text) => {
                    self.files.insert(path.clone(), text.clone());
                }
                None => {
                    self.files.remove(path);
                }
            }
        }
    }

    fn checks(&mut self) -> Check {
        if !self.has_checks {
            return Check::default();
        }

        /* a rule binds a file only once a step has changed it */
        let failed = self.rules.iter().find(|(path, text)| self.files.get(path).is_some_and(|content| Some(content) != self.initial.get(path) && !content.contains(text.as_str())));

        Check { ran: true, passed: failed.is_none(), log: if failed.is_some() { RUSTC_RED.to_string() } else { "ok".to_string() } }
    }

    fn run_tests(&mut self, writes: &[(String, String)], ids: &[String]) -> Option<BTreeMap<String, bool>> {
        if !self.test_runner {
            return None;
        }

        let mut tree = self.files.clone();

        for (path, text) in writes {
            tree.insert(path.clone(), text.clone());
        }

        let shop = tree.get("src/shop.py").cloned().unwrap_or_default();

        Some(ids.iter().map(|id| (id.clone(), shop.contains("sum(items)") && id.ends_with("test_total"))).collect())
    }

    fn escalate(&mut self, _step: &PlanStep, _error: &str, _touched: &[String]) -> Option<Check> {
        self.escalation.clone()
    }

    fn progress(&mut self, line: &str) {
        self.log.push(line.to_string());
    }
}

const PROMPT: &str = "The shop is for a small bakery.\n\nAdd a function total(items) in src/shop.py that returns the sum of the prices.\n\nAdd a function count(items) in src/cart.py that returns how many items there are.\n";

fn plan_of(prompt: &str) -> Plan {
    /* a model that labels nothing: the deterministic labeller makes the plan (the planner's own tests script the labels) */
    let mut engine = Scripted::default();

    plan::build(prompt, &mut engine, &|_| vec![]).unwrap()
}

fn edit(path: &str, symbol: &str, code: &str) -> String {
    format!("FILE: {path}\nSYMBOL: {symbol}\n```python\n{code}\n```")
}

const TOTAL_OK: &str = "def total(items):\n    return sum(items)";
const COUNT_OK: &str = "def count(items):\n    return len(items)";

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn a_plan_runs_step_by_step_each_in_a_fresh_prompt_and_the_next_step_gets_a_handover_note() {
    let root = dir();
    let plan = plan_of(PROMPT);
    let mut project = Fake::new(root.path()).must("src/shop.py", "sum(").must("src/cart.py", "len(");
    let mut engine = Scripted::with(&[&edit("src/shop.py", "total", TOTAL_OK), &edit("src/cart.py", "count", COUNT_OK)]);
    let (report, registry) = run(&plan, &mut engine, &mut project, &Config::default());

    assert!(report.steps.iter().all(|(_, result)| matches!(result, StepResult::Done { doneness: Doneness::Verified, .. })), "{:?}", report.steps);
    assert_eq!(report.local_share(), (2, 2));
    assert!(project.files["src/shop.py"].contains("sum(items)") && project.files["src/cart.py"].contains("len(items)"));

    /* fresh prompts: the second one holds the first step's handover and none of its chat */
    assert_eq!(engine.seen.len(), 2);
    assert!(engine.seen[1].prompt.contains("step-1 (R-001) done: src/shop.py: total"), "{}", engine.seen[1].prompt);
    assert!(!engine.seen[1].prompt.contains("return sum(items)\n```"), "no earlier answer is replayed");
    assert!(engine.seen[0].prompt.contains("small bakery") && engine.seen[0].prompt.ends_with("done when: Add a function total(items) in src/shop.py that returns the sum of the prices."), "{}", engine.seen[0].prompt);

    /* what a verified step left is recorded and locked */
    let receipt = registry::receipt(&["R-001".to_string(), "R-002".to_string()], &registry);

    assert_eq!(receipt[0].step, "step-1");
    assert!(receipt[0].evidence.starts_with("src/shop.py:1") && receipt[1].model == "ollama:fake", "{receipt:?}");
    assert!(report.render(&registry, &["R-001".into(), "R-002".into()]).contains("2 of 2 steps done, 2 of them by the local model"));
}

#[test]
fn a_failed_check_is_undone_cut_to_its_first_error_and_given_back_to_the_model_with_the_next_attempt() {
    let root = dir();
    let plan = plan_of(PROMPT);
    let mut project = Fake::new(root.path()).must("src/shop.py", "sum(").must("src/cart.py", "len(");
    let mut engine = Scripted::with(&[&edit("src/shop.py", "total", "def total(items):\n    return len(items)"), &edit("src/shop.py", "total", TOTAL_OK), &edit("src/cart.py", "count", COUNT_OK)]);
    let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

    assert!(matches!(&report.steps[0].1, StepResult::Done { attempts: 2, how: "local, retried", .. }), "{:?}", report.steps[0]);
    assert!(engine.seen[1].prompt.contains("checks failed after your edit (it was undone)") && engine.seen[1].prompt.contains("mismatched types"), "{}", engine.seen[1].prompt);
    assert_eq!(engine.seen[1].purpose, Purpose::Retry);
    assert!(project.files["src/shop.py"].contains("sum(items)"));
}

#[test]
fn bad_format_elision_and_a_locked_signature_come_back_as_reasons_the_model_can_act_on() {
    let root = dir();
    let plan = plan_of("Add a function total(items) in src/shop.py that returns the sum of the prices.\n");
    let mut project = Fake::new(root.path()).must("src/shop.py", "sum(");
    let mut engine = Scripted::with(&["I would just return the sum.", &edit("src/shop.py", "total", "def total(items):\n    # rest unchanged\n    return sum(items)"), &edit("src/shop.py", "total", TOTAL_OK)]);
    let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

    assert!(matches!(report.steps[0].1, StepResult::Done { attempts: 3, .. }), "{:?}", report.steps[0]);
    assert!(engine.seen[1].prompt.contains("holds no edit") && engine.seen[2].prompt.contains("leaves something out"), "{}\n---\n{}", engine.seen[1].prompt, engine.seen[2].prompt);
}

#[test]
fn a_step_that_cannot_be_done_is_blocked_with_the_tree_untouched_and_the_others_go_on() {
    let root = dir();
    let plan = plan_of(PROMPT);
    let mut project = Fake::new(root.path()).must("src/shop.py", "sum(").must("src/cart.py", "len(");
    let bad = edit("src/shop.py", "total", "def total(items):\n    return None");
    let mut engine = Scripted::with(&[&bad, &bad, &bad, &edit("src/cart.py", "count", COUNT_OK)]);
    let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

    assert!(matches!(&report.steps[0].1, StepResult::Blocked { reason } if reason.contains("3 attempt")), "{:?}", report.steps[0]);
    assert!(matches!(report.steps[1].1, StepResult::Done { .. }), "an independent step goes on");
    assert_eq!(project.files["src/shop.py"], "def total(items):\n    return 0\n", "the blocked step left nothing behind");
    assert_eq!(report.blocked().len(), 1);
    assert!(report.render(&registry::Registry::default(), &["R-001".into(), "R-002".into()]).contains("BLOCKED"));
}

#[test]
fn best_of_n_picks_the_candidate_the_checks_pass_and_only_after_the_retries_failed() {
    crate::v3::forcing(&["nr.best_of_n"], || {
        let root = dir();
        let plan = plan_of("Add a function total(items) in src/shop.py that returns the sum of the prices.\n");
        let mut project = Fake::new(root.path()).must("src/shop.py", "sum(");
        let bad = edit("src/shop.py", "total", "def total(items):\n    return None");
        let good = edit("src/shop.py", "total", TOTAL_OK);
        let mut engine = Scripted::with(&[&bad, &bad, &bad, &bad, &good, &bad]);
        let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

        assert!(matches!(&report.steps[0].1, StepResult::Done { how: "local, best of n", .. }), "{:?}", report.steps[0]);
        assert_eq!(engine.seen.iter().filter(|r| r.purpose == Purpose::Candidate).count(), 3);
        assert_eq!(engine.seen.iter().filter(|r| r.purpose == Purpose::Candidate).map(|r| r.temperature).collect::<Vec<_>>(), vec![Some(0.3), Some(0.7), Some(1.0)]);
        assert!(project.files["src/shop.py"].contains("sum(items)"));
    });
}

#[test]
fn without_the_switch_the_retries_are_the_last_local_chance() {
    let root = dir();
    let plan = plan_of("Add a function total(items) in src/shop.py that returns the sum of the prices.\n");
    let mut project = Fake::new(root.path()).must("src/shop.py", "sum(");
    let bad = edit("src/shop.py", "total", "def total(items):\n    return None");
    let mut engine = Scripted::with(&[&bad, &bad, &bad, &edit("src/shop.py", "total", TOTAL_OK)]);
    let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

    assert!(matches!(report.steps[0].1, StepResult::Blocked { .. }));
    assert_eq!(engine.seen.len(), 3);
}

#[test]
fn the_relay_is_asked_last_and_what_it_does_is_checked_and_counted_as_cloud() {
    let root = dir();
    let plan = plan_of("Add a function total(items) in src/shop.py that returns the sum of the prices.\n");
    let mut project = Fake::new(root.path()).must("src/shop.py", "sum(");

    project.escalation = Some(Check { ran: true, passed: true, log: String::new() });

    let bad = edit("src/shop.py", "total", "def total(items):\n    return None");
    let mut engine = Scripted::with(&[&bad, &bad, &bad]);
    let (report, registry) = run(&plan, &mut engine, &mut project, &Config::default());

    assert!(matches!(&report.steps[0].1, StepResult::Done { how: "relay", .. }), "{:?}", report.steps[0]);
    assert_eq!(report.local_share(), (0, 1));
    assert_eq!(registry.entries.iter().find(|entry| entry.step == "step-1").map(|entry| entry.model.as_str()), Some("chain"));

    /* a relay whose work still fails the checks does not rescue the step */
    let mut project = Fake::new(root.path()).must("src/shop.py", "sum(");

    project.escalation = Some(Check { ran: true, passed: false, log: String::new() });

    let mut engine = Scripted::with(&[&bad, &bad, &bad]);

    assert!(matches!(run(&plan, &mut engine, &mut project, &Config::default()).0.steps[0].1, StepResult::Blocked { .. }));
}

#[test]
fn a_folder_without_checks_is_done_but_never_called_proven_and_locks_nothing() {
    crate::v3::forcing(&["nr.contract_lock"], || {
        let root = dir();
        let plan = plan_of("Add a function total(items) in src/shop.py that returns the sum of the prices.\n");
        let mut project = Fake::new(root.path());

        project.has_checks = false;

        let mut engine = Scripted::with(&[&edit("src/shop.py", "total", TOTAL_OK)]);
        let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

        assert!(matches!(&report.steps[0].1, StepResult::Done { doneness: Doneness::Unverified, .. }));
        assert!(contracts::Lock::load(root.path()).signatures.is_empty(), "nothing unverified is locked");
        assert!(report.render(&registry::Registry::default(), &["R-001".into()]).contains("not proven"));
    });
}

#[test]
fn a_verified_step_locks_its_signature_and_a_later_step_cannot_change_it() {
    crate::v3::forcing(&["nr.contract_lock"], || {
        let root = dir();
        let plan = plan_of(PROMPT);
        let mut project = Fake::new(root.path()).must("src/shop.py", "sum(").must("src/cart.py", "len(");
        let changes_total = "FILE: src/shop.py\nSYMBOL: total\n```python\ndef total(items, tax):\n    return sum(items)\n```";
        let mut engine = Scripted::with(&[&edit("src/shop.py", "total", TOTAL_OK), changes_total, &edit("src/cart.py", "count", COUNT_OK)]);
        let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

        assert!(matches!(report.steps[0].1, StepResult::Done { .. }));
        assert!(contracts::Lock::load(root.path()).signatures.contains_key("src/shop.py::total"), "the verified step published its signature");

        /* step 2 tried to change total(): refused with the reason, the file untouched, and its second attempt did the real work */
        assert!(matches!(report.steps[1].1, StepResult::Done { attempts: 2, .. }), "{:?}", report.steps[1]);
        assert!(engine.seen[2].prompt.contains("locked contract") && engine.seen[2].prompt.contains("total"), "{}", engine.seen[2].prompt);
        assert_eq!(project.files["src/shop.py"], "def total(items):\n    return sum(items)\n");
    });
}

#[test]
fn a_step_whose_pack_cannot_fit_is_blocked_as_needs_split_and_never_cut() {
    let root = dir();
    let plan = plan_of(PROMPT);
    let mut project = Fake::new(root.path());
    let mut engine = Scripted::with(&[]);
    let (report, _) = run(&plan, &mut engine, &mut project, &Config { budget_tokens: 40, ..Config::default() });

    assert!(matches!(&report.steps[0].1, StepResult::Blocked { reason } if reason.starts_with("NeedsSplit")), "{:?}", report.steps[0]);
    assert!(engine.seen.is_empty(), "nothing was sent");
}

#[test]
fn a_long_document_is_written_in_sections_continued_when_cut_and_joined_only_whole() {
    crate::v3::forcing(&["nr.out_chunk"], || {
        let root = dir();
        let plan = plan_of("Write a user guide for the bakery shop in docs/guide.md.\n");
        let mut project = Fake::new(root.path());
        let mut engine = Scripted::with(&["1. Start\n2. Orders", "Start text\n<<SECTION 1 END>>", "Orders text that is cut", "and now it ends\n<<SECTION 2 END>>"]);
        let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

        assert!(matches!(&report.steps[0].1, StepResult::Done { how: "local, in sections", .. }), "{:?}", report.steps[0]);

        let guide = &project.files["docs/guide.md"];

        assert!(guide.contains("Start text") && guide.contains("Orders text that is cut\nand now it ends") && !guide.contains("<<SECTION"), "{guide}");
        assert!(engine.seen.iter().any(|r| r.prompt.contains("stopped before its end")));

        /* a section that never ends is not joined and the step is blocked */
        let mut project = Fake::new(root.path());
        let mut broken = Scripted::with(&["1. Only", "cut", "still cut", "still cut", "still cut"]);
        let (report, _) = run(&plan, &mut broken, &mut project, &Config::default());

        assert!(matches!(&report.steps[0].1, StepResult::Blocked { reason } if reason.contains("never reached its end marker") || reason.contains("never reached")), "{:?}", report.steps[0]);
        assert!(!project.files.contains_key("docs/guide.md"));
    });
}

#[test]
fn test_first_writes_tests_from_the_done_when_runs_them_on_the_stub_and_lands_the_code_after() {
    crate::v3::forcing(&["nr.test_first"], || {
        let root = dir();
        let plan = plan_of("Add a function total(items) in src/shop.py that returns the sum of the prices, keeps every price in whole cents, and is used by count(items) in src/cart.py when the cart is priced.\n");
        let mut project = Fake::new(root.path()).must("src/shop.py", "sum(");

        project.test_runner = true;

        let tests = "FILE: tests/test_shop.py\nTESTS: test_total\n```python\nfrom src.shop import total\n\ndef test_total():\n    assert total([1, 2]) == 3\n```";
        let mut engine = Scripted::with(&[tests, &edit("src/shop.py", "total", TOTAL_OK)]);
        let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

        assert!(matches!(&report.steps[0].1, StepResult::Done { how: "local, tests first", .. }), "{:?} {:?}", report.steps[0], report.notes);
        assert_eq!(engine.seen[0].purpose, Purpose::Tests);
        assert!(!engine.seen[0].prompt.contains("return 0"), "the test writer never sees the code");
        assert!(engine.seen[1].purpose == Purpose::Code && !engine.seen[1].prompt.contains("assert total"), "and the code writer never sees the tests");
        assert!(project.files.contains_key("tests/test_shop.py") && project.files["src/shop.py"].contains("sum(items)"));
        assert_eq!(project.files["tests/test_old.py"], "def test_old():\n    assert True\n", "the anchor is untouched");
    });
}

#[test]
fn test_first_steps_aside_when_the_project_cannot_run_single_tests() {
    crate::v3::forcing(&["nr.test_first"], || {
        let root = dir();
        let plan = plan_of("Add a function total(items) in src/shop.py that returns the sum of the prices, keeps every price in whole cents, and is used by count(items) in src/cart.py when the cart is priced.\n");
        let mut project = Fake::new(root.path()).must("src/shop.py", "sum(");
        let mut engine = Scripted::with(&[&edit("src/shop.py", "total", TOTAL_OK)]);
        let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

        assert!(matches!(&report.steps[0].1, StepResult::Done { how: "local", .. }), "{:?}", report.steps[0]);
        assert_eq!(engine.seen[0].purpose, Purpose::Code);
    });
}

#[test]
fn test_names_are_read_from_python_rust_and_js() {
    assert_eq!(test_names("def test_a():\n    pass\nasync def test_b():\n    pass\n"), vec!["test_a", "test_b"]);
    assert_eq!(test_names("#[test]\nfn adds() {}\n\n#[tokio::test]\nasync fn waits() {}\nfn helper() {}\n"), vec!["adds", "waits"]);
    assert_eq!(test_names("test('adds two', () => {});\nit(\"waits\", () => {});\n"), vec!["adds two", "waits"]);
}

#[test]
fn a_step_that_was_done_before_a_restart_is_not_run_again() {
    crate::v3::forcing(&["nr.git_shadow"], || {
        let root = dir();
        let git = |args: &[&str]| {
            std::process::Command::new("git").current_dir(root.path()).args(args).output().unwrap();
        };

        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.path().join("a.txt"), "x").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "first"]);

        let plan = plan_of("Add a function total(items) in src/shop.py that returns the sum of the prices.\n");
        let mut first_project = Fake::new(root.path()).must("src/shop.py", "sum(");
        let mut first = Scripted::with(&[&edit("src/shop.py", "total", TOTAL_OK)]);

        assert!(matches!(run(&plan, &mut first, &mut first_project, &Config::default()).0.steps[0].1, StepResult::Done { .. }));

        let mut second_project = Fake::new(root.path()).must("src/shop.py", "sum(");
        let mut second = Scripted::with(&[]);
        let (report, _) = run(&plan, &mut second, &mut second_project, &Config::default());

        assert_eq!(report.steps[0].1, StepResult::Resumed);
        assert!(second.seen.is_empty(), "no model call for a step that is done");
    });
}

#[test]
fn the_next_request_takes_its_sampling_once() {
    sampling::set(Some(0.7), None);
    assert_eq!(sampling::take(), Some((Some(0.7), None)));
    assert_eq!(sampling::take(), None);
    sampling::set(None, None);
    assert_eq!(sampling::take(), None);
}

#[test]
fn the_step_budget_follows_the_window_and_the_quantization() {
    assert_eq!(budget_for(16_384, Some("Q8_0")), 16_384 * 8 / 10 - 4_096);
    assert_eq!(budget_for(16_384, Some("Q4_K_M")), 16_384 * 6 / 10 - 4_096);
    assert_eq!(budget_for(2_000, None), 1_500);
}

#[test]
fn a_dead_model_blocks_every_step_honestly_and_changes_nothing() {
    let root = dir();
    let plan = plan_of(PROMPT);
    let mut project = Fake::new(root.path());
    let mut engine = Scripted::default();
    let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

    assert_eq!(report.blocked().len(), 2);
    assert_eq!(project.files["src/shop.py"], "def total(items):\n    return 0\n");
    let _ = spec::analyse("x");
}

#[test]
fn a_file_an_earlier_step_built_is_changed_not_created_again_and_a_copy_is_noted() {
    let root = dir();
    let plan = plan_of(PROMPT);
    let mut project = Fake::new(root.path()).must("src/shop.py", "sum(").must("src/cart.py", "len(");
    let again = "FILE: src/shop.py\n```python\ndef total(items):\n    return len(items)\n```";
    let mut engine = Scripted::with(&[&edit("src/shop.py", "total", TOTAL_OK), again, &edit("src/cart.py", "count", COUNT_OK)]);
    let (report, _) = run(&plan, &mut engine, &mut project, &Config::default());

    assert!(matches!(report.steps[1].1, StepResult::Done { attempts: 2, .. }), "{:?}", report.steps[1]);
    assert!(engine.seen[2].prompt.contains("already exists on disk") || engine.seen[2].prompt.contains("already in the registry"), "{}", engine.seen[2].prompt);
    assert!(engine.seen[2].prompt.contains("Change it with FILE and SYMBOL"), "{}", engine.seen[2].prompt);
}
