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
use std::sync::{Arc, Mutex};
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

/// Kills a process **and everything it started** (0.11.2).
///
/// On Windows an npm-installed CLI runs as `cmd.exe /c gemini.cmd …`, and the program that does the
/// work is a `node` *grandchild*. `Child::kill` ends only the `cmd` - so every cancelled sign-in and
/// every stopped Gemini turn left its `node` running, holding its OAuth callback port (twelve of
/// them were found on the machine this was measured on). `taskkill /T` ends the whole tree. On Unix
/// the direct child is the program itself and `Child::kill` is enough, so this does nothing there.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        /* CREATE_NO_WINDOW: a console flash for every Stop would be its own small bug. */
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .status();
    }

    #[cfg(not(windows))]
    {
        let _ = pid;
    }
}

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

/// A command **line** for the platform's own shell: `sh -c <line>` here, `cmd /C <line>` on Windows.
///
/// It is the program-and-arguments pair a line turns into, so `shell.run`'s `line` parameter can travel
/// the same path as its `command` parameter - the same deny list, the same capture, the same tool-call
/// pair, and on a host the same `sh -c` on the far side (0.7.13).
pub fn shell_for_line(line: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".to_string(), vec!["/C".to_string(), line.to_string()])
    } else {
        ("sh".to_string(), vec!["-c".to_string(), line.to_string()])
    }
}

/// Why a whole command **line** is refused, or `None` when it is allowed through (0.7.13).
///
/// `denied_reason` checks a program and its arguments, which is right for an engine's `run` step. A
/// terminal line is not that shape: `echo hi && shutdown /s` is one line whose *second* statement is
/// the thing to refuse, and a check anchored at position zero would wave it through.
///
/// So the line is split into statements on the shell's own separators - `;`, `&&`, `||`, `|`, `&`, a
/// newline - and each statement is checked where a program would be (its start), collapsed like the
/// other check. A statement that is a shell handed a payload gets one level of unwrapping, the same
/// single level `denied_reason` does.
///
/// It is still not a sandbox, and the comment on `denied_reason` says why: quoting can hide a word
/// (`'shu'tdown`), a second shell level can escape, and anything compiled and run is out of reach. The
/// real protection is the permission gate, the checkpoint taken before the run, and the fact that the
/// command runs as the **user** - which is why the deny list stays a speed bump with a reason attached.
pub fn denied_reason_line(line: &str) -> Option<String> {
    if let Some(hit) = matches_denied(&collapse(line)) {
        return Some(hit);
    }

    for statement in line.split([';', '|', '&', '\n']) {
        if let Some(hit) = matches_denied(&collapse(statement)) {
            return Some(hit);
        }

        let words: Vec<String> = statement.split_whitespace().map(str::to_string).collect();

        if let Some((program, rest)) = words.split_first() {
            if is_shell(program) {
                /* The payload of a *line* is everything after the flag, not one argument: `sh -c 'rm -rf /'`
                   arrived as five words here, and the quotes around it are the line's, not the shell's. */
                let flag = rest.iter().position(|arg| {
                    matches!(arg.to_lowercase().as_str(), "-c" | "/c" | "-command" | "-commandwithargs")
                });

                if let Some(flag) = flag {
                    let payload = rest[flag + 1..].join(" ");
                    let payload = payload.trim().trim_matches(['\'', '"']);

                    if let Some(hit) = matches_denied(&collapse(payload)) {
                        return Some(format!("{hit} (inside `{program}`)"));
                    }
                }
            }
        }
    }

    None
}

/// How much of a stream is kept. A command that prints a gigabyte must not become a gigabyte in the
/// event log; the rest is still drained (so the child never blocks) and then reported as truncated.
const CAPTURE_LIMIT: usize = 256 * 1024;

/// How many output lines are kept per long-running process. A login flow prints its URL in the first
/// handful; a dev server's log is its tail. 400 covers both and stays small.
pub const OUTPUT_LINES: usize = 400;

/// What the daemon knows about one long-running process: its output tail and whether it is alive.
///
/// It exists because `pty.open` used to be fire-and-forget, and a process you cannot read is a process
/// you cannot drive - which is exactly what a login flow needs: a CLI prints a URL, waits for a code on
/// its stdin, and says when it is done.
#[derive(Debug, Clone)]
pub struct Session {
    /// The program that was started (`claude`, `npm`, `ssh`).
    pub program: String,
    /// What this process is *for*, when the caller knows: `claude --resume`, or `deploy.sh on prod-1`.
    ///
    /// A remote process is an `ssh`, and a UI told that it is running `ssh` has been told nothing
    /// (0.7.13). The label is what a person sees.
    pub label: String,
    /// The host this process runs on, and the pid file that stops it, when it is not this machine.
    pub remote: Option<(crate::ssh::Ssh, String)>,
    pub lines: Vec<String>,
    /// What each stream (stdout, stderr) has printed since its last newline (0.11.2).
    ///
    /// A process that asks a question prints it **without a newline** and waits: Gemini's `Do you
    /// want to continue? [Y/n]:` is exactly that. A line reader never yields such a line, so the
    /// question was invisible to everything reading this ring - the login dialog, and the code that
    /// should have answered it. Output now includes these tails after the complete lines.
    pub partials: [String; 2],
    pub state: String,
    pub started: Instant,
}

impl Session {
    fn new(program: &str, label: Option<&str>, remote: Option<(crate::ssh::Ssh, String)>) -> Self {
        Self {
            program: program.to_string(),
            label: label.unwrap_or(program).to_string(),
            remote,
            lines: Vec::new(),
            partials: [String::new(), String::new()],
            state: "running".to_string(),
            started: Instant::now(),
        }
    }

    /// Appends one line, dropping the oldest when the ring is full.
    fn push_line(&mut self, line: String) {
        if self.lines.len() >= OUTPUT_LINES {
            self.lines.remove(0);
        }

        self.lines.push(line);
    }

    /// Everything printed so far, which is what a URL search and a log view both read.
    pub fn text(&self) -> String {
        self.visible_lines().join("\n")
    }

    /// The complete lines, then any half-written tail a stream is still sitting on.
    pub fn visible_lines(&self) -> Vec<String> {
        let mut lines = self.lines.clone();

        for partial in &self.partials {
            let tail = partial.trim_end_matches('\r');

            if !tail.trim().is_empty() {
                lines.push(tail.to_string());
            }
        }

        lines
    }
}

/// The registry of processes the daemon started: the long-running ones by id, plus one-shot runs.
#[derive(Default)]
pub struct PtyManager {
    children: Mutex<HashMap<String, Child>>,
    /// The output tail of each long-running process, filled by its reader threads.
    sessions: Arc<Mutex<HashMap<String, Session>>>,
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

    /// Spawns a command and keeps it - and, unlike a bare spawn, **keeps reading it**.
    ///
    /// The answer is what `pty.open` returns: an id, the program, and `tty: false` for the reason in
    /// the module doc. Its output arrives through `pty.output`, which keeps the last `OUTPUT_LINES`
    /// lines; a process that is never read is a process that blocks on a full pipe once it has said
    /// enough, and a login URL is exactly the thing that would be stuck in it.
    /// `label` is what a UI shows as "what is running" - the caller's own words, because a remote
    /// process's program is `ssh` (0.7.13). `remote` is the host it runs on and the pid file that stops
    /// it, when `close` has to kill a process on the far side rather than only this child.
    pub fn open(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&str>,
        label: Option<&str>,
        remote: Option<(crate::ssh::Ssh, String)>,
    ) -> Result<Value, ErrorObject> {
        /* The same resolution the engine uses: a CLI installed by npm on Windows is a `.cmd` shim, and
           `pty.open` is how the daemon drives that CLI's own login (`cli.login`). */
        let mut process = match crate::host::program::launch(command) {
            Some((executable, prefix)) => {
                let mut process = Command::new(executable);

                process.args(prefix);

                process
            }
            None => Command::new(command),
        };

        process.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(cwd) = cwd {
            process.current_dir(cwd);
        }

        let mut child = process.spawn().map_err(|error| {
            ErrorObject::internal(format!("`{command}` could not be started: {error}"))
        })?;
        let id = {
            let mut next = self.next_id.lock().map_err(|_| ErrorObject::internal("pty registry poisoned"))?;

            *next += 1;

            format!("pty-{}", *next)
        };

        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(id.clone(), Session::new(command, label, remote));
        }

        let mut readers: Vec<(usize, Box<dyn Read + Send>)> = Vec::new();

        if let Some(stdout) = child.stdout.take() {
            readers.push((0, Box::new(stdout)));
        }

        if let Some(stderr) = child.stderr.take() {
            readers.push((1, Box::new(stderr)));
        }

        /* One reader per stream, both appending into the session's ring. They end when the process
           closes its streams, which is also when the state flips to `exited`. Bytes, not lines: a
           question printed without a newline is kept as that stream's partial, visible at once. */
        for (stream, mut reader) in readers {
            let sessions = self.sessions.clone();
            let session_id = id.clone();

            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                let mut pending: Vec<u8> = Vec::new();

                loop {
                    let read = match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };

                    pending.extend_from_slice(&chunk[..read]);

                    let Ok(mut sessions) = sessions.lock() else {
                        break;
                    };
                    let Some(session) = sessions.get_mut(&session_id) else {
                        continue;
                    };

                    while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                        let raw: Vec<u8> = pending.drain(..=end).collect();

                        session.push_line(String::from_utf8_lossy(&raw).trim_end_matches(['\r', '\n']).to_string());
                    }

                    session.partials[stream] = String::from_utf8_lossy(&pending).to_string();
                }

                /* A last line with no newline at exit is still a line. */
                if let Ok(mut sessions) = sessions.lock() {
                    if let Some(session) = sessions.get_mut(&session_id) {
                        let tail = std::mem::take(&mut session.partials[stream]);

                        if !tail.trim().is_empty() {
                            session.push_line(tail.trim_end_matches('\r').to_string());
                        }
                    }
                }
            });
        }

        self.children
            .lock()
            .map_err(|_| ErrorObject::internal("pty registry poisoned"))?
            .insert(id.clone(), child);

        Ok(json!({ "ptyId": id, "command": command, "tty": false }))
    }

    /// The output tail of a long-running process, and whether it is still alive.
    ///
    /// This is what a login flow polls: the URL to show, the log to read, and the moment the CLI says
    /// it is done. `pty.output` on an id that never existed is a `not_found`, because a UI asking about
    /// a process the daemon does not have is a UI that has lost track of itself.
    pub fn output(&self, id: &str) -> Result<Value, ErrorObject> {
        let state = self.state_of(id);
        let sessions = self.sessions.lock().map_err(|_| ErrorObject::internal("pty registry poisoned"))?;
        let session = sessions
            .get(id)
            .ok_or_else(|| ErrorObject::not_found(format!("{id} is not a process this daemon started")))?;

        let lines = session.visible_lines();

        Ok(json!({
            "ptyId": id,
            "command": session.label,
            "state": state,
            "lineCount": lines.len(),
            "lines": lines,
            "ms": session.started.elapsed().as_millis() as u64,
        }))
    }

    /// `running`, `exited` or `gone`. Read from the child rather than remembered, so it cannot drift.
    pub fn state_of(&self, id: &str) -> String {
        let Ok(mut children) = self.children.lock() else {
            return "gone".to_string();
        };

        match children.get_mut(id) {
            None => "gone".to_string(),
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => "exited".to_string(),
                Ok(None) => "running".to_string(),
                Err(_) => "gone".to_string(),
            },
        }
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
    ///
    /// A process on a **host** takes one more step (0.7.13): dropping the local `ssh` closes the
    /// connection, and the remote process may keep running - so the pid file `ssh::ops::process_line`
    /// wrote is used to signal its **process group** there, on a detached thread so a Stop button does
    /// not wait on a slow link (the same rule `engines::cli::kill` follows for a turn).
    pub fn close(&self, id: &str) -> bool {
        let remote = self
            .sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(id).and_then(|session| session.remote.clone()));

        if let Some((ssh, pid_file)) = remote {
            std::thread::spawn(move || {
                let line = crate::ssh::ops::kill_line(&pid_file);

                let _ = ssh.run(&line, Duration::from_secs(15));
            });
        }

        let Ok(mut children) = self.children.lock() else {
            return false;
        };

        match children.remove(id) {
            Some(mut child) => {
                kill_tree(child.id());
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
        let error = manager.open("definitely-not-a-program-sdcd", &[], None, None, None).unwrap_err();

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
        let opened = manager.open(&program, &args, None, None, None).unwrap();
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

    /// A **line** is checked statement by statement, which is what a terminal needs (0.7.13).
    ///
    /// The case that matters is the refused program that is not the first word: anchored-at-zero would
    /// wave `git status && shutdown /s` straight through, and a terminal is where somebody types that.
    #[test]
    fn refuses_a_denied_program_anywhere_in_a_line() {
        for line in [
            "shutdown /s",
            "git status && shutdown /s",
            "echo hi ; rm -rf /",
            "cat notes.txt | reboot",
            "sleep 1 & dd if=/dev/zero of=/dev/sda",
            "sh -c 'rm -rf /'",
            "ls\nshutdown -h now",
        ] {
            assert!(denied_reason_line(line).is_some(), "{line} should be refused");
        }

        for line in [
            "git status -s",
            "pnpm test -- --run",
            "echo \"rm -rf /tmp/scratch\"",
            "grep -rn 'shutdown' ./src",
            "docker compose logs --tail=50 api",
        ] {
            assert!(denied_reason_line(line).is_none(), "{line} should be allowed: {:?}", denied_reason_line(line));
        }

        /* Printing a dangerous-looking string is still not running it - the same rule `denied_reason`
           follows, one statement at a time. */
        assert!(denied_reason_line("printf '%s' 'shutdown -h now'").is_none());
    }

    /// A line becomes a program and arguments for the platform's own shell - one behaviour per platform.
    #[test]
    fn a_line_becomes_the_platform_shell() {
        let (program, args) = shell_for_line("git status -s");
        let expected = if cfg!(windows) { "cmd" } else { "sh" };

        assert_eq!(program, expected);
        assert_eq!(args.len(), 2);
        assert!(args[0].eq_ignore_ascii_case("-c") || args[0].eq_ignore_ascii_case("/c"));
        assert_eq!(args[1], "git status -s", "the line is handed over whole, not split");
        /* The pair is a legal shell call, so the guard can look inside it - and it does. */
        assert!(denied_reason(&program, &shell_for_line("shutdown /s").1).is_some());
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


