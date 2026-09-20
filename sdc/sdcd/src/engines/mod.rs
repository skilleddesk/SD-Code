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

pub mod claude_code;
pub mod cli;
pub mod codex;
pub mod gemini;
pub mod native_api;
pub mod ollama;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

/// What a turn asks an engine to do.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub session_id: String,
    pub turn_id: String,
    pub text: String,
    /// The conversation so far, oldest first - the Session Bridge's payload (spec section 16.5).
    pub history: Vec<String>,
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
    Done { summary: String, meta: String, pass: Option<bool> },
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

/// The contract. `start` returns the whole stream as a vector; the daemon's turn task forwards each
/// item as a notification, which keeps every adapter free of transport knowledge.
#[async_trait]
pub trait Engine: Send + Sync {
    /// `claude_code`, `codex`, `gemini`, `native_api`, `ollama` - the id in `engine.start`.
    fn id(&self) -> &'static str;
    async fn start(&self, prompt: Prompt) -> Vec<EngineEvent>;
    async fn cancel(&self, turn_id: &str) -> bool;
    fn status(&self, turn_id: &str) -> EngineStatus;
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
        "text" | "assistant_text" | "content_block_delta" => {
            text_of(&value).map(EngineEvent::Delta).into_iter().collect()
        }
        "thinking" | "reasoning" => text_of(&value).map(EngineEvent::Thinking).into_iter().collect(),
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
        "error" => vec![EngineEvent::Failed(string_of(&value, "message", "engine error"))],
        "result" | "done" => vec![EngineEvent::Done {
            summary: string_of(&value, "summary", "Done"),
            meta: string_of(&value, "meta", ""),
            pass: value.get("pass").and_then(Value::as_bool),
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

/// Turns one adapter's raw stdout lines into events, ending the stream at `result`/`error`.
///
/// Every adapter's `start` is this function plus a `Command`; keeping the loop here means the
/// "what ends a turn" rule is written once.
pub fn collect_stream<I: IntoIterator<Item = String>>(lines: I) -> Vec<EngineEvent> {
    let mut events = Vec::new();

    for line in lines {
        for event in parse_stream_line(&line) {
            let terminal = matches!(event, EngineEvent::Done { .. } | EngineEvent::Failed(_));

            events.push(event);

            if terminal {
                return events;
            }
        }
    }

    /* A stream that ended without a `result` line is a turn that stopped talking. Saying so is the
       honest thing; silence would look like success (principle P4). */
    if !matches!(events.last(), Some(EngineEvent::Done { .. } | EngineEvent::Failed(_))) {
        events.push(EngineEvent::Failed("the engine's stream ended without a result".to_string()));
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
}
