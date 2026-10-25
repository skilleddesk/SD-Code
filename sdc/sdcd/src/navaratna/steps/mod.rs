//! **The step executor** - what turns the NAVARATNA libraries from parts into a way of working (switch `nr.fresh_step`, off).
//!
//! The chat loop keeps one growing conversation; a 9B model loses the middle of it. A big request is instead run as **steps**: the request is cut
//! and labelled (`plan`), each requirement becomes one step, and each step is a *fresh, short prompt* built from the ledger (`pack`) that asks for
//! edits in plain headed code blocks (`edits`). Around that one call everything else of the plan acts:
//!
//! 1. the step runs under a hidden git snapshot (N6-2) - a failure puts the tree back exactly, a restart skips steps that are done;
//! 2. for a `code` step with a testable done-when, tests are written first, must fail on the stub, and the code is written by a separate call (N5-1);
//! 3. edits go through the symbol patcher, the contract lock, the syntax gate and the step transaction (N4-2, N6-3, N4-3);
//! 4. the project's own checks judge the result; a failure is cut to its first error (N3-3) and the step is asked again with it, twice;
//! 5. then several candidates at different temperatures, the smallest passing diff winning (N7-1);
//! 6. then the relay, if the person allowed it, with the escalation packet (N8-2) - otherwise the step is honestly *blocked* and the others go on;
//! 7. a long document is written section by section with continuation (N4-4);
//! 8. what a verified step left is published: contracts locked, glossary and registry updated, a handover note for the next step (N6-1, N6-3).
//!
//! The model and the project are traits, so all of it is tested here on scripted fakes.

pub mod edits;
pub mod plan;

/// The temperature and thinking of the *next* model request on this thread (a step's candidate, a fix step): the request builder takes them once.
pub mod sampling {
    use std::cell::Cell;

    thread_local! {
        static NEXT: Cell<Option<(Option<f32>, Option<bool>)>> = const { Cell::new(None) };
    }

    pub fn set(temperature: Option<f32>, think: Option<bool>) {
        NEXT.with(|slot| slot.set((temperature.is_some() || think.is_some()).then_some((temperature, think))));
    }

    pub fn take() -> Option<(Option<f32>, Option<bool>)> {
        NEXT.with(Cell::take)
    }
}

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::navaratna::{advice, bestofn, chunk, contracts, errslice, pack, registry, route, shadow, symbols, testfirst, think};
use plan::{Plan, PlanStep};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Label,
    Translate,
    Tests,
    Code,
    Retry,
    Candidate,
    DocOutline,
    DocSection,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub purpose: Purpose,
    pub prompt: String,
    pub temperature: Option<f32>,
    pub think: Option<bool>,
}

impl Request {
    pub fn new(purpose: Purpose, prompt: String) -> Self {
        Self { purpose, prompt, temperature: None, think: None }
    }
}

/// The model. One completion per call, no tools.
pub trait Engine {
    fn complete(&mut self, request: &Request) -> Result<String, String>;

    fn name(&self) -> String {
        "local".into()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Check {
    /// A check ran at all (the folder has some).
    pub ran: bool,
    pub passed: bool,
    pub log: String,
}

/// The project the steps change.
pub trait Project {
    fn root(&self) -> PathBuf;
    fn read(&self, path: &str) -> Option<String>;
    fn files(&self) -> Vec<String>;
    /// Writes through the syntax gate and the step transaction; `Err` is the refusal sentence for the model.
    fn apply(&mut self, step: &str, writes: &[(String, String)]) -> Result<(), String>;
    /// Raw writes (`None` deletes) - restoring, trials. Not gated.
    fn put(&mut self, writes: &[(String, Option<String>)]);
    /// The project's own checks.
    fn checks(&mut self) -> Check;
    /// Runs named tests with `writes` laid over the tree for the run (and removed again). `None`: this project cannot run single tests.
    fn run_tests(&mut self, writes: &[(String, String)], ids: &[String]) -> Option<BTreeMap<String, bool>>;
    /// The relay: the chain is asked to do the step. `Some(check)` if it was asked (and what the checks say after); `None` if it may not be.
    fn escalate(&mut self, step: &PlanStep, error: &str, touched: &[String]) -> Option<Check>;
    fn progress(&mut self, line: &str);
}

/// Puts files back to what they were. `before` is `(path, old text or None if it did not exist)`.
fn restore(project: &mut dyn Project, before: &[(String, Option<String>)]) {
    project.put(before);
}

fn snapshot(project: &dyn Project, paths: &[String]) -> Vec<(String, Option<String>)> {
    paths.iter().map(|path| (path.clone(), project.read(path))).collect()
}

/// Lays `writes` over the tree, runs the checks, and puts everything back.
fn trial(project: &mut dyn Project, writes: &[(String, String)]) -> Check {
    let paths: Vec<String> = writes.iter().map(|(path, _)| path.clone()).collect();
    let before = snapshot(project, &paths);

    project.put(&writes.iter().map(|(path, text)| (path.clone(), Some(text.clone()))).collect::<Vec<_>>());

    let check = project.checks();

    restore(project, &before);
    check
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Tokens one step's prompt may use (the step target of the window).
    pub budget_tokens: u64,
    /// Retries after the first attempt, each with the sliced error.
    pub retries: u32,
    /// Candidates for Best-of-N (used only with `nr.best_of_n` on).
    pub candidates: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { budget_tokens: 6_000, retries: 2, candidates: 3 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doneness {
    /// The project's checks ran and passed.
    Verified,
    /// The folder has no checks: nothing is proven, and it is said.
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    Done { doneness: Doneness, edited: Vec<String>, attempts: u32, how: &'static str },
    /// The journal says an earlier run finished this step.
    Resumed,
    Blocked { reason: String },
}

#[derive(Debug, Clone)]
pub struct Report {
    pub steps: Vec<(PlanStep, StepResult)>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn blocked(&self) -> Vec<&PlanStep> {
        self.steps.iter().filter(|(_, result)| matches!(result, StepResult::Blocked { .. })).map(|(step, _)| step).collect()
    }

    pub fn local_share(&self) -> (usize, usize) {
        let done = self.steps.iter().filter(|(_, result)| matches!(result, StepResult::Done { .. })).count();
        let local = self.steps.iter().filter(|(_, result)| matches!(result, StepResult::Done { how, .. } if *how != "relay")).count();

        (local, done)
    }

    /// The summary the person reads.
    pub fn render(&self, registry: &registry::Registry, requirements: &[String]) -> String {
        let mut out = String::new();

        for (step, result) in &self.steps {
            let line = match result {
                StepResult::Done { doneness, edited, attempts, how } => format!("done ({how}, {attempts} attempt(s), {}): {}", if *doneness == Doneness::Verified { "checks passed" } else { "no checks in this folder - not proven" }, edited.join(", ")),
                StepResult::Resumed => "done earlier (resumed)".to_string(),
                StepResult::Blocked { reason } => format!("BLOCKED: {reason}"),
            };

            out.push_str(&format!("- {} [{}] {} - {line}\n", step.id, step.r_ids.join(","), step.goal.chars().take(80).collect::<String>()));
        }

        let (local, done) = self.local_share();

        out.push_str(&format!("\n{done} of {} steps done, {local} of them by the local model.\n", self.steps.len()));

        for note in &self.notes {
            out.push_str(&format!("note: {note}\n"));
        }

        out.push_str("\nRequirements:\n");

        for line in registry::receipt(requirements, registry) {
            out.push_str(&format!("- {} <- {} by {} ({})\n", line.r_id, line.step, line.model, line.evidence));
        }

        out
    }
}

const RULES: &str = "You are doing ONE step of a larger job. Answer ONLY with the edits for this step, no explanation. Each edit is a header and a code block:\n\
FILE: path/relative/to/the/project\n\
SYMBOL: Qualified::name   (to change one declaration, e.g. Cart::total or Cart.total; leave the line out to write a new file)\n\
MODE: replace | insert_after | create   (replace is the default with SYMBOL)\n\
then the COMPLETE new code in a fenced block. Never leave code out (no `// ...`, no `rest unchanged`); a block with SYMBOL must define exactly that one declaration. Keep every name, signature and constant the contracts and glossary list.";

/// Edits that are in, what the files were before, and what the checks said.
struct Landed {
    writes: Vec<(String, String)>,
    before: Vec<(String, Option<String>)>,
    check: Check,
}

struct Exec<'a> {
    engine: &'a mut dyn Engine,
    project: &'a mut dyn Project,
    config: &'a Config,
    root: PathBuf,
    lock: contracts::Lock,
    glossary: contracts::Glossary,
    registry: registry::Registry,
    handover: Vec<String>,
    notes: Vec<String>,
}

/// Runs a plan to the end. Blocked steps do not stop the others.
pub fn run(plan: &Plan, engine: &mut dyn Engine, project: &mut dyn Project, config: &Config) -> (Report, registry::Registry) {
    let root = project.root();
    let mut exec = Exec {
        lock: contracts::Lock::load(&root),
        glossary: contracts::Glossary::load(&root),
        registry: registry::Registry::load(&root),
        handover: Vec::new(),
        notes: plan.notes.clone(),
        engine,
        project,
        config,
        root,
    };
    let mut results = Vec::new();

    for step in &plan.steps {
        exec.project.progress(&format!("{} - {}", step.id, step.goal.chars().take(100).collect::<String>()));

        let result = exec.run_one(plan, step);

        exec.project.progress(&match &result {
            StepResult::Done { how, doneness, .. } => format!("{} done ({how}{})", step.id, if *doneness == Doneness::Unverified { ", not proven" } else { "" }),
            StepResult::Resumed => format!("{} was done earlier", step.id),
            StepResult::Blocked { reason } => format!("{} blocked: {reason}", step.id),
        });
        results.push((step.clone(), result));
    }

    let _ = exec.registry.save(&exec.root);
    let _ = exec.lock.save(&exec.root);
    let _ = exec.glossary.save(&exec.root);

    let registry = exec.registry.clone();

    (Report { steps: results, notes: exec.notes }, registry)
}

impl Exec<'_> {
    /// One step under the shadow: a failure puts the tree back, a restart skips what is done.
    fn run_one(&mut self, plan: &Plan, step: &PlanStep) -> StepResult {
        let mut result = StepResult::Blocked { reason: "not run".into() };
        let root = self.root.clone();
        let ran = shadow::run_step(&root, &step.id, &mut || {
            result = if step.doc { self.doc_step(plan, step) } else { self.code_step(plan, step) };

            match &result {
                StepResult::Blocked { reason } => Err(reason.clone()),
                _ => Ok(()),
            }
        });

        match ran {
            Ok(shadow::Ran::Skipped) => StepResult::Resumed,
            Ok(_) => result,
            Err(error) => {
                self.notes.push(format!("{}: the shadow could not be used ({error}); the step ran without it", step.id));

                if matches!(result, StepResult::Blocked { ref reason } if reason == "not run") {
                    result = if step.doc { self.doc_step(plan, step) } else { self.code_step(plan, step) };
                }

                result
            }
        }
    }

    fn build_pack(&self, plan: &Plan, step: &PlanStep, feedback: Option<&str>) -> Result<pack::Pack, String> {
        let units: Vec<&crate::navaratna::spec::Unit> = step.unit_ids.iter().filter_map(|id| plan.map.units.iter().find(|unit| &unit.id == id)).collect();
        let text: String = units.iter().map(|unit| unit.text.as_str()).collect::<Vec<_>>().join("\n");
        let files: Vec<pack::FileCtx> = step
            .files
            .iter()
            .filter_map(|path| {
                let text_of_file = self.project.read(path)?;
                let targets: Vec<String> = symbols::index_text(path, &text_of_file).map(|all| all.into_iter().filter(|symbol| symbol.depth <= 1 && word_in(&text, &symbol.name)).map(|symbol| symbol.qualified).collect()).unwrap_or_default();

                Some(pack::FileCtx { path: path.clone(), text: text_of_file, targets })
            })
            .collect();
        /* the architect's empty signatures for this step's requirements come first: the names and shapes to build */
        let mut blueprint_contracts: Vec<String> = plan.contracts.iter().filter(|contract| contract.requirements.is_empty() || contract.requirements.iter().any(|r| step.r_ids.contains(r))).map(|contract| format!("{}: {}", contract.file, contract.signature.lines().next().unwrap_or_default().trim())).collect();
        let mut contracts: Vec<String> = self.lock.signatures.iter().filter(|(key, _)| step.files.iter().any(|file| key.starts_with(&format!("{file}::")))).map(|(key, locked)| format!("{key}: {}", locked.signature)).collect();

        blueprint_contracts.append(&mut contracts);
        let mut contracts = blueprint_contracts;

        contracts.truncate(40);

        let mut background = plan.background.join(" ");

        background = background.chars().take(900).collect();

        let rules = if background.is_empty() { RULES.to_string() } else { format!("{RULES}\n\nBackground from the request: {background}") };
        let mut handover = self.handover.clone();

        if let Some(feedback) = feedback {
            handover.push(format!("The last attempt at THIS step failed - fix exactly this: {feedback}"));
        }

        let input = pack::StepInput {
            rules: &rules,
            goal: &step.goal,
            requirement_ids: step.r_ids.clone(),
            done_when: &step.done_when,
            units,
            files,
            contracts,
            glossary: self.glossary.pairs(),
            handover,
            example: None,
            budget_tokens: self.config.budget_tokens,
        };

        pack::build(&input, &pack::rank_by_terms(&step.goal)).map_err(|split| split.to_string())
    }

    /// Reads an answer, lays its edits over the tree, judges them: `Ok` when they are in and the checks are not red.
    fn try_answer(&mut self, step: &PlanStep, answer: &str) -> Result<Landed, String> {
        let parsed = edits::parse(answer)?;

        /* N6-1: nothing is built twice - a file an earlier step built is changed, not created again */
        for edit in parsed.iter().filter(|edit| edit.mode == edits::Mode::Create) {
            if let Some(entry) = self.registry.find(&edit.path).cloned() {
                if let registry::CreateDecision::ConvertToEdit { why, .. } = registry::before_create(&mut self.registry, &self.root, &edit.path) {
                    return Err(format!("{why}; {} built it for {}. Change it with FILE and SYMBOL instead of creating it again.", entry.step, entry.r_id));
                }
            }
        }
        let writes = edits::apply(&parsed, &|path| self.project.read(path))?;

        for (path, text) in &writes {
            if let Err(violations) = contracts::check_patch(&self.lock, path, text) {
                return Err(violations.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "));
            }
        }

        let paths: Vec<String> = writes.iter().map(|(path, _)| path.clone()).collect();
        let before = snapshot(self.project, &paths);

        self.project.apply(&step.id, &writes)?;

        let check = self.project.checks();

        if check.ran && !check.passed {
            restore(self.project, &before);

            let slice = errslice::slice(&self.root, &check.log, None, 700).map_or_else(|| check.log.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"), |slice| slice.text);

            return Err(format!("the project's checks failed after your edit (it was undone):\n{slice}"));
        }

        Ok(Landed { writes, before, check })
    }

    fn finish(&mut self, plan: &Plan, step: &PlanStep, landed: &Landed, attempts: u32, how: &'static str) -> StepResult {
        let Landed { writes, before, check } = landed;
        let mut changed_symbols = Vec::new();
        let mut first_evidence = None;

        for (path, text) in writes {
            let old = before.iter().find(|(p, _)| p == path).and_then(|(_, t)| t.clone()).unwrap_or_default();
            let names = crate::rotation::handover::symbol_changes(path, &old, text);

            if first_evidence.is_none() {
                let line = symbols::index_text(path, text).and_then(|all| all.into_iter().find(|symbol| names.iter().any(|name| name.contains(&symbol.name))).map(|symbol| symbol.start_line)).unwrap_or(1);

                first_evidence = Some(format!("{path}:{line}"));
            }

            changed_symbols.push(format!("{path}: {}", if names.is_empty() { "whole file".to_string() } else { names.join(", ") }));
            self.glossary.harvest(path, text);
        }

        /* the warnings the edit tools would have attached: a copy of something that exists, a member that does not */
        for (path, text) in writes {
            for note in registry::duplicate_note(&self.root, path, text).into_iter().chain(crate::navaratna::exist::note(&self.root, path, text)) {
                self.notes.push(format!("{}: {}", step.id, note.lines().take(3).collect::<Vec<_>>().join(" ")));
            }
        }

        let verified = check.ran && check.passed;

        for (path, text) in writes {
            self.registry.record(registry::Entry {
                artifact: path.clone(),
                r_id: step.r_ids.first().cloned().unwrap_or_default(),
                step: step.id.clone(),
                model: if how == "relay" { "chain".to_string() } else { self.engine.name() },
                hash: registry::content_hash(text),
                evidence: if verified { format!("{} (checks passed)", first_evidence.clone().unwrap_or_default()) } else { first_evidence.clone().unwrap_or_default() },
            });

            if verified {
                self.lock.publish(path, text, &step.id);
            }
        }

        /* the other R-IDs of a step share its evidence */
        for r_id in step.r_ids.iter().skip(1) {
            if let Some(first) = writes.first() {
                self.registry.record(registry::Entry { artifact: format!("{}#{r_id}", first.0), r_id: r_id.clone(), step: step.id.clone(), model: self.engine.name(), hash: String::new(), evidence: first_evidence.clone().unwrap_or_default() });
            }
        }

        self.handover.push(format!("{} ({}) done: {}", step.id, step.r_ids.join(","), changed_symbols.join("; ")));
        let _ = plan;

        StepResult::Done { doneness: if verified { Doneness::Verified } else { Doneness::Unverified }, edited: writes.iter().map(|(path, _)| path.clone()).collect(), attempts, how }
    }

    fn code_step(&mut self, plan: &Plan, step: &PlanStep) -> StepResult {
        let mut feedback: Option<String> = None;
        let mut attempts = 0;
        let mut has_check = false;

        /* N5-1: tests first, from the done-when only, for a `code` step */
        if crate::v3::on("nr.test_first") && step.tag == route::Tag::Code {
            match self.test_first(plan, step) {
                TestFirst::Done(result) => return result,
                TestFirst::Failed(reason) => feedback = Some(reason),
                TestFirst::Skipped => {}
            }
        }

        for attempt in 0..=self.config.retries {
            attempts = attempt + 1;

            let pack = match self.build_pack(plan, step, feedback.as_deref()) {
                Ok(pack) => pack,
                Err(split) => return StepResult::Blocked { reason: split },
            };
            let mut request = Request::new(if attempt == 0 { Purpose::Code } else { Purpose::Retry }, pack.text);

            if crate::v3::on("nr.think_budget") {
                request.think = think::decide(if attempt == 0 { think::Kind::Other } else { think::Kind::ErrorFix }, true);
            }

            let answer = match self.engine.complete(&request) {
                Ok(answer) => answer,
                Err(error) if error.contains("ContextOverflow") => return StepResult::Blocked { reason: error },
                Err(error) => {
                    feedback = Some(format!("the model call failed: {error}"));
                    continue;
                }
            };

            match self.try_answer(step, &answer) {
                Ok(landed) => return self.finish(plan, step, &landed, attempts, if attempt == 0 { "local" } else { "local, retried" }),
                Err(error) => {
                    has_check |= error.contains("checks failed");
                    feedback = Some(error);
                }
            }
        }

        let error = feedback.unwrap_or_default();

        /* N7-1: several candidates, the tests choose */
        let trigger = bestofn::Trigger { normal_failed: true, sliced_retries_failed: self.config.retries, hard: step.tag == route::Tag::Hard, has_check };

        if bestofn::should_run(&trigger).is_ok() {
            if let Some(result) = self.best_of_n(plan, step, &error, attempts) {
                return result;
            }
        }

        /* the relay, if it may be asked */
        let touched: Vec<String> = step.files.clone();

        if let Some(check) = self.project.escalate(step, &error, &touched) {
            if !check.ran || check.passed {
                let writes: Vec<(String, String)> = touched.iter().filter_map(|path| self.project.read(path).map(|text| (path.clone(), text))).collect();

                return self.finish(plan, step, &Landed { writes, before: Vec::new(), check }, attempts, "relay");
            }

            return StepResult::Blocked { reason: format!("the local model could not do it and the chain did not pass the checks either: {}", first_line(&error)) };
        }

        StepResult::Blocked { reason: format!("the local model could not do it in {attempts} attempt(s): {}", first_line(&error)) }
    }

    fn best_of_n(&mut self, plan: &Plan, step: &PlanStep, error: &str, attempts: u32) -> Option<StepResult> {
        struct Parties<'p, 'x> {
            exec: &'p mut Exec<'x>,
            plan: &'p Plan,
            step: &'p PlanStep,
            error: &'p str,
        }

        impl bestofn::Parties for Parties<'_, '_> {
            fn candidate(&mut self, _base: &bestofn::Tree, temperature: f32, _index: usize) -> Vec<(String, String)> {
                let Ok(pack) = self.exec.build_pack(self.plan, self.step, Some(self.error)) else { return Vec::new() };
                let mut request = Request::new(Purpose::Candidate, pack.text);

                request.temperature = Some(temperature);

                let Ok(answer) = self.exec.engine.complete(&request) else { return Vec::new() };
                let Ok(parsed) = edits::parse(&answer) else { return Vec::new() };

                edits::apply(&parsed, &|path| self.exec.project.read(path)).unwrap_or_default()
            }

            fn check(&mut self, tree: &bestofn::Tree) -> bestofn::Check {
                let writes: Vec<(String, String)> = tree.iter().map(|(path, text)| (path.clone(), text.clone())).collect();

                if writes.is_empty() {
                    return bestofn::Check { passed: false, failures: usize::MAX, error: "no usable edit".into() };
                }

                if writes.iter().any(|(path, text)| contracts::check_patch(&self.exec.lock, path, text).is_err()) {
                    return bestofn::Check { passed: false, failures: usize::MAX - 1, error: "a locked contract would change".into() };
                }

                let check = trial(self.exec.project, &writes);

                bestofn::Check { passed: check.ran && check.passed, failures: 1, error: errslice::slice(&self.exec.root, &check.log, None, 500).map_or(check.log, |slice| slice.text) }
            }
        }

        /* the base tree is empty and each candidate's tree is just its own writes: the trial lays them over the real files and puts them back */
        let n = self.config.candidates;
        let outcome = bestofn::run(&bestofn::Tree::new(), n, &mut Parties { exec: self, plan, step, error });

        match outcome {
            bestofn::Outcome::Chosen { writes, .. } => {
                let paths: Vec<String> = writes.iter().map(|(path, _)| path.clone()).collect();
                let before = snapshot(self.project, &paths);

                if self.project.apply(&step.id, &writes).is_err() {
                    return None;
                }

                let check = self.project.checks();

                if check.ran && !check.passed {
                    restore(self.project, &before);
                    return None;
                }

                Some(self.finish(plan, step, &Landed { writes, before, check }, attempts + n as u32, "local, best of n"))
            }
            bestofn::Outcome::HandOver { .. } => None,
        }
    }

    fn test_first(&mut self, plan: &Plan, step: &PlanStep) -> TestFirst {
        struct Parties<'p, 'x> {
            exec: &'p mut Exec<'x>,
            plan: &'p Plan,
            step: &'p PlanStep,
            last_error: Option<String>,
        }

        impl testfirst::Parties for Parties<'_, '_> {
            fn write_tests(&mut self, done_when: &str) -> Vec<testfirst::TestFile> {
                let prompt = format!(
                    "Write one to five SMALL tests for this requirement and nothing else. Do not write the implementation; you have not seen it.\nRequirement: {done_when}\nAnswer with, for each test file, `FILE: tests/path` and `TESTS: name_one, name_two` and the file in a fenced block.",
                );
                let Ok(answer) = self.exec.engine.complete(&Request::new(Purpose::Tests, prompt)) else { return Vec::new() };

                parse_test_files(&answer)
            }

            fn write_code(&mut self, _spec: &str) -> Vec<(String, String)> {
                let Ok(pack) = self.exec.build_pack(self.plan, self.step, None) else { return Vec::new() };
                let Ok(answer) = self.exec.engine.complete(&Request::new(Purpose::Code, pack.text)) else { return Vec::new() };

                match edits::parse(&answer).and_then(|parsed| edits::apply(&parsed, &|path| self.exec.project.read(path))) {
                    Ok(writes) => writes,
                    Err(error) => {
                        self.last_error = Some(error);
                        Vec::new()
                    }
                }
            }

            fn run(&mut self, tree: &testfirst::Tree, ids: &[String]) -> BTreeMap<String, bool> {
                let writes: Vec<(String, String)> = tree.iter().filter(|(path, text)| self.exec.project.read(path).as_ref() != Some(*text)).map(|(path, text)| (path.clone(), text.clone())).collect();

                self.exec.project.run_tests(&writes, ids).unwrap_or_default()
            }
        }

        let mut base = testfirst::Tree::new();

        for path in self.project.files() {
            if crate::graph::index::is_test_path(&path) {
                if let Some(text) = self.project.read(&path) {
                    base.insert(path, text);
                }
            }
        }

        /* every file the step may change is part of the base tree, so a code edit is a difference from it */
        for path in &step.files {
            if let Some(text) = self.project.read(path) {
                base.entry(path.clone()).or_insert(text);
            }
        }

        /* can this project run a single test at all? Ask with no tests: `None` means no. */
        if self.project.run_tests(&[], &[]).is_none() {
            return TestFirst::Skipped;
        }

        let anchors = testfirst::Anchors { ids: Default::default(), files: base.keys().filter(|path| crate::graph::index::is_test_path(path)).cloned().collect() };
        let test_step = testfirst::Step { id: step.id.clone(), tag: testfirst::Tag::Code, done_when: step.done_when.clone(), spec: step.goal.clone() };
        let outcome = testfirst::run_step(&test_step, &base, &anchors, &mut Parties { exec: self, plan, step, last_error: None });

        match outcome {
            testfirst::Outcome::Skipped(reason) => {
                self.notes.push(format!("{}: test-first skipped ({reason})", step.id));
                TestFirst::Skipped
            }
            testfirst::Outcome::AnchorViolation { detail } => TestFirst::Failed(format!("test-first: {detail}")),
            testfirst::Outcome::Flagged { tree, failing, .. } => {
                self.notes.push(format!("{}: a new test keeps failing while the code looks right - kept for a person to read: {}", step.id, failing.join(", ")));
                self.land(plan, step, &base, &tree).map_or(TestFirst::Skipped, TestFirst::Done)
            }
            testfirst::Outcome::Done { tree, new_tests, discarded_vacuous } => {
                if !discarded_vacuous.is_empty() {
                    self.notes.push(format!("{}: discarded vacuous test(s): {}", step.id, discarded_vacuous.join(", ")));
                }

                let _ = new_tests;
                self.land(plan, step, &base, &tree).map_or(TestFirst::Skipped, TestFirst::Done)
            }
        }
    }

    /// Writes the tree test-first ended with onto the project, through the gates, and judges it with the whole check.
    fn land(&mut self, plan: &Plan, step: &PlanStep, base: &testfirst::Tree, tree: &testfirst::Tree) -> Option<StepResult> {
        let writes: Vec<(String, String)> = tree.iter().filter(|(path, text)| base.get(*path) != Some(*text)).map(|(path, text)| (path.clone(), text.clone())).collect();

        if writes.is_empty() {
            return None;
        }

        for (path, text) in &writes {
            if contracts::check_patch(&self.lock, path, text).is_err() {
                return None;
            }
        }

        let paths: Vec<String> = writes.iter().map(|(path, _)| path.clone()).collect();
        let before = snapshot(self.project, &paths);

        if self.project.apply(&step.id, &writes).is_err() {
            return None;
        }

        let check = self.project.checks();

        if check.ran && !check.passed {
            restore(self.project, &before);
            return None;
        }

        Some(self.finish(plan, step, &Landed { writes, before, check }, 1, "local, tests first"))
    }

    /// A long document, section by section (N4-4).
    fn doc_step(&mut self, plan: &Plan, step: &PlanStep) -> StepResult {
        if !crate::v3::on("nr.out_chunk") {
            return self.code_step_no_doc(plan, step);
        }

        let outline = match self.engine.complete(&Request::new(Purpose::DocOutline, format!("List the section titles, one per line and nothing else (at most 12), for this document:\n{}", step.goal))) {
            Ok(text) => text,
            Err(error) => return StepResult::Blocked { reason: format!("no outline: {error}") },
        };
        let titles: Vec<String> = outline.lines().map(|line| line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | '*' | '#' | ' ')).trim().to_string()).filter(|title| !title.is_empty()).take(12).collect();

        if titles.is_empty() {
            return StepResult::Blocked { reason: "the model gave no section titles".into() };
        }

        let name = step.id.clone();
        let goal = step.goal.clone();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let mut writer = |prompt: &str| self.engine.complete(&Request::new(Purpose::DocSection, prompt.to_string())).unwrap_or_default();
        if let Err(error) = chunk::write_document(&self.root, &name, &goal, &refs, &mut writer) {
            return StepResult::Blocked { reason: format!("the sections could not be stored: {error}") };
        }

        match chunk::join(&self.root, &name) {
            Ok(text) => {
                let path = step.files.iter().find(|file| file.ends_with(".md")).cloned().unwrap_or_else(|| format!("docs/{}.md", step.r_ids.first().map_or("document", String::as_str)));
                let writes = vec![(path, format!("{text}\n"))];
                let before = snapshot(self.project, &[writes[0].0.clone()]);

                if let Err(refusal) = self.project.apply(&step.id, &writes) {
                    return StepResult::Blocked { reason: refusal };
                }

                let check = Check { ran: false, passed: true, log: String::new() };

                self.finish(plan, step, &Landed { writes, before, check }, 1, "local, in sections")
            }
            Err(open) => StepResult::Blocked { reason: format!("section(s) {open:?} never reached their end marker, so the document was not joined (the finished sections are kept in .sdc/docs/{name}/)") },
        }
    }

    fn code_step_no_doc(&mut self, plan: &Plan, step: &PlanStep) -> StepResult {
        let mut copy = step.clone();

        copy.doc = false;
        self.code_step(plan, &copy)
    }
}

enum TestFirst {
    Done(StepResult),
    Failed(String),
    Skipped,
}

fn first_line(text: &str) -> String {
    text.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().chars().take(200).collect()
}

fn word_in(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().last();
        let after = text[at + word.len()..].chars().next();

        !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// The test files in a model's answer: `FILE:` + optional `TESTS:` + a block; without `TESTS:` the names are read from the code.
pub fn parse_test_files(answer: &str) -> Vec<testfirst::TestFile> {
    let Ok(parsed) = edits::parse(answer) else { return Vec::new() };
    let listed: Vec<&str> = answer.lines().filter_map(|line| line.trim().trim_start_matches(['*', ' ']).strip_prefix("TESTS:")).collect();

    parsed
        .into_iter()
        .enumerate()
        .filter(|(_, edit)| edit.mode == edits::Mode::Create && crate::graph::index::is_test_path(&edit.path))
        .map(|(n, edit)| {
            let mut names: Vec<String> = listed.get(n).map(|line| line.split(',').map(|name| name.trim().trim_matches(['*', '`', ' ']).to_string()).filter(|name| !name.is_empty()).collect()).unwrap_or_default();

            if names.is_empty() {
                names = test_names(&edit.code);
            }

            testfirst::TestFile { names: names.into_iter().map(|name| format!("{}::{name}", edit.path)).collect(), path: edit.path, content: edit.code }
        })
        .collect()
}

/// The test names in a file's text: `def test_x`, `#[test] fn x`, `test('x'`, `it("x"`.
pub fn test_names(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut after_test_attribute = false;

    for line in code.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("def test").or_else(|| line.strip_prefix("async def test")) {
            names.push(format!("test{}", rest.split('(').next().unwrap_or_default()));
        } else if line.starts_with("#[test") || line.starts_with("#[tokio::test") {
            after_test_attribute = true;
            continue;
        } else if after_test_attribute {
            if let Some(rest) = line.strip_prefix("fn ").or_else(|| line.strip_prefix("async fn ")) {
                names.push(rest.split('(').next().unwrap_or_default().to_string());
            }
        } else if let Some(rest) = line.strip_prefix("test(").or_else(|| line.strip_prefix("it(")) {
            if let Some(name) = rest.trim_start_matches(['\'', '"', '`']).split(['\'', '"', '`']).next().filter(|name| !name.is_empty()) {
                names.push(name.to_string());
            }
        }

        after_test_attribute = false;
    }

    names.truncate(testfirst::MAX_TESTS);
    names
}

/// The step target for a window and a model's quantization - the budget one step's prompt may use.
pub fn budget_for(window: u64, quantization: Option<&str>) -> u64 {
    advice::step_target(window, quantization).saturating_sub(window / 4).max(1_500)
}

#[cfg(test)]
mod tests;
