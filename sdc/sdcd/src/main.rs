//! sdcd's entry point: the runtime, the argument parsing and the listeners (spec section 3.1).
//!
//! `cargo run` inside `sdcd/` starts the daemon, creates the SQLite file in the platform's data
//! directory (`%APPDATA%\sdc\sdc.db` on Windows, `~/.local/share/sdc/sdc.db` on Linux,
//! `~/Library/Application Support/sdc/sdc.db` on macOS) and then serves SDCP on:
//!
//! * the unix socket (`$XDG_RUNTIME_DIR/sdc/sdcd.sock`), which is the local transport the app
//!   prefers on Linux and macOS;
//! * `127.0.0.1:7811` (or `--port`), on every platform, because a WebView in a sandbox or a test
//!   harness can always reach loopback when it cannot open the socket.
//!
//! Both carry the same envelope - only the pipe differs (spec section 3.1). The Windows named pipe
//! is served by the app's own bridge (`app/src-tauri`), which is where a pipe server has to live.
//!
//! Framing is newline-delimited JSON, one object per line, in both directions. It is the simplest
//! thing that cannot desynchronise: there is no length prefix to get out of step with the payload.

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use sdcd::paths;
use sdcd::sdcp::envelope::{Envelope, Response};
use sdcd::sdcp::notifications::{ChannelNotifier, Notifier};
use sdcd::{DaemonState, SDCP_VERSION, VERSION};

#[tokio::main]
async fn main() -> Result<()> {
    let mut port = paths::DEFAULT_PORT;
    let mut database = None;
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => port = arguments.next().and_then(|value| value.parse().ok()).unwrap_or(port),
            "--database" => database = arguments.next().map(std::path::PathBuf::from),
            "--version" => {
                println!("sdcd {VERSION} (SDCP {SDCP_VERSION})");
                return Ok(());
            }
            "--help" => {
                println!(
                    "sdcd {VERSION}\n  --port <n>       loopback port (default {})\n  --database <p>   SQLite file (default: the platform data directory)",
                    paths::DEFAULT_PORT
                );
                return Ok(());
            }
            other => eprintln!("sdcd: ignoring unknown argument `{other}`"),
        }
    }

    let state = DaemonState::bootstrap(database)?;

    println!("sdcd {VERSION} · SDCP {SDCP_VERSION}");
    println!("  database   {}", state.store.path());
    println!("  events     {} replayed from the log", state.events.len());

    let tcp = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("binding 127.0.0.1:{port}"))?;

    println!("  loopback   127.0.0.1:{}", tcp.local_addr()?.port());

    #[cfg(unix)]
    {
        let socket = paths::socket_path()?;

        let _ = std::fs::remove_file(&socket);

        let listener = tokio::net::UnixListener::bind(&socket)
            .with_context(|| format!("binding {}", socket.display()))?;

        println!("  socket     {}", socket.display());

        let unix_state = state.clone();

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let daemon_state = unix_state.clone();

                        tokio::spawn(async move {
                            let _ = serve_unix(stream, daemon_state).await;
                        });
                    }
                    Err(error) => eprintln!("sdcd: socket accept failed: {error}"),
                }
            }
        });
    }

    #[cfg(windows)]
    println!("  pipe       \\\\.\\pipe\\sdcd (served by the Tauri bridge, see app/src-tauri)");

    loop {
        let (stream, peer) = tcp.accept().await?;
        let daemon_state = state.clone();

        tokio::spawn(async move {
            println!("sdcd: client {peer} connected");

            if let Err(error) = serve(stream, daemon_state).await {
                eprintln!("sdcd: client {peer} ended: {error}");
            }
        });
    }
}
/// Serves one connection: read a line, answer it, and forward everything the handler pushes.
///
/// The notifier writes through a channel so a handler can keep pushing *after* it has answered,
/// which is what an engine stream needs. One writer task owns the socket, so a response and a
/// notification can never interleave mid-line.
async fn serve(stream: TcpStream, state: Arc<DaemonState>) -> Result<()> {
    let (reader, writer) = stream.into_split();

    pump(reader, writer, state).await
}

/// The same loop for the unix socket, whose halves are different types.
#[cfg(unix)]
async fn serve_unix(stream: tokio::net::UnixStream, state: Arc<DaemonState>) -> Result<()> {
    let (reader, writer) = stream.into_split();

    pump(reader, writer, state).await
}

/// One connection's loop, generic over the two halves so the TCP and unix paths share it.
async fn pump<R, W>(reader: R, writer: W, state: Arc<DaemonState>) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (sender, mut receiver) = mpsc::unbounded_channel::<String>();
    /* One lock around the writer, held per line: a response and a notification can never interleave
       mid-line, and neither can two notifications (spec section 5.5). */
    let writer = Arc::new(tokio::sync::Mutex::new(writer));
    let notifier_writer = writer.clone();

    /* This connection listens to everything: the events other clients cause reach it too, which is
       what lets the app keep a socket that only subscribes (`sdcp_subscribe`). */
    let subscriber = state.fanout.subscribe(sender);

    tokio::spawn(async move {
        while let Some(line) = receiver.recv().await {
            let mut sink = notifier_writer.lock().await;

            if sink.write_all(line.as_bytes()).await.is_err() || sink.write_all(b"\n").await.is_err() {
                break;
            }

            if sink.flush().await.is_err() {
                break;
            }
        }
    });

    let notifier: Arc<dyn Notifier> = Arc::new(ChannelNotifier::new(state.events.clone(), state.fanout.clone()));
    let daemon = state.handler();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();

        if line.is_empty() {
            continue;
        }

        /* The log persists every event as it is appended (`EventLog::append`), so this loop only has
           to answer: a turn's stream keeps landing in the database after the response is written. */
        let response = match Envelope::parse(&line) {
            Ok(envelope) => daemon.handle(&envelope, notifier.clone()),
            Err(error) => Response::fail("unknown", error),
        };

        let mut answer = serde_json::to_string(&response)?;

        answer.push('\n');

        let mut sink = writer.lock().await;

        sink.write_all(answer.as_bytes()).await?;
        sink.flush().await?;
    }

    state.fanout.unsubscribe(subscriber);

    Ok(())
}


