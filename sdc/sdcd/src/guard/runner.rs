//! The command runner (P0-13): run a command, and always come back.
//!
//! # The two failures this exists to prevent
//!
//! 1. **A command that never ends.** `npm install` on a bad network, `tail -f`, a test runner waiting for stdin. If
//!    the agent waits for it, the turn waits for it, and the person sees a spinner and no way out. So every command
//!    has a deadline, and a command that passes it is killed - with its whole process group, because killing `sh`
//!    leaves `npm` running and the port still taken.
//! 2. **A command whose output never ends.** A build that prints a line a second for an hour is alive and correct and
//!    still unusable. Output is capped as it arrives, and the fact that it was capped is part of the answer.
//!
//! # What a caller gets
//!
//! An `Outcome` that always says how the command ended - `Exited`, `TimedOut`, `Signalled` - because "it did not
//! finish" and "it failed" are different things and a model told the wrong one will try the wrong fix. It also always
//! carries the output, even when the command was killed: the lines printed before the deadline are usually the ones
//! that say why.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::compress::{self, Compressed};

/// How long a command gets by default. Long enough for a test suite on a laptop, short enough that a person is not
/// left waiting on a command that will never answer.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// How much output is kept before it is compressed. A build's real output is a few hundred KiB; this is the point at
/// which the extra bytes stop being evidence and start being a bill.
pub const DEFAULT_OUTPUT_CAP: usize = 256 * 1024;

/// How the command ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Ending {
    /// The process finished on its own. `code` is `None` when the platform would not say (a signal on Unix).
    Exited { code: Option<i32> },
    /// The deadline passed and the runner killed it - the process group, not only the shell.
    TimedOut,
}

/// What a command said, and how it ended.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub ending: Ending,
    pub output: Compressed,
    pub seconds: f64,
    /// Whether the runner had to stop reading because the output cap was reached. Separate from `output.dropped`:
    /// that counts lines the *model* will not see, this counts bytes the *runner* stopped collecting.
    pub output_capped: bool,
    /// Whether the process group was killed rather than the process, which is what makes "killed" mean killed.
    pub group_killed: bool,
}

impl Outcome {
    pub fn succeeded(&self) -> bool {
        matches!(self.ending, Ending::Exited { code: Some(0) })
    }

    /// The one line a person or a model reads first.
    pub fn summary(&self) -> String {
        match self.ending {
            Ending::Exited { code: Some(0) } => format!("finished in {:.1}s", self.seconds),
            Ending::Exited { code: Some(code) } => format!("exited with {code} after {:.1}s", self.seconds),
            Ending::Exited { code: None } => format!("stopped by a signal after {:.1}s", self.seconds),
            Ending::TimedOut => format!("did not finish within the timeout and was stopped after {:.1}s", self.seconds),
        }
    }
}

/// How a command should be run.
#[derive(Clone, Debug)]
pub struct Settings {
    pub timeout: Duration,
    pub output_cap: usize,
    pub budget: usize,
    /// The folder to run in. `None` runs where the daemon is, which is never what an agent wants and is the caller's
    /// mistake to make.
    pub cwd: Option<std::path::PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { timeout: DEFAULT_TIMEOUT, output_cap: DEFAULT_OUTPUT_CAP, budget: 8 * 1024, cwd: None }
    }
}

/// Run a launch and always come back.
///
/// Blocking on purpose: the caller decides whether it is on a blocking thread. A `spawn` here would hand every caller
/// the same "and who cancels it?" question, and the answer would eventually be "nobody".
pub fn run(launch: &super::osnorm::Launch, settings: &Settings) -> Result<Outcome, String> {
    let started = Instant::now();

    let mut command = Command::new(&launch.program);

    command.args(&launch.args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    if let Some(cwd) = &settings.cwd {
        command.current_dir(cwd);
    }

    /* A new process group, so the deadline can kill the whole tree. Without this, killing the shell leaves the
       command it started holding the port, the lock and the output. */
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        command.process_group(0);
    }

    let mut child = command.spawn().map_err(|error| format!("{} could not be started: {error}", launch.display()))?;
    /* Windows (Phase 4.10): the child and its tree go into a Job Object - a process cap, a memory cap, and the whole tree dies when this
       function returns. Held until then; elsewhere this is nothing (`sandbox` has the mechanisms of macOS and Linux). */
    let _contained = super::winjob::contain_command(&child);
    let pid = child.id();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    /* The readers run on their own threads so that a command which fills the pipe cannot deadlock the wait: this is
       the classic way a "timed out" command hangs forever anyway. */
    let collect = |pipe: Option<Box<dyn Read + Send>>, cap: usize| {
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut capped = false;

            if let Some(mut pipe) = pipe {
                let mut buffer = [0u8; 8192];

                loop {
                    match pipe.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if kept.len() >= cap {
                                /* Keep reading so the child is not blocked on a full pipe, but stop collecting. */
                                capped = true;
                                continue;
                            }

                            let room = cap - kept.len();
                            kept.extend_from_slice(&buffer[..read.min(room)]);
                        }
                    }
                }
            }

            (kept, capped)
        })
    };

    let out_thread = collect(stdout.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>), settings.output_cap);
    let err_thread = collect(stderr.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>), settings.output_cap);

    /* The wait has a deadline. `try_wait` in a short loop rather than `wait_timeout`, because that is one fewer
       dependency and the cost is a thread sleeping, not a core spinning. */
    let deadline = started + settings.timeout;
    let mut timed_out = false;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(error) => return Err(format!("{} could not be waited for: {error}", launch.display())),
        }

        if Instant::now() >= deadline {
            timed_out = true;
            break None;
        }

        std::thread::sleep(Duration::from_millis(20));
    };

    let mut group_killed = false;

    if timed_out {
        /* The group is what has to die, not only the shell: killing `sh -c "npm run dev"` leaves `npm` holding the
           port. `pty::kill_tree` is the helper that already does this per platform (a process group on Unix,
           `taskkill /T /F` on Windows), and the claim in `group_killed` is made from the child being confirmed dead
           rather than from the call having been made. */
        super::super::pty::kill_tree(pid);

        let _ = child.kill();
        let _ = child.wait();

        group_killed = child.try_wait().map(|status| status.is_some()).unwrap_or(false);
    }

    let (out, out_capped) = out_thread.join().unwrap_or((Vec::new(), false));
    let (err, err_capped) = err_thread.join().unwrap_or((Vec::new(), false));

    let stdout = String::from_utf8_lossy(&out).to_string();
    let stderr = String::from_utf8_lossy(&err).to_string();
    let combined = match (stdout.is_empty(), stderr.is_empty()) {
        (true, _) => stderr,
        (_, true) => stdout,
        /* Both, in the order they matter: what the command said it did, then what it said went wrong. */
        _ => format!("{stdout}\n{stderr}"),
    };

    let output = compress::compress(&combined, settings.budget);
    let ending = match status {
        Some(status) => Ending::Exited { code: status.code() },
        None => Ending::TimedOut,
    };

    Ok(Outcome { ending, output, seconds: started.elapsed().as_secs_f64(), output_capped: out_capped || err_capped, group_killed })
}

#[cfg(test)]
mod tests {
    use super::super::osnorm::{Command as NormCommand, Os};
    use super::*;

    fn settings(timeout_secs: u64) -> Settings {
        Settings { timeout: Duration::from_secs(timeout_secs), ..Settings::default() }
    }

    fn launch(line: &str) -> super::super::osnorm::Launch {
        super::super::osnorm::normalise(&NormCommand::line(line), Os::here())
    }

    #[test]
    fn a_command_that_works_reports_its_output_and_its_exit_code() {
        let outcome = run(&launch("echo hello"), &settings(10)).unwrap();

        assert_eq!(outcome.ending, Ending::Exited { code: Some(0) });
        assert!(outcome.succeeded());
        assert!(outcome.output.text.contains("hello"), "{:?}", outcome.output.text);
        assert!(outcome.summary().starts_with("finished in"));
    }

    #[test]
    fn a_failing_command_says_so_and_keeps_what_it_printed() {
        let outcome = run(&launch("echo before && exit 3"), &settings(10)).unwrap();

        assert_eq!(outcome.ending, Ending::Exited { code: Some(3) });
        assert!(!outcome.succeeded());
        assert!(outcome.output.text.contains("before"), "the output before the failure is evidence");
        assert!(outcome.summary().contains("exited with 3"));
    }

    #[test]
    fn a_command_that_never_finishes_is_killed_at_the_deadline_and_says_so() {
        let started = Instant::now();
        let outcome = run(&launch("sleep 30"), &settings(1)).unwrap();

        assert_eq!(outcome.ending, Ending::TimedOut);
        assert!(!outcome.succeeded());
        assert!(started.elapsed() < Duration::from_secs(10), "the runner came back, which is the whole point: {:?}", started.elapsed());
        assert!(outcome.summary().contains("did not finish"), "{}", outcome.summary());
        assert!(outcome.group_killed, "the process group was killed, not only the shell");
    }

    #[test]
    fn a_child_of_the_shell_is_killed_with_it() {
        /* The case that makes a naive timeout useless: the shell is killed, its child keeps the pipe open and the
           port taken, and the runner appears to hang anyway. */
        let started = Instant::now();
        let outcome = run(&launch("sleep 300 & sleep 300"), &settings(1)).unwrap();

        assert_eq!(outcome.ending, Ending::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10), "both children were reached: {:?}", started.elapsed());
    }

    /// A command that prints `line0 … line19999`: far more than any cap holds. Written in the shell this system has -
    /// `cmd` has no `while`, and `sh` has no `for /L` - so the Windows run tests the same thing the other two do.
    #[cfg(unix)]
    const A_WALL_OF_LINES: &str = "i=0; while [ $i -lt 20000 ]; do echo line$i; i=$((i+1)); done";
    #[cfg(windows)]
    const A_WALL_OF_LINES: &str = "for /L %i in (1,1,20000) do @echo line%i";

    /// The same, ending with the one line that must survive the compression.
    #[cfg(unix)]
    const A_WALL_AND_THEN_AN_ERROR: &str =
        "i=0; while [ $i -lt 3000 ]; do echo noise$i; i=$((i+1)); done; echo 'error: the build broke'";
    #[cfg(windows)]
    const A_WALL_AND_THEN_AN_ERROR: &str = "(for /L %i in (1,1,3000) do @echo noise%i) & echo error: the build broke";

    #[test]
    fn the_output_cap_stops_collecting_and_says_it_did() {
        let mut limits = settings(30);

        limits.output_cap = 1_024;

        /* A command that prints far more than the cap. It has to finish, not be killed: the cap is about bytes, not
           about time. */
        let outcome = run(&launch(A_WALL_OF_LINES), &limits).unwrap();

        assert_eq!(outcome.ending, Ending::Exited { code: Some(0) });
        assert!(outcome.output_capped, "the runner stopped collecting and says so");
        assert!(outcome.output.text.len() <= 2_048, "and what it hands over is inside its own budget: {}", outcome.output.text.len());
    }

    #[test]
    fn an_error_line_survives_the_compression_of_a_wall_of_output() {
        let mut limits = settings(60);

        limits.budget = 2_000;

        let outcome = run(&launch(A_WALL_AND_THEN_AN_ERROR), &limits).unwrap();

        assert!(outcome.output.was_truncated());
        assert!(outcome.output.text.contains("error: the build broke"), "the line that matters is never dropped");
    }

    #[test]
    fn a_command_that_does_not_exist_is_an_error_and_not_a_panic() {
        let launch = super::super::osnorm::Launch { program: "definitely-not-a-real-command-sdc".into(), args: vec![], via_shell: false };

        let error = run(&launch, &settings(5)).unwrap_err();

        assert!(error.contains("could not be started"), "{error}");
    }

    #[test]
    fn the_folder_the_command_runs_in_is_the_one_asked_for() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(dir.path().join("marker.txt"), "here").unwrap();

        let mut limits = settings(10);

        limits.cwd = Some(dir.path().to_path_buf());

        let outcome = run(&launch("cat marker.txt"), &limits).unwrap();

        assert!(outcome.output.text.contains("here"), "{:?}", outcome.output.text);
    }

    #[test]
    fn the_default_timeout_is_long_enough_for_a_suite_and_short_enough_to_wait_for() {
        assert!(DEFAULT_TIMEOUT >= Duration::from_secs(60));
        assert!(DEFAULT_TIMEOUT <= Duration::from_secs(300));
    }
}
