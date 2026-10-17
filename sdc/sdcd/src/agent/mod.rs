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
pub mod ollama_native;
pub mod orient;
pub mod patch;
pub mod research;
pub mod search;
pub mod skills;
pub mod tools;
pub mod web;
pub mod workspace;

use std::collections::{HashMap, HashSet};

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

/// How often in a row a step is asked again after the provider's content filter blocked its reply.
const FILTER_RETRIES: usize = 3;

/// What the model is told after a blocked reply.
const FILTER_NOTE: &str = "[SDC: the provider's content filter blocked your last reply before anyone saw it. Write it again, shorter: summarise logs, command output, error text and quoted documents in your own words instead of copying them, and go on with the task.]";

/// Is this failure a provider's output filter rather than the network or the request? `reason` is
/// lowercase. Alibaba: `DataInspectionFailed` / `data_inspection_failed`; OpenAI-style: `content_filter`.
pub fn content_filtered(reason: &str) -> bool {
    ["datainspectionfailed", "data_inspection_failed", "may contain inappropriate content", "content_filter", "content management policy"]
        .iter()
        .any(|needle| reason.contains(needle))
}

/// Where the model for this turn lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// A provider behind an API key (`native_api`'s endpoints).
    Api,
    /// The local Ollama daemon, through its own `/api/chat` (0.16.1; `/v1` until then - it cannot
    /// carry the context size, see `ollama_native`).
    Ollama,
}

pub struct SdcAgent {
    backend: Backend,
    autonomy: Autonomy,
    max_steps: usize,
    auto_check: bool,
    /// A `/research` turn's limits (0.16.1) - `None` for every other turn.
    research: Option<research::Limits>,
    checkpoint: Option<tools::Checkpointer>,
    /// The folder's policy (0.12): what the tools must ask about or refuse before they act.
    policy: crate::trust::policy::Policy,
}

impl SdcAgent {
    pub fn new(backend: Backend, autonomy: Autonomy, max_steps: usize) -> Self {
        Self { backend, autonomy, max_steps: max_steps.max(1), auto_check: true, research: None, checkpoint: None, policy: Default::default() }
    }

    /// Makes this a `/research` turn: the research brief and tools, these limits, and sources at the end.
    ///
    /// v3 (owner's decision, 2026-10-14): no step cap of its own. The turn ends when the model answers, or
    /// when its research time is up and it is asked for the answer without tools.
    pub fn with_research(mut self, limits: research::Limits) -> Self {
        self.research = Some(limits);
        self
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
        let options = Options { autonomy: self.autonomy, max_steps: self.max_steps, auto_check: self.auto_check, research: self.research };
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
#[derive(Clone)]
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
    /// A local model's context, sent as `num_ctx` to Ollama's own endpoint - `None` for an API.
    local_context: Option<u64>,
    /// A local OpenAI-compatible server (0.22): small window, results cut to fit it.
    local_window: bool,
}

impl Target {
    /// The window the conversation is planned for: what Ollama was asked to load, or the model's own.
    fn window(&self, provider: Option<&str>) -> u64 {
        self.local_context
            .unwrap_or_else(|| crate::context::window_tokens("native_api", provider, &self.model))
    }
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
                url: ollama_native::URL.to_string(),
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                dialect: Dialect::OpenAi,
                local_context: Some(crate::engines::ollama::context_for(&model)),
                local_window: true,
                model,
                price: None,
                thinking: false,
                effort: None,
            })
        }
        Backend::Api => {
            let endpoint = crate::engines::native_api::endpoint_for(&prompt.model, prompt.provider.as_deref());
            let key = crate::auth::keychain::get(&endpoint.key_ref).unwrap_or_default();

            /* A local model server (0.22) has no key unless it was started with one. */
            if key.is_empty() && !crate::providers::local::is_local(&endpoint.provider) {
                return Err(crate::auth::keychain::missing_key_reason(&endpoint.key_ref, &endpoint.provider));
            }

            let local_window = crate::providers::local::is_local(&endpoint.provider);
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
                if !key.is_empty() {
                    headers.push(("authorization".to_string(), format!("Bearer {key}")));
                }

                Dialect::OpenAi
            };

            Ok(Target {
                url: endpoint.url,
                headers,
                dialect,
                thinking: dialect == Dialect::Anthropic && crate::engines::native_api::adaptive_thinking(&model),
                price: price_of(&endpoint.provider, &model),
                effort: prompt.effort.clone(),
                local_context: None,
                local_window,
                model,
            })
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Fake targets a test points at its own providers, by provider id (the failover, the router and the cascade).
    static FAKE_TARGETS: std::cell::RefCell<HashMap<String, Target>> = std::cell::RefCell::new(HashMap::new());
}

/// The endpoint and key of a provider/model, resolved from the person's settings (used by the router, the cascade
/// and the failover - all of which run inside `drive`, where `target` is already the turn's own Target).
fn resolve_target(prompt: &Prompt) -> Result<Target, String> {
    #[cfg(test)]
    if let Some(fake) = FAKE_TARGETS.with(|targets| targets.borrow().get(prompt.provider.as_deref().unwrap_or_default()).cloned()) {
        return Ok(fake);
    }

    target(Backend::Api, prompt)
}

/// The target a failover continues on (I9): the provider's own endpoint and key - or, in a test, its fake.
fn failover_target(prompt: &Prompt) -> Result<Target, String> {
    resolve_target(prompt)
}

/// v3 W1 (A-14): the provider's own input-token count for one short message - no tools, a one-line system
/// prompt. The bench subtracts a one-character message's count to get the text's own tokens.
pub(crate) fn probe_input_tokens(provider: &str, model: &str, text: &str) -> Result<u64, String> {
    let prompt = Prompt {
        session_id: String::new(),
        turn_id: String::new(),
        text: text.to_string(),
        model: model.to_string(),
        provider: Some(provider.to_string()),
        history: Vec::new(),
        project_root: None,
        remote: None,
        autonomy: Default::default(),
        resume: None,
        images: Vec::new(),
        effort: None,
    };
    let target = target(Backend::Api, &prompt)?;
    let messages = [dialect::user_message(text)];

    ask_model(Backend::Api, &target, ("", crate::guard::modelgate::Purpose::Main), "Reply with the single word OK.", &messages, &[], &EventSink::discarding(), &|| false).map(|reply| reply.input_tokens)
}

/// The catalogue's `"$4 / $20"` for a model, as numbers.
pub(crate) fn price_of(provider: &str, model: &str) -> Option<(f64, f64)> {
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

fn system_prompt(workspace: &Workspace, vision: bool, language: &crate::understand::Reading, stable_prefix: bool) -> String {
    /* v3 W5: with `stable_prefix` the date leaves the cached prefix and travels in the turn's own message. */
    let today = if stable_prefix { String::new() } else { format!("Today: {}\n", chrono::Local::now().format("%Y-%m-%d")) };

    let look = if vision {
        "\n- For a web page or UI you built, start its dev server with start_process and try it in the browser tool as a user would: open it, read it, click the buttons, fill the forms, and take a screenshot to check it looks right - on a phone width (390) too. Fix what does not work."
    } else {
        "\n- For a web page or UI you built, start its dev server with start_process and try it in the browser tool as a user would: open it, read it, click the buttons and fill the forms. Fix what does not work."
    };

    format!(
        "You are SDC Agent, a coding agent working inside a person's project through the tools you are given.\n\
         \n\
         {today}Project folder: {root}\n\
         Machine: {place}\n\
         Shell for run_command: {shell}\n\
         \n\
         Match your effort to the request - this is how you stay fast:\n\
         - A question, or a small change: answer it or do it directly, in as few steps as possible. No plan, no narration, no self-review. The shortest correct answer comes first.\n\
         - A change across several files, or a bug whose cause is not obvious: understand first, then change, then verify.\n\
         - A large task (many parts, a refactor, a feature): plan it with update_plan, work through it, verify, review.\n\
         - A message may start with an [SDC pace: ...] line, or a map of the project: follow the first, and use the second instead of listing the folder again.\n\
         \n\
         How to think:\n\
         - Know what \"done\" means before you start. If a message hides several requests, do every one.\n\
         - Ground every claim in this project: read the code, the config, the error. Never assume an API, a file, a flag or a version exists - check it.\n\
         - For a bug, find the root cause, not the symptom: reproduce it, form a hypothesis, confirm it in the code, fix the cause, and say what it was.\n\
         - The smallest change that fully solves it, in the project's own style. Handle the edge cases that matter (empty, missing, huge, wrong input). No secrets, dead code or debugging leftovers; a UI works at phone width.\n\
         \n\
         How to work:\n\
         - Put independent tool calls in ONE reply: reads, greps and globs run together, so asking for four files at once costs one step, not four. Never read one file per reply when you already know you need several.\n\
         - Find with grep or glob, then read only the lines you need (offset and limit). Do not read a large file whole, and do not read again what you have already read this turn. Stop looking as soon as you can answer.\n\
         - For broad exploration or research, hand self-contained jobs to task sub-agents - several in one reply run in parallel - instead of reading everything yourself.\n\
         - For anything with more than two steps, call update_plan first; the person watches that checklist. Call it again each time a step starts or finishes, and mark every step done before your final answer.\n\
         - Say what you are doing in one short line when you start a larger task or change direction - not before every call.\n\
         - Change files with edit_file (exact text replacement); use write_file for new files or full rewrites. Match the project's existing style.\n\
         - Verify what you change: build it, run the tests or the program with run_command, read the output, fix what fails. Add or update a test when you fix a bug or add behaviour and the project has tests.\n\
         - For a change across files SDC may show you your own diff before you finish. Re-read it as a strict reviewer: fix a real problem, otherwise give your summary.\n\
         - Be honest: never say a test passes or a thing works unless you ran it and saw it. If something still fails, or you could not verify it, say so.\n\
         - A command that does not exit on its own (a dev server, a watcher) goes in start_process, never in run_command. Stop what you started when you no longer need it, unless the person will want it running.{look}\n\
         - When you are unsure how a library, framework or API works, or the person asks about anything current (news, prices, versions, weather), look it up with web_search and web_fetch instead of guessing, and give the source address.\n\
         - When a decision belongs to the person (a design choice, deleting data, two readings of the request), ask with ask_user and offer options. Do not ask about what you can find out yourself.\n\
         - When the person states a lasting preference or a project convention, keep it with remember.\n\
         - Stay inside the project folder. Secrets (.env, keys) are hidden from you on purpose; do not try to read them.\n\
         - If the person declines an action, do not try it another way; explain what you wanted to do.\n\
         - Keep going until the task is completely done; do not stop half-way to ask whether to continue. When you are done, stop calling tools and answer. A question gets its answer, directly. After work, a short Markdown summary: the result in one line, then what changed (with paths), the root cause for a fix, how you verified it (commands and outcome), and anything the person must do. No filler, and do not repeat the question.\n\
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
         with file paths and line numbers, and the answer to the question. Keep what you saw apart from what you infer, \
         and say how sure you are. You cannot change anything; if something \
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
    /// A `/research` turn's limits (0.16.1).
    pub research: Option<research::Limits>,
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

    /* v3 R4: when the person names a VPS for this project, this turn's tools run there instead of here.
       The machine is asked whether it is there first: a turn that starts against a host that is off, or a key that no
       longer fits, would otherwise fail later and somewhere less useful - in the middle of a tool call. */
    let prompt = if prompt.remote.is_none() && crate::v3::on("runner") {
        match crate::v3::runner::remote_for(std::path::Path::new(root), crate::v3::runner::default_name().as_deref()) {
            Ok(Some(ssh)) => match crate::v3::runner::reachable(&ssh) {
                Ok(None) => {
                    let mut owned = prompt.clone();

                    sink.send(EngineEvent::Thinking(format!("This turn runs on {}, as `.sdc/runner` asks.\n", ssh.target.user_host)));
                    owned.remote = Some(ssh);
                    owned
                }
                Ok(Some(why)) => {
                    sink.send(EngineEvent::Thinking(format!("{} cannot be reached ({why}) - this turn stays on this machine.\n", ssh.target.user_host)));

                    prompt.clone()
                }
                Err(problem) => {
                    sink.send(EngineEvent::Thinking(format!("{problem} - this turn stays on this machine.\n")));

                    prompt.clone()
                }
            },
            Ok(None) => prompt.clone(),
            Err(problem) => {
                sink.send(EngineEvent::Thinking(format!("{problem} - this turn stays on this machine.\n")));

                prompt.clone()
            }
        }
    } else {
        prompt.clone()
    };

    let workspace = Workspace::new(root, prompt.remote.clone());

    /* P0-14: a step transaction a crash left half-committed is undone before anything else touches the project. */
    if prompt.remote.is_none() && crate::v3::on("edit.transaction") {
        for (id, what) in crate::runtime::transaction::recover(std::path::Path::new(root)) {
            sink.send(EngineEvent::Delta(format!("_(SDC finished a step that was interrupted: {id} - {what:?})_\n\n")));
        }
    }

    /* 0.21: a local model's turn starts Ollama itself when it is installed and stopped. */
    if backend == Backend::Ollama {
        crate::host::tools::ensure_ollama();
    }

    let target = match target(backend, &prompt) {
        Ok(target) => target,
        Err(reason) => {
            sink.send(EngineEvent::Failed(reason));

            return;
        }
    };

    drive(backend, &target, &workspace, options, checkpoint, policy, &prompt, sink);

    /* P0-9: a local model nobody has measured yet is measured once, after its first turn, so the turn never waits for it. */
    if backend == Backend::Ollama {
        crate::sme::probe::after_use(&target.model);
    }
}

/// A stream that remembers when its first byte arrived (P0-12: the time to first token of a model call).
struct FirstByte {
    inner: Box<dyn std::io::BufRead + Send>,
    started: std::time::Instant,
    first: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl std::io::Read for FirstByte {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;

        self.note(read);

        Ok(read)
    }
}

impl std::io::BufRead for FirstByte {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        let started = self.started;
        let first = self.first.clone();
        let filled = self.inner.fill_buf()?;

        if !filled.is_empty() {
            let _ = first.compare_exchange(0, started.elapsed().as_millis().max(1) as u64, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst);
        }

        Ok(filled)
    }

    fn consume(&mut self, amount: usize) {
        self.inner.consume(amount);
    }
}

impl FirstByte {
    fn note(&self, read: usize) {
        if read > 0 {
            let _ = self.first.compare_exchange(0, self.started.elapsed().as_millis().max(1) as u64, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// About how many tokens of reasoning a reply carried: its thinking blocks, or `reasoning_content`, by the daemon's one
/// estimate. An estimate, labelled as one in the trace.
fn thinking_tokens_of(message: &Value) -> u64 {
    let mut text = String::new();

    for key in ["reasoning_content", "reasoning"] {
        if let Some(part) = message[key].as_str() {
            text.push_str(part);
        }
    }

    for block in message["content"].as_array().into_iter().flatten() {
        if block["type"] == "thinking" {
            text.push_str(block["thinking"].as_str().unwrap_or_default());
        }
    }

    if text.is_empty() {
        0
    } else {
        crate::tokens::estimate(&text)
    }
}

/// One model call: the request, the stream, the assembled reply.
#[allow(clippy::too_many_arguments)]
fn ask_model(backend: Backend, target: &Target, turn: (&str, crate::guard::modelgate::Purpose), system: &str, messages: &[Value], specs: &[dialect::ToolSpec], sink: &EventSink, stopped: &dyn Fn() -> bool) -> Result<Reply, String> {
    /* P0-10, the Pin Guard: every model call of a turn asks the turn's gate first. A call to a model the person did not
       choose never reaches the network. */
    crate::guard::modelgate::of(turn.0).permit(turn.1, None, &target.model).map_err(|violation| violation.to_string())?;

    let mut body = dialect::body(target.dialect, &target.model, system, messages, specs, target.thinking);

    /* Effort (0.14) reaches only the models that take it: OpenAI's reasoning models. Another model sent an
       unknown field may refuse the whole request, and a turn is worth more than the knob. */
    if let (Some(level), true) = (target.effort.as_deref(), reasoning_model(&target.model)) {
        body["reasoning_effort"] = serde_json::json!(if level == "max" { "high" } else { level });
    }

    /* P0-8, the Local Runtime Guard: the request is counted before it is sent. A prompt that does not fit the window
       is never sent - Ollama would answer 200 and drop the start of it, a cloud API would refuse it - the caller gets
       `ContextOverflow` and splits the work. A local model's window is the `num_ctx` it is asked for; a cloud
       model's is the one its catalogue row documents, and a model with none documented is not judged. */
    let window = target.local_context.or_else(|| crate::guard::context::documented_window(&target.model));
    let mut expected_tokens = None;

    if let (Some(window), true) = (window, crate::v3::on("local.runtime_guard")) {
        let reserve = crate::guard::context::max_output(&body, window);

        match crate::guard::context::check(&body, window, reserve) {
            Ok(estimated) => expected_tokens = Some((estimated, window)),
            Err(overflow) => return Err(overflow.to_string()),
        }
    }

    /* A local model gets Ollama's own request, with the context size in it (0.16.1). */
    if let Some(num_ctx) = target.local_context {
        body = ollama_native::body(body, num_ctx);
    }

    /* P0-11: a bench run states its seed (and temperature) on every request; outside a bench this changes nothing. */
    crate::bench::rules::apply(&mut body, target.local_context.is_some(), target.thinking);

    /* 1.0: on Model Studio a request is sent with what makes a step fast, in three levels, and a level the
       endpoint refuses (a 400) is stepped down from - and not sent to that endpoint and model again:
         2  the cache marks (system prompt and the newest message) + level 1,
         1  `parallel_tool_calls` (Model Studio's default is one call per reply: measured on the owner's turns,
            60% of 1,181 steps carried a single call) and the effort the turn asked for (`reasoning_effort`),
         0  the plain request.
       Anywhere else the request is level 0, as it always was. */
    let levels: Vec<String> = if target.dialect == Dialect::OpenAi && target.local_context.is_none() && model_studio(&target.url) {
        let fast = fast_body(&body, target);
        let cap = level_cap(&target.url, &target.model);

        [dialect::mark_cache(&fast).to_string(), fast.to_string(), body.to_string()].into_iter().skip(2 - cap.min(2)).collect()
    } else {
        vec![body.to_string()]
    };
    /* A step the network or the provider dropped - before its first word or halfway through - is asked
       again rather than ending the turn (0.15.4): nothing of it reached the conversation yet, so asking
       again is the same question, and every step before it is kept. */
    let mut card = crate::engines::native_api::RetryCard::new(sink, "agent", &target.url);
    let first_byte = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let send = |body: &str, card: &mut crate::engines::native_api::RetryCard| {
        crate::engines::native_api::with_retries(
            stopped,
            |retry, wait, reason| card.notice(retry, wait, reason),
            || {
                let sent_at = std::time::Instant::now();
                /* A local model is read with patience (a socket that wakes, a limit on silence); an API as before. */
                let quiet = |seconds: u64| {
                    /* Not worded like a dropped connection: asking a stuck server again and again is what held the turn. */
                    format!(
                        "Ollama stopped answering `{}` (nothing arrived for {seconds} s). It may be stuck loading the model or out of graphics memory: quit Ollama and open it again, or pick a smaller model; then send \"continue\".",
                        target.model
                    )
                };
                let (lines, silent) = if target.local_context.is_some() {
                    crate::engines::native_api::open_stream_patient(&target.url, &target.headers, body, turn.0)
                        .map(|(lines, silent)| (lines, Some(silent)))
                        .map_err(|reason| if reason == "silent" { quiet(crate::engines::native_api::LOCAL_SILENCE_FIRST.as_secs()) } else { reason })?
                } else {
                    (crate::engines::native_api::open_stream(&target.url, &target.headers, body)?, None)
                };
                let timed: Box<dyn std::io::BufRead + Send> = Box::new(FirstByte { inner: lines, started: sent_at, first: first_byte.clone() });
                let lines = if target.local_context.is_some() { ollama_native::as_sse(timed) } else { timed };

                dialect::read_reply(target.dialect, lines, sink, stopped).map_err(|reason| match silent {
                    Some(silent) if silent.load(std::sync::atomic::Ordering::SeqCst) => quiet(crate::engines::native_api::LOCAL_SILENCE_AFTER.as_secs()),
                    _ => reason,
                })
            },
        )
    };
    let mut reply = Err("no request was sent".to_string());

    /* v3 W9: requests to one provider are paced (a token bucket), instead of setting off its rate limit. */
    crate::v3::rate::acquire(&target.url);

    for (index, request) in levels.iter().enumerate() {
        reply = send(request, &mut card);

        let refused = matches!(&reply, Err(reason) if reason.contains("(400)") || reason.to_lowercase().contains("cache_control") || reason.to_lowercase().contains("parallel_tool_calls") || reason.to_lowercase().contains("reasoning_effort"));

        if !refused || index + 1 == levels.len() {
            /* A lower level went through after a higher one was refused: the higher one was the problem. */
            if reply.is_ok() && index > 0 {
                lower_cap(&target.url, &target.model, levels.len() - 1 - index);
            }

            break;
        }
    }

    card.close(reply.is_ok());

    /* P0-12: the time to the first byte of the call that answered, and the reasoning it carried. */
    if let Ok(done) = reply.as_mut() {
        done.ttft_ms = first_byte.load(std::sync::atomic::Ordering::SeqCst);
        done.thinking_tokens = thinking_tokens_of(&done.message);
    }

    /* P0-8: what the runtime says it read is set against what was sent. A count that filled the window means the start
       of the prompt may be gone, and an answer to a cut prompt is not trusted. (A count below ours proves nothing: the
       runtime may have reused a cached prefix.) */
    if let (Ok(done), Some((estimated, window)), Some(_)) = (&reply, expected_tokens, target.local_context) {
        use crate::guard::context::{judge, Judgement};

        match judge(estimated, done.input_tokens, window) {
            Judgement::FilledWindow => {
                return Err(format!("the runtime read {} tokens against a window of {window}: the start of the prompt may have been cut, so this answer is not trusted", done.input_tokens));
            }
            /* The runtime read more than SDC counted but less than the window: nothing was cut, and the Step Trace
               (P0-12) is where the gap is recorded. */
            Judgement::Underestimated | Judgement::Consistent => {}
        }
    }

    reply.map_err(|reason| unreachable(backend, &target.model, &reason))
}

/// The level-1 body on Model Studio: several tool calls per reply, and the turn's effort for a model that
/// takes `reasoning_effort` there (DeepSeek V4: `low`, `high` - its default - and `max`).
fn fast_body(body: &Value, target: &Target) -> Value {
    let mut fast = body.clone();

    if fast["tools"].as_array().is_some_and(|tools| !tools.is_empty()) {
        fast["parallel_tool_calls"] = serde_json::json!(true);
    }

    if let Some(level) = deepseek_effort(&target.model, target.effort.as_deref()) {
        fast["reasoning_effort"] = serde_json::json!(level);
    }

    fast
}

/// DeepSeek V4's `reasoning_effort` for a turn's effort: `None` keeps the model's own default.
fn deepseek_effort(model: &str, effort: Option<&str>) -> Option<&'static str> {
    let bare = model.rsplit('/').next().unwrap_or(model).to_ascii_lowercase();

    if !bare.starts_with("deepseek-v4") {
        return None;
    }

    match effort? {
        "low" | "medium" => Some("low"),
        "high" => Some("high"),
        "max" => Some("max"),
        _ => None,
    }
}

/// The highest request level an endpoint and model still take (0.21's "refused the mark", generalised).
fn level_caps() -> &'static std::sync::Mutex<std::collections::HashMap<String, usize>> {
    static CAPS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, usize>>> = std::sync::OnceLock::new();

    CAPS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn level_cap(url: &str, model: &str) -> usize {
    *level_caps().lock().unwrap_or_else(|poison| poison.into_inner()).get(&format!("{url} {model}")).unwrap_or(&2)
}

fn lower_cap(url: &str, model: &str, level: usize) {
    level_caps().lock().unwrap_or_else(|poison| poison.into_inner()).insert(format!("{url} {model}"), level);
}

/// Alibaba Cloud Model Studio (DashScope), which takes Anthropic-style `cache_control` marks and
/// `parallel_tool_calls` on an OpenAI-compatible request.
fn model_studio(url: &str) -> bool {
    url.contains("aliyuncs.com") && (url.contains("dashscope") || url.contains("maas"))
}

/// What a re-read of an unchanged file is answered with instead of the file (0.21).
const UNCHANGED_READ: &str = "[Unchanged since you read this file earlier in this turn - its content is above. Read it again only after it changes.]";

/// A file read again in the same turn, with nothing changed in it, costs its whole text again on every later
/// step (0.21): models re-read "to be sure" a lot. The second read is answered with one line instead - the
/// first one is still in the conversation. Anything that changed the file (an edit, a command, the person)
/// changes its hash, so a read after a change is always the full text. `seen` maps a path to its text's hash.
fn dedupe_reads(seen: &mut std::collections::HashMap<String, u64>, uses: &[dialect::ToolUse], results: &mut [dialect::ToolResult]) {
    use std::hash::{Hash, Hasher};

    for (call, result) in uses.iter().zip(results.iter_mut()) {
        if call.name != "read_file" || result.2 || result.1.len() < 400 {
            continue;
        }

        let Some(path) = call.input["path"].as_str().map(|path| path.trim().replace('\\', "/").trim_start_matches("./").to_string()) else {
            continue;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        result.1.hash(&mut hasher);

        let hash = hasher.finish();

        if seen.get(&path) == Some(&hash) {
            result.1 = UNCHANGED_READ.to_string();
        } else {
            seen.insert(path, hash);
        }
    }
}

/// v3 W1 (A-10): the step's file reads as the ledger records them - whether the path was read before in the
/// turn (`earlier`, the paths `dedupe_reads` had seen) and whether it went back as the one-line stub.
fn read_entries(earlier: &HashSet<String>, uses: &[dialect::ToolUse], results: &[dialect::ToolResult]) -> Vec<crate::bench::ledger::FileRead> {
    uses.iter()
        .zip(results)
        .filter(|(call, result)| call.name == "read_file" && !result.2)
        .map(|(call, result)| {
            let path = call.input["path"].as_str().unwrap_or_default().trim().replace('\\', "/").trim_start_matches("./").to_string();

            crate::bench::ledger::FileRead {
                repeat: earlier.contains(&path),
                deduped: result.1 == UNCHANGED_READ,
                tokens: crate::context::tokens_of(&result.1),
                path,
            }
        })
        .collect()
}

/// The most a turn's conversation grows to before older tool output is folded, however large the window
/// (1.0): every token of it is read again on each step, and past this a step is slow even when cached.
const WORKING_TOKENS: u64 = 160_000;

/// Keeps a turn's conversation small (0.13; rewritten in 1.0): when it passes its ceiling - 60% of the
/// window, at most `WORKING_TOKENS` - older tool output is folded **in one go, down to half the ceiling**.
///
/// It used to fold past 55% keeping the newest six results, which on a long turn meant folding again on
/// almost every step (503 of 1,181 steps on the owner's turn). Each fold rewrites the start of the
/// conversation, and the provider's prompt cache only serves an unchanged start - so the cache missed on
/// every one of those steps and the whole history was processed again, and every file read before the fold
/// had to be read again. Folding rarely and deeply keeps the start unchanged for many steps at a time.
///
/// `scale` turns SDC's character count into the provider's own tokens (measured on the previous step), and
/// `fixed` is the system prompt and the tools. Answers whether anything was folded.
fn keep_small(target: &Target, window: u64, messages: &mut [Value], fixed: u64, scale: f64) -> bool {
    let ceiling = (window * 60 / 100).clamp(2_000, WORKING_TOKENS);
    let measure = |messages: &[Value]| ((dialect::tokens(messages) + fixed) as f64 * scale) as u64;

    if measure(messages) < ceiling {
        return false;
    }

    let mut folded = 0;

    for keep in [24, 12, 6, 3, 1] {
        folded += dialect::fold_old_results(target.dialect, messages, keep);

        if measure(messages) <= ceiling / 2 {
            break;
        }
    }

    folded > 0
}

/// What every request of a turn carries besides the conversation: the system prompt and the tool list.
fn fixed_tokens(system: &str, specs: &[dialect::ToolSpec]) -> u64 {
    crate::context::tokens_of(system)
        + specs.iter().map(|spec| crate::context::tokens_of(spec.description) + crate::context::tokens_of(&spec.schema.to_string()) + 4).sum::<u64>()
}

/// How many of the provider's tokens one of SDC's counted tokens is, from the last step: what the provider
/// said the request held, over what SDC counted for it. Kept within reason when a provider says nothing.
fn token_scale(reported: u64, counted: u64, previous: f64) -> f64 {
    if reported == 0 || counted < 500 {
        return previous;
    }

    (reported as f64 / counted as f64).clamp(0.5, 3.0)
}

/// Is this failure the request being longer than the model takes?
fn too_long(reason: &str) -> bool {
    let lowered = reason.to_lowercase();

    ["maximum context length", "context_length_exceeded", "context length", "range of input length", "prompt is too long", "input is too long", "too many tokens", "reduce the length"]
        .iter()
        .any(|needle| lowered.contains(needle))
}

/// Fast models a provider refused for this key (not on the plan, not in the region) - never asked again.
fn refused_fast() -> &'static std::sync::Mutex<HashSet<String>> {
    static REFUSED: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> = std::sync::OnceLock::new();

    REFUSED.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

/// The model a sub-agent explores with (0.21): the newest `fast` model of the **same provider** - same key,
/// same endpoint - when the turn's own model is a `balanced` or `deep` one. A sub-agent only reads and
/// reports, which a small model does as well at a fraction of the time and price; the turn's own model still
/// does all the thinking and every change. `None` keeps the turn's model: a local model (loading a second
/// one costs memory), a model that is already fast, or one the catalogue cannot place.
fn fast_target(backend: Backend, target: &Target, provider: Option<&str>) -> Option<Target> {
    if backend != Backend::Api {
        return None;
    }

    let provider = crate::engines::native_api::endpoint_for(&target.model, provider).provider;
    let blocks = crate::providers::models::blocked();
    let block = blocks.iter().find(|block| block.id == provider)?;
    let fast = pick_fast(&block.models, &target.model)?;

    if refused_fast().lock().unwrap_or_else(|poison| poison.into_inner()).contains(&fast) {
        return None;
    }

    Some(Target {
        thinking: target.dialect == Dialect::Anthropic && crate::engines::native_api::adaptive_thinking(&fast),
        price: price_of(&provider, &fast),
        effort: None,
        model: fast,
        ..target.clone()
    })
}

/// From a provider's catalogue rows (newest first): the first `fast` model, when `current` is a known slower one.
fn pick_fast(models: &[Value], current: &str) -> Option<String> {
    let tier = models.iter().find(|row| row["id"].as_str() == Some(current))?["tier"].as_str()?;

    if tier == "fast" {
        return None;
    }

    models
        .iter()
        .filter(|row| row["tier"].as_str() == Some("fast"))
        .filter_map(|row| row["id"].as_str())
        .find(|id| *id != current && !id.contains("embed"))
        .map(str::to_string)
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
    research_session: Option<std::sync::Arc<research::Session>>,
) -> (tools::Outcome, u64, u64) {
    let stopped = || crate::engines::cancel::requested(&parent.turn_id);
    /* A research turn's sub-agent reads one page or runs one search, counted against the same limits. */
    let system = if research_session.is_some() { research::sub_agent_prompt() } else { sub_agent_prompt(workspace) };
    let mut specs = tools::specs_for(tools::Caps { vision, subagent: true, patch: false });

    if research_session.is_some() {
        specs.retain(|spec| research::TOOLS.contains(&spec.name));
    }
    let window = target.window(parent.provider.as_deref());
    /* 1.0: a sub-agent finds and reads; it thinks at the lowest effort its model takes, whatever the turn's. */
    let explorer = Target { effort: Some("low".to_string()), ..target.clone() };
    let target = &explorer;
    let fixed = fixed_tokens(&system, &specs);
    let mut scale = 1.0;
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
        research: research_session,
        local: backend == Backend::Ollama,
        result_cap: if target.local_window { (window as usize).clamp(2_000, tools::RESULT_CAP) } else { result_cap(backend, window) },
    };
    let mut messages = vec![dialect::user_message(job)];
    let (mut input, mut output) = (0u64, 0u64);
    /* 0.21: explore with the provider's fast model; the turn's own model is the fallback. */
    let fast = if crate::guard::modelgate::of(&parent.turn_id).allows(crate::guard::modelgate::Purpose::SubAgentFast) { fast_target(backend, target, parent.provider.as_deref()) } else { None };
    let mut on_fast = fast.is_some();
    /* v3 W1: a sub-agent's requests join the turn's ledger. Its job was written by the model, not the person,
       so the first message counts as context. */
    let ledger_on = crate::bench::ledger::enabled(&parent.turn_id);
    let shape_of = |messages: &[Value], specs: &[dialect::ToolSpec]| {
        ledger_on.then(|| {
            let mut shape = crate::bench::ledger::shape(&system, specs, messages, 0, 0);

            shape.counted[2] += std::mem::take(&mut shape.counted[5]);
            shape
        })
    };
    let ledger = |step: usize, model: &str, shape: Option<crate::bench::ledger::Shape>, reply: &Reply, at: std::time::Instant| {
        if let Some(shape) = shape {
            crate::bench::ledger::record(&parent.turn_id, "sub", step, model, shape, reply, at.elapsed().as_millis() as u64);
        }
    };

    /* v3 (owner's decision, 2026-10-14): a sub-agent has no step cap, like the turn itself (it had 24). It ends
       when it reports, when the turn is stopped, or at the budget the person set. */
    let mut sub_step = 0;

    loop {
        sub_step += 1;

        if stopped() {
            return (tools::Outcome::error("The turn was stopped."), input, output);
        }

        let active = if on_fast { fast.as_ref().unwrap_or(target) } else { target };
        let shape = shape_of(&messages, &specs);
        let asked_at = std::time::Instant::now();
        let reply = match ask_model(backend, active, (&parent.turn_id, if on_fast { crate::guard::modelgate::Purpose::SubAgentFast } else { crate::guard::modelgate::Purpose::Main }), &system, &messages, &specs, &EventSink::discarding(), &stopped) {
            Ok(reply) => {
                ledger(sub_step, &active.model, shape, &reply, asked_at);
                reply
            }
            /* The fast model was refused (or failed): this sub-agent - and later ones, when the refusal is the
               key's - go on with the turn's own model. Only before its first reply: a conversation the fast
               model has already written is not handed to another one (Anthropic's thinking rules differ). */
            Err(reason) if on_fast && messages.len() == 1 => {
                if !crate::engines::native_api::transient(&reason) {
                    refused_fast().lock().unwrap_or_else(|poison| poison.into_inner()).insert(active.model.clone());
                }

                on_fast = false;
                continue;
            }
            Err(reason) => return (tools::Outcome::error(format!("The sub-agent failed: {reason}")), input, output),
        };

        scale = token_scale(reply.input_tokens, dialect::tokens(&messages) + fixed, scale);
        input += reply.input_tokens;
        output += reply.output_tokens;
        messages.push(reply.message.clone());

        if reply.tool_uses.is_empty() {
            let report = if reply.text.trim().is_empty() { "The sub-agent finished without a report.".to_string() } else { reply.text };

            return (tools::Outcome::ok(report), input, output);
        }

        /* 1.0: a sub-agent's reads run side by side, as the main loop's have since 0.15.6 - it ran them one
           after another, and a sub-agent is nearly all reads. */
        let all_reads = reply.tool_uses.len() > 1 && reply.tool_uses.iter().all(|call| PARALLEL_READS.contains(&call.name.as_str()));
        let results: Vec<dialect::ToolResult> = if all_reads {
            let calls: Vec<(usize, &dialect::ToolUse)> = reply.tool_uses.iter().enumerate().collect();
            let first = context.calls;

            context.calls += calls.len();

            let mut outcomes = run_reads(&context, first, &calls);

            outcomes.sort_by_key(|(index, _)| *index);
            outcomes
                .into_iter()
                .map(|(index, outcome)| (reply.tool_uses[index].id.clone(), outcome.content, outcome.is_error, outcome.image))
                .collect()
        } else {
            reply
                .tool_uses
                .iter()
                .map(|call| {
                    let outcome = tools::execute(&mut context, call);

                    (call.id.clone(), outcome.content, outcome.is_error, outcome.image)
                })
                .collect()
        };

        let mut results = results;

        crate::v3::guard_results(&parent.session_id, &reply.tool_uses, &mut results);
        messages.extend(dialect::tool_results(target.dialect, &results));
        keep_small(target, window, &mut messages, fixed, scale);
    }
}

/// The tools that only look, and so may run side by side (0.15.6).
const PARALLEL_READS: [&str; 5] = ["read_file", "grep", "glob", "list_dir", "search"];

/// One reply's reads, run at the same time. Each gets its own read-only context numbered after `first`,
/// so its card id is the one it would have had in turn.
fn run_reads(parent: &ToolContext, first: usize, calls: &[(usize, &dialect::ToolUse)]) -> Vec<(usize, tools::Outcome)> {
    std::thread::scope(|scope| {
        let running: Vec<_> = calls
            .iter()
            .enumerate()
            .map(|(offset, (index, call))| {
                let index = *index;

                scope.spawn(move || {
                    let mut context = ToolContext {
                        workspace: parent.workspace,
                        session_id: parent.session_id,
                        read_only: true,
                        vision: parent.vision,
                        sink: parent.sink,
                        turn_id: parent.turn_id,
                        autonomy: parent.autonomy,
                        always: parent.always.clone(),
                        calls: first + offset,
                        checkpoint: None,
                        checkpointed: true,
                        policy: parent.policy,
                        changed: HashSet::new(),
                        radius_allowed: parent.radius_allowed,
                        plan: None,
                        edits: 0,
                        ran_at: None,
                        research: parent.research.clone(),
                        local: parent.local,
                        result_cap: parent.result_cap,
                    };

                    (index, tools::execute(&mut context, call))
                })
            })
            .collect();

        running.into_iter().filter_map(|handle| handle.join().ok()).collect()
    })
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
    research_session: Option<std::sync::Arc<research::Session>>,
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
                let research_session = research_session.clone();

                scope.spawn(move || {
                    if job.trim().is_empty() {
                        return (index, card, tools::Outcome::error("task needs a prompt: the whole job, as the sub-agent has seen nothing of this conversation."), 0, 0);
                    }

                    let (outcome, input, output) = sub_agent(backend, target, workspace, policy, prompt, vision, &card, &job, sink, research_session);

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

/// How long a turn spent waiting for the model and for its tools: ` · model 18s · tools 6.1s` after the
/// totals (0.20), so where a slow turn went is on the screen and not a guess. Nothing for a quick turn.
/// ` · 12.1k cached`: how much of the input the provider's prompt cache served (0.21), or nothing.
fn cached(tokens: u64) -> String {
    match tokens {
        0 => String::new(),
        1..=999 => format!(" · {tokens} cached"),
        _ => format!(" · {:.1}k cached", tokens as f64 / 1000.0),
    }
}

fn timing(model: std::time::Duration, tools: std::time::Duration) -> String {
    let short = |time: std::time::Duration| {
        let seconds = time.as_secs_f64();

        if seconds < 10.0 {
            format!("{seconds:.1}s")
        } else if seconds < 60.0 {
            format!("{}s", seconds.round() as u64)
        } else {
            format!("{}m {:02}s", seconds as u64 / 60, seconds as u64 % 60)
        }
    };

    if model + tools < std::time::Duration::from_secs(2) {
        return String::new();
    }

    format!(" · model {} · tools {}", short(model), short(tools))
}

/// A change this size is worth one strict re-read: files, or changed lines.
const REVIEW_FILES: usize = 2;
const REVIEW_LINES: usize = 40;
/// The most of the diff that is handed back.
const REVIEW_CHARS: usize = 9_000;

/// The self-review note (0.20): the diff of the files the agent changed - only those, so the person's own
/// earlier uncommitted work in other files is not read as the agent's - when the change is big enough to
/// hide a mistake. `None` for a small change, a folder with no git history to diff against, or new files
/// only.
fn review_note(workspace: &Workspace, changed: &HashSet<String>) -> Option<String> {
    let mut paths: Vec<&String> = changed.iter().filter(|path| checkable(path)).collect();

    if paths.is_empty() {
        return None;
    }

    paths.sort();

    let posix = workspace.is_remote() || !cfg!(windows);
    let quoted = paths.iter().map(|path| if posix { crate::ssh::sh_quote(path) } else { format!("\"{path}\"") }).collect::<Vec<_>>().join(" ");
    let report = workspace
        .run(&format!("git diff HEAD --no-color -U2 -- {quoted}"), std::time::Duration::from_secs(8))
        .ok()
        .filter(|report| report.ok && !report.timed_out)?;
    let lines = report.stdout.lines().filter(|line| (line.starts_with('+') || line.starts_with('-')) && !line.starts_with("+++") && !line.starts_with("---")).count();

    if report.stdout.trim().is_empty() || (paths.len() < REVIEW_FILES && lines < REVIEW_LINES) {
        return None;
    }

    let diff: String = report.stdout.chars().take(REVIEW_CHARS).collect();
    let cut = if report.stdout.chars().count() > REVIEW_CHARS { "\n… (the rest of the diff is cut)" } else { "" };

    Some(format!(
        "[SDC: before you finish, re-read your own change as a strict reviewer would. It touches {} file(s), {lines} changed lines:\n```diff\n{diff}{cut}\n```\n\
         Look for: logic that is wrong or only half done, callers or other files that now break, cases it does not handle (empty, missing, huge, wrong input), typos, leftovers from debugging. \
         If you find a real problem, fix it and run the relevant check again. If the change is right, give your summary now - do not repeat the diff.]",
        paths.len()
    ))
}

/// The gate of a turn (P0-10): the person's model, whether a Rotation Chain has an enabled row, whether the project is
/// local-only, and the exceptions the person wrote down themselves.
fn turn_gate(backend: Backend, target: &Target, prompt: &Prompt, policy: &crate::trust::policy::Policy) -> crate::guard::modelgate::Gate {
    let chain_has_rows = crate::data::home().map(|home| crate::data::rotation::load(&home).0).is_some_and(|rotation| !rotation.usable().is_empty());

    /* The pin is the model the person's choice resolved to - `target` is built from the prompt, so in a turn the two are
       the same model; a local backend is the `ollama` provider whatever the prompt called it. */
    let provider = if backend == Backend::Ollama { Some("ollama") } else { prompt.provider.as_deref() };

    crate::guard::modelgate::Gate::new(provider, &target.model, chain_has_rows, policy.privacy_local())
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
    /* P0-10: the turn's gate - the model the person chose, and what that choice allows. Registered once for the turn;
       everything that calls a model for it (the loop, a sub-agent, the research answer) finds it by the turn's id. */
    let gate = turn_gate(backend, target, prompt, policy);
    let _gate_held = crate::guard::modelgate::register(&turn_id, gate.clone());
    /* 1.0: effort "auto" on DeepSeek V4 is SDC's choice, not the model's `high`: `low` for everything but a
       large many-part task. Measured on the same fixed task (5 bugs, 13 tests): `high` 266 s, `low` 159 s, both
       with every test passing. A person who picks an effort gets exactly that. */
    let tuned = match (&target.effort, deepseek_effort(&target.model, Some("low"))) {
        (None, Some(_)) => {
            let words = prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text);

            Target { effort: (orient::pace_of(words) != orient::Pace::Deep).then(|| "low".to_string()), ..target.clone() }
        }
        _ => target.clone(),
    };
    /* v3 L4: with `v3.bandit` the plain rule is replaced by what actually happened for this kind of request. */
    let routed_request = prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text).to_string();
    let bandit_kind = crate::v3::bandit::kind_of(&routed_request);
    let bandit = crate::v3::on("bandit") && gate.allows(crate::guard::modelgate::Purpose::Bandit);
    let tuned = if bandit {
        /* The arms are the model the person chose and the fast one, when they named one. */
        let main = tuned.model.clone();
        let fast = crate::v3::router::fast().map(|(_, model)| model);
        let mut arms: Vec<&str> = vec![main.as_str()];

        if let Some(fast) = fast.as_deref().filter(|fast| *fast != main) {
            arms.push(fast);
        }

        let record = crate::v3::bandit::Record::load(std::path::Path::new(workspace.root()));

        match record.choose(&bandit_kind, &arms) {
            Some(chosen) if chosen != main => {
                let mut wanted = prompt.clone();

                wanted.provider = crate::v3::router::fast().map(|(provider, _)| provider);
                wanted.model = chosen.to_string();

                resolve_target(&wanted).unwrap_or(tuned)
            }
            _ => tuned,
        }
    } else {
        tuned
    };
    /* v3 W6: a plainly light request goes to the fast model the person named; everything else stays here. */
    let tuned = if crate::v3::on("router") && gate.allows(crate::guard::modelgate::Purpose::Router) {
        crate::v3::router::route(tuned, prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text), &|provider, model| {
            let mut fast = prompt.clone();

            fast.provider = Some(provider.to_string());
            fast.model = model.to_string();
            resolve_target(&fast).ok()
        })
    } else {
        tuned
    };
    /* v3 Q5: a small model is given room to think instead of being replaced. */
    let (tuned, lifted) = if crate::v3::on("effort") && crate::v3::effort::is_weak(&tuned.model, tuned.price) {
        let (thinking, effort) = crate::v3::effort::thinking_for(&tuned.model);

        (Target { thinking: tuned.thinking || thinking, effort: tuned.effort.clone().or(effort), ..tuned }, true)
    } else {
        (tuned, false)
    };
    let target = &tuned;
    /* v3 Q2: this turn's token techniques, settled once for its model (profile, setting, default). */
    let _switches = crate::v3::TurnSwitches::begin(&turn_id, prompt.provider.as_deref().unwrap_or_default(), &target.model);
    let technique = |feature: &str| crate::v3::turn_on(&turn_id, feature);
    let vision = browser::vision(target.dialect == Dialect::Anthropic, &target.model);
    /* The project's skills (0.13): their names and when to use them; the agent reads one when it applies. */
    let language = person_language(prompt);
    let window = target.window(prompt.provider.as_deref());
    /* A `/research` turn (0.16.1): its own brief, its own tools, its limits and its sources. */
    let research_session = options.research.map(|limits| std::sync::Arc::new(research::Session::new(limits)));
    let research_config = research::config();
    let system = match &research_session {
        Some(session) => research::system_prompt(&language, &session.limits, research_config.provider.label(), window < 32_000),
        None => system_prompt(workspace, vision, &language, technique("stable_prefix")) + &skills::brief(&skills::find(workspace)),
    };
    /* A model that writes the final answer from what this one gathered (Settings → Research), opt-in. */
    let synthesis = research_session.as_ref().and(research_config.synthesis.clone());
    /* The one model change the person asked for in Settings -> Research: named on the gate, so it is the only other
       model this turn may call, and only for the final answer. */
    if let Some((provider, model)) = &synthesis {
        crate::guard::modelgate::allow_exception(&turn_id, crate::guard::modelgate::Purpose::ResearchFinal, Some(provider), model);
    }
    /* The project's own MCP servers (0.12; on a host too since 0.13): their tools join the agent's for this turn. */
    let (mut mcp, warnings) = if research_session.is_some() {
        (None, Vec::new())
    } else {
        match workspace.remote() {
        /* On a host, the servers run there, over the same ssh (0.13). */
        Some(ssh) => mcp::McpTools::start_remote(workspace.root(), ssh),
        None => mcp::McpTools::start(std::path::Path::new(workspace.root())),
        }
    };

    for warning in warnings {
        sink.send(EngineEvent::Thinking(format!("MCP: {warning}\n")));
    }

    let mut specs = tools::specs_for(tools::Caps { vision, subagent: false, patch: patch::speaks_patch(&target.model) });

    if research_session.is_some() {
        specs.retain(|spec| research::TOOLS.contains(&spec.name));
    } else if backend == Backend::Ollama && research_config.local_web_only_research {
        /* A local model kept off the web (Settings → Research) is not offered the web tools at all. Offered
           and refused, a small model asked for the weather tried thirteen times (live check, 0.16.1). */
        specs.retain(|spec| !matches!(spec.name, "web_search" | "web_fetch"));
    }

    if let Some(servers) = &mcp {
        specs.extend(servers.specs().iter().cloned());
    }

    /* v3 I7: the fleet tool, when switched on. */
    if crate::v3::on("fleet") && research_session.is_none() && backend == Backend::Api {
        specs.push(crate::v3::fleet::spec());
    }

    /* v3 W5: only the tools the request needs; a call to another one adds it for the rest of the turn. */
    let all_specs = specs.clone();
    let pruning = technique("tool_pruning") && research_session.is_none();

    if pruning {
        specs = crate::v3::tools::select(&all_specs, prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text));
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

    /* The map of the folder (0.20), unless the turn is a quick question: the first step can then be the one
       that matters instead of `list_dir .`. For this turn only - the conversation keeps the person's words. */
    let mut map_chars = 0;
    let text = {
        let words = prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text);
        let wants_map = research_session.is_none() && orient::pace_of(words) != orient::Pace::Quick;

        match wants_map.then(|| orient::orientation(workspace)).flatten() {
            Some(map) => {
                map_chars = map.len() + 2;
                format!("{map}\n\n{text}")
            }
            None => text,
        }
    };
    /* v3 W5: the date the system prompt no longer carries (stable_prefix) opens the turn's message instead. */
    let text = if technique("stable_prefix") && research_session.is_none() {
        let today = chrono::Local::now().format("%Y-%m-%d");

        map_chars += format!("[Today: {today}]\n").len();
        format!("[Today: {today}]\n{text}")
    } else {
        text
    };
    /* The person's own words, the part of the request SDC is answering (used by every note below). */
    let request_text = prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text).to_string();
    /* v3 W7b: `/plan` - this turn answers with the plan and changes nothing. */
    let plan_mode = crate::v3::plan::is_plan(&request_text);
    let _plan = crate::v3::plan::PlanMode::begin(plan_mode);
    /* v3 I4: a recipe from a similar verified task in this project, as data (switch `v3.recipes`). */
    let recipes_on = technique("recipes") && research_session.is_none() && prompt.remote.is_none();
    let text = match recipes_on.then(|| crate::v3::recipes::find(std::path::Path::new(workspace.root()), prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text))).flatten() {
        Some((_, recipe)) => {
            let note = crate::v3::recipes::as_note(&recipe);

            map_chars += note.len() + 2;
            format!("{note}\n\n{text}")
        }
        None => text,
    };
    /* v3 I5: `@budget 40k` / `@budget $0.10` in the person's words - told to the model, watched every step. */
    let budget = if crate::v3::on("budget") { crate::v3::budget::parse(prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text)) } else { None };
    let text = match &budget {
        Some(budget) => format!("{text}\n\n{}", budget.opening()),
        None => text,
    };
    /* v3 L1: what earlier work in this project learned the hard way, when this request matches it. */
    let lessons_on = technique("lessons") && research_session.is_none() && prompt.remote.is_none();
    /* v3 L0-L2: what this turn proves is kept even while sending it is off (switch `v3.learn`). */
    let learning = crate::v3::on("learn") && research_session.is_none() && prompt.remote.is_none();
    let mut lesson_note: Option<String> = lessons_on.then(|| crate::v3::lessons::find(std::path::Path::new(workspace.root()), &request_text)).flatten();
    /* v3 L3: a page an earlier turn read about this - with its address and how long ago it was read. */
    let knowledge_note = (technique("knowledge") && research_session.is_none() && prompt.remote.is_none()).then(|| crate::v3::knowledge::lookup(std::path::Path::new(workspace.root()), &request_text)).flatten();
    /* v3 W4: a map of the project's declarations, ranked for this request. */
    let map = if technique("repomap") && research_session.is_none() {
        crate::intel::repo_map(std::path::Path::new(workspace.root()), &request_text, &[], 1_200)
    } else {
        String::new()
    };
    /* v3 I8: one English brief of a message that is not in English - the person's technical words untouched. */
    let brief = if technique("interlingua") && research_session.is_none() {
        let root = std::path::Path::new(workspace.root());

        crate::v3::interlingua::learn(root, &request_text);
        let glossary = crate::v3::interlingua::glossary(root);

        crate::v3::interlingua::detect(&request_text, &glossary).map(|language| crate::v3::interlingua::brief(&request_text, language, &glossary))
    } else {
        None
    };
    let text = {
        let mut notes = String::new();

        for note in [
            plan_mode.then_some(crate::v3::plan::instruction()),
            lifted.then_some(crate::v3::effort::instruction()),
            lesson_note.as_deref(),
            knowledge_note.as_deref(),
            (!map.is_empty()).then_some(map.as_str()),
            brief.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            map_chars += note.len() + 2;
            notes.push_str(note);
            notes.push_str("\n\n");
        }

        format!("{notes}{text}")
    };

    let mut converged = false;
    let mut pending_note: Option<String> = None;
    /* v3 L1: the turn's steps, so a lesson can be distilled from the ones that failed. */
    let mut steps: Vec<crate::v3::lessons::Step> = Vec::new();
    /* v3 I9: what the capsule tells the next model - the steps taken and the last words said. */
    let mut step_log: Vec<String> = Vec::new();
    let mut passed_checks: Vec<String> = Vec::new();
    let mut last_words = String::new();
    /* v3 W1: the context ledger, beside the request and only when the bench (or a test) listens. */
    let ledger_on = crate::bench::ledger::enabled(&turn_id);
    /* v3 I10: the turn's token receipt (switch `v3.receipt`) - the input by part and what each technique saved. */
    let receipt_on = crate::v3::on("receipt");
    let mut receipt_parts = [0u64; 6];
    let all_specs_tokens = fixed_tokens("", &all_specs);
    let history_len = prompt.history.len();

    /* v3 W9: what the person pasted, and earlier turns, pass the same secret filter as tool output. */
    let text = if crate::v3::on("redact") {
        for message in messages.iter_mut() {
            if let Some(earlier) = message["content"].as_str().map(str::to_string) {
                message["content"] = Value::String(crate::v3::redact::scrub(&prompt.session_id, &earlier).0);
            }
        }

        crate::v3::redact::scrub(&prompt.session_id, &text).0
    } else {
        text
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
        research: research_session.clone(),
        local: backend == Backend::Ollama,
        result_cap: if target.local_window { (window as usize).clamp(2_000, tools::RESULT_CAP) } else { result_cap(backend, window) },
    };
    /* v3 R2: a turn the daemon lost continues here - its conversation rebuilt from the journal, the reads that
       never answered run again, and a change that never answered is reported as unknown rather than redone.
       v3 R4: a lost turn whose tools ran on another machine is not continued here at all: its tools ran over there, where
       this daemon's journal says nothing about what became of them, so it is reported as unknown and left alone. */
    let remote_lost = if prompt.remote.is_none() { crate::v3::journal::lost_remote(&prompt.session_id) } else { None };

    if let Some(host) = &remote_lost {
        sink.send(EngineEvent::Thinking(format!(
            "The turn before this one ran on {host} and never finished here. What it managed to do there is unknown, so it is not started again; ask me again and this turn goes on from what is on disk.\n"
        )));
    }

    let resumed = if research_session.is_none() && prompt.remote.is_none() && remote_lost.is_none() {
        crate::v3::journal::take(&prompt.session_id)
    } else {
        /* A lost remote turn's journal is spent by `lost_remote`, so there is nothing left to take. */
        None
    };

    if let Some(resume) = &resumed {
        messages = resume.messages.clone();
        crate::v3::redact::import(&prompt.session_id, &resume.vault);

        let (rerun, known) = crate::v3::journal::settle(resume, Some(std::path::Path::new(workspace.root())));
        let mut answers: HashMap<String, dialect::ToolResult> = known.into_iter().map(|(id, content, is_error)| (id.clone(), (id, content, is_error, None))).collect();

        for call in &rerun {
            let outcome = tools::execute(&mut context, call);

            answers.insert(call.id.clone(), (call.id.clone(), outcome.content, outcome.is_error, outcome.image));
        }

        let mut results: Vec<dialect::ToolResult> = resume.calls.iter().filter_map(|call| answers.remove(&call.id)).collect();
        let answered_calls: Vec<dialect::ToolUse> = resume.calls.iter().filter(|call| results.iter().any(|result| result.0 == call.id)).cloned().collect();

        crate::v3::guard_results(&prompt.session_id, &answered_calls, &mut results);

        if !results.is_empty() {
            messages.extend(dialect::tool_results(target.dialect, &results));
        }

        sink.send(EngineEvent::Thinking(format!(
            "Resumed from the journal of the interrupted turn{}: {} step(s) kept, {} call(s) run again.{}",
            /* P0-4: a resumed turn says which machine the work happened on, and whether a record was lost, so a
               turn that comes back short is explained rather than looking like the model forgot. */
            match resume.origin {
                crate::v3::journal::Origin::Vps => match &resume.remote {
                    Some(host) => format!(", on {host}"),
                    None => ", on the runner".to_string(),
                },
                crate::v3::journal::Origin::Local => String::new(),
            },
            resume.messages.iter().filter(|message| message["role"] == "assistant").count(),
            rerun.len(),
            match (resume.damaged, resume.skipped_newer, &resume.step_id) {
                (0, 0, step) if step.is_empty() => String::new(),
                (damaged, newer, step) => format!(
                    "{} record(s) did not read back and were skipped{}{}.\n",
                    damaged,
                    if newer > 0 { format!(", {newer} came from a newer build") } else { String::new() },
                    if step.is_empty() { String::new() } else { format!(" (interrupted inside {step})") },
                ),
            },
        )));
    }

    /* v3 R1: the journal - written ahead, step by step, for a local agent turn on an API model.
       v3 R4: a turn that runs on a runner is journaled the same way, with the machine it ran on recorded, so an interrupted
       turn is never silently redone: its tools ran over there, and replaying them here would be a different turn. */
    let journal = (backend == Backend::Api && research_session.is_none())
        .then(|| {
            crate::v3::journal::Writer::open_on(
                &turn_id,
                &prompt.session_id,
                serde_json::json!({
                    "messages": messages,
                    "model": prompt.model,
                    "provider": prompt.provider,
                    "root": prompt.project_root,
                    "autonomy": format!("{:?}", options.autonomy).to_lowercase(),
                    "effort": prompt.effort,
                    "resumeDepth": resumed.as_ref().map(|resume| resume.depth + 1).unwrap_or(0),
                    /* R4: the machine this turn's tools run on, when it is not this one. A resumed turn is never replayed
                       here when this is set: the work happened over there. */
                    "remote": prompt.remote.as_ref().map(|ssh| ssh.target.user_host.clone()),
                }),
                /* P0-4: `origin` is written on every record, so a crashed VPS turn can be found after a restart
                   without reading its payload, and the resume can say which machine the work happened on. */
                match prompt.remote {
                    Some(_) => crate::v3::journal::Origin::Vps,
                    None => crate::v3::journal::Origin::Local,
                },
            )
        })
        .flatten();
    let mut journaled = messages.len();
    let (mut input_tokens, mut output_tokens) = (0u64, 0u64);
    /* Of the input, what the provider read back from its prompt cache (0.21) - said in the footer. */
    let mut cached_tokens = 0u64;
    /* The research time ran out: one last call, without tools, for the answer. */
    let mut out_of_time = false;
    /* With a final-answer model, this model's words are its notes - shown as thinking, not as the answer. */
    let notes_sink = |separate: bool| -> EventSink {
        let inner = step_sink(sink, separate);

        if synthesis.is_none() {
            return inner;
        }

        EventSink::new(move |event| match event {
            EngineEvent::Delta(text) => inner.send(EngineEvent::Thinking(text)),
            other => inner.send(other),
        })
    };
    let mut said_something = false;
    /* Replies in a row the provider's content filter blocked. */
    let mut filtered = 0;
    let mut plan_nudged = false;
    /* A research answer from one page is asked, once, to read a second (live check, 0.16.1: a 9B model
       answered from one page with the brief saying two). */
    let mut sources_nudged = false;
    let mut check_rounds = 0;
    /* Where the turn's time went (0.20): the model's calls, and the tools between them. */
    let (mut model_time, mut tool_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    /* The change was shown back to the model for one strict re-read (0.20). */
    let mut reviewed = false;
    /* Files this turn has read, by the hash of what they held - an unchanged re-read is one line (0.21). */
    let mut seen_reads: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    /* v3 I1: the turn's whole-file reads, for sending a changed re-read as a diff (switch `v3.ledger_diff`). */
    let mut prior_reads: HashMap<String, String> = HashMap::new();
    /* v3 I10: tokens each technique saved this turn (SDC's count), for the receipt. */
    let mut saved: std::collections::BTreeMap<&'static str, u64> = std::collections::BTreeMap::new();
    /* 1.0: the system prompt and tools in every request, and SDC's count turned into the provider's tokens. */
    let mut fixed = fixed_tokens(&system, &specs);
    let mut scale = 1.0;
    /* The request was too long for the model once already: folded hard and asked again (1.0). */
    let mut shrunk = false;
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

        if let Some(journal) = &journal {
            journal.messages(step, &messages[journaled.min(messages.len())..]);
        }

        journaled = messages.len();

        if let Some(session) = research_session.as_ref().filter(|session| session.expired() && !out_of_time) {
            out_of_time = true;
            dialect::append_user_text(
                target.dialect,
                &mut messages,
                &format!("[SDC: the research time ({} minutes) is up. Write the answer now from what you found, citing [n], without calling tools.]", session.limits.max_minutes),
            );
        }

        let step_specs: &[dialect::ToolSpec] = if out_of_time { &[] } else { &specs };
        let shape = (ledger_on || receipt_on).then(|| crate::bench::ledger::shape(&system, step_specs, &messages, history_len, map_chars));
        let asked_at = std::time::Instant::now();
        let asked = ask_model(backend, target, (&turn_id, crate::guard::modelgate::Purpose::Main), &system, &messages, step_specs, &notes_sink(said_something), &stopped);

        model_time += asked_at.elapsed();

        /* P0-16: a local model whose speed suddenly halves, or that is not entirely on the GPU, is said so once. */
        if let (Ok(reply), Some(_), true) = (&asked, target.local_context, crate::v3::on("local.autotune")) {
            let rate = crate::trace::Step::rate(reply.output_tokens, reply.ttft_ms, asked_at.elapsed().as_millis() as u64);
            let warning = crate::sme::tuner::watch().lock().ok().and_then(|mut watch| if reply.output_tokens >= 20 { watch.observe(rate) } else { None });

            if let Some(warning) = warning {
                sink.send(EngineEvent::Delta(format!("_({warning})_\n\n")));
            }

            /* The other sign, read once per turn after the model is loaded: `ollama ps` says how much of it is on the GPU. */
            if step == 1 {
                let ps: Option<Value> = crate::engines::ollama::call("/api/ps", None).ok().and_then(|text| serde_json::from_str(&text).ok());
                let unified = crate::sme::tuner::detect().unified;

                if let Some(note) = ps.and_then(|ps| crate::sme::tuner::gpu_share(&ps, &target.model)).and_then(|share| crate::sme::tuner::spill_warning(share, unified)) {
                    sink.send(EngineEvent::Delta(format!("_({note})_\n\n")));
                }
            }
        }

        /* P0-12: one trace row per model call of the loop. The mode is the gate's, the cost the catalogue's price. */
        if let Ok(reply) = &asked {
            let wall_ms = asked_at.elapsed().as_millis() as u64;

            crate::trace::record(crate::trace::Step {
                task_id: turn_id.clone(),
                step_id: format!("{turn_id}.s{step}"),
                session_id: prompt.session_id.clone(),
                step: step as u32,
                model_key: format!("{}:{}", prompt.provider.as_deref().unwrap_or(if backend == Backend::Ollama { "ollama" } else { "" }), target.model),
                mode: gate.mode.name().to_string(),
                purpose: "main".to_string(),
                tokens_in: reply.input_tokens,
                tokens_out: reply.output_tokens,
                thinking_tokens: reply.thinking_tokens,
                ttft_ms: reply.ttft_ms,
                tok_per_s: crate::trace::Step::rate(reply.output_tokens, reply.ttft_ms, wall_ms),
                wall_ms,
                verify: None,
                cost_usd: target.price.map(|(input, output)| (reply.input_tokens - reply.cached_tokens.min(reply.input_tokens)) as f64 * input / 1e6 + reply.cached_tokens as f64 * input * 0.1 / 1e6 + reply.output_tokens as f64 * output / 1e6),
                escalation_reason: None,
            });
        }

        if let (Some(shape), Ok(reply)) = (shape, &asked) {
            if ledger_on {
                crate::bench::ledger::record(&turn_id, "main", step, &target.model, shape, reply, asked_at.elapsed().as_millis() as u64);
            }

            for (sum, part) in receipt_parts.iter_mut().zip(crate::bench::ledger::apportion(reply.input_tokens, &shape.counted)) {
                *sum += part;
            }

            if pruning {
                *saved.entry("tool pruning").or_default() += all_specs_tokens.saturating_sub(fixed_tokens("", step_specs));
            }
        }

        let mut reply: Reply = match asked {
            Ok(reply) => {
                filtered = 0;

                reply
            }
            Err(reason) if reason == "stopped" => return,
            /* 1.0: a request the model finds too long is folded hard and asked again, once - it used to end the
               turn, with all of its work, on a window SDC had guessed. */
            Err(reason) if too_long(&reason) && !shrunk => {
                shrunk = true;
                dialect::fold_old_results(target.dialect, &mut messages, 2);
                seen_reads.clear();
                prior_reads.clear();

                continue;
            }
            /* The provider's own filter blocked the reply (0.15.6). The report's two long turns ended on
               it, each mid-sentence while quoting logs. The reply never reached the conversation, so the
               step is asked again with a note to say it shorter - the work before it is kept. */
            Err(reason) if content_filtered(&reason.to_lowercase()) && filtered < FILTER_RETRIES => {
                filtered += 1;
                sink.send(EngineEvent::ToolStarted {
                    call_id: format!("{turn_id}-filter-{step}"),
                    tool: "run".to_string(),
                    name: "Content filter".to_string(),
                    target: "the provider blocked a reply".to_string(),
                });
                sink.send(EngineEvent::ToolOutput {
                    call_id: format!("{turn_id}-filter-{step}"),
                    level: "dim".to_string(),
                    text: format!("{reason}\nasking again for a shorter reply ({filtered} of {FILTER_RETRIES}) · the work so far is kept"),
                });
                sink.send(EngineEvent::ToolCompleted {
                    call_id: format!("{turn_id}-filter-{step}"),
                    status: "done".to_string(),
                    meta: "asked again".to_string(),
                    diff: None,
                });
                dialect::append_user_text(target.dialect, &mut messages, FILTER_NOTE);

                continue;
            }
            Err(reason) => {
                /* v3 I9: the provider failed for good - the task continues on the next model of `v3.failoverOrder`,
                   from a capsule of its state rather than this provider's conversation. */
                if backend == Backend::Api && research_session.is_none() && gate.allows(crate::guard::modelgate::Purpose::Failover) {
                    let mut tried = crate::v3::failover::tried(&turn_id);
                    let outermost = tried.is_empty();

                    if outermost {
                        tried.push((prompt.provider.clone().unwrap_or_default(), prompt.model.clone()));
                    }

                    while let Some((provider, model)) = crate::v3::failover::next(&tried) {
                        tried.push((provider.clone(), model.clone()));
                        crate::v3::failover::set_tried(&turn_id, &tried);

                        let capsule = crate::v3::failover::capsule(&crate::v3::failover::State {
                            goal: prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text),
                            failed: format!("{}:{}", prompt.provider.as_deref().unwrap_or("?"), target.model),
                            reason: &reason,
                            plan: context.open_plan_steps(),
                            changed: context.changed.iter().cloned().collect(),
                            steps: &step_log,
                            last_words: &last_words,
                        });
                        let next_prompt = Prompt { provider: Some(provider.clone()), model: model.clone(), text: capsule, history: Vec::new(), images: Vec::new(), ..prompt.clone() };

                        if let Ok(next_target) = failover_target(&next_prompt) {
                            sink.send(EngineEvent::Thinking(format!("{} failed ({reason}); continuing on {provider}:{model} from a state capsule.\n", target.model)));
                            drive(backend, &next_target, workspace, options, None, policy, &next_prompt, sink);

                            if outermost {
                                crate::v3::failover::forget(&turn_id);
                            }

                            return;
                        }
                    }

                    if outermost {
                        crate::v3::failover::forget(&turn_id);
                    }
                }

                sink.send(EngineEvent::Failed(reason));

                return;
            }
        };

        scale = token_scale(reply.input_tokens, dialect::tokens(&messages) + fixed, scale);
        input_tokens += reply.input_tokens;
        output_tokens += reply.output_tokens;
        cached_tokens += reply.cached_tokens;

        /* Totals so far, every step: the cost governor can stop a turn that crosses its budget mid-way. */
        sink.send(EngineEvent::Usage { input_tokens, output_tokens, cost_usd: None });

        /* v3 I5: at 100% of the person's budget the turn stops here, its work kept; at 80% it is asked to converge. */
        if let Some(used) = budget.as_ref().and_then(|budget| budget.used(input_tokens, output_tokens, target.price)) {
            if used >= 1.0 {
                sink.send(EngineEvent::Delta(format!(
                    "\n\nI stopped at the budget you set ({}). The work so far is kept - send \"continue\" (with a new @budget if you like) to go on.",
                    budget.as_ref().map(|budget| budget.label()).unwrap_or_default()
                )));
                sink.send(EngineEvent::Done {
                    summary: "Paused at the budget".to_string(),
                    meta: format!("{}{} · budget {:.0}% used", meta(step, input_tokens, output_tokens, target.price) + &cached(cached_tokens), timing(model_time, tool_time), used * 100.0),
                    pass: None,
                });

                return;
            }

            if used >= 0.8 && !converged {
                converged = true;
                pending_note = Some(crate::v3::budget::Budget::CONVERGE.to_string());
            }
        }
        said_something = said_something || !reply.text.trim().is_empty();

        if !reply.text.trim().is_empty() {
            last_words = reply.text.clone();
        }

        if reply.stop == "refusal" {
            sink.send(EngineEvent::Failed(
                "The model declined to continue this request (a safety refusal). Rephrase the task, or try another model.".to_string(),
            ));

            return;
        }

        messages.push(reply.message.clone());

        /* v3 W9: the model wrote `‹secret:N›`; the tools get the real value - and so does the journal, whose
           intents must still mean the same thing after a restart has emptied the vault. */
        if crate::v3::on("redact") {
            for call in &mut reply.tool_uses {
                call.input = crate::v3::redact::restore(&prompt.session_id, &call.input);
            }
        }

        if let Some(journal) = &journal {
            journal.vault(step, &crate::v3::redact::export(&prompt.session_id));
            journal.model(step, &reply.message, &reply.tool_uses);

            for call in &reply.tool_uses {
                journal.intent(step, call);
            }
        }

        journaled = messages.len();

        /* v3 W5: a tool the turn was not offered but the model asked for joins the list from now on. */
        if pruning {
            let mut added = false;

            for call in &reply.tool_uses {
                added |= crate::v3::tools::add_missing(&mut specs, &all_specs, &call.name);
            }

            if added {
                fixed = fixed_tokens(&system, &specs);
            }
        }

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
            if let Some(session) = research_session.as_ref().filter(|_| !sources_nudged && !out_of_time) {
                let (_, pages) = session.counts();

                if pages < 2 && pages < session.limits.max_pages && session.sources().len() > pages {
                    sources_nudged = true;
                    dialect::append_user_text(
                        target.dialect,
                        &mut messages,
                        "[SDC: you have read only one page. Read at least one more source from the results (web_fetch, a different site) to check the answer, then write it citing [n].]",
                    );

                    continue;
                }
            }

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

            /* The self-review (0.20): a change across files, or a long one, is shown back once, as a diff, for a
               strict re-read before the summary. A small change skips it - the point is to catch what a big one
               hides, not to add a step to every turn. */
            if !reviewed && research_session.is_none() && !stopped() {
                if let Some(note) = review_note(workspace, &context.changed) {
                    reviewed = true;
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

            if let Some(session) = &research_session {
                if let Some((provider, model)) = &synthesis {
                    let (input, output) = synthesize(prompt, provider, model, &language, &messages, sink, &stopped);

                    input_tokens += input;
                    output_tokens += output;
                    sink.send(EngineEvent::Usage { input_tokens, output_tokens, cost_usd: None });
                }

                let (searches, pages) = session.counts();

                sink.send(EngineEvent::Sources(session.sources_json()));
                sink.send(EngineEvent::Done {
                    summary: summary.to_string(),
                    meta: format!("{} · {searches} searches · {pages} pages", meta(step, input_tokens, output_tokens, target.price)),
                    pass: None,
                });

                return;
            }

            /* v3 L1: what this turn learned the hard way, kept for the next one - and only what the core admits. */
            if lessons_on || learning {
                for lesson in crate::v3::lessons::observe(&steps) {
                    let admitted = !crate::v3::on("knowledge_core")
                        || matches!(
                            crate::v3::knowledge_core::propose(
                                std::path::Path::new(workspace.root()),
                                crate::v3::knowledge_core::Candidate { kind: "lesson".into(), text: lesson.clone(), observations: vec![turn_id.clone()] },
                            ),
                            crate::v3::knowledge_core::Decision::Admitted(_)
                        );

                    if admitted {
                        crate::v3::lessons::save(std::path::Path::new(workspace.root()), &lesson);
                    }
                }
            }

            if (recipes_on || learning) && summary == "Done" {
                let done = crate::v3::recipes::Done {
                    request: prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text),
                    changed: context.changed.iter().cloned().collect(),
                    passed_checks: passed_checks.clone(),
                    tools: step_log.clone(),
                    changes: if crate::v3::on("cas") { crate::v3::recipes::changes(std::path::Path::new(workspace.root()), &turn_id) } else { String::new() },
                };

                if !crate::v3::on("knowledge_core")
                    || matches!(
                        crate::v3::knowledge_core::propose(
                            std::path::Path::new(workspace.root()),
                            crate::v3::knowledge_core::Candidate {
                                kind: "recipe".into(),
                                text: crate::v3::recipes::distil(&done).unwrap_or_default(),
                                observations: vec![turn_id.clone()],
                            },
                        ),
                        crate::v3::knowledge_core::Decision::Admitted(_)
                    )
                {
                    crate::v3::recipes::save(std::path::Path::new(workspace.root()), &done);
                }
            }

            /* v3 L0: a check that passed after a change in two separate turns becomes a fact in the project's memory. */
            if learning && summary == "Done" && crate::v3::on("knowledge_core") {
                let root = std::path::Path::new(workspace.root());
                let facts: HashSet<String> = passed_checks.iter().filter_map(|check| crate::v3::memory::check_fact(check)).collect();

                for fact in facts {
                    if let crate::v3::knowledge_core::Decision::Admitted(_) =
                        crate::v3::knowledge_core::propose(root, crate::v3::knowledge_core::Candidate { kind: "memory".into(), text: fact.clone(), observations: vec![turn_id.clone()] })
                    {
                        crate::v3::memory::remember(root, &fact);
                    }
                }
            }

            /* v3 L4: what this turn taught the bandit - it got there, and what it cost to get there. */
            if bandit {
                let cost = crate::v3::router::estimate(&routed_request, target.price, fixed + crate::context::tokens_of(&text));
                let budget = budget.as_ref().and_then(|budget| match budget {
                    crate::v3::budget::Budget::Dollars(limit) => Some(*limit),
                    crate::v3::budget::Budget::Tokens(_) => None,
                });
                let mut record = crate::v3::bandit::Record::load(std::path::Path::new(workspace.root()));

                record.observe(&bandit_kind, &target.model, crate::v3::bandit::reward(summary == "Done", cost, budget));
                record.save(std::path::Path::new(workspace.root()));
            }

            let estimate = crate::v3::on("router").then(|| crate::v3::router::estimate(&request_text, target.price, fixed + crate::context::tokens_of(&text))).flatten().map(|cost| format!(" · estimated ${cost:.3}"));

            sink.send(EngineEvent::Done {
                summary: summary.to_string(),
                meta: format!("{}{}{}{}", meta(step, input_tokens, output_tokens, target.price) + &cached(cached_tokens), timing(model_time, tool_time), estimate.unwrap_or_default(), if receipt_on { crate::v3::receipt(&receipt_parts, &saved) } else { String::new() }),
                pass: None,
            });

            return;
        }

        let tools_at = std::time::Instant::now();

        /* The step's calls, in order - except `task`, whose sub-agents run all at once. */
        let mut results: Vec<Option<dialect::ToolResult>> = vec![None; reply.tool_uses.len()];
        let tasks: Vec<(usize, &dialect::ToolUse)> = reply.tool_uses.iter().enumerate().filter(|(_, call)| call.name == "task").collect();

        /* Reads run at the same time (0.15.6): on a VPS each one is a round trip (~500 ms against the report's
           host), and a model often asks for four or five at once. Since 0.20 that holds for every *run* of
           reads inside a reply, not only for a reply made of nothing else - "read a, read b, edit c, read d"
           reads a and b together, then edits, then reads d, in the order the model wrote them, so a read
           after an edit still sees the edit. */
        let parallel = |call: &dialect::ToolUse, mcp: &Option<mcp::McpTools>| {
            PARALLEL_READS.contains(&call.name.as_str()) && mcp.as_ref().is_none_or(|servers| !servers.handles(&call.name))
        };
        let mut index = 0;

        while index < reply.tool_uses.len() {
            if stopped() {
                return;
            }

            let call = &reply.tool_uses[index];

            if call.name == "task" || results[index].is_some() {
                index += 1;

                continue;
            }

            if parallel(call, &mcp) {
                /* The run of reads that starts here (a `task` between them runs later anyway). */
                let mut run: Vec<(usize, &dialect::ToolUse)> = Vec::new();
                let mut next = index;

                while next < reply.tool_uses.len() {
                    let candidate = &reply.tool_uses[next];

                    if candidate.name != "task" {
                        if !parallel(candidate, &mcp) {
                            break;
                        }

                        run.push((next, candidate));
                    }

                    next += 1;
                }

                if run.len() > 1 {
                    let first = context.calls;

                    context.calls += run.len();

                    for (position, outcome) in run_reads(&context, first, &run) {
                        if let Some(journal) = &journal {
                            journal.result(step, &reply.tool_uses[position].id, &reply.tool_uses[position].name, &outcome.content, outcome.is_error);
                        }

                        results[position] = Some((reply.tool_uses[position].id.clone(), outcome.content, outcome.is_error, outcome.image));
                    }

                    index = next;

                    continue;
                }
            }

            let outcome = match mcp.as_mut() {
                Some(servers) if servers.handles(&call.name) => tools::mcp_call(&mut context, servers, call),
                _ => tools::execute(&mut context, call),
            };

            if let Some(journal) = &journal {
                journal.result(step, &call.id, &call.name, &outcome.content, outcome.is_error);
            }

            results[index] = Some((call.id.clone(), outcome.content, outcome.is_error, outcome.image));
            index += 1;
        }

        if !tasks.is_empty() {
            for (index, outcome, input, output) in run_tasks(backend, target, workspace, policy, prompt, vision, &tasks, sink, research_session.clone()) {
                input_tokens += input;
                output_tokens += output;

                if let Some(journal) = &journal {
                    journal.result(step, &reply.tool_uses[index].id, "task", &outcome.content, outcome.is_error);
                }

                results[index] = Some((reply.tool_uses[index].id.clone(), outcome.content, outcome.is_error, outcome.image));
            }

            sink.send(EngineEvent::Usage { input_tokens, output_tokens, cost_usd: None });
        }

        tool_time += tools_at.elapsed();

        let mut results: Vec<dialect::ToolResult> = results
            .into_iter()
            .enumerate()
            .map(|(index, result)| result.unwrap_or_else(|| (reply.tool_uses[index].id.clone(), "The turn was stopped.".to_string(), true, None)))
            .collect();

        let earlier: HashSet<String> = if ledger_on { seen_reads.keys().cloned().collect() } else { HashSet::new() };

        for (call, result) in reply.tool_uses.iter().zip(&results) {
            let what = ["path", "command", "pattern", "url", "query"].iter().find_map(|key| call.input[*key].as_str()).unwrap_or_default();

            /* v3 I4: a command that passed after the turn changed something is what proves a recipe. */
            if call.name == "run_command" && !result.2 && result.1.starts_with("exit code: 0") && !context.changed.is_empty() {
                passed_checks.push(what.to_string());
            }

            step_log.push(format!("{} {} → {}", call.name, what.chars().take(80).collect::<String>(), if result.2 { "error" } else { "ok" }));
            steps.push(crate::v3::lessons::Step {
                tool: call.name.clone(),
                target: what.chars().take(120).collect(),
                /* A command that exited non-zero failed, whichever tool ran it (not only exit code 1). */
                ok: !result.2 && crate::v3::lessons::exit_code(&result.1).is_none_or(|code| code == 0),
                note: result.1.lines().next().unwrap_or_default().chars().take(160).collect(),
            });

            /* v3 I11 canary: a lesson the turn was sent is taken back when the step it recommended fails here. (The
               step it warned about failing again proves the lesson right, not wrong.) */
            if let Some(note) = lesson_note.as_deref() {
                if crate::v3::lessons::recommended(note).is_some_and(|step| step == what.trim()) && !steps.last().is_some_and(|step| step.ok) && crate::v3::on("knowledge_core") {
                    crate::v3::knowledge_core::rollback_matching(std::path::Path::new(workspace.root()), note, "the step it recommended failed while it was sent");
                    lesson_note = None;
                }
            }
        }

        /* v3 I6: files marked secret never reach a model; what did is written down (local projects). */
        if crate::v3::on("privacy") {
            let sent = crate::v3::privacy::filter(policy, workspace.root(), &reply.tool_uses, &mut results);

            if prompt.remote.is_none() {
                crate::v3::privacy::audit(workspace.root(), prompt.provider.as_deref().unwrap_or_default(), &target.model, &sent);
            }
        }

        /* v3 W9: external text marked as data, secrets replaced - before anything reaches the model. */
        crate::v3::guard_results(&prompt.session_id, &reply.tool_uses, &mut results);

        let before_dedupe: u64 = results.iter().map(|result| crate::context::tokens_of(&result.1)).sum();

        dedupe_reads(&mut seen_reads, &reply.tool_uses, &mut results);
        *saved.entry("unchanged re-reads").or_default() += before_dedupe.saturating_sub(results.iter().map(|result| crate::context::tokens_of(&result.1)).sum());

        if technique("ledger_diff") {
            *saved.entry("diffs").or_default() += crate::v3::ledger_diff::apply(&mut prior_reads, &reply.tool_uses, &mut results);
        }

        if ledger_on {
            crate::bench::ledger::record_reads(&turn_id, "main", step, read_entries(&earlier, &reply.tool_uses, &results));
        }

        messages.extend(dialect::tool_results(target.dialect, &results));

        if let Some(note) = pending_note.take() {
            dialect::append_user_text(target.dialect, &mut messages, &note);
        }

        /* v3 I3: the draft model is stuck - the strong one takes the task over from a capsule of the state. */
        if crate::v3::on("cascade") && gate.allows(crate::guard::modelgate::Purpose::Cascade) && research_session.is_none() && crate::v3::cascade::stuck(&steps) {
            if let Some((provider, model)) = crate::v3::cascade::strong().filter(|(provider, _)| prompt.provider.as_deref() != Some(provider.as_str())) {
                let failed = steps.iter().rev().find(|step| !step.ok).map(|step| format!("`{}` failed: {}", step.target, step.note)).unwrap_or_else(|| "it kept failing".to_string());
                let text = crate::v3::cascade::handover(
                    &crate::v3::failover::State {
                        goal: &request_text,
                        failed: target.model.clone(),
                        reason: &failed,
                        plan: context.open_plan_steps(),
                        changed: context.changed.iter().cloned().collect(),
                        steps: &step_log,
                        last_words: &last_words,
                    },
                    &failed,
                );
                let strong = Prompt { provider: Some(provider.clone()), model: model.clone(), text, history: Vec::new(), images: Vec::new(), ..prompt.clone() };

                if let Ok(strong_target) = resolve_target(&strong) {
                    sink.send(EngineEvent::Thinking(format!("{model} is taking over: the model before it did not get past {failed}.\n")));
                    drive(backend, &strong_target, workspace, options, None, policy, &strong, sink);

                    return;
                }
            }
        }

        steer(&mut messages);

        let folded = keep_small(target, window, &mut messages, fixed, scale);

        /* Folded output is gone from the conversation: a file read before it must be sent in full again. */
        if folded {
            seen_reads.clear();
            prior_reads.clear();
        }

        sink.send(EngineEvent::Context { used_tokens: dialect::tokens(&messages), window_tokens: window, compacted: folded });
    }

    /* A research turn that used its steps still ends with what it found. */
    if let Some(session) = &research_session {
        sink.send(EngineEvent::Sources(session.sources_json()));
    }

    /* The step budget ran out with work still going on. That is said, with the way to continue, rather
       than dressed up as a finished turn. */
    sink.send(EngineEvent::Delta(format!(
        "\n\nI stopped after {} steps, the limit for one turn. Send \"continue\" to let me keep going from here.",
        options.max_steps
    )));
    sink.send(EngineEvent::Done {
        summary: "Paused at the step limit".to_string(),
        meta: format!("{}{}{}", meta(options.max_steps, input_tokens, output_tokens, target.price) + &cached(cached_tokens), timing(model_time, tool_time), if receipt_on { crate::v3::receipt(&receipt_parts, &saved) } else { String::new() }),
        pass: None,
    });
}

/// The most one tool answer may hand back (0.16.1): `RESULT_CAP` for an API model, and for a local model
/// about a quarter of its window (four characters a token) - one page used to be bigger than the whole
/// window Ollama was loading.
fn result_cap(backend: Backend, window: u64) -> usize {
    match backend {
        Backend::Api => tools::RESULT_CAP,
        Backend::Ollama => (window as usize).clamp(2_000, tools::RESULT_CAP),
    }
}

/// The final answer of a research turn, written by the model chosen for it in Settings → Research from
/// the notes this turn's model gathered (0.16.1, opt-in). If it cannot answer, the notes are the answer.
fn synthesize(
    prompt: &Prompt,
    provider: &str,
    model: &str,
    language: &crate::understand::Reading,
    messages: &[Value],
    sink: &EventSink,
    stopped: &dyn Fn() -> bool,
) -> (u64, u64) {
    let mut asked = prompt.clone();

    asked.model = model.to_string();
    asked.provider = Some(provider.to_string());

    let backend = if provider == "ollama" { Backend::Ollama } else { Backend::Api };
    let notes = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "assistant")
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default()
        .to_string();
    let attempt = target(backend, &asked).and_then(|synth| {
        let mut conversation = messages.to_vec();

        dialect::append_user_text(
            synth.dialect,
            &mut conversation,
            "[SDC: the research is done. Write the final answer to the person's question from the notes and pages above, citing [n].]",
        );
        /* The notes were OpenAI-shaped; an Anthropic model gets them as plain text instead. */
        let conversation = if synth.dialect == Dialect::Anthropic {
            vec![dialect::user_message(&format!("{}\n\n[Research notes]\n{}", prompt.text, flatten(messages)))]
        } else {
            conversation
        };

        ask_model(backend, &synth, (&prompt.turn_id, crate::guard::modelgate::Purpose::ResearchFinal), &research::synthesis_prompt(language), &conversation, &[], sink, stopped)
    });

    match attempt {
        Ok(reply) => (reply.input_tokens, reply.output_tokens),
        Err(reason) => {
            sink.send(EngineEvent::Delta(format!("{notes}\n\n_(The final-answer model {model} could not answer: {reason}. These are the research notes.)_")));

            (0, 0)
        }
    }
}

/// A conversation as plain text: what was asked, what was said, what the tools found.
fn flatten(messages: &[Value]) -> String {
    messages
        .iter()
        .filter_map(|message| {
            let role = message["role"].as_str().unwrap_or("user");
            let text = match &message["content"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            };

            (!text.trim().is_empty()).then(|| format!("[{role}]\n{text}"))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The sentence for a request that never reached a model.
fn unreachable(backend: Backend, model: &str, reason: &str) -> String {
    match backend {
        Backend::Ollama if reason.contains("refused") || reason.contains("11434") => format!(
            "Ollama is not answering on this machine ({reason}). {}",
            if crate::host::program::resolve("ollama").is_some() {
                "SDC could not start it - restart SDC and send the prompt again."
            } else {
                "It is not installed: Settings → Environment → Ollama → Install, then send the prompt again."
            }
        ),
        Backend::Ollama => crate::engines::ollama::explain_error(reason, model),
        _ => reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn openai_target() -> Target {
        Target {
            url: "https://dashscope-us.aliyuncs.com/compatible-mode/v1/chat/completions".to_string(),
            headers: Vec::new(),
            dialect: Dialect::OpenAi,
            model: "deepseek-v4.1-flash".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        }
    }

    /// A conversation of `results` tool results of `size` characters each, after one user message.
    fn conversation(results: usize, size: usize) -> Vec<Value> {
        let mut messages = vec![dialect::user_message("fix it")];

        for index in 0..results {
            messages.push(serde_json::json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": format!("c{index}"), "type": "function", "function": { "name": "read_file", "arguments": "{}" } }] }));
            messages.push(serde_json::json!({ "role": "tool", "tool_call_id": format!("c{index}"), "content": "x".repeat(size) }));
        }

        messages
    }

    /// 1.0: folding is rare and deep. Under the ceiling nothing changes (the cached start stays the same);
    /// past it, the conversation goes down to half the ceiling in one go, so the next steps do not fold again.
    #[test]
    fn a_long_turn_folds_rarely_and_deeply_so_the_cached_start_stays_the_same() {
        let target = openai_target();
        let window = 393_216;
        /* 40 results of ~3k tokens: 120k, under the 160k ceiling. */
        let mut messages = conversation(40, 12_000);
        let before = messages.clone();

        assert!(!keep_small(&target, window, &mut messages, 5_000, 1.0));
        assert_eq!(messages, before, "under the ceiling nothing is rewritten");

        /* 60 results: 180k, past it - folded to at most half the ceiling. */
        let mut messages = conversation(60, 12_000);

        assert!(keep_small(&target, window, &mut messages, 5_000, 1.0));
        assert!(dialect::tokens(&messages) + 5_000 <= 80_000, "{}", dialect::tokens(&messages));

        /* The next steps add results without folding again until the ceiling is reached once more. */
        let folded_start = messages[..20].to_vec();

        messages.extend(conversation(10, 12_000).into_iter().skip(1));
        assert!(!keep_small(&target, window, &mut messages, 5_000, 1.0));
        assert_eq!(messages[..20], folded_start[..], "the start the provider cached is unchanged");
    }

    /// 1.0: SDC's count is scaled to the provider's own tokens, so a conversation SDC under-counts still folds.
    #[test]
    fn the_providers_own_count_decides_when_to_fold() {
        let target = openai_target();
        let mut messages = conversation(40, 12_000);

        assert!(keep_small(&target, 393_216, &mut messages, 5_000, 1.5), "125k counted is 187k for the provider");
        assert_eq!(token_scale(150_000, 100_000, 1.0), 1.5);
        assert_eq!(token_scale(0, 100_000, 1.2), 1.2, "no usage reported: the last scale stays");
        assert_eq!(token_scale(10_000_000, 100_000, 1.0), 3.0, "kept within reason");
    }

    #[test]
    fn a_request_too_long_for_the_model_is_recognised() {
        assert!(too_long("The provider refused the request (400): Range of input length should be [1, 393216]"));
        assert!(too_long("This model's maximum context length is 131072 tokens"));
        assert!(!too_long("The provider refused the request (429): rate limit"));
    }

    /// 1.0: on Model Studio a step asks for several tool calls at once and the turn's effort reaches DeepSeek V4.
    #[test]
    fn model_studio_requests_ask_for_parallel_calls_and_the_turns_effort() {
        let mut target = openai_target();
        let spec = dialect::ToolSpec { name: "read_file", description: "read", schema: serde_json::json!({ "type": "object" }) };
        let body = dialect::body(Dialect::OpenAi, &target.model, "sys", &[dialect::user_message("hi")], &[spec], false);

        assert_eq!(fast_body(&body, &target)["parallel_tool_calls"], true);
        assert!(fast_body(&body, &target).get("reasoning_effort").is_none(), "no effort asked: the model's own default");

        target.effort = Some("medium".to_string());
        assert_eq!(fast_body(&body, &target)["reasoning_effort"], "low");
        target.effort = Some("max".to_string());
        assert_eq!(fast_body(&body, &target)["reasoning_effort"], "max");

        assert_eq!(deepseek_effort("qwen3.6-flash-us", Some("low")), None, "only DeepSeek V4 takes it there");
        assert!(model_studio(&target.url));
        assert!(model_studio("https://ws-1.us-east-1.maas.aliyuncs.com/compatible-mode/v1/chat/completions"));
        assert!(!model_studio("https://api.deepseek.com/v1/chat/completions"));

        let toolless = dialect::body(Dialect::OpenAi, &target.model, "sys", &[dialect::user_message("hi")], &[], false);

        assert!(fast_body(&toolless, &target).get("parallel_tool_calls").is_none(), "no tools, no parallel calls");
    }

    /// 1.0: a level a Model Studio endpoint refused is not sent to it again.
    #[test]
    fn a_refused_request_level_is_remembered_per_endpoint_and_model() {
        let url = "https://example.aliyuncs.com/levels-test";

        assert_eq!(level_cap(url, "m"), 2);
        lower_cap(url, "m", 1);
        assert_eq!(level_cap(url, "m"), 1);
        assert_eq!(level_cap(url, "other"), 2);
    }

    /// 0.20: where a turn's time went is said after its totals - and nothing is said for a quick turn.
    #[test]
    fn a_slow_turn_says_where_its_time_went() {
        let secs = std::time::Duration::from_secs_f64;

        assert_eq!(timing(secs(0.5), secs(0.3)), "");
        assert_eq!(timing(secs(18.2), secs(6.1)), " · model 18s · tools 6.1s");
        assert_eq!(timing(secs(75.0), secs(2.0)), " · model 1m 15s · tools 2.0s");
    }

    fn git(root: &std::path::Path, args: &[&str]) -> bool {
        std::process::Command::new("git").args(args).current_dir(root).output().is_ok_and(|output| output.status.success())
    }

    /// 0.20: a change across files is shown back as a diff of *those* files; a small one is not, and neither is
    /// a folder with nothing to diff against.
    #[test]
    fn a_change_across_files_is_shown_back_for_a_strict_reread() {
        let root = std::env::temp_dir().join(format!("sdc-review-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        if !git(&root, &["init", "-q"]) {
            return; /* no git on this machine: nothing to diff with */
        }

        for name in ["a.ts", "b.ts", "other.ts"] {
            std::fs::write(root.join(name), "one\ntwo\nthree\n").unwrap();
        }

        assert!(git(&root, &["add", "."]));
        assert!(git(&root, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "first"]));

        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let changed = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<HashSet<String>>();

        /* One small change: no review. */
        std::fs::write(root.join("a.ts"), "one\nTWO\nthree\n").unwrap();
        assert!(review_note(&workspace, &changed(&["a.ts"])).is_none());

        /* Two files: reviewed, and the person's own change in another file is not part of it. */
        std::fs::write(root.join("b.ts"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(root.join("other.ts"), "someone else's work\n").unwrap();

        let note = review_note(&workspace, &changed(&["a.ts", "b.ts"])).expect("two files are reviewed");

        assert!(note.contains("2 file(s)") && note.contains("+TWO") && note.contains("strict reviewer"), "{note}");
        assert!(!note.contains("someone else's work"), "only the agent's files: {note}");

        /* A note or an image is not code: nothing to review. */
        assert!(review_note(&workspace, &changed(&["notes.md", "logo.png"])).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

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

        run(Backend::Ollama, Options { autonomy: Autonomy::Ask, max_steps: 5, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());

        assert!(matches!(&recorder.events()[..], [EngineEvent::Failed(reason)] if reason.contains("Open a folder")));
    }

    #[test]
    fn a_sub_agent_explores_with_the_providers_fast_model_only_when_the_turns_model_is_slower() {
        let rows = vec![
            serde_json::json!({ "id": "claude-opus-5-5", "tier": "deep" }),
            serde_json::json!({ "id": "claude-sonnet-5", "tier": "balanced" }),
            serde_json::json!({ "id": "claude-haiku-4-5", "tier": "fast" }),
        ];

        assert_eq!(pick_fast(&rows, "claude-opus-5-5").as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(pick_fast(&rows, "claude-sonnet-5").as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(pick_fast(&rows, "claude-haiku-4-5"), None, "already fast");
        assert_eq!(pick_fast(&rows, "claude-unknown"), None, "a model the catalogue cannot place keeps itself");
        assert_eq!(pick_fast(&rows[..2], "claude-opus-5-5"), None, "no fast sibling");
    }

    #[test]
    fn an_unchanged_re_read_is_one_line_and_a_changed_one_is_whole() {
        let read = |id: &str, path: &str| dialect::ToolUse { id: id.into(), name: "read_file".into(), input: serde_json::json!({ "path": path }) };
        let file = "1\tfn main() {}\n".repeat(60);
        let mut seen = std::collections::HashMap::new();

        let mut first = vec![(String::from("a"), file.clone(), false, None)];
        dedupe_reads(&mut seen, &[read("a", "src/main.rs")], &mut first);
        assert_eq!(first[0].1, file, "the first read is the file");

        let mut again = vec![(String::from("b"), file.clone(), false, None)];
        dedupe_reads(&mut seen, &[read("b", "./src/main.rs")], &mut again);
        assert_eq!(again[0].1, UNCHANGED_READ, "the same text under the same path is one line");

        let changed = file.replace("main", "start");
        let mut after = vec![(String::from("c"), changed.clone(), false, None)];
        dedupe_reads(&mut seen, &[read("c", "src/main.rs")], &mut after);
        assert_eq!(after[0].1, changed, "after a change the whole file comes back");

        let mut failed = vec![(String::from("d"), changed.clone(), true, None)];
        dedupe_reads(&mut seen, &[read("d", "src/main.rs")], &mut failed);
        assert_eq!(failed[0].1, changed, "an error is never replaced");
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

    /// Like [`provider`], but each entry is the whole HTTP response (status line and all) rather than an
    /// always-200 SSE body - for a reply the provider refuses outright, such as a content filter's 400.
    fn raw_provider(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut bodies = Vec::new();

            for response in responses {
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

                write!(socket, "{response}").unwrap();
            }

            bodies
        });

        (url, server)
    }

    /// A 400 in Alibaba's shape: the provider's own filter refused the reply before anyone saw it.
    fn blocked() -> String {
        let body = serde_json::json!({
            "error": { "message": "<400> InternalError.Algo.DataInspectionFailed: Output data may contain inappropriate content." }
        })
        .to_string();

        format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    /// A normal 200 SSE reply, for [`raw_provider`].
    fn ok(sse: String) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{sse}")
    }

    fn chunk(delta: Value) -> String {
        format!("data: {}\n\n", serde_json::json!({ "choices": [{ "delta": delta }] }))
    }

    fn call(id: &str, name: &str, arguments: Value) -> String {
        chunk(serde_json::json!({ "tool_calls": [{ "index": 0, "id": id, "function": { "name": name, "arguments": arguments.to_string() } }] }))
            + "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n"
            + "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20}}\n\ndata: [DONE]\n\n"
    }

    /// Several tool calls in one reply, the way OpenAI-shaped providers stream them: one delta, an index each.
    fn calls(list: Vec<(&str, &str, Value)>) -> String {
        let deltas: Vec<Value> = list
            .into_iter()
            .enumerate()
            .map(|(index, (id, name, arguments))| serde_json::json!({ "index": index, "id": id, "function": { "name": name, "arguments": arguments.to_string() } }))
            .collect();

        chunk(serde_json::json!({ "tool_calls": deltas }))
            + "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n"
            + "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20}}\n\ndata: [DONE]\n\n"
    }

    /// 0.22, end to end through the real loop: a file read twice, unchanged, reaches the model in full once and as
    /// one line the second time; after an edit, the next read is the whole new file again.
    #[test]
    fn an_unchanged_re_read_reaches_the_model_as_one_line_and_a_read_after_an_edit_in_full() {
        let root = std::env::temp_dir().join(format!("sdc-agent-reread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let original = (1..=80).map(|line| format!("line {line}: the quick brown fox jumps over the lazy dog\n")).collect::<String>();
        std::fs::write(root.join("notes.txt"), &original).unwrap();

        let (url, server) = provider(vec![
            call("r1", "read_file", serde_json::json!({ "path": "notes.txt" })),
            call("r2", "read_file", serde_json::json!({ "path": "notes.txt" })),
            call("e1", "edit_file", serde_json::json!({ "path": "notes.txt", "old_text": "line 42: the quick", "new_text": "line 42: the QUICK" })),
            call("r3", "read_file", serde_json::json!({ "path": "notes.txt" })),
            chunk(serde_json::json!({ "content": "Done." })) + "data: [DONE]\n\n",
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-reread".into(),
            text: "read notes.txt twice, then change line 42".into(),
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

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());

        let bodies = server.join().unwrap();
        let result = |id: &str| -> String {
            bodies
                .last()
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|message| message["role"] == "tool" && message["tool_call_id"] == id)
                .and_then(|message| message["content"].as_str())
                .unwrap_or_default()
                .to_string()
        };

        assert!(result("r1").contains("line 80: the quick brown fox"), "the first read is the whole file");
        assert_eq!(result("r2"), UNCHANGED_READ, "the unchanged re-read is one line");
        assert!(result("r3").contains("line 42: the QUICK"), "after the edit the whole new file comes back");
        assert!(result("r1").len() > 20 * result("r2").len(), "and the saving is real: {} chars instead of {}", result("r2").len(), result("r1").len());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 0.20: "read a, read b, write c, read c" in one reply - the two reads run together, then the write, then
    /// the read, in the order the model wrote them, so the last read sees what the write made.
    #[test]
    fn a_read_after_a_write_in_the_same_reply_sees_the_write() {
        let root = std::env::temp_dir().join(format!("sdc-agent-mixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "alpha\n").unwrap();
        std::fs::write(root.join("b.txt"), "bravo\n").unwrap();

        let (url, server) = provider(vec![
            calls(vec![
                ("c1", "read_file", serde_json::json!({ "path": "a.txt" })),
                ("c2", "read_file", serde_json::json!({ "path": "b.txt" })),
                ("c3", "write_file", serde_json::json!({ "path": "c.txt", "content": "charlie\n" })),
                ("c4", "read_file", serde_json::json!({ "path": "c.txt" })),
            ]),
            chunk(serde_json::json!({ "content": "Read three files." })) + "data: [DONE]\n\n",
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-mixed".into(),
            text: "write c.txt after reading a.txt and b.txt".into(),
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

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 6, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());

        let bodies = server.join().unwrap();
        let results: Vec<(String, String)> = bodies[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .map(|message| (message["tool_call_id"].as_str().unwrap().to_string(), message["content"].as_str().unwrap().to_string()))
            .collect();

        assert_eq!(results.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["c1", "c2", "c3", "c4"], "answered in the order asked");
        assert!(results[0].1.contains("alpha") && results[1].1.contains("bravo"), "{results:?}");
        assert!(results[3].1.contains("charlie"), "the read after the write sees the write: {results:?}");
        assert!(bodies[0]["messages"][1]["content"].as_str().unwrap().contains("write c.txt after reading"), "the person's words reach the model");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 0.20: the model is handed a map of the folder - top level and the project's own check - with the
    /// person's words, except for a plain question.
    #[test]
    fn the_model_gets_a_map_of_the_project_except_for_a_quick_question() {
        let ask = |text: &str, label: &str| -> String {
            let root = std::env::temp_dir().join(format!("sdc-agent-map-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::write(root.join("package.json"), r#"{"scripts":{"test":"vitest run"}}"#).unwrap();

            let (url, server) = provider(vec![chunk(serde_json::json!({ "content": "ok" })) + "data: [DONE]\n\n"]);
            let target = Target {
                url,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                dialect: Dialect::OpenAi,
                model: "local-test".to_string(),
                price: None,
                thinking: false,
                effort: None,
                local_context: None,
                local_window: false,
            };
            let prompt = Prompt {
                session_id: "s1".into(),
                turn_id: format!("turn-map-{label}"),
                text: text.into(),
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

            drive(Backend::Api, &target, &Workspace::new(root.to_str().unwrap(), None), Options { autonomy: Autonomy::Auto, max_steps: 3, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());

            let sent = server.join().unwrap()[0]["messages"][1]["content"].as_str().unwrap().to_string();
            let _ = std::fs::remove_dir_all(&root);

            sent
        };

        let task = ask("add a dark mode toggle to the header", "task");

        assert!(task.contains("a map of the project") && task.contains("src/") && task.contains("package.json"), "{task}");
        assert!(task.ends_with("add a dark mode toggle to the header"), "the person's words come last: {task}");

        let question = ask("why does the header flicker?", "question");

        assert_eq!(question, "why does the header flicker?", "a quick question is sent as it was typed");
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
            local_context: None,
            local_window: false,
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

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 10, auto_check: false, research: None }, Some(checkpoint), &Default::default(), &prompt, &recorder.sink());

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

    /// v3 W1: the ledger sums to the provider's own count on every step, names the tools each reply called - and
    /// the requests are byte-for-byte the same with the ledger on and off.
    #[test]
    fn the_ledger_matches_the_providers_usage_and_leaves_the_request_unchanged() {
        let run = |turn: &str, capture: bool| {
            let root = std::env::temp_dir().join(format!("sdc-ledger-{turn}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("notes.txt"), "a line of notes\n".repeat(40)).unwrap();

            let usage = |input: u64, cached: u64| format!("data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":{input},\"completion_tokens\":7,\"prompt_tokens_details\":{{\"cached_tokens\":{cached}}}}}}}\n\ndata: [DONE]\n\n");
            let (url, server) = provider(vec![
                chunk(serde_json::json!({ "tool_calls": [{ "index": 0, "id": "c1", "function": { "name": "read_file", "arguments": "{\"path\":\"notes.txt\"}" } }] }))
                    + "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n"
                    + &usage(1_234, 0),
                chunk(serde_json::json!({ "content": "The notes repeat one line." })) + &usage(1_777, 1_200),
            ]);
            let target = Target {
                url,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                dialect: Dialect::OpenAi,
                model: "local-test".to_string(),
                price: None,
                thinking: false,
                effort: None,
                local_context: None,
                local_window: false,
            };
            let workspace = Workspace::new(root.to_str().unwrap(), None);
            let mut prompt = prompt_in(&root, turn, "[SDC wrapper] [The person's message] what is in notes.txt? explain the file in detail", "local-test");

            prompt.history = vec![crate::engines::Message::user("earlier"), crate::engines::Message::assistant("ok")];

            if capture {
                crate::bench::ledger::capture(turn);
            }

            drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 10, auto_check: false, research: None }, None, &Default::default(), &prompt, &crate::engines::Recorder::new().sink());

            let bodies = server.join().unwrap();
            let _ = std::fs::remove_dir_all(&root);

            (bodies, crate::bench::ledger::take(turn))
        };

        let (with_ledger, all) = run("ledger-on", true);
        let (without, none) = run("ledger-off", false);
        let (reads, records): (Vec<_>, Vec<_>) = all.into_iter().partition(|record| record.role.ends_with("-reads"));

        assert!(none.is_empty());
        assert_eq!(reads.len(), 1);
        assert_eq!((reads[0].reads[0].path.as_str(), reads[0].reads[0].repeat, reads[0].reads[0].deduped), ("notes.txt", false, false));
        assert!(reads[0].reads[0].tokens > 50);
        assert_eq!(records.len(), 2, "one record per request");
        assert_eq!(records.iter().map(|record| record.parts.iter().sum::<u64>()).collect::<Vec<_>>(), [1_234, 1_777], "the parts sum to the provider's input");
        assert_eq!(records[1].cached_tokens, 1_200);
        assert_eq!(records[0].tools_called, ["read_file"]);
        assert!(records[1].counted[4] > 0, "the second request carries the file as a tool result");
        assert!(records[0].counted[3] > 0 && records[0].counted[5] > 0 && records[0].counted[0] > 0 && records[0].counted[1] > 0);

        /* Byte-identical apart from the folder name, which the two runs cannot share. */
        let same = |bodies: &[Value], turn: &str| bodies.iter().map(|body| body.to_string().replace(&format!("sdc-ledger-{turn}-"), "sdc-ledger-X-")).collect::<Vec<_>>();

        assert_eq!(same(&with_ledger, "ledger-on"), same(&without, "ledger-off"), "the ledger does not change a request");
    }

    /// v3 W9: a key the agent reads never reaches the provider, and a command the model writes with the placeholder
    /// gets the real key.
    #[test]
    fn a_secret_the_agent_reads_never_reaches_the_provider_but_is_written_back() {
        let root = std::env::temp_dir().join(format!("sdc-redact-e2e-{}", std::process::id()));
        /* Built from pieces: no scanner should take a test fixture for a real key. */
        let key = &format!("{}{}", "sk-proj-", "FAKEfakeFAKEfake0123456789abcdefABCDEF");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.txt"), format!("model = small\nOPENAI_API_KEY={key}\n")).unwrap();

        let (url, server) = provider(vec![
            call("c1", "read_file", serde_json::json!({ "path": "config.txt" })),
            /* Through a command: a `write_file` with a key asks the person first (`tools::guard_write`), which a test cannot answer. */
            call("c2", "run_command", serde_json::json!({ "command": if cfg!(windows) { "echo OPENAI_API_KEY=‹secret:1›> copy.txt" } else { "printf 'OPENAI_API_KEY=%s\\n' ‹secret:1› > copy.txt" } })),
            chunk(serde_json::json!({ "content": "Copied." })) + "data: [DONE]\n\n",
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let mut prompt = prompt_in(&root, "turn-redact-e2e", "copy the key line of config.txt into copy.txt", "local-test");

        prompt.session_id = "chat-redact-e2e".into();

        /* Phase 0.3: reading a file with a key in it asks the person first; this person says yes. */
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let answering = {
            let stop = stop.clone();

            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    for call in 0..12 {
                        gate::resolve(&format!("perm-turn-redact-e2e-{call}"), "allow");
                    }

                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            })
        };

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 6, auto_check: false, research: None }, None, &Default::default(), &prompt, &crate::engines::Recorder::new().sink());
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        answering.join().unwrap();

        let bodies = server.join().unwrap();

        assert!(bodies.iter().all(|body| !body.to_string().contains(key)), "the key never left the machine");
        assert!(bodies[1].to_string().contains("‹secret:1›"), "the model saw the placeholder");
        assert!(std::fs::read_to_string(root.join("copy.txt")).unwrap().contains(&format!("OPENAI_API_KEY={key}")), "the command got the real key");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// v3 W5: with `stable_prefix` the system prompt has no date (the message has it); with `tool_pruning` a plain
    /// change is offered fewer tools, and a tool the model asks for anyway joins the list.
    #[test]
    fn a_stable_prefix_and_pruned_tools_change_only_what_they_say() {
        let run = |turn: &str, features: &[&str]| {
            let root = std::env::temp_dir().join(format!("sdc-w5-{turn}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();

            let (url, server) = provider(vec![
                call("c1", "web_fetch", serde_json::json!({ "url": "http://127.0.0.1:9/nothing" })),
                chunk(serde_json::json!({ "content": "ok" })) + "data: [DONE]\n\n",
            ]);
            let target = Target {
                url,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                dialect: Dialect::OpenAi,
                model: "local-test".to_string(),
                price: None,
                thinking: false,
                effort: None,
                local_context: None,
                local_window: false,
            };
            let workspace = Workspace::new(root.to_str().unwrap(), None);
            let prompt = prompt_in(&root, turn, "rename the variable total to sum in main.rs and check the build", "local-test");

            crate::v3::forcing(features, || {
                drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 4, auto_check: false, research: None }, None, &Default::default(), &prompt, &crate::engines::Recorder::new().sink())
            });

            let _ = std::fs::remove_dir_all(&root);
            server.join().unwrap()
        };
        let plain = run("w5-off", &[]);
        let tuned = run("w5-on", &["stable_prefix", "tool_pruning"]);
        let system = |bodies: &[Value]| bodies[0]["messages"][0]["content"].as_str().unwrap_or_default().to_string();
        let names = |body: &Value| body["tools"].as_array().unwrap().iter().map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string()).collect::<Vec<_>>();

        assert!(system(&plain).contains("Today: ") && !system(&tuned).contains("Today: "));
        assert!(tuned[0]["messages"].to_string().contains("[Today: "), "the date moved into the message");
        assert!(names(&tuned[0]).len() < names(&plain[0]).len());
        assert!(!names(&tuned[0]).contains(&"web_fetch".to_string()) && names(&tuned[1]).contains(&"web_fetch".to_string()), "asked for, then offered");
    }

    /// v3 I5: `@budget 300` - told at the start, asked to converge at 80%, stopped at 100% with the work kept.
    #[test]
    fn a_budget_is_told_converged_at_eighty_percent_and_stopped_at_full() {
        let root = std::env::temp_dir().join(format!("sdc-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "a\n").unwrap();

        /* Each reply reports 100 in + 20 out: 120, 240 (80%), 360 (over). */
        let (url, server) = provider(vec![
            call("c1", "read_file", serde_json::json!({ "path": "a.txt" })),
            call("c2", "read_file", serde_json::json!({ "path": "a.txt" })),
            call("c3", "read_file", serde_json::json!({ "path": "a.txt" })),
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = prompt_in(&root, "turn-budget", "look at a.txt @budget 300", "local-test");
        let recorder = crate::engines::Recorder::new();

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 10, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());

        let bodies = server.join().unwrap();

        assert_eq!(bodies.len(), 3, "no fourth request past the budget");
        assert!(bodies[0].to_string().contains("budget of 300 tokens"));
        assert!(bodies[2].to_string().contains("80% of the budget is used"));
        assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::Done { summary, .. } if summary == "Paused at the budget")));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// v3 I9 (fault injection): the first provider refuses outright; the turn continues on the next model of the
    /// failover order, from a capsule of the state - and finishes.
    #[test]
    fn a_failed_provider_hands_the_task_to_the_next_model_in_a_capsule() {
        let root = std::env::temp_dir().join(format!("sdc-failover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let refused = {
            let body = serde_json::json!({ "error": { "message": "Incorrect API key provided" } }).to_string();

            format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        };
        let (first_url, first) = raw_provider(vec![ok(call("c1", "write_file", serde_json::json!({ "path": "half.txt", "content": "started\n" }))), refused]);
        let (second_url, second) = provider(vec![chunk(serde_json::json!({ "content": "Finished from the capsule." })) + "data: [DONE]\n\n"]);
        let fake = |url: String| Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        };

        FAKE_TARGETS.with(|targets| targets.borrow_mut().insert("backup".to_string(), fake(second_url)));
        crate::v3::failover::set_order_for_test(vec![("backup".to_string(), "backup-model".to_string())]);

        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let mut prompt = prompt_in(&root, "turn-failover", "make half.txt and finish the job", "local-test");

        prompt.provider = Some("primary".into());

        let recorder = crate::engines::Recorder::new();

        /* The failover is what the contract switches off under a pin, so this proof of the failover itself runs with the
           contract off - which is also the proof that switch off is every earlier version's behaviour. */
        crate::v3::ablating(&["models.contract"], || drive(Backend::Api, &fake(first_url), &workspace, Options { autonomy: Autonomy::Auto, max_steps: 6, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink()));
        crate::v3::failover::set_order_for_test(Vec::new());

        let (_, handed) = (first.join().unwrap(), second.join().unwrap());
        let capsule = handed[0]["messages"].to_string();

        assert!(capsule.contains("SDC failover") && capsule.contains("make half.txt") && capsule.contains("half.txt"), "the capsule carries the goal and the changed file: {capsule}");
        assert!(capsule.contains("write_file half.txt → ok"));
        assert!(recorder.events().contains(&EngineEvent::Delta("Finished from the capsule.".into())));
        assert!(!recorder.events().iter().any(|event| matches!(event, EngineEvent::Failed(_))), "no failure reached the person");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// v3 I6 canary: a file marked secret is read, grepped for and printed by a command - its canary never reaches
    /// the provider, and the audit names only the files that did go.
    #[test]
    fn a_file_marked_secret_never_reaches_the_provider() {
        let root = std::env::temp_dir().join(format!("sdc-privacy-{}", std::process::id()));
        let canary = "CANARY-7f3a9c-PRIVATE";
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/keys.txt"), format!("token = {canary}\n")).unwrap();
        std::fs::write(root.join("app.txt"), "uses the token from the vault\n").unwrap();

        let (url, server) = provider(vec![
            calls(vec![
                ("c1", "read_file", serde_json::json!({ "path": "vault/keys.txt" })),
                ("c2", "read_file", serde_json::json!({ "path": "app.txt" })),
                ("c3", "grep", serde_json::json!({ "pattern": "CANARY|token" })),
            ]),
            call("c4", "run_command", serde_json::json!({ "command": if cfg!(windows) { "type vault\\keys.txt" } else { "cat vault/keys.txt" } })),
            chunk(serde_json::json!({ "content": "done" })) + "data: [DONE]\n\n",
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        };
        let policy = crate::trust::policy::Policy::parse("secret_paths = [\"vault/**\"]\n", "test");
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = prompt_in(&root, "turn-privacy", "where is the token used?", "local-test");

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 6, auto_check: false, research: None }, None, &policy, &prompt, &crate::engines::Recorder::new().sink());

        let bodies = server.join().unwrap();

        assert!(bodies.iter().all(|body| !body.to_string().contains(canary)), "the canary never left the machine");
        assert!(bodies[1].to_string().contains("marked secret"));

        let audit = std::fs::read_to_string(root.join(crate::v3::privacy::AUDIT_FILE)).unwrap();

        assert!(audit.contains("app.txt") && !audit.contains("vault/keys.txt"), "{audit}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// v3 W4/L1/I8: with the knowledge switches on, the request carries the English brief, the project's map and the
    /// two code-intelligence tools; with them off it reads exactly as it did before.
    #[test]
    fn the_knowledge_switches_change_the_request_only_when_they_are_on() {
        let run = |features: &[&str], tag: &str| {
            let root = std::env::temp_dir().join(format!("sdc-knowledge-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::write(root.join("src/orders.ts"), "export function createOrder() {}\nexport function listOrders() {}\n").unwrap();

            let (url, server) = provider(vec![chunk(serde_json::json!({ "content": "done" })) + "data: [DONE]\n\n"]);
            let target = Target {
                url,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                dialect: Dialect::OpenAi,
                model: "local-test".to_string(),
                price: None,
                thinking: false,
                effort: None,
                local_context: None,
                local_window: false,
            };
            let workspace = Workspace::new(root.to_str().unwrap(), None);
            let prompt = prompt_in(&root, "turn-knowledge", "kaj ta koro orders.ts e (do the work on orders)", "local-test");

            crate::v3::forcing(features, || {
                drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 2, auto_check: false, research: None }, None, &Default::default(), &prompt, &crate::engines::Recorder::new().sink())
            });

            let _ = std::fs::remove_dir_all(&root);
            server.join().unwrap()
        };
        let off = run(&[], "off");
        let on = run(&["interlingua", "repomap", "intel"], "on");
        let asked = |bodies: &[Value]| bodies[0]["messages"].to_string();
        let tools = |body: &Value| body["tools"].as_array().unwrap().iter().map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string()).collect::<Vec<_>>();

        assert!(!asked(&off).contains("English brief"), "off: nothing is added");
        assert!(!tools(&off[0]).contains(&"find_symbol".to_string()));
        assert!(asked(&on).contains("English brief of the person's message"), "on: one English brief: {}", asked(&on));
        assert!(asked(&on).contains("SDC code map"), "and the project's declarations");
        assert!(tools(&on[0]).contains(&"find_symbol".to_string()) && tools(&on[0]).contains(&"symbols".to_string()));
    }

    /// v3 I3/W7a/W7b: `/plan` changes nothing, the old content of a changed file is kept and a turn can be taken
    /// back, and a draft that keeps failing hands the task to the strong model.
    #[test]
    fn a_turn_is_planned_checkpointed_and_handed_over() {
        let root = std::env::temp_dir().join(format!("sdc-batch2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("orders.ts"), "the old body\n").unwrap();

        let fake = |url: String, model: &str| Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: model.to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);

        /* W7b: plan mode refuses the edit, says why, and answers with the plan. */
        let (url, server) = provider(vec![
            call("c1", "edit_file", serde_json::json!({ "path": "orders.ts", "old": "old", "new": "new" })),
            chunk(serde_json::json!({ "content": "here is the plan" })) + "data: [DONE]\n\n",
        ]);
        let prompt = prompt_in(&root, "turn-plan", "/plan add pagination to the orders list", "local-test");
        let recorder = crate::engines::Recorder::new();

        drive(Backend::Api, &fake(url, "main-model"), &workspace, Options { autonomy: Autonomy::Auto, max_steps: 4, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());

        let bodies = server.join().unwrap();

        assert!(bodies[0]["messages"].to_string().contains("plan mode") && bodies[0]["messages"].to_string().contains("change nothing"));
        assert!(bodies[1].to_string().contains("it was not run"), "the edit was refused: {}", bodies[1]["messages"]);
        assert_eq!(std::fs::read_to_string(root.join("orders.ts")).unwrap(), "the old body\n", "and the file is untouched");
        assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::Delta(text) if text.contains("here is the plan"))));

        /* W7a: a file a turn changes is kept by its address, and the turn can be taken back. */
        let (url, server) = provider(vec![
            call("c1", "write_file", serde_json::json!({ "path": "orders.ts", "content": "the new body\n" })),
            chunk(serde_json::json!({ "content": "done" })) + "data: [DONE]\n\n",
        ]);
        let prompt = prompt_in(&root, "turn-write", "rewrite orders.ts", "local-test");

        drive(Backend::Api, &fake(url, "main-model"), &workspace, Options { autonomy: Autonomy::Auto, max_steps: 4, auto_check: false, research: None }, None, &Default::default(), &prompt, &crate::engines::Recorder::new().sink());

        let _ = server.join().unwrap();

        assert_eq!(std::fs::read_to_string(root.join("orders.ts")).unwrap(), "the new body\n");
        assert_eq!(crate::v3::cas::undo_turn(&root, "turn-write"), 1);
        assert_eq!(std::fs::read_to_string(root.join("orders.ts")).unwrap(), "the old body\n", "the turn was taken back");

        /* I3: the draft fails the same step twice; the strong model takes over from the capsule and finishes. */
        let missing = call("c1", "read_file", serde_json::json!({ "path": "nothing-here.txt" }));
        let (draft_url, draft_server) = raw_provider(vec![ok(missing.clone()), ok(missing)]);
        let (strong_url, strong_server) = provider(vec![chunk(serde_json::json!({ "content": "finished from the handover" })) + "data: [DONE]\n\n"]);

        crate::v3::cascade::set_strong_for_test(Some(("backup".into(), "backup-model".into())));
        FAKE_TARGETS.with(|targets| targets.borrow_mut().insert("backup".to_string(), fake(strong_url, "backup-model")));

        let prompt = prompt_in(&root, "turn-cascade", "make the orders tests pass", "local-test");
        let recorder = crate::engines::Recorder::new();

        crate::v3::ablating(&["models.contract"], || drive(Backend::Api, &fake(draft_url, "draft-model"), &workspace, Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink()));
        crate::v3::cascade::set_strong_for_test(None);
        FAKE_TARGETS.with(|targets| targets.borrow_mut().clear());

        let (asked, handed) = (draft_server.join().unwrap(), strong_server.join().unwrap());

        assert_eq!(asked.len(), 2, "the draft was given two tries");
        assert!(handed[0]["messages"].to_string().contains("got stuck") && handed[0]["messages"].to_string().contains("make the orders tests pass"));
        assert!(recorder.events().contains(&EngineEvent::Delta("finished from the handover".into())));

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
            local_context: None,
            local_window: false,
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

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Ask, max_steps: 2, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());
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

    /// The 0.15.6 report: a reply the provider's content filter blocked is asked again rather than
    /// failing the turn, up to [`FILTER_RETRIES`] times - and each one that goes through resets the count,
    /// so the turn still finishes.
    #[test]
    fn a_blocked_reply_is_asked_again_and_the_turn_still_finishes() {
        let root = std::env::temp_dir().join(format!("sdc-agent-filter-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let (url, server) = raw_provider(vec![
            blocked(),
            blocked(),
            ok(chunk(serde_json::json!({ "content": "Done." })) + "data: [DONE]\n\n"),
        ]);
        let target = Target { url, headers: Vec::new(), dialect: Dialect::OpenAi, model: "local-test".to_string(), price: None, thinking: false, effort: None, local_context: None, local_window: false };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-filter-ok".into(),
            text: "summarise this log".into(),
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

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Ask, max_steps: 10, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());
        server.join().unwrap();

        let events = recorder.events();
        let cards: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                EngineEvent::ToolStarted { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(cards, ["Content filter", "Content filter"], "one retry card per blocked reply");
        assert!(events.contains(&EngineEvent::Delta("Done.".into())), "the turn must still reach the model's answer");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Past [`FILTER_RETRIES`] blocked replies in a row, the limiter stops asking again and fails the
    /// turn instead of retrying forever.
    #[test]
    fn blocked_replies_past_the_filter_limit_fail_the_turn() {
        let root = std::env::temp_dir().join(format!("sdc-agent-filter-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let (url, server) = raw_provider(vec![blocked(); FILTER_RETRIES + 1]);
        let target = Target { url, headers: Vec::new(), dialect: Dialect::OpenAi, model: "local-test".to_string(), price: None, thinking: false, effort: None, local_context: None, local_window: false };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-filter-limit".into(),
            text: "summarise this log".into(),
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

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Ask, max_steps: 10, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());
        server.join().unwrap();

        let events = recorder.events();
        let cards: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                EngineEvent::ToolStarted { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(cards.len(), FILTER_RETRIES, "exactly FILTER_RETRIES retries before the limiter gives up");
        assert!(
            matches!(events.last(), Some(EngineEvent::Failed(reason)) if reason.to_lowercase().contains("datainspectionfailed")),
            "{events:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 0.15.6: a reply that only reads runs its reads side by side, and each read still gets its own
    /// card and its own answer, in the order the model asked.
    #[test]
    fn a_reply_of_reads_runs_them_together_and_answers_each() {
        let root = std::env::temp_dir().join(format!("sdc-agent-reads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "alpha").unwrap();
        std::fs::write(root.join("b.txt"), "bravo").unwrap();

        let two_reads = chunk(serde_json::json!({ "tool_calls": [
                { "index": 0, "id": "r1", "function": { "name": "read_file", "arguments": "{\"path\":\"a.txt\"}" } },
                { "index": 1, "id": "r2", "function": { "name": "read_file", "arguments": "{\"path\":\"b.txt\"}" } }
            ] }))
            + "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        let (url, server) = provider(vec![two_reads, chunk(serde_json::json!({ "content": "Done." })) + "data: [DONE]\n\n"]);
        let target = Target { url, headers: Vec::new(), dialect: Dialect::OpenAi, model: "local-test".to_string(), price: None, thinking: false, effort: None, local_context: None, local_window: false };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-reads".into(),
            text: "read both".into(),
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

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Ask, max_steps: 10, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());

        let bodies = server.join().unwrap();
        let mut ids: Vec<String> = recorder
            .events()
            .iter()
            .filter_map(|event| match event {
                EngineEvent::ToolStarted { call_id, .. } => Some(call_id.clone()),
                _ => None,
            })
            .collect();

        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 2, "each read keeps its own card: {ids:?}");

        let sent = bodies[1]["messages"].to_string();
        let (alpha, bravo) = (sent.find("alpha").expect("a.txt's text"), sent.find("bravo").expect("b.txt's text"));

        assert!(alpha < bravo, "the answers go back in the order the model asked");

        let _ = std::fs::remove_dir_all(&root);
    }

    fn prompt_in(root: &std::path::Path, turn: &str, text: &str, model: &str) -> Prompt {
        Prompt {
            session_id: "s1".into(),
            turn_id: turn.into(),
            text: text.into(),
            model: model.into(),
            provider: None,
            history: Vec::new(),
            project_root: Some(root.to_str().unwrap().to_string()),
            remote: None,
            autonomy: Default::default(),
            resume: None,
            images: Vec::new(),
            effort: None,
        }
    }

    /// Ollama's own NDJSON, as one HTTP response.
    fn ndjson(lines: &[Value]) -> String {
        let body = lines.iter().map(|line| line.to_string()).collect::<Vec<_>>().join("\n") + "\n";

        format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n{body}")
    }

    /// 0.16.1, the plan's missing proof: a local model calls a tool through Ollama's own `/api/chat`,
    /// the request names its context (`num_ctx`), the tool's answer goes back in the native shape, and
    /// the turn finishes with the model's words.
    #[test]
    fn a_local_model_uses_tools_over_ollamas_own_endpoint_with_num_ctx() {
        let root = std::env::temp_dir().join(format!("sdc-agent-ollama-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("hello.txt"), "hi from the folder\n").unwrap();

        let (url, server) = raw_provider(vec![
            ndjson(&[
                serde_json::json!({ "message": { "role": "assistant", "content": "", "thinking": "Read it." }, "done": false }),
                serde_json::json!({ "message": { "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": "read_file", "arguments": { "path": "hello.txt" } } }] }, "done": false }),
                serde_json::json!({ "message": { "role": "assistant", "content": "" }, "done": true, "done_reason": "stop", "prompt_eval_count": 900, "eval_count": 12 }),
            ]),
            ndjson(&[
                serde_json::json!({ "message": { "role": "assistant", "content": "It says hi." }, "done": false }),
                serde_json::json!({ "message": { "role": "assistant", "content": "" }, "done": true, "done_reason": "stop", "prompt_eval_count": 950, "eval_count": 5 }),
            ]),
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "qwen3.5:9b".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: Some(8_192),
            local_window: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = prompt_in(&root, "turn-agent-ollama", "what does hello.txt say?", "qwen3.5:9b");
        let recorder = crate::engines::Recorder::new();

        drive(Backend::Ollama, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 5, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());

        let bodies = server.join().unwrap();
        let events = recorder.events();

        assert_eq!(bodies[0]["options"]["num_ctx"], 8_192, "the context is named on the request");
        assert!(bodies[0].get("stream_options").is_none(), "nothing the native endpoint does not take");
        assert!(bodies[0]["tools"].as_array().map(Vec::len).unwrap_or(0) > 0, "the tools go with it");

        let second = bodies[1]["messages"].as_array().unwrap();
        let asked = second.iter().find(|message| message["role"] == "assistant").expect("the model's call goes back");
        let answered = second.iter().find(|message| message["role"] == "tool").expect("the tool's answer goes back");

        assert_eq!(asked["tool_calls"][0]["function"]["arguments"]["path"], "hello.txt", "arguments as an object");
        assert_eq!(answered["tool_name"], "read_file");
        assert!(answered["content"].as_str().unwrap().contains("hi from the folder"));
        assert!(events.iter().any(|event| matches!(event, EngineEvent::Thinking(text) if text == "Read it.")));
        assert!(events.iter().any(|event| matches!(event, EngineEvent::Delta(text) if text.contains("It says hi."))));
        assert!(matches!(events.last(), Some(EngineEvent::Done { summary, .. }) if summary == "Done"));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn local_target(url: &str, num_ctx: u64) -> Target {
        Target {
            url: url.to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "qwen3.5:9b".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: Some(num_ctx),
            local_window: false,
        }
    }

    /// P0-8: a prompt twice the window is refused *before* anything is sent - the address here has no server, so a
    /// request that went out would fail with a connection error instead of `ContextOverflow`. With the switch off the
    /// guard is gone and the old behaviour (send it) is back, which is how the switch is proven to be the only
    /// difference.
    #[test]
    fn a_prompt_twice_the_window_is_never_sent_and_the_switch_restores_the_old_road() {
        let target = local_target("http://127.0.0.1:1/api/chat", 2_048);
        let huge = serde_json::json!([{ "role": "user", "content": "GOAL: PINEAPPLE\n".to_string() + &"filler ".repeat(6_000) }]);
        let recorder = crate::engines::Recorder::new();
        let stopped = || false;

        let guarded = ask_model(Backend::Ollama, &target, ("", crate::guard::modelgate::Purpose::Main), "system", huge.as_array().unwrap(), &[], &recorder.sink(), &stopped);

        assert!(guarded.as_ref().is_err_and(|reason| reason.starts_with("ContextOverflow")), "{guarded:?}");

        let unguarded = crate::v3::ablating(&["local.runtime_guard"], || ask_model(Backend::Ollama, &target, ("", crate::guard::modelgate::Purpose::Main), "system", huge.as_array().unwrap(), &[], &recorder.sink(), &|| true));

        assert!(unguarded.as_ref().is_err_and(|reason| !reason.starts_with("ContextOverflow")), "with the switch off the request is not refused by the guard: {unguarded:?}");
    }

    /// P0-8: the runtime says it read as many tokens as the window holds - the signature of a prompt cut at the front -
    /// and that answer is not trusted.
    #[test]
    fn an_answer_read_through_a_full_window_is_marked_invalid() {
        let (url, server) = raw_provider(vec![ndjson(&[
            serde_json::json!({ "message": { "role": "assistant", "content": "an answer to half a prompt" }, "done": false }),
            serde_json::json!({ "message": { "role": "assistant", "content": "" }, "done": true, "done_reason": "stop", "prompt_eval_count": 4_096, "eval_count": 5 }),
        ])]);
        let target = local_target(&url, 4_096);
        let messages = serde_json::json!([{ "role": "user", "content": "hello" }]);
        let recorder = crate::engines::Recorder::new();
        let result = ask_model(Backend::Ollama, &target, ("", crate::guard::modelgate::Purpose::Main), "system", messages.as_array().unwrap(), &[], &recorder.sink(), &|| false);

        server.join().unwrap();

        assert!(result.as_ref().is_err_and(|reason| reason.contains("may have been cut")), "{result:?}");
    }

    /// P0-8: Ollama is reached on its native `/api/chat` with an explicit context, never on the OpenAI-compatible `/v1`
    /// road that cannot carry `num_ctx`.
    #[test]
    fn ollama_is_never_reached_through_v1() {
        let root = std::env::temp_dir();
        let prompt = prompt_in(&root, "turn-native-road", "hi", "qwen3.5:9b");
        let target = target(Backend::Ollama, &prompt).expect("a local target");

        assert_eq!(target.url, ollama_native::URL);
        assert!(!target.url.contains("/v1"));
        assert!(target.local_context.is_some_and(|ctx| ctx > 0), "num_ctx is always sent");
    }

    /// A loopback address that must never be called: nothing accepts on it, and `untouched` says whether anyone tried.
    fn forbidden_provider() -> (String, impl Fn() -> bool) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());

        listener.set_nonblocking(true).unwrap();

        (url, move || matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock))
    }

    fn fake_target(url: String, model: &str) -> Target {
        Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: model.to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        }
    }

    /// P0-10 (Manual Pin): the person chose one model. A failover order, a cascade's strong model and a router's fast
    /// model are all configured - and none of them is ever called, whichever way the turn goes. Each forbidden
    /// address is a listener nobody accepts on; `untouched()` is false if anything so much as connected.
    #[test]
    fn a_pinned_turn_never_calls_another_model_whatever_is_configured() {
        let root = std::env::temp_dir().join(format!("sdc-pin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let (backup_url, backup_untouched) = forbidden_provider();

        FAKE_TARGETS.with(|targets| targets.borrow_mut().insert("backup".to_string(), fake_target(backup_url.clone(), "backup-model")));
        crate::v3::failover::set_order_for_test(vec![("backup".to_string(), "backup-model".to_string())]);
        crate::v3::cascade::set_strong_for_test(Some(("backup".into(), "backup-model".into())));
        crate::v3::router::set_fast_for_test(Some(("backup".into(), "backup-model".into())));

        let options = || Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None };

        /* 1. The provider refuses (a 401): the failover would hand the task on. Pinned, the failure is the answer. */
        let refused = {
            let body = serde_json::json!({ "error": { "message": "Incorrect API key provided" } }).to_string();

            format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        };
        let (url, server) = raw_provider(vec![refused]);
        let mut prompt = prompt_in(&root, "turn-pin-failover", "make half.txt and finish the job", "pinned-model");

        prompt.provider = Some("primary".into());

        let recorder = crate::engines::Recorder::new();

        drive(Backend::Api, &fake_target(url, "pinned-model"), &workspace, options(), None, &Default::default(), &prompt, &recorder.sink());
        server.join().unwrap();

        assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::Failed(_))), "the refusal reaches the person; nothing takes over");

        /* 2. A draft that keeps failing the same step: the cascade would hand over to the strong model. */
        let missing = call("c1", "read_file", serde_json::json!({ "path": "nothing-here.txt" }));
        let (url, server) = raw_provider(vec![ok(missing.clone()), ok(missing.clone()), ok(missing), ok(chunk(serde_json::json!({ "content": "I could not find it." })) + "data: [DONE]\n\n")]);
        let prompt = prompt_in(&root, "turn-pin-cascade", "make the orders tests pass", "pinned-model");
        let recorder = crate::engines::Recorder::new();

        drive(Backend::Api, &fake_target(url, "pinned-model"), &workspace, options(), None, &Default::default(), &prompt, &recorder.sink());

        let asked = server.join().unwrap();

        assert!(asked.iter().all(|body| body["model"] == "pinned-model"), "every request names the pinned model: {:?}", asked.iter().map(|body| body["model"].clone()).collect::<Vec<_>>());

        /* 3. A plainly light question: the router would send it to the fast model. */
        let (url, server) = provider(vec![chunk(serde_json::json!({ "content": "It means X." })) + "data: [DONE]\n\n"]);
        let prompt = prompt_in(&root, "turn-pin-router", "what does this error mean?", "pinned-model");

        drive(Backend::Api, &fake_target(url, "pinned-model"), &workspace, options(), None, &Default::default(), &prompt, &crate::engines::Recorder::new().sink());

        let asked = server.join().unwrap();

        assert!(asked.iter().all(|body| body["model"] == "pinned-model"));

        crate::v3::failover::set_order_for_test(Vec::new());
        crate::v3::cascade::set_strong_for_test(None);
        crate::v3::router::set_fast_for_test(None);
        FAKE_TARGETS.with(|targets| targets.borrow_mut().clear());

        assert!(backup_untouched(), "the other model was never so much as connected to");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// P0-10: the gate itself, on the agent's own funnel - a call for a model other than the turn's is refused before
    /// anything is sent (a `PinViolationError`), and is allowed again when the contract switch is off.
    #[test]
    fn a_forced_foreign_call_raises_pin_violation_and_the_switch_removes_the_gate() {
        let _held = crate::guard::modelgate::register("turn-forced", crate::guard::modelgate::Gate::new(Some("deepseek"), "deepseek-chat", false, false));
        let (url, untouched) = forbidden_provider();
        let foreign = fake_target(url, "gpt-5-nano");
        let messages = serde_json::json!([{ "role": "user", "content": "hi" }]);
        let recorder = crate::engines::Recorder::new();
        let refused = ask_model(Backend::Api, &foreign, ("turn-forced", crate::guard::modelgate::Purpose::Main), "system", messages.as_array().unwrap(), &[], &recorder.sink(), &|| true);

        assert!(refused.as_ref().is_err_and(|reason| reason.starts_with("PinViolationError")), "{refused:?}");
        assert!(untouched(), "and nothing was sent");

        /* The same call with the contract off passes the gate (and is then stopped by the test's own `stopped`). */
        let off = crate::v3::ablating(&["models.contract"], || crate::guard::modelgate::Gate::new(Some("deepseek"), "deepseek-chat", false, false));

        assert!(off.permit(crate::guard::modelgate::Purpose::Main, None, "gpt-5-nano").is_ok());
    }

    /// P0-12: a turn leaves a Step Trace row per model call, with the time to first token, the tokens, the mode it ran
    /// under and a cost from the catalogue's price - and with the switch off it leaves none.
    #[test]
    fn a_turn_leaves_one_trace_row_per_model_call() {
        let store = crate::trace::test_store();
        let root = std::env::temp_dir().join(format!("sdc-trace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("hello.txt"), "hi\n").unwrap();

        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let (url, server) = provider(vec![
            call("c1", "read_file", serde_json::json!({ "path": "hello.txt" })),
            chunk(serde_json::json!({ "content": "It says hi." })) + "data: [DONE]\n\n",
        ]);
        let mut target = fake_target(url, "trace-model");

        target.price = Some((1.0, 2.0));

        let mut prompt = prompt_in(&root, "turn-trace-on", "what does hello.txt say?", "trace-model");

        prompt.session_id = "session-trace".into();
        prompt.provider = Some("tracer".into());

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 5, auto_check: false, research: None }, None, &Default::default(), &prompt, &crate::engines::Recorder::new().sink());
        server.join().unwrap();
        crate::trace::flush();

        let rows = crate::trace::recent(&store, Some("session-trace"), 10);

        assert_eq!(rows.len(), 2, "two model calls, two rows: {rows:?}");
        assert_eq!(rows[0]["taskId"], "turn-trace-on");
        assert_eq!(rows[0]["stepId"], "turn-trace-on.s1");
        assert_eq!(rows[0]["modelKey"], "tracer:trace-model");
        assert_eq!(rows[0]["mode"], "pinned");
        assert!(rows[0]["ttftMs"].as_i64().unwrap() >= 1 && rows[0]["wallMs"].as_i64().unwrap() >= rows[0]["ttftMs"].as_i64().unwrap());

        /* The same turn with the switch off writes nothing. */
        let (url, server) = provider(vec![chunk(serde_json::json!({ "content": "hi" })) + "data: [DONE]\n\n"]);
        let mut prompt = prompt_in(&root, "turn-trace-off", "say hi", "trace-model");

        prompt.session_id = "session-trace-off".into();

        crate::v3::ablating(&["trace.steps"], || drive(Backend::Api, &fake_target(url, "trace-model"), &workspace, Options { autonomy: Autonomy::Auto, max_steps: 3, auto_check: false, research: None }, None, &Default::default(), &prompt, &crate::engines::Recorder::new().sink()));
        server.join().unwrap();
        crate::trace::flush();

        assert!(crate::trace::recent(&store, Some("session-trace-off"), 10).is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A SearXNG server on loopback that answers every search with the same two pages.
    fn searxng() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());

        std::thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let mut reader = BufReader::new(socket.try_clone().unwrap());

                loop {
                    let mut line = String::new();

                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }

                let body = serde_json::json!({ "results": [
                    { "title": "Release notes", "url": "https://example.org/notes", "content": "Version 2 adds X.", "publishedDate": "2026-03-01" },
                    { "title": "Blog", "url": "https://example.com/blog", "content": "A post about X." },
                ] })
                .to_string();
                let mut socket = socket;

                let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });

        address
    }

    /// 0.16.1: a `/research` turn has its own toolbox, stops at its limits, and ends with its numbered
    /// sources - the same page keeping its number.
    #[test]
    fn a_research_turn_keeps_to_its_tools_and_limits_and_lists_its_sources() {
        let store = crate::store::sqlite::Store::in_memory().unwrap();

        store.set_setting("research.searchProvider", "searxng").unwrap();
        store.set_setting("research.searxngUrl", &searxng()).unwrap();
        research::configure(&store);

        let root = std::env::temp_dir().join(format!("sdc-agent-research-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let (url, server) = provider(vec![
            call("r1", "web_search", serde_json::json!({ "query": "what is new in X" })),
            call("r2", "web_search", serde_json::json!({ "query": "X again" })),
            call("r3", "write_file", serde_json::json!({ "path": "notes.md", "content": "x" })),
            /* Answers from no page read: SDC asks once for a second source, and the model answers again. */
            chunk(serde_json::json!({ "content": "Version 2 adds X [1]." })) + "data: [DONE]\n\n",
            chunk(serde_json::json!({ "content": "Version 2 adds X [1]." })) + "data: [DONE]\n\n",
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "api-test".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: None,
            local_window: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = prompt_in(&root, "turn-agent-research", "what is new in X?", "api-test");
        let recorder = crate::engines::Recorder::new();
        let limits = research::Limits { max_searches: 1, max_pages: 2, max_minutes: 5 };

        drive(Backend::Api, &target, &workspace, Options { autonomy: Autonomy::Auto, max_steps: 10, auto_check: false, research: Some(limits) }, None, &Default::default(), &prompt, &recorder.sink());
        research::configure(&crate::store::sqlite::Store::in_memory().unwrap());

        let bodies = server.join().unwrap();
        let events = recorder.events();
        let offered: Vec<String> = bodies[0]["tools"].as_array().unwrap().iter().map(|tool| tool["function"]["name"].as_str().unwrap().to_string()).collect();

        assert!(offered.iter().all(|name| research::TOOLS.contains(&name.as_str())), "only the research tools: {offered:?}");
        assert!(bodies[0]["messages"][0]["content"].as_str().unwrap().contains("SDC Research"), "the research brief");

        let sent = bodies[3]["messages"].to_string();

        assert!(sent.contains("[1] Release notes"), "results are numbered: {sent}");
        assert!(sent.contains("research limit is reached"), "the second search is over the limit");
        assert!(sent.contains("not part of a research turn"), "write_file is refused");
        assert!(!root.join("notes.md").exists());
        assert!(bodies[4]["messages"].to_string().contains("read only one page"), "an answer from one page is asked for a second source");

        let sources = events.iter().find_map(|event| match event {
            EngineEvent::Sources(sources) => Some(sources.clone()),
            _ => None,
        });
        let sources = sources.expect("the turn lists its sources");

        assert_eq!(sources[0]["n"], 1);
        assert_eq!(sources[0]["url"], "https://example.org/notes");
        assert_eq!(sources[0]["date"], "2026-03-01");
        assert!(matches!(events.last(), Some(EngineEvent::Done { meta, .. }) if meta.contains("1 searches")));

        let _ = std::fs::remove_dir_all(&root);
    }
}

/// L5: the tool block is what every single step of every turn pays for, so its size is pinned here. A tool that grows past
/// its budget has to lose words somewhere else, and this says so at once rather than after a release.
#[cfg(test)]
mod l5_budget {
    use super::*;

    /// The whole block the model is told about the tools, in characters: descriptions and schemas together.
    const TOOL_BLOCK: usize = 9_300;

    /// One tool: the `browser` tool is the largest, and it is the one to watch.
    const ONE_TOOL: usize = 1_150;

    fn weights() -> Vec<(usize, String)> {
        let mut sizes: Vec<(usize, String)> = tools::specs()
            .iter()
            .map(|tool| (tool.description.len() + serde_json::to_string(&tool.schema).unwrap().len(), tool.name.to_string()))
            .collect();

        sizes.sort_by_key(|entry| std::cmp::Reverse(entry.0));

        sizes
    }

    #[test]
    fn the_tool_block_stays_within_its_budget() {
        let total: usize = weights().iter().map(|(size, _)| size).sum();

        assert!(total <= TOOL_BLOCK, "the tool block is {total} characters, over its budget of {TOOL_BLOCK}: trim a description somewhere");
    }

    #[test]
    fn no_single_tool_outgrows_the_block() {
        let (size, name) = weights().remove(0);

        assert!(size <= ONE_TOOL, "`{name}` alone is {size} characters, over its budget of {ONE_TOOL}");
    }

    #[test]
    fn no_tool_is_left_without_a_description() {
        for tool in tools::specs() {
            /* Short is fine - `stop_process` says all it needs to - but a tool nobody explained is not. */
            assert!(tool.description.len() >= 20, "`{}` says too little to be used: {:?}", tool.name, tool.description);
            assert!(tool.description.len() <= ONE_TOOL, "`{}` describes itself at length: {}", tool.name, tool.description.len());
        }
    }

    /// The brief on its own, before the workspace and the language are written into it.
    const SYSTEM_PROMPT: usize = 6_200;

    #[test]
    fn the_system_prompt_stays_within_its_budget() {
        let workspace = Workspace::new("/tmp/project", None);
        let prompt = system_prompt(&workspace, false, &crate::understand::Reading::BANGLISH, true);
        let vision = system_prompt(&workspace, true, &crate::understand::Reading::BANGLISH, true);

        assert!(prompt.len() <= SYSTEM_PROMPT, "the system prompt is {} characters, over its budget of {SYSTEM_PROMPT}: trim a paragraph", prompt.len());
        /* Vision is a paragraph more: it must not be a document more. */
        assert!(vision.len() - prompt.len() < 900, "the vision paragraph alone is {} characters", vision.len() - prompt.len());
        assert!(prompt.contains("/tmp/project"), "the folder the turn runs in is in the brief");
        assert!(prompt.contains("Banglish"), "so is the language the person writes in");
    }
}
