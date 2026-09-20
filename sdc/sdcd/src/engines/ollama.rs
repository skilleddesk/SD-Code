//! Ollama - the Local engine (master spec section 11.5).
//!
//! `127.0.0.1:11434` speaks plain HTTP, so this adapter needs no TLS client and is fully functional
//! today (unlike `native_api`'s remote endpoints):
//!
//! * `GET /api/tags` - the installed models, what the Local flow's second doctor row shows.
//! * `POST /api/chat` - streaming NDJSON, one `{"message":{"content":"…"}}` object per line.
//!
//! A daemon that is not running is reported as such: `daemon_running()` is what the Provider Hub's
//! Local tab asks, and "not running · start it with `ollama serve`" is a better answer than a
//! timeout.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

use async_trait::async_trait;
use serde_json::Value;

use crate::engines::{Engine, EngineEvent, EngineStatus, Prompt};

/// Where the local daemon listens.
pub const ENDPOINT: &str = "127.0.0.1:11434";

/// Plain-HTTP request/response. `GET` when `body` is empty, `POST` otherwise.
fn call(path: &str, body: Option<&str>) -> Result<String, String> {
    let address: SocketAddr = ENDPOINT.parse().map_err(|error| format!("{ENDPOINT}: {error}"))?;
    let mut socket = TcpStream::connect_timeout(&address, std::time::Duration::from_secs(2))
        .map_err(|error| format!("{ENDPOINT}: {error}"))?;

    socket.set_read_timeout(Some(std::time::Duration::from_secs(60))).ok();

    let request = match body {
        Some(body) => format!(
            "POST {path} HTTP/1.1\r\nHost: {ENDPOINT}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
        None => format!("GET {path} HTTP/1.1\r\nHost: {ENDPOINT}\r\nConnection: close\r\n\r\n"),
    };

    socket.write_all(request.as_bytes()).map_err(|error| error.to_string())?;

    let mut response = String::new();

    socket.read_to_string(&mut response).map_err(|error| error.to_string())?;

    Ok(response.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

/// True when the daemon answers `GET /api/tags` - the Local flow's first doctor row.
pub fn daemon_running() -> bool {
    call("/api/tags", None).is_ok()
}

/// The installed models, sorted: `llama3.2:3b`, `mistral:7b`, …
pub fn list_models() -> Vec<String> {
    let Ok(body) = call("/api/tags", None) else {
        return Vec::new();
    };

    let Ok(value) = serde_json::from_str::<Value>(&body) else {
        return Vec::new();
    };

    let mut models: Vec<String> = value
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    models.sort();
    models
}

/// Parses one NDJSON line of `/api/chat` into events. Pure, so the fixtures can hold it.
pub fn parse_chat_line(line: &str) -> Vec<EngineEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
        return Vec::new();
    };

    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return vec![EngineEvent::Failed(error.to_string())];
    }

    let mut events = Vec::new();

    if let Some(content) = value.pointer("/message/content").and_then(Value::as_str) {
        if !content.is_empty() {
            events.push(EngineEvent::Delta(content.to_string()));
        }
    }

    if value.get("done").and_then(Value::as_bool).unwrap_or(false) {
        events.push(EngineEvent::Done {
            summary: "Done".to_string(),
            meta: value
                .get("eval_count")
                .and_then(Value::as_i64)
                .map(|count| format!("{count} tokens"))
                .unwrap_or_default(),
            pass: None,
        });
    }

    events
}

pub struct Ollama;

impl Ollama {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Ollama {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for Ollama {
    fn id(&self) -> &'static str {
        "ollama"
    }

    async fn start(&self, prompt: Prompt) -> Vec<EngineEvent> {
        /* The session's model, not the first line of the transcript: this used to read
           `prompt.history.first()`, so a chat's second turn spoke to a model named after the user's
           first message. */
        let fallback = "llama3.2:3b".to_string();
        let model = if prompt.model.trim().is_empty() {
            fallback
        } else {
            crate::engines::native_api::api_model(&prompt.model).to_string()
        };
        let body = serde_json::json!({
            "model": model,
            "stream": true,
            "messages": [{ "role": "user", "content": prompt.text }],
        })
        .to_string();

        let Ok(response) = call("/api/chat", Some(&body)) else {
            return vec![EngineEvent::Failed(format!(
                "Ollama is not running. Start it with `ollama serve` (expected at http://{ENDPOINT})."
            ))];
        };

        let mut events = Vec::new();

        for line in response.lines() {
            let parsed = parse_chat_line(line);
            let terminal = parsed
                .iter()
                .any(|event| matches!(event, EngineEvent::Done { .. } | EngineEvent::Failed(_)));

            events.extend(parsed);

            if terminal {
                break;
            }
        }

        if events.is_empty() {
            events.push(EngineEvent::Failed("Ollama answered with nothing usable.".to_string()));
        }

        events
    }

    async fn cancel(&self, _turn_id: &str) -> bool {
        /* The request is one connection; a cancel closes it by dropping the future. */
        false
    }

    fn status(&self, _turn_id: &str) -> EngineStatus {
        if daemon_running() {
            EngineStatus::Idle
        } else {
            EngineStatus::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_streamed_answer_and_its_terminal_line() {
        let first = parse_chat_line(r#"{"message":{"content":"Hel"},"done":false}"#);
        let last = parse_chat_line(r#"{"message":{"content":"lo"},"done":true,"eval_count":7}"#);

        assert_eq!(first, vec![EngineEvent::Delta("Hel".into())]);
        assert_eq!(last.len(), 2);
        assert!(matches!(last[1], EngineEvent::Done { .. }));
    }

    #[test]
    fn reports_the_daemons_own_error_line() {
        assert_eq!(
            parse_chat_line(r#"{"error":"model 'x' not found"}"#),
            vec![EngineEvent::Failed("model 'x' not found".into())]
        );
        assert!(parse_chat_line("not json").is_empty());
    }
}
