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

/// Like `request`, but with params, and it keeps every notification as well.
///
/// `engine.start` answers *and* pushes: the answer is the `turnId`, and the notifications are the turn.
/// A test that only looked at the answer would never see what the log was told.
///
/// **It does not assume an order.** The first version of this helper returned as soon as it saw the
/// response, which meant it only ever collected notifications that happened to arrive first - true on
/// one machine, false on three CI runners, and a test that fails on a race teaches nothing. So it reads
/// until it has both the response and a `TurnStarted`, or until nothing more arrives.
fn request_collect(
    port: u16,
    id: &str,
    method: &str,
    params: serde_json::Value,
) -> (serde_json::Value, Vec<serde_json::Value>) {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connecting to sdcd");

    stream.set_read_timeout(Some(Duration::from_secs(3))).expect("a read timeout");

    let mut stream = stream;
    let mut seen = Vec::new();
    let mut answer: Option<serde_json::Value> = None;

    writeln!(
        stream,
        "{}",
        serde_json::json!({ "v": "0.1", "id": id, "method": method, "params": params })
    )
    .expect("writing the request");
    stream.flush().expect("flushing the request");

    let mut reader = BufReader::new(stream);
    let deadline = Instant::now() + Duration::from_secs(20);

    while Instant::now() < deadline {
        let mut line = String::new();

        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            /* A read timeout: nothing more is coming, which is the end of the interesting part. */
            Err(_) => break,
        }

        let Ok(message) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };

        let is_turn_started = message
            .pointer("/event/type")
            .and_then(serde_json::Value::as_str)
            == Some("TurnStarted");

        if message.get("id").and_then(serde_json::Value::as_str) == Some(id) {
            answer = Some(message);
        } else {
            seen.push(message);
        }

        if answer.is_some() && is_turn_started {
            break;
        }
    }

    (answer.expect("sdcd never answered; see the notifications collected"), seen)
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

/// The two halves of a host's life, measured on a real daemon over a socket.
///
/// Both halves were missing, and both were visible only from outside the daemon:
///
/// * adding the same `user@host` twice made a second row that the sidebar could not tell apart from
///   the first - `Website, Website, Website`, with no way to know which one was which;
/// * `host.remove` was declared in `protocol/types.ts` and answered `unknown method`, so a host
///   added by mistake was permanent;
/// * and `session.open` rewrote the host's row with the literals `Local` / `local` / `connected`, so
///   opening a chat on a VPS erased its name, its kind and its `target` - the very field the
///   duplicate check above keys on.
///
/// The `ssh` probe that `host.add` now starts is deliberately **not** asserted on: whether this
/// runner can reach the internet is not a property of the daemon, and a test that depends on it
/// would be a test that fails on a plane.
#[test]
fn a_host_is_added_once_and_can_be_removed_again() {
    let directory = TempDir::new().expect("a temporary directory");
    let (mut child, port) = start(&[], &directory);

    let (first, _) = request_collect(
        port,
        "add-1",
        "host.add",
        serde_json::json!({ "type": "ssh", "target": "root@vps.example", "label": "VPS" }),
    );

    let host_id = first
        .pointer("/result/hostId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();

    assert!(!host_id.is_empty(), "host.add answered no id: {first}");
    assert_eq!(
        first.pointer("/result/reused").and_then(serde_json::Value::as_bool),
        Some(false),
        "a fresh host must not report itself as a reuse: {first}"
    );

    let (second, _) = request_collect(
        port,
        "add-2",
        "host.add",
        serde_json::json!({ "type": "ssh", "target": "root@vps.example", "label": "VPS again" }),
    );

    assert_eq!(
        second.pointer("/result/hostId").and_then(serde_json::Value::as_str),
        Some(host_id.as_str()),
        "the same `user@host` was added as a second host: {second}"
    );
    assert_eq!(
        second.pointer("/result/reused").and_then(serde_json::Value::as_bool),
        Some(true),
        "the second add must say it reused the row: {second}"
    );

    /* A session on it, so the removal has something to take with it. */
    let (session, _) = request_collect(
        port,
        "open-1",
        "session.open",
        serde_json::json!({ "hostId": host_id, "title": "On the VPS" }),
    );

    assert!(session.get("result").is_some(), "session.open failed: {session}");

    let (listed, _) = request_collect(port, "list-1", "session.list", serde_json::json!({}));
    let hosts = listed
        .pointer("/result/hosts")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let added = hosts.iter().find(|host| host["hostId"] == serde_json::json!(host_id));

    assert!(added.is_some(), "session.list does not carry the added host: {listed}");
    assert_eq!(
        added.and_then(|host| host["sessions"].as_array()).map(Vec::len),
        Some(1),
        "session.list dropped the host's sessions: {listed}"
    );
    assert_eq!(
        added.and_then(|host| host["target"].as_str()),
        Some("root@vps.example"),
        "session.open overwrote the host's own row: {listed}"
    );

    let (removed, _) = request_collect(
        port,
        "remove-1",
        "host.remove",
        serde_json::json!({ "hostId": host_id }),
    );

    assert_eq!(
        removed.pointer("/result/removed").and_then(serde_json::Value::as_bool),
        Some(true),
        "host.remove did not remove the host: {removed}"
    );
    assert_eq!(
        removed.pointer("/result/sessions").and_then(serde_json::Value::as_i64),
        Some(1),
        "host.remove did not report the session it took with it: {removed}"
    );

    /* The row is gone from the list a restarting window folds. */
    let (after, _) = request_collect(port, "list-2", "session.list", serde_json::json!({}));
    let remaining = after
        .pointer("/result/hosts")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    assert!(
        !remaining.iter().any(|host| host["hostId"] == serde_json::json!(host_id)),
        "host.remove left the host behind: {after}"
    );

    /* `local` is the machine the daemon runs on, and refusing it is a sentence rather than a crash. */
    let (refused, _) = request_collect(
        port,
        "remove-local",
        "host.remove",
        serde_json::json!({ "hostId": "local" }),
    );

    assert_eq!(
        refused.pointer("/error/code").and_then(serde_json::Value::as_str),
        Some("bad_request"),
        "removing `local` must be refused with a reason: {refused}"
    );

    let _ = child.kill();
    let _ = child.wait();
}

