//! **The NAVARATNA step executor, in the agent** (switch `nr.fresh_step`, off): the glue between a turn and `navaratna::steps`.
//!
//! A big request to a local model, in Auto mode, is not run as one growing chat: it is cut into steps, each a fresh short prompt, each judged by the
//! project's own checks (see `navaratna::steps`). This file gives that executor its two hands - the model (`ask_model`, no tools, nothing streamed
//! into the chat) and the project (the folder's files through the Trust Kernel's rules, the project's own checks, the step transaction) - and the
//! relay (`relay::Hook`) for a step the local model could not do.
//!
//! It is used only where every guard of the chat loop still holds: the turn is Auto (so no per-edit question is skipped), the folder is local, a
//! checkpoint is taken before the first change, protected and secret paths are refused, the blast radius is kept.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use super::dialect::{self, Dialect};
use super::gate::Autonomy;
use super::relay;
use super::workspace::Workspace;
use super::{ask_model, Backend, Options, Prompt, Target};
use crate::engines::{EngineEvent, EventSink};
use crate::navaratna::steps::{self as nr, plan::PlanStep, Check, Config};

const SYSTEM: &str = "You are a careful programming assistant working on a project folder. Follow the requested answer format exactly and answer with nothing else.";

/// Is this turn one for the step executor?
pub(super) fn applies(backend: Backend, options: &Options, remote: bool, research: bool, request: &str, scope: &crate::v3::ModelCtx, repo: &crate::padma::scope::RepoStats) -> bool {
    /* PADMA P1-3: a long request is a big job - and so is a short one that points at more than one Context Pack (`padma.scope`). Either starts the executor. */
    let big = request.len() >= crate::navaratna::spec::BIG_PROMPT_CHARS || (crate::v3::on("padma.scope") && crate::padma::scope::classify(request, repo).large);

    backend == Backend::Ollama && !remote && !research && options.autonomy == Autonomy::Auto && big && crate::v3::on_in("nr.fresh_step", scope)
}

pub(super) struct Turn<'a> {
    pub backend: Backend,
    pub target: &'a Target,
    pub workspace: &'a Workspace,
    pub options: Options,
    pub policy: &'a crate::trust::policy::Policy,
    pub prompt: &'a Prompt,
    pub sink: &'a EventSink,
    pub checkpoint: Option<super::tools::Checkpointer>,
    pub gate: &'a crate::guard::modelgate::Gate,
    pub relay: &'a RefCell<Option<relay::Hook>>,
    pub request: &'a str,
    pub window: u64,
    pub turn_id: &'a str,
}

struct ModelEngine<'a, 'b> {
    turn: &'b Turn<'a>,
    stopped: &'b dyn Fn() -> bool,
    input: Cell<u64>,
    output: Cell<u64>,
    calls: Cell<usize>,
}

impl nr::Engine for ModelEngine<'_, '_> {
    fn complete(&mut self, request: &nr::Request) -> Result<String, String> {
        if (self.stopped)() {
            return Err("the turn was stopped".into());
        }

        /* A step answers in a fixed format from a short prompt: a thinking model (Qwen3.5) that is left to think spends minutes and often its whole output on hidden reasoning, which ends as "the answer holds no edit". Thinking is on only when the step asked for it (`nr.think_budget`). */
        nr::sampling::set(request.temperature, request.think.or(Some(false)));

        let messages = vec![dialect::user_message(&request.prompt)];
        let reply = ask_model(self.turn.backend, self.turn.target, (self.turn.turn_id, crate::guard::modelgate::Purpose::Main), SYSTEM, &messages, &[], &EventSink::discarding(), self.stopped)?;

        self.input.set(self.input.get() + reply.input_tokens);
        self.output.set(self.output.get() + reply.output_tokens);
        self.calls.set(self.calls.get() + 1);

        /* `SDC_STEP_TRACE=file`: every step prompt and answer is appended to the file, to see what a local model was asked and what it said. */
        if let Some(path) = std::env::var_os("SDC_STEP_TRACE") {
            use std::io::Write;

            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "===== PROMPT ({:?}) =====\n{}\n===== ANSWER ({} output tokens) =====\n{}\n", request.purpose, request.prompt, reply.output_tokens, reply.text);
            }
        }

        Ok(reply.text)
    }

    fn name(&self) -> String {
        format!("ollama:{}", self.turn.target.model)
    }
}

struct WorkspaceProject<'a, 'b> {
    turn: &'b Turn<'a>,
    root: PathBuf,
    plan: RefCell<Option<Vec<crate::verify::Check>>>,
    checkpointed: Cell<bool>,
    changed: RefCell<std::collections::HashSet<String>>,
    stopped: &'b dyn Fn() -> bool,
}

impl WorkspaceProject<'_, '_> {
    fn checks_plan(&self) -> Vec<crate::verify::Check> {
        self.plan
            .borrow_mut()
            .get_or_insert_with(|| {
                let names: Vec<String> = self.turn.workspace.list(".").map(|(entries, _)| entries.iter().map(|entry| entry.split(" (").next().unwrap_or(entry).trim_end_matches('/').to_string()).collect()).unwrap_or_default();

                crate::verify::plan(&names, &|name| self.turn.workspace.read(name).ok().map(|(text, _)| text), !cfg!(windows))
            })
            .clone()
    }

    fn run(&self, command: &str, seconds: u64) -> Check {
        let posix = !cfg!(windows);
        let line = crate::verify::non_interactive(command, posix);
        let (line, _) = crate::verify::sandboxed_for_mcp(&line, &self.root.to_string_lossy());

        match self.turn.workspace.run(&line, std::time::Duration::from_secs(seconds)) {
            Ok(report) => Check { ran: true, passed: report.ok, log: format!("{}\n{}", report.stdout, report.stderr) },
            Err(error) => Check { ran: true, passed: false, log: error.message },
        }
    }

    /// How a single test is run in this folder: `(label, command for path and name)`; `None` when the folder has no runner this knows.
    fn test_command(&self, path: &str, name: &str) -> Option<String> {
        let clean = |text: &str| text.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | ' ' | '-' | '/' | '.' | ':' | '(' | ')'));

        if !clean(path) || !clean(name) {
            return None;
        }

        let has = |file: &str| self.turn.workspace.exists(file);

        if has("Cargo.toml") {
            Some(format!("cargo test {name} --quiet"))
        } else if has("pytest.ini") || has("pyproject.toml") || has("setup.cfg") || has("tox.ini") || path.ends_with(".py") {
            /* the same interpreter name the project's own check uses: `python` on Windows (where `python3` is often only a Store stub) */
            Some(format!("{} -m pytest -q \"{path}::{name}\"", if cfg!(windows) { "python" } else { "python3" }))
        } else if has("package.json") && self.turn.workspace.read("package.json").is_ok_and(|(text, _)| text.contains("jest")) {
            Some(format!("npx jest \"{path}\" -t \"{name}\""))
        } else {
            None
        }
    }
}

impl nr::Project for WorkspaceProject<'_, '_> {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn read(&self, path: &str) -> Option<String> {
        match self.turn.workspace.read(path) {
            Ok((text, false)) => Some(text),
            _ => None,
        }
    }

    fn files(&self) -> Vec<String> {
        crate::search::files(&self.root, 3_000)
    }

    fn apply(&mut self, step: &str, writes: &[(String, String)]) -> Result<(), String> {
        for (path, text) in writes {
            if let Some(pattern) = self.turn.policy.protected(path, self.root.to_str()) {
                return Err(format!("{path} is protected by this project's policy (`{pattern}`); the step executor does not change protected files"));
            }

            if let Some(first) = crate::trust::scan::secrets_in_text(path, text).first() {
                return Err(format!("{path} would carry a secret ({}); put it in an environment variable", first["message"].as_str().unwrap_or("it looks like a key")));
            }
        }

        let max = self.turn.policy.max_files_per_turn;
        let mut changed = self.changed.borrow_mut();
        let new: usize = writes.iter().filter(|(path, _)| !changed.contains(path)).count();

        if max > 0 && changed.len() + new > max {
            return Err(format!("this would be the {}th file changed in one turn; the policy allows {max}", changed.len() + new));
        }

        if !self.checkpointed.replace(true) {
            if let Some(checkpoint) = &self.turn.checkpoint {
                checkpoint("Before the NAVARATNA steps");
            }
        }

        let edits: Vec<(String, Option<String>)> = writes.iter().map(|(path, text)| (path.clone(), Some(text.clone()))).collect();

        crate::runtime::step::apply(&self.root, step, &edits).map_err(|refusal| refusal.0)?;
        changed.extend(writes.iter().map(|(path, _)| path.clone()));
        Ok(())
    }

    fn put(&mut self, writes: &[(String, Option<String>)]) {
        for (path, text) in writes {
            let _ = match text {
                Some(text) => self.turn.workspace.write(path, text),
                None => self.turn.workspace.remove(path),
            };
        }
    }

    fn checks(&mut self) -> Check {
        let plan = self.checks_plan();

        if plan.is_empty() {
            return Check::default();
        }

        let mut ran = Vec::new();

        for check in plan.iter().take(3) {
            if (self.stopped)() {
                break;
            }

            let result = self.run(&check.command, 180);

            if !result.passed {
                return result;
            }

            ran.push(check.command.clone());
        }

        Check { ran: true, passed: true, log: ran.join("\n") }
    }

    fn run_tests(&mut self, writes: &[(String, String)], ids: &[String]) -> Option<BTreeMap<String, bool>> {
        /* with no tests named this only asks whether the folder can run a single test at all */
        let probe = self.test_command("tests/x.py", "x").is_some() || self.turn.workspace.exists("Cargo.toml") || self.turn.workspace.exists("package.json");

        if !probe {
            return None;
        }

        if ids.is_empty() {
            return Some(BTreeMap::new());
        }

        let paths: Vec<String> = writes.iter().map(|(path, _)| path.clone()).collect();
        let before: Vec<(String, Option<String>)> = paths.iter().map(|path| (path.clone(), nr::Project::read(self, path))).collect();

        nr::Project::put(self, &writes.iter().map(|(path, text)| (path.clone(), Some(text.clone()))).collect::<Vec<_>>());

        let results = ids
            .iter()
            .map(|id| {
                let (path, name) = id.split_once("::").unwrap_or(("", id));
                let passed = self.test_command(path, name).is_some_and(|command| self.run(&command, 120).passed);

                (id.clone(), passed)
            })
            .collect();

        nr::Project::put(self, &before);
        Some(results)
    }

    fn escalate(&mut self, step: &PlanStep, error: &str, touched: &[String]) -> Option<Check> {
        let mut slot = self.turn.relay.borrow_mut();
        let hook = slot.as_mut()?;
        let passed: Vec<String> = Vec::new();
        let env = relay::Env {
            workspace: self.turn.workspace,
            options: self.turn.options,
            policy: self.turn.policy,
            prompt: self.turn.prompt,
            sink: self.turn.sink,
            checkpoint: self.turn.checkpoint.clone(),
            dialect: self.turn.target.dialect,
            goal: &step.goal,
            changed: touched.to_vec(),
            passed_checks: &passed,
            error,
            local_model: &self.turn.target.model,
            call_timeout_s: hook.policy.call_timeout_s,
            gate: self.turn.gate,
        };
        let mut messages: Vec<Value> = Vec::new();

        /* the steps already failed three times: that is what the relay's rules read as "stuck" */
        hook.signals.verify_failures = 3;

        let handled = hook.handle(&env, &mut messages, &|_, _| None);
        let chain_failure = hook.failure.clone();

        drop(slot);

        match handled {
            relay::Handled::Done => Some(self.checks()),
            relay::Handled::Failed if !chain_failure.is_empty() => Some(Check { ran: true, passed: false, log: format!("{error}
(the chain could not do it either: {chain_failure})") }),
            relay::Handled::Failed => Some(Check { ran: true, passed: false, log: error.to_string() }),
            relay::Handled::Local => None,
        }
    }

    fn progress(&mut self, line: &str) {
        self.turn.sink.send(EngineEvent::Delta(format!("▸ {line}\n")));
    }
}

const ARCHITECT: &str = "You are the architect of a software job. You only design; you never write implementation. Answer with the single JSON object that was asked for and nothing else.";

/// N2-3 (`nr.blueprint`): for a large or tangled request, with the relay's consent, one call to the chain's architect row returns the skeleton of the
/// work; it is checked by code and, if good, becomes the plan. Any other case leaves the local plan as it is, and says why.
fn architect(turn: &Turn<'_>, plan: &mut nr::plan::Plan, rank: &dyn Fn(&str) -> Vec<String>, stopped: &dyn Fn() -> bool) {
    use crate::navaratna::{blueprint, roles};

    if !crate::v3::on("nr.blueprint") {
        return;
    }

    let mut slot = turn.relay.borrow_mut();
    let Some(hook) = slot.as_mut() else {
        plan.notes.push("Blueprint not used: the relay is not on for this turn (no consent, a local-only project, a model picked by hand, or no cloud row)".into());
        return;
    };
    let rows = roles::candidates_now(&hook.chain, roles::Role::Architect);

    let Some(row) = rows.into_iter().next() else {
        plan.notes.push("Blueprint not used: no healthy row in the chain".into());
        return;
    };

    if hook.used.cloud_steps >= hook.policy.max_cloud_steps_per_turn {
        plan.notes.push("Blueprint not used: this turn's cloud allowance is spent".into());
        return;
    }

    let context = blueprint::Context { relay_consent: hook.policy.enabled(), local_only: turn.policy.privacy_local(), manual_pin: false, architect_row: true };
    let fired = RefCell::new(Vec::new());
    let asked = Cell::new(false);
    let session = turn.prompt.session_id.clone();
    let local_map = plan.map.clone();
    let outcome = blueprint::run(
        &context,
        turn.request,
        &local_map,
        &|text| crate::rotation::capsule::clean(&session, "blueprint", text, &mut fired.borrow_mut()),
        &mut |request: &str| {
            asked.set(true);

            turn.gate.permit(crate::guard::modelgate::Purpose::Chain, Some(&row.provider), &row.model).map_err(|violation| violation.to_string())?;

            let nested = Prompt { provider: Some(row.provider.clone()), model: row.model.clone(), text: request.to_string(), history: Vec::new(), images: Vec::new(), resume: None, ..turn.prompt.clone() };
            let target = super::resolve_target(&nested)?;

            /* what is about to leave the machine is written to the audit first */
            let _ = crate::data::sent::append(
                std::path::Path::new(turn.workspace.root()),
                &crate::data::sent::Sent {
                    at: chrono::Utc::now().to_rfc3339(),
                    model: row.label(),
                    step: "blueprint".into(),
                    turn: turn.prompt.turn_id.clone(),
                    session: turn.prompt.session_id.clone(),
                    tokens: crate::tokens::estimate(request) as usize,
                    redactions: fired.borrow().clone(),
                    hash: crate::data::sent::hash(request),
                    left_the_machine: true,
                },
            );

            turn.sink.send(EngineEvent::Delta(format!("▸ the architect ({}) is asked for the skeleton of the work - the request's parts, secrets taken out.\n", row.label())));

            ask_model(Backend::Api, &target, (turn.turn_id, crate::guard::modelgate::Purpose::Chain), ARCHITECT, &[dialect::user_message(request)], &[], &EventSink::discarding(), stopped).map(|reply| reply.text)
        },
    );

    if asked.get() {
        hook.used.cloud_steps += 1;
    }

    match outcome {
        blueprint::Outcome::Blueprint(blueprint, map) => {
            if let Err(error) = nr::plan::adopt_blueprint(plan, &blueprint, *map, rank) {
                plan.notes.push(format!("Blueprint not used: {error}"));
            }
        }
        blueprint::Outcome::Local(reason) => plan.notes.push(format!("Blueprint not used: {reason}")),
    }
}

/// Runs the whole turn as steps and ends it. Never returns an error: whatever cannot be done is said in the summary.
pub(super) fn run(turn: &Turn<'_>, stopped: &dyn Fn() -> bool) {
    let root = PathBuf::from(turn.workspace.root());
    let mut engine = ModelEngine { turn, stopped, input: Cell::new(0), output: Cell::new(0), calls: Cell::new(0) };
    let mut project = WorkspaceProject { turn, root: root.clone(), plan: RefCell::new(None), checkpointed: Cell::new(false), changed: RefCell::new(Default::default()), stopped };
    let quantization = crate::sme::probe::latest("ollama", &turn.target.model).map(|profile| profile.key.quantization);

    turn.sink.send(EngineEvent::Delta(format!(
        "▸ NAVARATNA: this request is long, so it is done as steps - each a fresh short prompt, each judged by the project's own checks. ({} tokens a step)\n",
        nr::budget_for(turn.window, quantization.as_deref())
    )));

    let rank_root = root.clone();
    let plan = match nr::plan::build(turn.request, &mut engine, &|goal| crate::sme::retrieval::retrieve(&rank_root, goal, 6).into_iter().map(|hit| hit.path).collect()) {
        Ok(plan) => plan,
        Err(blocked) => {
            turn.sink.send(EngineEvent::Delta(format!("\nThe request could not be planned: {blocked}. Nothing was changed.\n")));
            turn.sink.send(EngineEvent::Done { summary: "Not planned".to_string(), meta: super::meta(0, engine.input.get(), engine.output.get(), turn.target.price), pass: Some(false) });

            return;
        }
    };

    let mut plan = plan;
    let rank = |goal: &str| -> Vec<String> { crate::sme::retrieval::retrieve(&rank_root, goal, 6).into_iter().map(|hit| hit.path).collect() };

    architect(turn, &mut plan, &rank, stopped);

    let _ = crate::navaratna::spec::store(&root, turn.request, &plan.map);

    turn.sink.send(EngineEvent::Delta(format!("▸ {} requirement(s) in {} step(s); every one of the request's {} parts is accounted for.\n", plan.map.requirements.len(), plan.steps.len(), plan.map.units.len())));

    let config = Config { budget_tokens: nr::budget_for(turn.window, quantization.as_deref()), ..Config::default() };
    let (report, registry) = nr::run(&plan, &mut engine, &mut project, &config);
    let requirements: Vec<String> = plan.map.requirements.keys().cloned().collect();

    turn.sink.send(EngineEvent::Delta(format!("\n{}", report.render(&registry, &requirements))));

    let blocked = report.blocked().len();

    turn.sink.send(EngineEvent::Done {
        summary: if blocked == 0 { "Done".to_string() } else { format!("{blocked} step(s) blocked") },
        meta: super::meta(engine.calls.get(), engine.input.get(), engine.output.get(), turn.target.price),
        pass: Some(blocked == 0),
    });
}

#[allow(dead_code)]
fn _dialect_is_used(_: Dialect) {}
