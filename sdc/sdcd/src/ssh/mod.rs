//! The SSH layer (0.7.13): how this daemon reaches another machine, and what it refuses to do on the way.
//!
//! The report this module answers: *"vps connect korai jasse nah. VPS connection ar jonno je sokol
//! layer proyojon sai rokom kono kisui aikhane nai."* 0.7.0 gave a host a parser, a key and a probe -
//! and left three quarters of the feature missing (see `docs/REMOTE.md`): the port was dropped on the
//! way to the database, the host key was trusted blindly through `accept-new`, and nothing could be
//! looked at *inside* the machine that was reached.
//!
//! Three modules, in the order a connection goes through them:
//!
//! | module | what it is |
//! | ------ | ---------- |
//! | this one | the connection: one hardened argument set, `run`, `run_with_stdin`, and `sh_quote` |
//! | `hostkey` | the trust decision: scan, fingerprint, pin, and refuse a changed key |
//! | `ops` | what is done on the far side: list a folder, read a file, write one, git, one shell command |
//!
//! ## The argument set is the security layer
//!
//! Every `ssh` this daemon runs goes through [`Ssh::base_args`], because a connection with the right
//! flags in one place and the wrong ones in another is how a tool ends up silently trusting a machine
//! it has never seen. The flags, and the reason each one is here:
//!
//! * `BatchMode=yes` - nothing may prompt; there is no terminal behind these calls.
//! * `-i <~/.ssh/sdc_ed25519>` + `IdentitiesOnly=yes` + `PreferredAuthentications=publickey` +
//!   `PasswordAuthentication=no` - SDC offers exactly one key, the one minted for this purpose. A VPS
//!   is somebody else's machine and has no business seeing the agent's keys.
//! * `StrictHostKeyChecking=yes` + `UserKnownHostsFile=<data>/ssh/known_hosts` - the host is checked
//!   against **SDC's own** pins, taken by a person who was shown a fingerprint. `accept-new` is not
//!   used anywhere: it is the flag that makes a man-in-the-middle's first connection succeed.
//! * `ConnectTimeout=10`, `ServerAliveInterval=15`, `ServerAliveCountMax=3` - a black-holed address
//!   costs seconds, not a stuck turn.
//! * `LogLevel=ERROR` - `ssh`'s warnings about a person's own `~/.ssh/config` are not this daemon's
//!   output.

use std::io::{Read, Write};
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::auth::remote::{key_path, parse_target, SshTarget};
use crate::sdcp::envelope::ErrorObject;

pub mod hostkey;
pub mod ops;
pub mod session;

/// The `ssh` every call runs: the one that can hold a sign-in open ([`session::program`]) when this
/// machine has it, otherwise the one on PATH.
pub fn program() -> Option<std::path::PathBuf> {
    session::program().or_else(|| crate::host::program::resolve("ssh"))
}

/// How much of one remote stream is kept. A remote `find` on `/` must not become 256 MB in memory:
/// the rest is drained (so the child never blocks on a full pipe) and reported as truncated.
const CAPTURE_LIMIT: usize = 256 * 1024;

/// How long a connection may take before it is killed. Every caller passes its own; this is the
/// default for the small reads.
pub const QUICK: Duration = Duration::from_secs(20);

/// One remote command's answer.
#[derive(Debug, Clone)]
pub struct SshOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub truncated: bool,
}

impl SshOutput {
    /// Did the command run to completion and exit 0?
    pub fn ok(&self) -> bool {
        !self.timed_out && self.code == Some(0)
    }

    /// The sentence for a command that did not succeed - `ssh`'s own words where there are any.
    ///
    /// A timeout says what it means, including the honest part: the local `ssh` was killed, and a
    /// command it had already sent to the host may still be running there.
    pub fn reason(&self) -> String {
        if self.timed_out {
            return "the connection timed out; a command already sent to the host may still be running there".to_string();
        }

        for stream in [&self.stderr, &self.stdout] {
            if let Some(line) = stream.lines().map(str::trim).find(|line| !line.is_empty()) {
                return line.to_string();
            }
        }

        match self.code {
            Some(code) => format!("the remote command exited {code}"),
            None => "the connection closed without an answer".to_string(),
        }
    }
}


/// The sentence a person needs when a host did not answer and SDC dialed the **default** port.
///
/// A hardened VPS very often does not answer on 22: the address from the original bug report
/// (`203.0.113.10`) refuses 22 outright and runs its sshd on **8443**. `did not answer: Connection
/// timed out` then reads as "the machine is down" when the truth is "SDC knocked on the wrong door", and a
/// person who logs in every day from their own terminal knows their port - they just did not write it in
/// the box. Empty when the target names a port, because then the answer is about the port they chose.
pub fn port_hint(target: &SshTarget) -> String {
    if target.port.is_some() {
        return String::new();
    }

    format!(
        " SDC dialed port 22. If that machine's sshd listens elsewhere, write the port down - `{} -p 8443` - or paste the whole command you use in your own terminal (`ssh -p 8443 {}`).",
        target.user_host, target.user_host
    )
}

/// One SSH destination, with the argument set every call shares.
#[derive(Debug, Clone)]
pub struct Ssh {
    pub target: SshTarget,
}

impl Ssh {
    pub fn new(target: SshTarget) -> Self {
        Self { target }
    }

    /// What a person typed: `user@host`, or the whole `ssh -p 8443 user@host` command.
    pub fn parse(raw: &str) -> Result<Self, ErrorObject> {
        parse_target(raw).map(Self::new).map_err(ErrorObject::bad_request)
    }

    /// `user@host`, with the port when the target named one - what a sentence calls this host.
    pub fn label(&self) -> String {
        match self.target.port {
            Some(port) => format!("{}:{port}", self.target.user_host),
            None => self.target.user_host.clone(),
        }
    }

    /// The arguments every `ssh` call for this host shares, ending with the destination.
    ///
    /// `pub` because the callers that are not [`Ssh::run`] - the PTY-driven key install and the tests -
    /// have to prove they use the same set.
    pub fn base_args(&self) -> Result<Vec<String>, ErrorObject> {
        self.args_with(false)
    }

    /// The same set, for the **one** call that is allowed to answer a prompt: the one-time key install
    /// (`auth::remote::install_key`), which types the password into `ssh`'s own prompt through the
    /// daemon's PTY.
    ///
    /// Three flags differ, and the reason they may is that this call happens exactly once, in front of
    /// a person, against a host whose key SDC has already pinned - so `StrictHostKeyChecking=yes` and
    /// our own `known_hosts` still apply, and a host presenting a different key gets no password.
    pub fn install_args(&self) -> Result<Vec<String>, ErrorObject> {
        self.args_with(true)
    }

    fn args_with(&self, interactive: bool) -> Result<Vec<String>, ErrorObject> {
        let mut args = self.target.port_args();
        let pins = hostkey::known_hosts_path()?;

        let mut options = vec![
            "ConnectTimeout=10".to_string(),
            "IdentitiesOnly=yes".to_string(),
            "StrictHostKeyChecking=yes".to_string(),
            "LogLevel=ERROR".to_string(),
        ];

        if interactive {
            options.push("BatchMode=no".to_string());
            options.push("NumberOfPasswordPrompts=1".to_string());
            options.push("PreferredAuthentications=publickey,keyboard-interactive,password".to_string());
        } else {
            options.push("BatchMode=yes".to_string());
            options.push("ServerAliveInterval=15".to_string());
            options.push("ServerAliveCountMax=3".to_string());
            options.push("PreferredAuthentications=publickey".to_string());
            options.push("PasswordAuthentication=no".to_string());
        }

        for option in options {
            args.push("-o".to_string());
            args.push(option);
        }

        args.push("-o".to_string());
        args.push(format!("UserKnownHostsFile={}", session::ssh_path(&pins)));

        /* Through the signed-in connection when there is one (0.8.1). Not for the key install: that call
           runs on the PTY with the platform's own `ssh`, which has no multiplexing. */
        if !interactive {
            args.extend(session::mux_options(self)?);
        }

        /* The key SDC owns, when it exists. `ensure_key` is what makes one, and it is called on the
           path that adds a host - never here, because reading a remote folder must not create a key
           as a side effect. */
        if let Some(key) = key_path().filter(|path| path.exists()) {
            args.push("-i".to_string());
            args.push(session::ssh_path(&key));
        }

        args.push(self.target.user_host.clone());

        Ok(args)
    }

    /// Runs one command on the far side. `script` is handed to the remote's shell, so every value
    /// inside it must have been through [`sh_quote`].
    pub fn run(&self, script: &str, timeout: Duration) -> Result<SshOutput, ErrorObject> {
        self.execute(script, None, timeout)
    }

    /// The same, with `input` on the remote command's stdin.
    ///
    /// This is how a file is written on the far side: the text is *data* on a pipe, not a word in a
    /// shell command, so nothing inside it can be interpreted - no escaping, no size limit from the
    /// command line, no quoting bug. `cat > <path>` on the other end is the whole implementation.
    pub fn run_with_stdin(&self, script: &str, input: &str, timeout: Duration) -> Result<SshOutput, ErrorObject> {
        self.execute(script, Some(input), timeout)
    }

    fn execute(&self, script: &str, input: Option<&str>, timeout: Duration) -> Result<SshOutput, ErrorObject> {
        let mut command = program().map(|path| crate::host::program::command_for(&path)).ok_or_else(|| {
            ErrorObject::not_found(
                "`ssh` is not on this machine's PATH, so SDC cannot reach another host. Install the OpenSSH client (Windows: Settings → Optional features → OpenSSH Client).",
            )
        })?;

        command.args(self.base_args()?).arg(script);

        command
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let started = Instant::now();
        let mut child = command
            .spawn()
            .map_err(|error| ErrorObject::internal(format!("`ssh` could not be started: {error}")))?;

        if let Some(text) = input {
            if let Some(mut pipe) = child.stdin.take() {
                let owned = text.to_string();

                /* On its own thread: a remote command that reads nothing would otherwise block this
                   call while the pipe fills, and the timeout would be the only way out. */
                std::thread::spawn(move || {
                    let _ = pipe.write_all(owned.as_bytes());
                    let _ = pipe.flush();
                });
            }
        }

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (out_tx, out_rx) = mpsc::channel();
        let (err_tx, err_rx) = mpsc::channel();

        std::thread::spawn(move || {
            let _ = out_tx.send(capture(stdout));
        });
        std::thread::spawn(move || {
            let _ = err_tx.send(capture(stderr));
        });

        let (code, timed_out) = wait(&mut child, started, timeout);
        let budget = Duration::from_millis(750);
        let (stdout, out_truncated) = out_rx.recv_timeout(budget).unwrap_or_else(|_| (String::new(), true));
        let (stderr, err_truncated) = err_rx.recv_timeout(budget).unwrap_or_else(|_| (String::new(), true));

        Ok(SshOutput {
            stdout,
            stderr,
            code,
            timed_out,
            truncated: out_truncated || err_truncated,
        })
    }
}

/// Waits for a child, killing it at the deadline. Returns `(exit code, timed out)`.
fn wait(child: &mut Child, started: Instant, timeout: Duration) -> (Option<i32>, bool) {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), false),
            Ok(None) => {}
            /* A wait that cannot be made is a process this call can no longer reason about: report it
               as a timeout rather than pretending it finished. */
            Err(_) => return (None, true),
        }

        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();

            return (None, true);
        }

        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Reads a stream to the capture limit, lossily - the same rule the PTY runner uses.
fn capture(stream: Option<impl Read>) -> (String, bool) {
    let Some(mut stream) = stream else {
        return (String::new(), false);
    };

    let mut kept = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;

    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if kept.len() < CAPTURE_LIMIT {
                    let room = CAPTURE_LIMIT - kept.len();
                    kept.extend_from_slice(&buffer[..read.min(room)]);
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }

    (String::from_utf8_lossy(&kept).to_string(), truncated)
}

/// One word, quoted for a POSIX shell: `it's` becomes `'it'\''s'`.
///
/// This is the function that stands between a person's folder name and a remote shell's parser. Every
/// path, branch name and argument that reaches a remote command goes through it, and the rule is the
/// standard one (`'` is POSIX; `$'…'` is bash): wrap in single quotes, close, escape the quote, open
/// again. A file called `$(rm -rf /)` is a file called `$(rm -rf /)` on the other end.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_word_is_quoted_so_nothing_inside_it_is_interpreted() {
        assert_eq!(sh_quote("/srv/app"), "'/srv/app'");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote("$(rm -rf /)"), "'$(rm -rf /)'");
        assert_eq!(sh_quote("`whoami`"), "'`whoami`'"); 
        assert_eq!(sh_quote("a;b"), "'a;b'");

        /* Nothing a shell treats as special is left outside the quotes. */
        for hostile in ["$HOME", "a|b", "a>b", "a\nb", "\"x\"", "~"] {
            let quoted = sh_quote(hostile);

            assert!(quoted.starts_with('\'') && quoted.ends_with('\''), "{quoted} is not quoted");
        }
    }

    /// The flags, spelled out: this is the security layer, and a missing `IdentitiesOnly` or a
    /// reintroduced `accept-new` is exactly the kind of change no behaviour test would catch.
    #[test]
    fn every_connection_carries_the_hardened_flag_set() {
        let ssh = Ssh::parse("ssh -p 8443 mehedi@203.0.113.10").unwrap();
        let args = ssh.base_args().unwrap().join(" ");

        assert!(args.contains("-p 8443"), "{args}");
        assert!(args.contains("BatchMode=yes"), "{args}");
        assert!(args.contains("IdentitiesOnly=yes"), "{args}");
        assert!(args.contains("PreferredAuthentications=publickey"), "{args}");
        assert!(args.contains("PasswordAuthentication=no"), "{args}");
        assert!(args.contains("StrictHostKeyChecking=yes"), "{args}");
        assert!(args.contains("UserKnownHostsFile="), "{args}");
        assert!(args.contains("known_hosts"), "{args}");
        assert!(args.contains("ConnectTimeout=10"), "{args}");
        assert!(args.ends_with("mehedi@203.0.113.10"), "{args}");

        /* And the three that must never come back. */
        assert!(!args.contains("accept-new"), "{args}");
        assert!(!args.contains("StrictHostKeyChecking=no"), "{args}");
        assert!(!args.contains(" -A "), "{args}");
    }

    /// A host with no port in it gets told **which door SDC knocked on** when it does not answer.
    ///
    /// The bug report's own address refuses 22 and runs its sshd on 8443, so `did not answer: Connection
    /// timed out` read as "the machine is down" while the machine was fine. This hint is what turns that
    /// into something a person can act on - and it is *absent* when the target named a port, because then
    /// the answer is about the port they chose.
    #[test]
    fn a_target_without_a_port_says_which_port_was_dialed() {
        let no_port = Ssh::parse("mehedi@vps.example").unwrap();
        let with_port = Ssh::parse("ssh -p 8443 mehedi@vps.example").unwrap();

        let hint = port_hint(&no_port.target);

        assert!(hint.contains("port 22"), "{hint}");
        assert!(hint.contains("-p 8443"), "{hint}");
        assert!(hint.contains("mehedi@vps.example"), "{hint}");
        assert_eq!(port_hint(&with_port.target), "", "a named port needs no hint");
    }

    /// And the probe's failure sentence carries it - the sentence a person reads on the host's row.
    ///
    /// A loopback address with nothing on port 22, or with an sshd that will not take SDC's key: either way
    /// the answer is a sentence, and when it is a failure it says which port was dialed.
    #[test]
    fn a_probe_that_fails_names_the_port_it_dialed() {
        let ssh = Ssh::parse("nobody@127.0.0.1").unwrap();
        let (status, detail) = crate::ssh::ops::probe(&ssh);

        if status == "connected" {
            assert!(detail.contains("reachable"), "{detail}");
        } else {
            assert!(detail.contains("port 22"), "the hint reaches the row: {detail}");
        }
    }

    #[test]
    fn a_target_is_parsed_into_a_destination_the_same_way_a_person_wrote_it() {
        assert_eq!(Ssh::parse("root@vps.example").unwrap().label(), "root@vps.example");
        assert_eq!(Ssh::parse("root@vps.example -p 2222").unwrap().label(), "root@vps.example:2222");
        assert!(Ssh::parse("vps.example").is_err(), "a host without a user is refused, not guessed");
    }

    /// A failure sentence is `ssh`'s own line where there is one, and says what a timeout means.
    #[test]
    fn a_failure_is_reported_in_the_programs_own_words() {
        let failed = SshOutput {
            stdout: String::new(),
            stderr: "\nHost key verification failed.\n".to_string(),
            code: Some(255),
            timed_out: false,
            truncated: false,
        };

        assert!(!failed.ok());
        assert_eq!(failed.reason(), "Host key verification failed.");

        let timed_out = SshOutput { timed_out: true, ..failed };

        assert!(timed_out.reason().contains("timed out"));
        assert!(timed_out.reason().contains("may still be running"));
    }
}

