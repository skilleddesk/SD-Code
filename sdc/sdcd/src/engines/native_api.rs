//! The native API engine - direct HTTPS, no CLI in the middle (master spec section 11.4).
//!
//! Three parts, and the split matters:
//!
//! 1. **`build_request`** turns a turn into an HTTP request: URL, headers (the key comes from the
//!    keychain, never from disk) and JSON body. Pure, so a test can assert it.
//! 2. **`parse_sse`** turns a Server-Sent-Events stream into `EngineEvent`s - one line at a time
//!    (`parse_sse_line`), so the live path and the VCR fixtures of spec section 11.6 go through the
//!    same rule. Pure, so the fixtures can hold both dialects to one contract.
//! 3. **`post_stream`** moves the bytes and pushes what they carried **as they arrive**:
//!    `https://` through `ureq` (rustls + webpki roots), `http://` through a socket this file opens
//!    itself, which is enough for a local OpenAI-compatible endpoint (LM Studio, vLLM, llama.cpp) and
//!    is the path the streaming test in `tests/streaming.rs` measures. TLS used to be absent, and the
//!    adapter said so - "a TLS client is not linked in this build" - which was honest and also the
//!    whole problem: an API key could not reach `api.anthropic.com` at all. A provider that rejects a
//!    key now reports the provider's own sentence (`invalid x-api-key (401)`) instead of a code with
//!    nothing behind it.
//!
//! Until 0.7.4 this adapter read the whole body and *then* parsed it, so a DeepSeek or Anthropic answer
//! reached the window in one piece the moment the stream ended (*"akbare answare disse"*). The
//! provider was streaming perfectly well; this file was collecting it.

use std::io::{Read, Write};
use std::net::TcpStream;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::engines::body::{body_lines, read_head};
use crate::engines::{Engine, EngineEvent, EngineStatus, EventSink, Prompt};

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

        let named = |wanted: &str| {
            block
                .models
                .iter()
                .any(|entry| entry["id"].as_str() == Some(wanted))
        };
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
///
/// This is the *collected* form, kept for a fixture or a replay of a finished stream. The live path
/// pushes each line as it arrives (`push_sse_line`), and the two share `parse_sse_line`, which is
/// what keeps "what a line means" in one place.
pub fn parse_sse(lines: &[String]) -> Vec<EngineEvent> {
    let mut events = Vec::new();

    for line in lines {
        let parsed = parse_sse_line(line);
        let terminal = parsed.iter().any(EngineEvent::is_terminal);

        events.extend(parsed);

        if terminal {
            break;
        }
    }

    events
}

/// One `data:` line, as events. Pure: no socket, no state, which is what lets a fixture hold the
/// contract for both dialects.
///
/// Four things a provider puts in a stream, and each one has a reason to be here:
///
/// * **text**, at `/delta/text` (Anthropic's `messages`) or `/choices/0/delta/content` (every
///   `chat/completions` provider - OpenAI, DeepSeek, Groq, OpenRouter, LM Studio);
/// * **reasoning**, which is the *thinking* block of spec section 7.5. DeepSeek's reasoner streams it
///   at `/choices/0/delta/reasoning_content`, Anthropic at `/delta/thinking` (`thinking_delta`), and
///   some OpenAI-compatible servers at `/choices/0/delta/reasoning` - all three are read, because a
///   provider whose reasoning is dropped looks like a model that does not think;
/// * **`[DONE]`**, the end of the turn for every dialect;
/// * **an error**, which both providers can send mid-stream (`{"type":"error","error":{…}}`). It used
///   to be skipped, so a turn that the provider aborted ended as a silence rather than as its own
///   sentence.
pub fn parse_sse_line(line: &str) -> Vec<EngineEvent> {
    let Some(payload) = line.strip_prefix("data:") else {
        return Vec::new();
    };

    let payload = payload.trim();

    if payload.is_empty() {
        return Vec::new();
    }

    if payload == "[DONE]" {
        return vec![EngineEvent::Done {
            summary: "Done".to_string(),
            meta: String::new(),
            pass: None,
        }];
    }

    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return Vec::new();
    };

    if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
        return vec![EngineEvent::Failed(message.to_string())];
    }

    if let Some(delta) = value.pointer("/delta/text").and_then(Value::as_str) {
        return vec![EngineEvent::Delta(delta.to_string())];
    }

    if let Some(choice) = value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
    {
        return vec![EngineEvent::Delta(choice.to_string())];
    }

    for pointer in [
        "/delta/thinking",
        "/choices/0/delta/reasoning_content",
        "/delta/reasoning_content",
        "/choices/0/delta/reasoning",
    ] {
        if let Some(reasoning) = value.pointer(pointer).and_then(Value::as_str) {
            return vec![EngineEvent::Thinking(reasoning.to_string())];
        }
    }

    Vec::new()
}

/// One line of a streamed body, on its way to the window: parse it, push what it carried, and latch
/// when it ended the turn.
///
/// The SSE counterpart of `cli::push_stream_line`, and a function for the same reason: the live path
/// and `parse_sse` have to agree, and `tests/vcr.rs` holds them to it over every fixture.
pub fn push_sse_line(line: &str, ended: &mut bool, sink: &EventSink) {
    if *ended {
        return;
    }

    for event in parse_sse_line(line) {
        *ended = event.is_terminal();
        sink.send(event);

        if *ended {
            break;
        }
    }
}

/// Posts the request and **pushes the events of the response as the bytes arrive**.
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
///                because it is the one transport a test can exercise without a network
///                (`tests/streaming.rs` measures exactly this call with a chunked answer).
///
/// Blocking, like the rest of this adapter: `NativeApi::start` hands it to `spawn_blocking`, so the
/// waiting happens on the blocking pool rather than on a runtime worker - and the *streaming* comes
/// from the sink, not from the return value.
///
/// An `Err` here is a turn that never started (a refused socket, a rejected key, a URL nobody speaks).
/// A failure that arrives *inside* the stream goes to the sink like any other event.
pub fn post_stream(
    url: &str,
    headers: &[(String, String)],
    body: &str,
    sink: &EventSink,
) -> Result<(), String> {
    if url.starts_with("https://") {
        return post_https(url, headers, body, sink);
    }

    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("{url}: only http:// and https:// endpoints are supported"))?;
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

    /* The head is read the moment it arrives, and then the body line by line - which is the whole
    difference between this and `read_to_string`: a local server that streams (`"stream": true` is
    what `build_request` asks for) is drawn as it writes instead of after it closes. The framing is
    handled here rather than by a client, because this path *is* the client. */
    let mut reader = std::io::BufReader::new(socket);
    let head = read_head(&mut reader).map_err(|error| error.to_string())?;

    if head.status != 0 && !(200..300).contains(&head.status) {
        /* A failure is one JSON sentence, not a stream: it is read whole so the provider's own words
        can be handed back. */
        return Err(rejection(head.status, &read_all(reader)));
    }

    drain_sse(body_lines(reader, head.chunked), sink);

    Ok(())
}

/// Reads an SSE body to its end, pushing every event as the line it came on arrives.
///
/// A body that ends without `[DONE]` is still a finished answer: the OpenAI-compatible providers close
/// the stream instead of writing the sentinel, and the alternative to saying `Done` here is a turn
/// that stays `running` in the app for ever. A body that ends without a word in it is a failure with a
/// sentence, because silence would look like an empty answer (principle P4).
fn drain_sse(mut lines: impl std::io::BufRead, sink: &EventSink) {
    let mut ended = false;
    let mut spoken = false;
    let mut line = String::new();

    loop {
        line.clear();

        match lines.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                spoken = spoken || !parse_sse_line(&line).is_empty();

                push_sse_line(&line, &mut ended, sink);
            }
            /* A read error mid-stream is the connection dying: the answer stops here, and what
               arrived so far stays on screen. */
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

    sink.send(EngineEvent::Failed("the provider's stream ended without a single event".to_string()));
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

/// The `https://` path: `ureq` does the TLS, the chunked decoding and the redirects, and its body
/// reader is a stream - `into_reader` hands the bytes over as they arrive, which is why this path
/// needs neither `read_head` nor `BodyReader` (the framing is already decoded).
fn post_https(
    url: &str,
    headers: &[(String, String)],
    body: &str,
    sink: &EventSink,
) -> Result<(), String> {
    let mut request = agent().post(url);

    for (name, value) in headers {
        request = request.set(name, value);
    }

    match request.send_string(body) {
        Ok(response) => {
            drain_sse(body_lines(response.into_reader(), false), sink);

            Ok(())
        }

        /*
         * A key the provider rejected is the common failure by far, and the reason for it is in the
         * *body*: `{"error":{"message":"invalid x-api-key"}}`. `ureq` hands the response over rather
         * than throwing the sentence away, so the user reads the provider's own words - "invalid
         * x-api-key (401)" - in the transcript instead of a status code with nothing behind it.
         */
        Err(ureq::Error::Status(status, response)) => {
            Err(rejection(status, &read_all(response.into_reader())))
        }

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
        Err(ureq::Error::Status(status, response)) => {
            Ok((status, read_all(response.into_reader())))
        }
        Err(ureq::Error::Transport(transport)) => Err(transport.to_string()),
    }
}

/// The sentence a provider rejected a request with, from its own error body when it has one.
pub fn rejection(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
    });

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

    async fn start(&self, prompt: Prompt, sink: &EventSink) {
        /* The session's own model, not a guess from the prompt's text: `endpoint_for` used to be
           handed `&prompt.text`, so an API turn went to whichever endpoint the first word of the
           prompt happened to match - and to the loopback one otherwise. */
        let endpoint = endpoint_for(&prompt.model, prompt.provider.as_deref());
        let key = crate::auth::keychain::get(&endpoint.key_ref).unwrap_or_default();

        if key.is_empty() {
            sink.send(EngineEvent::Failed(format!(
                "No API key for {}. Connect it in the Provider Hub; the key is stored in the OS keychain.",
                endpoint.provider
            )));

            return;
        }

        let (url, headers, body) = build_request(&endpoint, &key, &prompt.model, &prompt);
        let sink = sink.clone();

        /* The request blocks (TLS handshake, then a socket read per token), so it runs on the
        blocking pool rather than on a runtime worker - and the streaming has to come through the
        sink, because `spawn_blocking` can only be joined once the whole body has been read. That is
        the shape of a live turn: the future stays pending while the events land one by one. */
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(reason) = post_stream(&url, &headers, &body, &sink) {
                sink.send(EngineEvent::Failed(reason));
            }
        })
        .await;
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
    use crate::engines::Recorder;

    fn prompt(text: &str, history: Vec<String>) -> Prompt {
        Prompt {
            session_id: "s1".into(),
            turn_id: "t1".into(),
            text: text.into(),
            /* The registry's spelling of the model the session is set to. */
            model: "anthropic/claude-sonnet-4-5".into(),
            provider: None,
            history,
            /* A provider answers over the network, so this adapter has no working directory - the field is
               on the `Prompt` because it is a fact about the session, and only `cli.rs` uses it. The same
               goes for `remote`: `native_api` contacts an endpoint from *this* daemon, so a chat whose
               folder is on a host is still answered from here (0.7.13). */
            project_root: None,
            remote: None,
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
        let error = post_stream(
            "https://127.0.0.1:1/v1/messages",
            &[],
            "{}",
            &EventSink::discarding(),
        )
        .unwrap_err();

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

    /// The reasoning channel, which is the *thinking* block of spec section 7.5.
    ///
    /// DeepSeek's reasoner streams it at `reasoning_content`, Anthropic at `thinking`, and some
    /// OpenAI-compatible servers at `reasoning`. All three are the same fact, and a provider whose
    /// reasoning is dropped looks like a model that does not think - which is what the report
    /// *"claude code/cline e jemon thinking ki korse sob kisu live dakha jai"* asks for.
    #[test]
    fn reasoning_arrives_as_thinking_in_every_shape_the_providers_use() {
        for name in ["reasoning_content", "reasoning"] {
            let line = format!(r#"data: {{"choices":[{{"delta":{{"{name}":"weighing it"}}}}]}}"#);

            assert_eq!(
                parse_sse_line(&line),
                vec![EngineEvent::Thinking("weighing it".into())],
                "{name}"
            );
        }

        assert_eq!(
            parse_sse_line(r#"data: {"delta":{"thinking":"why"}}"#),
            vec![EngineEvent::Thinking("why".into())]
        );

        /* And text stays text: the two channels are not confused with each other. */
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#),
            vec![EngineEvent::Delta("Hi".into())]
        );
    }

    /// A provider that aborts mid-stream says so *in* the stream. It used to be skipped, so the turn
    /// ended with nothing on screen rather than with the provider's own sentence.
    #[test]
    fn an_error_inside_the_stream_is_a_failure_with_its_own_sentence() {
        assert_eq!(
            parse_sse_line(
                r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
            ),
            vec![EngineEvent::Failed("Overloaded".into())]
        );
    }

    /// A body that closes without `[DONE]` is a finished answer, not a turn that hangs; a body with
    /// nothing usable in it is a failure with a sentence.
    ///
    /// This is the rule the live path needed once it stopped collecting: `parse_sse` was only ever
    /// asked about a body that had already ended, so nothing had to decide what "the socket closed"
    /// means.
    #[test]
    fn a_drained_body_always_ends_the_turn() {
        let closed = Recorder::new();

        drain_sse(
            std::io::BufReader::new(&b"data: {\"delta\":{\"text\":\"hi\"}}\n"[..]),
            &closed.sink(),
        );

        assert!(
            matches!(
                closed.events().as_slice(),
                [EngineEvent::Delta(text), EngineEvent::Done { .. }] if text == "hi"
            ),
            "{:?}",
            closed.events()
        );

        let empty = Recorder::new();

        drain_sse(std::io::BufReader::new(&b""[..]), &empty.sink());

        assert!(
            matches!(empty.events().as_slice(), [EngineEvent::Failed(_)]),
            "{:?}",
            empty.events()
        );
    }
}
