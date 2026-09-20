//! The daemon's lifecycle test: the part of `sdcd` a user meets before any feature.
//!
//! An installed app starts the daemon it ships with and has to be able to stop it again. When that is
//! not true the failure is invisible and sticky: the process keeps port 7811 after the app is gone, and
//! the *next* install talks to the previous release's daemon - every method answering `unknown method`,
//! with nothing in the UI to say why. So the two halves are asserted here against the real binary
//! rather than argued about in a comment:
//!
//! * a client can end the daemon (`host.shutdown`), and the process really exits;
//! * a daemon started for the app leaves on its own once nobody is connected (`--idle-exit`);
//! * a daemon started by hand, with no `--idle-exit`, never leaves on its own.
//!
//! `CARGO_BIN_EXE_sdcd` is the binary under test - the same one the app spawns.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// A free loopback port. Bound and released: the daemon takes it a moment later, which is soon enough
/// for a test and avoids hard-coding a number another test could be using.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binding a probe port");

    listener.local_addr().expect("the probe port").port()
}

/// Starts the daemon with a database and a runtime directory of its own, and waits until it answers.
///
/// The environment is as important as the arguments: the socket path is derived from
/// `XDG_RUNTIME_DIR` (or the temp directory), so a test that inherited the runner's would share a
/// socket *and* a data directory with anything else on the machine. Each test gets its own.
///
/// The returned `Child` is the caller's to wait for: `#[allow(clippy::zombie_processes)]` is here
/// because that hand-over is exactly what the lint cannot see through, and the failure path below
/// kills and reaps the process itself so a test that cannot start a daemon leaves nothing running.
#[allow(clippy::zombie_processes)]
fn start(arguments: &[&str], directory: &TempDir) -> (Child, u16) {
    let port = free_port();
    let database: PathBuf = directory.path().join("sdc.db");
    let runtime: PathBuf = directory.path().join("run");

    std::fs::create_dir_all(&runtime).expect("a runtime directory for the daemon");

    let child = Command::new(env!("CARGO_BIN_EXE_sdcd"))
        .arg("--port")
        .arg(port.to_string())
        .arg("--database")
        .arg(&database)
        .args(arguments)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("TMPDIR", &runtime)
        .env("TEMP", &runtime)
        .env("TMP", &runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("starting sdcd");

    let deadline = Instant::now() + Duration::from_secs(30);

    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (child, port);
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    let mut child = child;

    let _ = child.kill();
    let _ = child.wait();

    panic!("sdcd never listened on 127.0.0.1:{port}");
}

/// One request on its own connection: what the app's bridge does for a probe, in miniature.
fn request(port: u16, id: &str, method: &str) -> serde_json::Value {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connecting to sdcd");

    stream.set_read_timeout(Some(Duration::from_secs(10))).expect("a read timeout");

    let mut stream = stream;

    writeln!(stream, "{}", serde_json::json!({ "v": "0.1", "id": id, "method": method, "params": {} }))
        .expect("writing the request");
    stream.flush().expect("flushing the request");

    let mut reader = BufReader::new(stream);

    loop {
        let mut line = String::new();

        if reader.read_line(&mut line).expect("reading the answer") == 0 {
            panic!("sdcd closed the connection before answering {method}");
        }

        let Ok(message) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };

        if message.get("id").and_then(serde_json::Value::as_str) == Some(id) {
            return message;
        }
    }
}

/// Like `request`, but keeps every notification that arrived first and the params you send.
///
/// `engine.start` answers *and* pushes: the answer is the `turnId`, and the notifications are the
/// turn. A test that only looked at the answer would never see what the log was told.
fn request_collect(
    port: u16,
    id: &str,
    method: &str,
    params: serde_json::Value,
) -> (serde_json::Value, Vec<serde_json::Value>) {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connecting to sdcd");

    stream.set_read_timeout(Some(Duration::from_secs(10))).expect("a read timeout");

    let mut stream = stream;
    let mut seen = Vec::new();

    writeln!(
        stream,
        "{}",
        serde_json::json!({ "v": "0.1", "id": id, "method": method, "params": params })
    )
    .expect("writing the request");
    stream.flush().expect("flushing the request");

    let mut reader = BufReader::new(stream);

    loop {
        let mut line = String::new();

        if reader.read_line(&mut line).expect("reading the answer") == 0 {
            panic!("sdcd closed the connection before answering {method}");
        }

        let Ok(message) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };

        if message.get("id").and_then(serde_json::Value::as_str) == Some(id) {
            return (message, seen);
        }

        seen.push(message);
    }
}

#[test]
fn a_turn_carries_the_prompt_the_user_sent_and_no_invented_price() {
    let directory = TempDir::new().expect("a temporary directory");
    let (mut child, port) = start(&["--idle-exit", "30"], &directory);

    let (answer, seen) = request_collect(
        port,
        "turn-1",
        "engine.start",
        serde_json::json!({
            "sessionId": "s-user",
            "prompt": "add a test for the limiter",
            "engine": "claude_code",
            "model": "sonnet",
            "tier": "Balanced",
        }),
    );

    assert!(
        answer.get("result").is_some(),
        "engine.start refused the turn: {answer}"
    );

    let started = seen
        .iter()
        .find(|notification| {
            notification.pointer("/event/type").and_then(serde_json::Value::as_str)
                == Some("TurnStarted")
        })
        .unwrap_or_else(|| panic!("no TurnStarted notification arrived; saw {seen:?}"));

    /* The user's own words travel with the turn: the window draws the question from the log, so a
       reload does not lose half of the conversation. */
    assert_eq!(
        started.pointer("/event/prompt").and_then(serde_json::Value::as_str),
        Some("add a test for the limiter"),
        "TurnStarted must carry the prompt it is answering"
    );

    /* And nothing pretends to know what it will cost. The old payload carried a fixed
       `~$0.10 - $0.28 forecast`, which is a price for a turn nobody has measured. */
    assert!(
        started.pointer("/event/forecast").is_none(),
        "TurnStarted still carries a forecast"
    );

    let _ = request(port, "stop", "host.shutdown");
    let _ = wait_for_exit(&mut child, Duration::from_secs(10));
}

fn wait_for_exit(child: &mut Child, within: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + within;

    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("asking after the child") {
            return Some(status);
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    None
}

#[test]
fn a_client_can_end_the_daemon_and_the_process_exits() {
    let directory = TempDir::new().expect("a temporary directory");
    /* A long idle exit, so the only thing that can end this run is the request below. */
    let (mut child, port) = start(&["--idle-exit", "30"], &directory);

    let status = request(port, "probe-1", "host.status");
    let running = status.pointer("/result/sdcd").and_then(serde_json::Value::as_str);

    assert_eq!(running, Some(sdcd::VERSION), "the daemon must report its own version");

    let answer = request(port, "probe-2", "host.shutdown");

    assert_eq!(answer.pointer("/result/stopping").and_then(serde_json::Value::as_bool), Some(true));

    let exit = wait_for_exit(&mut child, Duration::from_secs(10));

    if exit.is_none() {
        let _ = child.kill();
    }

    assert!(exit.is_some(), "the daemon answered `stopping` but is still running");
}

#[test]
fn a_daemon_started_for_the_app_leaves_when_nobody_is_connected() {
    let directory = TempDir::new().expect("a temporary directory");
    let (mut child, _port) = start(&["--idle-exit", "1"], &directory);

    let exit = wait_for_exit(&mut child, Duration::from_secs(15));

    if exit.is_none() {
        let _ = child.kill();
    }

    assert!(exit.is_some(), "a daemon with --idle-exit 1 must leave on its own");
}

#[test]
fn a_daemon_started_by_hand_stays_where_it_is() {
    let directory = TempDir::new().expect("a temporary directory");
    let (mut child, port) = start(&[], &directory);

    /* No `--idle-exit`: a terminal the user opened is a terminal the user closes. Two `host.status`
       calls, because a client that *did* connect must not change that either. */
    assert!(request(port, "probe-3", "host.status").get("result").is_some());

    assert!(wait_for_exit(&mut child, Duration::from_secs(3)).is_none(), "the daemon left on its own");

    let _ = child.kill();
    let _ = child.wait();
}
