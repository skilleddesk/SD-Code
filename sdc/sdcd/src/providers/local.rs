//! Local model servers that are not Ollama (0.22) - anything that speaks OpenAI's API on this computer or on
//! the local network: LM Studio, llama.cpp's `llama-server`, llamafile, LocalAI, vLLM, SGLang, Jan, KoboldCpp,
//! text-generation-webui, GPT4All, or a model someone built and serves themselves.
//!
//! Before this only Ollama was a local provider, and the one "custom endpoint" had to be typed by hand, sat at
//! `127.0.0.1:8080` and asked for an API key a local server does not have. Now:
//!
//!   * `local.discover` looks at the ports those servers use by default on this computer and answers what it
//!     finds, with their models;
//!   * `local.add` connects one by address - this computer, or another one on the network (a GPU box at
//!     `http://192.168.1.20:8000/v1`) - with an optional key for a server started with one;
//!   * every connected server is a provider of its own (`local-<host>-<port>`), with its live model list in the
//!     model menu, its context size read from the server when it says it, and no key needed.
//!
//! Servers are kept in `<data>/local-servers.json`; a key, when one is given, in the OS keychain like any other.

use std::time::Duration;

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;

/// Where each well-known local server listens by default, and what it is called. A port can be shared
/// (`8080` is llama.cpp's and LocalAI's and llamafile's), so the name says so.
pub const KNOWN: &[(u16, &str)] = &[
    (1234, "LM Studio"),
    (8080, "llama.cpp / llamafile / LocalAI"),
    (8000, "vLLM"),
    (30000, "SGLang"),
    (1337, "Jan"),
    (5001, "KoboldCpp"),
    (5000, "text-generation-webui"),
    (4891, "GPT4All"),
    (8081, "Local server"),
    (8888, "Local server"),
];

/// The context a local model gets when its server does not say (a small model on a laptop, usually).
pub const DEFAULT_CONTEXT: u64 = 8_192;

/// Whether a provider id is one of these servers.
pub fn is_local(provider: &str) -> bool {
    provider.starts_with("local-")
}

fn path() -> Option<std::path::PathBuf> {
    crate::paths::data_dir().ok().map(|dir| dir.join("local-servers.json"))
}

/// The connected servers: `[{ id, label, base, models: [{id, ctx}] }]`.
pub fn saved() -> Vec<Value> {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
        .unwrap_or_default()
}

fn save_all(servers: &[Value]) -> Result<(), String> {
    let path = path().ok_or("no data folder")?;

    std::fs::write(&path, serde_json::to_string_pretty(servers).unwrap_or_default()).map_err(|error| error.to_string())
}

/// `http://127.0.0.1:1234/v1` from what a person pasted: a trailing `/chat/completions` or `/models` is taken off,
/// a bare `host:port` gets `http://` and `/v1`. Only `http(s)://` to this computer, the local network or any
/// `https://` host is taken - an `http://` address on the internet would send prompts in the clear.
pub fn clean_base(input: &str) -> Result<String, String> {
    let mut base = input.trim().trim_end_matches('/').to_string();

    if base.is_empty() {
        return Err("Type the server's address, for example http://127.0.0.1:1234/v1".into());
    }

    if !base.contains("://") {
        base = format!("http://{base}");
    }

    for tail in ["/chat/completions", "/completions", "/models"] {
        if let Some(stripped) = base.strip_suffix(tail) {
            base = stripped.trim_end_matches('/').to_string();
        }
    }

    let after = base.split_once("://").map(|(_, rest)| rest.to_string()).unwrap_or_default();
    let host = after.split(['/', ':']).next().unwrap_or_default().to_ascii_lowercase();

    if !after.contains('/') {
        base.push_str("/v1");
    }

    if base.starts_with("https://") || (base.starts_with("http://") && private_host(&host)) {
        Ok(base)
    } else {
        Err(format!("{host} is not on this computer or the local network: use https:// for a server on the internet"))
    }
}

/// This computer or a private network address.
pub fn private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".lan") || host.ends_with(".home") {
        return true;
    }

    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local() || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1])),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

/// The provider id for a base URL: `local-127-0-0-1-1234`.
pub fn id_for(base: &str) -> String {
    let after = base.split_once("://").map(|(_, rest)| rest).unwrap_or(base);
    let authority = after.split('/').next().unwrap_or(after);
    let slug: String = authority.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();

    format!("local-{}", slug.trim_matches('-'))
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(timeout).timeout_read(Duration::from_secs(5)).build()
}

fn get(url: &str, key: Option<&str>, timeout: Duration) -> Result<Value, String> {
    let mut request = agent(timeout).get(url).set("accept", "application/json");

    if let Some(key) = key.filter(|key| !key.is_empty()) {
        request = request.set("authorization", &format!("Bearer {key}"));
    }

    match request.call() {
        Ok(response) => response.into_string().ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()).ok_or_else(|| format!("{url} answered something that is not a model list")),
        Err(ureq::Error::Status(code @ (401 | 403), _)) => Err(format!("the server wants a key (HTTP {code}) - add the one it was started with")),
        Err(ureq::Error::Status(code, _)) => Err(format!("{url} answered HTTP {code} - is it an OpenAI-compatible server?")),
        Err(error) => Err(format!("nothing answered at {url} ({error})")),
    }
}

/// The models a server lists, each with the context it says it runs with (or [`DEFAULT_CONTEXT`]).
pub fn probe(base: &str, key: Option<&str>, timeout: Duration) -> Result<Vec<Value>, String> {
    let body = get(&format!("{base}/models"), key, timeout)?;
    let rows = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.get("models").and_then(Value::as_array))
        .cloned()
        .ok_or_else(|| "the server answered without a model list".to_string())?;
    /* llama.cpp says the context it was started with on /props, not in its model list. */
    let served_ctx = base
        .strip_suffix("/v1")
        .and_then(|root| get(&format!("{root}/props"), key, timeout).ok())
        .and_then(|props| props.pointer("/default_generation_settings/n_ctx").and_then(Value::as_u64));

    Ok(rows
        .iter()
        .filter_map(|row| {
            let id = row.get("id").or_else(|| row.get("name")).and_then(Value::as_str)?.to_string();
            let ctx = context_of(row).or(served_ctx).unwrap_or(DEFAULT_CONTEXT);

            Some(json!({ "id": id, "ctx": ctx }))
        })
        .collect())
}

/// The context a model row says it runs with: vLLM's `max_model_len`, LM Studio's `loaded_context_length` /
/// `max_context_length`, OpenRouter-style `context_length`, llama.cpp's `meta.n_ctx_train`.
pub fn context_of(row: &Value) -> Option<u64> {
    ["max_model_len", "loaded_context_length", "context_length", "max_context_length", "context_window"]
        .iter()
        .find_map(|key| row.get(*key).and_then(Value::as_u64))
        .or_else(|| row.pointer("/meta/n_ctx_train").and_then(Value::as_u64))
        .filter(|ctx| *ctx >= 512)
}

/// The well-known ports on this computer, asked at once: what answers like an OpenAI-compatible server.
pub fn discover() -> Value {
    let known: Vec<String> = saved().iter().filter_map(|server| server["base"].as_str().map(str::to_string)).collect();
    let found: Vec<Value> = std::thread::scope(|scope| {
        let asking: Vec<_> = KNOWN
            .iter()
            .map(|(port, label)| {
                let known = &known;

                scope.spawn(move || {
                    let base = format!("http://127.0.0.1:{port}/v1");

                    /* A closed port is the common case: a TCP connect that fails fast, before any HTTP. */
                    std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().ok()?, Duration::from_millis(250)).ok()?;

                    let models = probe(&base, None, Duration::from_millis(800));
                    let (models, note) = match models {
                        Ok(models) => (models, None),
                        Err(reason) if reason.contains("wants a key") => (Vec::new(), Some(reason)),
                        Err(_) => return None,
                    };

                    Some(json!({
                        "id": id_for(&base),
                        "label": label,
                        "base": base,
                        "models": models,
                        "connected": known.contains(&base),
                        "note": note,
                    }))
                })
            })
            .collect();

        asking.into_iter().filter_map(|handle| handle.join().ok().flatten()).collect()
    });

    json!({ "servers": found })
}

/// Connects a server: checks it answers, keeps it, and makes it a provider the model menu lists.
pub fn add(input: &str, label: Option<&str>, key: Option<&str>) -> Result<Value, ErrorObject> {
    let base = clean_base(input).map_err(ErrorObject::bad_request)?;
    let key = key.map(str::trim).filter(|key| !key.is_empty());
    let models = probe(&base, key, Duration::from_secs(4)).map_err(ErrorObject::bad_request)?;

    if models.is_empty() {
        return Err(ErrorObject::bad_request("The server answered, but it has no model loaded. Load one in it, then connect again."));
    }

    let id = id_for(&base);
    let label = label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let port: Option<u16> = base.split(':').nth(2).and_then(|rest| rest.split('/').next()).and_then(|port| port.parse().ok());

            KNOWN.iter().find(|(known, _)| Some(*known) == port).map(|(_, name)| name.to_string())
        })
        .unwrap_or_else(|| "Local server".to_string());

    if let Some(key) = key {
        crate::auth::keychain::set(&crate::providers::key_ref(&id), key)?;
    }

    let mut servers: Vec<Value> = saved().into_iter().filter(|server| server["id"].as_str() != Some(id.as_str())).collect();

    servers.push(json!({ "id": id, "label": label, "base": base, "models": models }));
    save_all(&servers).map_err(ErrorObject::internal)?;

    Ok(json!({ "id": id, "label": label, "base": base, "models": models }))
}

/// Forgets a server (and its key).
pub fn remove(id: &str) -> Result<Value, ErrorObject> {
    let servers: Vec<Value> = saved().into_iter().filter(|server| server["id"].as_str() != Some(id)).collect();

    save_all(&servers).map_err(ErrorObject::internal)?;
    let _ = crate::auth::keychain::delete(&crate::providers::key_ref(id));

    Ok(json!({ "removed": id }))
}

/// The provider blocks the catalogue adds for the connected servers - their live list at `<base>/models`.
pub fn blocks() -> Vec<crate::providers::models::ProviderBlock> {
    saved()
        .into_iter()
        .filter_map(|server| {
            Some(crate::providers::models::ProviderBlock {
                id: server["id"].as_str()?.to_string(),
                label: server["label"].as_str().unwrap_or("Local server").to_string(),
                live: format!("{}/models", server["base"].as_str()?),
                protocol: "openai".to_string(),
                models: server["models"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|mut row| {
                        row["tier"] = json!("balanced");
                        row["cost"] = json!("free · local");
                        row
                    })
                    .collect(),
            })
        })
        .collect()
}

/// The rows `provider.list` adds: one card per connected server.
pub fn provider_rows() -> Vec<Value> {
    saved()
        .into_iter()
        .map(|server| {
            let models = server["models"].as_array().map(Vec::len).unwrap_or(0);

            json!({
                "id": server["id"],
                "name": server["label"],
                "kind": "local",
                "status": "connected",
                "detail": format!("{} · {models} model{}", server["base"].as_str().unwrap_or_default(), if models == 1 { "" } else { "s" }),
                "account": Value::Null,
                "logo": "local",
                "initial": "L",
                "url": server["base"],
                "protocol": "openai",
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_cleaned_and_only_this_computer_the_network_or_https_is_taken() {
        assert_eq!(clean_base("127.0.0.1:1234").unwrap(), "http://127.0.0.1:1234/v1");
        assert_eq!(clean_base("http://localhost:8080/v1/chat/completions").unwrap(), "http://localhost:8080/v1");
        assert_eq!(clean_base("http://192.168.1.20:8000/v1/models/").unwrap(), "http://192.168.1.20:8000/v1");
        assert_eq!(clean_base("http://10.0.0.5:5000/v1").unwrap(), "http://10.0.0.5:5000/v1");
        assert_eq!(clean_base("http://gpu-box.local:8000").unwrap(), "http://gpu-box.local:8000/v1");
        assert_eq!(clean_base("https://my.server.example/api/v1").unwrap(), "https://my.server.example/api/v1");
        assert!(clean_base("http://203.0.113.9:8000/v1").is_err(), "plain http to the internet would send prompts in the clear");
        assert!(clean_base("").is_err());
    }

    #[test]
    fn ids_are_stable_per_address_and_marked_local() {
        assert_eq!(id_for("http://127.0.0.1:1234/v1"), "local-127-0-0-1-1234");
        assert!(is_local(&id_for("http://192.168.1.20:8000/v1")));
        assert!(!is_local("qwen"));
    }

    #[test]
    fn the_context_is_read_from_whichever_field_the_server_uses() {
        assert_eq!(context_of(&json!({ "id": "m", "max_model_len": 32768 })), Some(32768));
        assert_eq!(context_of(&json!({ "id": "m", "loaded_context_length": 4096 })), Some(4096));
        assert_eq!(context_of(&json!({ "id": "m", "meta": { "n_ctx_train": 131072 } })), Some(131072));
        assert_eq!(context_of(&json!({ "id": "m" })), None);
    }

    /// A real OpenAI-compatible server on loopback (one thread, two answers): found, listed, its context read.
    #[test]
    fn a_server_on_loopback_is_probed_with_its_models_and_context() {
        use std::io::{BufRead, BufReader, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let mut line = String::new();

                BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();

                let body = if line.contains("/v1/models") {
                    json!({ "object": "list", "data": [{ "id": "qwen2.5-coder-7b", "object": "model" }] }).to_string()
                } else {
                    json!({ "default_generation_settings": { "n_ctx": 16384 } }).to_string()
                };

                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });

        let models = probe(&format!("http://127.0.0.1:{port}/v1"), None, Duration::from_secs(2)).unwrap();

        assert_eq!(models, vec![json!({ "id": "qwen2.5-coder-7b", "ctx": 16384 })]);
    }
}
