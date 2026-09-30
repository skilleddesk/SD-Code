//! The `PATH` a program started from a Dock, a Finder window or a desktop launcher does not get.
//!
//! WHY THIS EXISTS. A macOS app opened from Finder or the Dock inherits `launchd`'s `PATH`, which is
//! `/usr/bin:/bin:/usr/sbin:/sbin` and nothing else. `claude`, `codex`, `gemini`, `node` and `npm` live in
//! `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`, `~/.npm-global/bin` or an nvm folder, so the
//! daemon the app started reported every subscription CLI as **not installed** while `claude` ran fine in
//! the user's Terminal: no sign-in, no browser, "not connected" on a machine that was signed in (reported
//! on 0.15.7). A Linux desktop launcher has the same hole for nvm and `~/.local/bin`, and a Windows app
//! started before an install misses the folder that install added.
//!
//! The fix is what a terminal does: ask the user's own login shell for its `PATH`, then add the folders the
//! common installers use when they exist. It runs **once, in `main`, before any thread is started**, because
//! it changes the process environment - which every child the daemon spawns then inherits, so an npm CLI's
//! `#!/usr/bin/env node` finds `node` too.

use std::ffi::OsString;
use std::path::{Path, PathBuf};


/// Widens this process's `PATH`. Call it before any thread exists (see the module doc).
pub fn widen() {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let home = dirs::home_dir();
    let shell = if cfg!(windows) { None } else { login_shell_path() };
    let merged = merge(&current, shell.as_deref(), &extra_dirs(home.as_deref()));

    if merged != current {
        std::env::set_var("PATH", merged);
    }
}

/// `current` first (what the parent chose wins), then the login shell's entries, then `extra` - each
/// directory once, and only extras that exist.
pub fn merge(current: &OsString, shell: Option<&str>, extra: &[PathBuf]) -> OsString {
    let mut seen: Vec<PathBuf> = Vec::new();

    let mut push = |dir: PathBuf| {
        if !dir.as_os_str().is_empty() && !seen.contains(&dir) {
            seen.push(dir);
        }
    };

    for dir in std::env::split_paths(current) {
        push(dir);
    }

    if let Some(shell) = shell {
        for dir in std::env::split_paths(&OsString::from(shell)) {
            push(dir);
        }
    }

    for dir in extra.iter().filter(|dir| dir.is_dir()) {
        push(dir.clone());
    }

    std::env::join_paths(seen).unwrap_or_else(|_| current.clone())
}

/// The folders the usual installers put a CLI (or `node`) in, for this platform.
pub fn extra_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if cfg!(windows) {
        for var in ["APPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                dirs.push(PathBuf::from(base).join("npm"));
            }
        }

        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(&base).join("pnpm"));
            dirs.push(PathBuf::from(&base).join("Programs").join("nodejs"));
        }

        if let Some(base) = std::env::var_os("ProgramFiles") {
            dirs.push(PathBuf::from(base).join("nodejs"));
        }
    } else {
        for dir in ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/local/sbin", "/snap/bin", "/home/linuxbrew/.linuxbrew/bin"] {
            dirs.push(PathBuf::from(dir));
        }
    }

    if let Some(home) = home {
        for relative in [
            ".local/bin",
            ".claude/local",
            ".npm-global/bin",
            ".npm/bin",
            ".bun/bin",
            ".volta/bin",
            ".cargo/bin",
            ".deno/bin",
            ".local/share/pnpm",
            "Library/pnpm",
            ".yarn/bin",
        ] {
            dirs.push(home.join(relative));
        }

        /* nvm keeps one folder per Node version and puts none of them on PATH outside an interactive
           shell; the newest one is what `nvm alias default` usually is. */
        if let Some(bin) = newest_nvm_bin(&home.join(".nvm").join("versions").join("node")) {
            dirs.push(bin);
        }
    }

    dirs
}

/// `~/.nvm/versions/node/<newest>/bin`, by version number rather than by name.
fn newest_nvm_bin(root: &Path) -> Option<PathBuf> {
    let version = |name: &str| -> Vec<u64> {
        name.trim_start_matches('v').split('.').map(|part| part.parse().unwrap_or(0)).collect()
    };

    std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .max_by(|left, right| version(left).cmp(&version(right)))
        .map(|name| root.join(name).join("bin"))
        .filter(|bin| bin.is_dir())
}

/// The `PATH` the user's login shell builds (`$SHELL -ilc`), or `None` when it cannot be read in time.
///
/// `-i` as well as `-l`: nvm and many installers write to `.zshrc`/`.bashrc`, which only an interactive
/// shell reads. Markers around the value, because an interactive shell may print a banner or a warning.
/// Bounded at four seconds - a slow or broken shell profile must not stop the daemon from starting.
#[cfg(not(windows))]
fn login_shell_path() -> Option<String> {
    use std::io::Read;
    use std::time::{Duration, Instant};
    use std::process::{Command, Stdio};

    const START: &str = "__SDC_PATH_START__";
    const END: &str = "__SDC_PATH_END__";

    let shell = std::env::var("SHELL").ok().filter(|shell| !shell.trim().is_empty()).unwrap_or_else(|| {
        if cfg!(target_os = "macos") { "/bin/zsh".to_string() } else { "/bin/sh".to_string() }
    });
    let mut child = Command::new(&shell)
        .args(["-ilc", &format!("printf '%s%s%s' '{START}' \"$PATH\" '{END}'")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(4);

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();

                return None;
            }
        }
    }

    let mut text = String::new();

    child.stdout.take()?.read_to_string(&mut text).ok()?;

    between(&text, START, END)
}

#[cfg(windows)]
fn login_shell_path() -> Option<String> {
    None
}

/// The text between two markers, trimmed; `None` when either is missing or nothing is between them.
pub fn between(text: &str, start: &str, end: &str) -> Option<String> {
    let from = text.find(start)? + start.len();
    let to = from + text[from..].find(end)?;
    let value = text[from..to].trim();

    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_between_the_markers_survives_a_noisy_profile() {
        let noisy = "Last login: today\nwelcome!\n__S__/opt/homebrew/bin:/usr/bin__E__";

        assert_eq!(between(noisy, "__S__", "__E__").as_deref(), Some("/opt/homebrew/bin:/usr/bin"));
        assert_eq!(between("no markers", "__S__", "__E__"), None);
        assert_eq!(between("__S____E__", "__S__", "__E__"), None);
    }

    #[test]
    fn the_parents_path_comes_first_and_nothing_twice() {
        let a = tempfile::TempDir::new().unwrap();
        let b = tempfile::TempDir::new().unwrap();
        let current = std::env::join_paths([a.path()]).unwrap();
        let shell = std::env::join_paths([a.path(), b.path()]).unwrap();
        let missing = a.path().join("does-not-exist");
        let merged = merge(&current, shell.to_str(), &[b.path().to_path_buf(), missing.clone()]);
        let dirs: Vec<PathBuf> = std::env::split_paths(&merged).collect();

        assert_eq!(dirs, vec![a.path().to_path_buf(), b.path().to_path_buf()]);
        assert!(!dirs.contains(&missing));
    }

    #[test]
    fn the_newest_nvm_node_is_picked_by_number() {
        let root = tempfile::TempDir::new().unwrap();

        for version in ["v9.11.2", "v20.10.0", "v18.19.1"] {
            std::fs::create_dir_all(root.path().join(version).join("bin")).unwrap();
        }

        assert_eq!(newest_nvm_bin(root.path()), Some(root.path().join("v20.10.0").join("bin")));
    }
}
