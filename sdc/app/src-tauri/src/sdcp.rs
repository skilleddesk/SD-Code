//! The SDCP bridge - the Tauri half of the transport (master spec sections 5.3 and 5.4).
//!
//! The frontend never opens a socket. It calls three commands and listens for one event:
//!
//! | command            | what it does                                                       |
//! | ------------------ | ------------------------------------------------------------------ |
//! | `sdcp_status`      | whether the daemon is reachable, and on what address                |
//! | `sdcp_connect`     | connects, spawning the daemon first if it is not running            |
//! | `sdcp_call`        | one request, returns its result, keeps notifications for the reader  |
//! | `sdcp_stop_daemon` | stops a daemon this bridge started                                  |
//!
//! and the event `sdcp://event` carries every notification the daemon pushes - which is how a turn's
//! `TurnDelta` stream reaches the UI store while the `engine.start` call has already returned
//! (spec section 5.4, "the request answers, the events keep coming").
//!
//! ## Why loopback TCP and not a Windows named pipe
//!
//! The daemon already serves newline-delimited JSON over `127.0.0.1:7811` on every platform, and the
//! app needs exactly one client. A named pipe would be a second server implementation inside `sdcd`
//! (a Windows-only one, with its own framing) for no gain the user can see, so the bridge uses the
//! socket that exists and says so here rather than in a commit message.
//!
//! ## Who starts the daemon
//!
//! `sdcp_connect` tries the socket, and on a refused connection it looks for the `sdcd` binary next
//! to the app (a bundled sidecar) or in the development `target/` directories, starts it, and waits
//! for the port. A daemon that the user started by hand is never started twice and never stopped by
//! `sdcp_stop_daemon` - the bridge only stops what it started.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// The event name the frontend subscribes to (`app/src/lib/transport.ts`).
pub const EVENT_NAME: &str = "sdcp://event";

/// The daemon's default loopback port; `SDC_SDCP_PORT` overrides it.
pub const DEFAULT_PORT: u16 = 7811;

/// What `sdcp_status` answers with.
pub fn status_json(bridge: &SdcpBridge) -> Value {
    let daemon_version = bridge.daemon_version.try_lock().ok().and_then(|held| held.clone());

    json!({
        "transport": "loopback",
        "address": format!("127.0.0.1:{}", bridge.port()),
        "connected": bridge.connected.load(Ordering::SeqCst),
        "spawned": bridge.spawned.load(Ordering::SeqCst),
        "lastSeq": bridge.last_seq.load(Ordering::SeqCst),
        "appVersion": bridge.version,
        "daemonVersion": daemon_version,
        /* True when this bridge stopped a daemon of another version and started its own. The frontend
           says so once, so a version bump is never a silent surprise. */
        "restarted": bridge.restarted.swap(false, Ordering::SeqCst),
    })
}

/// The bridge's shared state.
pub struct SdcpBridge {
    port: u16,
    /// The app's own version. The daemon reports its version in `host.status`, and a mismatch is what
    /// tells the bridge that the process on the port belongs to an older install.
    version: String,
    /// The request connection. A `tokio::sync::Mutex` because a call awaits inside it: two calls in
    /// flight on one socket would interleave their responses.
    call: Mutex<Option<TcpStream>>,
    /// The daemon this bridge started, if it started one. Kept so the app can stop what it owns: a
    /// daemon the user started by hand is never touched (`stop_daemon`).
    child: StdMutex<Option<std::process::Child>>,
    next_id: AtomicI64,
    connected: AtomicBool,
    spawned: AtomicBool,
    last_seq: AtomicI64,
    /// The version of the daemon that answered, once one has. `None` until the first handshake.
    daemon_version: Mutex<Option<String>>,
    /// Set when a daemon of another version was asked to stop and replaced by the app's own.
    restarted: AtomicBool,
    /// Serializes `connect()`. Two calls can read `connected == false` at the same time - the app's
    /// first handshake and its first heartbeat do exactly that, since `App.tsx` starts both in the
    /// same tick - and the loser of that race used to open a **second notification socket**. Every
    /// event then reached the window twice, so every `SessionOpened` was folded twice and one click
    /// on `New chat` drew two rows. Holding this for the whole connect is what makes that impossible
    /// rather than unlikely.
    connecting: Mutex<()>,
    /// Whether the notification reader is running. One reader per bridge is the invariant: a second
    /// one is a second copy of everything the daemon pushes. Cleared by the reader itself when its
    /// socket ends, so a reconnect subscribes again.
    subscribed: AtomicBool,
}

impl SdcpBridge {
    pub fn new(version: impl Into<String>) -> Arc<Self> {
        let port = std::env::var("SDC_SDCP_PORT").ok().and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_PORT);

        Arc::new(Self {
            port,
            version: version.into(),
            call: Mutex::new(None),
            child: StdMutex::new(None),
            next_id: AtomicI64::new(1),
            connected: AtomicBool::new(false),
            spawned: AtomicBool::new(false),
            last_seq: AtomicI64::new(0),
            daemon_version: Mutex::new(None),
            restarted: AtomicBool::new(false),
            connecting: Mutex::new(()),
            subscribed: AtomicBool::new(false),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn address(&self) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port)
    }
}

/// Connects, spawning the daemon if nothing answers. Answers with `sdcp_status`'s shape.
///
/// A daemon that answers on the port but reports another version than the app is **replaced**: the
/// bridge asks it to stop (`host.shutdown`), waits for the port to close and starts the `sdcd` that
/// ships with this build. Without that step, an app update would silently talk to the previous
/// release's daemon - the one failure mode a user cannot diagnose, because every tool call simply
/// answers `unknown method`.
pub async fn connect(app: AppHandle, bridge: Arc<SdcpBridge>) -> Result<Value, String> {
    if bridge.connected.load(Ordering::SeqCst) {
        return Ok(status_json(&bridge));
    }

    /* One connect at a time. The check above is the fast path; this lock is what makes the slow path
       single: a second caller waits here, then finds the bridge connected and returns without opening
       anything. Without it, both callers reached the `subscribe()` at the bottom of this function. */
    let _connecting = bridge.connecting.lock().await;

    if bridge.connected.load(Ordering::SeqCst) {
        return Ok(status_json(&bridge));
    }

    if TcpStream::connect(bridge.address()).await.is_ok() {
        let running = probe_version(&bridge).await;

        match running.as_deref() {
            Some(version) if version != bridge.version => {
                replace_daemon(&bridge, version).await?;
            }
            _ => {}
        }
    }

    if TcpStream::connect(bridge.address()).await.is_err() {
        start_daemon(&bridge)?;

        /* A fresh daemon opens its database before it listens; give it a moment rather than failing
           on a race the user would see as "the app is broken". */
        let mut ready = false;

        for _ in 0..40 {
            if TcpStream::connect(bridge.address()).await.is_ok() {
                ready = true;
                break;
            }

            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        if !ready {
            return Err(format!("sdcd did not start listening on 127.0.0.1:{}", bridge.port));
        }
    }

    let stream = TcpStream::connect(bridge.address()).await.map_err(|error| error.to_string())?;

    *bridge.call.lock().await = Some(stream);
    bridge.connected.store(true, Ordering::SeqCst);

    /* The notification connection: a *second* socket, because a notification is not a response and
       must not be able to interleave with one. */
    subscribe(app, bridge.clone()).await?;

    Ok(status_json(&bridge))
}

/// What version the daemon on the port says it is, asked on a socket of its own.
///
/// A daemon that is too old to know `host.status` (none exists, but the shape is honest) answers with
/// an error and yields `None`, which is treated as "leave it alone": the bridge only replaces a daemon
/// it can name.
async fn probe_version(bridge: &SdcpBridge) -> Option<String> {
    let stream = TcpStream::connect(bridge.address()).await.ok()?;
    let mut stream = stream;
    let request = json!({ "v": "0.1", "id": "app-version-probe", "method": "host.status", "params": {} });

    stream.write_all(format!("{request}\n").as_bytes()).await.ok()?;
    stream.flush().await.ok()?;

    let mut reader = BufReader::new(stream);

    loop {
        let mut line = String::new();

        if reader.read_line(&mut line).await.ok()? == 0 {
            return None;
        }

        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };

        if message.get("id").and_then(Value::as_str) != Some("app-version-probe") {
            continue;
        }

        let version = message.pointer("/result/sdcd").and_then(Value::as_str).map(str::to_string);

        if let Some(found) = &version {
            if let Ok(mut held) = bridge.daemon_version.try_lock() {
                *held = Some(found.clone());
            }
        }

        return version;
    }
}

/// Stops a daemon of another version and starts this build's own, then waits for the port to answer
/// again. `host.shutdown` is what makes this possible over the wire: the daemon exits on a request,
/// which is the only lever a client with a socket has.
async fn replace_daemon(bridge: &SdcpBridge, version: &str) -> Result<(), String> {
    let mut stream = TcpStream::connect(bridge.address()).await.map_err(|error| error.to_string())?;
    let request = json!({ "v": "0.1", "id": "app-replace", "method": "host.shutdown", "params": {} });

    stream
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream.flush().await.map_err(|error| error.to_string())?;

    /* Read the answer before the daemon goes, so the log line below is about a daemon that said
       goodbye rather than one that was shot. */
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    let _ = reader.read_line(&mut line).await;
    eprintln!("sdcp: replaced sdcd {version} with {} (the app's own version)", bridge.version);

    for _ in 0..40 {
        if TcpStream::connect(bridge.address()).await.is_err() {
            bridge.restarted.store(true, Ordering::SeqCst);

            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    Err(format!(
        "sdcd {version} answered on 127.0.0.1:{} but did not stop, so this build's daemon could not start",
        bridge.port
    ))
}

/// One request, in the shape the frontend writes it: `{ method, params, id }`.
///
/// The answer is the daemon's **whole response envelope**, `id` included, because
/// `TauriTransport.accept` correlates it with the pending request - the bridge is a byte transport
/// and does not reshape what the daemon said.
pub async fn call(
    app: AppHandle,
    bridge: Arc<SdcpBridge>,
    method: String,
    params: Value,
    id: Option<String>,
) -> Result<Value, String> {
    /* Lazy connect: the frontend's first `sdcpCall` is what starts the daemon, so no UI flow has to
       remember to call a connect command first. */
    if !bridge.connected.load(Ordering::SeqCst) {
        connect(app, bridge.clone()).await?;
    }

    let id = id.unwrap_or_else(|| format!("app-{}", bridge.next_id.fetch_add(1, Ordering::SeqCst)));
    let envelope = json!({ "v": "0.1", "id": id, "method": method, "params": params });
    let mut guard = bridge.call.lock().await;
    let stream = guard.as_mut().ok_or_else(|| "the daemon connection was closed".to_string())?;

    stream
        .write_all(format!("{envelope}\n").as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream.flush().await.map_err(|error| error.to_string())?;

    /* Read until *this* id answers. Notifications that arrive first are skipped rather than dropped:
       the daemon may legitimately push an event before answering (`TurnStarted` can win the race
       against `engine.start`'s response), and the notification socket forwards them anyway. */
    let mut reader = BufReader::new(stream);

    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await.map_err(|error| error.to_string())?;

        if read == 0 {
            bridge.connected.store(false, Ordering::SeqCst);

            return Err("the daemon closed the connection".to_string());
        }

        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };

        if message.get("seq").is_some() {
            continue;
        }

        if message.get("id").and_then(Value::as_str) != Some(id.as_str()) {
            continue;
        }

        return Ok(message);
    }
}

/// The notification reader. Every envelope with a `seq` is forwarded as `sdcp://event`, in order.
pub async fn subscribe(app: AppHandle, bridge: Arc<SdcpBridge>) -> Result<(), String> {
    /* Idempotent, and that is the point: a second reader is a second copy of every event, so the
       sidebar would draw two rows for one chat. The flag is cleared by the reader when its socket
       ends, so the next connect subscribes again. */
    if bridge.subscribed.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let stream = match TcpStream::connect(bridge.address()).await {
        Ok(stream) => stream,
        Err(error) => {
            bridge.subscribed.store(false, Ordering::SeqCst);

            return Err(error.to_string());
        }
    };

    let mut reader = BufReader::new(stream);
    /* The first request on this connection asks for the whole log, so a window that reloads while a
       turn is running catches up instead of starting mid-sentence (spec section 5.4). */
    let since = bridge.last_seq.load(Ordering::SeqCst);
    let envelope = json!({ "v": "0.1", "id": "app-subscribe", "method": "event.list", "params": { "since": since } });

    reader
        .get_mut()
        .write_all(format!("{envelope}\n").as_bytes())
        .await
        .map_err(|error| error.to_string())?;

    tokio::spawn(async move {
        let mut line = String::new();

        loop {
            line.clear();

            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => {
                    bridge.connected.store(false, Ordering::SeqCst);
                    bridge.subscribed.store(false, Ordering::SeqCst);

                    return;
                }
                Ok(_) => {}
            }

            let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };

            /* The replay arrives as one `event.list` result; its entries are forwarded one by one so
               the frontend has exactly one code path for an event. */
            if message.get("id").and_then(Value::as_str) == Some("app-subscribe") {
                if let Some(events) = message.pointer("/result/events").and_then(Value::as_array) {
                    for entry in events {
                        bridge.last_seq.store(entry["seq"].as_i64().unwrap_or(0), Ordering::SeqCst);
                        let _ = app.emit(EVENT_NAME, entry.clone());
                    }
                }

                continue;
            }

            if let Some(seq) = message.get("seq").and_then(Value::as_i64) {
                bridge.last_seq.store(seq, Ordering::SeqCst);
                let _ = app.emit(EVENT_NAME, message);
            }
        }
    });

    Ok(())
}

/// Starts the daemon: the sidecar next to the app first, then the development `target/` builds.
fn start_daemon(bridge: &SdcpBridge) -> Result<(), String> {
    let program = find_daemon().ok_or_else(|| {
        "sdcd was not found. Install it, or run `cargo build` in sdc/sdcd, then start the app again."
            .to_string()
    })?;

    let mut command = std::process::Command::new(&program);

    command
        .arg("--port")
        .arg(bridge.port.to_string())
        /* The daemon leaves once the app has been gone for this long. Closing the app kills it
           outright (`shutdown`, wired to the window's exit), so this is the net under the case where
           the app was killed instead of closed - Task Manager, a crash, a reviewer's `Stop-Process`.
           Without it, an old daemon keeps the port and the next install talks to it. */
        .arg("--idle-exit")
        .arg("8")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    /* `sdcd` is a console program (it prints to stdout when you run it by hand), and Windows
       allocates a console for such a program unless it is told not to - which is why an installed
       build used to open a black window with the daemon's path in its title next to the app window.
       The pipes above already throw the output away, so the window is pure noise. */
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command
        .spawn()
        .map_err(|error| format!("starting {}: {error}", program.display()))?;

    *bridge.child.lock().map_err(|_| "the daemon handle was poisoned".to_string())? = Some(child);
    bridge.spawned.store(true, Ordering::SeqCst);

    Ok(())
}

/// Stops a daemon this bridge started. A daemon that was already running is left alone: it is the
/// user's, not the app's.
pub fn stop_daemon(bridge: &SdcpBridge) -> Value {
    if !bridge.spawned.swap(false, Ordering::SeqCst) {
        return json!({ "stopped": false, "reason": "the daemon was already running before the app" });
    }

    json!({ "stopped": kill_child(bridge) })
}

/// Kills the daemon this bridge started, and says whether there was one to kill.
///
/// Called when the window exits and from `sdcp_stop_daemon`. The child is *ours* - a daemon the user
/// started by hand never lands in this slot - so killing it cannot take anything from anybody.
pub fn kill_child(bridge: &SdcpBridge) -> bool {
    let Ok(mut guard) = bridge.child.lock() else {
        return false;
    };

    let Some(mut child) = guard.take() else {
        return false;
    };

    let _ = child.kill();
    let _ = child.wait();

    true
}

/// The window is going away: stop what this app started. `sdcd` is a child process, and on Windows a
/// child outlives its parent, so without this an installed app leaves a daemon - and its port - behind
/// after every quit.
pub fn shutdown(bridge: &SdcpBridge) {
    if bridge.spawned.swap(false, Ordering::SeqCst) {
        kill_child(bridge);
    }
}

/// The `sdcd` binary: `SDC_SDCD` if set, then next to the app, then the development builds.
fn find_daemon() -> Option<PathBuf> {
    let binary = if cfg!(windows) { "sdcd.exe" } else { "sdcd" };
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(explicit) = std::env::var("SDC_SDCD") {
        candidates.push(PathBuf::from(explicit));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(directory) = exe.parent() {
            candidates.push(directory.join(binary));
            candidates.push(directory.join("resources").join(binary));
            /* `app/src-tauri/target/debug/sdc[.exe]` → `sdc/sdcd/target/{debug,release}/sdcd[.exe]`. */
            for profile in ["release", "debug"] {
                candidates.push(
                    directory
                        .join("..")
                        .join("..")
                        .join("..")
                        .join("sdcd")
                        .join("target")
                        .join(profile)
                        .join(binary),
                );
            }
        }
    }

    candidates.into_iter().find(|candidate| candidate.is_file())
}
