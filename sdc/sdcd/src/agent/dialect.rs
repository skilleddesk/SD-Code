//! The two tool-calling dialects the agent speaks, over one streaming request each.
//!
//! **Anthropic** (`/v1/messages`): tools are `{name, description, input_schema}`, the reply is a list
//! of content blocks (`thinking`, `text`, `tool_use`), and a tool's answer goes back as a `tool_result`
//! block in the next user message. The reply's blocks are kept **exactly as they came**, thinking and
//! its signature included: a model that thinks between tool calls rejects a continuation whose earlier
//! thinking was dropped or edited.
//!
//! **OpenAI-compatible** (`/chat/completions` - OpenAI, DeepSeek, Groq, OpenRouter, a local server,
//! and Ollama's own `/v1` endpoint): tools are `{type: "function", function: {…}}`, a tool call arrives
//! in pieces keyed by `index`, and its answer goes back as a `role: "tool"` message.
//!
//! Both are read line by line off the same transport the chat path uses (`native_api::open_stream`),
//! and the text and the reasoning are pushed to the window **as they arrive** - the tool calls are the
//! only part that has to be assembled before it means anything.

use std::io::BufRead;

use serde_json::{json, Map, Value};

use crate::engines::{EngineEvent, EventSink};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Anthropic,
    OpenAi,
}

/// One tool call, assembled.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// One model reply, assembled from its stream.
#[derive(Debug, Clone, Default)]
pub struct Reply {
    /// The assistant message to append to the conversation, in the dialect's own shape.
    pub message: Value,
    pub text: String,
    pub tool_uses: Vec<ToolUse>,
    /// `end_turn` / `tool_use` / `max_tokens` / `refusal` (Anthropic), `stop` / `tool_calls` / `length`.
    pub stop: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A tool as the model is told about it.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

/// The request body for one step of the loop.
pub fn body(
    dialect: Dialect,
    model: &str,
    system: &str,
    messages: &[Value],
    tools: &[ToolSpec],
    thinking: bool,
) -> Value {
    match dialect {
        Dialect::Anthropic => {
            let mut body = json!({
                "model": model,
                "max_tokens": 32000,
                "stream": true,
                /* The system prompt and the tool list are the same for every step of a turn, so they are
                   cached once and read back on every later step for a tenth of the price. */
                "system": [{ "type": "text", "text": system, "cache_control": { "type": "ephemeral" } }],
                "tools": tools.iter().map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.schema,
                })).collect::<Vec<_>>(),
                "messages": messages,
            });

            if thinking {
                body["thinking"] = json!({ "type": "adaptive", "display": "summarized" });
            }

            body
        }
        Dialect::OpenAi => {
            let mut all = vec![json!({ "role": "system", "content": system })];

            all.extend(messages.iter().cloned());

            json!({
                "model": model,
                "stream": true,
                "stream_options": { "include_usage": true },
                "tools": tools.iter().map(|tool| json!({
                    "type": "function",
                    "function": { "name": tool.name, "description": tool.description, "parameters": tool.schema },
                })).collect::<Vec<_>>(),
                "messages": all,
            })
        }
    }
}

/// A plain user message, in either dialect.
pub fn user_message(text: &str) -> Value {
    json!({ "role": "user", "content": text })
}

/// A plain assistant message (a replayed earlier answer), in either dialect.
pub fn assistant_message(text: &str) -> Value {
    json!({ "role": "assistant", "content": text })
}

/// The tool answers of one step, as the message(s) that carry them back.
///
/// Anthropic wants **one** user message holding every `tool_result` of the step - splitting them teaches
/// the model to stop calling tools in parallel. OpenAI wants one `role: "tool"` message per call.
pub fn tool_results(dialect: Dialect, results: &[(String, String, bool)]) -> Vec<Value> {
    match dialect {
        Dialect::Anthropic => vec![json!({
            "role": "user",
            "content": results.iter().map(|(id, content, is_error)| json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": content,
                "is_error": is_error,
            })).collect::<Vec<_>>(),
        })],
        Dialect::OpenAi => results
            .iter()
            .map(|(id, content, _)| json!({ "role": "tool", "tool_call_id": id, "content": content }))
            .collect(),
    }
}

/// Reads one reply off the stream, pushing text and thinking to `sink` as they arrive.
///
/// `stop` is checked between lines, so a cancelled turn drops the connection instead of reading (and
/// paying for) the rest of the answer. An `Err` is a reply that cannot be used: the provider's own
/// error sentence, or a stream cut before it said anything.
pub fn read_reply(
    dialect: Dialect,
    mut lines: Box<dyn BufRead + Send>,
    sink: &EventSink,
    stop: &dyn Fn() -> bool,
) -> Result<Reply, String> {
    let mut state = match dialect {
        Dialect::Anthropic => Assembler::Anthropic(AnthropicState::default()),
        Dialect::OpenAi => Assembler::OpenAi(OpenAiState::default()),
    };
    let mut line = String::new();

    loop {
        if stop() {
            return Err("stopped".to_string());
        }

        line.clear();

        match lines.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if state.feed(line.trim_end(), sink)? {
                    break;
                }
            }
            Err(error) => return Err(format!("the connection to the provider dropped: {error}")),
        }
    }

    state.finish()
}

enum Assembler {
    Anthropic(AnthropicState),
    OpenAi(OpenAiState),
}

impl Assembler {
    /// One SSE line. `Ok(true)` when the stream says it is over.
    fn feed(&mut self, line: &str, sink: &EventSink) -> Result<bool, String> {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            return Ok(false);
        };

        if data == "[DONE]" {
            return Ok(true);
        }

        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Ok(false);
        };

        if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
            return Err(message.to_string());
        }

        match self {
            Assembler::Anthropic(state) => state.feed(&value, sink),
            Assembler::OpenAi(state) => state.feed(&value, sink),
        }
    }

    fn finish(self) -> Result<Reply, String> {
        match self {
            Assembler::Anthropic(state) => state.finish(),
            Assembler::OpenAi(state) => state.finish(),
        }
    }
}

#[derive(Default)]
struct AnthropicState {
    /// The content blocks in index order; a `tool_use` block's input is kept as text until it stops.
    blocks: Vec<Value>,
    partial_json: Vec<String>,
    text: String,
    stop: String,
    input_tokens: u64,
    output_tokens: u64,
    ended: bool,
}

impl AnthropicState {
    fn feed(&mut self, value: &Value, sink: &EventSink) -> Result<bool, String> {
        match value["type"].as_str().unwrap_or_default() {
            "message_start" => {
                let usage = &value["message"]["usage"];

                self.input_tokens = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
                    .iter()
                    .map(|key| usage[*key].as_u64().unwrap_or(0))
                    .sum();
            }
            "content_block_start" => {
                let index = value["index"].as_u64().unwrap_or(self.blocks.len() as u64) as usize;
                let mut block = value["content_block"].clone();

                if block["type"] == "tool_use" {
                    block["input"] = json!({});
                }

                while self.blocks.len() <= index {
                    self.blocks.push(Value::Null);
                    self.partial_json.push(String::new());
                }

                self.blocks[index] = block;
            }
            "content_block_delta" => {
                let index = value["index"].as_u64().unwrap_or(0) as usize;
                let delta = &value["delta"];

                if index >= self.blocks.len() {
                    return Ok(false);
                }

                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let piece = delta["text"].as_str().unwrap_or_default();

                        append(&mut self.blocks[index], "text", piece);
                        self.text.push_str(piece);
                        sink.send(EngineEvent::Delta(piece.to_string()));
                    }
                    "thinking_delta" => {
                        let piece = delta["thinking"].as_str().unwrap_or_default();

                        append(&mut self.blocks[index], "thinking", piece);
                        sink.send(EngineEvent::Thinking(piece.to_string()));
                    }
                    "signature_delta" => {
                        append(&mut self.blocks[index], "signature", delta["signature"].as_str().unwrap_or_default());
                    }
                    "input_json_delta" => {
                        self.partial_json[index].push_str(delta["partial_json"].as_str().unwrap_or_default());
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = value["index"].as_u64().unwrap_or(0) as usize;

                if index < self.blocks.len() && self.blocks[index]["type"] == "tool_use" {
                    let raw = self.partial_json[index].trim();
                    let input = if raw.is_empty() { Ok(json!({})) } else { serde_json::from_str::<Value>(raw) };

                    /* A tool input that does not parse is kept as a marker, not dropped: the loop answers
                       it with an error the model can correct, and the block still matches its id. */
                    self.blocks[index]["input"] = input.unwrap_or_else(|_| json!({ "__invalid_json": raw }));
                }
            }
            "message_delta" => {
                if let Some(stop) = value["delta"]["stop_reason"].as_str() {
                    self.stop = stop.to_string();
                }

                if let Some(tokens) = value["usage"]["output_tokens"].as_u64() {
                    self.output_tokens = tokens;
                }
            }
            "message_stop" => {
                self.ended = true;

                return Ok(true);
            }
            _ => {}
        }

        Ok(false)
    }

    fn finish(self) -> Result<Reply, String> {
        if !self.ended && self.blocks.is_empty() {
            return Err("the provider's stream ended before it said anything".to_string());
        }

        /* An empty text block is refused if it is sent back, so it is not kept. Thinking blocks are kept
           even when their text is empty: the signature is what the next request is checked against. */
        let content: Vec<Value> = self
            .blocks
            .into_iter()
            .filter(|block| !block.is_null())
            .filter(|block| !(block["type"] == "text" && block["text"].as_str().unwrap_or_default().is_empty()))
            .collect();
        let tool_uses = content
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .map(|block| ToolUse {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: block["name"].as_str().unwrap_or_default().to_string(),
                input: block["input"].clone(),
            })
            .collect();

        Ok(Reply {
            message: json!({ "role": "assistant", "content": content }),
            text: self.text,
            tool_uses,
            stop: self.stop,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }
}

#[derive(Default)]
struct OpenAiState {
    text: String,
    /// Tool calls by `index`: `(id, name, arguments so far)`.
    calls: Vec<(String, String, String)>,
    stop: String,
    input_tokens: u64,
    output_tokens: u64,
    spoke: bool,
}

impl OpenAiState {
    fn feed(&mut self, value: &Value, sink: &EventSink) -> Result<bool, String> {
        if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
            self.input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(self.input_tokens);
            self.output_tokens = usage["completion_tokens"].as_u64().unwrap_or(self.output_tokens);
        }

        let Some(choice) = value.pointer("/choices/0") else {
            return Ok(false);
        };
        let delta = &choice["delta"];

        self.spoke = true;

        for key in ["reasoning_content", "reasoning"] {
            if let Some(piece) = delta[key].as_str().filter(|piece| !piece.is_empty()) {
                sink.send(EngineEvent::Thinking(piece.to_string()));
            }
        }

        if let Some(piece) = delta["content"].as_str().filter(|piece| !piece.is_empty()) {
            self.text.push_str(piece);
            sink.send(EngineEvent::Delta(piece.to_string()));
        }

        for call in delta["tool_calls"].as_array().cloned().unwrap_or_default() {
            let index = call["index"].as_u64().unwrap_or(self.calls.len() as u64) as usize;

            while self.calls.len() <= index {
                self.calls.push((String::new(), String::new(), String::new()));
            }

            let slot = &mut self.calls[index];

            if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
                slot.0 = id.to_string();
            }

            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                slot.1.push_str(name);
            }

            if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
                slot.2.push_str(arguments);
            }
        }

        if let Some(reason) = choice["finish_reason"].as_str() {
            self.stop = reason.to_string();
        }

        Ok(false)
    }

    fn finish(self) -> Result<Reply, String> {
        if !self.spoke {
            return Err("the provider's stream ended before it said anything".to_string());
        }

        let calls: Vec<(String, String, String)> = self
            .calls
            .into_iter()
            .enumerate()
            .filter(|(_, (_, name, _))| !name.is_empty())
            /* Ollama and some local servers leave the id out; the loop needs one to pair the answer. */
            .map(|(index, (id, name, arguments))| {
                (if id.is_empty() { format!("call_{index}") } else { id }, name, arguments)
            })
            .collect();
        let tool_uses = calls
            .iter()
            .map(|(id, name, arguments)| {
                let raw = arguments.trim();
                let input = if raw.is_empty() { Ok(json!({})) } else { serde_json::from_str::<Value>(raw) };

                ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.unwrap_or_else(|_| json!({ "__invalid_json": raw })),
                }
            })
            .collect();

        let mut message = Map::new();

        message.insert("role".into(), json!("assistant"));
        message.insert("content".into(), if self.text.is_empty() { Value::Null } else { json!(self.text) });

        if !calls.is_empty() {
            message.insert(
                "tool_calls".into(),
                json!(calls
                    .iter()
                    .map(|(id, name, arguments)| json!({
                        "id": id,
                        "type": "function",
                        "function": { "name": name, "arguments": if arguments.trim().is_empty() { "{}" } else { arguments } },
                    }))
                    .collect::<Vec<_>>()),
            );
        }

        Ok(Reply {
            message: Value::Object(message),
            text: self.text,
            tool_uses,
            stop: self.stop,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }
}

fn append(block: &mut Value, key: &str, piece: &str) {
    let current = block[key].as_str().unwrap_or_default().to_string();

    block[key] = json!(current + piece);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::Recorder;

    fn stream(lines: &[&str]) -> Box<dyn BufRead + Send> {
        Box::new(std::io::Cursor::new(lines.join("\n").into_bytes()))
    }

    /// A real Messages API stream, trimmed: thinking with a signature, a sentence, then a tool call
    /// whose input arrives in three pieces.
    #[test]
    fn anthropic_tool_calls_are_assembled_and_the_thinking_is_kept_with_its_signature() {
        let recorder = Recorder::new();
        let reply = read_reply(
            Dialect::Anthropic,
            stream(&[
                "event: message_start",
                r#"data: {"type":"message_start","message":{"usage":{"input_tokens":120,"cache_read_input_tokens":30}}}"#,
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Read it first."}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-1"}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
                r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
                r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Looking."}}"#,
                r#"data: {"type":"content_block_stop","index":1}"#,
                r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_file","input":{}}}"#,
                r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"pa"}}"#,
                r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"th\": \"src/"}}"#,
                r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"pay.js\"}"}}"#,
                r#"data: {"type":"content_block_stop","index":2}"#,
                r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}"#,
                r#"data: {"type":"message_stop"}"#,
            ]),
            &recorder.sink(),
            &|| false,
        )
        .unwrap();

        assert_eq!(reply.tool_uses, vec![ToolUse { id: "toolu_1".into(), name: "read_file".into(), input: json!({ "path": "src/pay.js" }) }]);
        assert_eq!(reply.stop, "tool_use");
        assert_eq!((reply.input_tokens, reply.output_tokens), (150, 42));
        assert_eq!(reply.message["content"][0], json!({ "type": "thinking", "thinking": "Read it first.", "signature": "sig-1" }));
        assert_eq!(reply.message["content"][1], json!({ "type": "text", "text": "Looking." }));
        assert_eq!(recorder.events(), vec![EngineEvent::Thinking("Read it first.".into()), EngineEvent::Delta("Looking.".into())]);
    }

    #[test]
    fn openai_tool_calls_arrive_in_pieces_by_index() {
        let reply = read_reply(
            Dialect::OpenAi,
            stream(&[
                r#"data: {"choices":[{"delta":{"content":"On it."}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"run_command","arguments":"{\"com"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"mand\":\"npm test\"}"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
                r#"data: {"choices":[],"usage":{"prompt_tokens":80,"completion_tokens":12}}"#,
                "data: [DONE]",
            ]),
            &EventSink::discarding(),
            &|| false,
        )
        .unwrap();

        assert_eq!(reply.tool_uses[0].input, json!({ "command": "npm test" }));
        assert_eq!(reply.message["tool_calls"][0]["function"]["name"], "run_command");
        assert_eq!(reply.message["content"], "On it.");
        assert_eq!((reply.input_tokens, reply.output_tokens), (80, 12));
    }

    #[test]
    fn a_provider_error_inside_the_stream_is_its_own_sentence() {
        let reply = read_reply(
            Dialect::Anthropic,
            stream(&[r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#]),
            &EventSink::discarding(),
            &|| false,
        );

        assert_eq!(reply.unwrap_err(), "Overloaded");
    }

    #[test]
    fn a_stop_drops_the_stream() {
        let reply = read_reply(Dialect::OpenAi, stream(&["data: {}"]), &EventSink::discarding(), &|| true);

        assert_eq!(reply.unwrap_err(), "stopped");
    }

    #[test]
    fn broken_tool_input_is_kept_as_a_marker_so_the_model_can_be_told() {
        let reply = read_reply(
            Dialect::OpenAi,
            stream(&[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#,
                "data: [DONE]",
            ]),
            &EventSink::discarding(),
            &|| false,
        )
        .unwrap();

        assert_eq!(reply.tool_uses[0].id, "call_0", "a missing id is filled in");
        assert!(reply.tool_uses[0].input.get("__invalid_json").is_some());
    }

    #[test]
    fn tool_results_are_one_message_for_anthropic_and_one_per_call_for_openai() {
        let results = vec![("a".to_string(), "ok".to_string(), false), ("b".to_string(), "no".to_string(), true)];

        assert_eq!(tool_results(Dialect::Anthropic, &results).len(), 1);
        assert_eq!(tool_results(Dialect::Anthropic, &results)[0]["content"][1]["is_error"], true);
        assert_eq!(tool_results(Dialect::OpenAi, &results).len(), 2);
        assert_eq!(tool_results(Dialect::OpenAi, &results)[1]["tool_call_id"], "b");
    }
}
