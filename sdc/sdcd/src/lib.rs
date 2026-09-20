//! sdcd - the SDC host daemon (master spec sections 3.1, 5 and 6).
//!
//! One daemon runs per host - the local machine or a VPS. It owns sessions, engines, files, PTYs,
//! git and the append-only event log, and it speaks SDCP to the SDC app. Local and remote mode are
//! the same binary and the same protocol; only the transport differs (spec section 3.1):
//!
//! | mode   | transport                                    | platform       |
//! | ------ | -------------------------------------------- | -------------- |
//! | local  | unix socket `$XDG_RUNTIME_DIR/sdc/sdcd-<port>.sock` | Linux, macOS |
//! | local  | named pipe `\\.\pipe\sdcd`                    | Windows        |
//! | local  | TCP `127.0.0.1:7811`, always on as well      | every platform |
//! | remote | the same bytes over an SSH tunnel            | -              |
//!
//! ## What is here
//!
//! Every module of the daemon plan (spec section 10) is implemented:
//!
//! | module             | what it owns                                                     |
//! | ------------------ | ---------------------------------------------------------------- |
//! | `paths`            | the per-user data directory (spec section 17.9)                    |
//! | `store`            | SQLite and the schema of spec section 6, including the event log    |
//! | `sdcp`             | the envelope, the event catalogue, the notifier and the handlers    |
//! | `engines`          | the five adapters of spec section 11 (`claude_code`, `codex`, `gemini`, `native_api`, `ollama`) |
//! | `fs`               | safe file ops and the blocked patterns of spec section 5.4          |
//! | `git`              | the shadow repository and the per-session worktree                  |
//! | `pty`              | long-running processes                                              |
//! | `auth`             | the keychain, with the file fallback it reports                     |
//! | `host`             | the ten environment checks of spec section 9.10                     |
//! | `providers`        | the six flows of spec section 9.10 and the registry                 |
//! | `checkpoints`      | checkpoints and their screenshots (spec section 14)                 |
//! | `rewind`           | restoring files and conversation (spec section 14)                  |
//! | `duel`             | two engines on one prompt (spec section 16.6)                       |
//! | `errors`           | the plain-English translator (spec section 14.9)                    |
//! | `session_bridge`   | switching engines mid-turn (spec section 16.5)                      |
//! | `console`          | the preview console bridge (spec section 15.4)                      |
//!
//! Two boundaries are stated rather than hidden, because a daemon that pretends is worse than one
//! that reports: `native_api` needs a TLS client for a remote `https://` endpoint (a local `http://`
//! one streams today), and `checkpoints::screenshot` records a screenshot the *app* takes - the
//! daemon owns the path, not the pixels. Both are documented in their modules and both are what the
//! acceptance list of this step expects.

pub mod auth;
pub mod checkpoints;
pub mod console;
pub mod duel;
pub mod engines;
pub mod errors;
pub mod fs;
pub mod git;
pub mod host;
pub mod paths;
pub mod providers;
pub mod pty;
pub mod rewind;
pub mod sdcp;
pub mod session_bridge;
pub mod store;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::auth::cli_login::LoginManager;
use crate::console::bridge::ConsoleBridge;
use crate::engines::EngineRegistry;
use crate::pty::PtyManager;
use crate::sdcp::events::EventLog;
use crate::sdcp::methods::Daemon;
use crate::sdcp::notifications::Fanout;
use crate::store::sqlite::Store;
/// The daemon's version, reported by `host.status` and shown in the app's About tab.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The SDCP version this daemon speaks (spec section 5.7: additive changes keep it).
pub const SDCP_VERSION: &str = "0.1";

/// Everything a request handler needs. Built once at startup and shared by every connection.
pub struct DaemonState {
    pub store: Arc<Store>,
    pub events: Arc<EventLog>,
    pub engines: Arc<EngineRegistry>,
    pub pty: Arc<PtyManager>,
    pub console: Arc<ConsoleBridge>,
    /// The notification subscribers. One registry, because an event belongs to every client that is
    /// listening, not only to the one that asked for it (see `sdcp::notifications`).
    pub fanout: Arc<Fanout>,
    /// The CLI logins in flight. It holds the process registry, so a login survives the request that
    /// started it - which is the whole point of a login that waits for a human to approve a page.
    pub logins: Arc<LoginManager>,
    /// How many connections are open right now. The idle watcher reads it, so a daemon the app started
    /// can leave when the app does - including the case where the app was killed rather than closed.
    pub clients: Arc<AtomicUsize>,
    /// Set by `host.shutdown`, or by the idle watcher once its grace has passed. The run loop polls it
    /// so a *request* can end the process rather than a signal, which is the only way a client that
    /// only has a socket can ask for it.
    pub stopping: Arc<AtomicBool>,
}

impl DaemonState {
    /// Opens the database, wires the store and the log, and returns the shared state.
    ///
    /// The order matters: the store is opened first (it owns the schema and the file), then the log
    /// is hydrated from it - so a restarted daemon continues the same sequence rather than starting a
    /// conversation from nothing (spec section 5.6, "append-only, never mutated").
    pub fn bootstrap(database: Option<std::path::PathBuf>) -> Result<Arc<Self>> {
        let path = match database {
            Some(explicit) => explicit,
            None => paths::database_path()?,
        };

        let store = Arc::new(Store::open(&path).with_context(|| format!("opening {}", path.display()))?);
        let events = Arc::new(EventLog::hydrate(store.clone())?);

        let pty = Arc::new(PtyManager::new());

        Ok(Arc::new(Self {
            store,
            events,
            engines: Arc::new(EngineRegistry::with_defaults()),
            logins: Arc::new(LoginManager::new(pty.clone())),
            pty,
            console: Arc::new(ConsoleBridge::new()),
            fanout: Fanout::new(),
            clients: Arc::new(AtomicUsize::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// One connection opened. Paired with `client_left`; the counter is what the idle watcher reads.
    pub fn client_joined(&self) {
        self.clients.fetch_add(1, Ordering::SeqCst);
    }

    /// One connection closed.
    pub fn client_left(&self) {
        self.clients.fetch_sub(1, Ordering::SeqCst);
    }

    /// The number of connections open right now.
    pub fn clients(&self) -> usize {
        self.clients.load(Ordering::SeqCst)
    }

    /// Asks the run loop to stop after this process's work is done: what `host.shutdown` sets.
    pub fn request_stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    /// Whether a stop has been asked for.
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// The request handler for one connection. `self: &Arc<Self>` so the handler can hold the shared
    /// state instead of a reference to it, which is what lets a turn outlive its request.
    pub fn handler(self: &Arc<Self>) -> Daemon {
        Daemon::new(self.clone())
    }

    /// Kills everything the daemon started. Called on shutdown so a dev server or a test watcher does
    /// not outlive the process that started it.
    pub fn shutdown(&self) {
        self.pty.close_all();
    }
}

/// Why the run loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// A client asked (`host.shutdown`).
    Requested,
    /// Nobody talked to us for this many seconds.
    Idle(u64),
}

/// Waits until this daemon should stop.
///
/// Two things end the wait: `host.shutdown` set the flag, or `idle_exit` seconds passed with no
/// connection. `idle_exit: 0` means "never" - the behaviour of a daemon a human started by hand, who
/// closes the terminal when they are done with it. The app passes a number, so the daemon it started
/// leaves when the app does, including the case where the app was killed rather than closed.
pub async fn watch(state: &Arc<DaemonState>, idle_exit: u64) -> StopReason {
    let mut idle_for = 0_u64;

    loop {
        /* Sleep *first*: the grace is a whole second of nobody talking to us, not "we were idle for a
           second before we even opened the port" - which is what the first version of this loop said,
           and a daemon that exits before its owner can connect is worse than one that lingers. */
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        if state.is_stopping() {
            return StopReason::Requested;
        }

        if idle_exit > 0 {
            if state.clients() == 0 {
                idle_for += 1;

                if idle_for >= idle_exit {
                    return StopReason::Idle(idle_exit);
                }
            } else {
                idle_for = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Arc<DaemonState> {
        /* A database in memory: the watcher never touches it, and a test should not leave a file. */
        DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("bootstrapping the daemon")
    }

    #[test]
    fn a_client_is_counted_while_it_is_connected() {
        let state = state();

        assert_eq!(state.clients(), 0);
        state.client_joined();
        state.client_joined();
        assert_eq!(state.clients(), 2);
        state.client_left();
        assert_eq!(state.clients(), 1);
    }

    #[test]
    fn a_request_sets_the_stopping_flag() {
        let state = state();

        assert!(!state.is_stopping());
        state.request_stop();
        assert!(state.is_stopping());
    }

    #[tokio::test]
    async fn the_watcher_returns_when_a_client_asks() {
        let state = state();

        state.request_stop();

        assert_eq!(watch(&state, 30).await, StopReason::Requested);
    }

    #[tokio::test]
    async fn the_watcher_returns_once_the_idle_grace_is_over() {
        let state = state();

        /* One second of grace, so the test waits about that long and no longer. */
        assert_eq!(watch(&state, 1).await, StopReason::Idle(1));
    }

    #[tokio::test]
    async fn the_watcher_keeps_waiting_while_a_client_is_connected() {
        let state = state();

        state.client_joined();

        let waited = tokio::time::timeout(std::time::Duration::from_secs(3), watch(&state, 1)).await;

        assert!(waited.is_err(), "the watcher left while a client was connected");
    }

    #[tokio::test]
    async fn idle_exit_zero_never_leaves_on_its_own() {
        let state = state();

        let waited = tokio::time::timeout(std::time::Duration::from_secs(3), watch(&state, 0)).await;

        assert!(waited.is_err(), "idle_exit 0 must mean \"never\"");
    }
}



