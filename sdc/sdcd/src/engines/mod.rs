//! The engine adapters (master spec section 11).
//!
//! `trait Engine` is the whole contract, and every adapter implements it the same way: spawn
//! something, parse a **structured** stream, emit events. Nothing here reads a terminal screen,
//! matches a prompt string or sleeps and hopes (principle P3).
//!
//! | adapter       | what it talks to                    | how it speaks |
//! | ------------- | ----------------------------------- | ------------- |
//! | `claude_code` | `claude --include-partial-messages` | JSON lines    |
//! | `codex`       | `codex`                             | JSON lines    |
//! | `gemini`      | `gemini`                            | JSON lines    |
//! | `native_api`  | Anthropic / OpenAI over HTTPS       | SSE           |
//! | `ollama`      | `127.0.0.1:11434/api/chat`          | NDJSON        |
//!
//! The three CLI adapters share one implementation (`cli.rs`): a prompt on stdin, `{"type": …}`
//! objects on stdout, a child to kill. Their differences - the program, the structured-stream flag
//! and which JSON field carries the text - are constructor arguments.

pub mod body;
pub mod claude_code;
pub mod cli;
pub mod codex;
pub mod gemini;
pub mod native_api;
pub mod ollama;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

/// What a turn asks an engine to do.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub session_id: String,
    pub turn_id: String,
    pub text: String,
    /// The model the session is set to, as the registry spells it (`anthropic/claude-sonnet-4-5`).
    ///
    /// It travels *in the prompt* because the engines cannot guess it and two of them tried:
    /// `native_api` read the model out of the **prompt text** (so an API turn only ever reached the
    /// right provider when the user typed the provider's name, and otherwise went to the loopback
    /// endpoint), and `ollama` read it out of the **first history message**. Both are the same
    /// mistake - a fact about the session inferred from its content - and both are why "connect with
    /// an API key" did not work even with a valid key.
    pub model: String,
    /// The provider the model was picked from, when the caller knows it (`deepseek`).
    ///
    /// The model id alone is not always enough: a provider's *live* list contains ids this build's
    /// catalogue has never seen (`deepseek-v4-pro`), so nothing can say which endpoint it belongs to -
    /// and `native_api` fell back to the loopback `custom` endpoint, whose key is a different entry, and
    /// answered `No API key for custom` for a provider the person had already connected. The provider
    /// travels with the prompt for the same reason the model does: the engine cannot guess a fact about
    /// the session, and a wrong guess here is a failed turn, not a wrong label.
    pub provider: Option<String>,
    /// The conversation so far, oldest first - the Session Bridge's payload (spec section 16.5).
    pub history: Vec<String>,
    /// The folder this chat works in, or `None` for a chat that has no project (0.7.6).
    ///
    /// `cli.rs` starts the child process **in** this directory, which is the difference between an engine
    /// that edits your project and one that edits whatever folder the daemon happened to be started in.
    /// The two HTTP adapters (`native_api`, `ollama`) have no working directory of their own - a provider
    /// answers over the network - so for them the field travels and is unused. It is on the `Prompt`
    /// rather than in the CLI adapters because it is a fact about the session, like the model: the engine
    /// cannot guess it, and guessing it wrong is a turn that edits the wrong files.
    pub project_root: Option<String>,
    /// The **host** this chat's folder is on, when it is not this machine (0.7.13).
    ///
    /// With this set, `cli.rs` runs the CLI *there*: the child is an `ssh` whose command is
    /// `cd <project_root> && <cli> <args>`, the prompt still travels on stdin, the CLI's own JSON stream
    /// still arrives on stdout - so the parsing, the events and the window are unchanged - and the pid is
    /// written into a file on that host (see `ssh::ops::turn_line`) so `engine.cancel` really stops it.
    ///
    /// The two HTTP adapters ignore it for the same reason they ignore `project_root`: their turn is a
    /// network call from this daemon, not a process on a folder. A provider that answered text is not a
    /// provider whose tools ran anywhere.
    pub remote: Option<crate::ssh::Ssh>,
}

/// One item of an engine's stream. The checkpoint is *not* here: the daemon writes that itself,
/// before a mutating tool runs (principle P5).
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    Delta(String),
    Thinking(String),
    ToolStarted { call_id: String, tool: String, name: String, target: String },
    ToolOutput { call_id: String, level: String, text: String },
    ToolCompleted { call_id: String, status: String, meta: String },
    Failed(String),
    Done {
        summary: String,
        meta: String,
        pass: Option<bool>,
    },
}

impl EngineEvent {
    /// True for the two events that end a turn. `cli.rs` stops emitting after one of them, and the
    /// daemon's turn loop does not care - it just forwards.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed(_))
    }
}

/// Where an adapter's events go **while the turn is still running**.
///
/// This type is the answer to the report *"claude code/cline e jemon thinking ki korse sob kisu live
/// dakha jai, aitare tamon kisu hosse nah - akbare answare disse"*. `Engine::start` used to answer with
/// a `Vec<EngineEvent>`, i.e. it returned only once the engine had finished, so the daemon forwarded
/// a whole turn at the end: the transcript appeared in one piece and the window looked like it had
/// done nothing for the length of the run. The engine had streamed perfectly well - the daemon was
/// holding a closed hand.
///
/// A sink is cheap to clone (an `Arc` around one closure) and has no transport knowledge in it: the
/// daemon hands in a channel, a test hands in a `Recorder`, and neither the adapters nor their
/// parsers change shape because of it.
#[derive(Clone)]
pub struct EventSink {
    emit: Arc<dyn Fn(EngineEvent) + Send + Sync>,
}

impl EventSink {
    /// A sink over one closure.
    pub fn new(emit: impl Fn(EngineEvent) + Send + Sync + 'static) -> Self {
        Self { emit: Arc::new(emit) }
    }

    /// A sink nobody listens to - for a probe, or for a caller that wants only the side effect.
    pub fn discarding() -> Self {
        Self::new(|_| {})
    }

    /// The pair a turn uses: the engine writes, the daemon reads.
    ///
    /// `recv` answers `None` once the engine's future has ended and the sink with it, which is what
    /// ends the daemon's loop - there is no separate "the turn is over" signal to lose.
    pub fn channel() -> (Self, UnboundedReceiver<EngineEvent>) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();

        (
            Self::new(move |event| {
                /* A receive error means the daemon's side is gone (the window closed mid-turn). The
                   turn is already over for the reader; the engine keeps its own course. */
                let _ = sender.send(event);
            }),
            receiver,
        )
    }

    /// One event, now.
    pub fn send(&self, event: EngineEvent) {
        (self.emit)(event);
    }

    /// A whole parser answer - `parse_stream_line` returns zero or more events for one line.
    pub fn extend(&self, events: impl IntoIterator<Item = EngineEvent>) {
        for event in events {
            self.send(event);
        }
    }
}

/// A sink backed by a `Vec`, for tests and fixtures: `Recorder::new().sink()` writes, `events()`
/// reads. The recording counterpart of `sdcp::notifications::RecordingNotifier`.
#[derive(Clone, Default)]
pub struct Recorder {
    events: Arc<Mutex<Vec<EngineEvent>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sink(&self) -> EventSink {
        let events = self.events.clone();

        EventSink::new(move |event| {
            if let Ok(mut held) = events.lock() {
                held.push(event);
            }
        })
    }

    /// Everything the sink was given, in arrival order.
    pub fn events(&self) -> Vec<EngineEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }
}

/// How an engine is doing: what `engine.status` reports and what the amber "stuck" state of spec
/// section 12.9 is derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineStatus {
    Idle,
    Running,
    Awaiting,
    Stuck,
    Done,
    Failed,
    Killed,
}

impl EngineStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Awaiting => "awaiting",
            Self::Stuck => "stuck",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Killed => "killed",
        }
    }
}

/// The contract. `start` **pushes each item to `sink` as it arrives** - the daemon's turn task
/// forwards every one of them as a notification while the engine is still talking, which is what
/// makes the app's turn stream live and keeps every adapter free of transport knowledge.
///
/// An adapter that answers with a `Vec` it built along the way would still compile, and would be the
/// bug this signature exists to prevent: the answer, the thinking and the tool calls of a two-minute
/// turn would all land in the window at the end of it.
#[async_trait]
pub trait Engine: Send + Sync {
    /// `claude_code`, `codex`, `gemini`, `native_api`, `ollama` - the id in `engine.start`.
    fn id(&self) -> &'static str;

    /// Runs one turn to its end, emitting as it goes. The turn is over when this future resolves;
    /// a stream that ends without `Done` or `Failed` is the adapter's job to explain.
    async fn start(&self, prompt: Prompt, sink: &EventSink);

    async fn cancel(&self, turn_id: &str) -> bool;
    fn status(&self, turn_id: &str) -> EngineStatus;
}

/// Runs one turn and collects what it emitted - the shape `start` used to have, for a test or a
/// fixture that wants the whole stream in one piece.
pub async fn start_recording(engine: &dyn Engine, prompt: Prompt) -> Vec<EngineEvent> {
    let recorder = Recorder::new();

    engine.start(prompt, &recorder.sink()).await;

    recorder.events()
}

/// The registry: `engine.start` looks an engine up here.
#[derive(Default)]
pub struct EngineRegistry {
    engines: HashMap<&'static str, Arc<dyn Engine>>,
}

impl EngineRegistry {
    /// The five adapters, wired.
    pub fn with_defaults() -> Self {
        let mut registry = Self::default();

        registry.insert(Arc::new(claude_code::ClaudeCode::new()));
        registry.insert(Arc::new(codex::Codex::new()));
        registry.insert(Arc::new(gemini::Gemini::new()));
        registry.insert(Arc::new(native_api::NativeApi::new()));
        registry.insert(Arc::new(ollama::Ollama::new()));

        registry
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Engine>> {
        self.engines.get(id).cloned()
    }

    pub fn ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self.engines.keys().copied().collect();

        ids.sort_unstable();
        ids
    }

    /// Registers one adapter, replacing any previous one with the same id.
    pub fn insert(&mut self, engine: Arc<dyn Engine>) {
        self.engines.insert(engine.id(), engine);
    }
}

/// The tier an engine's answers are costed at, so a status line can say `Balanced` without asking.
pub fn tier_for(engine: &str) -> &'static str {
    match engine {
        "native_api" => "Deep",
        "ollama" => "Fast",
        _ => "Balanced",
    }
}

/// Parses one JSON line of a CLI's stream into zero or more `EngineEvent`s.
///
/// This is the only place a CLI's field names appear, which is what lets the VCR fixtures of spec
/// section 11.6 hold all three CLI adapters to one contract: feed lines in, get the same
/// `Vec<EngineEvent>` out. A line that is not JSON is skipped, not fatal - a CLI that prints a
/// deprecation warning must not kill a turn.
pub fn parse_stream_line(line: &str) -> Vec<EngineEvent> {
    let trimmed = line.trim();

    if trimmed.is_empty() {
        return Vec::new();
    }

    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    match value.get("type").and_then(Value::as_str).unwrap_or("text") {
        /* Claude Code wraps every event; only the inner one is interesting. */
        "stream_event" => value.get("event").map(parse_claude_event).unwrap_or_default(),

        /* Claude's finished message, which the partial deltas already carried. Dropping it is
           deliberate: emitting both prints every answer twice. */
        "assistant" => Vec::new(),

        /* Codex: one item per thing that happened, plus four words about the turn itself. */
        "item.completed" | "item.started" | "item.updated" => {
            value.get("item").map(parse_codex_item).unwrap_or_default()
        }
        "turn.completed" => vec![EngineEvent::Done {
            summary: "Done".to_string(),
            meta: usage_meta(&value),
            pass: Some(true),
        }],
        "turn.failed" => vec![EngineEvent::Failed(error_message(&value))],

        /* Gemini's stream-json: one message per delta. */
        "message" => text_of(&value).map(EngineEvent::Delta).into_iter().collect(),

        /* Claude's and Gemini's terminal line - and the shape the VCR fixtures spell. */
        "result" | "done" => result_event(&value),

        "error" => vec![EngineEvent::Failed(error_message(&value))],

        /* The prototype's own vocabulary. Kept because the fixtures of spec section 11.6 are written
        in it, and harmless because a shape a CLI does not send is never reached. */
        "text" | "assistant_text" | "content_block_delta" => text_of(&value)
            .map(EngineEvent::Delta)
            .into_iter()
            .collect(),
        "thinking" | "reasoning" => text_of(&value)
            .map(EngineEvent::Thinking)
            .into_iter()
            .collect(),
        "tool_use" | "tool_call" => vec![EngineEvent::ToolStarted {
            call_id: string_of(&value, "id", "call"),
            tool: string_of(&value, "tool", "run").to_lowercase(),
            name: string_of(&value, "name", "Tool"),
            target: string_of(&value, "target", ""),
        }],
        "tool_result" | "tool_output" => vec![EngineEvent::ToolOutput {
            call_id: string_of(&value, "id", "call"),
            level: string_of(&value, "level", "dim"),
            text: string_of(&value, "text", ""),
        }],
        "tool_done" => vec![EngineEvent::ToolCompleted {
            call_id: string_of(&value, "id", "call"),
            status: if value.get("ok").and_then(Value::as_bool).unwrap_or(true) {
                "done"
            } else {
                "failed"
            }
            .to_string(),
            meta: string_of(&value, "meta", "done"),
        }],
        _ => Vec::new(),
    }
}

/// A string field with a fallback, so a missing key never panics a turn.
fn string_of(value: &Value, key: &str, fallback: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or(fallback).to_string()
}

/// The text of a stream object: `delta`, `text` or `content`, and `delta.text` as a last resort.
fn text_of(value: &Value) -> Option<String> {
    for key in ["delta", "text", "content"] {
        if let Some(found) = value.get(key).and_then(Value::as_str) {
            return Some(found.to_string());
        }
    }

    value
        .get("delta")
        .and_then(|delta| delta.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// One inner event of Claude Code's `stream_event` wrapper.
fn parse_claude_event(event: &Value) -> Vec<EngineEvent> {
    match event.get("type").and_then(Value::as_str).unwrap_or("") {
        "content_block_delta" => {
            let Some(delta) = event.get("delta") else {
                return Vec::new();
            };

            match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                "text_delta" => delta
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| vec![EngineEvent::Delta(text.to_string())])
                    .unwrap_or_default(),
                "thinking_delta" => delta
                    .get("thinking")
                    .and_then(Value::as_str)
                    .map(|text| vec![EngineEvent::Thinking(text.to_string())])
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        }
        /* A tool call starts as a `content_block_start` whose block is the tool. */
        "content_block_start" => {
            let Some(block) = event.get("content_block") else {
                return Vec::new();
            };

            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                return Vec::new();
            }

            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();
            let input = block.get("input").cloned().unwrap_or(Value::Null);

            vec![EngineEvent::ToolStarted {
                call_id: block.get("id").and_then(Value::as_str).unwrap_or("call").to_string(),
                tool: tool_kind(&name).to_string(),
                name,
                target: target_of(&input),
            }]
        }
        /* `message_start`, `message_delta`, `message_stop`, `content_block_stop`: nothing to say. */
        _ => Vec::new(),
    }
}

/// One Codex `item`.
fn parse_codex_item(item: &Value) -> Vec<EngineEvent> {
    let text = item.get("text").and_then(Value::as_str);

    match item.get("type").and_then(Value::as_str).unwrap_or("") {
        "agent_message" => text.map(|text| vec![EngineEvent::Delta(text.to_string())]).unwrap_or_default(),
        "reasoning" => text.map(|text| vec![EngineEvent::Thinking(text.to_string())]).unwrap_or_default(),
        "command_execution" => {
            let command = item.get("command").and_then(Value::as_str).unwrap_or("").to_string();

            vec![EngineEvent::ToolStarted {
                call_id: string_of(item, "id", "call"),
                tool: "run".to_string(),
                name: command.split_whitespace().next().unwrap_or("command").to_string(),
                target: command,
            }]
        }
        "file_change" | "patch" | "apply_patch" => {
            let path = item
                .get("path")
                .or_else(|| {
                    item.get("changes")
                        .and_then(|changes| changes.get(0))
                        .and_then(|change| change.get("path"))
                })
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            vec![EngineEvent::ToolStarted {
                call_id: string_of(item, "id", "call"),
                tool: "edit".to_string(),
                name: "edit".to_string(),
                target: path,
            }]
        }
        "error" => vec![EngineEvent::ToolOutput {
            call_id: string_of(item, "id", "note"),
            level: "warn".to_string(),
            text: error_message(item),
        }],
        _ => Vec::new(),
    }
}

/// The terminal line, for a CLI that ends its stream with one.
fn result_event(value: &Value) -> Vec<EngineEvent> {
    let is_error = value.get("is_error").and_then(Value::as_bool).unwrap_or(false)
        || value
            .get("subtype")
            .and_then(Value::as_str)
            .map(|subtype| subtype.starts_with("error"))
            .unwrap_or(false);

    if is_error {
        return vec![EngineEvent::Failed(error_message(value))];
    }

    vec![EngineEvent::Done {
        /* Claude puts the whole answer in `result` and the fixtures put a label in `summary`. The
           deltas already printed the answer, so a summary that repeats it would print it twice: the
           label wins, and the provider's numbers go in `meta` where the footer reads them. */
        summary: value
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("Done")
            .to_string(),
        meta: usage_meta(value),
        pass: value.get("pass").and_then(Value::as_bool).or(Some(true)),
    }]
}

/// What a provider's own numbers say about the turn, when it sends any.
///
/// Claude ends with `duration_ms` and `total_cost_usd`, Codex with a `usage` object. Both are worth
/// showing next to the answer, and both are the provider's own numbers - nothing is estimated here.
fn usage_meta(value: &Value) -> String {
    /* The fixtures of spec section 11.6 spell the line themselves; a real stream has numbers. */
    if let Some(meta) = value.get("meta").and_then(Value::as_str) {
        return meta.to_string();
    }

    let mut parts = Vec::new();

    if let Some(cost) = value.get("total_cost_usd").and_then(Value::as_f64) {
        parts.push(format!("${cost:.4}"));
    }

    if let Some(ms) = value.get("duration_ms").and_then(Value::as_u64) {
        parts.push(format!("{:.1}s", ms as f64 / 1000.0));
    }

    if let Some(usage) = value.get("usage") {
        let input = usage.get("input_tokens").and_then(Value::as_u64);
        let output = usage.get("output_tokens").and_then(Value::as_u64);

        if let (Some(input), Some(output)) = (input, output) {
            parts.push(format!("{input} in · {output} out"));
        }
    }

    parts.join(" · ")
}

/// The first sentence of an error, from whichever key the CLI used.
fn error_message(value: &Value) -> String {
    for key in ["message", "result", "error"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return text.to_string();
        }

        if let Some(text) = value.get(key).and_then(|nested| nested.get("message")).and_then(Value::as_str) {
            return text.to_string();
        }
    }

    "the engine reported an error".to_string()
}

/// Which of the app's three tool kinds a CLI's tool name is (the UI colours them differently).
fn tool_kind(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "read" | "glob" | "grep" | "list" | "search" | "webfetch" | "websearch" | "notebookread" => "read",
        "edit" | "write" | "multiedit" | "notebookedit" | "patch" | "apply_patch" => "edit",
        _ => "run",
    }
}

/// What a tool is acting on, from the JSON its input carries.
fn target_of(input: &Value) -> String {
    for key in ["file_path", "path", "command", "pattern", "url", "query"] {
        if let Some(text) = input.get(key).and_then(Value::as_str) {
            return text.to_string();
        }
    }

    String::new()
}

/// The sentence a stream that stopped talking ends with, when the CLI itself said nothing.
///
/// `collect_stream` appends it and `explain_failure` replaces it with the CLI's own words when there
/// are any - one sentence, one constant, so the buffered and the streaming path cannot drift.
pub const GENERIC_ENDING: &str = "the engine's stream ended without a result";

/// Turns one adapter's raw stdout lines into events, ending the stream at `result`/`error`.
///
/// Every adapter's `start` is this function plus a `Command`; keeping the loop here means the
/// "what ends a turn" rule is written once.
///
/// The live path does not go through here any more - `CliAdapter::run` parses each line as it arrives
/// and pushes it, because a `Vec` is only complete once the turn is over. This stays because the rule
/// it states is the fixture contract of spec section 11.6, and because it is the shape a replay of a
/// finished stream wants.
pub fn collect_stream<I: IntoIterator<Item = String>>(lines: I) -> Vec<EngineEvent> {
    let mut events = Vec::new();

    for line in lines {
        for event in parse_stream_line(&line) {
            let terminal = event.is_terminal();

            events.push(event);

            if terminal {
                return events;
            }
        }
    }

    /* A stream that ended without a `result` line is a turn that stopped talking. Saying so is the
    honest thing; silence would look like success (principle P4). */
    if !matches!(
        events.last(),
        Some(EngineEvent::Done { .. } | EngineEvent::Failed(_))
    ) {
        events.push(EngineEvent::Failed(GENERIC_ENDING.to_string()));
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_five_event_kinds() {
        assert_eq!(
            parse_stream_line(r#"{"type":"assistant_text","text":"hello"}"#),
            vec![EngineEvent::Delta("hello".into())]
        );
        assert_eq!(
            parse_stream_line(r#"{"type":"thinking","delta":"why"}"#),
            vec![EngineEvent::Thinking("why".into())]
        );
        assert_eq!(
            parse_stream_line(r#"{"type":"tool_use","id":"c1","tool":"Edit","name":"Edit","target":"a.ts"}"#),
            vec![EngineEvent::ToolStarted { call_id: "c1".into(), tool: "edit".into(), name: "Edit".into(), target: "a.ts".into() }]
        );
        assert_eq!(
            parse_stream_line(r#"{"type":"result","summary":"Done","meta":"1s","pass":true}"#),
            vec![EngineEvent::Done { summary: "Done".into(), meta: "1s".into(), pass: Some(true) }]
        );
        assert_eq!(
            parse_stream_line(r#"{"type":"error","message":"boom"}"#),
            vec![EngineEvent::Failed("boom".into())]
        );
    }

    #[test]
    fn skips_noise_and_reports_a_stream_that_never_finished() {
        assert!(parse_stream_line("npm warn deprecated").is_empty());
        assert!(parse_stream_line("").is_empty());

        let events = collect_stream(vec![r#"{"type":"text","text":"hi"}"#.to_string()]);

        assert_eq!(events.len(), 2);
        assert!(matches!(events.last(), Some(EngineEvent::Failed(_))));
    }

    #[test]
    fn stops_at_the_terminal_event() {
        let events = collect_stream(vec![
            r#"{"type":"assistant_text","text":"a"}"#.to_string(),
            r#"{"type":"done","summary":"Done"}"#.to_string(),
            r#"{"type":"assistant_text","text":"after the end"}"#.to_string(),
        ]);

        assert_eq!(events.len(), 2);
    }
    /*
     * The shapes below are the ones the installed CLIs printed, copied out of the raw captures written
     * by `node _verify/cli-capture.mjs`. They are here because the invented shape this parser was
     * first written against is exactly why every chat turn produced nothing.
     */

    #[test]
    fn a_real_claude_turn_parses_to_text_and_a_finished_turn() {
        let lines = vec![
            r#"{"type":"system","subtype":"init","cwd":"H:\\SDC","tools":["Bash","Read"]}"#.to_string(),
            r#"{"type":"system","subtype":"status","status":"requesting"}"#.to_string(),
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-sonnet-5"}}}"#.to_string(),
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#.to_string(),
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"O"}}}"#.to_string(),
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"K"}}}"#.to_string(),
            /* The whole message, which the deltas already carried: it must not be emitted again. */
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"OK"}]}}"#.to_string(),
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0}}"#.to_string(),
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#.to_string(),
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning"}}"#.to_string(),
            r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":3293,"total_cost_usd":0.029004,"result":"OK"}"#.to_string(),
        ];

        let events = collect_stream(lines);

        assert_eq!(
            events,
            vec![
                EngineEvent::Delta("O".into()),
                EngineEvent::Delta("K".into()),
                EngineEvent::Done { summary: "Done".into(), meta: "$0.0290 · 3.3s".into(), pass: Some(true) },
            ]
        );
    }

    #[test]
    fn a_real_codex_turn_parses_to_text_and_a_finished_turn() {
        let lines = vec![
            r#"{"type":"thread.started","thread_id":"01a0bee0"}"#.to_string(),
            r#"{"type":"turn.started"}"#.to_string(),
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}"#.to_string(),
            r#"{"type":"turn.completed","usage":{"input_tokens":12926,"output_tokens":5}}"#.to_string(),
        ];

        let events = collect_stream(lines);

        assert_eq!(
            events,
            vec![
                EngineEvent::Delta("OK".into()),
                EngineEvent::Done {
                    summary: "Done".into(),
                    meta: "12926 in · 5 out".into(),
                    pass: Some(true),
                },
            ]
        );
    }

    #[test]
    fn a_codex_command_becomes_a_run_tool() {
        let events = parse_stream_line(
            r#"{"type":"item.completed","item":{"id":"item_3","type":"command_execution","command":"npm test","exit_code":0}}"#,
        );

        assert_eq!(
            events,
            vec![EngineEvent::ToolStarted {
                call_id: "item_3".into(),
                tool: "run".into(),
                name: "npm".into(),
                target: "npm test".into(),
            }]
        );
    }

    #[test]
    fn a_claude_tool_block_becomes_a_tool_with_its_target() {
        let events = parse_stream_line(
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"H:\\SDC\\README.md"}}}}"#,
        );

        assert_eq!(
            events,
            vec![EngineEvent::ToolStarted {
                call_id: "toolu_1".into(),
                tool: "read".into(),
                name: "Read".into(),
                target: "H:\\SDC\\README.md".into(),
            }]
        );
    }

    #[test]
    fn gemini_messages_are_deltas_and_its_result_ends_the_turn() {
        assert_eq!(
            parse_stream_line(r#"{"type":"message","role":"assistant","content":"OK","delta":true}"#),
            vec![EngineEvent::Delta("OK".into())]
        );

        let ending = parse_stream_line(r#"{"type":"result","stats":{"total_tokens":42}}"#);

        assert!(matches!(ending.as_slice(), [EngineEvent::Done { .. }]));
    }

    /// A codex `error` **item** is a note, not the end of the turn.
    ///
    /// Measured: `codex exec -m gpt-5-codex` answers
    /// `{"type":"item.completed","item":{"type":"error","message":"Model metadata for `gpt-5-codex` not
    /// found. Defaulting to fallback metadata…"}}` and then runs the turn anyway. Treating it as a
    /// failure ended the stream at the note and threw the answer away. `turn.failed` is still fatal.
    #[test]
    fn a_codex_note_does_not_end_the_turn() {
        let events = collect_stream(vec![
            r#"{"type":"thread.started","thread_id":"t"}"#.to_string(),
            r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Model metadata for `gpt-5-codex` not found."}}"#.to_string(),
            r#"{"type":"turn.started"}"#.to_string(),
            r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"OK"}}"#.to_string(),
            r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":2}}"#.to_string(),
        ]);

        assert!(
            matches!(events.as_slice(), [EngineEvent::ToolOutput { .. }, EngineEvent::Delta(text), EngineEvent::Done { .. }] if text == "OK"),
            "{events:?}"
        );
    }

    /// And the fatal one is fatal, in the CLI's own words.
    #[test]
    fn a_codex_turn_failure_keeps_the_providers_message() {
        let events = collect_stream(vec![
            r#"{"type":"error","message":"{\"type\":\"error\",\"status\":400}"}"#.to_string(),
            r#"{"type":"turn.failed","error":{"message":"The 'x' model is not supported when using Codex with a ChatGPT account."}}"#.to_string(),
        ]);

        assert!(matches!(events.first(), Some(EngineEvent::Failed(_))), "{events:?}");
    }

    #[test]
    fn a_failed_result_is_a_failure_not_an_empty_answer() {
        let events = parse_stream_line(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Credit balance is too low"}"#,
        );

        assert_eq!(events, vec![EngineEvent::Failed("Credit balance is too low".into())]);
    }
}
