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

/// Starts the daemon with a database of its own and waits until it answers.
///
/// The returned `Child` is the caller's to wait for: `#[allow(clippy::zombie_processes)]` is here
/// because that hand-over is exactly what the lint cannot see through, and the failure path below
/// kills and reaps the process itself so a test that cannot start a daemon leaves nothing running.
#[allow(clippy::zombie_processes)]
fn start(arguments: &[&str], directory: &TempDir) -> (Child, u16) {
    let port = free_port();
    let database: PathBuf = directory.path().join("sdc.db");

    let child = Command::new(env!("CARGO_BIN_EXE_sdcd"))
        .arg("--port")
        .arg(port.to_string())
        .arg("--database")
        .arg(&database)
        .args(arguments)
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

/// Polls the child until it is gone, or the deadline passes.
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
