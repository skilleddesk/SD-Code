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

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::engines::{collect_stream, EngineEvent, Prompt};

/// Where a CLI wants the prompt.
///
/// Two of the three read it from stdin and treat the closed pipe as "that is the whole prompt".
/// Gemini's headless mode takes it as the value of `-p` and treats stdin as the answer to its own
/// questions ("Appended to input on stdin (if any)"), so a piped prompt lands in its auth prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptPlacement {
    /// Written to the child's stdin, which is then closed.
    Stdin,
    /// Replaces `{prompt}` in `args`.
    Argument,
}

/// How one CLI is invoked. The differences between the three adapters, as data.
#[derive(Debug, Clone)]
pub struct CliSpec {
    /// The program on `PATH`.
    pub program: &'static str,
    /// Flags that ask for the structured stream, with `{prompt}` where the prompt goes.
    ///
    /// These were written from the CLIs' own `--help` and then **measured** (`_verify/cli-shapes.mjs`
    /// runs each candidate flag set and prints the exit status and the lines): two of the three were
    /// wrong in a way that produced an empty stream and no error at all, because stderr was thrown
    /// away. The comments on each spec say what the CLI answered when it was asked.
    pub args: &'static [&'static str],
    /// Where the prompt goes.
    pub prompt: PromptPlacement,
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
    ///
    /// Three things about this function are the result of a defect rather than of a design:
    ///
    /// * **the prompt's placement is the CLI's** (`PromptPlacement`), because Gemini's headless mode
    ///   takes it as an argument;
    /// * **stderr is captured**, not thrown away. `Stdio::null()` is how
    ///   `claude --include-partial-messages requires --print and --output-format=stream-json` stayed
    ///   invisible for a release: the child refused to run, printed that sentence to a pipe nobody
    ///   read, and the turn ended with an empty transcript;
    /// * **silence is reported as the CLI's own sentence**, never as "the stream ended".
    pub async fn run(&self, prompt: &Prompt) -> Vec<EngineEvent> {
        /* The body is built first, because an argument-placed prompt has to go in with the flags. */
        let mut body = String::new();

        for message in &prompt.history {
            body.push_str(message);
            body.push('\n');
        }

        body.push_str(&prompt.text);

        let args: Vec<String> = self
            .spec
            .args
            .iter()
            .map(|arg| {
                if *arg == "{prompt}" && self.spec.prompt == PromptPlacement::Argument {
                    body.clone()
                } else {
                    (*arg).to_string()
                }
            })
            .collect();

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
            .args(&args)
            .envs(self.spec.env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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

        /* Both pipes are drained at once: a child that fills its stderr buffer while nobody reads it
           blocks forever, and a CLI that failed at startup is exactly the case that does. */
        let stderr_task = child.stderr.take().map(|mut stderr| {
            tokio::spawn(async move {
                let mut text = String::new();

                let _ = stderr.read_to_string(&mut text).await;

                text
            })
        });

        if let Some(mut stdin) = child.stdin.take() {
            if self.spec.prompt == PromptPlacement::Stdin {
                let _ = stdin.write_all(body.as_bytes()).await;
                let _ = stdin.flush().await;
            }

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

        let stderr = match stderr_task {
            Some(task) => task.await.unwrap_or_default(),
            None => String::new(),
        };

        let mut events = collect_stream(lines.clone());

        explain_failure(&mut events, self.spec.program, &stderr, plain_text_of(&lines));

        events
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

/// The last line of stdout that is not JSON - what a CLI prints when it is talking to a person rather
/// than to a program. Gemini's `Opening authentication page in your browser. Do you want to continue?`
/// is exactly this: it arrives on stdout, before any JSON, and it is the whole explanation.
fn plain_text_of(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .rev()
        .map(|line| line.trim())
        .find(|line| {
            !line.is_empty()
                && serde_json::from_str::<serde_json::Value>(line).is_err()
        })
        .map(str::to_string)
}

/// Replaces silence with the CLI's own explanation.
///
/// This is the defect that made "the chat does nothing" possible for a release: the adapter threw the
/// child's stderr away (`Stdio::null()`), so a CLI that refused to start produced an empty stream and
/// the turn ended with `the engine's stream ended without a result` - a sentence that says nothing
/// about *why*, next to an empty transcript. `claude --include-partial-messages` printed
/// `Error: --include-partial-messages requires --print and --output-format=stream-json.` into that
/// pipe, and nobody ever saw it.
///
/// The rule now: a turn that did not reach `Done` ends with the first line the child wrote. A CLI that
/// already explained itself *in JSON* keeps its own wording, because that one is structured.
pub fn explain_failure(
    events: &mut Vec<EngineEvent>,
    program: &str,
    stderr: &str,
    stdout_sentence: Option<String>,
) {
    if matches!(events.last(), Some(EngineEvent::Done { .. })) {
        return;
    }

    let said = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .or(stdout_sentence);

    let Some(said) = said else {
        return;
    };

    let sentence = format!("`{program}` said: {said}");

    match events.last_mut() {
        /* The generic ending is the one this function exists to replace. */
        Some(EngineEvent::Failed(generic)) if generic.starts_with("the engine's stream ended") => {
            *generic = sentence;
        }
        /* A JSON `error`/`turn.failed` line is the CLI's own structured answer: leave it alone. */
        Some(EngineEvent::Failed(_)) => {}
        _ => events.push(EngineEvent::Failed(sentence)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::{claude_code::CLAUDE_SPEC, codex::CODEX_SPEC, gemini::GEMINI_SPEC};

    /// The shapes that were measured against the installed CLIs (`_verify/cli-shapes.mjs`).
    #[test]
    fn each_spec_says_where_the_prompt_goes_and_what_streams() {
        assert_eq!(CLAUDE_SPEC.prompt, PromptPlacement::Stdin);
        assert_eq!(CODEX_SPEC.prompt, PromptPlacement::Stdin);
        assert_eq!(GEMINI_SPEC.prompt, PromptPlacement::Argument);

        /* Claude: `-p` and stream-json are both mandatory next to the partial-messages flag. Without
           them the CLI exits 1 with its reason on stderr - which used to be thrown away. */
        assert!(CLAUDE_SPEC.args.contains(&"-p"));
        assert!(CLAUDE_SPEC.args.contains(&"stream-json"));
        assert!(CLAUDE_SPEC.args.contains(&"--include-partial-messages"));

        /* Codex: `--json` is not a flag of the CLI at all; `exec` is the non-interactive subcommand
           and `-` is how it reads the prompt from stdin. */
        assert_eq!(CODEX_SPEC.args.first(), Some(&"exec"));
        assert!(CODEX_SPEC.args.contains(&"-"));
        assert!(!CODEX_SPEC.args.contains(&"--quiet"));

        /* Gemini: the prompt is an argument, and its trusted-folder question is answered. */
        assert!(GEMINI_SPEC.args.contains(&"{prompt}"));
        assert!(GEMINI_SPEC.args.contains(&"--skip-trust"));
        assert!(!GEMINI_SPEC.args.contains(&"json"));
    }

    #[test]
    fn a_turn_that_never_finished_ends_with_the_clis_own_sentence() {
        let mut events = vec![EngineEvent::Failed("the engine's stream ended without a result".into())];

        explain_failure(
            &mut events,
            "claude",
            "Error: --include-partial-messages requires --print and --output-format=stream-json.\n",
            None,
        );

        assert_eq!(
            events,
            vec![EngineEvent::Failed(
                "`claude` said: Error: --include-partial-messages requires --print and --output-format=stream-json.".into()
            )]
        );
    }

    #[test]
    fn an_empty_stream_gets_a_failure_rather_than_nothing() {
        let mut events = Vec::new();

        explain_failure(
            &mut events,
            "gemini",
            "",
            Some("Opening authentication page in your browser. Do you want to continue? [Y/n]".into()),
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], EngineEvent::Failed(_)));
    }

    #[test]
    fn a_finished_turn_and_a_json_error_are_left_alone() {
        let mut done = vec![EngineEvent::Done { summary: "OK".into(), meta: String::new(), pass: Some(true) }];

        explain_failure(&mut done, "codex", "a warning nobody needs", None);
        assert_eq!(done.len(), 1);

        let mut failed = vec![EngineEvent::Failed("rate limit exceeded".into())];

        explain_failure(&mut failed, "codex", "something else entirely", None);
        assert_eq!(failed, vec![EngineEvent::Failed("rate limit exceeded".into())]);
    }

    #[test]
    fn a_persons_sentence_is_the_last_non_json_line_of_stdout() {
        let lines = vec![
            r#"{"type":"thread.started"}"#.to_string(),
            "Opening authentication page in your browser.".to_string(),
            String::new(),
        ];

        assert_eq!(
            plain_text_of(&lines),
            Some("Opening authentication page in your browser.".to_string())
        );
        assert_eq!(plain_text_of(&[r#"{"a":1}"#.to_string()]), None);
    }
}

