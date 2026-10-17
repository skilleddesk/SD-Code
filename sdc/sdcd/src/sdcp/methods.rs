//! The request handlers - one arm per SDCP method (master spec section 5.2).
//!
//! A handler validates its params, appends the events the request implies, and answers with a
//! `Response`. It never mutates state directly: the event log *is* the state change, which is the
//! daemon's half of spec section 3.3.
//!
//! `engine.start` is the interesting one. It answers with a `turnId` immediately and then, in a task
//! of its own, runs the engine adapter and pushes the whole turn through the notifier -
//! `TurnStarted`, `ThinkingDelta`, the tool calls, a `CheckpointSaved` *before* the mutating tool
//! (principle P5) and the `TurnDelta` stream, **each event as the engine produced it**. That is the
//! acceptance item "a fake `engine.start` streams a few TurnDelta events to the UI store", and it is
//! also how the real CLIs behave: the call returns, the answer arrives later.
//!
//! Every module of the daemon is reached from here, which is the point of the file - it is the one
//! place where a protocol method is mapped onto a capability.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::duel;
use crate::engines::{EngineStatus, EventSink, Prompt};
use crate::errors::translator;
use crate::host;
use crate::providers;
use crate::sdcp::envelope::{Envelope, ErrorObject, Response};
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;
use crate::session_bridge;
use crate::store::sqlite::Store;
use crate::{DaemonState, SDCP_VERSION, VERSION};

/* The Trust Kernel's, the Intent Engine's and the agency's methods (0.12) - child modules, so they share
   this file's private helpers rather than copying them. */
#[path = "kernel.rs"]
mod kernel;
#[path = "intent_methods.rs"]
mod intent_methods;
#[path = "ops_methods.rs"]
mod ops_methods;
#[path = "agent_methods.rs"]
pub(crate) mod agent_methods;
#[path = "anywhere_methods.rs"]
mod anywhere_methods;

#[path = "host_methods.rs"]
mod host_methods;
#[path = "workspace_methods.rs"]
mod workspace_methods;
#[path = "file_methods.rs"]
mod file_methods;
#[path = "model_methods.rs"]
mod model_methods;
#[path = "guard_methods.rs"]
mod guard_methods;
#[path = "fleet_methods.rs"]
mod fleet_methods;

/// One connection's request handler. It holds the shared daemon state and nothing else, so a second
/// connection is a second `Daemon` over the same state rather than a second source of truth.
pub struct Daemon {
    state: Arc<DaemonState>,
}

impl Daemon {
    /// Takes the shared state by value: the handler is cheap to clone and a turn keeps one.
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    /// Dispatches one envelope. The `Notifier` is where every event the request implies goes; it is
    /// behind an `Arc` because a turn outlives the call that started it.
    pub fn handle(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Response {
        match self.dispatch(envelope, out) {
            Ok(result) => Response::ok(envelope.id.clone(), result),
            Err(error) => Response::fail(envelope.id.clone(), error),
        }
    }

    fn store(&self) -> &Arc<Store> {
        &self.state.store
    }

    fn dispatch(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        /* Agency Mode (0.12): the person at the desk has a role, and the daemon - not the window - refuses
           what the role may not do. With no team configured the one person is the owner. */
        let role = crate::ops::team::current_role(self.store());

        /* Switching who is at the desk (`team.set` with only `current`) is open to every role - it is what
           the refusal below tells a person to do, and a role that could not do it was stuck in it for
           good (0.16.1). The members and their roles stay the owner's to change. */
        let switching = envelope.method == "team.set" && envelope.params.get("members").is_none();

        if role != "owner" && !switching {
            crate::ops::team::allowed(&role, &envelope.method).map_err(ErrorObject::permission_denied)?;
        }

        match envelope.method.as_str() {
            "verify.run" => self.verify_run(envelope, out),

            /* The Trust Kernel, then the Intent Engine, then the agency methods - each falls through to the
               next, and the last one answers `unsupported`. */
            _ => self.dispatch_host(envelope, out),
        }
    }

    /// The SSH side of a host, when the daemon has an address for it: `None` for `local`, and `None`
    /// for a host row that was never added as an SSH target.
    pub(super) fn ssh_for(&self, host_id: &str) -> Result<Option<crate::ssh::Ssh>, ErrorObject> {
        if host_id == "local" {
            return Ok(None);
        }

        let Some((Some(target), port)) = self.store().host_address(host_id).map_err(ErrorObject::internal)? else {
            return Ok(None);
        };

        Ok(Some(crate::ssh::Ssh::new(crate::auth::remote::SshTarget { user_host: target, port })))
    }

    /* -----------------------------------------------------------------------------------------
     * Projects: the folder a chat works in (0.7.6)
     * -------------------------------------------------------------------------------------- */

    /* -----------------------------------------------------------------------------------------
     * Providers, permission, checkpoints, rewind, duel, console and the event replay
     * -------------------------------------------------------------------------------------- */

    /* -----------------------------------------------------------------------------------------
     * The engines' environment: files, git, processes
     * -------------------------------------------------------------------------------------- */

    /// How much of a file `fs.read` will put in one answer: 1 MiB.
    ///
    /// The window's file view reads through this, and a 200 MB log is not something to hand a WebView.
    /// What comes back says it was cut (`truncated: true`, and `bytes` is the file's real size), and the
    /// `sha256` is still of the **whole file** - a hash of the first megabyte would be a hash of
    /// something that is not the file.
    const MAX_READ_BYTES: usize = 1024 * 1024;

    /* -----------------------------------------------------------------------------------------
     * Duel, the console bridge, and the event replay
     * -------------------------------------------------------------------------------------- */

}

/* ------------------------------------------------------------------------------------------------
 * The turn task. It is outside the `impl` because it owns nothing but the plan and the notifier: the
 * engine runs here, the events go out from here, and the state it touches is reached through the
 * daemon state it was handed.
 * ---------------------------------------------------------------------------------------------- */

/// Everything a turn needs after the call has already answered.
#[derive(Default)]
struct RunPlan {
    session_id: String,
    turn_id: String,
    engine_id: String,
    prompt_text: String,
    /// The model the turn runs on, resolved by `engine_start` from the request, the session or the
    /// registry. It travels to the engine inside the `Prompt`, because the engine cannot guess it -
    /// see the note on `Prompt::model`.
    model: String,
    /// The provider the model belongs to, when the caller said. `native_api` needs it to find the
    /// endpoint (and therefore the key entry) of a model id this build's catalogue has never seen.
    provider: Option<String>,
    history: Vec<crate::engines::Message>,
    /// The folder the chat works in, or `None` for a chat that has no project. It reaches the adapters
    /// inside the `Prompt`, which is where every fact about the session that the engine cannot guess
    /// travels - the same reason the model and the provider are there.
    project_root: Option<String>,
    /// The host that folder is on, when it is not this machine (0.7.13). It travels the same way and for
    /// the same reason: `cli.rs` uses it to run the CLI there instead of here.
    remote: Option<crate::ssh::Ssh>,
    /// The engine takes its own checkpoint before its first change (the SDC Agent), so the loop below
    /// must not take a second, late one when it sees the ToolStarted.
    self_checkpointing: bool,
    /// The autonomy the person chose, for the engines that run their own agent (the CLIs).
    autonomy: crate::agent::gate::Autonomy,
    /// The folder's policy, loaded when the turn started (0.12, the Trust Kernel).
    policy: crate::trust::policy::Policy,
    /// The log's sequence number just before the turn, so its score reads only its own events.
    first_seq: i64,
    /// What the turn was estimated to cost before it ran - kept to compare with what it did cost.
    estimate_usd: Option<f64>,
    /// The CLI conversation this turn continues (0.13).
    resume: Option<crate::engines::ResumeRef>,
    /// The images attached to the turn, saved where the engine can open them (0.13).
    images: Vec<crate::engines::Attachment>,
    /// A `/compact` turn: its answer becomes the chat's summary (0.13).
    compact: bool,
    /// `local` or the host - where a CLI conversation this turn starts lives.
    place: String,
    /// How hard the model should think (0.14).
    effort: Option<String>,
}

/// How long streamed text is gathered before it is pushed as one delta (0.11.7).
const DELTA_WINDOW: std::time::Duration = std::time::Duration::from_millis(50);

/// The gathering window on a slow link (0.12, Settings → Low-bandwidth mode): five times fewer pushes.
const LOW_BANDWIDTH_WINDOW: std::time::Duration = std::time::Duration::from_millis(250);

/// Streamed text that has not been pushed yet: at most one of the two is non-empty at a time.
struct Pending {
    answer: String,
    thinking: String,
    since: Option<std::time::Instant>,
    window: std::time::Duration,
}

impl Default for Pending {
    fn default() -> Self {
        Self { answer: String::new(), thinking: String::new(), since: None, window: DELTA_WINDOW }
    }
}

impl Pending {
    /// Old enough, or big enough, to go now.
    fn due(&self) -> bool {
        self.since.is_some_and(|since| since.elapsed() >= self.window) || self.answer.len() + self.thinking.len() >= 4096
    }

    fn flush(&mut self, out: &dyn Notifier, turn_id: &str, session: &Option<String>, turn: &Option<String>) {
        if !self.thinking.is_empty() {
            out.push(event::thinking_delta(turn_id, &std::mem::take(&mut self.thinking)), session.clone(), turn.clone());
        }

        if !self.answer.is_empty() {
            out.push(event::turn_delta(turn_id, &std::mem::take(&mut self.answer)), session.clone(), turn.clone());
        }

        self.since = None;
    }
}

/// Runs one turn and pushes its events - including the checkpoint that must exist *before* a mutating
/// tool runs (principle P5).
///
/// The engine's stream is *followed*, not awaited: each item is pushed the moment it arrives, which is
/// what makes the turn stream live (see the note on the channel below).
async fn run_turn(
    state: Arc<DaemonState>,
    engine: Arc<dyn crate::engines::Engine>,
    plan: RunPlan,
    out: Arc<dyn Notifier>,
) {
    let session = Some(plan.session_id.clone());
    let turn = Some(plan.turn_id.clone());
    let prompt = Prompt {
        session_id: plan.session_id.clone(),
        turn_id: plan.turn_id.clone(),
        text: plan.prompt_text.clone(),
        model: plan.model.clone(),
        provider: plan.provider.clone(),
        history: plan.history.clone(),
        project_root: plan.project_root.clone(),
        remote: plan.remote.clone(),
        autonomy: plan.autonomy,
        resume: plan.resume.clone(),
        images: plan.images.clone(),
        effort: plan.effort.clone(),
    };
    let mut answer = String::new();
    let mut checkpoint_written = false;
    /* What the turn did with its tools, one line per call - kept with the answer for the next turn. */
    let mut tools: Vec<(String, String)> = Vec::new();
    /* The Trust Kernel's view of the turn as it runs (0.12): what it cost so far, the files it changed,
       the shapes of a runaway, and whether the kernel already stopped it. */
    let mut usage = crate::trust::cost::Usage::default();
    let mut changed: Vec<String> = Vec::new();
    let mut runaway = crate::trust::cost::Runaway::default();
    let mut halted = false;
    /* The CLI's conversation id, when it said one (0.13): the next turn resumes it. */
    let mut conversation: Option<String> = None;
    let mut completed: Option<Value> = None;
    /* Why the turn ended early, when it did - told to the next turn so it continues instead of starting over. */
    let mut unfinished: Option<String> = None;
    let turn_cap = crate::trust::cost::turn_cap(&state.store, plan.policy.max_turn_usd);
    let stopper = engine.clone();

    crate::trust::kill::begin(&plan.turn_id, "turn", Some(&plan.session_id), &plan.prompt_text.chars().take(80).collect::<String>());

    /* Counted as running until this function returns, on any road out - a `--idle-exit` daemon does not
       leave in the middle of a turn because the window was closed (`lib.rs::watch`, 2.0.5). */
    let _turn = state.turn_guard();

    /* The engine runs on its own task and writes into a channel; this loop reads it and pushes each
      event out while the engine is still talking.
    *
    * `let events = engine.start(prompt).await;` is what used to be here, and it is the whole defect
    * the report *"akbare answare disse"* names: the adapter's stream was complete before the first
    * notification left this function, so a two-minute turn arrived as one lump - thinking, tool
    * calls and answer together - with the window looking idle until the end.
    *
    * The channel keeps the two properties the checkpoint rule needs: **order** (one producer, one
    * consumer, so a `ToolCallStarted` cannot overtake the `CheckpointSaved` written for it below) and
    * **a single place that talks to the notifier** (this loop, never the engine). */
    let (sink, mut stream) = EventSink::channel();
    let running = tokio::spawn(async move {
        engine.start(prompt, &sink).await;

        /* `sink` is dropped here, which is what ends the loop below: no separate "done" signal can be
        lost on the way. */
    });

    /*
     * Deltas are gathered for up to [`DELTA_WINDOW`] and pushed as one (0.11.7).
     *
     * An engine streams a token - often a single Bengali grapheme - at a time, and each one used to be
     * its own event: one turn in the report's log was 14,000 `TurnDelta`s, every one written to the
     * database, sent over the bridge and folded by the window, which re-drew the whole answer each time.
     * Fifty milliseconds is below what reads as a pause, so the stream looks exactly as live, at a
     * fraction of the events. Order is kept: the other kind of delta, and every non-delta event, flushes
     * what is pending first.
     */
    let low_bandwidth = matches!(state.store.setting("net.lowBandwidth").ok().flatten().as_deref(), Some("true" | "on"));
    let mut pending = Pending { window: if low_bandwidth { LOW_BANDWIDTH_WINDOW } else { DELTA_WINDOW }, ..Pending::default() };
    let mut first_thought = true;

    loop {
        let next = match pending.since {
            None => stream.recv().await,
            Some(since) => match tokio::time::timeout(pending.window.saturating_sub(since.elapsed()), stream.recv()).await {
                Ok(next) => next,
                Err(_) => {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                    continue;
                }
            },
        };

        let Some(event) = next else {
            break;
        };

        /* A stopped turn is over as far as the window is concerned: `engine.cancel` already pushed its
           `TurnCompleted`, and anything the engine still says would reopen it. The stream is still
           drained, so the engine task can finish and be joined below. */
        if crate::engines::cancel::requested(&plan.turn_id) {
            pending = Pending::default();
            continue;
        }

        match &event {
            crate::engines::EngineEvent::Delta(_) => {
                if !pending.thinking.is_empty() {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                }
            }
            crate::engines::EngineEvent::Thinking(_) => {
                if !pending.answer.is_empty() {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                }
            }
            _ => pending.flush(&*out, &plan.turn_id, &session, &turn),
        }

        match event {
            crate::engines::EngineEvent::Delta(delta) => {
                /* 0.21: the first words of the answer go out the moment they land - gathering is for the
                   flow that follows, not for the wait before anything shows. */
                let first = answer.is_empty();

                answer.push_str(&delta);
                pending.answer.push_str(&delta);
                pending.since.get_or_insert_with(std::time::Instant::now);

                if first || pending.due() {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                }
            }
            /* A slow link keeps the answer and drops the running commentary. */
            crate::engines::EngineEvent::Thinking(_) if low_bandwidth => {}
            crate::engines::EngineEvent::Thinking(text) => {
                let first = std::mem::replace(&mut first_thought, false);

                pending.thinking.push_str(&text);
                pending.since.get_or_insert_with(std::time::Instant::now);

                if first || pending.due() {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                }
            }
            crate::engines::EngineEvent::Usage { input_tokens, output_tokens, cost_usd } => {
                usage.merge(crate::trust::cost::Usage { input_tokens, output_tokens, cost_usd });

                /* A turn that crosses its budget while running is stopped here, whichever engine runs it. */
                if let Some(cap) = turn_cap {
                    let (spent, _) = crate::trust::cost::settle(&plan.engine_id, plan.provider.as_deref(), &plan.model, &usage);

                    if spent > cap && !halted {
                        halted = true;
                        halt(
                            &*out,
                            &stopper,
                            &plan,
                            "BudgetStop",
                            "budget",
                            &format!("Stopped at ${spent:.2}: this turn crossed its ${cap:.2} budget. The work so far is kept; raise the budget or continue with a cheaper model."),
                        );
                    }
                }
            }
            /* One card per call: an engine that names the same call twice must not open a second card or
               count twice toward the loop guard. */
            crate::engines::EngineEvent::ToolStarted { call_id, .. } if tools.iter().any(|(id, _)| *id == call_id) => {}
            crate::engines::EngineEvent::ToolStarted { call_id, tool, name, target } => {
                /* The kernel looks first (0.12). A runaway loop stops whichever engine runs it; for the
                   engines that run their own agent (the CLIs) a protected path, a denied command or a
                   turn past its blast radius is stopped the moment the stream shows it - the SDC Agent
                   asks *before* acting, in its own tools, so it is not stopped here. */
                if !halted {
                    if let Some(reason) = runaway.observe(&name, &target) {
                        halted = true;
                        halt(&*out, &stopper, &plan, "BudgetStop", "runaway", &reason);
                    }
                }

                if tool == "edit" && !target.is_empty() && !changed.contains(&target) {
                    changed.push(target.clone());
                }

                if !halted && !plan.self_checkpointing {
                    let root = plan.project_root.as_deref();
                    let verdict = match tool.as_str() {
                        "edit" => plan
                            .policy
                            .protected(&target, root)
                            .map(|pattern| ("protected-path", format!("`{target}` is protected by the policy (`{pattern}`). The turn was stopped before it could go further; the checkpoint before it can put the file back.")))
                            .or_else(|| {
                                (plan.policy.max_files_per_turn > 0 && changed.len() > plan.policy.max_files_per_turn).then(|| {
                                    ("blast-radius", format!("This turn changed {} files, over the policy's limit of {}. It was stopped so you can look at the change before it grows.", changed.len(), plan.policy.max_files_per_turn))
                                })
                            }),
                        "run" => plan
                            .policy
                            .denies(&target)
                            .map(|rule| ("denied-command", format!("`{target}` matches `{rule}`, which the policy denies. The turn was stopped.")) ),
                        _ => None,
                    };

                    if let Some((rule, sentence)) = verdict {
                        halted = true;
                        out.push(
                            event::policy_violation(&plan.session_id, Some(&plan.turn_id), rule, &target, "stopped", &sentence),
                            session.clone(),
                            turn.clone(),
                        );
                        halt(&*out, &stopper, &plan, "", rule, &sentence);
                    }
                }

                /* A command that reaches past the folder cannot be undone by a rewind: the newest checkpoint
                   says so, so the Time Machine never promises more than it can give (TM-6). */
                if tool == "run" && (crate::agent::gate::looks_dangerous(&target) || plan.policy.always_asks(&target).is_some()) {
                    if let Ok(Some(id)) = state.store.mark_irreversible(&plan.session_id, &format!("ran `{}`", target.chars().take(80).collect::<String>())) {
                        if let Ok(Some(row)) = state.store.checkpoint(&id) {
                            out.push(event::checkpoint_updated(&plan.session_id, row), session.clone(), turn.clone());
                        }
                    }
                }

                if !plan.self_checkpointing && !checkpoint_written && ["edit", "write", "delete", "run"].contains(&tool.as_str()) {
                    let ordinal = state.events.seq();
                    /* The chat's own folder, so a checkpoint written before a mutating tool hashes the
                       files that tool is about to touch. Until 0.7.6 this passed `None`, which meant the
                       checkpoint recorded a conversation and no files at all - and since 0.7.13 the folder
                       may be on a **host**, where the hash is the host's own shadow commit. */
                    let snapshot = match (&plan.remote, plan.project_root.as_deref()) {
                        (Some(ssh), Some(root)) => crate::checkpoints::Snapshot::Remote(ssh, root),
                        (None, Some(root)) => crate::checkpoints::Snapshot::Local(std::path::Path::new(root)),
                        _ => crate::checkpoints::Snapshot::Unbound,
                    };

                    if let Ok(fresh) = crate::checkpoints::create(
                        &state.store,
                        &plan.session_id,
                        ordinal,
                        &format!("Before {name} {target}"),
                        snapshot,
                        crate::checkpoints::screenshot::capture(),
                    ) {
                        let _ = state.store.set_checkpoint_turn(&fresh.id, &plan.turn_id);

                        out.push(
                            event::checkpoint_saved(&plan.session_id, fresh.to_event_payload()),
                            session.clone(),
                            turn.clone(),
                        );
                    }

                    checkpoint_written = true;
                }

                tools.push((call_id.clone(), format!("{name} {target}")));

                out.push(
                    event::tool_call_started(&plan.turn_id, &call_id, &tool, &name, &target),
                    session.clone(),
                    turn.clone(),
                );
            }
            /* A draft is the live view of a call still being written (0.14.2): not once its card is open,
               and not on a slow link, where it is commentary. */
            crate::engines::EngineEvent::ToolDraft { .. } if low_bandwidth => {}
            crate::engines::EngineEvent::ToolDraft { call_id, .. } if tools.iter().any(|(id, _)| *id == call_id) => {}
            crate::engines::EngineEvent::ToolDraft { call_id, name, target, chars, preview } => {
                out.push_live(
                    event::tool_call_drafting(&plan.turn_id, &call_id, &name, &target, chars, &preview),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::ToolOutput { call_id, level, text } => {
                out.push(
                    event::tool_call_output(&plan.turn_id, &call_id, &level, &text),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::ToolCompleted { call_id, status, meta, diff } => {
                if status == "done" && diff.as_ref().is_some_and(|diff| diff.as_array().is_none_or(|lines| !lines.is_empty())) {
                    runaway.progress();
                }

                if let Some(line) = tools.iter_mut().find(|(id, _)| *id == call_id) {
                    line.1 = format!("{} - {}", line.1, if meta.is_empty() { status.clone() } else { meta.clone() });
                }

                out.push(
                    event::tool_call_completed(&plan.turn_id, &call_id, &status, &meta, diff),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Permission { permission_id, title, sub, action, target, risk, explain } => {
                /* The agent is blocked on this question until `permission.resolve` answers it (agent::gate),
                   so from here the turn is the person's to move: the chat says so rather than looking busy. */
                note_waiting(&state.store, &plan.session_id, Some("awaiting_approval"), &*out);
                card_sessions()
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .insert(permission_id.clone(), plan.session_id.clone());

                out.push(
                    event::permission_requested(json!({
                        "permissionId": permission_id,
                        "sessionId": plan.session_id,
                        "turnId": plan.turn_id,
                        "title": title,
                        "sub": sub,
                        "action": action,
                        "target": target,
                        "risk": risk,
                        "explain": explain,
                        "checkpointId": Value::Null,
                    })),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Steered(text) => {
                out.push(event::turn_steered(&plan.turn_id, &text), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::SessionRef(id) => {
                /* Kept the moment the CLI names it, not when the turn ends: the next message can be sent
                   the instant this one completes (a queued prompt is), and it must find the record. */
                if !plan.compact {
                    let turns = crate::context::live_turns(&state.store, &plan.session_id, "").into_iter().map(|turn| turn.id).collect();

                    crate::context::save_resume(
                        &state.store,
                        &plan.session_id,
                        &plan.engine_id,
                        &crate::context::ResumeRecord { id: id.clone(), place: plan.place.clone(), root: plan.project_root.clone().unwrap_or_default(), turns },
                    );
                }

                conversation = Some(id);
            }
            crate::engines::EngineEvent::Question { question_id, question, options } => {
                /* `ask_user` waits on the person exactly as a permission card does. */
                note_waiting(&state.store, &plan.session_id, Some("awaiting_approval"), &*out);

                out.push(
                    event::question_asked(&plan.session_id, &plan.turn_id, &question_id, &question, &options),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Context { used_tokens, window_tokens, compacted } => {
                out.push(
                    event::context_updated(&plan.session_id, &plan.turn_id, used_tokens, window_tokens, compacted, false),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Sources(sources) => {
                out.push(event::research_sources(&plan.turn_id, &plan.session_id, sources), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::Plan(steps) => {
                /* The plan outlives the turn and the daemon (long-task memory, 0.12): the next agent turn in
                   this chat is told where it left off. */
                let _ = state.store.set_setting(&format!("plan.{}", plan.session_id), &steps.to_string());

                out.push(event::plan_updated(&plan.turn_id, steps), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::Failed(reason) => {
                unfinished = Some(reason.clone());

                /* 1.0: a key the provider refused is a card that is NOT connected, whatever the keychain
                   holds - the Hub said "connected" over models that all failed. Saving a key again clears it. */
                if let Some(provider) = plan.provider.as_deref().filter(|_| crate::providers::refused_key(&reason)) {
                    if crate::providers::mark_refused(&state.store, provider, &reason) {
                        out.push(
                            event::provider_status(json!({ "id": provider, "status": "needs-auth", "detail": "The provider refused this key - paste it again" })),
                            None,
                            None,
                        );
                    }
                }

                /* Every failure goes through the translator, so the card always has a sentence -
                   `ErrorRaised` carries the title and the explanation, never a raw stack. The
                   `event::error_raised` constructor supplies the catalogue's `type` field. */
                let translated = translator::translate(&plan.engine_id, &reason);

                out.push(
                    event::error_raised(
                        &plan.session_id,
                        Some(&plan.turn_id),
                        &translated.title,
                        &translated.explanation,
                        Some(&plan.engine_id),
                    ),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Done { summary, meta, pass } => {
                if summary.starts_with("Cut off") || summary.starts_with("Paused") {
                    unfinished = Some(summary.clone());
                }
                /* Pushed after the answer is stored (0.13, below): the window sends a queued prompt the
                   moment a turn completes, and that turn must already see this one's answer. */
                completed = Some(event::turn_completed(&plan.turn_id, &summary, &meta, pass));
            }
        }
    }

    pending.flush(&*out, &plan.turn_id, &session, &turn);

    /* The task is joined so the turn's own bookkeeping cannot race it: a `finish_turn` before the
    engine's last event was pushed would be a stored answer that is missing its tail. */
    let _ = running.await;

    let interrupted = crate::engines::cancel::requested(&plan.turn_id);
    /* 0.15.4: a turn that failed after it had written something used to be stored as a success. */
    let failed = answer.is_empty() || (unfinished.is_some() && completed.is_none());
    let state_name = if halted { "error" } else if interrupted { "idle" } else if failed { "error" } else { "success" };
    let summary = if halted { "Stopped by SDC" } else if interrupted { "Interrupted" } else { "Done" };

    crate::engines::cancel::clear(&plan.turn_id);
    crate::trust::kill::end(&plan.turn_id);

    /*
     * The stored answer is what the *next* turn is told this one said (session_bridge::history_for), and
     * a turn is more than its last paragraph: without its tool calls the next turn read "I ran the tests,
     * 5 passed" with no trace of a test run, and a model decided it had invented the result and apologised
     * for work it had really done. The window draws the turn from its events, so this record is for the
     * conversation, not for the screen.
     */
    let mut recorded = if tools.is_empty() {
        answer.clone()
    } else {
        let lines: Vec<String> = tools.iter().map(|(_, line)| format!("- {line}")).collect();

        format!("{answer}\n\n[Tool calls in this turn, as SDC recorded them:\n{}]", lines.join("\n"))
    };

    /*
     * 0.15.4: a turn that stopped part-way says so. The person stops a turn (or a provider drops it) and
     * sends "continue" to the same or another model; without this line the next model read a turn that
     * looked finished, and either said the work was done or began the whole task again from step one.
     */
    let why = if halted {
        Some("SDC's Trust Kernel stopped it".to_string())
    } else if interrupted {
        Some("the person stopped it".to_string())
    } else {
        unfinished.as_ref().map(|reason| reason.chars().take(300).collect())
    };

    if let Some(why) = why {
        recorded.push_str(&format!(
            "\n\n[SDC: this turn did NOT finish - {why}. Everything listed above really happened and is on disk. \
             When asked to continue, pick up from the last step above: do not redo finished steps, and do not start the task again.]"
        ));
    }

    let _ = state.store.finish_turn(&plan.turn_id, &recorded, summary, state_name);

    /* The turn is over, so the chat is nobody's to answer for any more. `permission.resolve` clears the mark
       the moment a card is answered; this is the road for a turn that ended past an unanswered one - stopped,
       failed, or the window quit - which otherwise kept an amber `!` on a chat with nothing behind it. */
    note_waiting(&state.store, &plan.session_id, None, &*out);

    /*
     * The chat's memory after the turn (0.13). A `/compact` answer becomes the summary every later turn
     * starts from, and the CLI conversations end with it - that is the point of compacting. Otherwise a
     * CLI that named its conversation is resumed next time, holding every live turn of the chat so far.
     */
    if plan.compact {
        if state_name == "success" && !answer.trim().is_empty() {
            crate::context::save_compaction(&state.store, &plan.session_id, &plan.turn_id, &answer);
            crate::context::forget_resume(&state.store, &plan.session_id);
        }
    } else if conversation.is_some() {
        /* The record written when the CLI named its conversation already holds this turn. */
    }

    if let Some(done) = completed {
        out.push(done, session.clone(), turn.clone());
    }


    /* The cost governor's record (0.12): measured when the provider said, priced from the catalogue when it
       sent only tokens, and never a made-up number. "Saved" exists only against a baseline and measured tokens. */
    {
        let (cost, source) = crate::trust::cost::settle(&plan.engine_id, plan.provider.as_deref(), &plan.model, &usage);
        let baseline = crate::trust::cost::baseline(&state.store, &usage);
        let site = plan
            .project_root
            .as_deref()
            .and_then(|root| state.store.sites().ok()?.into_iter().find(|site| site["root"].as_str() == Some(root)))
            .and_then(|site| site["id"].as_str().map(str::to_string));

        let _ = state.store.record_usage(
            &plan.turn_id,
            &plan.session_id,
            plan.project_root.as_deref(),
            site.as_deref(),
            &plan.engine_id,
            &plan.model,
            plan.provider.as_deref(),
            usage.input_tokens,
            usage.output_tokens,
            cost,
            source,
            plan.estimate_usd,
            baseline,
        );

        out.push(
            event::cost_updated(
                &plan.session_id,
                &plan.turn_id,
                &plan.engine_id,
                &plan.model,
                usage.input_tokens,
                usage.output_tokens,
                cost,
                source,
                plan.estimate_usd,
                baseline.map(|base| (base - cost).max(0.0)),
            ),
            session.clone(),
            turn.clone(),
        );
    }

    /* The turn's Trust score, from its own events - recomputed when Verify runs for it. */
    score_turn(&state, &*out, &plan.session_id, &plan.turn_id, plan.first_seq, &plan.policy, plan.project_root.as_deref());

    /* 1.0: the turn's streamed pieces become one event per run, so opening SDC replays a short log. */
    state.events.compact_turn(&plan.turn_id);

    let _ = state.store.update_session(&plan.session_id, None, Some(state_name), None, Some(0), None);

    out.push(
        event::session_updated(json!({
            "sessionId": plan.session_id,
            "state": state_name,
            "minutesAgo": 0,
        })),
        session,
        None,
    );
}

/// Says what the chat is doing when the doing is *waiting on the person*: a permission card, an `ask_user`
/// question. Master spec §12 has such a chat float to the top of its host with an amber `!`; without it the
/// window drew it exactly like a working chat, which is what "SDC is stuck for hours" turned out to be -
/// one stored turn spent 5 h 20 m on a single card while its chat still said `running`.
///
/// `None` is the same road back: the person answered, or the turn ended without them.
fn note_waiting(store: &crate::store::Store, session_id: &str, reason: Option<&str>, out: &dyn Notifier) {
    let changed = match reason {
        Some(reason) => store.update_session(session_id, None, None, Some(reason), None, None).is_ok(),
        None => store.clear_attention(session_id).unwrap_or(false),
    };

    /* Nothing marked and nothing to take off: say nothing. A turn that never waited on the person - which is
       most of them - must not push an extra event for it. */
    if changed {
        out.push(event::session_updated(json!({ "sessionId": session_id, "attention": reason })), None, None);
    }
}

/// Which chat each permission card belongs to. `permission.resolve` names a card and nothing else, and the
/// amber `!` has to come off the chat the moment the card is answered - not minutes later, when the turn the
/// answer set going finally ends.
fn card_sessions() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static CARDS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();

    CARDS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// The chat a card was asked in, once - the answer decides it, so the note does not outlive its use.
fn take_card_session(permission_id: &str) -> Option<String> {
    card_sessions()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(permission_id)
}

/// The kernel stops a running turn (0.12): the cancel mark first, so the loop drops whatever the engine
/// still says; the engine told to stop; and the turn closed with the kernel's own sentence, so the window
/// never shows a turn that was stopped as one that finished.
fn halt(out: &dyn Notifier, engine: &Arc<dyn crate::engines::Engine>, plan: &RunPlan, event_kind: &str, kind: &str, sentence: &str) {
    crate::engines::cancel::request(&plan.turn_id);

    let engine = engine.clone();
    let turn_id = plan.turn_id.clone();

    tokio::spawn(async move {
        engine.cancel(&turn_id).await;
    });

    if event_kind == "BudgetStop" {
        out.push(
            event::budget_stop(&plan.session_id, &plan.turn_id, kind, sentence),
            Some(plan.session_id.clone()),
            Some(plan.turn_id.clone()),
        );
    }

    out.push(
        event::turn_completed(&plan.turn_id, "Stopped by SDC", sentence, Some(false)),
        Some(plan.session_id.clone()),
        Some(plan.turn_id.clone()),
    );
}

/// Scores one turn from its own events and pushes `TrustScored`. Called when the turn ends and again when
/// a Verify run for it finishes, which is what lifts an unverified turn's cap.
pub(crate) fn score_turn(
    state: &Arc<DaemonState>,
    out: &dyn Notifier,
    session_id: &str,
    turn_id: &str,
    first_seq: i64,
    policy: &crate::trust::policy::Policy,
    root: Option<&str>,
) {
    let events = state.events.since(first_seq.max(0));
    let facts = crate::trust::score::facts(&events, turn_id, policy, root);
    let (score, level, reasons) = crate::trust::score::turn(&facts, policy.max_files_per_turn);
    let reasons = json!(reasons);

    let _ = state.store.save_trust_score(turn_id, session_id, score, level, &reasons);

    out.push(
        event::trust_scored(session_id, turn_id, score, level, reasons),
        Some(session_id.to_string()),
        Some(turn_id.to_string()),
    );
}

/* ----------------------------------------------------------------------------------------------------
 * Whether a host can actually be reached
 *
 * `host.add` used to answer `connecting` and leave it there, which is why adding a server felt like
 * nothing happened: the row appeared, the dot stayed blue, and the same host could be added again
 * and again until the sidebar was four copies of the same name. The two functions below turn that
 * into a measurement with a sentence attached.
 * -------------------------------------------------------------------------------------------------- */

/// One checkpoint's or one rewind's subject: the host (when the folder is not here) and the folder.
///
/// It exists because `checkpoints::Snapshot` borrows what it describes and a method handler owns both
/// halves for only part of its body - see `Daemon::subject`.
struct Subject {
    remote: Option<crate::ssh::Ssh>,
    root: Option<std::path::PathBuf>,
    /// The same root as a string, for the remote case (a host's path is a `String`, not a `PathBuf`).
    root_text: String,
}

impl Subject {
    /// The borrowed view the checkpoint and rewind modules take.
    fn snapshot(&self) -> crate::checkpoints::Snapshot<'_> {
        match (&self.remote, &self.root) {
            (Some(ssh), Some(_)) => crate::checkpoints::Snapshot::Remote(ssh, &self.root_text),
            (None, Some(root)) => crate::checkpoints::Snapshot::Local(root),
            _ => crate::checkpoints::Snapshot::Unbound,
        }
    }
}

/// The two steps a host still needs once its key is trusted: the one-time key install, and the probe.
///
/// Shared by `host.add` (a host whose key was already pinned) and `host.trust` (a host a person has
/// just decided about), because the two differ in *when* they get here and in nothing else.
///
/// The password is spent here and only here, and this function is only ever reached with a pinned key:
/// the probe is `ssh` with `BatchMode=yes`, which by design cannot answer a prompt, so the install is
/// what makes every later connection passwordless. `install_key` runs on the daemon's PTY (the one
/// terminal it has) and types the password into `ssh`'s own prompt.
/// What a person typed to sign in to a host: used once by [`finish_connection`], kept nowhere - unless
/// they ticked Remember (0.14.4), and then only the password, only in the OS keychain, and only after
/// the host accepted it.
struct Secrets {
    password: String,
    /// The verification code, for a host that asks for one (0.8.1). Never kept.
    code: String,
    /// The keychain entry the password goes to once the sign-in succeeds.
    keep: Option<String>,
    /// "Stay signed in" (0.16.0).
    stay: Stay,
}

/// "Stay signed in" on the Sign in card (0.16.0): keep the password and the authenticator's key in the
/// OS keychain, so a dropped connection - or a restarted SDC - is signed in again without a person.
#[derive(Clone, Debug, PartialEq)]
enum Stay {
    /// The card did not say; what is kept stays kept.
    Unchanged,
    /// Ticked off: forget the key.
    Off,
    /// Ticked on, with the setup key typed (empty: SDC reads it from `~/.google_authenticator`).
    On(String),
}

impl Stay {
    fn from_envelope(envelope: &Envelope) -> Self {
        match envelope.params.get("staySignedIn") {
            Some(Value::Bool(true)) => Stay::On(envelope.opt_str("totpSecret").unwrap_or_default().trim().to_string()),
            Some(Value::Bool(false)) => Stay::Off,
            _ => Stay::Unchanged,
        }
    }
}

/// The keychain entry a host's remembered password lives in, keyed by `user@host:port` rather than by
/// the host's id, which changes when the host is added again.
fn host_secret_ref(ssh: &crate::ssh::Ssh) -> String {
    format!("sdc.host.{}", ssh.label())
}

async fn finish_connection(
    state: Arc<DaemonState>,
    notifier: Arc<dyn Notifier>,
    host_id: String,
    name: String,
    ssh: crate::ssh::Ssh,
    secrets: Secrets,
    key_note: Option<String>,
) {
    let Secrets { password, code, keep, stay } = secrets;
    let target = ssh.target.user_host.clone();

    if let Some(note) = &key_note {
        notifier.push(
            event::host_status(&host_id, &name, "vps", "connecting", None, Some(note), None),
            None,
            None,
        );
    }

    /*
     * Sign in first (0.8.1): one authenticated connection, kept open, that every later call goes through.
     * This is the only way into a host that has public-key login switched off and asks for a
     * verification code and a password on every session - the host in the report. A machine without an
     * `ssh` that can hold a connection open falls back to the 0.7.13 key install below.
     */
    let mut install_key = !password.is_empty();

    if !password.is_empty() || !code.is_empty() {
        notifier.push(
            event::host_status(
                &host_id,
                &name,
                "vps",
                "connecting",
                None,
                Some(&format!("signing in to {}…", ssh.label())),
                None,
            ),
            None,
            None,
        );

        let signing = ssh.clone();
        let (secret, one_time) = (password.clone(), code.clone());
        /* The authenticator key: the one just typed, or the one "Stay signed in" kept (0.16.0). */
        let totp_text = match &stay {
            Stay::On(text) if !text.is_empty() => Some(text.clone()),
            Stay::Off => None,
            _ => crate::auth::keychain::get(&crate::ssh::watch::totp_ref(&ssh)),
        };
        let totp = totp_text.as_deref().and_then(crate::ssh::totp::decode_secret);

        if matches!(&stay, Stay::On(text) if !text.is_empty()) && totp.is_none() {
            let sentence = "That authenticator setup key is not one: it is the base32 text (letters A-Z and digits 2-7) shown when the authenticator was set up, or the first line of ~/.google_authenticator on the host. Leave the box empty and SDC reads it from the host.";

            notifier.push(event::host_status(&host_id, &name, "vps", "offline", None, Some(sentence), None), None, None);

            return;
        }

        let signed = tokio::task::spawn_blocking(move || crate::ssh::session::sign_in_with(&signing, &secret, &one_time, totp))
            .await
            .unwrap_or_else(|_| Err(crate::ssh::session::SignInError::Failed("the sign-in could not be run".into())));

        let refused = match signed {
            Ok(sentence) => {
                install_key = false;

                if let Some(entry) = &keep {
                    let _ = crate::auth::keychain::set(entry, &password);
                }

                let sentence = match stay_signed_in(&ssh, &stay, totp_text.as_deref()).await {
                    Some(note) => format!("{sentence} · {note}"),
                    None => sentence,
                };

                notifier.push(
                    event::host_status(&host_id, &name, "vps", "connecting", None, Some(&sentence), None),
                    None,
                    None,
                );

                None
            }
            Err(crate::ssh::session::SignInError::Unsupported) => None,
            Err(crate::ssh::session::SignInError::NeedsCode) => Some(format!(
                "{} asks for a verification code. Open Sign in, type the password and the code your authenticator shows right now, and press Sign in.",
                ssh.label()
            )),
            /* The sign-in checks the pin itself; a changed key ends it before anything is offered, and
               it gets the pin's own sentence rather than "refused the password". */
            Err(crate::ssh::session::SignInError::Failed(sentence)) if sentence.contains("Host key verification failed") => {
                Some(crate::ssh::ops::refusal(&ssh.label(), "Host key verification failed."))
            }
            Err(crate::ssh::session::SignInError::Failed(sentence)) => Some(sentence),
        };

        if let Some(sentence) = refused {
            let _ = state.store.upsert_host(&host_id, &name, "ssh", Some(&target), "offline", None);

            notifier.push(
                event::host_status(&host_id, &name, "vps", "offline", None, Some(&sentence), None),
                None,
                None,
            );

            return;
        }
    }

    if install_key {
        notifier.push(
            event::host_status(
                &host_id,
                &name,
                "vps",
                "connecting",
                None,
                Some("copying SDC's key with that password…"),
                None,
            ),
            None,
            None,
        );

        let pty = state.pty.clone();
        let install_target = ssh.target.clone();

        let installed = tokio::task::spawn_blocking(move || {
            crate::auth::remote::install_key(&pty, &install_target, &password)
        })
        .await
        .unwrap_or_else(|_| Err(ErrorObject::internal("the key install could not be run")));

        match installed {
            Ok(sentence) => {
                /* The sentence goes on the *status* line rather than into a `Toast`. A toast is written
                   to the event log and replayed on the next launch, so a four-line explanation became
                   four lines of furniture over the Provider Hub every time the app started. The host's
                   own row is where a fact about a host belongs. */
                notifier.push(
                    event::host_status(&host_id, &name, "vps", "connecting", None, Some(&sentence), None),
                    None,
                    None,
                );
            }
            Err(error) => {
                /* The install is the whole reason the password was asked for, so its failure is the
                   host's status - and its sentence is the actionable one. */
                let _ = state.store.upsert_host(&host_id, &name, "ssh", Some(&target), "offline", None);

                notifier.push(
                    event::host_status(&host_id, &name, "vps", "offline", None, Some(&error.message), None),
                    None,
                    None,
                );

                return;
            }
        }
    }

    let probe_target = ssh.clone();

    let (status, detail) = tokio::task::spawn_blocking(move || crate::ssh::ops::probe(&probe_target))
        .await
        .unwrap_or_else(|_| {
            ("offline".to_string(), format!("{target} could not be measured; check the daemon's log"))
        });

    let _ = state.store.upsert_host(&host_id, &name, "ssh", Some(&target), &status, None);

    /* One event, not two. The `HostStatus` carries the sentence and the host's row renders it; the
       `Toast` that used to accompany it was written to the log as well, so a machine that could not be
       reached produced the same four-line paragraph again on every launch. */
    notifier.push(
        event::host_status(&host_id, &name, "vps", &status, None, Some(&detail), None),
        None,
        None,
    );
}

/// After a sign-in, what "Stay signed in" asks for (0.16.0): the authenticator key goes to the keychain - the
/// one typed, or the first line of the host's own `~/.google_authenticator`, read over the connection that
/// was just opened. Off forgets it. The answer is a clause for the host's status line.
async fn stay_signed_in(ssh: &crate::ssh::Ssh, stay: &Stay, typed: Option<&str>) -> Option<String> {
    let entry = crate::ssh::watch::totp_ref(ssh);

    match stay {
        Stay::Unchanged => None,
        Stay::Off => {
            let _ = crate::auth::keychain::delete(&entry);
            crate::ssh::native::set_totp(ssh, None);

            None
        }
        Stay::On(_) => {
            let text = match typed.filter(|text| !text.is_empty()) {
                Some(text) => Some(text.to_string()),
                None => {
                    let reading = ssh.clone();

                    tokio::task::spawn_blocking(move || {
                        reading
                            .run("head -n 1 ~/.google_authenticator 2>/dev/null", crate::ssh::QUICK)
                            .ok()
                            .and_then(|output| crate::ssh::totp::secret_from_file(&output.stdout))
                    })
                    .await
                    .ok()
                    .flatten()
                }
            };

            let Some(secret) = text.as_deref().and_then(crate::ssh::totp::decode_secret) else {
                return Some("Stay signed in is NOT on: SDC found no ~/.google_authenticator on the host. Paste the authenticator's setup key in the Sign in card".to_string());
            };

            if crate::auth::keychain::set(&entry, text.as_deref().unwrap_or_default()).is_err() {
                return Some("Stay signed in is NOT on: this PC's keychain refused the key".to_string());
            }

            crate::ssh::native::set_totp(ssh, Some(secret));

            Some("Stay signed in is on: a dropped connection, or a restarted SDC, signs in again by itself".to_string())
        }
    }
}

/// [`finish_connection`] on its own task, for the caller that is not already in one (`host.trust`).
fn spawn_finish(
    state: Arc<DaemonState>,
    notifier: Arc<dyn Notifier>,
    host_id: String,
    name: String,
    ssh: crate::ssh::Ssh,
    secrets: Secrets,
    key_note: Option<String>,
) {
    tokio::spawn(finish_connection(state, notifier, host_id, name, ssh, secrets, key_note));
}

/// The sentence for a host whose key SDC has never seen: it is a **question**, and it says so.
///
/// This is the layer 0.7.0 did not have. The old probe ran with `accept-new`, so the first key ever
/// seen was trusted for ever and nobody was asked anything. The fingerprint in this sentence is what
/// the dialog puts on screen, and `host.trust` is how it is answered.
fn untrusted_sentence(target: &str, fingerprint: &str, key_note: Option<&str>) -> String {
    let host = crate::ssh::hostkey::host_of(target);
    /*
     * The command to check the fingerprint is the one that **works on that machine**, which is not always
     * `ssh-keyscan`: against OpenSSH 10.2 on Ubuntu, `ssh-keyscan` fails the key exchange while a real
     * `ssh` completes it, so the sentence used to send a person to a command that prints nothing - and the
     * one thing a pin must never do is make its own verification look broken. The handshake form is named
     * first for that reason (it is also what SDC's own scan falls back to), and the `ssh-keyscan` form is
     * offered as the shorter one where it negotiates.
     */
    let port = crate::auth::remote::parse_target(target)
        .ok()
        .and_then(|parsed| parsed.port)
        .unwrap_or(22);

    let base = format!(
        "{target} is reachable, and its host key is {fingerprint} - a key SDC has never seen. Nothing has been sent to it yet: no key was offered and no password was typed. Trust the key to pin it, and SDC finishes the connection. To check that fingerprint in your own terminal, this prints the same string on any machine: `ssh -p {port} -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=check {target} true`, then `ssh-keygen -lf check`. Where `ssh-keyscan` can negotiate with that server, the shorter `ssh-keyscan -p {port} {host} | ssh-keygen -lf -` says the same thing."
    );

    match key_note {
        Some(note) => format!("{base} One thing to fix first: {note}."),
        None => base,
    }
}

/// The sentence for a host presenting a key which is **not** the pinned one - the alarm.
///
/// Both fingerprints are named, because a person has to be able to see which is which, and there is
/// deliberately no "continue anyway" anywhere in this daemon: that is the one thing a man-in-the-middle
/// needs, and a machine whose key changed is a machine something happened to.
fn changed_sentence(target: &str, pinned: &[String], seen: &[crate::ssh::hostkey::HostKey]) -> String {
    format!(
        "{target} presented a host key that is not the one SDC pinned for it. SDC pinned {} and it now presents {}. Nothing was sent to it. If you changed the machine's keys yourself, remove the host and add it again to see and pin the new fingerprint - and if you did not, find out what did.",
        if pinned.is_empty() { "no key".to_string() } else { pinned.join(", ") },
        seen.iter().map(|key| key.fingerprint.clone()).collect::<Vec<_>>().join(", ")
    )
}


/// The last segment of a path: the name `Open folder` gives a project without asking the user for one.
/// `H:\SDC` becomes `SDC`, `/home/me/app` becomes `app`, and a path with no last segment (a drive root,
/// `/`) keeps itself rather than becoming an empty label.
/// A password as the person typed it. Whitespace-only is no password; otherwise only the line break a
/// paste can bring along is removed - spaces are characters of the password.
fn exact_password(raw: String) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }

    raw.trim_end_matches(['\r', '\n']).to_string()
}

fn folder_name(root: &str) -> String {
    std::path::Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| root.to_string())
}

/// `project.locate` on this machine (0.11.0): the usual places a project sits, checked rather than
/// guessed. `<base>/<query>` exactly first, then one level of each base for a folder whose name
/// contains the query (or the domain's first label - `skilleddesk.com` finds `skilleddesk-site`).
fn locate_local(query: &str) -> Vec<Value> {
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return Vec::new();
    };
    let home = std::path::PathBuf::from(home);
    let bases: Vec<std::path::PathBuf> = ["", "Projects", "projects", "code", "dev", "src", "www", "sites", "htdocs", "SDC Workspaces"]
        .iter()
        .map(|base| if base.is_empty() { home.clone() } else { home.join(base) })
        .collect();
    let label = query.split('.').next().unwrap_or(query).to_lowercase();
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |path: std::path::PathBuf, source: &str, candidates: &mut Vec<Value>| {
        let text = path.to_string_lossy().replace('\\', "/");

        if seen.insert(text.clone()) {
            candidates.push(json!({ "root": text, "source": source }));
        }
    };

    for base in &bases {
        let exact = base.join(query);

        if exact.is_dir() {
            push(exact, "an exact folder name", &mut candidates);
        }
    }

    for base in &bases {
        if candidates.len() >= 6 {
            break;
        }

        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };

        for entry in entries.flatten() {
            if candidates.len() >= 6 {
                break;
            }

            let path = entry.path();
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_lowercase();

            if path.is_dir() && label.len() >= 4 && name.contains(&label) {
                push(path, "a folder named like it", &mut candidates);
            }
        }
    }

    candidates
}

/// The folder name a provisioned workspace gets: the chat's title as a slug, with the session id
/// stapled on so two chats called "New chat" never share files. `Fix the login page` and session
/// `n42` become `fix-the-login-page-n42`; a title with nothing usable in it leaves just `chat-n42`.
fn workspace_name(title: &str, session_id: &str) -> String {
    let mut slug = String::new();

    for character in title.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }

        if slug.len() >= 40 {
            break;
        }
    }

    let slug = slug.trim_matches('-');

    if slug.is_empty() {
        format!("chat-{session_id}")
    } else {
        format!("{slug}-{session_id}")
    }
}

/// The first non-empty line of a program's output - a sentence, not a wall of stderr.
///
/// (0.7.13 moved the refusal sentences to `ssh::ops`, which reads `ssh`'s own words itself, so this
/// helper has no caller left here. It is kept only while a later change needs it - deleting it and
/// re-adding the same three lines twice is the alternative.)
#[allow(dead_code)]
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_string()
}


#[cfg(test)]
mod tests {
    use super::*;

    /// PAR's cap. Each agent is a whole turn of its own, so the daemon has to refuse the fourth one rather than
    /// start it: without this the board is a way to run ten model conversations at once by clicking ten times.
    #[test]
    fn the_board_stops_taking_agents_past_the_cap() {
        let state = crate::DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon");
        let daemon = Daemon::new(state.clone());
        let out: Arc<dyn Notifier> = Arc::new(crate::sdcp::notifications::ChannelNotifier::new(
            state.events.clone(),
            state.fanout.clone(),
        ));
        let folder = std::env::temp_dir().join(format!("sdcd-agent-cap-{}", std::process::id()));

        std::fs::create_dir_all(&folder).expect("a project folder");
        state.store.ensure_host("local", "Local", "local", "connected").expect("the local host");
        state
            .store
            .add_project("p1", "local", &folder.to_string_lossy(), "somewhere")
            .expect("a project row");
        state
            .store
            .insert_session("s1", "local", "A chat", "hello", Some("p1"))
            .expect("a chat");

        let ask = Envelope {
            v: crate::SDCP_VERSION.to_string(),
            id: "start-1".to_string(),
            method: "agents.start".to_string(),
            params: serde_json::Map::from_iter([
                ("sessionId".to_string(), json!("s1")),
                ("request".to_string(), json!("Add a test")),
            ]),
            host_id: None,
        };

        for number in 0..3 {
            let mut card = crate::v3::parallel::Agent {
                id: format!("agent-{number}"),
                request: "Something else".to_string(),
                worktree: folder.join(format!("wt-{number}")).to_string_lossy().to_string(),
                started_at: 1_700_000_000 + number,
                state: crate::v3::parallel::State::Running,
                note: String::new(),
                session_id: None,
                turn_id: None,
            };

            std::fs::create_dir_all(&card.worktree).expect("a worktree folder");
            crate::v3::parallel::save(&folder, &card).expect("a card");

            card.state = crate::v3::parallel::State::Running;
        }

        let refused = daemon.agents_start(&ask, out.clone()).expect_err("the fourth agent has to be refused");

        assert_eq!(refused.code, "too_many_agents", "{refused:?}");
        assert!(refused.message.contains("parallel.max"), "the refusal says how to raise it: {}", refused.message);

        /* And the cap is the person's to move: with room, the run is attempted (it fails later, on git, not
           here - which is what this test is about). */
        state.store.set_setting("parallel.max", "4").expect("a setting");

        let past = daemon.agents_start(&ask, out.clone()).map(|answer| answer["agent"]["id"].is_string());

        assert_ne!(past.err().map(|error| error.code), Some("too_many_agents".to_string()), "the raised cap was ignored");

        std::fs::remove_dir_all(&folder).ok();
    }

    /// AUTO's Run button and the daemon's clock share one path (`run_automation`), so they share its refusals
    /// too: an automation the file does not have, and one the person switched off, must start nothing - and
    /// say which of the two it was, because "nothing happened" is what a wrong answer looks like here.
    #[test]
    fn running_an_automation_that_is_missing_or_off_is_refused() {
        let state = crate::DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon");
        let daemon = Daemon::new(state.clone());
        let out: Arc<dyn Notifier> = Arc::new(crate::sdcp::notifications::ChannelNotifier::new(
            state.events.clone(),
            state.fanout.clone(),
        ));
        let folder = std::env::temp_dir().join(format!("sdcd-automation-method-{}", std::process::id()));

        std::fs::create_dir_all(folder.join(".sdc")).expect("a project folder");
        std::fs::write(
            folder.join(".sdc/automations.toml"),
            "[[automation]]\nname = \"nightly\"\nrequest = \"Check the suite\"\nevery = \"6h\"\nenabled = false\n",
        )
        .expect("an automations file");

        let missing = daemon.run_automation(&folder, "morning", out.clone()).expect_err("there is no such automation");
        let off = daemon.run_automation(&folder, "nightly", out.clone()).expect_err("it is switched off");

        assert_eq!(missing.code, "not_found", "{missing:?}");
        assert_eq!(off.code, "automation_disabled", "{off:?}");
        assert!(
            crate::v3::automations::last_runs(&folder).is_empty(),
            "a refusal is not a run, so nothing may be marked as run"
        );

        std::fs::remove_dir_all(&folder).ok();
    }

    /// 0.16.1: a password keeps its spaces; only a pasted line break goes, and blank is no password.
    #[test]
    fn a_password_is_kept_as_typed() {
        assert_eq!(exact_password(" pass word ".into()), " pass word ");
        assert_eq!(exact_password("secret\r\n".into()), "secret");
        assert_eq!(exact_password("   ".into()), "");
    }

    /// 0.10.0: an agent turn in a folderless chat gets a provisioned workspace instead of a refusal,
    /// and the folder's name is the chat's, sluggified, with the session id keeping two "New chat"s
    /// apart.
    #[test]
    fn a_workspace_is_named_after_the_chat_and_the_session() {
        assert_eq!(workspace_name("Fix the login page", "n42"), "fix-the-login-page-n42");
        assert_eq!(workspace_name("New chat", "n7"), "new-chat-n7");
        assert_eq!(workspace_name("New chat", "n8"), "new-chat-n8");
        assert_eq!(workspace_name("!!!", "n9"), "chat-n9");
        assert_eq!(workspace_name("", "n10"), "chat-n10");

        /* A very long title stays a folder name, not a path problem. */
        assert!(workspace_name(&"word ".repeat(30), "n11").len() <= 48);
    }

    /// `project.locate` on this machine finds a folder that really exists and never invents one:
    /// the exact name wins, a containing name follows, and an empty home answers with nothing.
    #[test]
    fn locate_local_finds_the_exact_folder_first() {
        let home = std::env::temp_dir().join("sdc-locate-test-home");
        let projects = home.join("Projects");

        std::fs::create_dir_all(projects.join("skilleddesk.com")).unwrap();
        std::fs::create_dir_all(projects.join("skilleddesk-old")).unwrap();

        /* The helper reads the home from the environment, so the test lends it one. */
        let saved = std::env::var_os("USERPROFILE");

        std::env::set_var("USERPROFILE", &home);

        let candidates = locate_local("skilleddesk.com");

        match saved {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }

        assert!(!candidates.is_empty(), "the exact folder exists and must be found");
        assert!(
            candidates[0]["root"].as_str().unwrap().ends_with("skilleddesk.com"),
            "exact name first: {candidates:?}"
        );
        assert_eq!(candidates[0]["source"], json!("an exact folder name"));
        assert!(
            candidates.iter().any(|candidate| candidate["root"].as_str().unwrap().ends_with("skilleddesk-old")),
            "the containing name follows: {candidates:?}"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// The VPS in the bug report: `ssh` refused because the host wants a password or a one-time
    /// verification code - the user logs in from a terminal every day, so "unreachable" is the wrong
    /// sentence and leaves them nothing to do. The sentence now lives in `ssh::ops::refusal` (0.7.13),
    /// where the same rules cover the probe, a file read and a git call; this test is the daemon-level
    /// guarantee that the *method* still answers with it rather than with "offline".
    #[test]
    fn a_host_that_wants_a_password_is_not_reported_as_unreachable() {
        let sentence = crate::ssh::ops::refusal(
            "root@vps.example",
            "root@vps.example: Permission denied (keyboard-interactive,publickey).",
        );

        assert!(sentence.contains("answered"), "{sentence}");
        assert!(sentence.contains("password"), "{sentence}");
        assert!(sentence.contains("authorized_keys"), "{sentence}");
        assert!(!sentence.contains("did not answer"), "{sentence}");
    }

    /// Everything else keeps `ssh`'s own first line, because that is what a person can act on.
    #[test]
    fn every_other_refusal_keeps_ssh_own_words() {
        let timed_out = crate::ssh::ops::refusal("h", "\nssh: connect to host h port 22: Connection timed out\nmore\n");

        assert_eq!(timed_out, "h did not answer: ssh: connect to host h port 22: Connection timed out");

        let host_key = crate::ssh::ops::refusal("h", "Host key verification failed.");

        assert!(host_key.contains("host key is not the one SDC pinned"), "{host_key}");
        assert!(host_key.contains("Add the host again"), "{host_key}");
    }

    /// The delta limiter (0.11.7): gathers streamed text for up to [`DELTA_WINDOW`], or until it grows
    /// past 4096 bytes, whichever comes first - the guard against the report's 14,000-event turn.
    #[test]
    fn the_delta_limiter_is_due_on_size_or_time_and_flushes_only_what_is_not_empty() {
        use crate::sdcp::notifications::RecordingNotifier;

        let mut pending = Pending::default();

        assert!(!pending.due(), "nothing pending is never due");

        pending.answer.push_str("hi");
        pending.since = Some(std::time::Instant::now());

        assert!(!pending.due(), "well under both the window and the size cap");

        pending.answer = "x".repeat(4096);

        assert!(pending.due(), "the size cap trips regardless of how young `since` still is");

        pending.answer.clear();
        pending.thinking.push_str("thinking");
        pending.since = Some(std::time::Instant::now() - DELTA_WINDOW);

        assert!(pending.due(), "the time window alone is enough, with nothing in `answer`");

        let notifier = RecordingNotifier::new();

        pending.flush(&notifier, "turn-1", &None, &None);

        assert_eq!(notifier.kinds(), vec!["ThinkingDelta"], "an empty answer is never pushed as its own delta");
        assert!(pending.thinking.is_empty(), "flush takes what it pushed");
        assert!(pending.since.is_none(), "flush resets the clock so the next byte starts a fresh window");
    }

    /// The size cap is on the *combined* length, not either field alone - and when both kinds have
    /// gathered text, `flush` pushes both, thinking first (an engine's thinking always precedes its
    /// answer within the same window).
    #[test]
    fn the_delta_limiter_caps_the_combined_length_and_flushes_both_kinds_in_order() {
        use crate::sdcp::notifications::RecordingNotifier;

        let mut pending = Pending { answer: "a".repeat(2048), thinking: "t".repeat(2047), ..Pending::default() };

        assert!(!pending.due(), "one byte under the combined cap");

        pending.thinking.push('t');

        assert!(pending.due(), "the combined length alone trips the cap, with neither field at 4096 on its own");

        let notifier = RecordingNotifier::new();

        pending.flush(&notifier, "turn-1", &None, &None);

        assert_eq!(notifier.kinds(), vec!["ThinkingDelta", "TurnDelta"], "thinking is flushed before the answer");
        assert!(pending.answer.is_empty() && pending.thinking.is_empty(), "flush takes both");
    }

    /// A custom window - what low-bandwidth mode sets `Pending::window` to - governs `due`, not the
    /// constant [`DELTA_WINDOW`].
    #[test]
    fn the_delta_limiter_is_due_on_its_own_window_not_the_default() {
        let mut pending = Pending { window: LOW_BANDWIDTH_WINDOW, ..Pending::default() };

        pending.answer.push_str("hi");
        pending.since = Some(std::time::Instant::now() - DELTA_WINDOW);

        assert!(!pending.due(), "past the default window, but the wider low-bandwidth window has not elapsed yet");

        pending.since = Some(std::time::Instant::now() - LOW_BANDWIDTH_WINDOW);

        assert!(pending.due(), "the wider window has now elapsed");
    }

    /// The other half of `the_delta_limiter_is_due_on_size_or_time_and_flushes_only_what_is_not_empty`:
    /// an answer alone is pushed as its own delta with nothing for `thinking`, and a `flush` with
    /// neither field holding anything pushes nothing at all - the no-op the loop falls into on every
    /// tick that finds no pending bytes.
    #[test]
    fn the_delta_limiter_flushes_answer_alone_and_nothing_when_both_are_empty() {
        use crate::sdcp::notifications::RecordingNotifier;

        let mut pending = Pending { answer: "hi".to_string(), ..Pending::default() };

        let notifier = RecordingNotifier::new();

        pending.flush(&notifier, "turn-1", &None, &None);

        assert_eq!(notifier.kinds(), vec!["TurnDelta"], "a thinking-less answer is never pushed as a ThinkingDelta");
        assert_eq!(notifier.streamed_text(), "hi");
        assert!(pending.answer.is_empty(), "flush takes what it pushed");

        let notifier = RecordingNotifier::new();

        pending.flush(&notifier, "turn-1", &None, &None);

        assert!(notifier.kinds().is_empty(), "nothing pending means nothing pushed");
    }

    /// The sentence a `host.add` puts on screen when the machine's key is one SDC has never seen, and
    /// the one it puts there when that key is *wrong*. Two different questions, two different actions.
    #[test]
    fn a_trust_question_and_a_changed_key_are_two_different_sentences() {
        let asked = untrusted_sentence("ssh -p 8443 root@vps.example", "SHA256:abc", None);

        assert!(asked.contains("SHA256:abc"), "{asked}");
        assert!(asked.contains("never seen"), "{asked}");
        assert!(asked.contains("Nothing has been sent"), "{asked}");
        /* The command a person can check the fingerprint with has to be one that **works there**: the
           handshake form first, because `ssh-keyscan` cannot negotiate with every server (it fails on the
           host in the bug report) - and the port from the target, not a guessed 22. */
        assert!(asked.contains("-o StrictHostKeyChecking=accept-new"), "{asked}");
        assert!(asked.contains("ssh-keygen -lf check"), "{asked}");
        assert!(asked.contains("-p 8443"), "{asked}");
        assert!(asked.contains("ssh-keyscan"), "the shorter form is offered too: {asked}");

        let with_note = untrusted_sentence("root@vps.example", "SHA256:abc", Some("ssh-keygen is missing"));
        assert!(with_note.ends_with("One thing to fix first: ssh-keygen is missing."), "{with_note}");

        let alarmed = changed_sentence(
            "root@vps.example",
            &["SHA256:pin".to_string()],
            &[crate::ssh::hostkey::HostKey {
                key_type: "ssh-ed25519".into(),
                base64: "AAAA".into(),
                fingerprint: "SHA256:now".into(),
                line: "root@vps.example ssh-ed25519 AAAA".into(),
            }],
        );

        assert!(alarmed.contains("SHA256:pin"), "{alarmed}");
        assert!(alarmed.contains("SHA256:now"), "{alarmed}");
        assert!(alarmed.contains("Nothing was sent"), "{alarmed}");
        assert!(!alarmed.to_lowercase().contains("continue anyway"), "{alarmed}");
    }

    /// The acceptance test for live streaming - the report *"akbare answare disse"*, in code.
    ///
    /// The fake engine **cannot finish until the daemon has already pushed its first delta**: it sends
    /// `Hel`, then waits for a signal that only the notifier's `push` of that delta can open, and only
    /// then sends the rest. If `run_turn` still collected the stream before forwarding it - which is
    /// exactly what it did until 0.7.4 - the turn would deadlock here, and the timeout below would fail
    /// the suite instead of hanging it.
    ///
    /// So the assertion is about **when** the event left the daemon, not only about its text: the
    /// second half of the answer exists only because the first half had already been delivered.
    #[tokio::test]
    async fn a_turn_streams_its_deltas_while_the_engine_is_still_running() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::sync::oneshot;

        /// An engine whose second half is gated on the daemon's delivery of its first half.
        struct Halfway {
            release: std::sync::Mutex<Option<oneshot::Receiver<()>>>,
        }

        #[async_trait::async_trait]
        impl crate::engines::Engine for Halfway {
            fn id(&self) -> &'static str {
                "halfway"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                sink.send(crate::engines::EngineEvent::Delta("Hel".to_string()));

                let release = self
                    .release
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .take();

                if let Some(release) = release {
                    let _ = release.await;
                }

                sink.send(crate::engines::EngineEvent::Delta("lo".to_string()));
                sink.send(crate::engines::EngineEvent::Done {
                    summary: "Done".to_string(),
                    meta: String::new(),
                    pass: Some(true),
                });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Running
            }
        }

        /// A notifier that opens the gate on the first `TurnDelta` it is given.
        struct GateNotifier {
            opened: AtomicBool,
            release: std::sync::Mutex<Option<oneshot::Sender<()>>>,
            events: std::sync::Mutex<Vec<Value>>,
        }

        impl GateNotifier {
            fn kinds(&self) -> Vec<String> {
                self.events
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .iter()
                    .filter_map(|event| {
                        event
                            .get("type")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect()
            }
        }

        impl Notifier for GateNotifier {
            fn push(
                &self,
                event: Value,
                _session: Option<String>,
                _turn: Option<String>,
            ) -> Option<i64> {
                let first_delta = event.get("type").and_then(Value::as_str) == Some("TurnDelta")
                    && !self.opened.swap(true, Ordering::SeqCst);

                if first_delta {
                    let release = self
                        .release
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .take();

                    if let Some(release) = release {
                        let _ = release.send(());
                    }
                }

                self.events
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .push(event);

                None
            }
        }

        let (release, gated) = oneshot::channel();
        let notifier = Arc::new(GateNotifier {
            opened: AtomicBool::new(false),
            release: std::sync::Mutex::new(Some(release)),
            events: std::sync::Mutex::new(Vec::new()),
        });
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "halfway".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(Halfway {
            release: std::sync::Mutex::new(Some(gated)),
        });
        let out: Arc<dyn Notifier> = notifier.clone();

        let ran = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_turn(state, engine, plan, out),
        )
        .await;

        assert!(
            ran.is_ok(),
            "the turn never ended: the daemon was not forwarding deltas as they arrived"
        );
        assert_eq!(
            notifier.kinds(),
            vec!["TurnDelta", "TurnDelta", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"]
        );
    }

    /// The limiter earns its keep here: an engine that sends its deltas back-to-back, with nothing
    /// gating it between them, still reaches the window as **one** `TurnDelta` - the coalescing
    /// `the_delta_limiter_is_due_on_size_or_time_and_flushes_only_what_is_not_empty` checks on `Pending`
    /// alone, this time through `run_turn` itself, where the 14,000-event turn actually happened.
    #[tokio::test]
    async fn rapid_deltas_are_coalesced_into_one_push_by_run_turn() {
        use crate::sdcp::notifications::RecordingNotifier;

        struct Rapid;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Rapid {
            fn id(&self) -> &'static str {
                "rapid"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                for piece in ["Hel", "lo", ", ", "wor", "ld"] {
                    sink.send(crate::engines::EngineEvent::Delta(piece.to_string()));
                }

                sink.send(crate::engines::EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let notifier = Arc::new(RecordingNotifier::new());
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "rapid".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(Rapid);
        let out: Arc<dyn Notifier> = notifier.clone();

        run_turn(state, engine, plan, out).await;

        assert_eq!(
            notifier.kinds(),
            vec!["TurnDelta", "TurnDelta", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"],
            "the first delta goes out at once (0.21); the four after it, which never waited for the window, land as one push"
        );
        assert_eq!(notifier.streamed_text(), "Hello, world");
    }

    /// The limiter's other trigger, also through `run_turn`: a delta that alone crosses the 4096-byte
    /// cap must not sit and wait for [`DELTA_WINDOW`] - it goes out the moment it lands, same as the
    /// unit test on `Pending` checks, but here through the loop that decides when `flush` runs.
    #[tokio::test]
    async fn an_oversized_delta_is_flushed_by_run_turn_without_waiting_for_the_window() {
        use crate::sdcp::notifications::RecordingNotifier;

        struct Oversized;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Oversized {
            fn id(&self) -> &'static str {
                "oversized"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                sink.send(crate::engines::EngineEvent::Delta("x".repeat(4096)));
                sink.send(crate::engines::EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let notifier = Arc::new(RecordingNotifier::new());
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "oversized".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(Oversized);
        let out: Arc<dyn Notifier> = notifier.clone();

        let ran = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            run_turn(state, engine, plan, out),
        )
        .await;

        assert!(
            ran.is_ok(),
            "a delta past the size cap should flush on arrival, not sit until the window elapses"
        );
        assert_eq!(
            notifier.kinds(),
            vec!["TurnDelta", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"],
            "the oversized delta is its own push, not folded into whatever follows"
        );
        assert_eq!(notifier.streamed_text(), "x".repeat(4096));
    }

    /// The limiter must not reorder the stream it is smoothing: a delta sat buffered under the window
    /// has to reach the notifier *before* whatever non-delta event the engine sends next, or a person
    /// would see `ToolCallStarted` on screen ahead of the words that, in the engine's own timeline,
    /// came first. That is the `_ => pending.flush(...)` arm above, and until now nothing exercised it.
    #[tokio::test]
    async fn a_pending_delta_flushes_before_the_next_non_delta_event() {
        use crate::sdcp::notifications::RecordingNotifier;

        struct ThenATool;

        #[async_trait::async_trait]
        impl crate::engines::Engine for ThenATool {
            fn id(&self) -> &'static str {
                "then-a-tool"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                sink.send(crate::engines::EngineEvent::Delta("checking the tests".to_string()));
                sink.send(crate::engines::EngineEvent::ToolStarted {
                    call_id: "c1".into(),
                    tool: "read".into(),
                    name: "Read".into(),
                    target: "tests.rs".into(),
                });
                sink.send(crate::engines::EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let notifier = Arc::new(RecordingNotifier::new());
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "then-a-tool".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(ThenATool);
        let out: Arc<dyn Notifier> = notifier.clone();

        run_turn(state, engine, plan, out).await;

        assert_eq!(
            notifier.kinds(),
            vec!["TurnDelta", "ToolCallStarted", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"],
            "the buffered delta must be flushed ahead of the tool event, not left to trail in behind it"
        );
        assert_eq!(notifier.streamed_text(), "checking the tests");
    }

    /// The dedup arm above (0.14.1): an engine that names the same call_id again - Claude Code resends a
    /// finished tool_use block on occasion - must not open a second card, and must not feed the repeat to
    /// [`crate::trust::cost::Runaway`] a second time. Five resends of the same call_id would cross
    /// [`crate::trust::cost::REPEAT_LIMIT`] if each were counted, and stop the turn as a loop that never happened.
    #[tokio::test]
    async fn a_repeated_call_id_opens_one_card_and_is_not_counted_twice() {
        use crate::sdcp::notifications::RecordingNotifier;

        struct Repeater;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Repeater {
            fn id(&self) -> &'static str {
                "repeater"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                use crate::engines::EngineEvent;

                for _ in 0..5 {
                    sink.send(EngineEvent::ToolStarted {
                        call_id: "c1".into(),
                        tool: "run".into(),
                        name: "Bash".into(),
                        target: "ls".into(),
                    });
                }
                sink.send(EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let notifier = Arc::new(RecordingNotifier::new());
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "repeater".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(Repeater);
        let out: Arc<dyn Notifier> = notifier.clone();

        run_turn(state, engine, plan, out).await;

        assert_eq!(
            notifier.kinds(),
            vec!["ToolCallStarted", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"],
            "five resends of the same call_id must open exactly one card and never trip the runaway stop"
        );
    }

    /// The answer stored for the next turn carries the tool calls: without them a model read its own
    /// "I ran the tests" with no run in sight and apologised for inventing work it had really done.
    #[tokio::test]
    async fn the_stored_answer_remembers_the_turns_tool_calls() {
        struct Worker;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Worker {
            fn id(&self) -> &'static str {
                "worker"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                use crate::engines::EngineEvent;

                sink.send(EngineEvent::ToolStarted { call_id: "c1".into(), tool: "run".into(), name: "Run".into(), target: "npm test".into() });
                sink.send(EngineEvent::ToolCompleted { call_id: "c1".into(), status: "done".into(), meta: "exit 0 · 400ms".into(), diff: None });
                sink.send(EngineEvent::Delta("All five tests pass.".into()));
                sink.send(EngineEvent::Done { summary: "Done".into(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        struct Quiet;

        impl Notifier for Quiet {
            fn push(&self, _event: Value, _session: Option<String>, _turn: Option<String>) -> Option<i64> {
                None
            }
        }

        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon for the test");

        state.store.ensure_session("s1").unwrap();
        state.store.start_turn("turn-7", "s1", 1, "worker", "m", "Balanced", "run the tests").unwrap();

        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-7".to_string(),
            engine_id: "worker".to_string(),
            prompt_text: "run the tests".to_string(),
            model: "m".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };

        run_turn(state.clone(), Arc::new(Worker), plan, Arc::new(Quiet)).await;

        let history = session_bridge::history_for(&state.store, "s1").unwrap();
        let said = &history.last().unwrap().text;

        assert!(said.starts_with("All five tests pass."), "{said}");
        assert!(said.contains("- Run npm test - exit 0 · 400ms"), "{said}");
    }

    /// Master spec §12: a chat waiting on the person is not a chat that is working. The stored turn that
    /// brought this home spent 5 h 20 m on one permission card while its chat still read `running`, so the
    /// sidebar said "working" to a person who was the one holding it up.
    #[test]
    fn a_chat_waiting_on_the_person_says_so_and_says_when_it_stops() {
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon");
        let recorder = std::sync::Arc::new(crate::sdcp::notifications::RecordingNotifier::new());
        let out: &dyn Notifier = &*recorder;

        state.store.ensure_host("local", "Local", "local", "connected").expect("the local host");
        state.store.insert_session("s1", "local", "A chat", "hello", None).expect("a chat");

        note_waiting(&state.store, "s1", Some("awaiting_approval"), out);

        assert_eq!(state.store.sessions_for_host("local").expect("rows")[0]["attention"], "awaiting_approval");
        assert_eq!(recorder.kinds(), vec!["SessionUpdated"]);

        note_waiting(&state.store, "s1", None, out);

        assert_eq!(
            state.store.sessions_for_host("local").expect("rows")[0]["attention"],
            Value::Null,
            "the amber ! comes off with the answer, not with the turn"
        );

        let last = recorder.events().pop().expect("an event");

        assert_eq!(last["attention"], Value::Null, "the window is told too: {last}");
    }

    /// The wiring behind the two above, in a live turn: the loop marks the chat the moment an engine asks for
    /// a card, keeps the card's chat (so `permission.resolve` can take the mark off at once), and clears it
    /// when the turn is over.
    #[tokio::test]
    async fn a_permission_card_in_a_live_turn_marks_the_chat_and_the_turn_end_clears_it() {
        struct Asking;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Asking {
            fn id(&self) -> &'static str {
                "asking"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                sink.send(crate::engines::EngineEvent::Permission {
                    permission_id: "perm-7".to_string(),
                    title: "Run the suite?".to_string(),
                    sub: String::new(),
                    action: "run".to_string(),
                    target: "cargo test".to_string(),
                    risk: "SAFE".to_string(),
                    explain: String::new(),
                });
                sink.send(crate::engines::EngineEvent::Delta("ok".to_string()));
                sink.send(crate::engines::EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: Some(true) });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Running
            }
        }

        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon");

        state.store.ensure_host("local", "Local", "local", "connected").expect("the local host");
        state.store.insert_session("s1", "local", "A chat", "hello", None).expect("a chat");

        let recorder = std::sync::Arc::new(crate::sdcp::notifications::RecordingNotifier::new());
        let out: Arc<dyn Notifier> = recorder.clone();
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-card".to_string(),
            engine_id: "asking".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };

        run_turn(state.clone(), Arc::new(Asking), plan, out).await;

        let kinds = recorder.kinds();
        let card = kinds.iter().position(|kind| kind == "PermissionRequested").expect("the card");

        assert_eq!(kinds[card - 1], "SessionUpdated", "the chat is marked when the card is asked: {kinds:?}");
        assert!(
            recorder.events().iter().any(|event| event["attention"] == "awaiting_approval" && event["sessionId"] == "s1"),
            "the window is told which chat it is waiting on: {:?}",
            recorder.kinds()
        );
        assert_eq!(take_card_session("perm-7"), Some("s1".to_string()), "the card knows the chat it was asked in");
        assert_eq!(
            state.store.sessions_for_host("local").expect("rows")[0]["attention"],
            Value::Null,
            "the turn ended, so nothing is waiting on the person any more"
        );
    }
}
