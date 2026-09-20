//! The model catalogue - and the reason it is *data* (master spec section 9.10).
//!
//! "The model list is always up to date" is a promise that is only honest if it says where each row
//! came from. So every row carries a `source`, and there are exactly three:
//!
//! * **`live`** - the provider's own endpoint answered just now (`/v1/models` for the OpenAI-shaped
//!   ones, `/api/tags` for Ollama). This is the only source that can know about a model released after
//!   this build, and it is also the only one that can fail.
//! * **`cache`** - the last live answer, kept in SQLite with the time it arrived. A refresh that fails
//!   does not throw the answer away.
//! * **`bundled`** - `protocol/models.json`, compiled in, so a fresh install with no network still has
//!   sensible names, tiers and prices.
//!
//! A row that a provider returns but neither cache nor bundle mentions is still shown: a new model is
//! exactly what "up to date" is for, and refusing to show it because this build has never heard of it
//! would be the opposite. Curated metadata (tier, context, price) is merged in when the bundle knows
//! the id, and left empty when it does not - an unknown price is shown as unknown, not guessed.
//!
//! Nothing here needs a code change when a provider ships a new model. That is the point.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;
use crate::store::sqlite::Store;

/// The catalogue, compiled in so the daemon has a list before it has a network.
pub const BUNDLED: &str = include_str!("../../../protocol/models.json");

/// The `settings` key the chosen model lives under.
pub const SELECTED_KEY: &str = "model.selected";

/// One provider's block of the bundle: its live endpoint and the models this build curated.
#[derive(Debug, Clone)]
pub struct ProviderBlock {
    pub id: String,
    pub label: String,
    pub live: String,
    pub protocol: String,
    pub models: Vec<Value>,
}

/// The bundle, parsed once per call (it is a few kilobytes, and parsing it is cheaper than a cache
/// that could go stale against a edited file).
pub fn blocked() -> Vec<ProviderBlock> {
    let parsed: Value = serde_json::from_str(BUNDLED).unwrap_or(Value::Null);
    let mut blocks = Vec::new();

    for group in ["providers", "subscriptions"] {
        for entry in parsed.get(group).and_then(Value::as_array).cloned().unwrap_or_default() {
            blocks.push(ProviderBlock {
                id: entry["id"].as_str().unwrap_or_default().to_string(),
                label: entry["label"].as_str().unwrap_or_default().to_string(),
                live: entry["live"].as_str().unwrap_or_default().to_string(),
                protocol: entry["protocol"].as_str().unwrap_or("openai").to_string(),
                models: entry["models"].as_array().cloned().unwrap_or_default(),
            });
        }
    }

    blocks
}

/// The day the bundle was last curated, which is what the UI shows next to a `bundled` row.
pub fn snapshot_date() -> String {
    serde_json::from_str::<Value>(BUNDLED)
        .ok()
        .and_then(|parsed| parsed.get("snapshot").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// How a row was obtained. The UI shows it, so the strings live here rather than in the UI.
pub fn sources() -> Value {
    json!([])
}

/// A `GET` that answers JSON, over `http` or `https`.
///
/// It used to refuse `https://` - "a TLS client is not linked in this build" - which meant a live
/// model list was only ever possible for a local endpoint: the rows a user picked from were the ones
/// this build curated on the day it shipped, however many models the provider had added since. 0.6.1
/// routes it through the same TLS client the native engine uses, so `Refresh` answers with what the
/// provider lists *today*.
///
/// The provider's own key travels with the request when one is stored, because the endpoints that
/// need it (Anthropic, DeepSeek, Groq) answer `401` without it; OpenRouter's list is public, and a
/// provider that does not look at the header is unaffected by it.
pub fn get_json(url: &str, provider_id: &str, key: Option<&str>) -> Result<Value, String> {
    if url.starts_with("https://") {
        let key = key.filter(|key| !key.trim().is_empty());
        let mut headers = vec![("accept".to_string(), "application/json".to_string())];

        if let Some(key) = key {
            if provider_id == "anthropic-api" {
                headers.push(("x-api-key".to_string(), key.to_string()));
                headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
            } else {
                headers.push(("authorization".to_string(), format!("Bearer {key}")));
            }
        }

        let (status, body) = crate::engines::native_api::get_json(url, &headers)?;

        if !(200..300).contains(&status) {
            return Err(crate::engines::native_api::rejection(status, &body));
        }

        return serde_json::from_str(&body).map_err(|_| "the endpoint did not answer JSON".to_string());
    }

    let rest = url.strip_prefix("http://").ok_or_else(|| {
        format!("{url}: only http:// and https:// endpoints can be listed live")
    })?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let host = authority.split(':').next().unwrap_or(authority);
    let mut socket = TcpStream::connect_timeout(
        &authority.parse().map_err(|error| format!("{authority}: {error}"))?,
        std::time::Duration::from_secs(2),
    )
    .map_err(|error| format!("{authority}: {error}"))?;

    socket.set_read_timeout(Some(std::time::Duration::from_secs(15))).ok();

    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n");

    socket.write_all(request.as_bytes()).map_err(|error| error.to_string())?;

    let mut response = String::new();

    socket.read_to_string(&mut response).map_err(|error| error.to_string())?;

    let (head, body) = response.split_once("\r\n\r\n").unwrap_or(("", response.as_str()));

    /* The status line matters here too, and it used to be ignored: a local endpoint that answers `401`
       (anything OpenAI-compatible behind a password) reported "the endpoint answered without a model
       list" - the body's own sentence is what a person needs. */
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);

    if !(200..300).contains(&status) {
        return Err(crate::engines::native_api::rejection(status, body));
    }

    serde_json::from_str(body).map_err(|_| "the endpoint did not answer JSON".to_string())
}

/// The models a provider's own endpoint lists, normalised to `{ id, tier, ctx, cost }`.
pub fn live(provider: &ProviderBlock) -> Result<Vec<Value>, String> {
    if provider.live.is_empty() {
        return Err("this provider does not list its models".to_string());
    }

    let key = crate::auth::keychain::get(&crate::providers::key_ref(&provider.id));
    let body = get_json(&provider.live, &provider.id, key.as_deref())?;

    /* Two shapes cover every provider we ship: OpenAI's `{data:[{id}]}` and Ollama's `{models:[{name}]}`. */
    let rows = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.get("models").and_then(Value::as_array))
        .cloned()
        .ok_or_else(|| "the endpoint answered without a model list".to_string())?;

    Ok(rows
        .iter()
        .filter_map(|row| {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| row.get("name").and_then(Value::as_str))?
                .to_string();

            Some(json!({ "id": id, "providerId": provider.id }))
        })
        .collect())
}

/// The merged catalogue: live where the provider could be reached, cached where it could not, and the
/// curated bundle underneath both.
///
/// `refresh` decides whether the provider is asked at all. A refresh that fails is **not** an error: the
/// answer carries a `note` saying why, and the rows still come from the cache or the bundle, because a
/// model list that disappears when the network hiccups is worse than a stale one that admits it.
pub fn list(store: &Arc<Store>, provider_id: Option<&str>, refresh: bool) -> Result<Value, ErrorObject> {
    let wanted: Vec<ProviderBlock> = blocked()
        .into_iter()
        .filter(|block| provider_id.map(|wanted| wanted == block.id).unwrap_or(true))
        .collect();

    list_blocks(store, wanted, refresh)
}

/// The body of `list`, over a set of provider blocks.
///
/// Separate from `list` so that a **test can supply the blocks**. That is not tidiness: the test that
/// proves "a refresh that failed keeps the list" used to reach the real `api.groq.com`, which put the
/// public internet inside the release's `Checks` step - it failed on one runner and passed on three
/// others for the same commit. One loopback server is the same code path with none of that.
pub fn list_blocks(
    store: &Arc<Store>,
    wanted: Vec<ProviderBlock>,
    refresh: bool,
) -> Result<Value, ErrorObject> {
    let mut rows: Vec<Value> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    for block in wanted {
        let curated: Vec<Value> =
            block.models.iter().map(|model| decorate(model, &block.id, "bundled", None)).collect();
        let mut provider_rows: Vec<Value> = Vec::new();
        let mut source = "bundled";
        let mut fetched_at: Option<String> = None;
        let mut note: Option<String> = None;

        if refresh {
            match live(&block) {
                Ok(live_rows) if !live_rows.is_empty() => {
                    let now = chrono::Utc::now().to_rfc3339();

                    store.replace_models(&block.id, &live_rows, &now).map_err(ErrorObject::internal)?;
                    provider_rows = merge(&live_rows, &curated, "live", Some(&now));
                    source = "live";
                    fetched_at = Some(now);
                }
                Ok(_) => note = Some(format!("{} answered with an empty list", block.label)),
                Err(reason) => note = Some(format!("{}: {reason}", block.label)),
            }
        }

        if provider_rows.is_empty() {
            let cached = store.cached_models(&block.id).map_err(ErrorObject::internal)?;

            if !cached.is_empty() {
                let cached_at = cached[0]["fetchedAt"].as_str().map(str::to_string);

                provider_rows = merge(&cached, &curated, "cache", cached_at.as_deref());
                source = "cache";
                fetched_at = cached_at;
            }
        }

        if provider_rows.is_empty() {
            provider_rows = curated;
        }

        if let Some(note) = note {
            notes.push(note);
        }

        for mut row in provider_rows {
            row["providerLabel"] = json!(block.label);
            row["source"] = json!(source);
            row["fetchedAt"] = json!(fetched_at);
            rows.push(row);
        }
    }

    Ok(json!({
        "models": rows,
        "snapshot": snapshot_date(),
        "refreshed": refresh,
        "notes": notes,
        "selected": selected(store),
    }))
}


/// The live (or cached) rows, with the bundle's curated fields filled in where it knows the id.
fn merge(rows: &[Value], curated: &[Value], source: &str, fetched_at: Option<&str>) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            let id = row["id"].as_str().unwrap_or_default();
            let mut merged = match curated.iter().find(|model| model["id"] == json!(id)) {
                Some(known) => known.clone(),
                /* A model this build has never heard of is still shown - that is the point of
                   refreshing - and its price is left empty rather than guessed. */
                None => json!({ "id": id, "tier": "balanced", "ctx": 0, "cost": "" }),
            };

            merged["providerId"] = row.get("providerId").cloned().unwrap_or(json!(""));
            merged["source"] = json!(source);

            /* A provider's own list has ids and nothing else, and an id is not a name a person wants to
               read in a menu: the bundle's `name` fills that in, and the id stays as the fallback. */
            if merged.get("name").and_then(Value::as_str).unwrap_or("").is_empty() {
                merged["name"] = json!(friendly_name(id));
            }

            if let Some(fetched_at) = fetched_at {
                merged["fetchedAt"] = json!(fetched_at);
            }

            merged
        })
        .collect()
}

/// One bundled row, as a model row: `{ id, name, providerId, tier, ctx, cost, source }`.
fn decorate(model: &Value, provider_id: &str, source: &str, fetched_at: Option<&str>) -> Value {
    let id = model["id"].as_str().unwrap_or_default();
    let name = model
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| friendly_name(id));

    json!({
        "id": model["id"],
        "name": name,
        "providerId": provider_id,
        "tier": model.get("tier").and_then(Value::as_str).unwrap_or("balanced"),
        "ctx": model.get("ctx").and_then(Value::as_i64).unwrap_or(0),
        "cost": model.get("cost").and_then(Value::as_str).unwrap_or(""),
        "size": model.get("size"),
        "source": source,
        "fetchedAt": fetched_at,
    })
}

/// A readable name for a model id, for the providers that send ids and nothing else.
///
/// `claude-sonnet-4-5` becomes `Claude Sonnet 4.5`, `deepseek-reasoner` becomes `DeepSeek Reasoner`,
/// `gpt-5-mini` becomes `GPT-5 Mini`. It is a spelling rule, not a catalogue: the bundle's own `name`
/// always wins when it has one, and a live row for a model this build has never heard of gets the
/// rule's answer rather than an invented one.
pub fn friendly_name(id: &str) -> String {
    id.split(['/', '-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let known = match part {
                "gpt" => Some("GPT"),
                "api" => Some("API"),
                "gguf" => Some("GGUF"),
                "it" => Some("IT"),
                "mini" => Some("Mini"),
                "pro" => Some("Pro"),
                "flash" => Some("Flash"),
                "turbo" => Some("Turbo"),
                "vl" => Some("VL"),
                "r1" => Some("R1"),
                "v3" => Some("V3"),
                "deepseek" => Some("DeepSeek"),
                "openai" => Some("OpenAI"),
                "nvidia" => Some("NVIDIA"),
                "github" => Some("GitHub"),
                _ => None,
            };

            match known {
                Some(word) => word.to_string(),
                None => {
                    let mut chars = part.chars();

                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    }
                }
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
        /* `GPT 5` is written `GPT-5`, and version parts read better with a point between them:
           `4-5` is 4.5, not 4 5. */
        .replace("GPT ", "GPT-")
        .replace(" 4 5", " 4.5")
        .replace(" 4 6", " 4.6")
        .replace(" 3 5", " 3.5")
        .replace(" 2 5", " 2.5")
        .replace(" 3 1", " 3.1")
        .replace(" 3 3", " 3.3")
        .replace(" 6 7b", " 6.7B")
}

/// Records the chosen model. A setting rather than an event, on purpose: which model is *selected* is a
/// UI preference (spec section 3.3), while the models themselves are the daemon's catalogue.
pub fn select(store: &Arc<Store>, model_id: &str, provider_id: Option<&str>) -> Result<Value, ErrorObject> {
    store.set_setting(SELECTED_KEY, model_id).map_err(ErrorObject::internal)?;

    if let Some(provider_id) = provider_id {
        store.set_setting("model.provider", provider_id).map_err(ErrorObject::internal)?;
    }

    Ok(json!({ "modelId": model_id, "providerId": provider_id }))
}

/// The chosen model, or `null`s when the user has not picked one.
pub fn selected(store: &Store) -> Value {
    json!({
        "modelId": store.setting(SELECTED_KEY).ok().flatten(),
        "providerId": store.setting("model.provider").ok().flatten(),
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_covers_every_provider_the_app_offers() {
        let blocks = blocked();
        let ids: Vec<&str> = blocks.iter().map(|block| block.id.as_str()).collect();

        for provider in [
            "ollama",
            "anthropic-api",
            "openai-api",
            "deepseek",
            "groq",
            "openrouter",
            "custom",
            "claude",
            "openai",
            "gemini",
        ] {
            assert!(ids.contains(&provider), "{provider} is missing from protocol/models.json");
        }

        assert_eq!(snapshot_date().len(), 10, "the bundle carries an ISO date");
    }

    #[test]
    fn a_list_without_a_refresh_is_the_bundle_and_says_so() {
        let store = Arc::new(Store::in_memory().unwrap());
        let listed = list(&store, Some("ollama"), false).unwrap();

        assert_eq!(listed["refreshed"], json!(false));
        assert!(listed["models"].as_array().unwrap().len() >= 3);

        for model in listed["models"].as_array().unwrap() {
            assert_eq!(model["source"], json!("bundled"));
            assert_eq!(model["providerId"], json!("ollama"));
            assert!(model["tier"].is_string());
        }
    }

    /// A rejected key is reported in the provider's own words - hermetically.
    ///
    /// This is the sentence the user meets when a key is wrong, and it is the reason `rejection()` exists.
    /// A loopback server answering the way Groq does keeps the test off the public internet.
    #[test]
    fn a_rejected_key_is_reported_in_the_providers_own_words() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];

                let _ = socket.read(&mut buffer);

                let body = r#"{"error":{"message":"Invalid API Key","type":"invalid_request_error"}}"#;
                let response = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );

                let _ = socket.write_all(response.as_bytes());
            }
        });

        let mut block = blocked().into_iter().find(|block| block.id == "groq").unwrap();

        block.live = format!("http://127.0.0.1:{port}/openai/v1/models");

        assert_eq!(live(&block).unwrap_err(), "Invalid API Key (401)");
    }

    #[test]
    fn a_model_id_becomes_a_name_a_person_can_read() {
        assert_eq!(friendly_name("claude-sonnet-4-5"), "Claude Sonnet 4.5");
        assert_eq!(friendly_name("claude-opus-4"), "Claude Opus 4");
        assert_eq!(friendly_name("gpt-5-mini"), "GPT-5 Mini");
        assert_eq!(friendly_name("deepseek-reasoner"), "DeepSeek Reasoner");
        assert_eq!(friendly_name("gemini-2.5-pro"), "Gemini 2.5 Pro");
        assert_eq!(friendly_name("anthropic/claude-sonnet-4-5"), "Anthropic Claude Sonnet 4.5");
        assert_eq!(friendly_name("llama-3.3-70b-versatile"), "Llama 3.3 70b Versatile");
        assert_eq!(friendly_name(""), "");
    }

    /// A bundle row's own `name` wins over the spelling rule - that is the whole point of carrying one.
    #[test]
    fn a_bundled_name_is_kept() {
        let row = json!({ "id": "gpt-5", "name": "GPT-5 (the release name)" });
        let decorated = decorate(&row, "openai-api", "bundled", None);

        assert_eq!(decorated["name"], json!("GPT-5 (the release name)"));
        assert_eq!(decorated["providerId"], json!("openai-api"));
        assert_eq!(decorated["source"], json!("bundled"));
    }

    /// A refresh test needs the blocks in hand, which is why `list_blocks` is public.
    #[test]
    fn a_failed_refresh_keeps_the_list_and_says_why() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];

                let _ = socket.read(&mut buffer);

                let body = r#"{"error":{"message":"Invalid API Key","type":"invalid_request_error"}}"#;
                let response = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );

                let _ = socket.write_all(response.as_bytes());
            }
        });

        let mut block = blocked().into_iter().find(|block| block.id == "groq").unwrap();

        block.live = format!("http://127.0.0.1:{port}/openai/v1/models");

        let curated = block.models.len();
        let store = Arc::new(Store::in_memory().unwrap());
        let listed = list_blocks(&store, vec![block], true).unwrap();
        let notes = listed["notes"].as_array().unwrap();
        let note = notes[0].as_str().unwrap_or_default().to_lowercase();
        let models = listed["models"].as_array().unwrap();

        assert_eq!(notes.len(), 1, "a refresh that reached nothing explains itself once");
        assert!(note.contains("groq"), "the note names the provider: {note}");
        assert!(note.contains("invalid api key"), "in the provider's own words: {note}");
        assert!(models.len() >= curated.min(2), "the curated rows stay");
        assert!(models.iter().all(|model| model["source"] == json!("bundled")));
    }

    /// The one refresh that *can* succeed in this build: a local `http://` endpoint.
    #[test]
    fn a_reachable_http_endpoint_is_listed_live_and_cached() {
        let store = Arc::new(Store::in_memory().unwrap());

        /* A tiny HTTP server that answers the way `/v1/models` does, on a port the test picks. */
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];

                let _ = socket.read(&mut buffer);
                let body = r#"{"data":[{"id":"local-model-a"},{"id":"local-model-b"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );

                let _ = socket.write_all(response.as_bytes());
            }
        });

        let mut block = blocked().into_iter().find(|block| block.id == "custom").unwrap();

        block.live = format!("http://127.0.0.1:{port}/v1/models");

        let listed = live(&block).unwrap();

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["id"], json!("local-model-a"));

        /* And once cached, a later list without a refresh still shows them. */
        store.replace_models("custom", &listed, "2026-09-20T00:00:00Z").unwrap();

        let cached = store.cached_models("custom").unwrap();

        assert_eq!(cached.len(), 2);
        assert_eq!(cached[0]["id"], json!("local-model-a"));
    }

    #[test]
    fn a_model_the_bundle_does_not_know_is_still_listed() {
        let store = Arc::new(Store::in_memory().unwrap());
        let live_rows = vec![json!({ "id": "brand-new-model", "providerId": "groq" })];

        store.replace_models("groq", &live_rows, "2026-09-20T00:00:00Z").unwrap();

        let curated = vec![json!({
            "id": "llama-3.3-70b-versatile", "tier": "balanced", "ctx": 128000, "cost": "$0.59 / $0.79"
        })];
        let merged = merge(&live_rows, &curated, "live", None);

        assert_eq!(merged[0]["id"], json!("brand-new-model"));
        assert_eq!(merged[0]["tier"], json!("balanced"));
        assert_eq!(merged[0]["cost"], json!(""), "an unknown price is left empty, not guessed");

        /* And it survives into a list that does not refresh, from the cache. */
        let listed = list(&store, Some("groq"), false).unwrap();

        assert!(listed["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == json!("brand-new-model")));
    }

    #[test]
    fn selecting_a_model_is_remembered() {
        let store = Arc::new(Store::in_memory().unwrap());

        assert_eq!(selected(&store)["modelId"], Value::Null);
        select(&store, "claude-sonnet-4-5", Some("anthropic-api")).unwrap();
        assert_eq!(selected(&store)["modelId"], json!("claude-sonnet-4-5"));
        assert_eq!(selected(&store)["providerId"], json!("anthropic-api"));
    }

    /// The refusal that is left, and why it is a different one.
    ///
    /// `https://` is no longer refused by a missing TLS client (0.6.1 linked one); what is refused is a
    /// scheme this function cannot speak, and the sentence names the scheme rather than the build.
    /// The https *path* itself is covered by `engines::native_api`'s own test, which needs no network:
    /// 127.0.0.1:1 refuses the connection immediately.
    #[test]
    fn an_unknown_scheme_is_refused_by_name() {
        let error = get_json("ftp://example.invalid/models", "groq", None).unwrap_err();

        assert!(error.contains("ftp://example.invalid/models"), "{error}");
        assert!(!error.contains("TLS"), "{error}");
    }
}
