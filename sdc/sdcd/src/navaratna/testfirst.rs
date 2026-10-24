//! **N5-1 - test-first with anchors and the stub check** (switch `nr.test_first`, off).
//!
//! Code that compiles can still be wrong, and a model that writes both the code and its tests can make the same mistake in both. So for a
//! step tagged `code` that has a testable done-when:
//!
//! 1. a fresh model call writes one to five small tests from the **done-when only** - it never sees the code;
//! 2. SDC runs them against the **stub** (the tree as it is, the target unimplemented): a new test must *fail* there, or it is vacuous and is
//!    discarded;
//! 3. a **separate** fresh call writes the code - it never sees, and may not touch, the new tests;
//! 4. the new tests and every **anchor** test (written by a person, or passing before) are run; anchors must stay green and byte-identical;
//! 5. a new test that keeps failing while every anchor passes and the code looks right is **flagged for review**, never silently deleted.
//!
//! The model calls and the test runner are given in, so the whole flow runs here against scripted fakes.

use std::collections::{BTreeMap, BTreeSet};

/// The project's files, path to text: the tree the runner runs against.
pub type Tree = BTreeMap<String, String>;

/// Tests per step: at least one, at most five.
pub const MAX_TESTS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Code,
    /// Configuration, a rename, docs, boilerplate: test-first is not used.
    Trivial,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub id: String,
    pub tag: Tag,
    pub done_when: String,
    pub spec: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestFile {
    pub path: String,
    pub content: String,
    /// The tests in it, as ids (`path::name`).
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchors {
    /// Test ids that must stay green.
    pub ids: BTreeSet<String>,
    /// Files that hold them: byte-identical after the step.
    pub files: BTreeSet<String>,
}

/// The three outside parties of the flow.
pub trait Parties {
    /// Writes tests from the done-when alone.
    fn write_tests(&mut self, done_when: &str) -> Vec<TestFile>;
    /// Writes the code from the step's spec. Never given the new tests. Answers the files it wants written.
    fn write_code(&mut self, spec: &str) -> Vec<(String, String)>;
    /// Runs the named tests against a tree: id to passed.
    fn run(&mut self, tree: &Tree, ids: &[String]) -> BTreeMap<String, bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Test-first was not used (switch off, a trivial step, no testable done-when, or every test was vacuous).
    Skipped(String),
    /// Green: the tree after the step, and the tests that stand.
    Done { tree: Tree, new_tests: Vec<String>, discarded_vacuous: Vec<String> },
    /// The step touched an anchor test, or broke one. The tree is as it was.
    AnchorViolation { detail: String },
    /// A new test still fails while all anchors pass: for a person to look at. The tree has the code; the test is kept.
    Flagged { tree: Tree, failing: Vec<String>, discarded_vacuous: Vec<String> },
}

fn test_path(path: &str) -> bool {
    crate::graph::index::is_test_path(path)
}

/// Runs one step test-first against `base`.
pub fn run_step(step: &Step, base: &Tree, anchors: &Anchors, parties: &mut dyn Parties) -> Outcome {
    if !crate::v3::on("nr.test_first") {
        return Outcome::Skipped("nr.test_first is off".into());
    }

    if step.tag == Tag::Trivial {
        return Outcome::Skipped("a trivial step is not tested first".into());
    }

    if step.done_when.trim().is_empty() {
        return Outcome::Skipped("the step has no testable done-when".into());
    }

    /* 1. tests from the done-when alone; a file that would replace an anchor's file is refused, the rest kept */
    let mut files: Vec<TestFile> = parties.write_tests(&step.done_when).into_iter().filter(|file| !anchors.files.contains(&file.path) && !base.contains_key(&file.path)).collect();
    let mut count = 0;

    for file in &mut files {
        file.names.truncate(MAX_TESTS.saturating_sub(count));
        count += file.names.len();
    }

    files.retain(|file| !file.names.is_empty());

    if files.is_empty() {
        return Outcome::Skipped("no usable new test was written".into());
    }

    let all_new: Vec<String> = files.iter().flat_map(|file| file.names.clone()).collect();
    let mut stub = base.clone();

    for file in &files {
        stub.insert(file.path.clone(), file.content.clone());
    }

    /* 2. each new test must fail against the stub */
    let on_stub = parties.run(&stub, &all_new);
    let (vacuous, standing): (Vec<String>, Vec<String>) = all_new.iter().cloned().partition(|id| on_stub.get(id).copied().unwrap_or(false));

    if standing.is_empty() {
        return Outcome::Skipped(format!("every new test passed on the stub, so none proves anything: {}", vacuous.join(", ")));
    }

    /* 3. the code, written without the tests */
    let edits = parties.write_code(&step.spec);
    let new_paths: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let mut after = base.clone();

    for (path, content) in &edits {
        if anchors.files.contains(path) || new_paths.contains(path.as_str()) || (test_path(path) && base.contains_key(path)) {
            return Outcome::AnchorViolation { detail: format!("the code call tried to write the test file {path}") };
        }

        after.insert(path.clone(), content.clone());
    }

    for file in &anchors.files {
        if after.get(file) != base.get(file) {
            return Outcome::AnchorViolation { detail: format!("anchor file {file} changed") };
        }
    }

    for file in &files {
        after.insert(file.path.clone(), file.content.clone());
    }

    /* 4. new tests and every anchor */
    let anchor_ids: Vec<String> = anchors.ids.iter().cloned().collect();
    let ran: Vec<String> = standing.iter().cloned().chain(anchor_ids.iter().cloned()).collect();
    let results = parties.run(&after, &ran);
    let broken: Vec<&String> = anchor_ids.iter().filter(|id| !results.get(*id).copied().unwrap_or(false)).collect();

    if !broken.is_empty() {
        return Outcome::AnchorViolation { detail: format!("anchor test(s) no longer pass: {}", broken.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(", ")) };
    }

    let failing: Vec<String> = standing.iter().filter(|id| !results.get(*id).copied().unwrap_or(false)).cloned().collect();

    /* 5. a test that still fails is flagged, not deleted */
    if failing.is_empty() {
        Outcome::Done { tree: after, new_tests: standing, discarded_vacuous: vacuous }
    } else {
        Outcome::Flagged { tree: after, failing, discarded_vacuous: vacuous }
    }
}

#[cfg(test)]
mod tests;
