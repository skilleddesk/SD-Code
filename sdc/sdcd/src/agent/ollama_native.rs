//! The agent's road to a local model: Ollama's own `/api/chat` (0.16.1).
//!
//! The agent used to reach Ollama through its OpenAI-compatible `/v1/chat/completions`, which works for
//! tools but has no way to say how much context to load - Ollama's own documentation: "The OpenAI API
//! does not have a way of setting the context size for a model." So every local turn ran at Ollama's
//! default, which is **4 096 tokens** on a machine with less than 24 GiB of VRAM, while SDC planned the
//! conversation for 32 000: the system prompt and the tool list alone filled it, and Ollama cut the rest
//! off without a word. The native endpoint takes `options.num_ctx`.
//!
//! The rest of the agent speaks the OpenAI dialect - its messages, its reply reader - and keeps doing so.
//! This module is the translation at the edge, in both directions:
//!
//! * `body` turns the OpenAI request into a native one: tool-call arguments as objects rather than JSON
//!   text, a tool result named after its call, images as base64 beside the text, and `num_ctx`;
//! * `as_sse` turns the native NDJSON stream into the OpenAI SSE lines `dialect::read_reply` reads.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Read};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Map, Value};

/// The native chat endpoint.
pub const URL: &str = "http://127.0.0.1:11434/api/chat";

/// The OpenAI-shaped request `dialect::body` built, as the native endpoint takes it.
pub fn body(openai: Value, num_ctx: u64) -> Value {
    let mut names: HashMap<String, String> = HashMap::new();
    let messages: Vec<Value> = openai["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|message| native_message(message, &mut names))
        .collect();

    json!({
        "model": openai["model"],
        "stream": true,
        "messages": messages,
        "tools": openai["tools"],
        "options": { "num_ctx": num_ctx },
    })
}

fn native_message(message: Value, names: &mut HashMap<String, String>) -> Value {
    let role = message["role"].as_str().unwrap_or("user").to_string();
    let mut out = Map::new();

    out.insert("role".into(), json!(role));

    match &message["content"] {
        /* Text and images: the native API carries the images beside the text, as bare base64. */
        Value::Array(parts) => {
            let text: Vec<&str> = parts.iter().filter_map(|part| part["text"].as_str()).collect();
            let images: Vec<String> = parts
                .iter()
                .filter_map(|part| part.pointer("/image_url/url").and_then(Value::as_str))
                .map(|url| url.split_once("base64,").map(|(_, data)| data).unwrap_or(url).to_string())
                .collect();

            out.insert("content".into(), json!(text.join("\n")));

            if !images.is_empty() {
                out.insert("images".into(), json!(images));
            }
        }
        Value::String(text) => {
            out.insert("content".into(), json!(text));
        }
        _ => {
            out.insert("content".into(), json!(""));
        }
    }

    if let Some(calls) = message["tool_calls"].as_array() {
        let native: Vec<Value> = calls
            .iter()
            .map(|call| {
                let id = call["id"].as_str().unwrap_or_default().to_string();
                let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default().to_string();
                let arguments = match call.pointer("/function/arguments") {
                    Some(Value::String(raw)) => serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({})),
                    Some(object @ Value::Object(_)) => object.clone(),
                    _ => json!({}),
                };

                names.insert(id, name.clone());

                json!({ "function": { "name": name, "arguments": arguments } })
            })
            .collect();

        out.insert("tool_calls".into(), json!(native));
    }

    if role == "tool" {
        if let Some(name) = message["tool_call_id"].as_str().and_then(|id| names.get(id)) {
            out.insert("tool_name".into(), json!(name));
        }
    }

    Value::Object(out)
}

/// A process-wide number for tool-call ids, since the native API sends none.
static CALLS: AtomicU64 = AtomicU64::new(1);

/// The native NDJSON stream, read as the OpenAI SSE stream `dialect::read_reply` expects.
pub fn as_sse(lines: Box<dyn BufRead + Send>) -> Box<dyn BufRead + Send> {
    Box::new(std::io::BufReader::new(NdjsonAsSse { inner: lines, out: VecDeque::new(), calls: 0, ended: false }))
}

struct NdjsonAsSse {
    inner: Box<dyn BufRead + Send>,
    out: VecDeque<u8>,
    /// Tool calls seen so far in this reply - their `index` in the OpenAI shape.
    calls: u64,
    ended: bool,
}

impl NdjsonAsSse {
    fn push(&mut self, value: Value) {
        self.out.extend(format!("data: {value}\n\n").bytes());
    }

    /// One native line, as zero or more SSE lines.
    fn translate(&mut self, line: &str) {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            return;
        };

        if let Some(error) = value["error"].as_str() {
            self.push(json!({ "error": { "message": error } }));

            return;
        }

        let message = &value["message"];
        let mut delta = Map::new();

        if let Some(text) = message["content"].as_str().filter(|text| !text.is_empty()) {
            delta.insert("content".into(), json!(text));
        }

        if let Some(thinking) = message["thinking"].as_str().filter(|text| !text.is_empty()) {
            delta.insert("reasoning_content".into(), json!(thinking));
        }

        if let Some(calls) = message["tool_calls"].as_array().filter(|calls| !calls.is_empty()) {
            let shaped: Vec<Value> = calls
                .iter()
                .map(|call| {
                    let index = self.calls;

                    self.calls += 1;

                    let id = call["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_local_{}", CALLS.fetch_add(1, Ordering::SeqCst)));
                    let arguments = match call.pointer("/function/arguments") {
                        Some(Value::String(raw)) => raw.clone(),
                        Some(other) => other.to_string(),
                        None => "{}".to_string(),
                    };

                    json!({
                        "index": index,
                        "id": id,
                        "type": "function",
                        "function": { "name": call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default(), "arguments": arguments },
                    })
                })
                .collect();

            delta.insert("tool_calls".into(), json!(shaped));
        }

        let done = value["done"].as_bool().unwrap_or(false);

        if !delta.is_empty() || !done {
            self.push(json!({ "choices": [{ "index": 0, "delta": Value::Object(delta) }] }));
        }

        if done {
            let finish = match value["done_reason"].as_str() {
                Some("length") => "length",
                _ if self.calls > 0 => "tool_calls",
                _ => "stop",
            };

            self.push(json!({
                "choices": [{ "index": 0, "delta": {}, "finish_reason": finish }],
                "usage": {
                    "prompt_tokens": value["prompt_eval_count"].as_u64().unwrap_or(0),
                    "completion_tokens": value["eval_count"].as_u64().unwrap_or(0),
                },
            }));
            self.out.extend(b"data: [DONE]\n\n");
            self.ended = true;
        }
    }
}

impl Read for NdjsonAsSse {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        while self.out.is_empty() {
            if self.ended {
                return Ok(0);
            }

            let mut line = String::new();

            if self.inner.read_line(&mut line)? == 0 {
                self.ended = true;

                return Ok(0);
            }

            self.translate(&line);
        }

        let count = buffer.len().min(self.out.len());

        for (slot, byte) in buffer.iter_mut().zip(self.out.drain(..count)) {
            *slot = byte;
        }

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::dialect::{read_reply, Dialect};
    use crate::engines::Recorder;

    /// The request: arguments as objects, a tool result named after its call, images beside the text, and
    /// the context size the whole change is for.
    #[test]
    fn the_request_is_the_native_shape_with_num_ctx() {
        let openai = json!({
            "model": "qwen3.5:9b",
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": [{ "type": "function", "function": { "name": "read_file", "description": "Read", "parameters": {} } }],
            "messages": [
                { "role": "system", "content": "You are SDC Agent." },
                { "role": "user", "content": [
                    { "type": "text", "text": "What is in this?" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
                ] },
                { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.md\"}" } },
                ] },
                { "role": "tool", "tool_call_id": "c1", "content": "hello" },
            ],
        });
        let native = body(openai, 16_384);

        assert_eq!(native["options"]["num_ctx"], 16_384);
        assert!(native.get("stream_options").is_none());
        assert_eq!(native["messages"][1]["content"], "What is in this?");
        assert_eq!(native["messages"][1]["images"][0], "AAAA");
        assert_eq!(native["messages"][2]["content"], "");
        assert_eq!(native["messages"][2]["tool_calls"][0]["function"]["arguments"]["path"], "a.md");
        assert_eq!(native["messages"][3]["tool_name"], "read_file");
    }

    /// The stream: text, thinking and a tool call arrive through the same reader every other model uses.
    #[test]
    fn the_native_stream_reads_as_a_reply() {
        let ndjson = [
            r#"{"message":{"role":"assistant","content":"","thinking":"Look first."},"done":false}"#,
            r#"{"message":{"role":"assistant","content":"Reading it."},"done":false}"#,
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"read_file","arguments":{"path":"a.md"}}}]},"done":false}"#,
            r#"{"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":321,"eval_count":12}"#,
        ]
        .join("\n");
        let recorder = Recorder::new();
        let lines: Box<dyn BufRead + Send> = Box::new(std::io::Cursor::new(ndjson.into_bytes()));
        let reply = read_reply(Dialect::OpenAi, as_sse(lines), &recorder.sink(), &|| false).unwrap();

        assert_eq!(reply.text, "Reading it.");
        assert_eq!(reply.tool_uses.len(), 1);
        assert_eq!(reply.tool_uses[0].name, "read_file");
        assert_eq!(reply.tool_uses[0].input["path"], "a.md");
        assert!(reply.tool_uses[0].id.starts_with("call_local_"));
        assert_eq!(reply.stop, "tool_calls");
        assert_eq!((reply.input_tokens, reply.output_tokens), (321, 12));
    }

    /// An error line - a model that is not pulled, a machine out of memory - is the reply's error.
    #[test]
    fn an_error_line_fails_the_reply() {
        let lines: Box<dyn BufRead + Send> = Box::new(std::io::Cursor::new(br#"{"error":"model \"qwen3.5:9b\" not found, try pulling it first"}"#.to_vec()));
        let error = read_reply(Dialect::OpenAi, as_sse(lines), &Recorder::new().sink(), &|| false).unwrap_err();

        assert!(error.contains("not found"), "{error}");
    }
}
