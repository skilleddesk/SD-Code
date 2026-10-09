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
    pub fn with_research(mut self, limits: research::Limits) -> Self {
        self.research = Some(limits);
        self.max_steps = self.max_steps.min(limits.steps());
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
                return Err(crate::auth::keychain::missing_key_reason(&endpoint.key_ref, &endpoint.provider));
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
                local_context: None,
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
         Today: {today}\n\
         Project folder: {root}\n\
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
        today = chrono::Local::now().format("%Y-%m-%d"),
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

    let workspace = Workspace::new(root, prompt.remote.clone());

    /* 0.21: a local model's turn starts Ollama itself when it is installed and stopped. */
    if backend == Backend::Ollama {
        crate::host::tools::ensure_ollama();
    }

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

    /* A local model gets Ollama's own request, with the context size in it (0.16.1). */
    if let Some(num_ctx) = target.local_context {
        body = ollama_native::body(body, num_ctx);
    }

    let plain = body.to_string();
    /* 0.21: Alibaba's explicit prompt cache - the system prompt and the tools marked once, read back on every
       later step at a tenth of the price and without being processed again. A model that refuses the mark
       is asked again without it, and the mark is not sent to that endpoint again. */
    let marked = (target.dialect == Dialect::OpenAi && dashscope_cache(&target.url)).then(|| dialect::mark_system_cache(&body).to_string());
    /* A step the network or the provider dropped - before its first word or halfway through - is asked
       again rather than ending the turn (0.15.4): nothing of it reached the conversation yet, so asking
       again is the same question, and every step before it is kept. */
    let mut card = crate::engines::native_api::RetryCard::new(sink, "agent", &target.url);
    let send = |body: &str, card: &mut crate::engines::native_api::RetryCard| {
        crate::engines::native_api::with_retries(
            stopped,
            |retry, wait, reason| card.notice(retry, wait, reason),
            || {
                let lines = crate::engines::native_api::open_stream(&target.url, &target.headers, body)?;
                let lines = if target.local_context.is_some() { ollama_native::as_sse(lines) } else { lines };

                dialect::read_reply(target.dialect, lines, sink, stopped)
            },
        )
    };
    let reply = match marked {
        Some(marked) => match send(&marked, &mut card) {
            Err(reason) if reason.contains("(400)") || reason.to_lowercase().contains("cache_control") => {
                let again = send(&plain, &mut card);

                /* Only when the plain request went through was the mark the problem (a 400 can be anything). */
                if again.is_ok() {
                    no_explicit_cache().lock().unwrap_or_else(|poison| poison.into_inner()).insert(target.url.clone());
                }

                again
            }
            other => other,
        },
        None => send(&plain, &mut card),
    };

    card.close(reply.is_ok());
    reply.map_err(|reason| unreachable(backend, &target.model, &reason))
}

/// Endpoints that refused an explicit cache mark (0.21) - they are sent plain requests from then on.
fn no_explicit_cache() -> &'static std::sync::Mutex<HashSet<String>> {
    static PLAIN: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> = std::sync::OnceLock::new();

    PLAIN.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

/// Alibaba Cloud Model Studio (DashScope), which takes Anthropic-style `cache_control` marks on an
/// OpenAI-compatible request - and has not refused one from this endpoint yet.
fn dashscope_cache(url: &str) -> bool {
    url.contains("dashscope") && url.contains("aliyuncs.com") && !no_explicit_cache().lock().unwrap_or_else(|poison| poison.into_inner()).contains(url)
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
    const SUB_STEPS: usize = 24;

    let stopped = || crate::engines::cancel::requested(&parent.turn_id);
    /* A research turn's sub-agent reads one page or runs one search, counted against the same limits. */
    let system = if research_session.is_some() { research::sub_agent_prompt() } else { sub_agent_prompt(workspace) };
    let mut specs = tools::specs_for(tools::Caps { vision, subagent: true, patch: false });

    if research_session.is_some() {
        specs.retain(|spec| research::TOOLS.contains(&spec.name));
    }
    let window = target.window(parent.provider.as_deref());
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
        result_cap: result_cap(backend, window),
    };
    let mut messages = vec![dialect::user_message(job)];
    let (mut input, mut output) = (0u64, 0u64);
    /* 0.21: explore with the provider's fast model; the turn's own model is the fallback. */
    let fast = fast_target(backend, target, parent.provider.as_deref());
    let mut on_fast = fast.is_some();

    for _ in 0..SUB_STEPS {
        if stopped() {
            return (tools::Outcome::error("The turn was stopped."), input, output);
        }

        let active = if on_fast { fast.as_ref().unwrap_or(target) } else { target };
        let reply = match ask_model(backend, active, &system, &messages, &specs, &EventSink::discarding(), &stopped) {
            Ok(reply) => reply,
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

    let active = if on_fast { fast.as_ref().unwrap_or(target) } else { target };

    match ask_model(backend, active, &system, &messages, &[], &EventSink::discarding(), &stopped) {
        Ok(reply) => {
            input += reply.input_tokens;
            output += reply.output_tokens;

            (tools::Outcome::ok(reply.text), input, output)
        }
        Err(reason) => (tools::Outcome::error(format!("The sub-agent ran out of steps: {reason}")), input, output),
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
    let language = person_language(prompt);
    let window = target.window(prompt.provider.as_deref());
    /* A `/research` turn (0.16.1): its own brief, its own tools, its limits and its sources. */
    let research_session = options.research.map(|limits| std::sync::Arc::new(research::Session::new(limits)));
    let research_config = research::config();
    let system = match &research_session {
        Some(session) => research::system_prompt(&language, &session.limits, research_config.provider.label(), window < 32_000),
        None => system_prompt(workspace, vision, &language) + &skills::brief(&skills::find(workspace)),
    };
    /* A model that writes the final answer from what this one gathered (Settings → Research), opt-in. */
    let synthesis = research_session.as_ref().and(research_config.synthesis.clone());
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
    let text = {
        let words = prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text);
        let wants_map = research_session.is_none() && orient::pace_of(words) != orient::Pace::Quick;

        match wants_map.then(|| orient::orientation(workspace)).flatten() {
            Some(map) => format!("{map}\n\n{text}"),
            None => text,
        }
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
        result_cap: result_cap(backend, window),
    };
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

        if let Some(session) = research_session.as_ref().filter(|session| session.expired() && !out_of_time) {
            out_of_time = true;
            dialect::append_user_text(
                target.dialect,
                &mut messages,
                &format!("[SDC: the research time ({} minutes) is up. Write the answer now from what you found, citing [n], without calling tools.]", session.limits.max_minutes),
            );
        }

        let step_specs: &[dialect::ToolSpec] = if out_of_time { &[] } else { &specs };
        let asked_at = std::time::Instant::now();
        let asked = ask_model(backend, target, &system, &messages, step_specs, &notes_sink(said_something), &stopped);

        model_time += asked_at.elapsed();

        let reply: Reply = match asked {
            Ok(reply) => {
                filtered = 0;

                reply
            }
            Err(reason) if reason == "stopped" => return,
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
                sink.send(EngineEvent::Failed(reason));

                return;
            }
        };

        input_tokens += reply.input_tokens;
        output_tokens += reply.output_tokens;
        cached_tokens += reply.cached_tokens;

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

            sink.send(EngineEvent::Done {
                summary: summary.to_string(),
                meta: format!("{}{}", meta(step, input_tokens, output_tokens, target.price) + &cached(cached_tokens), timing(model_time, tool_time)),
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

            results[index] = Some((call.id.clone(), outcome.content, outcome.is_error, outcome.image));
            index += 1;
        }

        if !tasks.is_empty() {
            for (index, outcome, input, output) in run_tasks(backend, target, workspace, policy, prompt, vision, &tasks, sink, research_session.clone()) {
                input_tokens += input;
                output_tokens += output;
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

        dedupe_reads(&mut seen_reads, &reply.tool_uses, &mut results);
        messages.extend(dialect::tool_results(target.dialect, &results));
        steer(&mut messages);

        let folded = keep_small(target, window, &mut messages);

        /* Folded output is gone from the conversation: a file read before it must be sent in full again. */
        if folded {
            seen_reads.clear();
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
        meta: format!("{}{}", meta(options.max_steps, input_tokens, output_tokens, target.price) + &cached(cached_tokens), timing(model_time, tool_time)),
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

        ask_model(backend, &synth, &research::synthesis_prompt(language), &conversation, &[], sink, stopped)
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
        let target = Target { url, headers: Vec::new(), dialect: Dialect::OpenAi, model: "local-test".to_string(), price: None, thinking: false, effort: None, local_context: None };
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
        let target = Target { url, headers: Vec::new(), dialect: Dialect::OpenAi, model: "local-test".to_string(), price: None, thinking: false, effort: None, local_context: None };
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
        let target = Target { url, headers: Vec::new(), dialect: Dialect::OpenAi, model: "local-test".to_string(), price: None, thinking: false, effort: None, local_context: None };
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
