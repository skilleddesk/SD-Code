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
///
/// The fields are owned `String`s because 0.7.2 resolves them from the catalogue rather than from a
/// hand-written table: the provider that listed the model is the provider the request goes to
/// (`protocol/models.json`), and its key comes from the same keychain entry the Provider Hub wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub provider: String,
    /// The keychain entry this endpoint's key comes from (`providers::key_ref(provider)`).
    pub key_ref: String,
    pub url: String,
    /// `anthropic` speaks `messages`, `openai` speaks `chat/completions`.
    pub dialect: &'static str,
}

/// The endpoints this build can fall back to when the catalogue has never heard of the model.
const FALLBACKS: &[(&str, &str, &str, &str)] = &[
    ("anthropic", "sdc.provider.anthropic-api", "https://api.anthropic.com/v1/messages", "anthropic"),
    ("openai", "sdc.provider.openai-api", "https://api.openai.com/v1/chat/completions", "openai"),
    ("custom", "sdc.provider.custom", "http://127.0.0.1:8080/v1/chat/completions", "openai"),
];

/// The chat URL for a catalogue block, derived from the URL it lists models at.
///
/// Every block's `live` is the `…/models` endpoint of the same API (`https://api.deepseek.com/v1/models`),
/// so the chat URL is that URL minus `models` plus `chat/completions` - or `messages` for an
/// `anthropic`-protocol block. `None` when the URL is not of that shape, which is the honest answer:
/// deriving a guess from a non-model URL is how a request ends up at a host nobody chose.
fn chat_url(live: &str, dialect: &str) -> Option<String> {
    let base = live.strip_suffix("/models")?;
    let path = if dialect == "anthropic" { "/messages" } else { "/chat/completions" };

    Some(format!("{base}{path}"))
}

/// The endpoint for a model id, resolved from the catalogue that listed it.
///
/// **This is the bug the user hit.** It used to be a three-row table - `anthropic`, `openai`, `custom` -
/// matched by the model id's prefix. Every other provider the catalogue ships (DeepSeek, Groq,
/// OpenRouter) therefore matched *nothing* and fell through to `custom`, whose key is
/// `sdc.provider.custom`: the chat answered `No API key for custom` for a provider the person had
/// already connected, and the model they picked could never work.
///
/// Two sources are consulted, in this order:
///
///  1. **the provider the app sent with the turn** - a fact, so it decides by itself. This is the one
///     that covers a model the catalogue has never seen, which every provider's live list is full of.
///  2. **the catalogue block whose models contain the id** - for callers that send only a model id.
///
/// `custom` remains the last resort, for an id nothing claims - which is what a hand-typed model id for
/// a hand-configured endpoint is.
pub fn endpoint_for(model: &str, provider: Option<&str>) -> Endpoint {
    let blocks = crate::providers::models::blocked();

    if let Some(id) = provider.filter(|id| !id.trim().is_empty()) {
        if let Some(block) = blocks.iter().find(|block| block.id == id && block.protocol != "ollama") {
            if let Some(endpoint) = endpoint_from(block) {
                return endpoint;
            }
        }
    }

    let head = model.split('/').next().unwrap_or(model);
    let name = api_model(model);

    for block in &blocks {
        /* Ollama is its own engine with its own protocol (`/api/chat`), so a native API turn has no
           business being routed to it. */
        if block.protocol == "ollama" {
            continue;
        }

        let named = |wanted: &str| block.models.iter().any(|entry| entry["id"].as_str() == Some(wanted));
        let is_this_provider = block.id == head || block.id.trim_end_matches("-api") == head;

        if !(named(name) || (is_this_provider && named(model))) {
            continue;
        }

        if let Some(endpoint) = endpoint_from(block) {
            return endpoint;
        }
    }

    let row = FALLBACKS
        .iter()
        .find(|(provider, _key, _url, _dialect)| *provider == head)
        .copied()
        .unwrap_or(FALLBACKS[2]);

    Endpoint {
        provider: row.0.to_string(),
        key_ref: row.1.to_string(),
        url: row.2.to_string(),
        dialect: row.3,
    }
}

/// A catalogue block as an endpoint: its key entry, its dialect, and the chat URL its live URL implies.
/// `None` when the block has no derivable chat URL, so the caller can try the next candidate rather
/// than send a request to a host nobody chose.
fn endpoint_from(block: &crate::providers::models::ProviderBlock) -> Option<Endpoint> {
    let dialect = if block.protocol == "anthropic" { "anthropic" } else { "openai" };

    Some(Endpoint {
        key_ref: crate::providers::key_ref(&block.id),
        provider: block.id.clone(),
        url: chat_url(&block.live, dialect)?,
        dialect,
    })
}

/// The model id a provider's own API expects: the catalogue's id without its provider prefix.
///
/// The catalogue spells a model `deepseek/deepseek-chat` because one list holds every provider, and
/// `api.deepseek.com` knows it as `deepseek-chat`. Only a prefix that *is* a catalogue provider is
/// stripped, because the rest of the id is the provider's own spelling: OpenRouter's
/// `openrouter/anthropic/claude-sonnet-4-5` must reach it as `anthropic/claude-sonnet-4-5`, not as
/// `claude-sonnet-4-5`, which is not a model it knows.
pub fn api_model(model: &str) -> &str {
    let Some((head, rest)) = model.split_once('/') else {
        return model;
    };

    let head_is_a_provider = crate::providers::models::blocked()
        .iter()
        .any(|block| block.id == head || block.id.trim_end_matches("-api") == head);

    if head_is_a_provider {
        rest
    } else {
        model
    }
}


/// The URL, headers and body of one turn. The key is a *parameter*, not a field: this function never
/// touches the keychain, which is what keeps it pure and testable.
pub fn build_request(
    endpoint: &Endpoint,
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
        json!({ "model": api_model(model), "max_tokens": 4096, "stream": true, "messages": messages })
    } else {
        json!({ "model": api_model(model), "stream": true, "messages": messages })
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

    (endpoint.url.clone(), headers, body.to_string())
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
        /* The session's own model, not a guess from the prompt's text: `endpoint_for` used to be
           handed `&prompt.text`, so an API turn went to whichever endpoint the first word of the
           prompt happened to match - and to the loopback one otherwise. */
        let endpoint = endpoint_for(&prompt.model, prompt.provider.as_deref());
        let key = crate::auth::keychain::get(&endpoint.key_ref).unwrap_or_default();

        if key.is_empty() {
            return vec![EngineEvent::Failed(format!(
                "No API key for {}. Connect it in the Provider Hub; the key is stored in the OS keychain.",
                endpoint.provider
            ))];
        }

        let (url, headers, body) = build_request(&endpoint, &key, &prompt.model, &prompt);

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
        Prompt {
            session_id: "s1".into(),
            turn_id: "t1".into(),
            text: text.into(),
            /* The registry's spelling of the model the session is set to. */
            model: "anthropic/claude-sonnet-4-5".into(),
            provider: None,
            history,
        }
    }

    /// The model that reaches the provider is its own id, not the registry's prefixed one.
    #[test]
    fn the_registry_prefix_does_not_travel_to_the_provider() {
        assert_eq!(api_model("anthropic/claude-sonnet-4-5"), "claude-sonnet-4-5");
        assert_eq!(api_model("openai/gpt-5"), "gpt-5");
        assert_eq!(api_model("sonnet"), "sonnet");

        let (_url, _headers, body) = build_request(
            &endpoint_for("anthropic/claude-sonnet-4-5", None),
            "sk-ant-1",
            "anthropic/claude-sonnet-4-5",
            &prompt("hi", vec![]),
        );

        assert!(body.contains(r#""model":"claude-sonnet-4-5""#), "{body}");
    }

    /// The endpoint follows the *session's* model. It used to follow the prompt's text, so this
    /// assertion is the regression: a chat on an API model reached the loopback endpoint unless the
    /// user happened to type the provider's name first.
    #[test]
    fn the_endpoint_follows_the_model_not_the_prompt() {
        assert_eq!(endpoint_for("anthropic/claude-sonnet-4-5", None).provider, "anthropic-api");
        assert_eq!(endpoint_for("openai/gpt-5", None).provider, "openai-api");
        assert_eq!(endpoint_for("llama3.2:3b", None).provider, "custom");
    }

    /// The bug the user hit: `deepseek-v4-pro` came from DeepSeek's own live list, so no block in this
    /// build's catalogue mentions it. Without the provider it reaches the loopback endpoint and asks for
    /// a key under `sdc.provider.custom`, while the DeepSeek key sits under `sdc.provider.deepseek`.
    #[test]
    fn the_provider_the_app_sent_decides_even_for_a_model_the_catalogue_never_saw() {
        let chosen = endpoint_for("deepseek-v4-pro", Some("deepseek"));

        assert_eq!(chosen.provider, "deepseek");
        assert_eq!(chosen.key_ref, "sdc.provider.deepseek");
        assert_eq!(chosen.url, "https://api.deepseek.com/v1/chat/completions");

        /* And without it the same id is unresolvable, which is exactly why the provider has to travel
           with the turn. */
        assert_eq!(endpoint_for("deepseek-v4-pro", None).provider, "custom");
    }

    /// A catalogue id still resolves on its own, for a caller that sends only a model.
    #[test]
    fn a_catalogue_id_resolves_to_the_provider_that_listed_it() {
        let deepseek = endpoint_for("deepseek-chat", None);

        assert_eq!(deepseek.provider, "deepseek");
        assert_eq!(deepseek.key_ref, "sdc.provider.deepseek");

        let groq = endpoint_for("groq/llama-3.3-70b-versatile", None);

        assert_eq!(groq.provider, "groq");
        assert_eq!(groq.url, "https://api.groq.com/openai/v1/chat/completions");
    }

    /// A prefixed id keeps the rest of its own spelling: OpenRouter's ids carry a vendor prefix, and
    /// sending it `claude-sonnet-4-5` would be a model it does not know.
    #[test]
    fn the_provider_prefix_comes_off_and_nothing_else_does() {
        assert_eq!(api_model("deepseek/deepseek-chat"), "deepseek-chat");
        assert_eq!(api_model("openrouter/anthropic/claude-sonnet-4-5"), "anthropic/claude-sonnet-4-5");
        assert_eq!(api_model("sonnet"), "sonnet");
    }

    /// The catalogue's own spelling decides which provider a request goes to, so an `anthropic` block
    /// still speaks `messages` with the key in `x-api-key`.
    #[test]
    fn anthropic_still_gets_its_own_dialect_from_the_catalogue() {
        let anthropic = endpoint_for("anthropic/claude-sonnet-4-5", None);

        assert_eq!(anthropic.dialect, "anthropic");
        assert_eq!(anthropic.url, "https://api.anthropic.com/v1/messages", "{anthropic:?}");
    }

    #[test]
    fn anthropic_puts_the_key_in_its_own_header() {
        let (url, headers, body) = build_request(
            &endpoint_for("anthropic/x", None),
            "sk-ant-1",
            "sonnet",
            &prompt("hi", vec![]),
        );

        assert!(url.ends_with("/v1/messages"));
        assert!(headers.iter().any(|(name, value)| name == "x-api-key" && value == "sk-ant-1"));
        assert!(body.contains("\"stream\":true"));
    }

    #[test]
    fn openai_uses_a_bearer_token_and_replays_the_history() {
        let (_url, headers, body) = build_request(
            &endpoint_for("openai/gpt-5", None),
            "sk-1",
            "gpt-5",
            &prompt("hi", vec!["earlier".into()]),
        );

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
        assert_eq!(endpoint_for("ollama/llama3", None).provider, "custom");
    }
}
