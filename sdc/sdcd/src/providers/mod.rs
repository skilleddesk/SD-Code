//! The provider backend - the six flows of spec section 9.10, server side.
//!
//! The Provider Hub in the app is a view over this module. Each flow ends in a `ProviderStatus` event
//! (so the card, the topbar dot and the status bar all move together) and answers a method result:
//!
//! | flow          | methods                             | what it writes          |
//! | ------------- | ----------------------------------- | ----------------------- |
//! | `api_key`     | `provider.test`, `provider.save`    | keychain entry + row    |
//! | `subscription`| `provider.oauth.open`, `…callback`  | a recorded state        |
//! | `local`       | `provider.local.doctor`             | nothing (a read)        |
//! | `custom`      | `provider.save` with a url          | row with url + protocol |
//! | `registry`    | `provider.registry.list`, `…set`    | `models.enabled`        |
//! | `doctor`      | `host.doctor`                       | nothing (a read)        |
//!
//! The subscription flow stops at the token step on purpose: the plan's spec section 15 says provider
//! OAuth lands in a later step, so `oauth_callback` records the state and says so rather than
//! inventing a token that would fail somewhere stranger. Everything else is real, including the
//! keychain write.

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
    ("openai-api", "OpenAI API", "api-key", "openai", "O", "Direct API key · $12.40 / $50.00 this month"),
    ("deepseek", "DeepSeek", "api-key", "deepseek", "D", "Direct API · cheap, fast"),
    ("groq", "Groq", "api-key", "groq", "G", "Ultra-fast inference"),
    ("openrouter", "OpenRouter", "api-key", "openrouter", "O", "One key · 200+ models"),
    ("ollama", "Ollama", "local", "ollama", "O", "Local models · auto-detected on this machine"),
];

/// The twelve models of the registry: `(id, provider, tier, ctx, cost)`.
pub const MODELS: &[(&str, &str, &str, i64, &str)] = &[
    ("anthropic/claude-sonnet-4-5", "anthropic-api", "balanced", 200_000, "$3 / $15"),
    ("anthropic/claude-opus-4", "anthropic-api", "deep", 200_000, "$15 / $75"),
    ("anthropic/claude-haiku-4", "anthropic-api", "fast", 200_000, "$0.80 / $4"),
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

/// Flow 1's `Test`.
///
/// It is honest about what it did. A key's *shape* is checked locally; the provider is contacted only
/// where this build can contact it (the local Ollama daemon, and an `http://` endpoint). The answer
/// says which of the two happened in `verified`, and `detail` spells it out, because a green tick
/// that means "we never asked the provider" is worse than no tick at all.
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

    match candidate {
        Some(secret) if !secret.trim().is_empty() => json!({
            "ok": true,
            "verified": false,
            "models": MODELS.len(),
            "detail": format!(
                "Key accepted (shape checked) · {} models available · the provider was not contacted: no TLS client in this build",
                MODELS.len()
            ),
            "account": keychain::mask(&secret),
        }),
        _ => json!({
            "ok": false,
            "verified": false,
            "models": 0,
            "detail": "",
            "error": "Enter a key first",
        }),
    }
}

/// The nine provider cards, with whatever the store and the keychain already know.
pub fn list(store: &Arc<Store>) -> Vec<Value> {
    CATALOG
        .iter()
        .map(|(id, name, kind, logo, initial, detail)| {
            let stored = store.provider(id).ok().flatten();
            let status = stored
                .as_ref()
                .and_then(|row| row["status"].as_str().map(str::to_string))
                .unwrap_or_else(|| {
                    if *kind == "api-key" && keychain::get(&key_ref(id)).is_none() {
                        "available".to_string()
                    } else if *kind == "local" {
                        "needs-auth".to_string()
                    } else {
                        "connected".to_string()
                    }
                });

            json!({
                "id": id,
                "name": name,
                "kind": kind,
                "status": status,
                "detail": stored.as_ref().and_then(|row| row["detail"].as_str()).unwrap_or(detail),
                "account": stored.as_ref().and_then(|row| row["account"].as_str()),
                "logo": logo,
                "initial": initial,
                "url": stored.as_ref().and_then(|row| row["url"].as_str()),
                "protocol": stored.as_ref().and_then(|row| row["protocol"].as_str()),
            })
        })
        .collect()
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

