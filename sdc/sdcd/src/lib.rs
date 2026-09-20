//! sdcd - the SDC host daemon (master spec sections 3.1, 5 and 6).
//!
//! One daemon runs per host - the local machine or a VPS. It owns sessions, engines, files, PTYs,
//! git and the append-only event log, and it speaks SDCP to the SDC app. Local and remote mode are
//! the same binary and the same protocol; only the transport differs (spec section 3.1):
//!
//! | mode   | transport                                    | platform       |
//! | ------ | -------------------------------------------- | -------------- |
//! | local  | unix socket `$XDG_RUNTIME_DIR/sdc/sdcd.sock`  | Linux, macOS   |
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

use std::sync::Arc;

use anyhow::{Context, Result};

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

        Ok(Arc::new(Self {
            store,
            events,
            engines: Arc::new(EngineRegistry::with_defaults()),
            pty: Arc::new(PtyManager::new()),
            console: Arc::new(ConsoleBridge::new()),
            fanout: Fanout::new(),
        }))
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



