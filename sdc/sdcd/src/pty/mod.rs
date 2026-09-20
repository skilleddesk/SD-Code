//! Long-running processes *and* one-shot commands (master spec section 5.6).
//!
//! Two shapes, one module, because they are the same mechanism with different lifetimes:
//!
//! * **`pty.open` / `pty.write` / `pty.close`** - a dev server, a test watcher, a REPL: something an
//!   agent starts and then keeps reading from. It is remembered under an id and killed on shutdown.
//! * **`shell.run`** - one command, its output captured, its exit code reported. This is the step an
//!   agent loop needs to *act*: run a build, run the tests, install a dependency, and see what came
//!   back. Without it the daemon could describe work but never do any.
//!
//! A real pseudo-terminal (ConPTY on Windows, `openpty` elsewhere) is what a *terminal* UI needs; the
//! agents of spec section 5.6 only need stdio, and piped stdio is what this gives them. The
//! difference is stated here rather than hidden: `pty.open` reports `"tty": false` so nothing
//! downstream can assume a terminal that is not there.
//!
//! ## What `shell.run` is not
//!
//! It is not a sandbox. The deny list below is a **speed bump with a reason attached**, not a security
//! boundary: a determined command can still destroy the machine, which is exactly why the app asks for
//! permission before a mutating action and writes a checkpoint before it happens (principles P5 and
//! P6). Saying that here is worth more than a longer regular-expression list that would give a false
//! sense of safety.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;

/// Patterns refused outright, as `(pattern, why)`. Matched against the joined command line, lowercased.
pub const DENIED: &[(&str, &str)] = &[
    ("rm -rf /", "it deletes the filesystem root"),
    ("mkfs", "it formats a filesystem"),
    ("format c:", "it formats the system drive"),
    ("diskpart", "it rewrites the disk's partitions"),
    ("del /f /s /q c:\\", "it empties the system drive"),
    ("shutdown", "it powers the machine off under the user"),
    ("reboot", "it reboots the machine under the user"),
    ("dd if=", "it writes raw bytes to a device"),
    (":(){:|:&};:", "it is a fork bomb"),
    ("git push --force", "it rewrites published history"),
];

/// Why a command line is refused, or `None` when it is allowed through.
///
/// Matching is anchored at the **start** of the line, because that is where the program is: a pattern
/// buried in an argument is somebody else's business - `echo "rm -rf /tmp"` is a command that prints
/// text, and refusing it would make the deny list useless by making it unpredictable.
///
/// The one bypass that anchoring opens is a shell, so a shell's payload is checked too:
/// `sh -c "rm -rf /"` is the same intent as a bare `rm -rf /`, and one level of unwrapping catches it.
/// It is still not a sandbox - a second level (`sh -c 'sh -c …'`) gets through, and so does anything
/// compiled and run - which is why the permission gate and the checkpoint remain the real protection.
pub fn denied_reason(command: &str, args: &[String]) -> Option<String> {
    if let Some(hit) = matches_denied(&collapse(&format!("{command} {}", args.join(" ")))) {
        return Some(hit);
    }

    if is_shell(command) {
        if let Some(payload) = shell_payload(args) {
            if let Some(hit) = matches_denied(&collapse(payload)) {
                return Some(format!("{hit} (inside `{command}`)"));
            }
        }
    }

    None
}

/// The denied pattern a command line starts with, with the reason attached.
fn matches_denied(line: &str) -> Option<String> {
    DENIED
        .iter()
        .find(|(pattern, _)| line.starts_with(pattern))
        .map(|(pattern, why)| format!("`{pattern}` was refused because {why}"))
}

/// Whitespace collapsed to single spaces, lowercased - so `rm   -rf   /` matches like `rm -rf /`.
fn collapse(line: &str) -> String {
    line.split_whitespace().collect::<Vec<&str>>().join(" ").to_lowercase()
}

/// A shell that can be handed a command line of its own.
fn is_shell(command: &str) -> bool {
    let name = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .trim_end_matches(".exe")
        .to_lowercase();

    ["sh", "bash", "zsh", "dash", "cmd", "powershell", "pwsh"].contains(&name.as_str())
}

/// What a shell was asked to run: `sh -c <line>`, `cmd /c <line>`, `powershell -Command <line>`.
fn shell_payload(args: &[String]) -> Option<&str> {
    let flag = args.iter().position(|arg| {
        matches!(arg.to_lowercase().as_str(), "-c" | "/c" | "-command" | "-commandwithargs")
    })?;

    args.get(flag + 1).map(String::as_str)
}

/// How much of a stream is kept. A command that prints a gigabyte must not become a gigabyte in the
/// event log; the rest is still drained (so the child never blocks) and then reported as truncated.
const CAPTURE_LIMIT: usize = 256 * 1024;

/// The registry of processes the daemon started: the long-running ones by id, plus one-shot runs.
#[derive(Default)]
pub struct PtyManager {
    children: Mutex<HashMap<String, Child>>,
    next_id: Mutex<u64>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// One command, run to completion or to its timeout, with both streams captured.
    ///
    /// The answer is what `shell.run` returns and what an agent loop reads: the exit code, the output,
    /// how long it took, whether it was killed for time, and - when it failed - the *translated*
    /// failure (spec section 14.9), so a caller does not have to parse a stack trace to say something
    /// useful.
    pub fn run_once(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, ErrorObject> {
        if let Some(reason) = denied_reason(command, args) {
            return Err(ErrorObject::permission_denied(format!("{command}: {reason}")));
        }

        let mut process = Command::new(command);

        process.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(cwd) = cwd {
            process.current_dir(cwd);
        }

        let started = Instant::now();
        let mut child = process
            .spawn()
            .map_err(|error| ErrorObject::internal(format!("`{command}` could not be started: {error}")))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        /* Both streams are read on their own thread: a child that fills one pipe while the other is
           being read would block for ever otherwise, and the timeout would then be the only way out.
           The threads are *detached* rather than joined, and their results arrive over a channel with
           a short deadline: killing a shell does not always kill what it spawned (there is no process
           group in std), and a grandchild that inherited the pipe would otherwise keep this call
           waiting for as long as it likes. What was captured arrives; the straggler is left behind. */
        let (out_tx, out_rx) = std::sync::mpsc::channel();
        let (err_tx, err_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let _ = out_tx.send(capture(stdout));
        });
        std::thread::spawn(move || {
            let _ = err_tx.send(capture(stderr));
        });

        let mut timed_out = false;
        let mut finished = None;

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    finished = Some(status);
                    break;
                }
                Ok(None) => {}
                Err(error) => return Err(ErrorObject::internal(error.to_string())),
            }

            if started.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }

            std::thread::sleep(Duration::from_millis(25));
        }

        let budget = Duration::from_millis(750);
        let (stdout, out_truncated) = out_rx.recv_timeout(budget).unwrap_or_else(|_| (String::new(), true));
        let (stderr, err_truncated) = err_rx.recv_timeout(budget).unwrap_or_else(|_| (String::new(), true));
        let exit_code = finished.and_then(|status| status.code());
        let ok = !timed_out && finished.map(|status| status.success()).unwrap_or(false);

        Ok(json!({
            "command": command,
            "args": args,
            "cwd": cwd,
            "exitCode": exit_code,
            "ok": ok,
            "stdout": stdout,
            "stderr": stderr,
            "durationMs": started.elapsed().as_millis() as u64,
            "timedOut": timed_out,
            "truncated": out_truncated || err_truncated,
            "error": if ok { Value::Null } else { translated(command, &stderr, &stdout, timed_out) },
        }))
    }

    /// Spawns a command and keeps it. The answer is what `pty.open` returns: an id, the program, and
    /// `tty: false` for the reason in the module doc.
    pub fn open(&self, command: &str, args: &[String], cwd: Option<&str>) -> Result<Value, ErrorObject> {
        let mut process = Command::new(command);

        process.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(cwd) = cwd {
            process.current_dir(cwd);
        }

        let child = process.spawn().map_err(|error| {
            ErrorObject::internal(format!("`{command}` could not be started: {error}"))
        })?;
        let id = {
            let mut next = self.next_id.lock().map_err(|_| ErrorObject::internal("pty registry poisoned"))?;

            *next += 1;

            format!("pty-{}", *next)
        };

        self.children
            .lock()
            .map_err(|_| ErrorObject::internal("pty registry poisoned"))?
            .insert(id.clone(), child);

        Ok(json!({ "ptyId": id, "command": command, "tty": false }))
    }

    /// Writes to a process's stdin. Bytes, not text: a REPL's prompt is not UTF-8-shaped.
    pub fn write(&self, id: &str, data: &str) -> Result<(), ErrorObject> {
        let mut children = self.children.lock().map_err(|_| ErrorObject::internal("pty registry poisoned"))?;
        let child = children
            .get_mut(id)
            .ok_or_else(|| ErrorObject::not_found(format!("{id} is not running")))?;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| ErrorObject::internal(format!("{id} has no stdin")))?;

        stdin.write_all(data.as_bytes()).map_err(|error| ErrorObject::internal(error.to_string()))?;
        stdin.flush().map_err(|error| ErrorObject::internal(error.to_string()))?;

        Ok(())
    }

    /// Kills one process. `false` when the id was already gone, which is not an error - a client that
    /// closes twice is not a client that is wrong.
    pub fn close(&self, id: &str) -> bool {
        let Ok(mut children) = self.children.lock() else {
            return false;
        };

        match children.remove(id) {
            Some(mut child) => {
                let _ = child.kill();
                let _ = child.wait();

                true
            }
            None => false,
        }
    }

    /// How many are running; `host.status` reports it.
    pub fn running(&self) -> usize {
        self.children.lock().map(|children| children.len()).unwrap_or(0)
    }

    /// Kills everything. Called on shutdown so a dev server does not outlive the daemon.
    pub fn close_all(&self) {
        if let Ok(mut children) = self.children.lock() {
            for (_, mut child) in children.drain() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

}

/// Reads a stream up to `CAPTURE_LIMIT`, then keeps draining it. Returns `(text, truncated)`.
fn capture(stream: Option<impl Read>) -> (String, bool) {
    let Some(mut stream) = stream else {
        return (String::new(), false);
    };

    let mut kept = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;

    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if kept.len() < CAPTURE_LIMIT {
                    let room = CAPTURE_LIMIT - kept.len();

                    kept.extend_from_slice(&buffer[..read.min(room)]);

                    if read > room {
                        truncated = true;
                    }
                } else {
                    /* Still drained, so the child never blocks on a full pipe. */
                    truncated = true;
                }
            }
        }
    }

    (String::from_utf8_lossy(&kept).to_string(), truncated)
}

/// The plain-English failure a non-zero exit gets, so a caller never has to read a stack trace.
fn translated(command: &str, stderr: &str, stdout: &str, timed_out: bool) -> Value {
    if timed_out {
        let translation = crate::errors::translator::translate(command, "timed out");

        return json!({
            "title": translation.title,
            "explanation": "The command was still running when its timeout expired, so it was stopped.",
            "rule": translation.rule,
            "fixable": translation.fixable,
        });
    }

    /* stderr first, then stdout: a compiler writes its errors to the former and its summary to the
       latter, and the former is usually the sentence worth showing. */
    let source = if stderr.trim().is_empty() { stdout } else { stderr };
    let translation = crate::errors::translator::translate(command, source);

    json!({
        "title": translation.title,
        "explanation": translation.explanation,
        "rule": translation.rule,
        "fixable": translation.fixable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cmd /C …` on Windows, `sh -c …` elsewhere: one line per platform, one behaviour.
    fn shell(line: &str) -> (String, Vec<String>) {
        if cfg!(windows) {
            ("cmd".to_string(), vec!["/C".to_string(), line.to_string()])
        } else {
            ("sh".to_string(), vec!["-c".to_string(), line.to_string()])
        }
    }

    #[test]
    fn reports_a_missing_program_without_panicking() {
        let manager = PtyManager::new();
        let error = manager.open("definitely-not-a-program-sdcd", &[], None).unwrap_err();

        assert_eq!(error.code, "internal");
        assert_eq!(manager.running(), 0);
    }

    #[test]
    fn closing_an_unknown_process_is_not_an_error() {
        assert!(!PtyManager::new().close("pty-404"));
    }

    #[test]
    fn opens_writes_and_closes_a_real_process() {
        let (program, args) = shell("exit 0");
        let manager = PtyManager::new();
        let opened = manager.open(&program, &args, None).unwrap();
        let id = opened["ptyId"].as_str().unwrap().to_string();

        assert_eq!(opened["tty"], json!(false));
        assert_eq!(manager.running(), 1);
        assert!(manager.close(&id));
        assert_eq!(manager.running(), 0);
    }

    #[test]
    fn runs_a_command_and_captures_its_own_output() {
        let (program, args) = shell("echo sdcd-shell-run");
        let result = PtyManager::new().run_once(&program, &args, None, Duration::from_secs(20)).unwrap();

        assert_eq!(result["ok"], json!(true));
        assert_eq!(result["exitCode"], json!(0));
        assert_eq!(result["timedOut"], json!(false));
        assert!(result["stdout"].as_str().unwrap().contains("sdcd-shell-run"));
        assert!(result["durationMs"].as_u64().is_some());
        assert_eq!(result["error"], Value::Null);
    }

    /// A failing command is the interesting case: the caller gets a *sentence*, not a stack trace.
    ///
    /// The command is `exit 3` rather than something cleverer, so the test says the same thing on
    /// every machine: a non-zero exit is a failure, and a failure always arrives with a translation.
    #[test]
    fn a_failing_command_comes_back_with_a_translated_failure() {
        let (program, args) = shell("exit 3");
        let result = PtyManager::new().run_once(&program, &args, None, Duration::from_secs(20)).unwrap();

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["exitCode"], json!(3));
        assert_ne!(result["error"], Value::Null);
        assert!(result["error"]["title"].is_string());
        assert!(!result["error"]["explanation"].as_str().unwrap().is_empty());
    }

    /// The message a tool prints is what the card shows: `stderr` first, then `stdout`.
    #[test]
    fn the_failure_sentence_uses_what_the_command_printed() {
        let (program, args) = shell("echo 'cannot find module abc' 1>&2 & exit 1");

        let result = PtyManager::new().run_once(&program, &args, None, Duration::from_secs(20)).unwrap();

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["error"]["rule"], json!("missing-module"));
        assert!(result["error"]["explanation"].as_str().unwrap().contains("cannot find module"));
    }

    /// The deny list, as a unit test: the reason is what a user reads, so it is asserted too.
    #[test]
    fn refuses_the_commands_that_destroy_a_machine() {
        let refused = denied_reason("rm", &["-rf".to_string(), "/".to_string()]).unwrap();

        assert!(refused.contains("filesystem root"));
        assert!(denied_reason("shutdown", &["/s".to_string()]).is_some());
        assert!(denied_reason("git", &["push".to_string(), "--force".to_string()]).is_some());

        /* And the refusal is a `permission_denied`, not an internal error. */
        let error = PtyManager::new()
            .run_once("rm", &["-rf".to_string(), "/".to_string()], None, Duration::from_secs(5))
            .unwrap_err();

        assert_eq!(error.code, "permission_denied");
    }

    #[test]
    fn allows_the_commands_an_agent_actually_needs() {
        for (program, args) in [
            ("cargo", vec!["test"]),
            ("pnpm", vec!["install"]),
            ("git", vec!["status", "--porcelain"]),
            ("node", vec!["-e", "console.log(1)"]),
            /* The false positive this anchoring exists to avoid: printing a dangerous-looking string
               is not running it. */
            ("echo", vec!["rm -rf /tmp/scratch"]),
            ("printf", vec!["%s", "shutdown -h now"]),
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_string).collect();

            assert!(denied_reason(program, &args).is_none(), "{program} should be allowed");
        }
    }

    /// A shell is the one place the anchoring could be bypassed, so its payload is checked too.
    #[test]
    fn looks_inside_a_shell_that_was_handed_a_command() {
        let inside = denied_reason("sh", &["-c".to_string(), "rm -rf /".to_string()]).unwrap();

        assert!(inside.contains("filesystem root"));
        assert!(inside.contains("inside `sh`"));
        assert!(denied_reason("cmd", &["/C".to_string(), "shutdown /s".to_string()]).is_some());
        assert!(denied_reason("bash", &["-c".to_string(), "ls -la".to_string()]).is_none());
    }

    /// A command that never finishes is stopped, and says so - rather than hanging a turn for ever.
    #[test]
    fn a_command_that_overruns_its_timeout_is_killed() {
        let (program, args) = if cfg!(windows) {
            ("cmd".to_string(), vec!["/C".to_string(), "ping -n 30 127.0.0.1 > NUL".to_string()])
        } else {
            ("sh".to_string(), vec!["-c".to_string(), "sleep 30".to_string()])
        };
        let started = Instant::now();
        let result = PtyManager::new().run_once(&program, &args, None, Duration::from_millis(600)).unwrap();

        assert_eq!(result["timedOut"], json!(true));
        assert_eq!(result["ok"], json!(false));
        assert!(started.elapsed() < Duration::from_secs(20), "it did not wait for the child");
        assert!(result["error"]["explanation"].as_str().unwrap().contains("timeout"));
    }
}


