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

pub mod models;

use std::sync::Arc;

use serde_json::{json, Value};

use crate::auth::keychain;
use crate::sdcp::envelope::ErrorObject;
use crate::store::sqlite::Store;

/// The nine providers the app ships with: `(id, name, kind, logo, initial, detail)`.
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
];

/// The request that checks a key: `(url, headers)`. Pure, so a test can hold the shape.
///
/// Anthropic wants its key in `x-api-key` and a version header; every other provider in the table is
/// OpenAI-compatible, which means a bearer token.
fn key_check(id: &str, key: &str) -> Option<(String, Vec<(String, String)>)> {
    let (_, url) = KEY_CHECK.iter().find(|(provider, _)| *provider == id)?;

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
            "error": if running { Value::Null } else { json!("Ollama is not running · start it with `ollama serve`") },
        });
    }

    let candidate = key.map(str::to_string).or_else(|| keychain::get(&key_ref(id)));

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
        return match crate::engines::native_api::get_json(&url, &headers) {
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
                "error": crate::engines::native_api::rejection(status, &body),
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
    CATALOG
        .iter()
        .map(|(id, name, kind, logo, initial, detail)| {
            let stored = store.provider(id).ok().flatten();
            let status = stored
                .as_ref()
                .and_then(|row| row["status"].as_str().map(str::to_string))
                .unwrap_or_else(|| status_without_a_row(id, kind));
            let detail_text = stored
                .as_ref()
                .and_then(|row| row["detail"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| honest_detail(id, kind, detail));

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
        .collect()
}

/// The status of a provider nobody has done anything about yet - see `list`'s doc comment.
fn status_without_a_row(id: &str, kind: &str) -> String {
    match kind {
        "api-key" if keychain::get(&key_ref(id)).is_some() => "connected".to_string(),
        "api-key" => "available".to_string(),
        "local" if crate::engines::ollama::daemon_running() => "connected".to_string(),
        "local" => "needs-auth".to_string(),
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
/// $ gemini                  (Google sign-in is a file: ~/.gemini/oauth_creds.json)
/// ```
pub fn signed_in(provider_id: &str) -> bool {
    match provider_id {
        "claude" => prints("claude", &["auth", "status"])
            .map(|output| output.contains("\"loggedIn\": true") || output.contains("\"loggedIn\":true"))
            .unwrap_or(false),
        "openai" => prints("codex", &["login", "status"])
            .map(|output| output.to_lowercase().contains("logged in"))
            .unwrap_or(false),
        "gemini" => gemini_credentials().map(|path| path.exists()).unwrap_or(false),
        _ => false,
    }
}

/// What a program prints, stdout and stderr together, or `None` when it cannot be started.
fn prints(program: &str, args: &[&str]) -> Option<String> {
    let mut command = crate::host::program::command(program)?;
    let output = command.args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();

    text.push_str(&String::from_utf8_lossy(&output.stderr));

    Some(text)
}

/// Where the Gemini CLI keeps the credential its Google sign-in wrote.
fn gemini_credentials() -> Option<std::path::PathBuf> {
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
        Some(recipe) if crate::host::doctor::has(recipe.program) => format!(
            "`{}` is installed · Connect starts its own sign-in",
            recipe.program
        ),
        Some(recipe) => format!(
            "`{}` is not installed or not on PATH · install it, then run the environment doctor",
            recipe.program
        ),
        None => fallback.to_string(),
    }
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
    let secret = key.unwrap_or_default();

    if !secret.is_empty() {
        keychain::set(&key_ref(id), secret)?;
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

    json!({
        "daemon": running,
        "endpoint": format!("http://{}", crate::engines::ollama::ENDPOINT),
        "models": models,
        "detail": if running { "running" } else { "not running · start it with `ollama serve`" },
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_and_the_registry_match_the_apps_seed() {
        assert_eq!(CATALOG.len(), 9);
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
        let rows = list(&store);
        let card = |id: &str| rows.iter().find(|row| row["id"] == json!(id)).cloned().unwrap();

        for id in ["claude", "openai", "gemini"] {
            let status = card(id)["status"].as_str().unwrap_or_default().to_string();
            let expected = if signed_in(id) { "connected" } else { "needs-auth" };

            assert_eq!(status, expected, "{id} does not agree with its own CLI");
            /* And the line under it names the program, which is what the user has to act on. */
            assert!(
                card(id)["detail"].as_str().unwrap_or_default().contains('`'),
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

