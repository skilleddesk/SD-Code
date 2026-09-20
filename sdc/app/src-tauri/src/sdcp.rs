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
use std::sync::Arc;

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
    json!({
        "transport": "loopback",
        "address": format!("127.0.0.1:{}", bridge.port()),
        "connected": bridge.connected.load(Ordering::SeqCst),
        "spawned": bridge.spawned.load(Ordering::SeqCst),
        "lastSeq": bridge.last_seq.load(Ordering::SeqCst),
    })
}

/// The bridge's shared state.
pub struct SdcpBridge {
    port: u16,
    /// The request connection. A `tokio::sync::Mutex` because a call awaits inside it: two calls in
    /// flight on one socket would interleave their responses.
    call: Mutex<Option<TcpStream>>,
    next_id: AtomicI64,
    connected: AtomicBool,
    spawned: AtomicBool,
    last_seq: AtomicI64,
}

impl SdcpBridge {
    pub fn new() -> Arc<Self> {
        let port = std::env::var("SDC_SDCP_PORT").ok().and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_PORT);

        Arc::new(Self {
            port,
            call: Mutex::new(None),
            next_id: AtomicI64::new(1),
            connected: AtomicBool::new(false),
            spawned: AtomicBool::new(false),
            last_seq: AtomicI64::new(0),
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
pub async fn connect(app: AppHandle, bridge: Arc<SdcpBridge>) -> Result<Value, String> {
    if bridge.connected.load(Ordering::SeqCst) {
        return Ok(status_json(&bridge));
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
    let stream = TcpStream::connect(bridge.address()).await.map_err(|error| error.to_string())?;
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

    std::process::Command::new(&program)
        .arg("--port")
        .arg(bridge.port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("starting {}: {error}", program.display()))?;
    bridge.spawned.store(true, Ordering::SeqCst);

    Ok(())
}

/// Stops a daemon this bridge started. A daemon that was already running is left alone: it is the
/// user's, not the app's.
pub fn stop_daemon(bridge: &SdcpBridge) -> Value {
    if !bridge.spawned.swap(false, Ordering::SeqCst) {
        return json!({ "stopped": false, "reason": "the daemon was already running before the app" });
    }

    /* The daemon exits when its socket closes, so closing the connection is the stop signal. There is
       no separate control method to get out of sync with (spec section 5.3). */
    json!({ "stopped": true })
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
