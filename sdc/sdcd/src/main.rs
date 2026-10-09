//! sdcd's entry point: the runtime, the argument parsing and the listeners (spec section 3.1).
//!
//! `cargo run` inside `sdcd/` starts the daemon, creates the SQLite file in the platform's data
//! directory (`%APPDATA%\sdc\sdc.db` on Windows, `~/.local/share/sdc/sdc.db` on Linux,
//! `~/Library/Application Support/sdc/sdc.db` on macOS) and then serves SDCP on:
//!
//! * the unix socket (`$XDG_RUNTIME_DIR/sdc/sdcd-<port>.sock`, so two daemons on two ports do not
//!   collide), which is the local transport the app prefers on Linux and macOS;
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
use sdcd::sdcp::envelope::{Envelope, ErrorObject, Response};
use sdcd::sdcp::notifications::{ChannelNotifier, Notifier};
use sdcd::{DaemonState, SDCP_VERSION, VERSION};

fn main() -> Result<()> {
    /* Started by `ssh` as its `SSH_ASKPASS` during a sign-in (`ssh::session`): answer the one prompt and
       leave. Nothing of the daemon is started. */
    if let Ok(spec) = std::env::var(sdcd::ssh::session::ASKPASS_ENV) {
        let prompt = std::env::args().nth(1).unwrap_or_default();

        std::process::exit(sdcd::ssh::session::answer_prompt(&spec, &prompt));
    }

    /* Started as a turn's, an MCP server's or the Terminal's `ssh` (0.16.0): relay one command over the
       daemon's own connection to the host and leave with its exit code. */
    {
        let all: Vec<String> = std::env::args().skip(1).collect();

        if all.first().map(String::as_str) == Some(sdcd::ssh::bridge::FLAG) {
            std::process::exit(sdcd::ssh::bridge::client_main(&all[1..]));
        }
    }

    /* Before the runtime exists, because it changes the process environment: a daemon started from the
       macOS Dock has only `/usr/bin:/bin:/usr/sbin:/sbin`, and found none of the CLIs (0.15.8). */
    sdcd::host::env_path::widen();

    /* 0.21: Git on a Windows machine that has none, fetched in the background - checkpoints need it. Only
       for the daemon itself, not for a headless `sdcd run|verify|kill|audit` that leaves in a moment. */
    if !std::env::args().nth(1).is_some_and(|command| matches!(command.as_str(), "run" | "verify" | "kill" | "audit")) {
        sdcd::host::tools::provision_essentials();
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?
        .block_on(run())
}

async fn run() -> Result<()> {

    sdcd::crash::install_hook();

    /* Headless commands (0.12): `sdcd run|verify|kill|audit …` talk to the daemon that is already running,
       so a turn from a terminal or a CI job gets the same Trust Kernel as one typed in the window. */
    {
        let all: Vec<String> = std::env::args().skip(1).collect();

        if let Some(command) = all.first().filter(|command| matches!(command.as_str(), "run" | "verify" | "kill" | "audit")) {
            let port = all
                .iter()
                .position(|arg| arg == "--port")
                .and_then(|index| all.get(index + 1))
                .and_then(|value| value.parse().ok())
                .unwrap_or(paths::DEFAULT_PORT);
            let command = command.clone();
            let code = tokio::task::spawn_blocking(move || sdcd::cli::main(&command, &all[1..], port)).await.unwrap_or(1);

            std::process::exit(code);
        }
    }

    let mut port = paths::DEFAULT_PORT;
    let mut database = None;
    /* 0 means "never leave on my own", which is right for a daemon a human started: a terminal you
       can watch is a terminal you close yourself. The app passes a number, so the daemon it started
       leaves when the app does - even if the app was killed rather than closed (spec section 3.1). */
    let mut idle_exit = 0_u64;
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => port = arguments.next().and_then(|value| value.parse().ok()).unwrap_or(port),
            "--database" => database = arguments.next().map(std::path::PathBuf::from),
            "--idle-exit" => {
                idle_exit = arguments.next().and_then(|value| value.parse().ok()).unwrap_or(idle_exit)
            }
            /* The uninstaller's "Delete the application data" box (0.13): keys and the data folder. */
            "--forget-everything" => {
                let dir = paths::data_dir()?;

                for line in sdcd::reset::forget_everything(&dir) {
                    println!("removed {line}");
                }

                return Ok(());
            }
            "--version" => {
                println!("sdcd {VERSION} (SDCP {SDCP_VERSION})");
                return Ok(());
            }
            "--help" => {
                println!(
                    "sdcd {VERSION}\n  --port <n>        loopback port (default {})\n  --database <p>    SQLite file (default: the platform data directory)\n  --idle-exit <secs> exit after <secs> with no client (default: never)",
                    paths::DEFAULT_PORT
                );
                return Ok(());
            }
            other => eprintln!("sdcd: ignoring unknown argument `{other}`"),
        }
    }

    let default_database = database.is_none();
    let state = DaemonState::bootstrap(database)?;

    /* SDC Anywhere starts only if the person left it on (plan principle 8: off by default). */
    state.anywhere.attach(tokio::runtime::Handle::current());
    state.anywhere.resume(&state);

    /* The day's copy of the real database (0.15.5), off the start-up path. A scratch `--database` is
       nobody's history and is not copied. */
    if default_database {
        let store = state.store.clone();

        std::thread::spawn(move || {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();

            match sdcd::paths::backup_dir().and_then(|dir| sdcd::paths::backup_daily(&store, &dir, &today)) {
                Ok(copy) => println!("  backup     {}", copy.display()),
                Err(error) => eprintln!("sdcd: the daily database copy was not written: {error:#}"),
            }
        });
    }

    println!("sdcd {VERSION} · SDCP {SDCP_VERSION}");
    println!("  database   {}", state.store.path());
    println!("  events     {} replayed from the log", state.events.len());

    let tcp = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("binding 127.0.0.1:{port}"))?;

    println!("  loopback   127.0.0.1:{}", tcp.local_addr()?.port());

    /* A restore the last daemon was in the middle of is finished first (the Time Machine's journal, 0.12),
       before any window can ask about the folder it was restoring. */
    for sentence in sdcd::rewind::recover(&state.store) {
        println!("  journal    {sentence}");
        state.events.append(sdcd::sdcp::events::event::toast(&sentence, None, Some(12_000)), None, None);
    }

    /* Health Watch and the Night Guardian (0.12): every site on its own interval. */
    sdcd::ops::health::spawn(
        state.clone(),
        Arc::new(ChannelNotifier::new(state.events.clone(), state.fanout.clone())),
    );

    /* The phone's read-only status page, when the person turned it on. */
    if state.store.setting("status.share").ok().flatten().as_deref() == Some("on") {
        let answer = sdcd::status::configure(state.clone(), true, None);

        println!("  status     {}", answer["url"].as_str().unwrap_or("could not start"));
    }

    /* The catalogue keeps itself fresh (0.9.0): the connected providers are asked at start and every
       twelve hours, and every window hears `ModelsUpdated` when a list moved. A model a provider ships
       tomorrow is in the dropdown tomorrow, with no button pressed and no build. */
    {
        let store = state.store.clone();
        let notifier: Arc<dyn Notifier> =
            Arc::new(ChannelNotifier::new(state.events.clone(), state.fanout.clone()));

        tokio::spawn(async move {
            /* After the window's first burst of requests, not during it. */
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;

            loop {
                sdcd::providers::models::refresh_and_tell(store.clone(), notifier.clone());
                tokio::time::sleep(std::time::Duration::from_secs(12 * 60 * 60)).await;
            }
        });
    }

    /* A connection that dies says so (0.11.5): every connected VPS row is measured again on an
       interval, and a host whose sign-in went away turns `offline` on screen - which is what opens the
       window's sign-in card - instead of looking connected until the next file read fails. */
    sdcd::ssh::watch::spawn(
        state.store.clone(),
        Arc::new(ChannelNotifier::new(state.events.clone(), state.fanout.clone())),
    );

    #[cfg(unix)]
    {
        let socket = paths::socket_path(port)?;

        let _ = std::fs::remove_file(&socket);

        /* A unix socket that cannot be bound is a warning, not a reason to refuse to start: the
           loopback transport below is always on, and it is the one the app uses. A daemon that died
           because another daemon owned a socket *path* would be a daemon that took the whole app with
           it for no reason. */
        match tokio::net::UnixListener::bind(&socket) {
            Ok(listener) => {
                println!("  socket     {}", socket.display());

                let unix_state = state.clone();

                tokio::spawn(async move {
                    loop {
                        match listener.accept().await {
                            Ok((stream, _)) => {
                                let daemon_state = unix_state.clone();

                                let guard = daemon_state.client_guard();

                                tokio::spawn(async move {
                                    let _guard = guard;
                                    let _ = serve_unix(stream, daemon_state.clone()).await;
                                });
                            }
                            Err(error) => eprintln!("sdcd: socket accept failed: {error}"),
                        }
                    }
                });
            }
            Err(error) => eprintln!("sdcd: no unix socket at {}: {error}", socket.display()),
        }
    }

    #[cfg(windows)]
    println!("  pipe       \\\\.\\pipe\\sdcd (served by the Tauri bridge, see app/src-tauri)");

    if idle_exit > 0 {
        println!("  idle exit  {idle_exit}s with no client");
    }

    let tcp_state = state.clone();

    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = tcp.accept().await else {
                break;
            };

            let daemon_state = tcp_state.clone();

            /* Counted by a guard, so a handler that panics still lets the count down (0.16.1). */
            let guard = daemon_state.client_guard();

            tokio::spawn(async move {
                let _guard = guard;

                println!("sdcd: client {peer} connected");

                if let Err(error) = serve(stream, daemon_state.clone()).await {
                    eprintln!("sdcd: client {peer} ended: {error}");
                }
            });
        }
    });

    /* The run loop ends for exactly two reasons: a client asked (`host.shutdown`, how the app stops a
       daemon that is not the version it needs), or nobody has talked to us for the time the app asked
       for. Both end here rather than in the accept loop, so the stopping is orderly: the children the
       daemon started are killed first (spec section 3.1). */
    match sdcd::watch(&state, idle_exit).await {
        sdcd::StopReason::Idle(seconds) => println!("sdcd: no client for {seconds}s, exiting"),
        sdcd::StopReason::Requested => println!("sdcd: stopping on request"),
    }

    state.shutdown();

    /* Leave now. Returning from `main` drops the runtime, and a runtime drop **waits** for every
       `spawn_blocking` task still running - since 0.9.0 that includes the catalogue refresh, an HTTP
       call a slow provider can hold for minutes. A daemon asked to stop must stop: the children are
       already killed above and every event is already on disk (the log writes as it appends). */
    std::process::exit(0);
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
    let (subscriber, backlog) = state.fanout.subscribe_counted(sender);

    tokio::spawn(async move {
        while let Some(line) = receiver.recv().await {
            backlog.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

            let mut sink = notifier_writer.lock().await;

            if sink.write_all(line.as_bytes()).await.is_err() || sink.write_all(b"\n").await.is_err() {
                break;
            }

            if sink.flush().await.is_err() {
                break;
            }
        }

        /* The registry let this connection go (it stopped reading, 0.16.1), or the connection is ending.
           Either way the socket closes, so the client sees it and reconnects - and replays what it
           missed - rather than waiting for ever on notifications that no longer come. */
        let _ = notifier_writer.lock().await.shutdown().await;
    });

    let notifier: Arc<dyn Notifier> = Arc::new(ChannelNotifier::new(state.events.clone(), state.fanout.clone()));
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();

        if line.is_empty() {
            continue;
        }

        /* The log persists every event as it is appended (`EventLog::append`), so this loop only has
           to answer: a turn's stream keeps landing in the database after the response is written. */
        /* The handlers are synchronous and some wait on the network (an `ssh` call can take 20 s). On the
           blocking pool that wait holds a thread meant for waiting, not one of the few that move every
           other connection's notifications (0.16.1). Requests still answer in the order they came. */
        let response = match Envelope::parse(&line) {
            Ok(envelope) => {
                let daemon = state.handler();
                let notifier = notifier.clone();
                let id = envelope.id.clone();

                tokio::task::spawn_blocking(move || daemon.handle(&envelope, notifier))
                    .await
                    .unwrap_or_else(|_| {
                        Response::fail(id, ErrorObject::internal("the request stopped unexpectedly inside the daemon; the daemon is still running"))
                    })
            }
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


