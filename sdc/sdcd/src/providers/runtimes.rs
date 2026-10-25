//! **Local runtime detection (Phase 2)**: what is running on *this* computer that can serve a model - Ollama, LM Studio,
//! llama.cpp's server, llamafile, LocalAI, vLLM, SGLang, Jan, KoboldCpp, text-generation-webui, GPT4All, or anything else that
//! speaks OpenAI's API on a known port.
//!
//! The rules, all of them about being calm:
//!
//! * a scan **never starts anything** (it does not run `ollama serve`), never writes outside `local-servers.json`, and never
//!   changes the model the person chose - a runtime that is not there is simply not listed;
//! * every candidate is asked in parallel with short timeouts, so the whole scan is bounded (a closed port costs nothing, a silent
//!   server costs `read` milliseconds and is reported as `silent`, not as an error);
//! * nothing found is an ordinary answer (`none: true`) with an install hint, never an error;
//! * OpenAI-compatible servers found are connected as providers (marked `auto`) so their models show in the model menu labelled
//!   local, and an `auto` server that has since gone away is forgotten again; a server the person added by hand is never touched;
//! * Ollama's models are measured in the background by the existing probe (`sme::probe::after_use`, switch `models.probe`).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::local;

/// Where to look and how long to wait.
#[derive(Clone)]
pub struct Config {
    pub host: String,
    pub ollama_port: u16,
    pub ports: Vec<(u16, &'static str)>,
    pub connect: Duration,
    pub read: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self { host: "127.0.0.1".into(), ollama_port: 11434, ports: local::KNOWN.to_vec(), connect: Duration::from_millis(250), read: Duration::from_millis(1200) }
    }
}

fn connect(host: &str, port: u16, timeout: Duration) -> Option<TcpStream> {
    let address: SocketAddr = format!("{host}:{port}").parse().ok()?;

    TcpStream::connect_timeout(&address, timeout).ok()
}

/// A bounded `GET` over plain HTTP to a loopback port: the body, or why not.
fn get(host: &str, port: u16, path: &str, config: &Config) -> Result<String, &'static str> {
    let mut socket = connect(host, port, config.connect).ok_or("closed")?;

    socket.set_read_timeout(Some(config.read)).ok();
    socket.set_write_timeout(Some(config.read)).ok();
    socket.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n").as_bytes()).map_err(|_| "silent")?;

    let started = Instant::now();
    let mut response = Vec::new();
    let mut buffer = [0u8; 8192];

    loop {
        match socket.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buffer[..n]),
            Err(_) => break,
        }

        if response.len() > 4_000_000 || started.elapsed() > config.read * 2 {
            break;
        }
    }

    if response.is_empty() {
        return Err("silent");
    }

    let text = String::from_utf8_lossy(&response).to_string();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("silent")?;

    if head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200") {
        Ok(body.to_string())
    } else if head.contains(" 401 ") || head.contains(" 403 ") {
        Err("needs-key")
    } else {
        Err("other")
    }
}

fn ollama(config: &Config) -> Option<Value> {
    match get(&config.host, config.ollama_port, "/api/tags", config) {
        Ok(body) => {
            let tags: Value = serde_json::from_str(body.trim()).ok()?;
            let models: Vec<Value> = tags["models"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|model| {
                    let name = model["name"].as_str()?;
                    let probe = crate::sme::probe::latest("ollama", name).map(|profile| crate::sme::probe::row(&profile));

                    Some(json!({
                        "id": name,
                        "ctx": crate::engines::ollama::context_for(name),
                        "sizeBytes": model["size"],
                        "probe": probe,
                        "profileLine": crate::padma::model::row_summary(name),
                        "local": true,
                    }))
                })
                .collect();

            Some(json!({
                "id": "ollama",
                "kind": "ollama",
                "label": "Ollama",
                "base": format!("http://{}:{}", config.host, config.ollama_port),
                "models": models,
                "state": if models.is_empty() { "empty" } else { "ready" },
            }))
        }
        Err("silent") => Some(json!({ "id": "ollama", "kind": "ollama", "label": "Ollama", "base": format!("http://{}:{}", config.host, config.ollama_port), "models": [], "state": "silent" })),
        Err(_) => None,
    }
}

fn openai_compatible(port: u16, label: &str, config: &Config) -> Option<Value> {
    connect(&config.host, port, config.connect)?;

    let base = format!("http://{}:{port}/v1", config.host);

    match local::probe(&base, None, config.read) {
        Ok(models) if !models.is_empty() => Some(json!({
            "id": local::id_for(&base),
            "kind": "openai-compatible",
            "label": label,
            "base": base,
            "models": models.iter().map(|model| json!({ "id": model["id"], "ctx": model["ctx"], "local": true })).collect::<Vec<_>>(),
            "state": "ready",
        })),
        Ok(_) => Some(json!({ "id": local::id_for(&base), "kind": "openai-compatible", "label": label, "base": base, "models": [], "state": "empty" })),
        Err(reason) if reason.contains("wants a key") => Some(json!({ "id": local::id_for(&base), "kind": "openai-compatible", "label": label, "base": base, "models": [], "state": "needs-key" })),
        /* An open port that does not speak this API (a web server on 8080) is not a model runtime. A silent one might be a
           model that is still loading: reported quietly. */
        Err(reason) if reason.contains("not a model list") || reason.contains("HTTP") => None,
        Err(_) => Some(json!({ "id": local::id_for(&base), "kind": "openai-compatible", "label": label, "base": base, "models": [], "state": "silent" })),
    }
}

/// How to get a runtime, for the calm "nothing found" state.
pub fn install_hint() -> &'static str {
    "No local model runtime found. That is fine - cloud models work as they are. If you want a model on this computer, Ollama (ollama.com) or LM Studio (lmstudio.ai) are the easiest to start with."
}

/// Looks at every candidate at once. Takes at most about `connect + 2 x read`, whatever is (not) there.
pub fn scan_with(config: &Config) -> Value {
    let started = Instant::now();
    let mut found: Vec<Value> = std::thread::scope(|scope| {
        let mut asking = vec![scope.spawn(|| ollama(config))];

        for (port, label) in &config.ports {
            asking.push(scope.spawn(move || openai_compatible(*port, label, config)));
        }

        asking.into_iter().filter_map(|handle| handle.join().ok().flatten()).collect()
    });

    /* One entry per address: two labels for one port (8080) are one server. */
    found.dedup_by(|a, b| a["base"] == b["base"]);

    let none = found.iter().all(|runtime| runtime["state"] == "silent" || runtime["state"] == "empty") && found.iter().all(|runtime| runtime["models"].as_array().is_none_or(Vec::is_empty));

    json!({
        "runtimes": found,
        "none": found.is_empty(),
        "noModels": none,
        "installHint": if found.is_empty() { json!(install_hint()) } else { Value::Null },
        "scannedAt": chrono::Utc::now().to_rfc3339(),
        "ms": started.elapsed().as_millis() as u64,
    })
}

/// The scan the app asks for: [`scan_with`] on this computer's default ports, then the follow-through - OpenAI-compatible
/// servers found are connected (so their models reach the menu), `auto` ones that vanished are forgotten, and Ollama's
/// unmeasured models are probed in the background. Answers the scan plus `changed` (whether the provider list moved).
pub fn scan() -> Value {
    let mut answer = scan_with(&Config::default());
    let changed = reconcile(&answer);

    for runtime in answer["runtimes"].as_array().into_iter().flatten().filter(|runtime| runtime["kind"] == "ollama") {
        for model in runtime["models"].as_array().into_iter().flatten().filter(|model| model["probe"].is_null()).take(3) {
            if let Some(name) = model["id"].as_str() {
                crate::sme::probe::after_use(name);
            }
        }
    }

    answer["changed"] = json!(changed);
    answer
}

/// Connects what was found and forgets what the scan itself had connected and is gone. Never touches a server added by hand.
pub fn reconcile(scan: &Value) -> bool {
    let mut changed = false;
    let found: Vec<&Value> = scan["runtimes"].as_array().into_iter().flatten().filter(|runtime| runtime["kind"] == "openai-compatible").collect();
    let saved = local::saved();

    for runtime in &found {
        let Some(base) = runtime["base"].as_str() else {
            continue;
        };

        if runtime["state"] == "ready" && !saved.iter().any(|server| server["base"].as_str() == Some(base)) {
            if let Ok(added) = local::add(base, runtime["label"].as_str(), None) {
                local::mark_auto(added["id"].as_str().unwrap_or_default(), true);
                changed = true;
            }
        }
    }

    for server in saved.iter().filter(|server| server["auto"] == true) {
        let id = server["id"].as_str().unwrap_or_default();
        let still_there = found.iter().any(|runtime| runtime["id"].as_str() == Some(id));

        if !still_there {
            let _ = local::remove(id);
            changed = true;
        }
    }

    changed
}

/// At startup: one scan on its own thread, so a model that is already running is in the menu before the person opens it.
pub fn spawn_startup_scan() {
    std::thread::spawn(|| {
        let _ = scan();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    fn fast() -> Config {
        Config { connect: Duration::from_millis(150), read: Duration::from_millis(400), ports: Vec::new(), ollama_port: 1, ..Config::default() }
    }

    fn reply(stream: &mut TcpStream, body: &str) {
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    }

    /// Imitates Ollama's /api/tags.
    fn mock_ollama(models: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for stream in listener.incoming().take(8).flatten() {
                let mut stream = stream;
                let mut line = String::new();

                /* the scan opens a bare connection first: a closed one is not a request */
                if BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }

                reply(&mut stream, if line.contains("/api/tags") { models } else { "{}" });
            }
        });

        port
    }

    /// Imitates LM Studio / llama.cpp: /v1/models (and /props).
    fn mock_openai(label: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for stream in listener.incoming().take(10).flatten() {
                let mut stream = stream;
                let mut line = String::new();

                if BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }

                reply(&mut stream, &if line.contains("/v1/models") { json!({ "data": [{ "id": label, "max_model_len": 8192 }] }).to_string() } else { "{}".to_string() });
            }
        });

        port
    }

    /// Accepts the connection and never answers.
    fn mock_silent() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            let mut held = Vec::new();

            for stream in listener.incoming().take(4).flatten() {
                held.push(stream);
                std::thread::sleep(Duration::from_secs(3));
            }
        });

        port
    }

    #[test]
    fn nothing_running_is_a_calm_answer_not_an_error() {
        let scan = scan_with(&fast());

        assert_eq!(scan["none"], true);
        assert_eq!(scan["runtimes"], json!([]));
        assert!(scan["installHint"].as_str().unwrap().contains("No local model runtime found"));
        assert!(scan["ms"].as_u64().unwrap() < 2000);
    }

    #[test]
    fn an_ollama_like_server_and_an_openai_like_server_are_found_with_their_models() {
        let ollama_port = mock_ollama(r#"{"models":[{"name":"qwen3.5:9b","size":6600000000},{"name":"llama3.2:3b","size":2000000000}]}"#);
        let lm_port = mock_openai("qwen2.5-coder-7b");
        let config = Config { ollama_port, ports: vec![(lm_port, "LM Studio")], ..fast() };
        let scan = scan_with(&config);
        let runtimes = scan["runtimes"].as_array().unwrap();

        assert_eq!(scan["none"], false);
        assert_eq!(runtimes.len(), 2, "{scan}");

        let ollama = runtimes.iter().find(|runtime| runtime["kind"] == "ollama").unwrap();

        assert_eq!(ollama["state"], "ready");
        assert_eq!(ollama["models"].as_array().unwrap().len(), 2);
        assert!(ollama["models"][0]["local"] == true);

        let lm = runtimes.iter().find(|runtime| runtime["label"] == "LM Studio").unwrap();

        assert_eq!(lm["models"][0], json!({ "id": "qwen2.5-coder-7b", "ctx": 8192, "local": true }));
    }

    #[test]
    fn a_silent_server_does_not_hold_the_scan_and_is_reported_quietly() {
        let silent = mock_silent();
        let lm_port = mock_openai("m");
        let config = Config { ports: vec![(silent, "Local server"), (lm_port, "LM Studio")], ..fast() };
        let started = Instant::now();
        let scan = scan_with(&config);
        let runtimes = scan["runtimes"].as_array().unwrap();

        assert!(started.elapsed() < Duration::from_secs(3), "bounded: {:?}", started.elapsed());
        assert!(runtimes.iter().any(|runtime| runtime["state"] == "silent"), "{scan}");
        assert!(runtimes.iter().any(|runtime| runtime["state"] == "ready"), "the healthy one is still found: {scan}");
        assert_eq!(scan["none"], false);
    }

    #[test]
    fn a_web_server_on_a_model_port_is_not_taken_for_a_runtime() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for stream in listener.incoming().take(3).flatten() {
                let mut stream = stream;
                let mut line = String::new();

                BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
                write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });

        let scan = scan_with(&Config { ports: vec![(port, "Local server")], ..fast() });

        assert_eq!(scan["none"], true, "{scan}");
    }
}
