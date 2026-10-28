//! The provider backend - the six flows of spec section 9.10, server side.
//!
//! The Provider Hub in the app is a view over this module. Each flow ends in a `ProviderStatus` event
//! (so the card, the topbar dot and the status bar all move together) and answers a method result:
//!
//! | flow          | methods                             | what it writes          |
//! | ------------- | ----------------------------------- | ----------------------- |
//! | `api_key`     | `provider.test`, `provider.save`    | keychain entry + row    |
//! | `subscription`| `cli.login` (this daemon drives the CLI), `provider.oauth.*` | the CLI's own store |
//! | `local`       | `provider.local.doctor`             | nothing (a read)        |
//! | `custom`      | `provider.save` with a url          | row with url + protocol |
//! | `registry`    | `models.list`, `models.select`, `provider.registry.*` | the model cache + a setting |
//! | `doctor`      | `host.doctor`                       | nothing (a read)        |
//!
//! The subscription flow is honest about who owns the credential: the **CLI** does. `provider.oauth.*`
//! reports that a token exchange is not wired, and the real path - `auth::cli_login` - drives the CLI's
//! own login and hands back only the code the user pasted. Nothing here ever sees a token.

pub mod catalog;
pub mod clis;
pub mod local;
pub mod runtimes;
pub mod models;

use std::sync::Arc;

use serde_json::{json, Value};

use crate::auth::keychain;
use crate::sdcp::envelope::ErrorObject;
use crate::store::sqlite::Store;

/// The provider cards the app ships with: `(id, name, kind, logo, initial, detail)`.
///
/// 0.9.0 widened this list to the original model makers ("sokol AI provider der add korte cassi jara
/// … orginal AI model provider"): Google, xAI, Moonshot, Mistral, Qwen and Z.ai, all API-key cards
/// whose model lists refresh from the provider's own endpoint - a new model shows up here the day the
/// provider lists it, with no build. Each one also has a block in `protocol/models.json`, which is
/// what routes its chat traffic (`native_api::endpoint_for`).
pub const CATALOG: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("claude", "Claude", "subscription", "claude", "C", "Claude Pro / Max subscription · uses your own login"),
    ("openai", "OpenAI", "subscription", "openai", "O", "ChatGPT Plus · Codex CLI subscription"),
    ("gemini", "Gemini", "subscription", "gemini", "G", "Google AI · Gemini CLI"),
    ("anthropic-api", "Anthropic API", "api-key", "claude", "A", "Direct API key · pay per token"),
    /* No spend figure here. This row used to read `$12.40 / $50.00 this month`, which was invented
       twice over: this build has no way to see a provider's billing, and it had never contacted the
       provider at all. A number like that is the kind of thing a person makes a decision on. */
    ("openai-api", "OpenAI API", "api-key", "openai", "O", "Direct API key · pay per token"),
    ("deepseek", "DeepSeek", "api-key", "deepseek", "D", "Direct API · cheap, fast"),
    ("groq", "Groq", "api-key", "groq", "G", "Ultra-fast inference"),
    ("google", "Google Gemini API", "api-key", "gemini", "G", "Direct API key · pay per token"),
    ("xai", "xAI Grok", "api-key", "xai", "X", "Direct API key · pay per token"),
    ("moonshot", "Moonshot Kimi", "api-key", "moonshot", "K", "Direct API key · pay per token"),
    ("mistral", "Mistral", "api-key", "mistral", "M", "Direct API key · pay per token"),
    ("qwen", "Alibaba Cloud", "api-key", "qwen", "A", "Model Studio · Qwen, DeepSeek, Kimi, GLM · list updates itself"),
    /* 0.14.4: a Coding Plan key is not a Model Studio key. It answers only at `coding-intl`, lists its own
       ten models, and Model Studio refuses it (401) - so one saved on the card above said Connected and
       then showed nothing but the bundle ("deepseek soho onk model bad diyaso"). It has its own card. */
    ("qwen-coding", "Alibaba Coding Plan", "api-key", "qwen", "A", "Coding Plan subscription · Qwen Coder, Kimi, GLM, MiniMax"),
    ("zai", "Z.ai GLM", "api-key", "zai", "Z", "Direct API key · pay per token"),
    ("openrouter", "OpenRouter", "api-key", "openrouter", "O", "One key · 200+ models"),
    ("ollama", "Ollama", "local", "ollama", "O", "Local models · auto-detected on this machine"),
];

/// The twelve models of the registry: `(id, provider, tier, ctx, cost)`.
pub const MODELS: &[(&str, &str, &str, i64, &str)] = &[
    ("anthropic/claude-sonnet-5", "anthropic-api", "balanced", 1_000_000, "$2 / $10"),
    ("anthropic/claude-opus-5-5", "anthropic-api", "deep", 1_000_000, "$4 / $20"),
    ("anthropic/claude-haiku-4-5", "anthropic-api", "fast", 200_000, "$1 / $5"),
    ("openai/gpt-5", "openai-api", "deep", 400_000, "$10 / $30"),
    ("openai/gpt-5-mini", "openai-api", "fast", 400_000, "$0.25 / $2"),
    ("google/gemini-2.5-pro", "gemini", "deep", 2_000_000, "$1.25 / $5"),
    ("google/gemini-2.5-flash", "gemini", "fast", 1_000_000, "$0.075 / $0.30"),
    ("deepseek/deepseek-chat", "deepseek", "balanced", 64_000, "$0.14 / $0.28"),
    ("groq/llama-3.3-70b", "groq", "balanced", 128_000, "$0.59 / $0.79"),
    ("openrouter/anthropic/claude-sonnet-4-5", "openrouter", "balanced", 200_000, "$3 / $15"),
    ("ollama/deepseek-coder:6.7b", "ollama", "balanced", 16_000, "free"),
    ("ollama/llama3.2:3b", "ollama", "fast", 128_000, "free"),
];

/// The keychain entry a provider's secret lives in.
pub fn key_ref(id: &str) -> String {
    format!("sdc.provider.{id}")
}

/// The registry, honouring a set of disabled ids - `provider.registry.list`'s answer.
pub fn registry(disabled: &[String]) -> Vec<Value> {
    MODELS
        .iter()
        .map(|(id, provider, tier, ctx, cost)| {
            json!({
                "id": id,
                "provider": provider,
                "tier": tier,
                "ctx": ctx,
                "cost": cost,
                "enabled": !disabled.contains(&id.to_string()),
            })
        })
        .collect()
}

/// The model-list endpoint that answers "is this key still good", per provider id.
///
/// A key's *shape* proves nothing about the key, and 0.6.1 linked a TLS client, so `Test connection`
/// can mean it now: the key is used for one read-only call and the answer is the provider's own. A
/// provider that is not listed here is reported as **not contacted** rather than guessed at.
const KEY_CHECK: &[(&str, &str)] = &[
    ("anthropic-api", "https://api.anthropic.com/v1/models"),
    ("openai-api", "https://api.openai.com/v1/models"),
    ("deepseek", "https://api.deepseek.com/models"),
    ("groq", "https://api.groq.com/openai/v1/models"),
    ("openrouter", "https://openrouter.ai/api/v1/models"),
    ("google", "https://generativelanguage.googleapis.com/v1beta/openai/models"),
    ("xai", "https://api.x.ai/v1/models"),
    ("moonshot", "https://api.moonshot.ai/v1/models"),
    ("mistral", "https://api.mistral.ai/v1/models"),
    ("qwen", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models"),
    ("qwen-coding", "https://coding-intl.dashscope.aliyuncs.com/v1/models"),
    ("zai", "https://api.z.ai/api/paas/v4/models"),
];

/// The request that checks a key: `(url, headers)`. Pure, so a test can hold the shape.
///
/// Anthropic wants its key in `x-api-key` and a version header; every other provider in the table is
/// OpenAI-compatible, which means a bearer token.
fn key_check(id: &str, key: &str) -> Option<(String, Vec<(String, String)>)> {
    let overrides = crate::providers::models::endpoint_overrides();

    key_check_at(id, key, overrides.get(id).map(String::as_str))
}

/// [`key_check`] with the base URL the person set, if any. 0.12.2 checked a Qwen key against the built-in
/// `dashscope-intl` host even with a workspace URL saved, and a workspace key is refused there (401) - so
/// the Test button said "Incorrect API key" about a key the workspace accepts.
fn key_check_at(id: &str, key: &str, base: Option<&str>) -> Option<(String, Vec<(String, String)>)> {
    let (_, built_in) = KEY_CHECK.iter().find(|(provider, _)| *provider == id)?;
    let url = base.map(|base| format!("{}/models", base.trim_end_matches('/'))).unwrap_or_else(|| built_in.to_string());

    let headers = if id == "anthropic-api" {
        vec![
            ("x-api-key".to_string(), key.to_string()),
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
        ]
    } else {
        vec![("authorization".to_string(), format!("Bearer {key}"))]
    };

    Some((url.to_string(), headers))
}

/// The provider's own sentence, and - where it does not say what to do - what to do. Alibaba answers
/// "Incorrect API key" whether the key is wrong or was made in another region or workspace; measured in
/// 0.12.3 against all three public hosts and a workspace host.
fn with_hint(id: &str, status: u16, error: String) -> String {
    if id == "qwen-coding" && status == 401 {
        return format!(
            "{error} · A Coding Plan key is made in Model Studio → Coding Plan. A pay-as-you-go Model Studio key belongs on the Alibaba Cloud card."
        );
    }

    if id == "qwen" && status == 401 {
        return format!(
            "{error} · Alibaba ties a key to one region and workspace: create it in Model Studio → API Keys of the workspace whose Base URL is set here, and paste it again."
        );
    }

    error
}

/// Asks the provider whether a key is good: `(status, model list body)`.
///
/// The model list is the question for every provider but one. Measured 2026-09-29: the Alibaba Coding
/// Plan's `/models` answers **200 to any key at all** (`sk-bogus-0000` included), so a check there proved
/// nothing. Its chat route does judge the key, and an empty request is judged without a token spent: 401
/// for a key it refuses, 400 (the empty request) for one it accepts - then the list is read for the count.
fn judge(id: &str, url: &str, headers: &[(String, String)]) -> Result<(u16, String), String> {
    if id == "qwen-coding" {
        let chat = format!("{}/chat/completions", url.trim_end_matches("/models"));
        let (status, body) =
            crate::engines::native_api::post_json(&chat, headers, r#"{"model":"qwen3-coder-plus","messages":[]}"#)?;

        if status != 400 {
            return Ok((status, body));
        }
    }

    crate::engines::native_api::get_json(url, headers)
}

/// A pasted API key without the whitespace a paste brings along. No provider's key contains any.
fn clean_key(key: &str) -> String {
    key.chars().filter(|character| !character.is_whitespace()).collect()
}

/// How many models the provider listed. Both dialects answer `{"data":[ … ]}`.
fn model_count(body: &str) -> usize {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value.get("data").and_then(Value::as_array).map(Vec::len))
        .unwrap_or(0)
}

/// Flow 1's `Test`.
///
/// It is honest about what it did, and 0.6.1 changed *what it can do*. A key is now used for one
/// read-only call to the provider's own model list: a key the provider accepts answers with the
/// models it knows, and a key it rejects answers with its own sentence ("invalid x-api-key (401)"),
/// which is the only useful thing to show a person who is looking at a field they just filled in.
/// The answer still says which of the two happened in `verified`, because a green tick that means
/// "we never asked" is worse than no tick at all.
pub fn test(id: &str, key: Option<&str>) -> Value {
    let catalog = CATALOG.iter().find(|row| row.0 == id);
    let kind = catalog.map(|row| row.2).unwrap_or("api-key");

    if kind == "local" {
        let running = crate::engines::ollama::daemon_running();
        let models = if running { crate::engines::ollama::list_models() } else { Vec::new() };

        return json!({
            "ok": running,
            "verified": true,
            "models": models.len(),
            "detail": if running {
                format!("OK · contacted · {} models installed", models.len())
            } else {
                String::new()
            },
            "error": if running { Value::Null } else if crate::host::program::resolve("ollama").is_some() { json!("Ollama is installed but would not start") } else { json!("Ollama is not installed · Settings → Environment → Ollama → Install") },
        });
    }

    let candidate = key.map(clean_key).or_else(|| keychain::get(&key_ref(id)));

    let Some(secret) = candidate.filter(|secret| !secret.trim().is_empty()) else {
        return json!({
            "ok": false,
            "verified": false,
            "models": 0,
            "detail": "",
            "error": "Enter a key first",
        });
    };

    if let Some((url, headers)) = key_check(id, &secret) {
        return match judge(id, &url, &headers) {
            Ok((status, body)) if (200..300).contains(&status) => {
                let models = model_count(&body);

                json!({
                    "ok": true,
                    "verified": true,
                    "models": models,
                    "detail": format!("OK · key valid · {models} models available"),
                    "account": keychain::mask(&secret),
                })
            }
            Ok((status, body)) => json!({
                "ok": false,
                "verified": true,
                "models": 0,
                "detail": "",
                "error": with_hint(id, status, crate::engines::native_api::rejection(status, &body)),
            }),
            /* Not reachable is a different sentence from rejected: the key was never judged. */
            Err(reason) => json!({
                "ok": false,
                "verified": false,
                "models": 0,
                "detail": "",
                "error": format!("could not reach {url} · {reason}"),
            }),
        };
    }

    json!({
        "ok": true,
        "verified": false,
        "models": MODELS.len(),
        "detail": format!(
            "Key accepted (shape checked) · {} models in the registry · this provider has no check endpoint, so it was not contacted",
            MODELS.len()
        ),
        "account": keychain::mask(&secret),
    })
}

/// The nine provider cards, with whatever the store, the keychain and this machine already know.
///
/// **A status is evidence, or it is `needs-auth`.** The old version of this function ended in
/// `else { "connected" }` for anything that was not an API key - so a fresh install reported Claude,
/// OpenAI and Gemini *connected* on a machine where none of the three CLIs existed, and the Hub drew
/// three green cards for sign-ins nobody had done. That is the same defect the app's seeded demo had,
/// one layer down, and it is worse here: the app can only show a lie the daemon hands it.
///
/// What counts as evidence now:
///
///   * a stored row - the user saved a key, or a login wrote a status (it wins, and it is what an
///     explicit `provider.save` records);
///   * for `api-key`, a secret in the keychain;
///   * for `local`, the Ollama daemon actually answering on this machine;
///   * for `subscription`, the CLI's own program being present on `PATH`. Present is still not
///     *signed in*, so the status stays `needs-auth` and the detail says what would change it - which
///     is also the sentence `cli.recipes` prints and `Connect` acts on.
pub fn list(store: &Arc<Store>) -> Vec<Value> {
    let mut rows: Vec<Value> = CATALOG
        .iter()
        .map(|(id, name, kind, logo, initial, detail)| {
            let stored = store.provider(id).ok().flatten();
            let mut status = settle_status(id, kind, stored.as_ref().and_then(|row| row["status"].as_str()));
            /* 1.0.0: Google refused this machine's Gemini sign-in (`IneligibleTierError`) - the card is not connected,
               whatever the row and the credential file say, until the person signs in again. */
            let refused = *id == "gemini" && gemini_refused(store);

            if refused {
                status = "needs-auth".to_string();
            }
            /* An **empty** stored detail is not a detail (3.2): every subscription row on this machine had one, so the
               cards showed a blank line where the sentence that says what to do should be. */
            let detail_text = stored
                .as_ref()
                .and_then(|row| row["detail"].as_str())
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| honest_detail(id, kind, detail));
            let detail_text = if refused { GEMINI_REFUSED.to_string() } else { detail_text };

            json!({
                "id": id,
                "name": name,
                "kind": kind,
                "status": status,
                "detail": detail_text,
                "account": stored.as_ref().and_then(|row| row["account"].as_str()),
                "logo": logo,
                "initial": initial,
                "url": stored.as_ref().and_then(|row| row["url"].as_str()),
                "protocol": stored.as_ref().and_then(|row| row["protocol"].as_str()),
            })
        })
        .collect();

    /* 0.22: every local model server the person connected (LM Studio, llama.cpp, vLLM, …) is a card of its own. */
    rows.extend(local::provider_rows());
    rows
}

/// Is this turn failure the provider refusing the key itself (not a rate limit, a region or the network)?
pub fn refused_key(reason: &str) -> bool {
    let lowered = reason.to_lowercase();

    lowered.contains("(401)")
        || ["invalid api key", "invalid_api_key", "incorrect api key", "invalid x-api-key", "invalidapikey", "authentication_error"]
            .iter()
            .any(|needle| lowered.contains(needle))
}

/// Writes `needs-auth` on an API-key provider's row after a refused key. Answers whether it did.
pub fn mark_refused(store: &Arc<Store>, id: &str, _reason: &str) -> bool {
    let Some((name, kind)) = CATALOG.iter().find(|row| row.0 == id).map(|row| (row.1, row.2)) else {
        return false;
    };

    if kind != "api-key" {
        return false;
    }

    /* The row's account, URL and protocol stay; only the status changes. */
    let row = store.provider(id).ok().flatten().unwrap_or(Value::Null);
    let text = |field: &str| row[field].as_str().map(str::to_string);

    store
        .upsert_provider(id, name, kind, "needs-auth", text("account").as_deref(), text("url").as_deref(), text("protocol").as_deref(), Some(&key_ref(id)))
        .is_ok()
}

/// What the Gemini card says once Google refused its sign-in (1.0.0).
pub const GEMINI_REFUSED: &str = "Google no longer serves Gemini CLI to personal Google accounts (Google AI Pro included) · use the Google Gemini API card with a free key from aistudio.google.com/apikey - same models, gemini-4-argon too";

/// Whether Google refused this machine's Gemini CLI sign-in, as last seen.
pub fn gemini_refused(store: &Arc<Store>) -> bool {
    store.setting("gemini.refused").ok().flatten().as_deref() == Some("1")
}

/// Records Google's refusal: the Gemini card is `needs-auth` and its models leave the menu. Answers whether it changed.
pub fn mark_gemini_refused(store: &Arc<Store>) -> bool {
    if gemini_refused(store) {
        return false;
    }

    let _ = mark_signed_out(store, "gemini");

    store.set_setting("gemini.refused", "1").is_ok()
}

/// Does Google refuse this Gemini CLI sign-in? Checked once in the background at start when the card says connected:
/// the refusal comes from the account setup, before any model is asked, so it costs no quota.
pub fn probe_gemini(store: &Arc<Store>) {
    let connected = store.provider("gemini").ok().flatten().is_some_and(|row| row["status"].as_str() == Some("connected"));

    if !connected || gemini_has_a_route() || gemini_refused(store) || !crate::host::doctor::has("gemini") {
        return;
    }

    if let Some(output) = prints_within("gemini", &["--skip-trust", "-p", "Reply with OK"], 90) {
        if crate::errors::translator::translate("gemini", &output).rule == "gemini-refused" {
            mark_gemini_refused(store);
        }
    }
}

/// Records that a sign-in **this app drove** finished, so the card survives a restart (3.2).
///
/// Before this, a finished CLI login only pushed a `ProviderStatus` event: nothing was written, so after a restart the
/// only thing that could make a subscription card green again was guessing from a credential file - which is exactly
/// the silent sign-in that had to go. The row *is* the consent, and `cli.logout` is what takes it back.
pub fn mark_signed_in(store: &Arc<Store>, id: &str, detail: &str) -> bool {
    let Some((name, kind)) = CATALOG.iter().find(|row| row.0 == id).map(|row| (row.1, row.2)) else {
        return false;
    };

    if kind != "subscription" {
        return false;
    }

    if id == "gemini" {
        let _ = store.set_setting("gemini.refused", "0");
    }

    store.upsert_provider(id, name, kind, "connected", Some(detail), None, None, None).is_ok()
}

/// Takes the sign-in record away: the card is `needs-auth` again until the person signs in from the app (3.2).
pub fn mark_signed_out(store: &Arc<Store>, id: &str) -> bool {
    let Some((name, kind)) = CATALOG.iter().find(|row| row.0 == id).map(|row| (row.1, row.2)) else {
        return false;
    };

    if kind != "subscription" {
        return false;
    }

    store.upsert_provider(id, name, kind, "needs-auth", None, None, None, None).is_ok()
}

/// A card's status: the stored row, checked against what this machine shows **now** (1.0).
///
/// The row used to win outright, and rows go stale. Measured on the owner's machine: an `ollama` row saved as
/// `available` on 10-03 kept the card "not connected" while Ollama answered on 11434 with seven models, so
/// Connect "did nothing"; a `deepseek` row said `available` with the key in the keychain. The other way round,
/// a row that says `connected` for a key that is gone drew a green card whose models all fail. Evidence wins:
/// a local server is connected exactly when it answers, an API key exactly when the keychain has it. A row
/// still decides what evidence cannot: `needs-auth` written after the provider refused the key (a bad key is
/// present but not working), and a subscription's own login result.
fn settle_status(id: &str, kind: &str, stored: Option<&str>) -> String {
    let evidence = status_without_a_row(id, kind);

    match (kind, stored) {
        ("local", _) => evidence,
        ("api-key", Some("needs-auth")) if evidence == "connected" => "needs-auth".to_string(),
        ("api-key", _) => evidence,
        /* 3.2: a card the person signed into **from SDC** stays signed in only while the credential is still there.
           A sign-out done in the CLI itself, or SDC's own `cli.logout`, must show on the card at once - the row alone
           used to keep it green over a credential that had been deleted. */
        /* 3.2: Gemini's sign-in is a *file*, so finding one is not evidence that **this person** signed in - the
           consent is the row a sign-in SDC drove wrote. See `subscription_status`. */
        ("subscription", _) if needs_explicit_consent(id) => subscription_status(stored, consent_still_holds(id)),
        /* Every other subscription is *asked* its own command, now (`claude auth status`, `codex login status`), so
           the live answer decides and a `logout` run in the terminal shows on the card. The stored row no longer
           outlives it: a row that said connected over a CLI that had been signed out drew a green card whose every
           turn failed. */
        ("subscription", _) => evidence,
        (_, Some("connected")) => "connected".to_string(),
        (_, _) => evidence,
    }
}

/// Must this provider's sign-in have been done **from the app**, or is its own command enough?
///
/// `true` only for Gemini, and only because its credential is a file on disk that any other program could have
/// written. Claude Code and Codex answer "are you signed in?" themselves, which is a live fact about the account in
/// use rather than a leftover.
pub fn needs_explicit_consent(id: &str) -> bool {
    id == "gemini"
}

/// A subscription card's status from the two facts that decide it (3.2), as a pure function a test can read.
///
/// `stored` is the consent - the row a sign-in **this app drove** wrote. `credential` is whether the credential that
/// sign-in produced is still on this computer, as far as SDC can tell ([`consent_still_holds`]).
///
/// The table, and why each row is what it is:
///
/// | stored        | credential | status       |                                                       |
/// | ------------- | ---------- | ------------ | ----------------------------------------------------- |
/// | `connected`   | yes        | `connected`  | the person signed in here and it still holds          |
/// | `connected`   | no         | `needs-auth` | signed out in the CLI itself, or by `cli.logout`      |
/// | anything else | yes        | `needs-auth` | **a credential is not a sign-in** - the silent one    |
/// | anything else | no         | `needs-auth` | nothing has happened yet                              |
///
/// The third row is the whole point. It used to read `connected`.
pub fn subscription_status(stored: Option<&str>, credential: bool) -> String {
    match (stored, credential) {
        (Some("connected"), true) => "connected".to_string(),
        _ => "needs-auth".to_string(),
    }
}

/// Is the sign-in the stored row records still true on this computer?
///
/// Only Gemini can answer it from a file (its Google sign-in *is* a file). Claude Code and Codex are *asked* their own
/// command by [`signed_in`], which is a live answer rather than a leftover file; for them the row is not overruled by a
/// CLI that happens to be slow or busy, so `signed_in` deciding is the honest reading.
fn consent_still_holds(id: &str) -> bool {
    match id {
        "gemini" => gemini_credentials().map(|path| path.exists()).unwrap_or(false),
        _ => signed_in(id),
    }
}

/// The status of a provider nobody has done anything about yet - see `list`'s doc comment.
fn status_without_a_row(id: &str, kind: &str) -> String {
    match kind {
        "api-key" if keychain::get(&key_ref(id)).is_some() => "connected".to_string(),
        "api-key" => "available".to_string(),
        "local" if crate::engines::ollama::daemon_running() => "connected".to_string(),
        "local" => "needs-auth".to_string(),
        /* A provider whose sign-in must have come from the app has no evidence path at all: a credential found lying
           about is exactly what must not turn the card green (3.2). */
        _ if needs_explicit_consent(id) => "needs-auth".to_string(),
        _ if signed_in(id) => "connected".to_string(),
        _ => "needs-auth".to_string(),
    }
}

/// Whether a subscription CLI is signed in **now**.
///
/// The comment this replaces said a subscription's sign-in "is a separate fact, and one only
/// `cli.login.status` can report". That was true when the daemon had no way to ask - and it meant every
/// subscription card said `needs-auth` in a window where the user had just signed in, so the model menu
/// read `Claude Code · not connected` next to three Claude models it was offering. The report was "even
/// the one that succeeded isn't shown". All three CLIs answer the question when they are asked, and
/// asking is cheap (a few hundred milliseconds, once per `provider.list`):
///
/// ```text
/// $ claude auth status      {"loggedIn": true, "authMethod": "claude.ai", …}
/// $ codex login status      Logged in using ChatGPT
/// ```
///
/// **Gemini is deliberately not asked** (3.2). Its Google sign-in is a *file* (`~/.gemini/oauth_creds.json`), and
/// treating that file as a sign-in meant SDC marked the card connected for a credential some earlier session - or
/// another program entirely - had left behind, without the person ever pressing Sign in. That is the report: *"SDC
/// auto-detects an existing Gemini login and signs in silently. I do NOT want that."* So the Gemini card is
/// `needs-auth` until a sign-in **this app drove** finishes and writes the row (`cli.login.status` does), and
/// `cli.logout` takes both the row and the credential away again.
///
/// Claude Code and Codex are different in kind and are left as they were: neither is read off a file - each is *asked*
/// its own command, now, and answers for the account that is signed in at this moment.
pub fn signed_in(provider_id: &str) -> bool {
    match provider_id {
        "claude" => prints("claude", &["auth", "status"])
            .map(|output| output.contains("\"loggedIn\": true") || output.contains("\"loggedIn\":true"))
            .unwrap_or(false),
        "openai" => prints("codex", &["login", "status"])
            .map(|output| codex_logged_in(&output))
            .unwrap_or(false),
        _ => false,
    }
}

/// Whether the Gemini CLI has a way to be served besides a personal Google sign-in.
fn gemini_has_a_route() -> bool {
    ["GOOGLE_CLOUD_PROJECT", "GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENAI_USE_VERTEXAI"]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()))
}


/// Whether `codex login status` says signed in. "Not logged in" contains "logged in", which made every
/// Codex card say connected on a machine that had never signed in (0.15.8, seen on a clean Mac).
pub fn codex_logged_in(output: &str) -> bool {
    let text = output.to_lowercase();

    text.contains("logged in") && !text.contains("not logged in")
}
/// What a program prints, stdout and stderr together, or `None` when it cannot be started.
fn prints(program: &str, args: &[&str]) -> Option<String> {
    prints_within(program, args, 15)
}

/// [`prints`] with its own bound in seconds.
fn prints_within(program: &str, args: &[&str], seconds: u64) -> Option<String> {
    use std::io::Read;
    use std::process::Stdio;

    /* Bounded (0.15.8): this runs inside `provider.list`, and a CLI that waits on something - a Keychain
       question, a first-run prompt, a network call - used to hold the whole provider list, and the
       window with it, for as long as it waited. */
    let mut command = crate::host::program::command(program)?;
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let started = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(seconds) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                crate::pty::kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();

                return None;
            }
        }
    }

    let mut text = out.join().unwrap_or_default();

    text.push_str(&err.join().unwrap_or_default());

    Some(text)
}

/// Where the Gemini CLI keeps the credential its Google sign-in wrote.
pub fn gemini_credentials() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;

    Some(std::path::PathBuf::from(home).join(".gemini").join("oauth_creds.json"))
}

/// The detail line for a provider, saying something this machine actually checked.
///
/// A subscription's sentence names the CLI and whether it is here, which is what the user has to act
/// on; the catalogue's own line ("uses your own login") says what the product is, not whether *this*
/// install can use it.
fn honest_detail(id: &str, kind: &str, fallback: &str) -> String {
    if kind != "subscription" {
        return fallback.to_string();
    }

    let recipe = crate::auth::cli_login::RECIPES.iter().find(|recipe| recipe.provider_id == id);

    match recipe {
        /* 3.2: the Gemini card says what pressing Sign in does, because that is now the only way it turns green -
           SDC no longer reads a credential off the disk and calls it a sign-in. The second clause is measured
           (2026-09-26, CLI 0.60.0, a finished sign-in): Google answers a personal account's turns from this CLI with
           `IneligibleTierError`, so the route that does work is named beside it. */
        Some(recipe) if recipe.provider_id == "gemini" && crate::host::doctor::has(recipe.program) => format!(
            "`{}` is installed · Sign in opens Google in your browser",
            recipe.program,
        ),
        Some(recipe) if crate::host::doctor::has(recipe.program) => format!(
            "`{}` is installed · Connect starts its own sign-in",
            recipe.program
        ),
        /* 3.2: "not installed" and "installed but it cannot start" are different problems with different fixes, and
           this machine had the second one: `gemini.cmd` and `codex.cmd` were both on PATH and both answered
           `'"node"' is not recognized`, because Node.js was gone. The card used to say they were not installed, which
           sends a person to reinstall a CLI that is already there. */
        Some(recipe) if crate::host::program::resolve(recipe.program).is_some() => format!(
            "`{}` is on this computer but will not start · it needs Node.js - Settings → Environment installs it",
            recipe.program
        ),
        Some(recipe) => format!(
            "`{}` is not installed · Connect installs it in one click",
            recipe.program
        ),
        None => fallback.to_string(),
    }
}

/// Every public Alibaba endpoint a key can belong to, in the order they are asked: `(card, base URL)`.
///
/// Alibaba answers "Incorrect API key" for a key from another region **and** for a Coding Plan key on
/// Model Studio, so the only way to know where a key lives is to ask each place once. Measured
/// 2026-09-29: the user's Model Studio key answers on `dashscope-intl` only, their Coding Plan key on
/// `coding-intl` only.
const ALIBABA: &[(&str, &str)] = &[
    ("qwen", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
    ("qwen-coding", "https://coding-intl.dashscope.aliyuncs.com/v1"),
    ("qwen", "https://dashscope-us.aliyuncs.com/compatible-mode/v1"),
    ("qwen", "https://dashscope.aliyuncs.com/compatible-mode/v1"),
    ("qwen-coding", "https://coding.dashscope.aliyuncs.com/v1"),
];

/// Which of the places that accepted a key to keep (0.21): a URL the person set on the card always (it is
/// index 0 when `has_override`); otherwise, on the card of the first acceptance, the region that answered
/// fastest - a key that works in Singapore and in Virginia is used from the nearer one. `accepted` is
/// `(index into candidates, time it took)`.
pub fn fastest_accepted(candidates: &[(String, String)], accepted: &[(usize, std::time::Duration)], has_override: bool) -> Option<usize> {
    let (first, _) = *accepted.first()?;

    if has_override && first == 0 {
        return Some(0);
    }

    let card = &candidates[first].0;

    accepted.iter().filter(|(index, _)| &candidates[*index].0 == card).min_by_key(|(_, took)| *took).map(|(index, _)| *index)
}

/// Whether a card is one of Alibaba's two, whose keys [`place_key`] sorts between them.
pub fn is_alibaba(id: &str) -> bool {
    id == "qwen" || id == "qwen-coding"
}

/// What to do with a card's base URL once its key was placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// Leave whatever is set.
    Keep,
    /// The card's built-in endpoint answered: clear an old override.
    BuiltIn,
    /// This URL answered.
    At(String),
}

/// Where a key is accepted, asked **before** it is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// The provider accepted it - on this card (which may not be the one it was pasted on).
    Home { id: String, base: Base },
    /// Every place that was asked answered 401 / 403, in the provider's own words.
    Rejected(String),
    /// Nobody could be asked (no check endpoint, or no network): saved as before, unjudged.
    Unchecked,
}

/// 0.14.4: a key is judged before it is saved. `provider.save` used to write any string to the keychain
/// and mark the card Connected, so a key the provider refuses looked connected and the model menu quietly
/// fell back to the bundle. For Alibaba it also finds the key's home: a Coding Plan key pasted on the
/// Model Studio card goes to the Coding Plan card, and a US or Beijing key gets its region's URL.
pub fn place_key(id: &str, key: &str, url: Option<&str>) -> Placement {
    let key = clean_key(key);

    if key.is_empty() {
        return Placement::Unchecked;
    }

    let ask = |card: &str, base: Option<&str>| -> Option<Result<(), String>> {
        let (url, headers) = key_check_at(card, &key, base)?;

        Some(match judge(card, &url, &headers) {
            Ok((status, _)) if (200..300).contains(&status) => Ok(()),
            Ok((status @ (401 | 403), body)) => Err(with_hint(card, status, crate::engines::native_api::rejection(status, &body))),
            /* Anything else (a 5xx, a timeout) did not judge the key. */
            _ => return None,
        })
    };

    if let Some(url) = url.map(str::trim).filter(|url| !url.is_empty()) {
        return match ask(id, Some(url)) {
            Some(Ok(())) => Placement::Home { id: id.to_string(), base: Base::At(url.to_string()) },
            Some(Err(reason)) => Placement::Rejected(reason),
            None => Placement::Unchecked,
        };
    }

    if !is_alibaba(id) {
        return match ask(id, crate::providers::models::endpoint_overrides().get(id).map(String::as_str)) {
            Some(Ok(())) => Placement::Home { id: id.to_string(), base: Base::Keep },
            Some(Err(reason)) => Placement::Rejected(reason),
            None => Placement::Unchecked,
        };
    }

    /* The card's own URL first (a workspace host the person set), then the public ones, this card's first. */
    let mut candidates: Vec<(String, String)> = Vec::new();

    if let Some(base) = crate::providers::models::endpoint_overrides().get(id) {
        candidates.push((id.to_string(), base.clone()));
    }

    for same in [true, false] {
        for (card, base) in ALIBABA.iter().filter(|(card, _)| (*card == id) == same) {
            if !candidates.iter().any(|(_, known)| known == base) {
                candidates.push((card.to_string(), base.to_string()));
            }
        }
    }

    let mut first_rejection: Option<String> = None;
    let mut unjudged = false;
    /* All asked at once - one after another took 13.6 s measured, most of it Beijing - and the answers
       read in the order above, so the card the key was pasted on still wins a tie. */
    let has_override = crate::providers::models::endpoint_overrides().contains_key(id);
    /* 0.21: each answer is timed - the round trip of a key check is the round trip every turn will pay. */
    let answers: Vec<(Option<Result<(), String>>, std::time::Duration)> = std::thread::scope(|scope| {
        let asking: Vec<_> = candidates
            .iter()
            .map(|(card, base)| {
                let ask = &ask;

                scope.spawn(move || {
                    let started = std::time::Instant::now();

                    (ask(card, Some(base)), started.elapsed())
                })
            })
            .collect();

        asking.into_iter().map(|handle| handle.join().unwrap_or((None, std::time::Duration::MAX))).collect()
    });
    let accepted: Vec<(usize, std::time::Duration)> = answers
        .iter()
        .enumerate()
        .filter(|(_, (answer, _))| matches!(answer, Some(Ok(()))))
        .map(|(index, (_, took))| (index, *took))
        .collect();

    if let Some(chosen) = fastest_accepted(&candidates, &accepted, has_override) {
        let (card, base) = candidates[chosen].clone();
        let built_in = ALIBABA.iter().find(|(owner, _)| *owner == card).map(|(_, url)| *url);
        let base = if built_in == Some(base.as_str()) { Base::BuiltIn } else { Base::At(base) };

        return Placement::Home { id: card, base };
    }

    for answer in answers.into_iter().map(|(answer, _)| answer) {
        match answer {
            Some(Ok(())) => {}
            Some(Err(reason)) => {
                first_rejection.get_or_insert(reason);
            }
            None => unjudged = true,
        }
    }

    match first_rejection {
        Some(_) if unjudged => Placement::Unchecked,
        Some(reason) => Placement::Rejected(format!(
            "{reason} · Asked every Alibaba endpoint (Model Studio Singapore, US, Beijing and the Coding Plan): none accepts this key."
        )),
        None => Placement::Unchecked,
    }
}

/// The start-up repair for a key that is on the wrong Alibaba card - the state 0.14.3 left behind when a
/// Coding Plan key was pasted on the Model Studio card. Only a key its own card **refuses** is moved;
/// one that answers, or cannot be asked, stays where it is. Answers the card it moved to.
pub fn repair_alibaba(store: &Arc<Store>) -> Option<String> {
    let key = keychain::get(&key_ref("qwen")).filter(|key| !key.trim().is_empty())?;
    let overrides = crate::providers::models::endpoint_overrides();
    let (url, headers) = key_check_at("qwen", &key, overrides.get("qwen").map(String::as_str))?;

    match crate::engines::native_api::get_json(&url, &headers) {
        Ok((401 | 403, _)) => {}
        _ => return None,
    }

    let Placement::Home { id, base } = place_key("qwen", &key, None) else {
        return None;
    };

    if id == "qwen" {
        /* The same card in another region: point it there. */
        let url = match base {
            Base::At(url) => Some(url),
            _ => None,
        };

        crate::providers::models::set_endpoint_override("qwen", url.as_deref()).ok()?;

        return Some(id);
    }

    if keychain::get(&key_ref(&id)).filter(|other| !other.trim().is_empty()).is_some() {
        /* The other card has a key of its own; nothing is overwritten. */
        return None;
    }

    if let Base::At(url) = &base {
        crate::providers::models::set_endpoint_override(&id, Some(url)).ok()?;
    }

    save(store, &id, "api-key", Some(&key), None, None, None).ok()?;
    remove(store, "qwen").ok()?;
    store.replace_models("qwen", &[], "").ok();

    Some(id)
}

/// Flow 1's `Save` and flow 4's endpoint: the secret goes to the keychain, the *rest* to SQLite.
pub fn save(
    store: &Arc<Store>,
    id: &str,
    kind: &str,
    key: Option<&str>,
    label: Option<&str>,
    url: Option<&str>,
    protocol: Option<&str>,
) -> Result<Value, ErrorObject> {
    let name = CATALOG
        .iter()
        .find(|row| row.0 == id)
        .map(|row| row.1.to_string())
        .unwrap_or_else(|| id.to_string());
    /* An API key has no whitespace in it; a paste can (a trailing newline, a key wrapped across lines). */
    let cleaned = clean_key(key.unwrap_or_default());
    let secret = cleaned.as_str();

    if !secret.is_empty() {
        keychain::set(&key_ref(id), secret)?;
    }

    /* A catalogue provider with its own base URL (0.12): an Alibaba Model Studio workspace answers at its
       workspace host, and a key made there is refused anywhere else. The custom endpoint keeps its URL on
       its own row, as before. */
    if id != "custom" {
        if let Some(base) = url {
            crate::providers::models::set_endpoint_override(id, Some(base)).map_err(ErrorObject::bad_request)?;
        }
    }

    let account = if secret.is_empty() {
        label
            .map(str::to_string)
            .or_else(|| keychain::get(&key_ref(id)).map(|secret| keychain::mask(&secret)))
    } else {
        Some(keychain::mask(secret))
    };

    store
        .upsert_provider(id, &name, kind, "connected", account.as_deref(), url, protocol, Some(&key_ref(id)))
        .map_err(ErrorObject::internal)?;

    Ok(json!({
        "id": id,
        "status": "connected",
        "account": account,
        "keychain": keychain::backend(),
    }))
}

/// Flow 1's `Remove`: forgets the secret and puts the card back to `available`.
///
/// It is the same event the Provider Hub's own seed pushes (`ProviderStatus` with `available`), so a
/// card that is disconnected in the app is disconnected in the daemon too. The row keeps its
/// `key_ref`; only the secret goes.
pub fn remove(store: &Arc<Store>, id: &str) -> Result<Value, ErrorObject> {
    let (name, kind) = CATALOG
        .iter()
        .find(|row| row.0 == id)
        .map(|row| (row.1.to_string(), row.2.to_string()))
        .unwrap_or_else(|| (id.to_string(), "api-key".to_string()));

    keychain::delete(&key_ref(id))?;
    store
        .upsert_provider(id, &name, &kind, "available", None, None, None, None)
        .map_err(ErrorObject::internal)?;

    Ok(json!({ "removed": true, "id": id, "status": "available" }))
}

/// Flow 2's first half: the URL the app opens, and the `state` it must send back - so a callback that
/// arrives out of order cannot be mistaken for a login.
pub fn oauth_open(id: &str) -> Value {
    json!({
        "url": format!("https://auth.example.com/{id}/authorize?state=sdcd-{id}"),
        "state": format!("sdcd-{id}"),
    })
}

/// Flow 2's second half. The token exchange is a later step, and this says so.
pub fn oauth_callback(id: &str, state: &str) -> Value {
    json!({
        "ok": false,
        "state": state,
        "account": Value::Null,
        "error": format!("{id}: the token exchange lands with the OAuth step; the state was recorded"),
    })
}

/// Flow 3: the local daemon's two rows, from the Ollama adapter's real probes.
pub fn local_doctor() -> Value {
    let running = crate::engines::ollama::daemon_running();
    let models = if running { crate::engines::ollama::list_models() } else { Vec::new() };

    /* P0-16: what this machine can carry, as a proposal the person reads - never applied. */
    let tuning = if running && crate::v3::on("local.autotune") { crate::sme::tuner::tuning_for_ollama(&models) } else { Value::Null };

    json!({
        "daemon": running,
        "endpoint": format!("http://{}", crate::engines::ollama::ENDPOINT),
        "tuning": tuning,
        "models": models,
        "detail": if running { "running" } else if crate::host::program::resolve("ollama").is_some() { "installed · would not start" } else { "not installed · Settings → Environment → Install" },
        "installed": crate::host::program::resolve("ollama").is_some(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 1.0: a stored row no longer outlives the evidence. An API-key card with no key is not connected
    /// whatever its row says; a subscription's own login result still stands.
    #[test]
    fn a_stale_row_does_not_outlive_the_evidence() {
        assert_eq!(settle_status("no-such-provider-for-tests", "api-key", Some("connected")), "available");
        assert_eq!(settle_status("no-such-provider-for-tests", "api-key", None), "available");
        /* 3.2: a subscription row no longer outlives its CLI either. The row used to win here; now the CLI's own
           answer does, so a `logout` run in the terminal shows on the card instead of a green card that fails. */
        assert_eq!(settle_status("no-such-provider-for-tests", "subscription", Some("connected")), "needs-auth");
    }

    /// **A credential is not a sign-in** (3.2) - the report "SDC auto-detects an existing Gemini login".
    ///
    /// The table in [`subscription_status`], as assertions, plus the two facts that make it reachable: `signed_in`
    /// never answers for Gemini (so no file on disk can turn the card green), and the Gemini card is the only one
    /// that needs the app's own sign-in.
    #[test]
    fn a_gemini_credential_found_on_the_disk_is_not_a_sign_in() {
        assert_eq!(subscription_status(Some("connected"), true), "connected", "signed in here, and it still holds");
        assert_eq!(subscription_status(Some("connected"), false), "needs-auth", "signed out since");
        assert_eq!(subscription_status(None, true), "needs-auth", "a credential nobody signed in with");
        assert_eq!(subscription_status(Some("needs-auth"), true), "needs-auth");
        assert_eq!(subscription_status(None, false), "needs-auth");

        /* Whatever is on this computer's disk, the Gemini card cannot be green without the app's own sign-in. */
        assert!(!signed_in("gemini"), "Gemini is never read off a file");
        assert_eq!(settle_status("gemini", "subscription", None), "needs-auth");
        assert_eq!(status_without_a_row("gemini", "subscription"), "needs-auth");

        assert!(needs_explicit_consent("gemini"));
        assert!(!needs_explicit_consent("claude"), "Claude Code answers `auth status` itself");
        assert!(!needs_explicit_consent("openai"), "Codex answers `login status` itself");
    }

    #[test]
    fn a_refused_key_is_told_apart_from_other_failures() {
        assert!(refused_key("The provider refused the request (401): Incorrect API key provided"));
        assert!(refused_key("invalid x-api-key"));
        assert!(!refused_key("The provider refused the request (429): rate limited"));
        assert!(!refused_key("could not reach the provider"));
    }

    #[test]
    fn a_key_good_in_two_regions_is_used_from_the_faster_one() {
        let ms = std::time::Duration::from_millis;
        let candidates: Vec<(String, String)> = [
            ("qwen", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
            ("qwen-coding", "https://coding-intl.dashscope.aliyuncs.com/v1"),
            ("qwen", "https://dashscope-us.aliyuncs.com/compatible-mode/v1"),
        ]
        .iter()
        .map(|(card, url)| (card.to_string(), url.to_string()))
        .collect();

        assert_eq!(fastest_accepted(&candidates, &[(0, ms(900)), (2, ms(300))], false), Some(2), "Virginia answered faster");
        assert_eq!(fastest_accepted(&candidates, &[(0, ms(200)), (2, ms(300))], false), Some(0));
        assert_eq!(fastest_accepted(&candidates, &[(0, ms(900)), (1, ms(100))], false), Some(0), "another card's region is not this key's home");
        assert_eq!(fastest_accepted(&candidates, &[(0, ms(900)), (2, ms(100))], true), Some(0), "the URL the person set wins");
        assert_eq!(fastest_accepted(&candidates, &[], false), None);
    }

    /// 0.15.8: "Not logged in" is not a sign-in. The clean Mac probe showed Codex as connected.
    #[test]
    fn not_logged_in_is_not_logged_in() {
        assert!(!codex_logged_in("Not logged in\n"));
        assert!(codex_logged_in("Logged in using ChatGPT\n"));
        assert!(codex_logged_in("Logged in using an API key - sk-proj-***"));
    }

    #[test]
    fn the_catalog_and_the_registry_match_the_apps_seed() {
        assert_eq!(CATALOG.len(), 16);
        /* Every card whose models come from an endpoint has a block in the bundle, so its chat can be
           routed and its list can refresh - a card without one would show models it cannot run. */
        let blocks = models::blocked();

        for (id, _, kind, ..) in CATALOG {
            if *kind == "api-key" || *kind == "local" {
                assert!(blocks.iter().any(|block| block.id == *id), "{id} has no bundle block");
            }
        }

        assert_eq!(MODELS.len(), 12);
        assert_eq!(registry(&[]).len(), 12);
        assert_eq!(registry(&["deepseek/deepseek-chat".to_string()])[7]["enabled"], json!(false));
    }

    /// The second half of the 0.5.0 fix, one layer down: a status has to be evidence.
    ///
    /// `list` used to answer `connected` for every subscription, on any machine - three green cards for
    /// three CLIs nobody had signed in, and the app can only draw what the daemon tells it. 0.7.0 moved
    /// the other way for the *signed in* case: the card now asks the CLI (`signed_in`), so a status that
    /// says `connected` means one of them answered `loggedIn: true`, and this test holds on any machine -
    /// it compares the card with the CLI's own answer instead of assuming either one.
    #[test]
    fn a_subscription_reports_what_its_cli_says() {
        let store = Arc::new(Store::in_memory().unwrap());
        let mut rows = list(&store);
        let card = |rows: &[Value], id: &str| rows.iter().find(|row| row["id"] == json!(id)).cloned().unwrap();

        for id in ["claude", "openai", "gemini"] {
            /* Each answer is a live, time-bounded CLI call: on a loaded machine one of the two can time out, so a disagreement is asked again before it is a failure. */
            let mut attempts = 0;

            let (status, expected) = loop {
                let status = card(&rows, id)["status"].as_str().unwrap_or_default().to_string();
                let expected = if signed_in(id) { "connected" } else { "needs-auth" };

                attempts += 1;

                if status == expected || attempts == 3 {
                    break (status, expected);
                }

                rows = list(&store);
            };

            assert_eq!(status, expected, "{id} does not agree with its own CLI");
            /* And the line under it names the program, which is what the user has to act on. */
            assert!(
                card(&rows, id)["detail"].as_str().unwrap_or_default().contains('`'),
                "{id} has no CLI in its detail line"
            );
        }

        /* A provider id that is not a CLI has no sign-in to ask about. */
        assert!(!signed_in("deepseek"));
        assert!(!signed_in(""));

        /* No invented money anywhere in the catalogue: this build cannot see a provider's billing. */
        for row in &rows {
            let detail = row["detail"].as_str().unwrap_or_default();

            assert!(!detail.contains('$'), "a price this build invented: {detail}");
        }
    }

    /// A dedicated id for the tests: a unit test must not depend on - or disturb - whatever key the
    /// machine running it happens to hold for a real provider.
    const TEST_PROVIDER: &str = "smoke-only-provider";

    #[test]
    fn test_is_structured_and_says_whether_it_verified() {
        let missing = test(TEST_PROVIDER, None);

        assert_eq!(missing["ok"], json!(false));
        assert_eq!(missing["verified"], json!(false));
        assert!(missing["error"].is_string());

        let present = test(TEST_PROVIDER, Some("sk-abcdefghijkl"));

        assert_eq!(present["ok"], json!(true));
        assert_eq!(present["models"], json!(12));
        assert_eq!(present["account"], json!("sk-a…ijkl"));
        /* The part that matters: it does not claim the provider was asked. */
        assert_eq!(present["verified"], json!(false));
        assert!(present["detail"].as_str().unwrap().contains("was not contacted"));
    }

    #[test]
    fn a_check_request_uses_each_dialects_own_auth_header() {
        let (url, headers) = key_check("anthropic-api", "sk-ant-1").expect("anthropic-api is checkable");

        assert!(url.starts_with("https://api.anthropic.com/"));
        assert!(headers.iter().any(|(name, value)| name == "x-api-key" && value == "sk-ant-1"));
        assert!(headers.iter().any(|(name, _)| name == "anthropic-version"));

        let (url, headers) = key_check("openai-api", "sk-1").expect("openai-api is checkable");

        assert!(url.starts_with("https://api.openai.com/"));
        assert!(headers.iter().any(|(name, value)| name == "authorization" && value == "Bearer sk-1"));

        /* A provider with no check endpoint is not guessed at. */
        assert!(key_check("smoke-only-provider", "sk-1").is_none());

        /* A saved workspace URL is where the key is checked, not the built-in host. */
        let workspace = "https://ws-1.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/";

        assert_eq!(key_check_at("qwen", "sk-1", Some(workspace)).unwrap().0, "https://ws-1.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/models");
        assert_eq!(key_check_at("qwen", "sk-1", None).unwrap().0, "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models");

        /* A pasted key loses its whitespace, and Alibaba's 401 says what to do. */
        assert_eq!(clean_key(" sk-ab\r\ncd \n"), "sk-abcd");
        assert!(with_hint("qwen", 401, "Incorrect API key provided. (401)".into()).contains("Model Studio"));
        assert_eq!(with_hint("deepseek", 401, "no".into()), "no");
    }

    #[test]
    fn a_model_list_is_counted_from_either_dialect() {
        assert_eq!(model_count(r#"{"data":[{},{},{}]}"#), 3);
        assert_eq!(model_count("not json at all"), 0);
        assert_eq!(model_count(r#"{"error":{"message":"nope"}}"#), 0);
    }

    #[test]
    fn the_local_provider_is_actually_contacted() {
        let local = test("ollama", None);

        assert_eq!(local["verified"], json!(true));

        /* Whether Ollama runs here or not, the answer has to be one of the two honest ones. */
        assert!(local["ok"] == json!(true) || local["error"].is_string());
    }

    #[test]
    fn removing_a_provider_forgets_the_secret_and_frees_the_card() {
        let store = Arc::new(Store::in_memory().unwrap());

        save(&store, "groq", "api-key", Some("gsk-abcdefghijkl"), None, None, None).unwrap();
        assert!(crate::auth::keychain::get(&key_ref("groq")).is_some());

        let removed = remove(&store, "groq").unwrap();

        assert_eq!(removed["removed"], json!(true));
        assert_eq!(removed["status"], json!("available"));
        assert!(crate::auth::keychain::get(&key_ref("groq")).is_none());
        assert_eq!(store.provider("groq").unwrap().unwrap()["status"], json!("available"));
    }

    #[test]
    fn saving_a_key_writes_the_keychain_and_only_the_mask_to_the_database() {
        let store = Arc::new(Store::in_memory().unwrap());
        let saved = save(&store, "openai-api", "api-key", Some("sk-abcdefghijkl"), None, None, None).unwrap();

        assert_eq!(saved["status"], json!("connected"));
        assert_eq!(
            crate::auth::keychain::get(&key_ref("openai-api")).as_deref(),
            Some("sk-abcdefghijkl")
        );

        let row = store.provider("openai-api").unwrap().unwrap();

        assert_eq!(row["status"], json!("connected"));
        assert_ne!(row["account"], json!("sk-abcdefghijkl"));

        crate::auth::keychain::delete(&key_ref("openai-api")).unwrap();
    }

    #[test]
    fn oauth_reports_that_the_exchange_is_not_wired() {
        let opened = oauth_open("gemini");

        assert!(opened["url"].as_str().unwrap().contains("gemini"));

        let callback = oauth_callback("gemini", opened["state"].as_str().unwrap());

        assert_eq!(callback["ok"], json!(false));
        assert!(callback["error"].as_str().unwrap().contains("OAuth step"));
    }
}

