//! Finding a program the way the platform's own shell finds it.
//!
//! WHY THIS EXISTS. On Windows, npm installs a CLI as a *shim*: `claude` is a file called
//! `claude.cmd`, and so are `codex` and `gemini`. `std::process::Command::new("claude")` does not find
//! it - it appends `.exe` and gives up - so this daemon reported all three as **not installed** on a
//! machine where `claude --version` prints `2.1.278 (Claude Code)` in any terminal. The consequence was
//! total: the Provider Hub said "install it", the doctor's row said `fail`, and `engine.start`
//! answered with the missing-program sentence. Every subscription provider was unreachable on Windows
//! while the user's own shell ran them fine.
//!
//! Two rules, both copied from the shell the user tested with:
//!
//!   * **resolution follows PATHEXT** (`.exe`, `.cmd`, `.bat`, `.com`), tried along `PATH`, with the
//!     bare name first - which is what `cmd.exe` does;
//!   * **a `.cmd`/`.bat` is wrapped in `cmd.exe /c`**, because `CreateProcess` cannot start a batch
//!     file by itself. That is also what the shell does; doing it here means the callers above do not
//!     have to know which kind of file they found.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The extensions a program name is tried with, in the order the platform itself tries them.
///
/// The empty string is last and deliberate: on Windows an **extensionless** file exists beside the real
/// one (npm writes `claude` - a POSIX shell script - next to `claude.cmd`), and starting that with
/// `CreateProcess` fails with `os error 193, %1 is not a valid Win32 application`. `cmd.exe` and
/// PowerShell both prefer the extensions, and so does this.
#[cfg(windows)]
const EXTENSIONS: &[&str] = &[".exe", ".cmd", ".bat", ".com", ""];

/// Everywhere else a program is a file with the execute bit, and its name is its name.
#[cfg(not(windows))]
const EXTENSIONS: &[&str] = &[""];

/// The names to look for, in order - extensions before the bare name on Windows, and the name as
/// written when it already carries one.
///
/// Pure, so the rule can be asserted without a filesystem.
pub fn candidates(program: &str) -> Vec<String> {
    let lower = program.to_ascii_lowercase();

    /* `node.exe` means `node.exe`, not `node.exe.exe`. */
    if EXTENSIONS.iter().any(|extension| !extension.is_empty() && lower.ends_with(extension)) {
        return vec![program.to_string()];
    }

    EXTENSIONS
        .iter()
        .map(|extension| format!("{program}{extension}"))
        .collect()
}

/// The path of `program`, or `None` when nothing on `PATH` can run it.
///
/// A name that already contains a separator is taken as a path and only extended, never searched for:
/// `/opt/x/claude` means that file, not a search hit.
pub fn resolve(program: &str) -> Option<PathBuf> {
    let names = candidates(program);

    if program.contains(['/', '\\']) {
        return names
            .into_iter()
            .map(PathBuf::from)
            .find(|candidate| candidate.is_file());
    }

    let path = std::env::var_os("PATH")?;

    for directory in std::env::split_paths(&path) {
        for name in &names {
            let candidate = directory.join(name);

            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// A `Command` that can actually start `program`.
///
/// `None` when it cannot be found - the caller keeps its own error message for that case, because the
/// sentence a user needs ("`claude` is not installed or not on PATH") belongs to the engine, not here.
pub fn command(program: &str) -> Option<Command> {
    let (executable, prefix) = launch(program)?;
    let mut command = Command::new(executable);

    command.args(prefix);

    Some(command)
}

/// The executable to start and the arguments that must come *before* the program's own.
///
/// This is the form both kinds of caller can use: `std::process::Command` (the doctor, the pty) and
/// `tokio::process::Command` (an engine), which cannot share a value but can both be told what to run.
/// A batch file comes back as `cmd.exe` with `/c <path>`, because `CreateProcess` cannot start one.
pub fn launch(program: &str) -> Option<(PathBuf, Vec<String>)> {
    let resolved = resolve(program)?;

    if needs_a_shell(&resolved) {
        return Some((
            PathBuf::from("cmd.exe"),
            vec!["/c".to_string(), resolved.to_string_lossy().into_owned()],
        ));
    }

    Some((resolved, Vec::new()))
}

/// The `Command` for an already-resolved path, with the batch-file wrap applied.
pub fn command_for(resolved: &Path) -> Command {
    if needs_a_shell(resolved) {
        let mut command = Command::new("cmd.exe");

        command.arg("/c").arg(resolved);

        return command;
    }

    Command::new(resolved)
}

/// True for a batch file, which `CreateProcess` cannot start on its own.
///
/// Windows only: on every other platform a `.cmd` is an ordinary file and wrapping it in a `cmd.exe`
/// that does not exist would be worse than useless. The Linux and macOS CI jobs caught exactly that
/// when this was not gated - the unit test that runs a `.cmd` failed on both.
#[cfg(windows)]
fn needs_a_shell(resolved: &Path) -> bool {
    let extension = resolved
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    extension == "cmd" || extension == "bat"
}

#[cfg(not(windows))]
fn needs_a_shell(_resolved: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    /* Only the Windows test touches the filesystem, so the import is gated with it - an unused import
       is an error under `clippy -D warnings`, which every release job runs. */
    #[cfg(windows)]
    use std::fs;

    /// The rule, without a filesystem: the platform's extensions first, the bare name last.
    ///
    /// That order is the fix for `os error 193`: npm writes an extensionless `claude` (a POSIX shell
    /// script) beside `claude.cmd`, so a resolver that prefers the bare name finds a file Windows
    /// cannot start.
    #[test]
    fn candidates_put_the_platforms_extensions_first() {
        let names = candidates("gemini");

        assert_eq!(names[0], if cfg!(windows) { "gemini.exe" } else { "gemini" });
        assert_eq!(names.last().map(String::as_str), Some("gemini"));

        if cfg!(windows) {
            assert!(names.contains(&"gemini.cmd".to_string()));
        }

        /* A name that already carries an extension is left alone. */
        assert_eq!(candidates("node.exe"), vec!["node.exe".to_string()]);
    }

    /// A name that is already a path is not searched for.
    #[test]
    fn a_path_is_taken_as_a_path() {
        assert!(resolve("./no/such/program-anywhere").is_none());
    }

    /// And the wrap: `npm`'s shim is a `.cmd`, so it is started through `cmd.exe`.
    ///
    /// Windows only, and that is not tidiness: on Linux and macOS this test *ran the `.cmd`*, failed,
    /// and turned three release jobs red before a single installer was built. A platform rule belongs in
    /// a test that names its platform.
    #[cfg(windows)]
    #[test]
    fn a_batch_file_is_started_through_a_shell() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let shim = directory.path().join("sdc-probe.cmd");

        fs::write(&shim, "@echo off\r\necho 9.9.9 (sdc-probe)\r\n").expect("writing the shim");

        assert!(needs_a_shell(&shim));
        assert_eq!(resolve(shim.to_str().expect("a path")).as_deref(), Some(shim.as_path()));

        /* The command runs the shim, so the answer is the shim's own output - not "file not found". */
        let output = command_for(&shim).arg("--version").output().expect("running it");

        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("sdc-probe"));
    }
}
