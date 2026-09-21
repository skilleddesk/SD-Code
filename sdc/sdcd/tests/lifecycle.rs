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

/**
 * 0.7.6, end to end: `Open folder` binds a chat to a real directory, and everything downstream finds it.
 *
 * Three pieces live in three places, which is why this is asserted against the real binary:
 *
 *   `project.add`      writes the row and validates that the path is a folder;
 *   `session.open`     binds the chat to it, and the `SessionOpened` event carries the folder so the
 *                      window's chip is right on its first render;
 *   `session.list`     reports it back, which is what makes the chat's folder survive a reload - and
 *                      `git.status` / `fs.search` / the checkpoint paths then need only the session id,
 *                      which is the whole point of the daemon knowing where the chat lives.
 */
#[test]
fn a_folder_opened_on_a_chat_is_where_its_tools_look() {
    let directory = TempDir::new().expect("a temporary directory");
    let (mut child, port) = start(&["--idle-exit", "30"], &directory);

    /* The folder the person "picks": a real directory, with a file for the search below to find. */
    let project = directory.path().join("my-project");
    std::fs::create_dir_all(&project).expect("a project folder");
    std::fs::write(project.join("README.md"), "# hello from the folder\n").expect("a file to find");

    let root = project.display().to_string();
    let (added, _) = request_collect(port, "add-1", "project.add", serde_json::json!({ "root": root }));

    let project_id = added
        .pointer("/result/projectId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("project.add did not answer with a projectId: {added}"))
        .to_string();

    /* The name is the last segment, because the dialog picks a path and nobody wants to type a label. */
    assert_eq!(
        added.pointer("/result/name").and_then(serde_json::Value::as_str),
        Some("my-project")
    );

    /* Opening the same folder twice reuses the row: the button is safe to press again, which is what
       stops a person from ending up with four rows for one directory. */
    let (again, _) = request_collect(port, "add-2", "project.add", serde_json::json!({ "root": root }));

    assert_eq!(
        again.pointer("/result/projectId").and_then(serde_json::Value::as_str),
        Some(project_id.as_str()),
        "a second project.add for one folder must reuse its row"
    );

    /* A path that is not a folder is refused in words, rather than becoming a chat with no directory. */
    let (refused, _) = request_collect(
        port,
        "add-3",
        "project.add",
        serde_json::json!({ "root": project.join("README.md").display().to_string() }),
    );

    assert!(refused.get("error").is_some(), "a file is not a folder: {refused}");

    /* A **second** folder in the same second is a different project. This is the id-collision case: a
       folder `INSERT OR REPLACE`d over another folder would change the first one's root and drag its
       chats along, and `project.add` pushes no event, so an id taken from the event sequence would be
       the same number twice. */
    let other = directory.path().join("another-project");
    std::fs::create_dir_all(&other).expect("a second project folder");

    let (second, _) = request_collect(
        port,
        "add-4",
        "project.add",
        serde_json::json!({ "root": other.display().to_string() }),
    );

    assert_ne!(
        second.pointer("/result/projectId").and_then(serde_json::Value::as_str),
        Some(project_id.as_str()),
        "two folders must not share a project id: {second}"
    );

    /* And the first folder is still the first folder - not silently re-pointed at the second. */
    let (still, _) = request_collect(
        port,
        "add-5",
        "project.add",
        serde_json::json!({ "root": root }),
    );

    assert_eq!(
        still.pointer("/result/root").and_then(serde_json::Value::as_str),
        Some(root.as_str()),
        "the first folder kept its own row: {still}"
    );

    let (opened, notifications) = request_collect(
        port,
        "open-1",
        "session.open",
        serde_json::json!({ "hostId": "local", "projectId": project_id, "title": "my-project" }),
    );

    let session_id = opened
        .pointer("/result/sessionId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("session.open did not answer with a sessionId: {opened}"))
        .to_string();

    let opened_event = notifications
        .iter()
        .find(|notification| {
            notification.pointer("/event/type").and_then(serde_json::Value::as_str) == Some("SessionOpened")
        })
        .unwrap_or_else(|| panic!("no SessionOpened notification arrived; saw {notifications:?}"));

    assert_eq!(
        opened_event.pointer("/event/projectRoot").and_then(serde_json::Value::as_str),
        Some(root.as_str()),
        "SessionOpened must carry the folder the chat was opened on"
    );

    /* `session.list` is what a reloading window folds - the folder has to be in it. */
    let (listed, _) = request_collect(port, "list-1", "session.list", serde_json::json!({}));
    let empty = Vec::new();
    let hosts = listed
        .pointer("/result/hosts")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);

    let row = hosts
        .iter()
        .flat_map(|host| {
            host.get("sessions")
                .and_then(serde_json::Value::as_array)
                .unwrap_or(&empty)
        })
        .find(|session| {
            session.get("sessionId").and_then(serde_json::Value::as_str) == Some(session_id.as_str())
        })
        .unwrap_or_else(|| panic!("the new chat is not in session.list: {listed}"));

    assert_eq!(row.get("projectRoot").and_then(serde_json::Value::as_str), Some(root.as_str()));
    assert_eq!(row.get("projectId").and_then(serde_json::Value::as_str), Some(project_id.as_str()));

    /* And the tools that used to demand a `root` parameter now take the session's: this search is the
       *file in the folder*, found with nothing but a chat id. */
    let (found, _) = request_collect(
        port,
        "search-1",
        "fs.search",
        serde_json::json!({ "sessionId": session_id, "query": "hello from the folder" }),
    );

    assert!(
        found
            .pointer("/result/hits")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|hits| !hits.is_empty()),
        "fs.search with a sessionId must look in that chat's folder: {found}"
    );

    /* The window's file tree (0.7.7) lists the same folder, again from nothing but the session id, and it
       needs three things the plain name listing could not give it: which rows are folders, how big they
       are, and how many names the guard kept out. */
    std::fs::write(project.join(".env"), "SECRET=1\n").expect("a file the guard must hide");

    let (listed_dir, _) =
        request_collect(port, "dir-1", "fs.list", serde_json::json!({ "sessionId": session_id }));

    assert_eq!(
        listed_dir.pointer("/result/path").and_then(serde_json::Value::as_str),
        Some(root.as_str()),
        "fs.list must name the folder it listed: {listed_dir}"
    );
    assert_eq!(
        listed_dir.pointer("/result/hidden").and_then(serde_json::Value::as_u64),
        Some(1),
        "the guard hid `.env` and the answer must say so: {listed_dir}"
    );

    let empty = Vec::new();
    let entries = listed_dir
        .pointer("/result/entries")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);
    let readme = entries
        .iter()
        .find(|entry| entry.get("name").and_then(serde_json::Value::as_str) == Some("README.md"))
        .unwrap_or_else(|| panic!("README.md is not in the listing: {listed_dir}"));

    assert_eq!(readme.get("dir").and_then(serde_json::Value::as_bool), Some(false));
    /* The size is the file's own, not a guess: the tree shows it beside the name. */
    let expected_size = std::fs::metadata(project.join("README.md")).expect("the fixture file").len();

    assert_eq!(readme.get("size").and_then(serde_json::Value::as_u64), Some(expected_size));
    assert!(
        readme.get("path").and_then(serde_json::Value::as_str).is_some_and(|path| path.ends_with("README.md")),
        "each row carries its absolute path: {readme}"
    );
    assert!(
        !entries.iter().any(|entry| entry.get("name").and_then(serde_json::Value::as_str) == Some(".env")),
        "a blocked name must not be listed at all: {listed_dir}"
    );

    /* A file bigger than the read cap comes back cut, and **says so** - with the hash still of the whole
       file, because a hash of the first megabyte is a hash of something that is not the file. */
    let big = project.join("big.log");
    std::fs::write(&big, "x".repeat(1_200_000)).expect("a file larger than the cap");

    let (read_big, _) = request_collect(
        port,
        "read-1",
        "fs.read",
        serde_json::json!({ "path": big.display().to_string() }),
    );

    assert_eq!(
        read_big.pointer("/result/truncated").and_then(serde_json::Value::as_bool),
        Some(true),
        "a 1.2 MB file must come back truncated: {read_big}"
    );
    assert_eq!(
        read_big.pointer("/result/bytes").and_then(serde_json::Value::as_u64),
        Some(1_200_000),
        "`bytes` is the file's real size, not the preview's: {read_big}"
    );
    assert_eq!(
        read_big.pointer("/result/text").and_then(serde_json::Value::as_str).map(str::len),
        Some(1024 * 1024),
        "the text is the first megabyte: {read_big}"
    );

    /* The small file is not cut, and says that too. */
    let (read_readme, _) = request_collect(
        port,
        "read-2",
        "fs.read",
        serde_json::json!({ "path": project.join("README.md").display().to_string() }),
    );

    assert_eq!(
        read_readme.pointer("/result/truncated").and_then(serde_json::Value::as_bool),
        Some(false),
        "README.md fits: {read_readme}"
    );

    let _ = request(port, "stop", "host.shutdown");
    let _ = wait_for_exit(&mut child, Duration::from_secs(10));
}

/**
 * 0.7.6: a chat that has **run a turn** can be deleted.
 *
 * `session.close` was `DELETE FROM sessions WHERE id = ?` while `foreign_keys` is ON and a turn references
 * its session, so deleting a chat that had been used answered `FOREIGN KEY constraint failed` - and the
 * window's Delete button showed exactly that sentence as a toast, with the chat still there. The 0.7.5 probe
 * never saw it: it deleted the chat it had just made, which had no turns. This is the same deletion for a
 * used chat, and it asserts the turn rows go with it rather than being orphaned.
 */
#[test]
fn a_chat_that_has_run_a_turn_can_be_deleted() {
    let directory = TempDir::new().expect("a temporary directory");
    let (mut child, port) = start(&["--idle-exit", "30"], &directory);

    /* A turn, so rows reference the chat. The engine does not have to exist: `engine.start` writes the turn
       row (and pushes `TurnStarted`) before it looks for the program. */
    let (started, _) = request_collect(
        port,
        "turn-1",
        "engine.start",
        serde_json::json!({
            "sessionId": "s-used",
            "prompt": "hello",
            "engine": "claude_code",
            "model": "sonnet",
            "tier": "Balanced",
        }),
    );

    assert!(started.get("result").is_some(), "engine.start refused the turn: {started}");

    let (closed, _) = request_collect(port, "close-1", "session.close", serde_json::json!({ "sessionId": "s-used" }));

    assert!(
        closed.get("error").is_none(),
        "closing a chat that had run a turn failed: {closed}"
    );

    /* And the chat is gone from the list - not merely reported closed. */
    let (listed, _) = request_collect(port, "list-1", "session.list", serde_json::json!({}));
    let empty = Vec::new();
    let hosts = listed
        .pointer("/result/hosts")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);

    let gone = hosts
        .iter()
        .flat_map(|host| {
            host.get("sessions")
                .and_then(serde_json::Value::as_array)
                .unwrap_or(&empty)
        })
        .all(|session| session.get("sessionId").and_then(serde_json::Value::as_str) != Some("s-used"));

    assert!(gone, "the deleted chat is still in session.list: {listed}");

    let _ = request(port, "stop", "host.shutdown");
    let _ = wait_for_exit(&mut child, Duration::from_secs(10));
}

/**
 * 0.7.8: `session.fork` - the method `protocol/types.ts` has declared since it was written, and the daemon
 * answered `unknown method` for.
 *
 * A fork is a new chat with the same conversation up to a turn. Three things are asserted, and the third is
 * the one that matters to a *window*:
 *
 *   1. the answer names the new chat, how many turns came with it, and the derived title;
 *   2. `atTurn` is inclusive, so forking at turn 1 of a two-turn chat copies one turn;
 *   3. the copied turns arrive as **events** (`TurnStarted` / `TurnCompleted`) - the window's transcript is
 *      the log, so a fork whose history lived only in the database would look like an empty chat.
 */
#[test]
fn a_forked_chat_carries_the_conversation_that_came_before_it() {
    let directory = TempDir::new().expect("a temporary directory");
    let (mut child, port) = start(&["--idle-exit", "30"], &directory);

    /* Two turns, so `atTurn` has something to stop at. The engine is not installed, which does not matter:
       the turn rows are written before the program is looked for. */
    for index in 1..=2 {
        let (started, _) = request_collect(
            port,
            &format!("turn-{index}"),
            "engine.start",
            serde_json::json!({
                "sessionId": "s-parent",
                "prompt": format!("question {index}"),
                "engine": "claude_code",
                "model": "sonnet",
                "tier": "Balanced",
            }),
        );

        assert!(started.get("result").is_some(), "engine.start refused turn {index}: {started}");
    }

    /* Forked at turn 1: the branch point is inclusive, so exactly one turn should come with it. */
    let (forked, notifications) = request_collect(
        port,
        "fork-1",
        "session.fork",
        serde_json::json!({ "sessionId": "s-parent", "atTurn": 1 }),
    );

    let fork_id = forked
        .pointer("/result/sessionId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("session.fork did not answer with a sessionId: {forked}"))
        .to_string();

    assert_eq!(forked.pointer("/result/turns").and_then(serde_json::Value::as_u64), Some(1));

    let title = forked
        .pointer("/result/title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    assert!(title.ends_with("(fork)"), "the fork's title must say where it came from: {title}");

    /* The fork is announced, and its one turn is replayed into the log. */
    let opened = notifications
        .iter()
        .find(|notification| {
            notification.pointer("/event/type").and_then(serde_json::Value::as_str) == Some("SessionOpened")
                && notification.pointer("/event/sessionId").and_then(serde_json::Value::as_str)
                    == Some(fork_id.as_str())
        })
        .unwrap_or_else(|| panic!("no SessionOpened for the fork; saw {notifications:?}"));

    assert_eq!(opened.pointer("/event/title").and_then(serde_json::Value::as_str), Some(title));

    let replayed = notifications
        .iter()
        .filter(|notification| {
            notification.pointer("/event/sessionId").and_then(serde_json::Value::as_str)
                == Some(fork_id.as_str())
                && notification.pointer("/event/type").and_then(serde_json::Value::as_str)
                    == Some("TurnStarted")
        })
        .count();

    assert_eq!(replayed, 1, "the fork's turn must be replayed into the log: {notifications:?}");

    /* And the whole conversation forks when `atTurn` is left out. */
    let (whole, _) = request_collect(port, "fork-2", "session.fork", serde_json::json!({ "sessionId": "s-parent" }));

    assert_eq!(
        whole.pointer("/result/turns").and_then(serde_json::Value::as_u64),
        Some(2),
        "a fork with no `atTurn` carries the whole conversation: {whole}"
    );

    /* Both chats are in the list, and the parent was not touched. */
    let (listed, _) = request_collect(port, "list-1", "session.list", serde_json::json!({}));
    let empty = Vec::new();
    let ids: Vec<&str> = listed
        .pointer("/result/hosts")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .flat_map(|host| {
            host.get("sessions")
                .and_then(serde_json::Value::as_array)
                .unwrap_or(&empty)
        })
        .filter_map(|session| session.get("sessionId").and_then(serde_json::Value::as_str))
        .collect();

    assert!(ids.contains(&"s-parent"), "the parent is gone after a fork: {listed}");
    assert!(ids.contains(&fork_id.as_str()), "the fork is not in the list: {listed}");

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

