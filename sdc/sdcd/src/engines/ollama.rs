//! Ollama - the Local engine (master spec section 11.5).
//!
//! `127.0.0.1:11434` speaks plain HTTP, so this adapter needs no TLS client and is fully functional
//! today (unlike `native_api`'s remote endpoints):
//!
//! * `GET /api/tags` - the installed models, what the Local flow's second doctor row shows.
//! * `POST /api/chat` - streaming NDJSON, one `{"message":{"content":"…"}}` object per line, **pushed
//!   as each line arrives** (`engines::body` hides the chunked framing the Go server uses).
//!
//! A daemon that is not running is reported as such: `daemon_running()` is what the Provider Hub's
//! Local tab asks, and "not running · start it with `ollama serve`" is a better answer than a
//! timeout.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

use async_trait::async_trait;
use serde_json::Value;

use crate::engines::body::{body_lines, read_head};
use crate::engines::{Engine, EngineEvent, EngineStatus, EventSink, Prompt};

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

/// The sentence the Local flow shows when nothing answers on the port.
///
/// It used to be printed for *every* failure, which was right about the common case (no daemon) and
/// wrong about the rest (a port that answered with a 500). It now belongs to the failure it describes.
fn not_running() -> String {
    format!("Ollama is not running. Start it with `ollama serve` (expected at http://{ENDPOINT}).")
}

/// `POST /api/chat`, with the NDJSON body pushed line by line as it arrives.
///
/// This is the local engine, and its tokens are the fastest the app ever draws - which is exactly why
/// collecting the whole body before parsing it was the wrong shape: a fast model arrived in one piece.
fn chat(body: &str, sink: &EventSink) {
    let address: SocketAddr = match ENDPOINT.parse() {
        Ok(address) => address,
        Err(error) => {
            sink.send(EngineEvent::Failed(format!("{ENDPOINT}: {error}")));

            return;
        }
    };

    let mut socket = match TcpStream::connect_timeout(&address, std::time::Duration::from_secs(2)) {
        Ok(socket) => socket,
        Err(_) => {
            sink.send(EngineEvent::Failed(not_running()));

            return;
        }
    };

    /* Per read, not for the whole answer: a model that streams for two minutes is fine, a server that
    has gone quiet is not. The same 60 seconds `call` uses. */
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(60)))
        .ok();

    let request = format!(
        "POST /api/chat HTTP/1.1\r\nHost: {ENDPOINT}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    if let Err(error) = socket.write_all(request.as_bytes()) {
        sink.send(EngineEvent::Failed(format!("{ENDPOINT}: {error}")));

        return;
    }

    let mut reader = std::io::BufReader::new(socket);
    let head = match read_head(&mut reader) {
        Ok(head) => head,
        Err(error) => {
            sink.send(EngineEvent::Failed(format!("{ENDPOINT}: {error}")));

            return;
        }
    };

    if head.status != 0 && !(200..300).contains(&head.status) {
        sink.send(EngineEvent::Failed(format!(
            "{ENDPOINT} answered HTTP {}",
            head.status
        )));

        return;
    }

    drain_chat(body_lines(reader, head.chunked), sink);
}

/// Reads the NDJSON stream to its end, pushing each object as the line it arrived on.
///
/// Ollama's last line carries `"done": true`. A stream that stops without it is still the end of what
/// the model had to say - so the turn ends here rather than waiting for a timeout - and a stream with
/// nothing in it keeps the sentence the collected version had.
fn drain_chat(mut lines: impl std::io::BufRead, sink: &EventSink) {
    let mut ended = false;
    let mut spoken = false;
    let mut line = String::new();

    loop {
        line.clear();

        match lines.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                spoken = spoken || !parse_chat_line(&line).is_empty();

                push_chat_line(&line, &mut ended, sink);
            }
            Err(_) => break,
        }
    }

    if ended {
        return;
    }

    if spoken {
        sink.send(EngineEvent::Done {
            summary: "Done".to_string(),
            meta: String::new(),
            pass: None,
        });

        return;
    }

    sink.send(EngineEvent::Failed("Ollama answered with nothing usable.".to_string()));
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
                .filter_map(|model| {
                    model
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
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

/// One line of `/api/chat`, on its way to the window: parse it, push what it carried, and latch when
/// it ended the turn.
///
/// The third of the three live-path rules - next to `cli::push_stream_line` and
/// `native_api::push_sse_line` - and the same shape for the same reason: `tests/vcr.rs` holds the live
/// path and the collected parser together over every fixture, so a line cannot mean one thing on
/// arrival and another in a batch.
pub fn push_chat_line(line: &str, ended: &mut bool, sink: &EventSink) {
    if *ended {
        return;
    }

    for event in parse_chat_line(line) {
        *ended = event.is_terminal();
        sink.send(event);

        if *ended {
            break;
        }
    }
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

    async fn start(&self, prompt: Prompt, sink: &EventSink) {
        /* The session's model, not the first line of the transcript: this used to read
           `prompt.history.first()`, so a chat's second turn spoke to a model named after the user's
           first message. */
        let fallback = "llama3.2:3b".to_string();
        let model = if prompt.model.trim().is_empty() {
            fallback
        } else {
            crate::engines::native_api::api_model(&prompt.model).to_string()
        };
        /* The conversation so far, then the new prompt. Only the prompt used to be sent, so every turn
           of an Ollama chat started from nothing - the second question had no first one to refer to. */
        let mut messages: Vec<serde_json::Value> = prompt
            .history
            .iter()
            .map(|message| serde_json::json!({ "role": message.role.as_str(), "content": message.text }))
            .collect();

        messages.push(serde_json::json!({ "role": "user", "content": prompt.text }));

        let body = serde_json::json!({
            "model": model,
            "stream": true,
            "messages": messages,
        })
        .to_string();
        let sink = sink.clone();

        /* The socket read blocks for as long as the model talks, so it runs on the blocking pool -
        and the answer travels out through the sink while this future is still pending. */
        let _ = tokio::task::spawn_blocking(move || chat(&body, &sink)).await;
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
    use crate::engines::Recorder;

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

    /// The live path's rule, over the same lines: every object is pushed as it arrives, in order, and
    /// a stream that closes without `"done": true` still ends the turn.
    #[test]
    fn a_drained_chat_always_ends_the_turn() {
        let recorder = Recorder::new();

        drain_chat(
            std::io::BufReader::new(
                &b"{\"message\":{\"content\":\"Hel\"},\"done\":false}\n{\"message\":{\"content\":\"lo\"},\"done\":true}\n"[..],
            ),
            &recorder.sink(),
        );

        assert!(
            matches!(
                recorder.events().as_slice(),
                [EngineEvent::Delta(first), EngineEvent::Delta(second), EngineEvent::Done { .. }]
                    if first == "Hel" && second == "lo"
            ),
            "{:?}",
            recorder.events()
        );

        /* A stream with nothing usable in it keeps the sentence the collected version had. */
        let empty = Recorder::new();

        drain_chat(std::io::BufReader::new(&b"5b\n"[..]), &empty.sink());

        assert!(
            matches!(empty.events().as_slice(), [EngineEvent::Failed(_)]),
            "{:?}",
            empty.events()
        );
    }
}
