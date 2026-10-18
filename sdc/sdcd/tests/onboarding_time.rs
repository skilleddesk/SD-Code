//! Onboarding in five minutes (Phase 4.3): the part of "pick a folder, connect a model, first task, first Proof" that a machine can time.
//!
//! The real daemon binary is started on an empty data directory (a fresh install), a mock OpenAI-compatible server stands in for the model,
//! and every step the first-run wizard performs is called over the wire - add the folder, connect the model (`local.add`), open a chat, run
//! the first task, export the first Proof Pack - with the time each one took printed in a table (`cargo test --test onboarding_time --
//! --nocapture`). The assertion is the promise: the machine's share of onboarding is far under the five minutes. What is *not* measured here is
//! the person's own time - typing a key, reading a screen - which no script can measure honestly.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::TempDir;

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// An OpenAI-compatible server: `/v1/models`, `/props`, and a streamed chat answer.
fn mock_model() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();

                if reader.read_line(&mut first).unwrap_or(0) == 0 {
                    return;
                }

                let mut length = 0usize;

                loop {
                    let mut line = String::new();

                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }

                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }

                let mut body = vec![0u8; length];
                let _ = reader.read_exact(&mut body);

                let (kind, text) = if first.contains("/v1/models") {
                    ("application/json", json!({ "data": [{ "id": "mock-coder", "max_model_len": 16384 }] }).to_string())
                } else if first.contains("chat/completions") {
                    (
                        "text/event-stream",
                        format!(
                            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                            json!({ "choices": [{ "delta": { "content": "Hello from the first task." } }] }),
                            json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": 40, "completion_tokens": 6 } })
                        ),
                    )
                } else {
                    ("application/json", "{}".to_string())
                };

                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
            });
        }
    });

    port
}

struct Wire {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next: u32,
    events: Vec<Value>,
}

impl Wire {
    fn open(port: u16) -> Self {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();

        stream.set_read_timeout(Some(Duration::from_secs(90))).unwrap();

        Self { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream, next: 1, events: Vec::new() }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = format!("t{}", self.next);

        self.next += 1;
        writeln!(self.writer, "{}", json!({ "v": "0.1", "id": id, "method": method, "params": params })).unwrap();

        loop {
            let mut line = String::new();

            assert!(self.reader.read_line(&mut line).unwrap() > 0, "the daemon closed the connection during {method}");

            let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };

            if message["id"] == id {
                return message.get("error").filter(|error| !error.is_null()).map_or_else(|| Ok(message["result"].clone()), |error| Err(error.clone()));
            }

            self.events.push(message);
        }
    }

    /// Reads until an event of this type arrives (or the time is up).
    fn until(&mut self, kind: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;

        if self.events.iter().any(|event| event.to_string().contains(&format!("\"type\":\"{kind}\""))) {
            return true;
        }

        while Instant::now() < deadline {
            let mut line = String::new();

            if self.reader.read_line(&mut line).unwrap_or(0) == 0 {
                return false;
            }

            if line.contains(&format!("\"type\":\"{kind}\"")) {
                return true;
            }
        }

        false
    }
}

#[test]
fn a_fresh_install_gets_from_a_folder_to_its_first_proof_in_seconds() {
    let data = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let runtime = data.path().join("run");
    let port = free_port();
    let model_port = mock_model();
    let mut steps: Vec<(&str, Duration)> = Vec::new();
    let total = Instant::now();
    let mark = |steps: &mut Vec<(&str, Duration)>, name: &'static str, since: Instant| steps.push((name, since.elapsed()));

    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::write(project.path().join("README.md"), "# demo\n").unwrap();

    let started = Instant::now();
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_sdcd"))
            .arg("--port")
            .arg(port.to_string())
            .arg("--database")
            .arg(data.path().join("sdc.db"))
            .env("SDC_DATA_DIR", data.path().join("data"))
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("TMPDIR", &runtime)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(started.elapsed() < Duration::from_secs(30), "the daemon never listened");
        std::thread::sleep(Duration::from_millis(50));
    }

    mark(&mut steps, "the app starts its daemon", started);

    let mut wire = Wire::open(port);
    let step = Instant::now();

    wire.call("event.subscribe", json!({})).unwrap();

    let host = wire.call("host.status", json!({})).unwrap();

    assert!(host["version"].is_string() || host.is_object());
    mark(&mut steps, "host check", step);

    let step = Instant::now();
    let added = wire.call("project.add", json!({ "hostId": "local", "root": project.path().to_string_lossy() })).unwrap();
    let project_id = added["projectId"].as_str().unwrap().to_string();

    mark(&mut steps, "pick a folder", step);

    let step = Instant::now();
    let connected = wire.call("local.add", json!({ "url": format!("http://127.0.0.1:{model_port}/v1") })).unwrap();
    let provider = connected["id"].as_str().unwrap().to_string();

    assert_eq!(connected["models"][0]["id"], "mock-coder");
    mark(&mut steps, "connect a model", step);

    let step = Instant::now();
    let session = wire.call("session.open", json!({ "hostId": "local", "projectId": project_id, "title": "First task", "prompt": "hello" })).unwrap();
    let session_id = session["sessionId"].as_str().unwrap().to_string();
    let turn = wire.call("engine.start", json!({ "sessionId": session_id, "engine": "native_api", "provider": provider, "model": "mock-coder", "prompt": "say hello", "agent": false })).unwrap();

    assert!(turn["turnId"].is_string(), "{turn}");
    assert!(wire.until("TurnCompleted", Duration::from_secs(60)), "the first task never completed");
    mark(&mut steps, "first task", step);

    let step = Instant::now();
    let proof = wire.call("proof.export", json!({ "sessionId": session_id })).unwrap();

    assert!(proof["html"].as_str().is_some_and(|html| html.len() > 200), "the first Proof Pack is a page: {proof}");
    mark(&mut steps, "first Proof", step);

    let elapsed = total.elapsed();

    println!("\nonboarding, machine share (fresh data directory, mock model):");

    for (name, took) in &steps {
        println!("  {name:<28} {:>7.2} s", took.as_secs_f64());
    }

    println!("  {:<28} {:>7.2} s   (budget: 300 s)", "total", elapsed.as_secs_f64());

    assert!(elapsed < Duration::from_secs(300), "the machine share of onboarding took {elapsed:?}");
}
