//! Platform data paths, and the ownership rule that goes with them (master spec section 17.9).
//!
//! One daemon per host, but not necessarily one daemon per *machine*: on a shared box each user runs
//! their own `sdcd`, so everything it writes is namespaced by UID. The layout is the platform's own,
//! reached through `dirs`, with `sdc/` as the last component:
//!
//! | platform | root                                        |
//! | -------- | ------------------------------------------- |
//! | Linux    | `$XDG_DATA_HOME/sdc` (`~/.local/share/sdc`) |
//! | macOS    | `~/Library/Application Support/sdc`         |
//! | Windows  | `%APPDATA%\sdc`                             |
//!
//! The socket is *not* under the data directory on Linux: runtime sockets belong in
//! `$XDG_RUNTIME_DIR`, which is already per-user and cleaned up by the session. On Windows the pipe
//! name carries the user's SID instead, for the same reason.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// `…/sdc` - the daemon's data directory. Created on first use.
pub fn data_dir() -> Result<PathBuf> {
    let root = dirs::data_dir().context("no platform data directory for this user")?;
    let dir = root.join("sdc");

    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    Ok(dir)
}

/// `…/sdc/sdc.db` - the SQLite file whose schema is spec section 6. The acceptance list names this
/// path explicitly, so it is computed in exactly one place.
pub fn database_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("sdc.db"))
}

/// `…/sdc/checkpoints` - screenshots and file snapshots (spec section 14).
pub fn checkpoints_dir() -> Result<PathBuf> {
    let dir = data_dir()?.join("checkpoints");

    std::fs::create_dir_all(&dir)?;

    Ok(dir)
}

/// `…/sdc/git` - the shadow repository and the per-session worktrees (spec section 7.2 of the
/// daemon plan). Never the user's own `.git`.
pub fn shadow_git_dir() -> Result<PathBuf> {
    let dir = data_dir()?.join("git");

    std::fs::create_dir_all(&dir)?;

    Ok(dir)
}

/// The unix socket, or the Windows pipe name.
pub fn socket_path() -> Result<PathBuf> {
    #[cfg(unix)]
    {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir());

        let dir = runtime.join("sdc");

        std::fs::create_dir_all(&dir)?;

        return Ok(dir.join("sdcd.sock"));
    }

    #[cfg(windows)]
    {
        Ok(PathBuf::from(r"\\.\pipe\sdcd"))
    }
}

/// The loopback TCP port. It is on by default in addition to the socket or pipe: a WebView in a
/// sandbox, or a test harness, can always reach `127.0.0.1` when it cannot open either.
pub const DEFAULT_PORT: u16 = 7811;

/// True when `path` is inside the daemon's own data directory - the check the file guard uses to
/// keep an engine from rewriting the daemon's database (spec section 5.4, blocked patterns).
pub fn is_internal(path: &Path) -> bool {
    match data_dir() {
        Ok(root) => path.starts_with(root),
        Err(_) => false,
    }
}
