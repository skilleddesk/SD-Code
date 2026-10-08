//! Processes the agent leaves running (0.13): a dev server, a watcher, a queue worker.
//!
//! `run_command` waits for its command to end, which is right for a build and wrong for a server - the
//! agent used to be told never to start one, so it could not open the site it had just built, look at
//! it, or test it against a running API. Claude Code runs such commands in the background and reads
//! their output later; this is that, on both machines:
//!
//! * **this machine** - a child of the daemon, its stdout and stderr kept as the last lines in memory;
//! * **a host** - `nohup` over `ssh` into `~/.sdc/run/bg-<id>.log`, with the pid file `kill_line` uses,
//!   so a Stop reaches the process group there.
//!
//! A process outlives the turn that started it (the Preview needs the server after the answer) and is
//! stopped by `stop_process`, by the chat's Processes list, or when the daemon exits.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;
use crate::ssh::Ssh;

/// Lines of output kept per process.
const KEPT_LINES: usize = 2_000;
/// How many processes may run at once, all chats together.
const MAX_RUNNING: usize = 12;

struct Process {
    id: String,
    session_id: String,
    command: String,
    place: String,
    started: Instant,
    kind: Kind,
}

enum Kind {
    Local { child: Arc<Mutex<Child>>, pid: u32, lines: Arc<Mutex<VecDeque<String>>>, exit: Arc<Mutex<Option<i32>>> },
    Remote { ssh: Ssh, log: String, pid_file: String },
}

fn registry() -> &'static Mutex<Vec<Process>> {
    static REGISTRY: OnceLock<Mutex<Vec<Process>>> = OnceLock::new();

    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

fn next_id() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    format!("bg{}", COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Starts `line` in `root`, in the background. Answers the process id.
pub fn start(session_id: &str, root: &str, remote: Option<&Ssh>, line: &str) -> Result<String, ErrorObject> {
    if let Some(reason) = crate::pty::denied_reason_line(line) {
        return Err(ErrorObject::permission_denied(format!("{line}: {reason}")));
    }

    let running = list(None).iter().filter(|process| process["running"] == true).count();

    if running >= MAX_RUNNING {
        return Err(ErrorObject::bad_request(format!(
            "{running} background processes are already running; stop one (stop_process) before starting another"
        )));
    }

    let id = next_id();
    let kind = match remote {
        Some(ssh) => {
            let pid_file = format!("run/{}-{id}.pid", session_id.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>());
            let log = format!("{}/run/{}-{id}.log", crate::ssh::ops::state_dir(), session_id.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>());
            let wrapped = crate::ssh::ops::raw_line(line, Some(root), &pid_file)?;
            let script = format!(
                "mkdir -p {}/run; nohup sh -c {} > {log} 2>&1 < /dev/null & sleep 1; echo SDC-STARTED",
                crate::ssh::ops::state_dir(),
                crate::ssh::sh_quote(&wrapped)
            );
            let output = ssh.run(&script, Duration::from_secs(30))?;

            if !output.stdout.contains("SDC-STARTED") {
                return Err(ErrorObject::internal(format!("the process did not start on {}: {}", ssh.label(), output.reason())));
            }

            Kind::Remote { ssh: ssh.clone(), log, pid_file }
        }
        None => {
            let mut command = crate::pty::line_command(line);

            command.current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
            command.env("FORCE_COLOR", "0").env("NO_COLOR", "1").env("BROWSER", "none");

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;

                command.creation_flags(0x0800_0000);
            }

            crate::pty::own_group(&mut command);

            let mut child = command.spawn().map_err(|error| ErrorObject::internal(format!("could not start `{line}`: {error}")))?;
            let pid = child.id();
            let lines = Arc::new(Mutex::new(VecDeque::new()));
            let exit = Arc::new(Mutex::new(None));

            for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)]
                .into_iter()
                .flatten()
            {
                let lines = lines.clone();

                std::thread::spawn(move || {
                    for line in BufReader::new(stream).lines().map_while(Result::ok) {
                        if let Ok(mut kept) = lines.lock() {
                            kept.push_back(strip_ansi(&line));

                            while kept.len() > KEPT_LINES {
                                kept.pop_front();
                            }
                        }
                    }
                });
            }

            let child = Arc::new(Mutex::new(child));

            {
                let (child, exit) = (child.clone(), exit.clone());

                std::thread::spawn(move || loop {
                    let status = child.lock().ok().and_then(|mut child| child.try_wait().ok().flatten());

                    if let Some(status) = status {
                        if let Ok(mut exit) = exit.lock() {
                            *exit = Some(status.code().unwrap_or(-1));
                        }

                        break;
                    }

                    std::thread::sleep(Duration::from_millis(300));
                });
            }

            Kind::Local { child, pid, lines, exit }
        }
    };

    let place = remote.map(|ssh| ssh.label()).unwrap_or_else(|| "this machine".to_string());

    if let Ok(mut processes) = registry().lock() {
        processes.push(Process { id: id.clone(), session_id: session_id.to_string(), command: line.to_string(), place, started: Instant::now(), kind });
    }

    Ok(id)
}

/// The last `lines` lines a process printed, whether it is still running, and its exit code if not.
pub fn output(id: &str, lines: usize) -> Result<(String, bool, Option<i32>), ErrorObject> {
    let (remote, local) = {
        let processes = registry().lock().map_err(|_| ErrorObject::internal("process list is poisoned"))?;
        let process = processes
            .iter()
            .find(|process| process.id == id)
            .ok_or_else(|| ErrorObject::not_found(format!("there is no background process `{id}`")))?;

        match &process.kind {
            Kind::Remote { ssh, log, pid_file } => (Some((ssh.clone(), log.clone(), pid_file.clone())), None),
            Kind::Local { lines: kept, exit, .. } => {
                let kept = kept.lock().map(|kept| kept.iter().rev().take(lines).rev().cloned().collect::<Vec<_>>().join("\n")).unwrap_or_default();
                let exit = exit.lock().ok().and_then(|exit| *exit);

                (None, Some((kept, exit)))
            }
        }
    };

    if let Some((text, exit)) = local {
        return Ok((text, exit.is_none(), exit));
    }

    let (ssh, log, pid_file) = remote.expect("one of the two");
    let pid_path = format!("{}/{pid_file}", crate::ssh::ops::state_dir());
    let script = format!(
        "tail -n {lines} {log} 2>/dev/null; if [ -r {pid_path} ] && kill -0 \"$(cat {pid_path})\" 2>/dev/null; then echo; echo SDC-RUNNING; else echo; echo SDC-EXITED; fi"
    );
    let output = ssh.run(&script, Duration::from_secs(30))?;
    let running = output.stdout.contains("SDC-RUNNING");
    let text = output.stdout.replace("\nSDC-RUNNING", "").replace("\nSDC-EXITED", "").trim_end().to_string();

    Ok((strip_ansi(&text), running, None))
}

/// Stops a process: the tree on this machine, the group on a host. `false` when there was none.
pub fn stop(id: &str) -> bool {
    let process = registry().lock().ok().and_then(|mut processes| {
        let index = processes.iter().position(|process| process.id == id)?;

        Some(processes.remove(index))
    });

    let Some(process) = process else {
        return false;
    };

    match process.kind {
        Kind::Local { child, pid, .. } => {
            crate::pty::kill_tree(pid);

            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        Kind::Remote { ssh, pid_file, .. } => {
            let _ = ssh.run(&crate::ssh::ops::kill_line(&pid_file), Duration::from_secs(20));
        }
    }

    true
}

/// Stops every process of a chat (or all of them, with `None`) - the daemon's exit, a closed chat.
pub fn stop_all(session_id: Option<&str>) {
    let ids: Vec<String> = registry()
        .lock()
        .map(|processes| {
            processes
                .iter()
                .filter(|process| session_id.map(|session| process.session_id == session).unwrap_or(true))
                .map(|process| process.id.clone())
                .collect()
        })
        .unwrap_or_default();

    for id in ids {
        stop(&id);
    }
}

/// The processes, for the window's list and for the agent: id, command, where, how long, running.
pub fn list(session_id: Option<&str>) -> Vec<Value> {
    let Ok(processes) = registry().lock() else {
        return Vec::new();
    };

    processes
        .iter()
        .filter(|process| session_id.map(|session| process.session_id == session).unwrap_or(true))
        .map(|process| {
            let running = match &process.kind {
                Kind::Local { exit, .. } => exit.lock().map(|exit| exit.is_none()).unwrap_or(false),
                /* A host's process is asked about by `output`; the list does not open a connection per row. */
                Kind::Remote { .. } => true,
            };

            json!({
                "processId": process.id,
                "sessionId": process.session_id,
                "command": process.command,
                "place": process.place,
                "seconds": process.started.elapsed().as_secs(),
                "running": running,
            })
        })
        .collect()
}

/// A terminal's colour codes out of a line: the model reads text, not escapes.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();

                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }

            continue;
        }

        out.push(c);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_process_runs_prints_and_stops() {
        let root = std::env::temp_dir();
        let line = if cfg!(windows) { "echo ready& ping -n 30 127.0.0.1 >nul" } else { "echo ready; sleep 30" };
        let id = start("s-bg", root.to_str().unwrap(), None, line).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);

        loop {
            let (text, running, _) = output(&id, 20).unwrap();

            if text.contains("ready") {
                assert!(running, "still running after printing");
                break;
            }

            assert!(Instant::now() < deadline, "no output: {text:?}");
            std::thread::sleep(Duration::from_millis(100));
        }

        assert_eq!(list(Some("s-bg")).len(), 1);
        assert!(stop(&id));
        assert!(list(Some("s-bg")).is_empty());
        assert!(!stop(&id), "a stopped process is gone");
    }

    #[test]
    fn colour_codes_are_removed() {
        assert_eq!(strip_ansi("\u{1b}[32m  ➜  Local:\u{1b}[0m http://localhost:5173/"), "  ➜  Local: http://localhost:5173/");
    }

    /// The running-processes limiter: [`MAX_RUNNING`] processes may run at once, all chats together, so
    /// a caller past that is turned back - naming the count rather than queuing forever - and the next
    /// slot opens the moment one of them stops.
    ///
    /// Counted from whatever is already running rather than an assumed zero, since the registry is
    /// process-wide and another test's background process may still be alive alongside this one.
    #[test]
    fn starting_past_max_running_is_refused_until_one_stops() {
        let root = std::env::temp_dir();
        let root = root.to_str().unwrap();
        let line = if cfg!(windows) { "ping -n 30 127.0.0.1 >nul" } else { "sleep 30" };
        let session = "s-bg-limit";

        let already_running = list(None).iter().filter(|process| process["running"] == true).count();
        let needed = MAX_RUNNING.saturating_sub(already_running);

        assert!(needed > 0, "MAX_RUNNING already reached before this test started any process");

        let ids: Vec<String> = (0..needed).map(|_| start(session, root, None, line).unwrap()).collect();

        let error = start(session, root, None, line).unwrap_err();
        assert!(error.message.contains("already running"), "{}", error.message);

        assert!(stop(&ids[0]), "freeing a slot");

        let freed = start(session, root, None, line).unwrap();

        for id in ids[1..].iter().chain(std::iter::once(&freed)) {
            stop(id);
        }
    }
}
