//! The native API engine - direct HTTPS, no CLI in the middle (master spec section 11.4).
//!
//! Three parts, and the split matters:
//!
//! 1. **`build_request`** turns a turn into an HTTP request: URL, headers (the key comes from the
//!    keychain, never from disk) and JSON body. Pure, so a test can assert it.
//! 2. **`parse_sse`** turns a Server-Sent-Events stream into `EngineEvent`s. Pure, so the VCR
//!    fixtures of spec section 11.6 can hold both dialects to one contract.
//! 3. **`post_stream`** moves the bytes: `https://` through `ureq` (rustls + webpki roots), `http://`
//!    through a socket this file opens itself, which is enough for a local OpenAI-compatible endpoint
//!    (LM Studio, vLLM, llama.cpp). TLS used to be absent, and the adapter said so - "a TLS client is
//!    not linked in this build" - which was honest and also the whole problem: an API key could not
//!    reach `api.anthropic.com` at all. A provider that rejects a key now reports the provider's own
//!    sentence (`invalid x-api-key (401)`) instead of a code with nothing behind it.

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

/// Posts the request and returns the response's body lines.
///
/// Two transports, one function, and the split is deliberate:
///
///   `https://`   the real thing, over `ureq` (rustls with the webpki roots). This is what makes an
///                API key usable at all: `api.anthropic.com` and `api.openai.com` answer `401` with a
///                key they do not recognise and stream tokens with one they do. Until 0.6.1 this
///                function refused every `https://` URL with "a TLS client is not linked in this
///                build" - an honest sentence, and a wall between the user and the feature the
///                sentence was about.
///   `http://`    the loopback path, kept because it is what LM Studio, vLLM and llama.cpp speak, and
///                because it is the one transport a test can exercise without a network.
///
/// Blocking, like the rest of this adapter: the daemon's engine calls run on a multi-threaded
/// runtime, and a turn is one long-lived call either way.
pub fn post_stream(url: &str, headers: &[(String, String)], body: &str) -> Result<Vec<String>, String> {
    if url.starts_with("https://") {
        return post_https(url, headers, body);
    }

    let rest = url.strip_prefix("http://").ok_or_else(|| {
        format!("{url}: only http:// and https:// endpoints are supported")
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

    /* `Connection: close` keeps this simple on purpose: the loopback path is for a local server that
       always answers in one body. The https path gets chunked decoding from the client. */
    Ok(response.split("\r\n\r\n").nth(1).unwrap_or("").lines().map(str::to_string).collect())
}

/// The HTTP agent every request goes through, and the two numbers that matter.
///
/// `timeout_connect` is the same eight seconds `ssh` gets in `probe_ssh`: a black-holed address costs
/// seconds rather than a stuck turn. `timeout_read` is per read, not for the whole body, so a turn
/// that streams tokens for two minutes is fine while a socket that has gone quiet is not.
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(8))
        .timeout_read(std::time::Duration::from_secs(120))
        .build()
}

/// The `https://` path: `ureq` does the TLS, the chunked decoding and the redirects.
fn post_https(url: &str, headers: &[(String, String)], body: &str) -> Result<Vec<String>, String> {
    let mut request = agent().post(url);

    for (name, value) in headers {
        request = request.set(name, value);
    }

    match request.send_string(body) {
        Ok(response) => read_lines(response.into_reader()),

        /*
         * A key the provider rejected is the common failure by far, and the reason for it is in the
         * *body*: `{"error":{"message":"invalid x-api-key"}}`. `ureq` hands the response over rather
         * than throwing the sentence away, so the user reads the provider's own words - "invalid
         * x-api-key (401)" - in the transcript instead of a status code with nothing behind it.
         */
        Err(ureq::Error::Status(status, response)) => Err(rejection(status, &read_all(response.into_reader()))),

        Err(ureq::Error::Transport(transport)) => Err(format!("{url}: {transport}")),
    }
}

/// A `GET` for the provider checks: the status code and the body of a URL that answers JSON.
///
/// Both dialects' "is this key still good" endpoint is a model list, and a rejected key answers with
/// the provider's own sentence, so the status and the body both matter here.
pub fn get_json(url: &str, headers: &[(String, String)]) -> Result<(u16, String), String> {
    let mut request = agent().get(url);

    for (name, value) in headers {
        request = request.set(name, value);
    }

    match request.call() {
        Ok(response) => {
            let status = response.status();

            Ok((status, read_all(response.into_reader())))
        }
        /* A 4xx is not a transport failure: it *is* the answer, and the body explains it. */
        Err(ureq::Error::Status(status, response)) => Ok((status, read_all(response.into_reader()))),
        Err(ureq::Error::Transport(transport)) => Err(transport.to_string()),
    }
}

/// The sentence a provider rejected a request with, from its own error body when it has one.
pub fn rejection(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value.pointer("/error/message").and_then(Value::as_str).map(str::to_string));

    match message {
        Some(message) => format!("{message} ({status})"),
        None => format!("HTTP {status}: {}", body.trim()),
    }
}

/// Every line the reader has, for a body that is read once (an error, not a stream).
fn read_all(mut reader: impl Read) -> String {
    let mut text = String::new();

    let _ = reader.read_to_string(&mut text);

    text
}

/// The response body, line by line - the shape `parse_sse` takes.
fn read_lines(reader: impl Read) -> Result<Vec<String>, String> {
    use std::io::BufRead;

    let mut lines = Vec::new();

    for line in std::io::BufReader::new(reader).lines() {
        lines.push(line.map_err(|error| error.to_string())?);
    }

    Ok(lines)
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
    fn https_reaches_a_socket_instead_of_refusing() {
        /*
         * 127.0.0.1:1 is the reserved `tcpmux` port - nothing listens there, so the connection is
         * refused at once and without DNS. That is all this test needs: the failure has to be a
         * *socket* error, which proves the TLS client is linked and a real https request was
         * attempted, and not the old "a TLS client is not linked in this build".
         */
        let error = post_stream("https://127.0.0.1:1/v1/messages", &[], "{}").unwrap_err();

        assert!(!error.contains("not linked"), "{error}");
        assert!(error.contains("127.0.0.1:1"), "{error}");
    }

    #[test]
    fn a_rejected_key_reports_the_providers_own_sentence() {
        /* Anthropic and OpenAI both put the reason at `error.message`; both shapes are checked here
           because the two dialects are the two ways a key gets rejected in practice. */
        assert_eq!(
            rejection(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
            "invalid x-api-key (401)"
        );

        assert_eq!(
            rejection(401, r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#),
            "Incorrect API key provided (401)"
        );

        /* A body that is not the shape we expect is still reported rather than replaced with nothing. */
        assert_eq!(rejection(502, "  <html>bad gateway</html> "), "HTTP 502: <html>bad gateway</html>");
    }

    #[test]
    fn the_unknown_provider_falls_back_to_the_custom_endpoint() {
        assert_eq!(endpoint_for("ollama/llama3").provider, "custom");
    }
}
