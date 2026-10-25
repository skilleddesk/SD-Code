//! **PADMA P4-1 - the NAVARATNA engines inside Recipe steps** (switch `padma.recipe_engines`, off).
//!
//! The chat loop uses the NAVARATNA engines only for a long prompt; a Recipe is the short-prompt-for-a-big-job route, so its model steps use them
//! **always**, whatever the person typed. With the switch on, a recipe run forces these on for its own thread (`v3::with_switches`):
//!
//! | engine | what a recipe step gets |
//! | ------ | ----------------------- |
//! | N3-1 `nr.fresh_step` | a new short prompt: the goal and what done means first, the step's work, the goal and done-when again as the last line; no chat history |
//! | N3-2 `nr.skeleton_pack` | the step's `context` files as skeletons (signatures with line numbers), never whole bodies |
//! | N4-2 `nr.symbol_patch` | the fix recipe may replace one whole function by its name instead of copying exact old lines |
//! | N5-1 `nr.test_first` | a step tagged `test_first` may not touch a test: the red test is the stub check, the tests are the anchors |
//! | N6-1 `nr.registry` | what a step wrote is recorded (file, recipe, step, model, hash) and a create over an existing file is noted as an edit |
//! | N6-2 `nr.git_shadow` | a step that writes runs under a hidden snapshot; a failure puts the tree back exactly |
//! | M1-4 relay | a unit the local model could not do after its retries goes once to the chain, only when the person consented |
//!
//! Every use is written to a per-thread receipt, which the turn shows as one line - so "the engines were active" is something a run can be asked, not
//! something this file claims.

use std::cell::RefCell;
use std::path::Path;

use super::codefns::{Data, Env};
use crate::navaratna::{pack, registry, shadow, symbols};

pub const SWITCH: &str = "padma.recipe_engines";

/// The `nr.*` switches a recipe run forces on for its own thread.
const FORCED: &[&str] = &["nr.fresh_step", "nr.skeleton_pack", "nr.symbol_patch", "nr.test_first", "nr.registry", "nr.git_shadow"];

/// The most related files whose skeletons a step gets, and the most characters of them.
const MAX_SKELETONS: usize = 3;
const MAX_SKELETON_CHARS: usize = 3_500;

pub fn active() -> bool {
    crate::v3::on(SWITCH)
}

/// Runs `body` with the engines forced on when the recipe switch is on; otherwise runs it as it is.
pub fn with_engines<T>(body: impl FnOnce() -> T) -> T {
    if !active() {
        return body();
    }

    let settings: Vec<(&str, bool)> = FORCED.iter().map(|name| (*name, true)).collect();

    crate::v3::with_switches(&settings, body)
}

thread_local! {
    static RECEIPT: RefCell<Vec<Used>> = const { RefCell::new(Vec::new()) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Used {
    pub engine: &'static str,
    pub times: u32,
    pub details: Vec<String>,
}

/// Writes one use of an engine to this thread's receipt.
pub fn note(engine: &'static str, detail: impl Into<String>) {
    let detail = detail.into();

    RECEIPT.with(|receipt| {
        let mut receipt = receipt.borrow_mut();
        let row = match receipt.iter().position(|row| row.engine == engine) {
            Some(at) => at,
            None => {
                receipt.push(Used { engine, times: 0, details: Vec::new() });

                receipt.len() - 1
            }
        };

        receipt[row].times += 1;

        if !detail.is_empty() && receipt[row].details.len() < 3 && !receipt[row].details.contains(&detail) {
            receipt[row].details.push(detail);
        }
    });
}

/// The receipt so far, and empties it.
pub fn take_receipt() -> Vec<Used> {
    RECEIPT.with(|receipt| std::mem::take(&mut *receipt.borrow_mut()))
}

pub fn clear_receipt() {
    let _ = take_receipt();
}

/// The one line the turn shows: which engines ran and how often.
pub fn receipt_line(used: &[Used]) -> String {
    if used.is_empty() {
        return "Engines used: none".to_string();
    }

    let rows: Vec<String> = used
        .iter()
        .map(|row| {
            let detail: String = row.details.join("; ").chars().take(160).collect();

            if detail.is_empty() { format!("{} x{}", row.engine, row.times) } else { format!("{} x{} ({detail})", row.engine, row.times) }
        })
        .collect();

    format!("Engines used: {}", rows.join(", "))
}

/* ---- N3-1: the fresh step prompt ------------------------------------------------------------------------------------------------- */

/// The step's prompt as a fresh, short one: goal and done-when first, the work, then the goal and done-when once more as the last line.
pub fn fresh(goal: &str, done_when: &str, prompt: String) -> String {
    if !crate::v3::on("nr.fresh_step") {
        return prompt;
    }

    let goal = if goal.trim().is_empty() { "(the request as given)" } else { goal.trim() };
    let done = if done_when.trim().is_empty() { "this step's answer is in exactly the form asked" } else { done_when.trim() };

    note("fresh_step", "");

    format!("## Goal\n{goal}\nDone when: {done}\n\n{}\n\n{}", prompt.trim_end(), pack::closing_line(goal, done))
}

/* ---- N3-2: skeletons of the related files ---------------------------------------------------------------------------------------- */

/// The project files a step's `context` names: what a reviewed file imports (`imports`), or the files a failure points at (`suspects`).
pub fn related(env: &Env, context: &str, unit: Option<&str>, data: &Data) -> Vec<String> {
    let mut found: Vec<String> = match context {
        "imports" => {
            let path = unit.map(|unit| super::codefns::split_unit(unit).0.to_string()).unwrap_or_default();

            super::fixfns::imports_of(env, &path).into_iter().filter(|file| *file != path).collect()
        }
        "suspects" => data
            .get("context")
            .into_iter()
            .flatten()
            .flat_map(|text| text.lines())
            .filter_map(|line| line.strip_prefix("File: ").and_then(|rest| rest.split(" (lines").next()))
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };

    found.dedup();
    found.truncate(MAX_SKELETONS);

    found
}

/// The skeleton block for these files: signatures with line numbers; a file with no grammar shows its first lines, and says so.
pub fn skeleton_block(root: &Path, files: &[String]) -> Option<String> {
    if !crate::v3::on("nr.skeleton_pack") || files.is_empty() {
        return None;
    }

    let mut out = String::from("## Related files (skeletons: signatures with line numbers, not the code)\n");

    for path in files {
        let Ok(text) = std::fs::read_to_string(root.join(path)) else { continue };
        let body = match symbols::skeleton(path, &text) {
            Some(skeleton) => format!("--- {path} (skeleton) ---\n{}", skeleton.trim_end()),
            None => format!("--- {path} (no grammar: first 30 lines) ---\n{}", text.lines().take(30).collect::<Vec<_>>().join("\n")),
        };

        if out.len() + body.len() > MAX_SKELETON_CHARS {
            break;
        }

        out.push_str(&body);
        out.push('\n');
    }

    if out.lines().count() <= 1 {
        return None;
    }

    note("skeleton_pack", format!("{} file(s) as skeletons", files.len()));

    Some(out)
}

/// The skeleton block inserted into a prompt, ahead of nothing else: after the step's own text.
pub fn with_skeletons(env: &Env, context: &str, unit: Option<&str>, data: &Data, prompt: String) -> String {
    if context.is_empty() {
        return prompt;
    }

    match skeleton_block(&env.root, &related(env, context, unit, data)) {
        Some(block) => format!("{}\n\n{block}", prompt.trim_end()),
        None => prompt,
    }
}

/* ---- N4-2: the symbol patch hint -------------------------------------------------------------------------------------------------- */

/// What the fix prompt says about replacing a whole function; empty when the symbol patch is off.
pub fn symbol_hint() -> &'static str {
    if crate::v3::on("nr.symbol_patch") {
        "\nWhen one whole function has to change, you may instead write:\nFILE: path/relative/to/the/project\nSYMBOL: the_function_name\n```\nthe complete new function, nothing left out\n```\nThe function must keep its name. Use this only when the SEARCH/REPLACE form would be awkward."
    } else {
        ""
    }
}

/* ---- N5-1: the tests are the anchors --------------------------------------------------------------------------------------------- */

/// Whether this step is tagged `test_first` and the engine is on.
pub fn test_first(data: &Data) -> bool {
    crate::v3::on("nr.test_first") && data.get("_tags").is_some_and(|tags| tags.iter().any(|tag| tag == "test_first"))
}

/// A test-first step may not change a test (an anchor): a patch that "fixes" the test instead of the code is refused with the reason.
pub fn refuse_test_edits(paths: &[String]) -> Result<(), String> {
    let is_test = |path: &String| {
        let lowered = path.to_ascii_lowercase();
        let stem = lowered.rsplit('/').next().unwrap_or(&lowered).split('.').next().unwrap_or_default().to_string();

        crate::graph::index::is_test_path(path) || matches!(stem.as_str(), "test" | "tests" | "spec" | "conftest")
    };

    match paths.iter().find(|path| is_test(path)) {
        Some(path) => Err(format!("{path} is a test - it is an anchor and may not be changed; fix the code the test checks, not the test")),
        None => Ok(()),
    }
}

/* ---- N6-2: the Git Shadow ---------------------------------------------------------------------------------------------------------- */

fn text_of(data: &Data, key: &str) -> String {
    data.get(key).and_then(|lines| lines.first()).cloned().unwrap_or_default()
}

/// Runs a step's writes under the hidden snapshot (a failure puts the tree back exactly); a project that is not a git repository runs plainly, and says so.
pub fn shadowed<T>(env: &Env, data: &Data, body: &mut dyn FnMut() -> Result<T, String>) -> Result<T, String> {
    if !crate::v3::on("nr.git_shadow") {
        return body();
    }

    if !shadow::is_repository(&env.root) {
        note("git_shadow", "skipped: the project is not a git repository");

        return body();
    }

    let key = format!("{}.{}", text_of(data, "_task"), text_of(data, "_step"));
    let mut out = None;
    let ran = shadow::run_step(&env.root, &key, &mut || {
        body().map(|value| {
            out = Some(value);
        })
    });

    match ran {
        Ok(shadow::Ran::Done) => {
            note("git_shadow", "snapshot taken, kept");

            out.ok_or_else(|| "the step finished without a result".to_string())
        }
        Ok(shadow::Ran::Failed(error)) => {
            note("git_shadow", "a failed step was put back exactly");

            Err(error)
        }
        /* The journal already says this step is done: it is run again plainly rather than skipped without its output. */
        Ok(shadow::Ran::Skipped) => body(),
        /* A snapshot that could not be taken is not a reason to stop the work; the step's own undo still applies. */
        Err(reason) => {
            note("git_shadow", format!("skipped: {reason}"));

            body()
        }
    }
}

/* ---- N6-1: the registry ------------------------------------------------------------------------------------------------------------ */

/// Before files are created: any that already exist are noted as edits (the registry's "create becomes edit"), never refused.
pub fn before_create(env: &Env, artifacts: &[String]) {
    if !crate::v3::on("nr.registry") {
        return;
    }

    let mut reg = registry::Registry::load(&env.root);

    for artifact in artifacts {
        if let registry::CreateDecision::ConvertToEdit { why, .. } = registry::before_create(&mut reg, &env.root, artifact) {
            note("registry", format!("create became edit: {why}"));
        }
    }

    let _ = reg.save(&env.root);
}

/// After a step wrote files: each is recorded with the recipe, the step, the model and a content hash; a possible duplicate is a warning.
pub fn register(env: &Env, data: &Data, artifacts: &[String], evidence: &str) {
    if !crate::v3::on("nr.registry") || artifacts.is_empty() {
        return;
    }

    let mut reg = registry::Registry::load(&env.root);

    for artifact in artifacts {
        let text = std::fs::read_to_string(env.root.join(artifact)).unwrap_or_default();

        if let Some(warning) = registry::duplicate_note(&env.root, artifact, &text) {
            note("registry", warning.lines().nth(1).unwrap_or("possible duplicate").trim_start_matches("- ").to_string());
        }

        reg.record(registry::Entry { artifact: artifact.clone(), r_id: text_of(data, "_recipe"), step: text_of(data, "_step"), model: env.model.clone(), hash: registry::content_hash(&text), evidence: evidence.to_string() });
    }

    let _ = reg.save(&env.root);

    note("registry", format!("{} artifact(s) recorded", artifacts.len()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/util.js"), "export function double(n) {\n  return n * 2;\n}\n\nexport function half(n) {\n  return n / 2;\n}\n").unwrap();
        std::fs::write(dir.path().join("src/main.js"), "import { double } from './util.js';\n\nexport function run() {\n  return double(4);\n}\n").unwrap();

        dir
    }

    fn data(pairs: &[(&str, &str)]) -> Data {
        pairs.iter().map(|(key, value)| (key.to_string(), vec![value.to_string()])).collect()
    }

    #[test]
    fn a_fresh_prompt_has_the_goal_first_and_again_last_and_nothing_with_the_switch_off() {
        clear_receipt();
        crate::v3::with_switches(&[("nr.fresh_step", true)], || {
            let text = fresh("review the whole project", "every file has a verdict", "Review this file.".to_string());

            assert!(text.starts_with("## Goal\nreview the whole project\nDone when: every file has a verdict"), "{text}");
            assert!(text.contains("Review this file."));
            assert!(text.trim_end().ends_with("Remember - goal: review the whole project | done when: every file has a verdict"), "{text}");
        });
        crate::v3::with_switches(&[("nr.fresh_step", false)], || assert_eq!(fresh("g", "d", "plain".to_string()), "plain"));

        assert_eq!(take_receipt(), vec![Used { engine: "fresh_step", times: 1, details: Vec::new() }], "only the run that used it is written down");
    }

    #[test]
    fn a_reviewed_file_gets_the_skeletons_of_what_it_imports_and_never_their_bodies() {
        let dir = project();
        let env = Env::new(dir.path().to_path_buf());

        clear_receipt();
        crate::v3::with_switches(&[("nr.skeleton_pack", true)], || {
            let prompt = with_skeletons(&env, "imports", Some("src/main.js"), &Data::new(), "Review.".to_string());

            assert!(prompt.starts_with("Review."), "{prompt}");
            assert!(prompt.contains("src/util.js (skeleton)"), "{prompt}");
            assert!(prompt.contains("double") && prompt.contains("half"), "{prompt}");
            assert!(!prompt.contains("return n * 2"), "a skeleton has signatures, not bodies: {prompt}");
        });

        assert_eq!(take_receipt()[0].engine, "skeleton_pack");

        crate::v3::with_switches(&[("nr.skeleton_pack", false)], || assert_eq!(with_skeletons(&env, "imports", Some("src/main.js"), &Data::new(), "Review.".to_string()), "Review."));
    }

    #[test]
    fn suspect_files_are_read_from_the_localized_context() {
        let dir = project();
        let env = Env::new(dir.path().to_path_buf());
        let context = data(&[("context", "File: src/util.js (lines 1-7)\n1| export function double(n) {\n\nFile: src/main.js (lines 1-5)\n1| import")]);

        assert_eq!(related(&env, "suspects", None, &context), vec!["src/util.js".to_string(), "src/main.js".to_string()]);
        assert!(related(&env, "", None, &context).is_empty());
    }

    #[test]
    fn a_test_first_step_may_not_touch_a_test_and_the_tag_is_what_turns_it_on() {
        assert!(refuse_test_edits(&["src/util.js".to_string()]).is_ok());
        assert!(refuse_test_edits(&["test.js".to_string()]).is_err() && refuse_test_edits(&["pkg/conftest.py".to_string()]).is_err());

        let refused = refuse_test_edits(&["src/util.js".to_string(), "tests/util.test.js".to_string()]).unwrap_err();

        assert!(refused.contains("tests/util.test.js") && refused.contains("anchor"), "{refused}");

        crate::v3::with_switches(&[("nr.test_first", true)], || {
            assert!(test_first(&data(&[("_tags", "test_first")])));
            assert!(!test_first(&data(&[("_tags", "writes")])));
            assert!(!test_first(&Data::new()));
        });
        crate::v3::with_switches(&[("nr.test_first", false)], || assert!(!test_first(&data(&[("_tags", "test_first")]))));
    }

    #[test]
    fn the_registry_records_what_was_written_and_notes_a_create_over_an_existing_file_as_an_edit() {
        let dir = project();
        let mut env = Env::new(dir.path().to_path_buf());

        env.model = "qwen3.5:9b".to_string();

        let facts = data(&[("_recipe", "R-FIX"), ("_step", "patch")]);

        clear_receipt();
        crate::v3::with_switches(&[("nr.registry", true)], || {
            before_create(&env, &["src/util.js".to_string(), "src/new.js".to_string()]);
            register(&env, &facts, &["src/util.js".to_string()], "tests pass");
        });

        let saved = registry::Registry::load(dir.path());

        assert_eq!(saved.entries.len(), 1);
        assert_eq!((saved.entries[0].artifact.as_str(), saved.entries[0].r_id.as_str(), saved.entries[0].step.as_str(), saved.entries[0].model.as_str()), ("src/util.js", "R-FIX", "patch", "qwen3.5:9b"));
        assert_eq!(saved.entries[0].hash, registry::content_hash(&std::fs::read_to_string(dir.path().join("src/util.js")).unwrap()));
        assert!(saved.conversions.iter().any(|line| line.contains("src/util.js")) && !saved.conversions.iter().any(|line| line.contains("src/new.js")));
        assert!(take_receipt().iter().any(|row| row.engine == "registry" && row.details.iter().any(|detail| detail.contains("create became edit"))));
    }

    #[test]
    fn the_receipt_counts_each_engine_once_per_use_and_reads_as_one_line() {
        clear_receipt();
        note("fresh_step", "");
        note("fresh_step", "");
        note("registry", "1 artifact(s) recorded");

        let used = take_receipt();

        assert_eq!(receipt_line(&used), "Engines used: fresh_step x2, registry x1 (1 artifact(s) recorded)");
        assert_eq!(receipt_line(&[]), "Engines used: none");
        assert!(take_receipt().is_empty(), "taking the receipt empties it");
    }

    #[test]
    fn the_engines_are_forced_on_only_when_the_recipe_switch_is_on() {
        crate::v3::with_switches(&[(SWITCH, false)], || with_engines(|| assert!(!crate::v3::on("nr.fresh_step"))));
        crate::v3::with_switches(&[(SWITCH, true)], || {
            with_engines(|| {
                for name in FORCED {
                    assert!(crate::v3::on(name), "{name}");
                }
            });
            assert!(!crate::v3::on("nr.fresh_step"), "and put back afterwards");
        });
    }
}
