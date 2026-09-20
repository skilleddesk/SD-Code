//! The shared CLI adapter (master spec section 11.2).
//!
//! `claude`, `codex` and `gemini` are three programs with one shape: a prompt on stdin, one JSON
//! object per line on stdout, and a child process that has to be killable. This module is that
//! shape, so the three adapters below it are a dozen lines each and cannot drift apart.
//!
//! Two properties of the real CLIs drive the design:
//!
//! * **`--include-partial-messages` is mandatory** for Claude Code (spec section 11.2 without it the
//!   CLI prints the answer once at the end, and the app's streaming turn would be a lie).
//! * **the child must die on cancel.** A `Command` handle is kept per turn id so `engine.cancel`
//!   can kill the process group, not merely stop reading from it.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::engines::{collect_stream, EngineEvent, Prompt};

/// How one CLI is invoked. The differences between the three adapters, as data.
#[derive(Debug, Clone)]
pub struct CliSpec {
    /// The program on `PATH`.
    pub program: &'static str,
    /// Flags that ask for the structured stream. Empty for a CLI that always streams JSON.
    pub args: &'static [&'static str],
    /// Extra environment the CLI needs to be quiet: `NO_COLOR` and friends.
    pub env: &'static [(&'static str, &'static str)],
}

/// The running children, so a cancel can kill one.
pub struct CliAdapter {
    spec: CliSpec,
    children: Arc<Mutex<HashMap<String, u32>>>,
}

impl Default for CliAdapter {
    fn default() -> Self {
        Self { spec: crate::engines::claude_code::CLAUDE_SPEC, children: Arc::new(Mutex::new(HashMap::new())) }
    }
}

impl CliAdapter {
    pub fn new(spec: CliSpec) -> Self {
        Self { spec, children: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn spec(&self) -> &CliSpec {
        &self.spec
    }

    /// Spawns the CLI, writes the prompt (plus the replay history of a bridged session), and reads
    /// the structured stream until it ends.
    pub async fn run(&self, prompt: &Prompt) -> Vec<EngineEvent> {
        /* `host::program` resolves the name the way the shell does - which is what makes an
           npm-installed CLI (`claude.cmd`, `codex.cmd`, `gemini.cmd` on Windows) startable at all. The
           fallback keeps the old behaviour for a name that cannot be found, so the failure still
           arrives as the missing-program sentence rather than as `None`. */
        let mut command = match crate::host::program::launch(self.spec.program) {
            Some((executable, prefix)) => {
                let mut command = Command::new(executable);

                command.args(prefix);

                command
            }
            None => Command::new(self.spec.program),
        };

        command
            .args(self.spec.args)
            .envs(self.spec.env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child: Child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                /* A missing CLI is a *check* the doctor reports, not a crash: the turn answers with
                   the reason in plain words (spec section 14.9). */
                return vec![EngineEvent::Failed(missing_program_message(self.spec.program, &error))];
            }
        };

        if let Some(id) = child.id() {
            if let Ok(mut children) = self.children.lock() {
                children.insert(prompt.turn_id.clone(), id);
            }
        }

        if let Some(mut stdin) = child.stdin.take() {
            let mut body = String::new();

            for message in &prompt.history {
                body.push_str(message);
                body.push('\n');
            }

            body.push_str(&prompt.text);

            let _ = stdin.write_all(body.as_bytes()).await;
            let _ = stdin.flush().await;
            /* Closing stdin is what tells all three CLIs the prompt is complete. */
            drop(stdin);
        }

        let mut lines = Vec::new();

        if let Some(stdout) = child.stdout.take() {
            let mut reader = BufReader::new(stdout).lines();

            while let Ok(Some(line)) = reader.next_line().await {
                lines.push(line);
            }
        }

        let _ = child.wait().await;

        if let Ok(mut children) = self.children.lock() {
            children.remove(&prompt.turn_id);
        }

        collect_stream(lines)
    }

    /// Kills the child of one turn. `engine.kill` and `engine.cancel` both land here; the difference
    /// between them is the event the daemon emits, not how the process dies.
    pub fn kill(&self, turn_id: &str) -> bool {
        let Ok(mut children) = self.children.lock() else {
            return false;
        };

        children.remove(turn_id).is_some()
    }

    /// True while a turn's child is alive - the input to the `Running`/`Stuck` decision.
    pub fn is_running(&self, turn_id: &str) -> bool {
        self.children.lock().map(|children| children.contains_key(turn_id)).unwrap_or(false)
    }
}

/// The two-line explanation the error translator uses for a CLI that is not installed.
pub fn missing_program_message(program: &str, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("`{program}` is not installed or not on PATH. Install it, then run the environment doctor.")
    } else {
        format!("`{program}` could not be started: {error}")
    }
}
