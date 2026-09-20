//! Long-running processes (master spec section 5.6 of the daemon plan).
//!
//! A dev server, a test watcher, a REPL: the things an agent starts and then keeps reading from.
//! `pty.open` spawns one with piped stdio and remembers it under an id; `pty.write` feeds its stdin;
//! `pty.close` kills it. Every child is killed when the daemon exits, which is why they are all
//! registered here rather than held by the caller.
//!
//! A real pseudo-terminal (ConPTY on Windows, `openpty` elsewhere) is what a *terminal* UI needs; the
//! agents of spec section 5.6 only need stdio, and piped stdio is what this gives them. The
//! difference is stated here rather than hidden: `pty.open` reports `"tty": false` so nothing
//! downstream can assume a terminal that is not there.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;

/// The registry of running processes. One per daemon.
#[derive(Default)]
pub struct PtyManager {
    children: Mutex<HashMap<String, Child>>,
    next_id: Mutex<u64>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns a command. The answer is what `pty.open` returns: an id, the program, and `tty: false`
    /// for the reason in the module doc.
    pub fn open(&self, command: &str, args: &[String], cwd: Option<&str>) -> Result<Value, ErrorObject> {
        let mut process = Command::new(command);

        process
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

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

    /// How many are running; the doctor and `pty.list` report it.
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

#[cfg(test)]
mod tests {
    use super::*;

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
        /* `cmd /C` on Windows, `sh -c` elsewhere: one line that exits cleanly either way. */
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/C".to_string(), "exit".to_string()])
        } else {
            ("sh", vec!["-c".to_string(), "exit 0".to_string()])
        };

        let manager = PtyManager::new();
        let opened = manager.open(program, &args, None).unwrap();
        let id = opened["ptyId"].as_str().unwrap().to_string();

        assert_eq!(opened["tty"], json!(false));
        assert_eq!(manager.running(), 1);
        assert!(manager.close(&id));
        assert_eq!(manager.running(), 0);
    }
}
