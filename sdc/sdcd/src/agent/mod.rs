//! **SDC Agent** - the daemon's own agent loop (docs/ROADMAP-v4.md, Phase 2).
//!
//! Until v4, "give it a prompt and the project gets built" worked exactly as far as the CLI behind the
//! chat could take it: `claude`, `codex` and `gemini` are agents, and an API key or a local model was a
//! question-and-answer chat. This module is the loop those CLIs have inside them, run by the daemon:
//!
//! ```text
//!   ask the model ──► it answers with text, and maybe tool calls
//!        ▲                     │
//!        │          for each call: card ► (checkpoint) ► gate ► run ► result
//!        └───── results go back as the next message, until it stops calling tools
//! ```
//!
//! Why here and not in the window: the eight tools sit on the daemon paths that already exist and
//! already work **on a host** (`fs.*`, `shell.run`, `git.diff` over `ssh`), so an agent whose chat is
//! on a VPS works inside that VPS with no second implementation. The checkpoint before the first
//! change (P5), the deny list, the file guard and the permission dialog are the daemon's, so the agent
//! obeys them without re-implementing them.
//!
//! Bounds, because a runaway loop is a bill: the runaway detector and the cost governor, a Stop that
//! drops the connection mid-answer, and a token count on the turn's footer. A step count is only a
//! bound when the caller asks for one (`maxSteps`); by default a turn runs until the work is done.

pub mod background;
pub mod browser;
pub mod dialect;
pub mod gate;
pub mod mcp;
pub mod patch;
pub mod search;
pub mod skills;
pub mod tools;
pub mod web;
pub mod workspace;

use std::collections::HashSet;

use async_trait::async_trait;
use serde_json::Value;

use crate::engines::{EngineEvent, EngineStatus, EventSink, Prompt, Role};
use dialect::{Dialect, Reply};
use gate::Autonomy;
use tools::ToolContext;
use workspace::Workspace;

/// The number of model calls one turn may make when the caller names none: no limit (0.15.2).
///
/// The report: *"kono rate limit to dorkar nai project a"* - a landing page built from two long specs
/// stopped at 60 steps, still reading, and asked for "continue". With older tool output folded as a turn
/// grows (`keep_small`), a long turn does not outgrow the model's window, so a count of steps protects
/// nothing a person wants. What stops a turn that goes wrong is still there: Stop, the cost governor
/// (a budget set in Settings) and the runaway detector (the same call again and again). A caller that
/// wants a bound - `sdcd run --max-steps` - still passes `maxSteps`.
pub const DEFAULT_STEPS: usize = usize::MAX;

/// Where the model for this turn lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// A provider behind an API key (`native_api`'s endpoints).
    Api,
    /// The local Ollama daemon, through its OpenAI-compatible `/v1` endpoint.
    Ollama,
}

pub struct SdcAgent {
    backend: Backend,
    autonomy: Autonomy,
    max_steps: usize,
    auto_check: bool,
    checkpoint: Option<tools::Checkpointer>,
    /// The folder's policy (0.12): what the tools must ask about or refuse before they act.
    policy: crate::trust::policy::Policy,
}

impl SdcAgent {
    pub fn new(backend: Backend, autonomy: Autonomy, max_steps: usize) -> Self {
        Self { backend, autonomy, max_steps: max_steps.max(1), auto_check: true, checkpoint: None, policy: Default::default() }
    }

    /// Whether SDC runs the project's checks when the agent says it is done (Settings → Agent, 0.13).
    pub fn with_auto_check(mut self, auto_check: bool) -> Self {
        self.auto_check = auto_check;
        self
    }

    /// The Trust Kernel's policy for this turn's folder.
    pub fn with_policy(mut self, policy: crate::trust::policy::Policy) -> Self {
        self.policy = policy;
        self
    }

    /// The daemon's checkpoint for this turn, taken by the agent itself before its first change.
    pub fn with_checkpoint(mut self, checkpoint: tools::Checkpointer) -> Self {
        self.checkpoint = Some(checkpoint);
        self
    }
}

#[async_trait]
impl crate::engines::Engine for SdcAgent {
    fn id(&self) -> &'static str {
        "sdc_agent"
    }

    async fn start(&self, prompt: Prompt, sink: &EventSink) {
        let sink = sink.clone();
        let backend = self.backend;
        let options = Options { autonomy: self.autonomy, max_steps: self.max_steps, auto_check: self.auto_check };
        let checkpoint = self.checkpoint.clone();
        let policy = self.policy.clone();

        /* Every step blocks - a streaming HTTP read, a file read over ssh, a command - so the whole loop
           runs on the blocking pool, and its events travel through the sink while it does. */
        let _ = tokio::task::spawn_blocking(move || run(backend, options, checkpoint, &policy, &prompt, &sink)).await;
    }

    async fn cancel(&self, turn_id: &str) -> bool {
        /* The loop checks the cancel mark between every read and every step; setting it (which
           `engine.cancel` has already done) is the whole cancellation. */
        crate::engines::cancel::request(turn_id);

        true
    }

    fn status(&self, _turn_id: &str) -> EngineStatus {
        EngineStatus::Idle
    }
}

/// The resolved endpoint of a turn: where to POST, with which headers, in which dialect.
struct Target {
    url: String,
    headers: Vec<(String, String)>,
    dialect: Dialect,
    model: String,
    /// `$in / $out` per million tokens, when the catalogue knows the model.
    price: Option<(f64, f64)>,
    thinking: bool,
    /// The turn's effort (0.14), for a model that takes `reasoning_effort`.
    effort: Option<String>,
}

fn target(backend: Backend, prompt: &Prompt) -> Result<Target, String> {
    match backend {
        Backend::Ollama => {
            let model = if prompt.model.trim().is_empty() {
                "llama3.2:3b".to_string()
            } else {
                crate::engines::native_api::api_model(&prompt.model).to_string()
            };

            Ok(Target {
                url: "http://127.0.0.1:11434/v1/chat/completions".to_string(),
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                dialect: Dialect::OpenAi,
                model,
                price: None,
                thinking: false,
                effort: None,
            })
        }
        Backend::Api => {
            let endpoint = crate::engines::native_api::endpoint_for(&prompt.model, prompt.provider.as_deref());
            let key = crate::auth::keychain::get(&endpoint.key_ref).unwrap_or_default();

            if key.is_empty() {
                return Err(format!(
                    "No API key for {}. Connect it in the Provider Hub; the key is stored in the OS keychain.",
                    endpoint.provider
                ));
            }

            let model = crate::engines::native_api::api_model(&prompt.model).to_string();
            let mut headers = vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("accept".to_string(), "text/event-stream".to_string()),
            ];
            let dialect = if endpoint.dialect == "anthropic" {
                headers.push(("x-api-key".to_string(), key));
                headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
                Dialect::Anthropic
            } else {
                headers.push(("authorization".to_string(), format!("Bearer {key}")));
                Dialect::OpenAi
            };

            Ok(Target {
                url: endpoint.url,
                headers,
                dialect,
                thinking: dialect == Dialect::Anthropic && crate::engines::native_api::adaptive_thinking(&model),
                price: price_of(&endpoint.provider, &model),
                effort: prompt.effort.clone(),
                model,
            })
        }
    }
}

/// The catalogue's `"$4 / $20"` for a model, as numbers.
fn price_of(provider: &str, model: &str) -> Option<(f64, f64)> {
    let blocks = crate::providers::models::blocked();
    let row = blocks
        .iter()
        .filter(|block| block.id == provider || provider.is_empty())
        .flat_map(|block| block.models.iter())
        .find(|row| row["id"].as_str() == Some(model))?;

    parse_price(row["cost"].as_str()?)
}

fn parse_price(cost: &str) -> Option<(f64, f64)> {
    let mut numbers = cost.split('/').map(|part| part.trim().trim_start_matches('$').trim().parse::<f64>().ok());

    Some((numbers.next()??, numbers.next()??))
}

/// The turn's footer: tokens in and out, steps, and the price when the catalogue knows it.
fn meta(steps: usize, input: u64, output: u64, price: Option<(f64, f64)>) -> String {
    let tokens = |count: u64| {
        if count >= 1000 { format!("{:.1}k", count as f64 / 1000.0) } else { count.to_string() }
    };
    let mut meta = format!(
        "{} step{} · {} in · {} out",
        steps,
        if steps == 1 { "" } else { "s" },
        tokens(input),
        tokens(output)
    );

    if let Some((per_in, per_out)) = price {
        let cost = input as f64 / 1_000_000.0 * per_in + output as f64 / 1_000_000.0 * per_out;

        /* Two decimals said "≈$0.00" for a real, small bill; below a cent the digits that matter are shown. */
        if cost >= 0.01 {
            meta.push_str(&format!(" · ≈${cost:.2}"));
        } else {
            meta.push_str(&format!(" · ≈${cost:.4}"));
        }
    }

    meta
}

/// The language the person writes in, read from their own words - not from SDC's brief around them, which
/// is English and would make every chat look English.
fn person_language(prompt: &Prompt) -> crate::understand::Reading {
    let words = prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text);
    let reading = crate::understand::Reading::of(words);

    if reading.code != "en" {
        return reading;
    }

    /* A short English follow-up in a chat written in Bengali is still a Bengali chat. */
    prompt
        .history
        .iter()
        .rev()
        .filter(|message| message.role == Role::User)
        .take(6)
        .map(|message| crate::understand::Reading::of(&message.text))
        .find(|earlier| earlier.code != "en")
        .unwrap_or(reading)
}

fn system_prompt(workspace: &Workspace, vision: bool, language: &crate::understand::Reading) -> String {
    let look = if vision {
        "\n- For a web page or UI you built, start its dev server with start_process and try it in the browser tool as a user would: open it, read it, click the buttons, fill the forms, and take a screenshot to check it looks right - on a phone width (390) too. Fix what does not work."
    } else {
        "\n- For a web page or UI you built, start its dev server with start_process and try it in the browser tool as a user would: open it, read it, click the buttons and fill the forms. Fix what does not work."
    };

    format!(
        "You are SDC Agent, a coding agent working inside a person's project through the tools you are given.\n\
         \n\
         Project folder: {root}\n\
         Machine: {place}\n\
         Shell for run_command: {shell}\n\
         \n\
         How to work:\n\
         - Understand before changing: find the files that matter with glob and grep, then read them (offset and limit for long files).\n\
         - For broad exploration or research, hand self-contained jobs to task sub-agents - several in one reply run in parallel - instead of reading everything yourself.\n\
         - For anything with more than two steps, call update_plan first. The person watches that checklist: call update_plan again each time a step starts or finishes, and mark every step done before your final answer.\n\
         - Change files with edit_file (exact text replacement); use write_file for new files or full rewrites. Match the project's existing style.\n\
         - Verify your work: build it, run the tests or run the program with run_command, read the output, and fix what fails.\n\
         - A command that does not exit on its own (a dev server, a watcher) goes in start_process, never in run_command. Stop what you started when you no longer need it, unless the person will want it running.{look}\n\
         - When you are unsure how a library, framework or API works, look it up with web_search and web_fetch instead of guessing.\n\
         - When a decision belongs to the person (a design choice, deleting data, two readings of the request), ask with ask_user and offer options. Do not ask about what you can find out yourself.\n\
         - When the person states a lasting preference or a project convention, keep it with remember.\n\
         - Stay inside the project folder. Secrets (.env, keys) are hidden from you on purpose; do not try to read them.\n\
         - If the person declines an action, do not try it another way; explain what you wanted to do.\n\
         - Keep going until the task is completely done; do not stop half-way to ask whether to continue. When you are done, stop calling tools and answer with a short summary: what you changed, how you verified it, and anything the person must do themselves.\n\
         - Language: the person writes in {label}. Write every answer, summary and question to them in {reply} - never switch to another language (not Chinese, not German, not English unless that is theirs). Keep code, commands, paths and error messages exactly as they are.",
        label = language.label,
        reply = if language.code == "en" { "English" } else { language.reply_in },
        root = workspace.root(),
        place = workspace.place(),
        shell = workspace.shell(),
    )
}

fn sub_agent_prompt(workspace: &Workspace) -> String {
    format!(
        "You are a research sub-agent of SDC Agent, working in a person's project with read-only tools.\n\
         \n\
         Project folder: {root}\n\
         Machine: {place}\n\
         \n\
         Do the job you are given - find, read, compare, look up - and answer with a report: the facts found, \
         with file paths and line numbers, and the answer to the question. You cannot change anything; if something \
         should change, say what and where. Be thorough but stop as soon as you can answer. Your report is read by \
         another model, not by the person: no pleasantries.",
        root = workspace.root(),
        place = workspace.place(),
    )
}

/// OpenAI's reasoning models, which take `reasoning_effort`.
fn reasoning_model(model: &str) -> bool {
    let bare = model.rsplit('/').next().unwrap_or(model).to_ascii_lowercase();

    bare.starts_with("gpt-5") || bare.starts_with("o1") || bare.starts_with("o3") || bare.starts_with("o4") || bare.contains("codex")
}

/// A sink that separates the text of one step from the text of the step before it.
///
/// The answer is every step's text in a row; without a break, "Reading the file." and "Fixed it." of
/// two steps run together into one sentence.
fn step_sink(sink: &EventSink, separate: bool) -> EventSink {
    let inner = sink.clone();
    let pending = std::sync::Mutex::new(separate);

    EventSink::new(move |event| {
        if let EngineEvent::Delta(text) = &event {
            if let Ok(mut pending) = pending.lock() {
                if *pending && !text.trim().is_empty() {
                    *pending = false;
                    inner.send(EngineEvent::Delta("\n\n".to_string()));
                }
            }
        }

        inner.send(event);
    })
}

/// How one turn of the agent runs: the person's autonomy, how many model calls it may make, and
/// whether SDC runs the project's checks when the agent says it is done.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub autonomy: Autonomy,
    pub max_steps: usize,
    pub auto_check: bool,
}

/// The loop.
pub fn run(
    backend: Backend,
    options: Options,
    checkpoint: Option<tools::Checkpointer>,
    policy: &crate::trust::policy::Policy,
    prompt: &Prompt,
    sink: &EventSink,
) {
    let Some(root) = prompt.project_root.as_deref().filter(|root| !root.trim().is_empty()) else {
        sink.send(EngineEvent::Failed(
            "Agent mode works inside a folder, and this chat has none. Open a folder for it (Files → Open folder), or switch to Chat."
                .to_string(),
        ));

        return;
    };

    let workspace = Workspace::new(root, prompt.remote.clone());
    let target = match target(backend, prompt) {
        Ok(target) => target,
        Err(reason) => {
            sink.send(EngineEvent::Failed(reason));

            return;
        }
    };

    drive(backend, &target, &workspace, options, checkpoint, policy, prompt, sink);
}

/// One model call: the request, the stream, the assembled reply.
fn ask_model(backend: Backend, target: &Target, system: &str, messages: &[Value], specs: &[dialect::ToolSpec], sink: &EventSink, stopped: &dyn Fn() -> bool) -> Result<Reply, String> {
    let mut body = dialect::body(target.dialect, &target.model, system, messages, specs, target.thinking);

    /* Effort (0.14) reaches only the models that take it: OpenAI's reasoning models. Another model sent an
       unknown field may refuse the whole request, and a turn is worth more than the knob. */
    if let (Some(level), true) = (target.effort.as_deref(), reasoning_model(&target.model)) {
        body["reasoning_effort"] = serde_json::json!(if level == "max" { "high" } else { level });
    }

    let body = body.to_string();
    let lines = crate::engines::native_api::open_stream(&target.url, &target.headers, &body).map_err(|reason| unreachable(backend, &reason))?;

    dialect::read_reply(target.dialect, lines, sink, stopped)
}

/// Keeps a turn's conversation inside the model's window (0.13): older tool output is folded when the
/// conversation passes 55% of the window, harder past 75%. Answers whether anything was folded.
fn keep_small(target: &Target, window: u64, messages: &mut [Value]) -> bool {
    let used = dialect::tokens(messages);

    if used * 100 < window * 55 {
        return false;
    }

    let mut folded = dialect::fold_old_results(target.dialect, messages, 6);

    if dialect::tokens(messages) * 100 >= window * 75 {
        folded += dialect::fold_old_results(target.dialect, messages, 2);
    }

    folded > 0
}

/// A sub-agent (`task`, 0.13): its own conversation and read-only tools, reporting into one card of the
/// parent's turn. Answers the report, and the tokens it used.
#[allow(clippy::too_many_arguments)]
fn sub_agent(
    backend: Backend,
    target: &Target,
    workspace: &Workspace,
    policy: &crate::trust::policy::Policy,
    parent: &Prompt,
    vision: bool,
    card: &str,
    job: &str,
    sink: &EventSink,
) -> (tools::Outcome, u64, u64) {
    const SUB_STEPS: usize = 24;

    let stopped = || crate::engines::cancel::requested(&parent.turn_id);
    let system = sub_agent_prompt(workspace);
    let specs = tools::specs_for(tools::Caps { vision, subagent: true, patch: false });
    let window = crate::context::window_tokens("native_api", parent.provider.as_deref(), &parent.model);
    /* What the sub-agent does shows as lines on its card; its words and thinking stay its own. */
    let lines = {
        let sink = sink.clone();
        let card = card.to_string();

        EventSink::new(move |event| {
            if let EngineEvent::ToolStarted { name, target, .. } = event {
                sink.send(EngineEvent::ToolOutput { call_id: card.clone(), level: "dim".to_string(), text: format!("{name} {target}") });
            }
        })
    };
    let mut context = ToolContext {
        workspace,
        session_id: &parent.session_id,
        read_only: true,
        vision,
        sink: &lines,
        turn_id: &parent.turn_id,
        autonomy: Autonomy::Auto,
        always: HashSet::new(),
        calls: 0,
        checkpoint: None,
        checkpointed: true,
        policy,
        changed: HashSet::new(),
        radius_allowed: false,
        plan: None,
        edits: 0,
        ran_at: None,
    };
    let mut messages = vec![dialect::user_message(job)];
    let (mut input, mut output) = (0u64, 0u64);

    for _ in 0..SUB_STEPS {
        if stopped() {
            return (tools::Outcome::error("The turn was stopped."), input, output);
        }

        let reply = match ask_model(backend, target, &system, &messages, &specs, &EventSink::discarding(), &stopped) {
            Ok(reply) => reply,
            Err(reason) => return (tools::Outcome::error(format!("The sub-agent failed: {reason}")), input, output),
        };

        input += reply.input_tokens;
        output += reply.output_tokens;
        messages.push(reply.message.clone());

        if reply.tool_uses.is_empty() {
            let report = if reply.text.trim().is_empty() { "The sub-agent finished without a report.".to_string() } else { reply.text };

            return (tools::Outcome::ok(report), input, output);
        }

        let results: Vec<dialect::ToolResult> = reply
            .tool_uses
            .iter()
            .map(|call| {
                let outcome = tools::execute(&mut context, call);

                (call.id.clone(), outcome.content, outcome.is_error, outcome.image)
            })
            .collect();

        messages.extend(dialect::tool_results(target.dialect, &results));
        keep_small(target, window, &mut messages);
    }

    /* Out of steps: one last call without tools, for whatever it found. */
    dialect::append_user_text(target.dialect, &mut messages, "[SDC: you are out of steps. Write your report now from what you found, without calling tools.]");

    match ask_model(backend, target, &system, &messages, &[], &EventSink::discarding(), &stopped) {
        Ok(reply) => {
            input += reply.input_tokens;
            output += reply.output_tokens;

            (tools::Outcome::ok(reply.text), input, output)
        }
        Err(reason) => (tools::Outcome::error(format!("The sub-agent ran out of steps: {reason}")), input, output),
    }
}

/// The `task` calls of one reply, run at the same time (0.13): each draws its card, runs, and closes it.
#[allow(clippy::too_many_arguments)]
fn run_tasks(
    backend: Backend,
    target: &Target,
    workspace: &Workspace,
    policy: &crate::trust::policy::Policy,
    prompt: &Prompt,
    vision: bool,
    calls: &[(usize, &dialect::ToolUse)],
    sink: &EventSink,
) -> Vec<(usize, tools::Outcome, u64, u64)> {
    std::thread::scope(|scope| {
        let running: Vec<_> = calls
            .iter()
            .map(|(index, call)| {
                let card = format!("{}-task-{}", prompt.turn_id, call.id);
                let description = call.input["description"].as_str().unwrap_or("Research").to_string();
                let job = call.input["prompt"].as_str().unwrap_or_default().to_string();

                sink.send(EngineEvent::ToolStarted { call_id: card.clone(), tool: "read".to_string(), name: "Agent".to_string(), target: description });

                let index = *index;

                scope.spawn(move || {
                    if job.trim().is_empty() {
                        return (index, card, tools::Outcome::error("task needs a prompt: the whole job, as the sub-agent has seen nothing of this conversation."), 0, 0);
                    }

                    let (outcome, input, output) = sub_agent(backend, target, workspace, policy, prompt, vision, &card, &job, sink);

                    (index, card, outcome, input, output)
                })
            })
            .collect();

        running
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .map(|(index, card, outcome, input, output)| {
                sink.send(EngineEvent::ToolCompleted {
                    call_id: card,
                    status: if outcome.is_error { "failed" } else { "done" }.to_string(),
                    meta: format!("{} · {}k tokens", if outcome.is_error { "failed" } else { "reported" }, (input + output) / 1000),
                    diff: None,
                });

                (index, outcome, input, output)
            })
            .collect()
    })
}

/// Whether a changed file is one a build or a test could care about: notes, images and SDC's own
/// memory are not worth a test run.
fn checkable(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();

    !(lowered.starts_with(".sdc/")
        || [".md", ".txt", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".ico", ".pdf", ".log"].iter().any(|extension| lowered.ends_with(extension)))
}

/// The completion gate's check (0.13): the agent changed files and says it is done, but ran nothing since
/// its last change. `Some(note)` names the project's own checks and asks it to run them - unless the
/// person said not to, which the model reads in their words, in whatever language they wrote them.
///
/// SDC used to run the checks itself here. Measured: a person who wrote "test chalanor dorkar nai" (no need
/// to run the tests) had `npm test` run anyway. The agent decides, because it has read the person.
fn completion_note(context: &ToolContext) -> Option<String> {
    if context.ran_at == Some(context.edits) {
        return None;
    }

    let workspace = context.workspace;
    let names: Vec<String> = workspace
        .list(".")
        .map(|(entries, _)| entries.iter().map(|entry| entry.split(" (").next().unwrap_or(entry).trim_end_matches('/').to_string()).collect())
        .unwrap_or_default();
    let posix = workspace.is_remote() || !cfg!(windows);
    let checks = crate::verify::plan(&names, &|name| workspace.read(name).ok().map(|(text, _)| text), posix);

    if checks.is_empty() {
        return None;
    }

    let mut changed: Vec<&String> = context.changed.iter().collect();

    changed.sort();

    Some(format!(
        "[SDC: a note from SDC, not from the person]\n\
         You changed {} and have not run anything since. This project's own checks are: {}.\n\
         Run the ones that cover your change with run_command before you finish - unless the person told you not to \
         run tests or builds, in which case just give your summary. If a check fails because of your change, fix it. \
         If it fails in code you did not touch that was already broken, do NOT fix it - say so in your summary and offer to.",
        changed.iter().map(|path| format!("`{path}`")).collect::<Vec<_>>().join(", "),
        checks.iter().map(|check| format!("`{}`", check.command)).collect::<Vec<_>>().join(", ")
    ))
}

/// The loop over a resolved endpoint - separate from `run` so a test can point it at a loopback
/// server and watch a whole turn: files written, commands run, the answer, the footer.
#[allow(clippy::too_many_arguments)]
fn drive(
    backend: Backend,
    target: &Target,
    workspace: &Workspace,
    options: Options,
    checkpoint: Option<tools::Checkpointer>,
    policy: &crate::trust::policy::Policy,
    prompt: &Prompt,
    sink: &EventSink,
) {
    let turn_id = prompt.turn_id.clone();
    let stopped = || crate::engines::cancel::requested(&turn_id);
    let vision = browser::vision(target.dialect == Dialect::Anthropic, &target.model);
    /* The project's skills (0.13): their names and when to use them; the agent reads one when it applies. */
    let system = system_prompt(workspace, vision, &person_language(prompt)) + &skills::brief(&skills::find(workspace));
    let window = crate::context::window_tokens(if backend == Backend::Ollama { "ollama" } else { "native_api" }, prompt.provider.as_deref(), &target.model);
    /* The project's own MCP servers (0.12; on a host too since 0.13): their tools join the agent's for this turn. */
    let (mut mcp, warnings) = match workspace.remote() {
        /* On a host, the servers run there, over the same ssh (0.13). */
        Some(ssh) => mcp::McpTools::start_remote(workspace.root(), ssh),
        None => mcp::McpTools::start(std::path::Path::new(workspace.root())),
    };

    for warning in warnings {
        sink.send(EngineEvent::Thinking(format!("MCP: {warning}\n")));
    }

    let mut specs = tools::specs_for(tools::Caps { vision, subagent: false, patch: patch::speaks_patch(&target.model) });

    if let Some(servers) = &mcp {
        specs.extend(servers.specs().iter().cloned());
    }
    let mut messages: Vec<Value> = prompt
        .history
        .iter()
        .map(|message| match message.role {
            Role::User => dialect::user_message(&message.text),
            Role::Assistant => dialect::assistant_message(&message.text),
        })
        .collect();

    /* The images the person attached (0.13): shown to a model that can see, and said out loud to one that cannot. */
    let images: Vec<(String, String)> = if vision {
        prompt.images.iter().filter_map(|image| image.base64().map(|data| (image.media_type.clone(), data))).collect()
    } else {
        Vec::new()
    };
    let text = if !vision && !prompt.images.is_empty() {
        format!(
            "{}\n\n[SDC: the person attached {} image(s), but this model cannot see images. If they matter, say so and suggest a model that can.]",
            prompt.text,
            prompt.images.len()
        )
    } else {
        prompt.text.clone()
    };

    messages.push(dialect::user_message_with_images(target.dialect, &text, &images));

    let mut context = ToolContext {
        workspace,
        session_id: &prompt.session_id,
        read_only: false,
        vision,
        sink,
        turn_id: &turn_id,
        autonomy: options.autonomy,
        always: HashSet::new(),
        calls: 0,
        checkpoint,
        checkpointed: false,
        policy,
        changed: HashSet::new(),
        radius_allowed: false,
        plan: None,
        edits: 0,
        ran_at: None,
    };
    let (mut input_tokens, mut output_tokens) = (0u64, 0u64);
    let mut said_something = false;
    let mut plan_nudged = false;
    let mut check_rounds = 0;
    /* The edit count when the checks last ran: they run again only after the agent changed something. */
    let mut checked_at = usize::MAX;
    /* Words sent while the turn runs join it between steps (0.12.5) - closed when the loop returns. */
    let inbox = crate::engines::steer::open(&turn_id);
    let steer = |messages: &mut Vec<Value>| -> bool {
        let texts = inbox.take();

        if texts.is_empty() {
            return false;
        }

        for text in &texts {
            sink.send(EngineEvent::Steered(text.clone()));
        }

        dialect::append_user_text(target.dialect, messages, &crate::engines::steer::as_message(&texts));

        true
    };

    for step in 1..=options.max_steps {
        if stopped() {
            return;
        }

        let reply: Reply = match ask_model(backend, target, &system, &messages, &specs, &step_sink(sink, said_something), &stopped) {
            Ok(reply) => reply,
            Err(reason) if reason == "stopped" => return,
            Err(reason) => {
                sink.send(EngineEvent::Failed(reason));

                return;
            }
        };

        input_tokens += reply.input_tokens;
        output_tokens += reply.output_tokens;

        /* Totals so far, every step: the cost governor can stop a turn that crosses its budget mid-way. */
        sink.send(EngineEvent::Usage { input_tokens, output_tokens, cost_usd: None });
        said_something = said_something || !reply.text.trim().is_empty();

        if reply.stop == "refusal" {
            sink.send(EngineEvent::Failed(
                "The model declined to continue this request (a safety refusal). Rephrase the task, or try another model.".to_string(),
            ));

            return;
        }

        messages.push(reply.message.clone());

        /* About to finish, but the person said something meanwhile: that is the next thing to do. */
        if reply.tool_uses.is_empty() && steer(&mut messages) {
            continue;
        }

        if reply.tool_uses.is_empty() && reply.stop != "max_tokens" && reply.stop != "length" {
            /*
             * The completion gate (0.13). An agent that says "done" with steps of its own plan still open is
             * reminded once; one that changed files and ran nothing since is told the project's checks and
             * asked to run them - unless the person said not to - at most twice, and only after new changes.
             */
            if !plan_nudged {
                if let Some(open) = context.open_plan_steps() {
                    plan_nudged = true;
                    dialect::append_user_text(
                        target.dialect,
                        &mut messages,
                        &format!("[SDC: your plan still has open steps:\n{open}\nFinish them, or call update_plan to mark the ones no longer needed - then give your summary.]"),
                    );

                    continue;
                }
            }

            if options.auto_check && check_rounds < 2 && checked_at != context.edits && context.changed.iter().any(|path| checkable(path)) && !stopped() {
                check_rounds += 1;
                checked_at = context.edits;

                if let Some(note) = completion_note(&context) {
                    dialect::append_user_text(target.dialect, &mut messages, &note);

                    continue;
                }
            }
        }

        if reply.tool_uses.is_empty() {
            let summary = if reply.stop == "max_tokens" || reply.stop == "length" {
                "Cut off: the answer reached the model's length limit"
            } else {
                "Done"
            };

            sink.send(EngineEvent::Done {
                summary: summary.to_string(),
                meta: meta(step, input_tokens, output_tokens, target.price),
                pass: None,
            });

            return;
        }

        /* The step's calls, in order - except `task`, whose sub-agents run all at once. */
        let mut results: Vec<Option<dialect::ToolResult>> = vec![None; reply.tool_uses.len()];
        let tasks: Vec<(usize, &dialect::ToolUse)> = reply.tool_uses.iter().enumerate().filter(|(_, call)| call.name == "task").collect();

        for (index, call) in reply.tool_uses.iter().enumerate() {
            if stopped() {
                return;
            }

            if call.name == "task" {
                continue;
            }

            let outcome = match mcp.as_mut() {
                Some(servers) if servers.handles(&call.name) => tools::mcp_call(&mut context, servers, call),
                _ => tools::execute(&mut context, call),
            };

            results[index] = Some((call.id.clone(), outcome.content, outcome.is_error, outcome.image));
        }

        if !tasks.is_empty() {
            for (index, outcome, input, output) in run_tasks(backend, target, workspace, policy, prompt, vision, &tasks, sink) {
                input_tokens += input;
                output_tokens += output;
                results[index] = Some((reply.tool_uses[index].id.clone(), outcome.content, outcome.is_error, outcome.image));
            }

            sink.send(EngineEvent::Usage { input_tokens, output_tokens, cost_usd: None });
        }

        let results: Vec<dialect::ToolResult> = results
            .into_iter()
            .enumerate()
            .map(|(index, result)| result.unwrap_or_else(|| (reply.tool_uses[index].id.clone(), "The turn was stopped.".to_string(), true, None)))
            .collect();

        messages.extend(dialect::tool_results(target.dialect, &results));
        steer(&mut messages);

        let folded = keep_small(target, window, &mut messages);

        sink.send(EngineEvent::Context { used_tokens: dialect::tokens(&messages), window_tokens: window, compacted: folded });
    }

    /* The step budget ran out with work still going on. That is said, with the way to continue, rather
       than dressed up as a finished turn. */
    sink.send(EngineEvent::Delta(format!(
        "\n\nI stopped after {} steps, the limit for one turn. Send \"continue\" to let me keep going from here.",
        options.max_steps
    )));
    sink.send(EngineEvent::Done {
        summary: "Paused at the step limit".to_string(),
        meta: meta(options.max_steps, input_tokens, output_tokens, target.price),
        pass: None,
    });
}

/// The sentence for a request that never reached a model.
fn unreachable(backend: Backend, reason: &str) -> String {
    match backend {
        Backend::Ollama if reason.contains("refused") || reason.contains("11434") => format!(
            "Ollama is not answering on this machine ({reason}). Start it with `ollama serve`, then send the prompt again."
        ),
        _ => reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalogue_price_is_read_as_two_numbers() {
        assert_eq!(parse_price("$4 / $20"), Some((4.0, 20.0)));
        assert_eq!(parse_price("$0.14 / $0.28"), Some((0.14, 0.28)));
        assert_eq!(parse_price("free"), None);
        assert_eq!(parse_price("subscription"), None);
        assert_eq!(price_of("anthropic-api", "claude-opus-5-5"), Some((4.0, 20.0)));
    }

    #[test]
    fn the_footer_says_what_the_turn_used_and_what_it_cost() {
        assert_eq!(meta(1, 900, 40, None), "1 step · 900 in · 40 out");
        assert_eq!(meta(7, 12_400, 3_100, Some((4.0, 20.0))), "7 steps · 12.4k in · 3.1k out · ≈$0.11");
        assert_eq!(meta(8, 17_500, 886, Some((0.14, 0.28))), "8 steps · 17.5k in · 886 out · ≈$0.0027");
    }

    #[test]
    fn a_chat_without_a_folder_is_told_what_to_do() {
        let recorder = crate::engines::Recorder::new();
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-nofolder".into(),
            text: "build it".into(),
            model: "llama3.2:3b".into(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
                    autonomy: Default::default(),
                    resume: None,
                    images: Vec::new(),
                    effort: None,
        };

        run(Backend::Ollama, Options { autonomy: Autonomy::Ask, max_steps: 5, auto_check: false }, None, &Default::default(), &prompt, &recorder.sink());

        assert!(matches!(&recorder.events()[..], [EngineEvent::Failed(reason)] if reason.contains("Open a folder")));
    }

    #[test]
    fn steps_are_separated_only_when_the_new_step_says_something() {
        let recorder = crate::engines::Recorder::new();
        let sink = step_sink(&recorder.sink(), true);

        sink.send(EngineEvent::Delta(" ".into()));
        sink.send(EngineEvent::Delta("Fixed it.".into()));
        sink.send(EngineEvent::Delta(" Done.".into()));

        assert_eq!(
            recorder.events(),
            vec![
                EngineEvent::Delta(" ".into()),
                EngineEvent::Delta("\n\n".into()),
                EngineEvent::Delta("Fixed it.".into()),
                EngineEvent::Delta(" Done.".into()),
            ]
        );
    }
}

/// A whole turn against a scripted provider on loopback: the agent must write a real file, run a real
/// command in the folder, feed both results back, and finish with the model's summary.
#[cfg(test)]
mod end_to_end {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// One SSE reply per request, in order. Each request's body is kept so the test can read what the
    /// agent sent back.
    fn provider(replies: Vec<String>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut bodies = Vec::new();

            for reply in replies {
                let (socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut length = 0usize;

                loop {
                    let mut line = String::new();

                    reader.read_line(&mut line).unwrap();

                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }

                    if line == "\r\n" {
                        break;
                    }
                }

                let mut body = vec![0u8; length];

                reader.read_exact(&mut body).unwrap();
                bodies.push(serde_json::from_slice(&body).unwrap());

                let mut socket = socket;

                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{reply}").unwrap();
            }

            bodies
        });

        (url, server)
    }

    fn chunk(delta: Value) -> String {
        format!("data: {}\n\n", serde_json::json!({ "choices": [{ "delta": delta }] }))
    }

    fn call(id: &str, name: &str, arguments: Value) -> String {
        chunk(serde_json::json!({ "tool_calls": [{ "index": 0, "id": id, "function": { "name": name, "arguments": arguments.to_string() } }] }))
            + "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n"
            + "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20}}\n\ndata: [DONE]\n\n"
    }

    #[test]
    fn the_agent_writes_runs_reads_the_results_and_finishes() {
        let root = std::env::temp_dir().join(format!("sdc-agent-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let listing = if cfg!(windows) { "type hello.txt" } else { "cat hello.txt" };
        let (url, server) = provider(vec![
            call("c1", "write_file", serde_json::json!({ "path": "hello.txt", "content": "made by the agent\n" })),
            call("c2", "run_command", serde_json::json!({ "command": listing })),
            chunk(serde_json::json!({ "content": "Created hello.txt and checked it." })) + "data: [DONE]\n\n",
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-e2e".into(),
            text: "make hello.txt".into(),
            model: "local-test".into(),
            provider: None,
            history: vec![crate::engines::Message::user("earlier"), crate::engines::Message::assistant("ok")],
            project_root: Some(root.to_str().unwrap().to_string()),
            remote: None,
                    autonomy: Default::default(),
                    resume: None,
                    images: Vec::new(),
                    effort: None,
        };
        let recorder = crate::engines::Recorder::new();

        /* The checkpoint must see the folder as it was before the write - the race this test pins. */
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, bool)>::new()));
        let probe = seen.clone();
        let file = root.join("hello.txt");
        let checkpoint: tools::Checkpointer = std::sync::Arc::new(move |title: &str| {
            probe.lock().unwrap().push((title.to_string(), file.exists()));
        });

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 10, auto_check: false }, Some(checkpoint), &Default::default(), &prompt, &recorder.sink());

        assert_eq!(
            *seen.lock().unwrap(),
            vec![("Before Create hello.txt".to_string(), false)],
            "one checkpoint, taken before the file existed"
        );

        let bodies = server.join().unwrap();
        let events = recorder.events();

        /* The file is real, and the command that read it back saw what the agent wrote. */
        assert_eq!(std::fs::read_to_string(root.join("hello.txt")).unwrap(), "made by the agent\n");
        assert!(bodies[2]["messages"].as_array().unwrap().iter().any(|message| {
            message["role"] == "tool" && message["content"].as_str().unwrap_or_default().contains("made by the agent")
        }));

        /* The conversation carried the history with its roles, the system prompt first. */
        assert_eq!(bodies[0]["messages"][0]["role"], "system");
        assert_eq!(bodies[0]["messages"][2]["role"], "assistant");
        assert_eq!(bodies[0]["tools"].as_array().unwrap().len(), tools::specs().len());

        /* The window saw two cards (Create, Run), the answer, and a footer with what the turn used. */
        let cards: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                EngineEvent::ToolStarted { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(cards, ["Create", "Run"]);
        assert!(events.contains(&EngineEvent::Delta("Created hello.txt and checked it.".into())));
        assert!(matches!(events.last(), Some(EngineEvent::Done { meta, .. }) if meta.starts_with("3 steps · 200 in · 40 out")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_step_limit_pauses_the_turn_and_says_how_to_continue() {
        let root = std::env::temp_dir().join(format!("sdc-agent-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let (url, server) = provider(vec![
            call("c1", "list_dir", serde_json::json!({ "path": "." })),
            call("c2", "list_dir", serde_json::json!({ "path": "." })),
        ]);
        let target = Target {
            url,
            headers: Vec::new(),
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-limit".into(),
            text: "look around".into(),
            model: "local-test".into(),
            provider: None,
            history: Vec::new(),
            project_root: Some(root.to_str().unwrap().to_string()),
            remote: None,
                    autonomy: Default::default(),
                    resume: None,
                    images: Vec::new(),
                    effort: None,
        };
        let recorder = crate::engines::Recorder::new();

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Ask, max_steps: 2, auto_check: false }, None, &Default::default(), &prompt, &recorder.sink());
        server.join().unwrap();

        assert!(matches!(recorder.events().last(), Some(EngineEvent::Done { summary, .. }) if summary == "Paused at the step limit"));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 0.15.2: a turn with no `maxSteps` has no step limit ("kono rate limit to dorkar nai"), and one
    /// that names a large bound gets it rather than a hidden ceiling.
    #[test]
    fn a_turn_has_no_step_limit_unless_the_caller_names_one() {
        assert_eq!(SdcAgent::new(Backend::Api, Autonomy::Auto, DEFAULT_STEPS).max_steps, usize::MAX);
        assert_eq!(SdcAgent::new(Backend::Api, Autonomy::Auto, 500).max_steps, 500);
        assert_eq!(SdcAgent::new(Backend::Api, Autonomy::Auto, 0).max_steps, 1);
    }
}
