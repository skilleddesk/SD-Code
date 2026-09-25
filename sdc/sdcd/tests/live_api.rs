//! Live checks against the real providers, opt-in and off by default.
//!
//! They are `#[ignore]`d because CI has no API keys and a test that needs the public internet is a
//! test that fails for a reason nobody can act on. Run them by hand:
//!
//!   cargo test --test live_api -- --ignored --nocapture
//!
//! What they prove, with a **deliberately invalid key**: the TLS client is linked, the endpoint URL
//! and the auth header are shaped the way the provider expects, and a rejection is reported in the
//! provider's own words. A `401 invalid x-api-key` is the *success* case here - it means the request
//! reached Anthropic and was understood; only the key was wrong. With a real key the same call
//! streams tokens, which is the one thing this test must not do (it would need one).
//!
//! Before 0.6.1 both of these failed with "a TLS client is not linked in this build" and never left
//! the machine.

use sdcd::engines::native_api::{build_request, endpoint_for, post_stream};
use sdcd::engines::{EventSink, Prompt};

fn prompt(text: &str) -> Prompt {
    Prompt {
        session_id: "live".to_string(),
        turn_id: "live".to_string(),
        text: text.to_string(),
        /* The live checks ask a provider directly, so the model is the provider's own id. */
        model: "claude-sonnet-4-5".to_string(),
        provider: None,
        history: Vec::new(),
        project_root: None,
        /* A live API check talks to a provider from this machine; a folder on a host is irrelevant to it. */
        remote: None,
    }
}

/// The sentence for a bogus key at Anthropic's `/v1/messages`.
#[test]
#[ignore = "needs the public internet"]
fn anthropic_answers_a_bogus_key_with_its_own_sentence() {
    let endpoint = endpoint_for("anthropic/claude-sonnet-4", None);
    let (url, headers, body) = build_request(&endpoint, "sk-ant-bogus-key", "claude-sonnet-4", &prompt("hi"));

    let reason = post_stream(&url, &headers, &body, &EventSink::discarding()).expect_err("a bogus key must not be accepted");

    println!("anthropic said: {reason}");

    assert!(reason.contains("401"), "{reason}");
}

/// The same for OpenAI's `/v1/chat/completions`.
#[test]
#[ignore = "needs the public internet"]
fn openai_answers_a_bogus_key_with_its_own_sentence() {
    let endpoint = endpoint_for("openai/gpt-5", None);
    let (url, headers, body) = build_request(&endpoint, "sk-bogus-key", "gpt-5", &prompt("hi"));

    let reason = post_stream(&url, &headers, &body, &EventSink::discarding()).expect_err("a bogus key must not be accepted");

    println!("openai said: {reason}");

    assert!(reason.contains("401"), "{reason}");
}
