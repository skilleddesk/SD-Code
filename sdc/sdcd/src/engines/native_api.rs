//! The native API engine - direct HTTPS, no CLI in the middle (master spec section 11.4).
//!
//! Three parts, and the split matters:
//!
//! 1. **`build_request`** turns a turn into an HTTP request: URL, headers (the key comes from the
//!    keychain, never from disk) and JSON body. Pure, so a test can assert it.
//! 2. **`parse_sse`** turns a Server-Sent-Events stream into `EngineEvent`s. Pure, so the VCR
//!    fixtures of spec section 11.6 can hold both dialects to one contract.
//! 3. **`post_stream`** moves the bytes. It speaks plain HTTP, which is enough for a local
//!    OpenAI-compatible endpoint (LM Studio, vLLM, llama.cpp), and it is the transport boundary for
//!    everything else: a remote `https://` endpoint reports that a TLS client is not linked yet
//!    rather than pretending the turn started. Principle P4 applied to a dependency.

use std::io::{Read, Write};
use std::net::TcpStream;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::engines::{Engine, EngineEvent, EngineStatus, Prompt};

/// Which provider a model id belongs to, and where its endpoint is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub provider: &'static str,
    /// The keychain entry this endpoint's key comes from.
    pub key_ref: &'static str,
    pub url: &'static str,
    /// `anthropic` speaks `messages`, `openai` speaks `chat/completions`.
    pub dialect: &'static str,
}

/// The endpoints the adapter knows.
pub const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        provider: "anthropic",
        key_ref: "sdc.provider.anthropic-api",
        url: "https://api.anthropic.com/v1/messages",
        dialect: "anthropic",
    },
    Endpoint {
        provider: "openai",
        key_ref: "sdc.provider.openai-api",
        url: "https://api.openai.com/v1/chat/completions",
        dialect: "openai",
    },
    Endpoint {
        provider: "custom",
        key_ref: "sdc.provider.custom",
        url: "http://127.0.0.1:8080/v1/chat/completions",
        dialect: "openai",
    },
];

/// The endpoint for a model id: the first provider that prefixes it, else the custom loopback one.
pub fn endpoint_for(model: &str) -> Endpoint {
    ENDPOINTS
        .iter()
        .find(|endpoint| model.starts_with(endpoint.provider))
        .copied()
        .unwrap_or(ENDPOINTS[2])
}

/// The URL, headers and body of one turn. The key is a *parameter*, not a field: this function never
/// touches the keychain, which is what keeps it pure and testable.
pub fn build_request(
    endpoint: Endpoint,
    key: &str,
    model: &str,
    prompt: &Prompt,
) -> (String, Vec<(String, String)>, String) {
    let mut messages = Vec::new();

    for message in &prompt.history {
        messages.push(json!({ "role": "user", "content": message }));
    }

    messages.push(json!({ "role": "user", "content": prompt.text }));

    let body = if endpoint.dialect == "anthropic" {
        json!({ "model": model, "max_tokens": 4096, "stream": true, "messages": messages })
    } else {
        json!({ "model": model, "stream": true, "messages": messages })
    };

    let (auth_name, auth_value) = if endpoint.dialect == "anthropic" {
        ("x-api-key", key.to_string())
    } else {
        ("authorization", format!("Bearer {key}"))
    };

    let headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("accept".to_string(), "text/event-stream".to_string()),
        (auth_name.to_string(), auth_value),
    ];

    (endpoint.url.to_string(), headers, body.to_string())
}

/// Parses an SSE stream. Each `data:` line is one JSON object; `data: [DONE]` ends it. A comment or
/// `event:` line is skipped, because both providers send them and neither is an event.
pub fn parse_sse(lines: &[String]) -> Vec<EngineEvent> {
    let mut events = Vec::new();

    for line in lines {
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };

        let payload = payload.trim();

        if payload.is_empty() {
            continue;
        }

        if payload == "[DONE]" {
            events.push(EngineEvent::Done {
                summary: "Done".to_string(),
                meta: String::new(),
                pass: None,
            });
            break;
        }

        let value: Value = match serde_json::from_str(payload) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if let Some(delta) = value.pointer("/delta/text").and_then(Value::as_str) {
            events.push(EngineEvent::Delta(delta.to_string()));
            continue;
        }

        if let Some(choice) = value.pointer("/choices/0/delta/content").and_then(Value::as_str) {
            events.push(EngineEvent::Delta(choice.to_string()));
            continue;
        }

        if let Some(reasoning) = value.pointer("/delta/thinking").and_then(Value::as_str) {
            events.push(EngineEvent::Thinking(reasoning.to_string()));
        }
    }

    events
}

/// Posts the request and returns the response's body lines. `http://` only - see the module doc.
pub fn post_stream(url: &str, headers: &[(String, String)], body: &str) -> Result<Vec<String>, String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        "a TLS client is not linked in this build; only http:// endpoints stream today".to_string()
    })?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let host = authority.split(':').next().unwrap_or(authority);

    let mut socket = TcpStream::connect(authority).map_err(|error| format!("{authority}: {error}"))?;
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );

    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }

    request.push_str("\r\n");
    request.push_str(body);

    socket.write_all(request.as_bytes()).map_err(|error| error.to_string())?;

    let mut response = String::new();

    socket.read_to_string(&mut response).map_err(|error| error.to_string())?;

    /* `Connection: close` keeps this simple on purpose: chunked decoding belongs to the TLS-capable
       client that replaces this function. */
    Ok(response.split("\r\n\r\n").nth(1).unwrap_or("").lines().map(str::to_string).collect())
}

pub struct NativeApi;

impl NativeApi {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativeApi {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for NativeApi {
    fn id(&self) -> &'static str {
        "native_api"
    }

    async fn start(&self, prompt: Prompt) -> Vec<EngineEvent> {
        let endpoint = endpoint_for(&prompt.text);
        let key = crate::auth::keychain::get(endpoint.key_ref).unwrap_or_default();

        if key.is_empty() {
            return vec![EngineEvent::Failed(format!(
                "No API key for {}. Connect it in the Provider Hub; the key is stored in the OS keychain.",
                endpoint.provider
            ))];
        }

        let (url, headers, body) = build_request(endpoint, &key, endpoint.provider, &prompt);

        match post_stream(&url, &headers, &body) {
            Ok(lines) => parse_sse(&lines),
            Err(reason) => vec![EngineEvent::Failed(reason)],
        }
    }

    async fn cancel(&self, _turn_id: &str) -> bool {
        /* One HTTP call, no child to kill: dropping the turn's future is the cancellation, and the
           timeout of spec section 12.9 is what notices. */
        false
    }

    fn status(&self, _turn_id: &str) -> EngineStatus {
        EngineStatus::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str, history: Vec<String>) -> Prompt {
        Prompt { session_id: "s1".into(), turn_id: "t1".into(), text: text.into(), history }
    }

    #[test]
    fn anthropic_puts_the_key_in_its_own_header() {
        let (url, headers, body) =
            build_request(endpoint_for("anthropic/x"), "sk-ant-1", "sonnet", &prompt("hi", vec![]));

        assert!(url.ends_with("/v1/messages"));
        assert!(headers.iter().any(|(name, value)| name == "x-api-key" && value == "sk-ant-1"));
        assert!(body.contains("\"stream\":true"));
    }

    #[test]
    fn openai_uses_a_bearer_token_and_replays_the_history() {
        let (_url, headers, body) =
            build_request(endpoint_for("openai/gpt-5"), "sk-1", "gpt-5", &prompt("hi", vec!["earlier".into()]));

        assert!(headers.iter().any(|(name, value)| name == "authorization" && value == "Bearer sk-1"));
        assert!(body.contains("earlier"));
    }

    #[test]
    fn parses_both_dialects_and_stops_at_done() {
        let lines = vec![
            "event: message_start".to_string(),
            r#"data: {"delta":{"text":"hel"}}"#.to_string(),
            r#"data: {"choices":[{"delta":{"content":"lo"}}]}"#.to_string(),
            "data: [DONE]".to_string(),
            r#"data: {"delta":{"text":"after"}}"#.to_string(),
        ];
        let events = parse_sse(&lines);

        assert_eq!(events.len(), 3);
        assert_eq!(events[0], EngineEvent::Delta("hel".into()));
        assert!(matches!(events[2], EngineEvent::Done { .. }));
    }

    #[test]
    fn refuses_https_with_a_plain_reason() {
        assert!(post_stream("https://api.anthropic.com/v1/messages", &[], "{}")
            .unwrap_err()
            .contains("TLS"));
    }

    #[test]
    fn the_unknown_provider_falls_back_to_the_custom_endpoint() {
        assert_eq!(endpoint_for("ollama/llama3").provider, "custom");
    }
}
