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

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::engines::{parse_stream_line, EngineEvent, EventSink, Prompt, GENERIC_ENDING};
use crate::sdcp::envelope::ErrorObject;

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
    /// The flag that carries the chosen model, when the CLI has one (`--model`, `-m`).
    ///
    /// Until 0.7.0 the model the user picked in the prompt toolbar was **decoration** for the three
    /// CLI engines: it was named in the trigger, the turn meta and the status bar, and never sent. The
    /// dropdown offered `Opus` and the CLI ran whatever its own default was. Passing it is one flag
    /// per CLI - and `default` is skipped, because that is the id this build uses for "whatever the
    /// CLI decides", not a model name any of them knows.
    pub model_flag: Option<&'static str>,
    /// Extra environment the CLI needs to be quiet: `NO_COLOR` and friends.
    pub env: &'static [(&'static str, &'static str)],
    /// The flags that carry the person's autonomy to the CLI's own agent (0.9.0): `[ask, pro, auto]`.
    ///
    /// In `--print` mode none of the three CLIs can ask a question, so a tool that would need
    /// permission does not wait - it **fails**, silently from the transcript's point of view: the model
    /// says "I'll create the file", the write is refused, and the turn ends `Done` with nothing on
    /// disk. That was measured, not imagined (`greet.txt`, local and on a host, 0.8.1). The folder is
    /// checkpointed before every change and Rewind exists, so the careful level maps to "edits inside
    /// the folder are fine, anything wider is not", not to "fail everything without saying so".
    pub autonomy: [&'static [&'static str]; 3],
}

/// The autonomy flags of `spec` for `level` - its own function so the tests can hold the mapping.
pub fn autonomy_args(spec: &CliSpec, level: crate::agent::gate::Autonomy) -> &'static [&'static str] {
    match level {
        crate::agent::gate::Autonomy::Ask => spec.autonomy[0],
        crate::agent::gate::Autonomy::Pro => spec.autonomy[1],
        crate::agent::gate::Autonomy::Auto => spec.autonomy[2],
    }
}

/// The running children, so a cancel can kill one: the local pid, and - for a turn on a host - the
/// connection and the pid file the remote process wrote (0.7.13).
pub struct CliAdapter {
    spec: CliSpec,
    children: Arc<Mutex<HashMap<String, u32>>>,
    /// `turn id → (host, pid file)`, for the turns whose CLI runs on the far side of an `ssh`.
    ///
    /// A local cancel can only close the connection; this is what makes it a *kill*: the daemon knows
    /// which pid file to read on that host (`ssh::ops::kill_line`), so a Stop button stops the process
    /// rather than the pipe. See `turn_line` for why the pid in that file is the CLI's own.
    remotes: Arc<Mutex<HashMap<String, (crate::ssh::Ssh, String)>>>,
}

impl Default for CliAdapter {
    fn default() -> Self {
        Self {
            spec: crate::engines::claude_code::CLAUDE_SPEC,
            children: Arc::new(Mutex::new(HashMap::new())),
            remotes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl CliAdapter {
    pub fn new(spec: CliSpec) -> Self {
        Self {
            spec,
            children: Arc::new(Mutex::new(HashMap::new())),
            remotes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn spec(&self) -> &CliSpec {
        &self.spec
    }

    /// The command for a turn whose CLI runs **on a host**: an `ssh` whose remote command runs the CLI
    /// in the chat's folder there, with the pid tracked so a cancel can kill it (0.7.13).
    ///
    /// The `ssh` binary itself is resolved through `host::program`, like every other program this daemon
    /// starts - and `get_program`/`get_args` are how the batch-file wrap (`cmd.exe /c …` on Windows)
    /// survives the move from `std::process::Command` to tokio's.
    fn remote_command(&self, prompt: &Prompt, args: &[String]) -> Result<Command, ErrorObject> {
        let Some(ssh) = prompt.remote.as_ref() else {
            return Err(ErrorObject::internal("a remote turn without a host"));
        };

        let pid_file = crate::ssh::ops::pid_file(&prompt.turn_id);
        let line = crate::ssh::ops::turn_line(
            self.spec.program,
            args,
            prompt.project_root.as_deref(),
            self.spec.env,
            &pid_file,
        )?;
        let launcher = crate::ssh::program().map(|path| crate::host::program::command_for(&path)).ok_or_else(|| {
            ErrorObject::not_found("`ssh` is not on this machine's PATH, so no turn can run on a host")
        })?;
        let mut command = Command::new(launcher.get_program());

        command.args(launcher.get_args()).args(ssh.base_args()?).arg(line);

        if let Ok(mut remotes) = self.remotes.lock() {
            remotes.insert(prompt.turn_id.clone(), (ssh.clone(), pid_file));
        }

        Ok(command)
    }

    /// The program, resolved the way the shell would, **inside the chat's folder**.
    ///
    /// This is the half of `run` that decides where the engine works. `host::program` resolves the name
    /// the way the shell does - which is what makes an npm-installed CLI (`claude.cmd`, `codex.cmd`,
    /// `gemini.cmd` on Windows) startable at all - and the folder makes the difference between an engine
    /// that edits the project someone opened and one that edits the folder the daemon was started in.
    /// Until 0.7.6 a chat had no folder, so every turn ran in the second one.
    ///
    /// A root that is not a directory is **ignored** rather than fatal: the folder may have been moved or
    /// renamed since the chat was opened, and a turn that refuses to start is worse than one that runs
    /// where the daemon is. It is also why this is a `is_dir` check and not a migration.
    ///
    /// Extracted from `run` because `Command::get_current_dir` is the only way to assert the working
    /// directory without starting a process - see `the_engine_runs_in_the_chats_folder` below.
    fn command(&self, prompt: &Prompt) -> Command {
        let mut command = match crate::host::program::launch(self.spec.program) {
            Some((executable, prefix)) => {
                let mut command = Command::new(executable);

                command.args(prefix);

                command
            }
            None => Command::new(self.spec.program),
        };

        if let Some(root) = prompt
            .project_root
            .as_deref()
            .filter(|root| std::path::Path::new(root).is_dir())
        {
            command.current_dir(root);
        }

        command
    }

    /// Spawns the CLI, writes the prompt (plus the replay history of a bridged session), and reads the
    /// structured stream until it ends - **pushing each line's events as the line arrives**.
    ///
    /// Four things about this function are the result of a defect rather than of a design:
    ///
    /// * **the prompt's placement is the CLI's** (`PromptPlacement`), because Gemini's headless mode
    ///   takes it as an argument;
    /// * **stderr is captured**, not thrown away. `Stdio::null()` is how
    ///   `claude --include-partial-messages requires --print and --output-format=stream-json` stayed
    ///   invisible for a release: the child refused to run, printed that sentence to a pipe nobody
    ///   read, and the turn ended with an empty transcript;
    /// * **silence is reported as the CLI's own sentence**, never as "the stream ended";
    /// * **the stream is not collected first.** It used to be read into a `Vec<String>`, parsed once
    ///   the child had exited, and handed back as one batch - so a turn that streamed for two minutes
    ///   reached the window in one piece at the end of it (*"akbare answare disse"*). Every event now
    ///   goes to `sink` on the line it arrived on, and the daemon forwards it while the CLI is still
    ///   talking. The prompt is still the only thing written to stdin, and stdin is still closed right
    ///   after it - the CLIs read the prompt, not the stream, from there.
    pub async fn run(&self, prompt: &Prompt, sink: &EventSink) {
        /* The body is built first, because an argument-placed prompt has to go in with the flags. */
        let mut body = String::new();

        for message in &prompt.history {
            body.push_str(&message.text);
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
            .chain(model_args(self.spec.model_flag, &prompt.model))
            .chain(autonomy_args(&self.spec, prompt.autonomy).iter().map(|arg| (*arg).to_string()))
            .collect();

        /* `host::program` resolves the name the way the shell does - which is what makes an
           npm-installed CLI (`claude.cmd`, `codex.cmd`, `gemini.cmd` on Windows) startable at all. The
           command it builds already carries the chat's folder - see `command`. On a **host**, the whole
           turn goes over `ssh` instead: same args, same prompt placement, same stream - a different
           machine (0.7.13). */
        let mut command = match prompt.remote {
            Some(_) => match self.remote_command(prompt, &args) {
                Ok(command) => command,
                Err(error) => {
                    sink.send(EngineEvent::Failed(error.message));

                    return;
                }
            },
            None => {
                let mut command = self.command(prompt);

                command.args(&args);

                /* A connected Google API key reaches a local Gemini turn as `GEMINI_API_KEY` (0.11.0),
                   so Gemini works with no browser sign-in at all - see `gemini::auth_plan` for the
                   measured matrix. Every other CLI gets nothing extra. */
                for (name, value) in crate::engines::gemini::turn_env_for(self.spec.program) {
                    command.env(name, value);
                }

                command
            }
        };

        command
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
                sink.send(EngineEvent::Failed(missing_program_message(
                    self.spec.program,
                    &error,
                )));

                return;
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

        let (lines, ended, question) = match child.stdout.take() {
            Some(stdout) => read_structured(stdout, sink).await,
            None => (Vec::new(), false, None),
        };

        if question.is_some() {
            let _ = child.kill().await;
        }

        let _ = child.wait().await;

        if let Ok(mut children) = self.children.lock() {
            children.remove(&prompt.turn_id);
        }

        if let Ok(mut remotes) = self.remotes.lock() {
            remotes.remove(&prompt.turn_id);
        }

        let stderr = match stderr_task {
            Some(task) => task.await.unwrap_or_default(),
            None => String::new(),
        };

        /* A CLI that stopped on a question gets the one sentence that says what to do about it,
           instead of `explain_failure`'s generic one - the question itself is the explanation. */
        if let Some(asked) = question {
            if !ended {
                sink.send(EngineEvent::Failed(waiting_for_a_person(self.spec.program, &asked)));
            }

            return;
        }

        /* A stream that never reached a result ends with the CLI's own sentence, in the CLI's own
           words when it has any - the rule `explain_failure` stated for a collected stream, applied
           here to one that has already gone out. Nothing is added when the CLI did reach `result`
           or `error`: that line is its own structured answer and it has already been sent. */
        if !ended {
            let mut ending = vec![EngineEvent::Failed(GENERIC_ENDING.to_string())];

            explain_failure(&mut ending, self.spec.program, &stderr, plain_text_of(&lines));

            for event in ending {
                sink.send(event);
            }
        }
    }

    /// Kills the child of one turn. `engine.kill` and `engine.cancel` both land here; the difference
    /// between them is the event the daemon emits, not how the process dies.
    ///
    /// A turn on a **host** takes one more step (0.7.13): dropping the local `ssh` closes the connection,
    /// which is not the same thing as stopping the CLI on the far side, so the daemon also runs
    /// `ssh::ops::kill_line` there - on a **detached thread**, because this method is called from the
    /// request that answered `engine.cancel` and a Stop button must not wait for a connection to a host
    /// that may be slow. The remote process gets a `SIGTERM` a moment later; nothing here blocks.
    pub fn kill(&self, turn_id: &str) -> bool {
        let remote = self
            .remotes
            .lock()
            .ok()
            .and_then(|mut remotes| remotes.remove(turn_id));

        if let Some((ssh, pid_file)) = remote {
            std::thread::spawn(move || {
                let line = crate::ssh::ops::kill_line(&pid_file);

                let _ = ssh.run(&line, std::time::Duration::from_secs(15));
            });
        }

        let Ok(mut children) = self.children.lock() else {
            return false;
        };

        children.remove(turn_id).is_some()
    }

    /// True while a turn's child is alive - the input to the `Running`/`Stuck` decision.
    pub fn is_running(&self, turn_id: &str) -> bool {
        self.children
            .lock()
            .map(|children| children.contains_key(turn_id))
            .unwrap_or(false)
    }
}

/// One line of a CLI's stream, on its way to the window: parse it, push what it carried, and latch
/// when it ended the turn.
///
/// **This is the live path's rule**, and it is a function rather than three lines inside `run` so a
/// test can hold it against `collect_stream` over the VCR fixtures (`tests/vcr.rs`): the same line
/// must produce the same event whether it is pushed on arrival or parsed out of a finished batch, and
/// nothing after a `result`/`error` line belongs to the turn.
///
/// `ended` is the caller's latch, because `run` keeps reading to the end of stdout: a child that is
/// still writing must not block on a full pipe while the daemon has stopped listening.
pub fn push_stream_line(line: &str, ended: &mut bool, sink: &EventSink) {
    if *ended {
        return;
    }

    for event in parse_stream_line(line) {
        *ended = event.is_terminal();
        sink.send(event);

        if *ended {
            break;
        }
    }
}

/// The `--model` pair for a turn, or nothing when the CLI has no flag or the id is not a model name.
///
/// `default` is this build's word for "whatever the CLI decides" (it is the id the registry gives
/// Codex's subscription row), and no CLI accepts it as a value. An empty id is the app saying it has
/// no opinion. Both cases send nothing, which is exactly the old behaviour.
pub fn model_args(flag: Option<&'static str>, model: &str) -> Vec<String> {
    let model = model.trim();

    match flag {
        Some(flag) if !model.is_empty() && model != "default" => {
            vec![flag.to_string(), model.to_string()]
        }
        _ => Vec::new(),
    }
}

/// How long a half-written line may sit unchanged before it is treated as a question. Fifteen seconds
/// of *nothing new* behind a partial non-JSON line is a prompt, not thinking: a structured stream
/// writes whole lines, and a model's silence has no half-line in front of it.
const INTERACTIVE_STALL: std::time::Duration = std::time::Duration::from_secs(15);

/// Reads a CLI's structured stream **in bytes, not lines** - pushing each complete line's events as
/// it arrives, and watching the half-written tail for a question.
///
/// Bytes matter because a CLI that is talking to a person prints its question *without a newline*
/// and waits forever. Gemini's `Opening authentication page in your browser. Do you want to
/// continue? [Y/n]:` when nobody is signed in is exactly that, measured on 0.60.0: a line reader
/// never yields, the child never exits, and the turn hangs with nothing on screen - even with stdin
/// closed. Reading bytes lets the half-written line be *seen*, so the turn can end with a kill and a
/// sentence instead of with silence.
///
/// Answers `(the complete lines, whether a terminal event was pushed, the question it stopped on)`.
/// A known question (`interactive_prompt`) is recognised the moment it arrives; any other
/// half-written non-JSON line is given `INTERACTIVE_STALL` of silence first, because a chunk
/// boundary can split an honest JSON line anywhere.
async fn read_structured(
    mut stdout: impl tokio::io::AsyncRead + Unpin,
    sink: &EventSink,
) -> (Vec<String>, bool, Option<String>) {
    let mut lines = Vec::new();
    let mut ended = false;
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];

    loop {
        match tokio::time::timeout(INTERACTIVE_STALL, stdout.read(&mut chunk)).await {
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(read)) => {
                pending.extend_from_slice(&chunk[..read]);

                while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                    let raw: Vec<u8> = pending.drain(..=end).collect();
                    let line = String::from_utf8_lossy(&raw).trim_end().to_string();

                    push_stream_line(&line, &mut ended, sink);
                    lines.push(line);
                }

                let tail = String::from_utf8_lossy(&pending).trim().to_string();

                if let Some(asked) = interactive_prompt(&tail) {
                    return (lines, ended, Some(asked));
                }
            }
            Err(_) => {
                /* Silence alone is fine - thinking time is silent. Silence *behind* a half-written
                   line that is not JSON is a prompt waiting for a keyboard this turn does not have. */
                let tail = String::from_utf8_lossy(&pending).trim().to_string();

                if !tail.is_empty() && !tail.starts_with('{') {
                    return (lines, ended, Some(tail));
                }
            }
        }
    }

    (lines, ended, None)
}

/// The half-written line that means a CLI has stopped to talk to a person, when it has.
///
/// These phrases were measured, not imagined: signed out, `gemini -p … --output-format stream-json`
/// prints `Opening authentication page in your browser. Do you want to continue? [Y/n]:` - no
/// newline - and waits forever, even with stdin closed (Gemini CLI 0.60.0). The known phrases are
/// caught the moment they arrive; anything else half-written is left to the stall timer, because a
/// partial JSON line mid-flight must never be mistaken for a question.
pub fn interactive_prompt(tail: &str) -> Option<String> {
    if tail.is_empty() || tail.starts_with('{') {
        return None;
    }

    const QUESTIONS: &[&str] = &[
        "do you want to continue",
        "[y/n]",
        "opening authentication page",
        "please set an auth method",
        "waiting for auth",
        "press enter to",
    ];

    let lowered = tail.to_lowercase();

    QUESTIONS
        .iter()
        .any(|question| lowered.contains(question))
        .then(|| tail.to_string())
}

/// The sentence a turn ends with when its CLI stopped on a question. A sign-in question names the
/// fix (the app's own sign-in flow); any other question says where a keyboard is.
pub fn waiting_for_a_person(program: &str, question: &str) -> String {
    let lowered = question.to_lowercase();
    let sign_in = lowered.contains("auth") || lowered.contains("sign in") || lowered.contains("log in");

    if sign_in {
        format!(
            "`{program}` is not signed in - it stopped to ask \"{question}\", and a turn cannot answer that. Sign in first (Settings → Providers → {program} → Sign in), then send the prompt again."
        )
    } else {
        format!(
            "`{program}` stopped to ask \"{question}\", and a turn has no keyboard to answer it. Run `{program}` once in a terminal to answer it, then send the prompt again."
        )
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
        .find(|line| !line.is_empty() && serde_json::from_str::<serde_json::Value>(line).is_err())
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

    /// A prompt whose only interesting field is the folder, with the rest at "nothing was chosen" - the
    /// shape `run` builds for a chat that has no provider, no history and a default model.
    fn prompt_in(folder: Option<&str>) -> Prompt {
        Prompt {
            session_id: "s1".to_string(),
            turn_id: "t1".to_string(),
            text: "hello".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: folder.map(str::to_string),
            remote: None,
            autonomy: Default::default(),
        }
    }

    /// 0.9.0: the autonomy level travels to each CLI's own permission flags - measured first on
    /// `claude -p`, where a Write without them is silently refused (`greet.txt` stayed missing while
    /// the turn ended `Done`). The careful level accepts edits only: the folder is checkpointed.
    #[test]
    fn the_autonomy_level_reaches_the_clis_own_flags() {
        use crate::agent::gate::Autonomy;
        use crate::engines::claude_code::CLAUDE_SPEC;

        assert_eq!(
            autonomy_args(&CLAUDE_SPEC, Autonomy::Ask),
            &["--permission-mode", "acceptEdits"]
        );
        assert!(autonomy_args(&CLAUDE_SPEC, Autonomy::Pro).contains(&"--allowedTools"));
        assert_eq!(autonomy_args(&CLAUDE_SPEC, Autonomy::Auto), &["--dangerously-skip-permissions"]);
        assert_eq!(
            autonomy_args(&crate::engines::codex::CODEX_SPEC, Autonomy::Ask),
            &["--sandbox", "workspace-write"]
        );
        assert_eq!(
            autonomy_args(&crate::engines::gemini::GEMINI_SPEC, Autonomy::Auto),
            &["--yolo"]
        );
    }

    /// 0.7.13: a turn on a **host** runs the CLI over `ssh`, in the chat's folder *there*.
    ///
    /// The assertion worth having is the command that would be started, and it can be built without a
    /// connection: `remote_command` is where the `ssh` invocation is assembled, and the line it carries
    /// is what `ssh::ops::turn_line` wrote - `cd <folder> && sh -c 'echo $$ > <pid>; exec env … <cli> …'`.
    #[test]
    fn a_turn_on_a_host_is_an_ssh_to_the_chats_folder_with_the_pid_tracked() {
        let ssh = crate::ssh::Ssh::parse("ssh -p 8443 root@vps.example").unwrap();
        let adapter = CliAdapter::new(CLAUDE_SPEC);
        let mut remote = prompt_in(Some("/srv/app"));

        remote.turn_id = "turn-9".to_string();
        remote.remote = Some(ssh);

        let command = adapter.remote_command(&remote, &["--print".to_string()]).unwrap();
        let args: Vec<String> = command.as_std().get_args().map(|arg| arg.to_string_lossy().to_string()).collect();
        let line = args.last().cloned().unwrap_or_default();

        assert!(args.contains(&"-p".to_string()), "the port travels: {args:?}");
        assert!(args.contains(&"8443".to_string()), "{args:?}");
        assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"), "{args:?}");
        assert!(line.contains("cd '/srv/app'"), "{line}");
        assert!(line.contains("claude"), "{line}");
        assert!(line.contains("run/turn-9.pid"), "the pid is tracked so a cancel can kill it: {line}");
        assert!(line.contains("exec"), "{line}");
        assert!(
            !args.iter().any(|arg| arg == "--print"),
            "the CLI's own args travel inside the remote line, not as ssh arguments: {args:?}"
        );
        assert!(line.contains("'--print'"), "{line}");
    }

    /// 0.7.6: the child process is started **in the chat's folder**.
    ///
    /// This is the assertion the whole feature exists for. Before it, a chat had no working directory, so
    /// `Command::spawn` inherited the daemon's own - the folder someone happened to start `sdcd` from.
    /// An engine that is asked to "fix the login route" would then look for it in the daemon's folder
    /// rather than in the project.
    #[test]
    fn the_engine_runs_in_the_chats_folder() {
        let folder = std::env::temp_dir();
        let engine = CliAdapter::new(CLAUDE_SPEC);
        let command = engine.command(&prompt_in(Some(&folder.display().to_string())));

        assert_eq!(command.as_std().get_current_dir(), Some(folder.as_path()));
    }

    /// And a chat with no folder runs where the daemon is, with no `current_dir` set at all - which is the
    /// behaviour every chat had before 0.7.6, kept deliberately: the alternative is refusing to start.
    #[test]
    fn a_chat_without_a_folder_runs_where_the_daemon_is() {
        let engine = CliAdapter::new(CLAUDE_SPEC);

        assert!(engine.command(&prompt_in(None)).as_std().get_current_dir().is_none());
    }

    /// A folder that is gone (moved, deleted, renamed) is ignored rather than fatal: the turn starts in
    /// the daemon's directory and the transcript says what the engine did, which is better than an
    /// `engine.start` that fails because yesterday's path no longer exists.
    #[test]
    fn a_folder_that_is_gone_is_ignored() {
        let missing = std::env::temp_dir().join("sdc-no-such-folder-0-7-6");
        let engine = CliAdapter::new(CLAUDE_SPEC);
        let command = engine.command(&prompt_in(Some(&missing.display().to_string())));

        assert!(command.as_std().get_current_dir().is_none());
    }

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
    fn the_chosen_model_reaches_the_cli() {
        /* The bug this covers: the dropdown offered Opus and the CLI ran its own default, because the
           model was only ever printed. */
        assert_eq!(model_args(Some("--model"), "opus"), vec!["--model", "opus"]);
        assert_eq!(model_args(Some("-m"), "gpt-5"), vec!["-m", "gpt-5"]);

        /* `default` is this build's word for "the CLI decides", and no CLI knows it. */
        assert!(model_args(Some("-m"), "default").is_empty());
        assert!(model_args(Some("-m"), "  ").is_empty());
        assert!(model_args(None, "opus").is_empty());

        /* And each of the three CLIs has a flag, so none of them loses the choice. */
        assert_eq!(crate::engines::claude_code::CLAUDE_SPEC.model_flag, Some("--model"));
        assert_eq!(crate::engines::codex::CODEX_SPEC.model_flag, Some("-m"));
        assert_eq!(crate::engines::gemini::GEMINI_SPEC.model_flag, Some("-m"));
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

    /// The hang itself, end to end against the reader: a stream that carries one honest JSON line and
    /// then Gemini's sign-in question **without a newline**, the way a signed-out `gemini` writes it.
    /// The reader must hand the question back at once - not wait for a newline that will never come -
    /// while keeping the line that did arrive.
    #[tokio::test]
    async fn a_question_without_a_newline_ends_the_read_instead_of_hanging_it() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let recorder = crate::engines::Recorder::new();
        let sink = recorder.sink();
        let read = tokio::spawn(async move { read_structured(reader, &sink).await });

        writer
            .write_all(
                b"{\"type\":\"init\"}\nOpening authentication page in your browser. Do you want to continue? [Y/n]:",
            )
            .await
            .unwrap();

        /* The writer stays open - the real child never exits - so a reader that waits for EOF or for
           a newline hangs here, which is exactly the defect. The timeout is the assertion. */
        let (lines, ended, question) =
            tokio::time::timeout(std::time::Duration::from_secs(5), read).await.expect("the reader hung on a question").unwrap();

        assert_eq!(lines, vec![r#"{"type":"init"}"#.to_string()]);
        assert!(!ended);
        assert!(question.unwrap().contains("Do you want to continue?"));

        drop(writer);
    }

    /// The hang this release fixes: signed out, Gemini prints its auth question **without a newline**
    /// and never exits, so the old line reader waited forever and the turn showed nothing. The
    /// question is recognised in the partial buffer and the turn ends with a sentence that names the
    /// fix.
    #[test]
    fn a_signed_out_gemini_is_a_sentence_not_a_hang() {
        let tail = "Opening authentication page in your browser. Do you want to continue? [Y/n]:";

        assert_eq!(interactive_prompt(tail), Some(tail.to_string()));

        let message = waiting_for_a_person("gemini", tail);

        assert!(message.contains("not signed in"), "{message}");
        assert!(message.contains("Settings"), "{message}");
    }

    /// A partial JSON line mid-flight is the normal case while a chunk boundary splits a line; it is
    /// never a question, whatever words it happens to contain.
    #[test]
    fn a_half_written_json_line_is_not_a_question() {
        assert_eq!(interactive_prompt(r#"{"type":"delta","text":"do you want to continue"#), None);
        assert_eq!(interactive_prompt(""), None);
        assert_eq!(interactive_prompt("plain progress text with no question"), None);
    }

    /// A question that is not about signing in still ends the turn with words rather than a hang,
    /// and points at a terminal instead of at Settings.
    #[test]
    fn any_other_question_points_at_a_terminal() {
        let message = waiting_for_a_person("codex", "Overwrite existing config? [y/N]");

        assert!(message.contains("no keyboard"), "{message}");
        assert!(message.contains("terminal"), "{message}");
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
