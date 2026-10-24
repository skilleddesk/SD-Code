//! **N9-2 - the NAVARATNA bench** (`sdcd bench navaratna`).
//!
//! Every `nr.*` switch is judged by a number, not a claim. For each switch there is a scenario with a planted trap - a requirement in the middle of
//! a prompt, a spec that invites a duplicate, a document longer than the model will write in one go, a test the code is tempted to break, a package
//! that does not exist - and the scenario is run twice, with the switch off and on, on a scripted fake model (no live model, no network). The
//! report is one row per switch: the metric, the value off, the value on, and whether the switch helped.
//!
//! These numbers measure the **code** around the model on scripted behaviour. They say nothing about how the real 9B behaves; that is N9-3
//! (`sdcd bench navaratna --live`, with Ollama running), which the owner runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{chunk, contracts, errslice, exist, golden, registry, shadow, spec, testfirst, window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Better {
    Lower,
    Higher,
    /// The switch changes behaviour but a smaller or larger number is not "better" (advice shown, calls routed).
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub switch: &'static str,
    pub metric: &'static str,
    pub better: Better,
    pub off: f64,
    pub on: f64,
    pub wall_ms: u128,
}

impl Row {
    pub fn verdict(&self) -> &'static str {
        let (off, on) = (self.off, self.on);

        match self.better {
            Better::Info => "info",
            Better::Lower if on < off => "helps",
            Better::Higher if on > off => "helps",
            _ if (on - off).abs() < f64::EPSILON => "no change",
            _ => "worse",
        }
    }
}

struct Scenario {
    switch: &'static str,
    metric: &'static str,
    better: Better,
    run: fn(bool) -> f64,
}

fn scratch(name: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    let dir = std::env::temp_dir().join(format!("sdc-navaratna-bench-{}-{}-{name}", std::process::id(), COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn git(root: &Path, args: &[&str]) {
    let _ = std::process::Command::new("git").current_dir(root).args(args).output();
}

/* ---- the scenarios -------------------------------------------------------------------------------------------------------------------- */

/// A twelve-unit prompt whose seventh unit is a requirement a lazy labeller skips.
const MID_PROMPT: &str = "# Shop\n\nBuild a small shop API. It must list items.\n\nThe cart must hold at most 50 items.\n\nIt must compute a total with 3.5 percent tax.\n\nIt must never allow a negative quantity.\n\nAdd an endpoint /items.\n\nShow a message when the cart is empty.\n\nExport the report as CSV.\n\nWrite the tests for the total.\n\nThe project is for a small bakery.\n";

fn missing_requirements(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.sentence_cover", on)], || {
        let units = spec::parse(MID_PROMPT);
        let middle = units.len() / 2;
        let truth: Vec<String> = units.iter().filter(|u| matches!(spec::heuristic(u, None), Some(spec::Label::Requirement))).map(|u| u.id.clone()).collect();
        /* a lazy labeller: it forgets the unit in the middle of the prompt */
        let mut map = spec::label(units.clone(), &mut |unit, last| if unit.id == units[middle].id { None } else { spec::heuristic(unit, last) });

        if let Err(blocked) = spec::coverage_gate(&map) {
            /* planning is blocked: the unmapped units are labelled again, by the deterministic labeller */
            let again: Vec<String> = blocked.unmapped;

            map = spec::label(units, &mut |unit, last| if again.contains(&unit.id) || map.labels.contains_key(&unit.id) { spec::heuristic(unit, last) } else { None });
        }

        truth.iter().filter(|id| !map.labels.contains_key(*id)).count() as f64
    })
}

fn unresolved_duplicates(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.registry", on)], || {
        let root = scratch("dup");
        let original = "pub fn compute_tax(amount: f64, rate: f64) -> f64 {\n    let base = amount * rate;\n    let rounded = (base * 100.0).round() / 100.0;\n    rounded\n}\n";

        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/tax.rs"), original).unwrap();

        let mut reg = registry::Registry::default();

        reg.record(registry::Entry { artifact: "src/tax.rs::compute_tax".into(), r_id: "R-001".into(), step: "s1".into(), model: "local".into(), hash: registry::content_hash(original), evidence: "src/tax.rs:1".into() });

        /* the spec invites a second create step for the same function; the writer would copy the body into a new file */
        let decision = registry::before_create(&mut reg, &root, "src/tax.rs::compute_tax");

        if matches!(decision, registry::CreateDecision::Create) {
            std::fs::write(root.join("src/tax2.rs"), original.replace("compute_tax", "tax_total")).unwrap();
        }

        let files: Vec<(String, String)> = ["src/tax.rs", "src/tax2.rs"].iter().filter_map(|p| std::fs::read_to_string(root.join(p)).ok().map(|t| (p.to_string(), t))).collect();
        let count = if files.len() == 2 { registry::find_duplicates(&files, "src/tax2.rs").len() } else { 0 };

        let _ = std::fs::remove_dir_all(&root);
        count as f64
    })
}

fn silent_truncations_output(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.out_chunk", on)], || {
        let root = scratch("chunk");
        /* the model stops halfway through section 2 the first time it writes it */
        let mut stopped_once = false;
        let mut writer = |prompt: &str| -> String {
            if prompt.contains("stopped before its end") {
                return format!("and the rest.\n{}", chunk::marker(2));
            }

            if prompt.contains("section 2 of 2") && !stopped_once {
                stopped_once = true;

                return "the first half of the second section, then the model just stops".to_string();
            }

            let n = if prompt.contains("section 1 of 2") { 1 } else { 2 };

            format!("section {n} text\n{}", chunk::marker(n))
        };
        let truncated = if on {
            let manifest = chunk::write_document(&root, "doc", "a document", &["one", "two"], &mut writer).unwrap();

            usize::from(chunk::join(&root, "doc").is_err() && !manifest.complete())
        } else {
            /* without chunking the whole document is one request and whatever comes back is the document */
            let text = ["section 1 text", "the first half of the second section, then the model just stops"].join("\n");

            usize::from(!text.contains(&chunk::marker(2)) && !text.ends_with("section 2 text"))
        };

        let _ = std::fs::remove_dir_all(&root);
        truncated as f64
    })
}

fn broken_writes_landed(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.edit_gate", on)], || {
        let root = scratch("gate");
        let broken = [("a.rs", "fn main() {\n    let x = 1;\n"), ("b.py", "def f():\nreturn 1\n"), ("c.ts", "export function f( {\n  return 1;\n}\n"), ("d.json", "{ \"a\": 1,, }"), ("e.rs", "fn broken( {\n")];
        let mut landed = 0;

        for (n, (path, text)) in broken.iter().enumerate() {
            /* the model, refused once, sends exactly the same text again */
            for attempt in 0..2 {
                let _ = crate::runtime::step::apply(&root, &format!("b{n}-{attempt}"), &[(path.to_string(), Some(text.to_string()))]);
            }

            if root.join(path).exists() {
                landed += 1;
            }
        }

        let _ = std::fs::remove_dir_all(&root);
        landed as f64
    })
}

fn files_damaged_by_lazy_edits(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.symbol_patch", on)], || {
        let source = "pub fn one() -> u64 {\n    1\n}\n\npub fn two() -> u64 {\n    2\n}\n\npub fn three() -> u64 {\n    3\n}\n";
        /* six model answers for "change two()": each is the symbol alone (for the patcher) and the same answer as a whole-file rewrite; three are lazy */
        let whole = |two: &str| format!("pub fn one() -> u64 {{\n    1\n}}\n\n{two}\n\npub fn three() -> u64 {{\n    3\n}}\n");
        let lazy_whole = "pub fn one() -> u64 {\n    1\n}\n\n// ... existing code ...\n".to_string();
        let answers = [
            ("pub fn two() -> u64 {\n    // ... rest unchanged\n}", lazy_whole.clone()),
            ("pub fn two() -> u64 {\n    22\n}", whole("pub fn two() -> u64 {\n    22\n}")),
            ("pub fn two() -> u64 {\n    // existing code\n    2 + 1\n}", lazy_whole.clone()),
            ("pub fn two() -> u64 {\n    23\n}", whole("pub fn two() -> u64 {\n    23\n}")),
            ("pub fn two() -> u64 {\n    # rest of the function stays the same\n}", lazy_whole),
            ("pub fn two() -> u64 {\n    24\n}", whole("pub fn two() -> u64 {\n    24\n}")),
        ];
        let mut damaged = 0;

        for (symbol_answer, whole_answer) in answers {
            /* without the patcher the model's whole-file rewrite is written over the file as it came */
            let after = if on { super::patcher::replace_symbol("a.rs", source, "two", symbol_answer).ok() } else { Some(whole_answer) };

            if let Some(after) = after {
                let kept_others = after.contains("pub fn one() -> u64 {\n    1\n}") && after.contains("pub fn three() -> u64 {\n    3\n}");

                if !kept_others || super::patcher::elision(&after).is_some() {
                    damaged += 1;
                }
            }
        }

        damaged as f64
    })
}

struct Scripted {
    code: Vec<(String, String)>,
}

impl testfirst::Parties for Scripted {
    fn write_tests(&mut self, _done_when: &str) -> Vec<testfirst::TestFile> {
        vec![testfirst::TestFile { path: "tests/test_new.py".into(), content: "# new".into(), names: vec!["tests/test_new.py::test_double".into()] }]
    }

    fn write_code(&mut self, _spec: &str) -> Vec<(String, String)> {
        self.code.clone()
    }

    fn run(&mut self, tree: &testfirst::Tree, ids: &[String]) -> BTreeMap<String, bool> {
        let calc = tree.get("src/calc.py").cloned().unwrap_or_default();

        ids.iter()
            .map(|id| {
                let passed = match id.as_str() {
                    "tests/test_anchor.py::test_add" => tree.get("tests/test_anchor.py").is_some_and(|t| t.contains("add(1, 2) == 3")),
                    "tests/test_new.py::test_double" => calc.contains("return x * 2"),
                    _ => false,
                };

                (id.clone(), passed)
            })
            .collect()
    }
}

fn anchor_violations(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.test_first", on)], || {
        let base = testfirst::Tree::from([
            ("src/calc.py".to_string(), "def add(a, b):\n    return a + b\n\ndef double(x):\n    raise NotImplementedError\n".to_string()),
            ("tests/test_anchor.py".to_string(), "def test_add(): assert add(1, 2) == 3\n".to_string()),
        ]);
        let anchors = testfirst::Anchors { ids: ["tests/test_anchor.py::test_add".to_string()].into(), files: ["tests/test_anchor.py".to_string()].into() };
        /* the temptation: the code call also "fixes" the anchor test so that everything goes green */
        let tempted = vec![("src/calc.py".to_string(), "def add(a, b):\n    return a + b + 1\n\ndef double(x):\n    return x * 2\n".to_string()), ("tests/test_anchor.py".to_string(), "def test_add(): assert add(1, 2) == 4\n".to_string())];
        let step = testfirst::Step { id: "s".into(), tag: testfirst::Tag::Code, done_when: "double(3) is 6".into(), spec: "implement double".into() };
        let mut parties = Scripted { code: tempted.clone() };

        match testfirst::run_step(&step, &base, &anchors, &mut parties) {
            testfirst::Outcome::AnchorViolation { .. } => 0.0,
            testfirst::Outcome::Skipped(_) => {
                /* test-first off: the writes simply land, and the anchor file is checked afterwards */
                let mut tree = base.clone();

                for (path, text) in tempted {
                    tree.insert(path, text);
                }

                f64::from(tree.get("tests/test_anchor.py") != base.get("tests/test_anchor.py"))
            }
            _ => 0.0,
        }
    })
}

fn invented_names_unflagged(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.exist_gate", on)], || {
        let root = scratch("exist");

        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/shape.rs"), "pub struct Shape {\n    pub side: f64,\n}\n\nimpl Shape {\n    pub fn new(side: f64) -> Self { Shape { side } }\n    pub fn area(&self) -> f64 { self.side * self.side }\n}\n").unwrap();

        let invented = ["Shape::volume(&s)", "Shape::perimeter(&s)"];
        let flagged = exist::note(&root, "src/main.rs", &format!("fn main() {{ let s = Shape::new(1.0); {}; {}; Shape::area(&s); }}", invented[0], invented[1])).map_or(0, |note| invented.iter().filter(|call| note.contains(&format!("`{}`", call.split("::").nth(1).unwrap().split('(').next().unwrap()))).count());

        let _ = std::fs::remove_dir_all(&root);
        (invented.len() - flagged) as f64
    })
}

fn prompts_sent_beyond_the_window(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.ctx_lock", on)], || {
        let usable = 4_096;
        let sizes = [500usize, 3_000, 9_000, 20_000];
        let mut sent_too_big = 0;

        for words in sizes {
            let body = serde_json::json!({ "messages": [{ "role": "user", "content": "word ".repeat(words) }], "tools": [] });
            let blocked = crate::v3::on("nr.ctx_lock") && window::preflight(&body, usable, 512).is_err();

            if !blocked && crate::guard::context::estimate_request(&body) + 512 > usable {
                sent_too_big += 1;
            }
        }

        f64::from(sent_too_big)
    })
}

fn golden_hit_rate(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.golden_trace", on)], || {
        let root = scratch("golden");
        let found = errslice::Found { tool: errslice::Tool::Rustc, message: "error[E0308]: mismatched types".into(), file: Some("src/a.rs".into()), line: Some(3), expected: None, actual: None, total: 1 };
        let signature = golden::signature(&found, "fn", "src/a.rs");
        let (mut hits, mut asked) = (0, 0);

        for turn in 0..4 {
            asked += 1;

            if golden::lookup(&root, &signature).is_some() {
                hits += 1;
            } else {
                /* the cloud fixes it; the fix passed every test */
                golden::record(&root, &golden::Fix { signature: signature.clone(), sliced_error: "mismatched types".into(), final_diff: "-let n: i32 = \"a\";\n+let n: i32 = 1;".into(), turn: format!("t{turn}"), tests_passed: true, touched: vec!["src/a.rs".into()] }, &|_| false, false);
            }
        }

        let _ = std::fs::remove_dir_all(&root);
        f64::from(hits) / f64::from(asked)
    })
}

struct Candidates {
    good: bool,
}

impl super::bestofn::Parties for Candidates {
    fn candidate(&mut self, _base: &super::bestofn::Tree, temperature: f32, _index: usize) -> Vec<(String, String)> {
        vec![("a.py".into(), if self.good && temperature > 0.5 { "ok".into() } else { "bad".into() })]
    }

    fn check(&mut self, tree: &super::bestofn::Tree) -> super::bestofn::Check {
        super::bestofn::Check { passed: tree.get("a.py").is_some_and(|t| t == "ok"), failures: 1, error: "assert".into() }
    }
}

fn local_step_share(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.best_of_n", on)], || {
        /* ten steps; seven pass at once; three fail the normal attempt and two sliced retries - a candidate sampler can solve two of those three */
        let mut local = 7;

        for solvable in [true, true, false] {
            let trigger = super::bestofn::Trigger { normal_failed: true, sliced_retries_failed: 2, hard: false, has_check: true };

            if super::bestofn::should_run(&trigger).is_ok() && matches!(super::bestofn::run(&Default::default(), 3, &mut Candidates { good: solvable }), super::bestofn::Outcome::Chosen { .. }) {
                local += 1;
            }
        }

        f64::from(local) / 10.0
    })
}

fn contract_breaks(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.contract_lock", on)], || {
        let shop = "pub fn total(items: &[u64]) -> u64 {\n    items.iter().sum()\n}\n";
        let mut lock = contracts::Lock::default();

        lock.publish("src/shop.rs", shop, "s1");

        let changed = shop.replace("pub fn total(items: &[u64]) -> u64", "pub fn total(items: &[u64], tax: u64) -> u64");

        f64::from(contracts::check_patch(&lock, "src/shop.rs", &changed).is_ok() && changed != shop)
    })
}

fn residue_after_a_failed_step(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.git_shadow", on)], || {
        let root = scratch("shadow");

        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.email", "t@t.t"]);
        git(&root, &["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "first"]);

        let _ = shadow::run_step(&root, "s1", &mut || {
            std::fs::write(root.join("a.txt"), "broken\n").unwrap();
            std::fs::write(root.join("junk.txt"), "x\n").unwrap();

            Err("checks failed".into())
        });

        let residue = usize::from(std::fs::read_to_string(root.join("a.txt")).unwrap_or_default() != "one\n") + usize::from(root.join("junk.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
        residue as f64
    })
}

fn malformed_calls_reaching_tools(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.tool_envelope", on)], || {
        let specs = vec![crate::agent::dialect::ToolSpec { name: "read_file", description: "read", schema: serde_json::json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"], "additionalProperties": false }) }];
        let mut guard = super::envelope::Guard::default();
        let calls = [serde_json::json!({}), serde_json::json!({ "path": 5 }), serde_json::json!({ "path": "a", "x": 1 }), serde_json::json!({ "path": "ok.rs" }), serde_json::json!({ "path": "ok.rs" })];
        /* malformed: the first three; the fifth is a repeat of the fourth */
        let mut bad_through = 0;

        for (n, arguments) in calls.iter().enumerate() {
            let verdict = guard.judge(&specs, "read_file", arguments);

            if verdict == super::envelope::Verdict::Run && (n < 3 || n == 4) {
                bad_through += 1;
            }
        }

        f64::from(bad_through)
    })
}

fn lost_identifiers_unflagged(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.lang_spec", on)], || {
        let original = "shop.py ফাইলে total() ফাংশনে ৩.৫ শতাংশ কর যোগ করো।";
        let units = spec::parse(original);
        /* a lossy restatement: the file name and the rate are gone */
        let lossy = "Add a tax to the total function.";
        let lost = super::langspec::missing(original, lossy).len();

        if crate::v3::on("nr.lang_spec") {
            let report = super::langspec::restate(&units, &mut |_| lossy.to_string());
            let flagged: usize = report.mismatches().iter().map(|line| line.missing.len()).sum();

            lost.saturating_sub(flagged) as f64
        } else {
            /* nothing checks the restatement: whatever it lost stays lost, unsaid */
            lost as f64
        }
    })
}

fn prompt_tokens_at_step_fifty(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.fresh_step", on)], || {
        let units = spec::parse("Add a function double(x) that returns twice x.");
        let notes: Vec<String> = (0..50).map(|n| format!("step {n} changed src/a.rs and passed")).collect();

        if on {
            let input = super::pack::StepInput { rules: "rules", goal: "g", done_when: "d", units: units.iter().collect(), handover: notes, budget_tokens: 100_000, ..Default::default() };

            super::pack::build(&input, &|_| 1.0).map_or(0.0, |pack| pack.tokens as f64)
        } else {
            /* the conversation so far: every step's request and answer are still in the context */
            (0..50).map(|n| crate::tokens::estimate(&format!("step {n}: the model's request and its answer and the tool result of about this length\n{}", notes[n]))).sum::<u64>() as f64 + 20.0
        }
    })
}

fn pack_tokens(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.skeleton_pack", on)], || {
        let units = spec::parse("Change total().");
        let text: String = (0..80).map(|n| format!("pub fn helper_{n}(v: u64) -> u64 {{\n    let a = v + {n};\n    let b = a * 2;\n    b - {n}\n}}\n\n")).collect::<String>() + "pub fn total(v: &[u64]) -> u64 {\n    v.iter().sum()\n}\n";
        let input = super::pack::StepInput { rules: "r", goal: "g", done_when: "d", units: units.iter().collect(), files: vec![super::pack::FileCtx { path: "src/a.rs".into(), text, targets: vec!["total".into()] }], budget_tokens: 100_000, ..Default::default() };

        super::pack::build(&input, &|_| 1.0).map_or(0.0, |pack| pack.tokens as f64)
    })
}

fn error_tokens(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.error_slice", on)], || {
        let mut log = String::from("   Compiling demo v0.1.0\n");

        for n in 0..60 {
            log.push_str(&format!("warning: unused variable: `v{n}`\n --> src/lib.rs:{}:9\n  |\n{} |     let v{n} = 2;\n  |         ^^ help: prefix it with an underscore\n\n", n + 1, n + 1));
        }

        log.push_str("error[E0308]: mismatched types\n --> src/lib.rs:70:18\n  |\n70 |     let n: i32 = \"a\";\n  |                  ^^^ expected `i32`, found `&str`\n\nerror: could not compile `demo` due to 1 previous error\n");

        if on {
            let dir = scratch("errs");
            let tokens = errslice::slice(&dir, &log, None, 400).map_or(0, |slice| crate::tokens::estimate(&slice.text));

            let _ = std::fs::remove_dir_all(&dir);
            tokens as f64
        } else {
            crate::tokens::estimate(&log) as f64
        }
    })
}

fn thinking_tokens_on_boilerplate(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.think_budget", on)], || {
        use super::think::{decide, reserve, Kind};

        /* a capable model that thinks by default: eight boilerplate and two fix steps */
        let kinds = [Kind::Boilerplate; 8].into_iter().chain([Kind::ErrorFix; 2]);
        let mut thinking_reserved = 0u64;

        for kind in kinds {
            let decision = if crate::v3::on("nr.think_budget") { decide(kind, true) } else { None };
            let thinks = decision.unwrap_or(true);

            if thinks && matches!(kind, Kind::Boilerplate) {
                thinking_reserved += reserve(2_000, Some(true)) - 2_000;
            }
        }

        thinking_reserved as f64
    })
}

fn packet_tokens(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.escalation_packet", on)], || {
        let dir = scratch("packet");
        let packet = super::packet::Packet {
            requirements: vec![("R-001".into(), "total() sums".into())],
            skeletons: vec![("src/a.rs".into(), "L1: pub fn total()".into())],
            last_diff: "-a\n+b".into(),
            sliced_error: Some("rustc error at src/a.rs:3".into()),
            ..Default::default()
        };
        let whole = "pub fn filler() {}\n".repeat(300);
        let files = vec![("src/a.rs".to_string(), whole)];
        let step = crate::rotation::capsule::Step { session_id: "bench", goal: "fix total", done_when: &[], error: &"warning: noise\n".repeat(200), notes: &[], files: if on { &[] } else { &files }, packet: Some(&packet) };
        let prepared = crate::rotation::capsule::Prepared { stable: "RULES".into(), ..Default::default() };
        let capsule = crate::rotation::capsule::finish(&prepared, &dir, &crate::trust::policy::Policy::default(), &step, crate::rotation::capsule::DEFAULT_BUDGET_TOKENS);

        let _ = std::fs::remove_dir_all(&dir);
        capsule.tokens as f64
    })
}

fn stalled_turns_on_a_429(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.role_rotation", on)], || {
        use super::roles::{candidates, Cause, Cooling, Role};

        let chain: Vec<crate::rotation::executor::ChainModel> = ["one", "two"].iter().map(|m| crate::rotation::executor::ChainModel { provider: "p".into(), model: m.to_string(), max_cost_per_step: 0.0, role: String::new() }).collect();
        let mut cooling = Cooling::default();
        let now = std::time::Instant::now();

        cooling.cool("p:one", Cause::RateLimited, now);

        /* with the switch off the chain is walked in order and row one (cooling) is asked first; the turn stalls if the caller takes only the first row */
        let first_row = candidates(&chain, Role::Fixer, &cooling, now).first().map(|row| row.model.clone());

        f64::from(first_row.as_deref() == Some("one"))
    })
}

fn architecture_steps_routed(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.tag_route", on)], || {
        use super::route::{route, Mode, Route, Tag};

        let mode = Mode { manual_pin: false, local_only: false, relay_auto: true, architect_row: true };

        [Tag::Simple, Tag::Code, Tag::Hard, Tag::Architecture, Tag::Code, Tag::Architecture].iter().filter(|tag| route(**tag, &mode) == Route::Architect).count() as f64
    })
}

fn cloud_calls_without_consent(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.blueprint", on)], || {
        let prompt = format!("{MID_PROMPT}{}", "Also add a thing. It must work.\n\n".repeat(400));
        let map = spec::analyse(&prompt);
        let context = super::blueprint::Context { relay_consent: false, local_only: false, manual_pin: false, architect_row: true };
        let mut calls = 0;

        super::blueprint::run(&context, &prompt, &map, &|t| t.to_string(), &mut |_| {
            calls += 1;

            Ok(String::new())
        });

        f64::from(calls)
    })
}

fn second_local_calls_started(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.mem_budget", on)], || {
        /* an 8 GB machine: the budgeter says one slot */
        let limit = if crate::v3::on("nr.mem_budget") { super::membudget::budget(8 << 30, 5 << 30, 1 << 30, 147_456, 16_384).slots } else { usize::MAX / 2 };
        let slots = super::membudget::Slots::new(limit);
        let first = slots.try_take();
        let second = slots.try_take();

        drop(first);
        f64::from(second.is_some())
    })
}

fn advice_lines_for_a_q4_model(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.quant_advice", on)], || f64::from(super::advice::code_for("qwen3:9b-q4_K_M", Some("Q4_K_M")).is_some()))
}

fn symbols_found(on: bool) -> f64 {
    crate::v3::with_switches(&[("nr.symbol_index", on)], || {
        super::symbols::index_text("a.rs", "pub fn a() {}\npub fn b() {}\n").map_or(0, |s| s.len()) as f64
    })
}

const SCENARIOS: &[Scenario] = &[
    Scenario { switch: "nr.ctx_lock", metric: "oversize prompts sent to the model", better: Better::Lower, run: prompts_sent_beyond_the_window },
    Scenario { switch: "nr.quant_advice", metric: "advice lines shown for a Q4 model", better: Better::Info, run: advice_lines_for_a_q4_model },
    Scenario { switch: "nr.mem_budget", metric: "second concurrent local call started on an 8 GB machine", better: Better::Lower, run: second_local_calls_started },
    Scenario { switch: "nr.sentence_cover", metric: "requirements missing from the plan", better: Better::Lower, run: missing_requirements },
    Scenario { switch: "nr.lang_spec", metric: "lost numbers/names left unflagged", better: Better::Lower, run: lost_identifiers_unflagged },
    Scenario { switch: "nr.blueprint", metric: "cloud calls with the consent off", better: Better::Lower, run: cloud_calls_without_consent },
    Scenario { switch: "nr.fresh_step", metric: "prompt tokens at step 50", better: Better::Lower, run: prompt_tokens_at_step_fifty },
    Scenario { switch: "nr.skeleton_pack", metric: "tokens of a pack with a big related file", better: Better::Lower, run: pack_tokens },
    Scenario { switch: "nr.error_slice", metric: "tokens of a failure shown to the model", better: Better::Lower, run: error_tokens },
    Scenario { switch: "nr.symbol_index", metric: "symbols found in a fixture", better: Better::Info, run: symbols_found },
    Scenario { switch: "nr.symbol_patch", metric: "files damaged by a lazy edit", better: Better::Lower, run: files_damaged_by_lazy_edits },
    Scenario { switch: "nr.edit_gate", metric: "broken writes that reached the disk", better: Better::Lower, run: broken_writes_landed },
    Scenario { switch: "nr.out_chunk", metric: "silent truncations of a long document", better: Better::Lower, run: silent_truncations_output },
    Scenario { switch: "nr.test_first", metric: "anchor violations", better: Better::Lower, run: anchor_violations },
    Scenario { switch: "nr.exist_gate", metric: "invented members not flagged", better: Better::Lower, run: invented_names_unflagged },
    Scenario { switch: "nr.registry", metric: "unresolved duplicates", better: Better::Lower, run: unresolved_duplicates },
    Scenario { switch: "nr.git_shadow", metric: "files left changed by a failed step", better: Better::Lower, run: residue_after_a_failed_step },
    Scenario { switch: "nr.contract_lock", metric: "locked signatures changed without a contract step", better: Better::Lower, run: contract_breaks },
    Scenario { switch: "nr.best_of_n", metric: "share of steps done locally", better: Better::Higher, run: local_step_share },
    Scenario { switch: "nr.tool_envelope", metric: "malformed or repeated calls reaching a tool", better: Better::Lower, run: malformed_calls_reaching_tools },
    Scenario { switch: "nr.think_budget", metric: "thinking tokens reserved on boilerplate steps", better: Better::Lower, run: thinking_tokens_on_boilerplate },
    Scenario { switch: "nr.tag_route", metric: "steps routed to the architect row", better: Better::Info, run: architecture_steps_routed },
    Scenario { switch: "nr.escalation_packet", metric: "tokens of the capsule sent to the chain", better: Better::Lower, run: packet_tokens },
    Scenario { switch: "nr.role_rotation", metric: "turns stalled on a rate-limited first row", better: Better::Lower, run: stalled_turns_on_a_429 },
    Scenario { switch: "nr.golden_trace", metric: "Golden Trace hit rate", better: Better::Higher, run: golden_hit_rate },
];

/// Runs every scenario with its switch off and on.
pub fn run_all() -> Vec<Row> {
    SCENARIOS
        .iter()
        .map(|scenario| {
            let started = Instant::now();
            let off = (scenario.run)(false);
            let on = (scenario.run)(true);

            Row { switch: scenario.switch, metric: scenario.metric, better: scenario.better, off, on, wall_ms: started.elapsed().as_millis() }
        })
        .collect()
}

fn number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 { format!("{}", value as i64) } else { format!("{value:.2}") }
}

/// The report: a table with one row per switch.
pub fn render(rows: &[Row]) -> String {
    let mut out = String::from("# NAVARATNA bench\n\nSCRIPTED run on a fake model. These numbers measure the code around the model; they say nothing about the live 9B (that is N9-3, `--live`).\n\n| switch | metric | off | on | verdict | ms |\n| --- | --- | --- | --- | --- | --- |\n");

    for row in rows {
        out.push_str(&format!("| {} | {} | {} | {} | {} | {} |\n", row.switch, row.metric, number(row.off), number(row.on), row.verdict(), row.wall_ms));
    }

    let helps = rows.iter().filter(|row| row.verdict() == "helps").count();
    let worse = rows.iter().filter(|row| row.verdict() == "worse").count();

    out.push_str(&format!("\n{} switches measured: {helps} helped, {worse} worse, {} unchanged or informational.\n", rows.len(), rows.len() - helps - worse));
    out
}

/// `sdcd bench navaratna [--out FILE] [--live]`.
pub fn command(args: &[String]) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--live") {
        return Err("OWNER-LATER (N9-3): the live bench needs Ollama running with the 9B and is run by the owner; this build only has the scripted bench (`sdcd bench navaratna`)".into());
    }

    let rows = run_all();
    let text = render(&rows);

    println!("{text}");

    if let Some(path) = args.iter().position(|arg| arg == "--out").and_then(|at| args.get(at + 1)) {
        std::fs::write(path, &text).map_err(|error| format!("{path}: {error}"))?;
        println!("written to {path}");
    }

    Ok(())
}

#[cfg(test)]
mod tests;
