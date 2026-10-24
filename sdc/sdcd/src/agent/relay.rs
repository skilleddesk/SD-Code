//! **Silent Relay in the agent loop (M1-4, M1-5, M1-9, M1-10)**: the glue between the loop and `crate::rotation`.
//!
//! `rotation/` holds the rules (decision, capsule, executor, handover, ledger) as pure parts a test can drive. This file is the one place
//! that knows the loop: it reads the loop's state into [`StepSignals`], asks [`decide`], and when the answer is "relay" runs the chain model
//! on the **same agent loop** (`drive`) with a narrower tool list - so every edit goes through the Step Transaction, every model call through
//! the ModelGate as `Purpose::Chain`, and Stop, steering and permissions work because it is the same turn. When the step is done and the
//! project's checks are green the local model carries on, with a handover note written from the diff.
//!
//! Only when the gate says [`Mode::SilentRelay`](crate::guard::modelgate::Mode): a local model, a chain with a cloud row, a project that is not
//! `local-only`, and the person's consent. Everything else - a model picked by hand above all - never reaches this file's code.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::*;
use crate::rotation::capsule::{self, Prepared};
use crate::rotation::decision::{decide, Decision, RelayReason, StepSignals, Used};
use crate::rotation::executor::{self, Attempt, ChainModel, Verified};
use crate::rotation::handover;
use crate::rotation::ledger::{self, Label, TurnLedger};
use crate::rotation::policy::RelayPolicy;

/// The tools a chain model gets for one step (plan 3.5): look, search, change, and run the project's checks.
pub(super) const TOOLS: &[&str] = &["read_file", "list_dir", "grep", "glob", "search", "ranked_search", "code_graph", "edit_file", "write_file", "apply_patch", "run_command", "git_diff"];

/// The most model calls a relayed step may make (its tool-call limit).
const STEP_CALLS: usize = 12;

/// Local steps that must pass before a step that no chain model could do is relayed again.
const COOLDOWN: u32 = 3;

thread_local! {
    static SCOPE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// When the chain model's time on the step runs out (the watchdog, M1-9).
    static DEADLINE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Has the chain model used up its time on this step? `drive` asks it with the Stop button: either one ends the nested loop.
pub(super) fn timed_out() -> bool {
    DEADLINE.with(std::cell::Cell::get).is_some_and(|deadline| std::time::Instant::now() >= deadline)
}

/// Is the current thread inside a relayed step? `drive` reads it to narrow its tools and to ask the gate as the chain.
pub(super) fn in_scope() -> bool {
    SCOPE.with(std::cell::Cell::get)
}

struct Scope;

impl Scope {
    fn enter(limit: std::time::Duration) -> Self {
        SCOPE.with(|scope| scope.set(true));
        DEADLINE.with(|deadline| deadline.set(Some(std::time::Instant::now() + limit)));

        Scope
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        SCOPE.with(|scope| scope.set(false));
        DEADLINE.with(|deadline| deadline.set(None));
    }
}

/// What a nested loop reported while it ran.
#[derive(Default)]
struct Capture {
    input: u64,
    output: u64,
    cost: Option<f64>,
    finished: bool,
    failed: Option<String>,
    answer: String,
    edited: Vec<String>,
}

/// The loop's state a relay needs, borrowed for the moment of the relay.
pub(super) struct Env<'a> {
    pub workspace: &'a Workspace,
    pub options: Options,
    pub policy: &'a crate::trust::policy::Policy,
    pub prompt: &'a Prompt,
    pub sink: &'a EventSink,
    pub checkpoint: Option<tools::Checkpointer>,
    pub dialect: Dialect,
    pub goal: &'a str,
    pub changed: Vec<String>,
    pub passed_checks: &'a [String],
    pub error: &'a str,
    pub local_model: &'a str,
    pub call_timeout_s: u32,
    pub gate: &'a crate::guard::modelgate::Gate,
}

/// What became of a step the loop asked the relay about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handled {
    /// The signals did not call for the chain (or the limits, the cooldown or the person said no): the local model goes on.
    Local,
    /// The chain did the step; the local model has a handover note.
    Done,
    /// The chain was tried and could not do it; the local model has been told.
    Failed,
}

pub(super) struct Hook {
    pub policy: RelayPolicy,
    pub chain: Vec<ChainModel>,
    pub used: Used,
    pub ledger: TurnLedger,
    pub signals: StepSignals,
    root: std::path::PathBuf,
    prepared: Option<std::thread::JoinHandle<Prepared>>,
    ready: Option<Prepared>,
    relays: u32,
    cooldown: u32,
    /// Relayed steps in a row, with no local step between (`rotation.toml`'s `offload_limit`).
    streak: u32,
    offload_limit: u32,
}

impl Hook {
    /// The relay of this turn, or `None` when the turn is not in Silent Relay mode or has nowhere to relay to.
    pub(super) fn start(gate: &crate::guard::modelgate::Gate, rotation: &crate::data::rotation::Rotation, trust: &crate::trust::policy::Policy, workspace: &Workspace, request: &str, session_id: &str) -> Option<Self> {
        if !gate.relays() || workspace.is_remote() || !rotation.relay.enabled() {
            return None;
        }

        let chain = executor::chain_of(rotation, trust.privacy_local());

        if chain.is_empty() {
            return None;
        }

        let root = std::path::PathBuf::from(workspace.root());

        /* M1-10: the first chain model's connection is opened now, beside the local model's work - no model call, just the handshake. */
        crate::engines::native_api::warm(&chain[0].model, Some(&chain[0].provider));

        /* The half of the capsule that does not depend on the failure is built while the local model is still trying. */
        let prepared = capsule::spawn_prepare(root.clone(), request.to_string(), trust.clone(), session_id.to_string());

        Some(Self { policy: rotation.relay.clone(), chain, used: Used::default(), ledger: TurnLedger::default(), signals: StepSignals::default(), root, prepared: Some(prepared), ready: None, relays: 0, cooldown: 0, streak: 0, offload_limit: rotation.global.offload_limit })
    }

    /// The loop finished a step on the local model.
    pub(super) fn local_step(&mut self) {
        self.ledger.local_step();
        self.cooldown = self.cooldown.saturating_sub(1);
        self.streak = 0;
    }

    /// Reads one step's tool calls and answers into the signals.
    pub(super) fn observe(&mut self, calls: &[dialect::ToolUse], results: &[dialect::ToolResult], edits: usize, printed_refusals: usize, caution_level: u8, ttft_ms: u64) {
        self.signals.invalid_tool_calls = printed_refusals as u32;
        self.signals.ttft_ms = Some(ttft_ms);
        self.signals.looping = caution_level >= 3;
        self.signals.low_confidence = caution_level >= 2;

        for call in calls {
            let Some(result) = results.iter().find(|result| result.0 == call.id) else {
                continue;
            };

            let exit = crate::v3::lessons::exit_code(&result.1);

            if call.name == "run_command" && edits > 0 {
                match exit {
                    Some(0) if !result.2 => self.signals.verify_failures = 0,
                    Some(_) => self.signals.verify_failures += 1,
                    None if result.2 => self.signals.verify_failures += 1,
                    None => {}
                }
            }

            /* An edit the syntax gate refused is a failed check in the same sense. */
            if matches!(call.name.as_str(), "edit_file" | "write_file" | "apply_patch") && result.2 {
                self.signals.verify_failures += 1;
            }

            if call.name == "remember" && result.2 && result.1.contains("was not kept") {
                self.signals.quote_failures += 1;
            }
        }
    }

    pub(super) fn decision(&self) -> Decision {
        if self.cooldown > 0 || (self.offload_limit > 0 && self.streak >= self.offload_limit) {
            return Decision::StayLocal;
        }

        decide(&self.signals, &self.policy, &self.used)
    }

    /// Decides from the signals and acts: relays, asks the person once when the turn's allowance is used, or leaves the step to the local
    /// model. `ask` puts a question to the person and answers what they chose (`None`: they closed it, or the turn was stopped).
    pub(super) fn handle(&mut self, env: &Env, messages: &mut Vec<Value>, ask: &dyn Fn(&str, &[String]) -> Option<String>) -> Handled {
        match self.decision() {
            Decision::StayLocal => Handled::Local,
            Decision::Relay(reason) => self.run(env, messages, reason),
            Decision::AskUser(reason) => {
                self.used.asked = true;

                let question = format!("The local model is stuck ({}), and this turn has used its cloud allowance. Send more steps to your chain?", reason.sentence());

                match ask(&question, &["Yes, for this turn".to_string(), "No, stop here".to_string()]) {
                    Some(answer) if answer.starts_with("Yes") => {
                        self.used = Used { cloud_steps: 0, usd: 0.0, asked: true };

                        self.run(env, messages, reason)
                    }
                    _ => Handled::Local,
                }
            }
        }
    }

    fn run(&mut self, env: &Env, messages: &mut Vec<Value>, reason: RelayReason) -> Handled {
        if self.relay(env, messages, reason) {
            Handled::Done
        } else {
            Handled::Failed
        }
    }

    /// Waits for the prepared half of the capsule (normally long finished).
    fn prepared(&mut self) -> &Prepared {
        if let Some(handle) = self.prepared.take() {
            self.ready = Some(handle.join().unwrap_or_default());
        }

        self.ready.get_or_insert_with(Prepared::default)
    }

    /// Relays the step to the chain. Appends what the local model needs to carry on to `messages`; answers whether the step was done.
    fn relay(&mut self, env: &Env, messages: &mut Vec<Value>, reason: RelayReason) -> bool {
        let root = self.root.clone();
        let session = env.prompt.session_id.clone();
        let turn = env.prompt.turn_id.clone();
        let rel = |path: &str| -> String {
            let normal = path.replace('\\', "/");
            let base = format!("{}/", root.to_string_lossy().replace('\\', "/").trim_end_matches('/'));

            normal.strip_prefix(&base).unwrap_or(&normal).to_string()
        };
        let files: Vec<(String, String)> = env.changed.iter().take(3).filter_map(|path| std::fs::read_to_string(root.join(rel(path))).ok().map(|content| (rel(path), content))).collect();
        let prepared = self.prepared().clone();
        let done_when: Vec<String> = env.passed_checks.iter().map(|check| format!("`{check}` keeps passing")).collect();
        let built = capsule::finish(&prepared, &root, env.policy, &capsule::Step { session_id: &session, goal: env.goal, done_when: &done_when, error: env.error, notes: &[], files: &files }, capsule::DEFAULT_BUDGET_TOKENS);

        env.sink.send(EngineEvent::Thinking(format!("Relaying this step to the chain ({}): about {} tokens leave this computer, secrets taken out.\n", reason.sentence(), built.tokens)));

        let note_to_local = |text: String, messages: &mut Vec<Value>| dialect::append_user_text(env.dialect, messages, &text);

        /* What is sent is written down first: an audit may not lack a send that happened. */
        if let Err(error) = capsule::audit(&root, &self.chain[0].label(), &turn, &session, &built) {
            env.sink.send(EngineEvent::Thinking(format!("The relay was not sent: the audit log could not be written ({error}).\n")));
            self.cooldown = COOLDOWN;

            return false;
        }

        /* A way back before anything changes: a failed attempt's edits are put back from this. */
        let Ok(shadow) = crate::git::Shadow::of(&root) else {
            env.sink.send(EngineEvent::Thinking("The relay was not started: SDC cannot take a safety checkpoint here (git is still being installed).\n".to_string()));
            self.cooldown = COOLDOWN;

            return false;
        };
        let Ok(sha) = crate::git::checkpoint(&root, "sdcd: before a relayed step") else {
            env.sink.send(EngineEvent::Thinking("The relay was not started: the safety checkpoint failed.\n".to_string()));
            self.cooldown = COOLDOWN;

            return false;
        };

        let labels = std::cell::Cell::new(Label::Measured);
        let local_label = format!("ollama:{}", env.local_model);
        let relayed = executor::run(
            &self.chain,
            reason,
            env.gate,
            &mut |model, position| {
                let (attempt, label) = attempt(env, model, position, &built.text, built.tokens);

                labels.set(labels.get().weakest(label));

                attempt
            },
            &mut || verify(env.workspace, &root),
            &mut |paths| {
                for path in paths {
                    let _ = crate::git::restore_file(&root, &sha, &rel(path));
                }
            },
        );

        self.relays += 1;
        self.streak += 1;

        for record in &relayed.records {
            let step_id = format!("{turn}.r{}", self.relays);

            env.sink.send(EngineEvent::RelayStep(ledger::step_json(&session, &turn, &step_id, &local_label, record, labels.get())));
        }

        self.ledger.relayed(&relayed, labels.get());
        self.used.cloud_steps += 1;
        self.used.usd += relayed.cost_usd();
        self.signals = StepSignals::default();

        if !relayed.ok {
            self.cooldown = COOLDOWN;
            env.sink.send(EngineEvent::Thinking(format!("The chain could not do this step: {}.\n", relayed.failure)));
            note_to_local(format!("[SDC: this step was sent to the chain and could not be done: {}. Tell the person plainly what is blocking you and what you tried, or take a different approach.]", relayed.failure), messages);

            return false;
        }

        /* Back to the local model, with a note written from the diff. */
        let changes: Vec<(String, Vec<String>)> = relayed
            .edited
            .iter()
            .map(|path| {
                let path = rel(path);
                let before = shadow.run(&["show", &format!("{sha}:{path}")]).unwrap_or_default();
                let after = std::fs::read_to_string(root.join(&path)).unwrap_or_default();

                (path.clone(), handover::symbol_changes(&path, &before, &after))
            })
            .collect();
        let note = handover::note(&relayed.model, env.goal, &changes, &relayed.verified);

        env.sink.send(EngineEvent::Thinking(format!("{} did the step ({}). The local model carries on.\n", relayed.model, match &relayed.verified {
            Verified::Green(_) => "checks passed",
            Verified::NoChecks => "no checks to run in this folder",
            Verified::Red(_) => "checks failing",
        })));
        note_to_local(note, messages);

        true
    }

    /// Emits the turn's split once the turn is over (switch `relay.ledger`).
    pub(super) fn summary(&self, sink: &EventSink, session_id: &str, turn_id: &str) {
        if self.ledger.steps_total() == 0 && self.ledger.attempts == 0 || !crate::v3::on("relay.ledger") {
            return;
        }

        sink.send(EngineEvent::RelaySummary(self.ledger.summary_json(session_id, turn_id)));
    }
}

/// One call to a chain model: the nested loop on the same turn, its events forwarded, its numbers captured.
fn attempt(env: &Env, model: &ChainModel, _position: usize, capsule_text: &str, capsule_tokens: u64) -> (Attempt, Label) {
    let nested = Prompt { provider: Some(model.provider.clone()), model: model.model.clone(), text: capsule_text.to_string(), history: Vec::new(), images: Vec::new(), resume: None, ..env.prompt.clone() };
    let target = match resolve_target(&nested) {
        Ok(target) => target,
        Err(reason) => return (Attempt { failure: format!("{} cannot run as a relayed step ({reason})", model.label()), ..Default::default() }, Label::Estimate),
    };
    let shared = Arc::new(Mutex::new(Capture::default()));
    let capture = Arc::clone(&shared);
    let parent = env.sink.clone();
    let sink = EventSink::new(move |event| {
        let Ok(mut seen) = capture.lock() else {
            return;
        };

        match event {
            EngineEvent::Usage { input_tokens, output_tokens, cost_usd } => {
                seen.input = input_tokens;
                seen.output = output_tokens;

                if cost_usd.is_some() {
                    seen.cost = cost_usd;
                }
            }
            EngineEvent::Done { .. } => seen.finished = true,
            EngineEvent::Failed(reason) => seen.failed = Some(reason),
            EngineEvent::Delta(text) => {
                seen.answer.push_str(&text);
                parent.send(EngineEvent::Thinking(text));
            }
            EngineEvent::ToolStarted { ref tool, ref target, .. } => {
                if tool == "edit" && !target.is_empty() && !seen.edited.contains(target) {
                    seen.edited.push(target.clone());
                }

                parent.send(event);
            }
            other => parent.send(other),
        }
    });
    let switches = crate::v3::TurnSwitches::snapshot(&env.prompt.turn_id);
    let _scope = Scope::enter(std::time::Duration::from_secs(u64::from(env.call_timeout_s)));

    drive(Backend::Api, &target, env.workspace, Options { autonomy: env.options.autonomy, max_steps: STEP_CALLS, auto_check: true, research: None }, env.checkpoint.clone(), env.policy, &nested, &sink);

    let timed_out = timed_out();

    crate::v3::TurnSwitches::restore(&env.prompt.turn_id, switches);

    let seen = shared.lock().map(|seen| (seen.input, seen.output, seen.cost, seen.finished, seen.failed.clone(), seen.answer.clone(), seen.edited.clone())).unwrap_or_default();
    let (input, output, reported_cost, finished, failed, answer, edited) = seen;
    let reported = input + output > 0;
    let (input, output) = if reported { (input, output) } else { (capsule_tokens, crate::tokens::estimate(&answer)) };
    let priced = target.price.map(|(price_in, price_out)| input as f64 * price_in / 1e6 + output as f64 * price_out / 1e6);
    let cost = reported_cost.or(priced);
    let label = match (reported_cost.is_some(), reported, priced.is_some()) {
        (true, _, _) => Label::Measured,
        (false, true, true) => Label::Priced,
        _ => Label::Estimate,
    };

    (Attempt { finished: finished && failed.is_none() && !timed_out, answer, tokens_in: input, tokens_out: output, cost_usd: cost, failure: if timed_out { format!("it took longer than {} s", env.call_timeout_s) } else { failed.unwrap_or_default() }, edited }, label)
}

/// The project's own checks, after the chain model's work. Green is the only success.
fn verify(workspace: &Workspace, root: &std::path::Path) -> Verified {
    let posix = !cfg!(windows);
    let names: Vec<String> = workspace.list(".").map(|(entries, _)| entries.iter().map(|entry| entry.split(" (").next().unwrap_or(entry).trim_end_matches('/').to_string()).collect()).unwrap_or_default();
    let checks = crate::verify::plan(&names, &|name| workspace.read(name).ok().map(|(text, _)| text), posix);

    if checks.is_empty() {
        return Verified::NoChecks;
    }

    let mut passed = Vec::new();

    for check in checks.iter().take(3) {
        let line = crate::verify::non_interactive(&check.command, posix);
        let (line, _) = crate::verify::sandboxed_for_mcp(&line, &root.to_string_lossy());

        match workspace.run(&line, std::time::Duration::from_secs(180)) {
            Ok(report) if report.ok => passed.push(check.command.clone()),
            Ok(report) => {
                let text = format!("{}\n{}", report.stdout, report.stderr);

                return Verified::Red(text.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"));
            }
            Err(error) => return Verified::Red(error.message),
        }
    }

    Verified::Green(passed)
}
