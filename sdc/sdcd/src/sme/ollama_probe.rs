//! The Ollama side of the Capability Probe (P0-9): reads a model's identity and measures it through `/api/chat`.
//!
//! The parsing is separate from the network so it can be checked against the shapes Ollama documents without Ollama:
//! [`identity_from`], [`memory_from`] and [`reply_from_lines`] take JSON text. [`OllamaRunner`] only wires them to
//! `127.0.0.1:11434`. This build was written on a machine with no Ollama, so the wiring itself has not been run
//! against a live server; that is for the person to do (`sdcd probe qwen3.5:9b`).

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::probe::{Ask, ModelKey, Reply, Runner};
use crate::engines::body::{body_lines, read_head};

pub struct OllamaRunner {
    pub model: String,
}

/// `{model, digest, quantization, runtime}` from the three documented answers: `/api/tags` (digest), `/api/show`
/// (`details.quantization_level`) and `/api/version`.
pub fn identity_from(model: &str, tags: &str, show: &str, version: &str) -> Result<ModelKey, String> {
    let tags: Value = serde_json::from_str(tags).map_err(|error| format!("/api/tags: {error}"))?;
    let entry = tags["models"].as_array().into_iter().flatten().find(|entry| entry["name"].as_str() == Some(model) || entry["model"].as_str() == Some(model)).ok_or_else(|| format!("{model} is not installed"))?;
    let show: Value = serde_json::from_str(show).unwrap_or(Value::Null);
    let version: Value = serde_json::from_str(version).unwrap_or(Value::Null);

    Ok(ModelKey {
        provider: "ollama".into(),
        model: model.into(),
        digest: entry["digest"].as_str().unwrap_or("unknown").to_string(),
        quantization: show["details"]["quantization_level"].as_str().or_else(|| entry["details"]["quantization_level"].as_str()).unwrap_or("unknown").to_string(),
        runtime: version["version"].as_str().unwrap_or("unknown").to_string(),
    })
}

/// `(bytes in video memory, share on the GPU)` for the loaded model, from `/api/ps`.
pub fn memory_from(model: &str, ps: &str) -> Option<(u64, f64)> {
    let ps: Value = serde_json::from_str(ps).ok()?;
    let entry = ps["models"].as_array()?.iter().find(|entry| entry["name"].as_str() == Some(model) || entry["model"].as_str() == Some(model))?;
    let (size, vram) = (entry["size"].as_u64()?, entry["size_vram"].as_u64()?);

    Some((vram, if size == 0 { 0.0 } else { vram as f64 / size as f64 }))
}

/// A reply from the NDJSON lines of `/api/chat`, each with the moment (ms since the request) it arrived.
pub fn reply_from_lines(lines: &[(u64, String)]) -> Reply {
    let mut reply = Reply::default();
    let mut first = None;

    for (at, line) in lines {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let content = value["message"]["content"].as_str().unwrap_or("");
        let thinking = value["message"]["thinking"].as_str().unwrap_or("");
        let calls = value["message"]["tool_calls"].as_array();

        if first.is_none() && (!content.is_empty() || !thinking.is_empty() || calls.is_some_and(|calls| !calls.is_empty())) {
            first = Some(*at);
        }

        reply.text.push_str(content);

        if !thinking.is_empty() {
            reply.had_thinking = true;
        }

        for call in calls.into_iter().flatten() {
            reply.tool_calls.push((call["function"]["name"].as_str().unwrap_or("").to_string(), call["function"]["arguments"].clone()));
        }

        if value.get("logprobs").is_some_and(|logprobs| !logprobs.is_null()) || value["message"].get("logprobs").is_some_and(|logprobs| !logprobs.is_null()) {
            reply.had_logprobs = true;
        }

        if value["done"] == true {
            reply.prompt_tokens = value["prompt_eval_count"].as_u64().unwrap_or(0);
            reply.output_tokens = value["eval_count"].as_u64().unwrap_or(0);
            reply.total_ms = *at;
        }
    }

    reply.first_token_ms = first.unwrap_or(reply.total_ms);

    reply
}

fn request_body(model: &str, ask: &Ask) -> Value {
    let mut body = json!({
        "model": model,
        "stream": true,
        "messages": [{ "role": "user", "content": ask.prompt }],
        /* temperature 0 and a fixed seed: the same question gets the same answer, so two probes are comparable. */
        "options": { "num_ctx": ask.num_ctx, "num_predict": ask.max_output, "temperature": 0, "seed": ask.seed },
        "logprobs": true,
    });

    if let Some(think) = ask.think {
        body["think"] = json!(think);
    }

    if let Some((name, schema)) = &ask.tool {
        body["tools"] = json!([{ "type": "function", "function": { "name": name, "description": "A tool.", "parameters": schema } }]);
    }

    if let Some(schema) = &ask.schema {
        body["format"] = schema.clone();
    }

    body
}

impl Runner for OllamaRunner {
    fn identity(&self) -> Result<ModelKey, String> {
        let tags = crate::engines::ollama::call("/api/tags", None)?;
        let show = crate::engines::ollama::call("/api/show", Some(&json!({ "model": self.model }).to_string())).unwrap_or_default();
        let version = crate::engines::ollama::call("/api/version", None).unwrap_or_default();

        identity_from(&self.model, &tags, &show, &version)
    }

    fn ask(&mut self, ask: &Ask) -> Result<Reply, String> {
        let address: SocketAddr = crate::engines::ollama::ENDPOINT.parse().map_err(|error| format!("{error}"))?;
        let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|error| format!("Ollama is not answering: {error}"))?;
        let body = request_body(&self.model, ask).to_string();

        /* A probe asks for long contexts on a cold model: a read may take minutes. */
        socket.set_read_timeout(Some(Duration::from_secs(600))).ok();
        socket
            .write_all(format!("POST /api/chat HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", crate::engines::ollama::ENDPOINT, body.len()).as_bytes())
            .map_err(|error| error.to_string())?;

        let started = Instant::now();
        let mut reader = BufReader::new(socket);
        let head = read_head(&mut reader).map_err(|error| error.to_string())?;

        if head.status != 0 && !(200..300).contains(&head.status) {
            return Err(format!("Ollama answered HTTP {}", head.status));
        }

        let mut lines = Vec::new();

        for line in body_lines(reader, head.chunked).lines().map_while(Result::ok) {
            lines.push((started.elapsed().as_millis() as u64, line));
        }

        Ok(reply_from_lines(&lines))
    }

    fn memory(&mut self) -> Option<(u64, f64)> {
        memory_from(&self.model, &crate::engines::ollama::call("/api/ps", None).ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAGS: &str = r#"{"models":[{"name":"qwen3.5:9b","model":"qwen3.5:9b","digest":"sha256:6a1b2c","details":{"quantization_level":"Q4_K_M"}},{"name":"other:1b","digest":"sha256:ffff"}]}"#;

    #[test]
    fn identity_is_the_digest_the_quantization_and_the_runtime() {
        let key = identity_from("qwen3.5:9b", TAGS, r#"{"details":{"quantization_level":"Q4_K_M"}}"#, r#"{"version":"0.12.3"}"#).unwrap();

        assert_eq!((key.digest.as_str(), key.quantization.as_str(), key.runtime.as_str()), ("sha256:6a1b2c", "Q4_K_M", "0.12.3"));
        assert!(identity_from("missing:1b", TAGS, "", "").unwrap_err().contains("not installed"));

        /* A server that answers /api/show and /api/version with nothing still gives an identity - "unknown" parts,
           never a guess. */
        let bare = identity_from("qwen3.5:9b", TAGS, "", "").unwrap();

        assert_eq!(bare.runtime, "unknown");
        assert_eq!(bare.quantization, "Q4_K_M", "falls back to /api/tags' own details");
    }

    #[test]
    fn vram_and_gpu_share_come_from_ps() {
        let ps = r#"{"models":[{"name":"qwen3.5:9b","size":7000000000,"size_vram":6300000000}]}"#;

        assert_eq!(memory_from("qwen3.5:9b", ps), Some((6_300_000_000, 0.9)));
        assert_eq!(memory_from("another:1b", ps), None);
        assert_eq!(memory_from("qwen3.5:9b", "not json"), None);
    }

    #[test]
    fn a_reply_carries_text_calls_timings_and_counts() {
        let lines: Vec<(u64, String)> = vec![
            (400, r#"{"message":{"role":"assistant","content":"","thinking":"hm"},"done":false}"#.into()),
            (450, r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"add","arguments":{"a":1,"b":2}}}]},"done":false}"#.into()),
            (900, r#"{"message":{"role":"assistant","content":"done"},"done":true,"prompt_eval_count":321,"eval_count":40}"#.into()),
        ];
        let reply = reply_from_lines(&lines);

        assert_eq!(reply.first_token_ms, 400);
        assert_eq!(reply.total_ms, 900);
        assert_eq!((reply.prompt_tokens, reply.output_tokens), (321, 40));
        assert_eq!(reply.tool_calls, vec![("add".to_string(), json!({ "a": 1, "b": 2 }))]);
        assert!(reply.had_thinking && reply.text == "done" && !reply.had_logprobs);
    }

    #[test]
    fn the_request_pins_the_seed_and_asks_for_logprobs() {
        let ask = Ask { prompt: "hi".into(), tool: None, schema: Some(json!({ "type": "object" })), num_ctx: 4_096, max_output: 50, seed: 9, think: Some(false) };
        let body = request_body("m", &ask);

        assert_eq!(body["think"], false, "a thinking model must not spend the probe's few output tokens on hidden reasoning");
        assert!(request_body("m", &Ask { think: None, ..ask.clone() }).get("think").is_none());
        assert_eq!(body["options"]["seed"], 9);
        assert_eq!(body["options"]["temperature"], 0);
        assert_eq!(body["options"]["num_ctx"], 4_096);
        assert_eq!(body["format"]["type"], "object");
        assert!(body["logprobs"] == true);
    }
}
