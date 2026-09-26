//! Sign in once, then reuse the connection (0.8.1).
//!
//! The report: *"kono vabai vps a connect hoi nah"*. The host in it answers every connection with
//! `Permission denied (keyboard-interactive)` - it has public-key login switched **off**, and it asks for
//! a `Verification code:` and then a `Password:` on every new session. The 0.7.13 design (install SDC's
//! key once with the password, then run every call with `BatchMode=yes`) cannot reach such a machine at
//! all: there is no key it will accept, and every file read would need a fresh one-time code.
//!
//! What such a host needs is what a person's own terminal does with `ControlMaster`: authenticate
//! **once**, keep that connection open, and send every later command through it. That is this module:
//!
//! * [`program`] - the `ssh` that can hold a master. Windows' own OpenSSH cannot (it has no multiplexing),
//!   so on Windows the one that ships with Git for Windows is used - SDC already needs Git.
//! * [`control_path`] - where the master's socket lives: SDC's own `ssh` data folder, one per host.
//! * [`sign_in`] - starts the master. Its prompts are answered through `SSH_ASKPASS`, which points at
//!   **this daemon's own binary** ([`answer_prompt`]); the answers come back over a one-shot loopback
//!   listener that only a caller holding its random token can talk to. The password and the code are
//!   used for this one sign-in and kept nowhere - not in the store, not in the log, not on disk.
//!
//! Every other call ([`super::Ssh::base_args`]) goes through that connection with `-O proxy` while it is
//! open ([`mux_options`]), and nothing is asked; when there is none, it uses the key and `BatchMode=yes`
//! exactly as before.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::auth::remote::{key_path, prompt_in, Prompt};
use crate::sdcp::envelope::ErrorObject;

use super::Ssh;

/// The environment variable that turns `sdcd` into an askpass helper: `<port>:<token>`.
pub const ASKPASS_ENV: &str = "SDC_ASKPASS";

/// How long a master stays open with nothing using it. A working day: a person signs in once in the
/// morning, and a laptop left overnight asks again.
const PERSIST: &str = "10h";

/// How long a sign-in may take: a handshake, two prompts, and the fork.
const SIGN_IN_BUDGET: Duration = Duration::from_secs(45);

/// The `ssh` that can hold a master connection, or `None` when this machine has none.
pub fn program() -> Option<PathBuf> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();

    FOUND.get_or_init(find_program).clone()
}

#[cfg(windows)]
fn find_program() -> Option<PathBuf> {
    /* Git for Windows: `<root>\cmd\git.exe` (or `bin`, or `mingw64\bin`) next to `<root>\usr\bin\ssh.exe`.
       That build is Cygwin-based, so it has the Unix-socket emulation a master needs. */
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Some(git) = crate::host::program::resolve("git") {
        roots.extend(git.ancestors().skip(1).take(3).map(PathBuf::from));
    }

    for base in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
        if let Some(dir) = std::env::var_os(base) {
            roots.push(PathBuf::from(&dir).join("Git"));
            roots.push(PathBuf::from(dir).join("Programs").join("Git"));
        }
    }

    roots
        .into_iter()
        .map(|root| root.join("usr").join("bin").join("ssh.exe"))
        .find(|candidate| candidate.is_file())
}

#[cfg(not(windows))]
fn find_program() -> Option<PathBuf> {
    crate::host::program::resolve("ssh")
}

/// A path the chosen `ssh` reads correctly. The Cygwin build takes `C:/…`, which Windows' own OpenSSH
/// takes too - so every path SDC hands to `ssh` is written with forward slashes.
pub fn ssh_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Where this host's master socket lives: `<data>/ssh/cm-<hash>`. Short on purpose - a Unix socket path
/// has a length limit near 100 bytes.
pub fn control_path(ssh: &Ssh) -> Result<PathBuf, ErrorObject> {
    use sha2::{Digest, Sha256};

    let pins = super::hostkey::known_hosts_path()?;
    let folder = pins
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| ErrorObject::internal("SDC's ssh folder has no parent"))?;
    let digest = Sha256::digest(ssh.label().as_bytes());

    Ok(folder.join(format!("cm-{}", &hex::encode(digest)[..16])))
}

/// The options that route a call through the signed-in connection, when one is open. Empty otherwise,
/// so the argument set is what it was: the key, `BatchMode=yes`, nothing asked.
///
/// `-O proxy` rather than the ordinary multiplexing client, and this is measured, not taste: the
/// ordinary client hands its stdin/stdout/stderr to the master as file descriptors, which the Cygwin
/// build Git for Windows ships cannot pass - `-O check` said `Master running` and every command failed
/// with `read from master failed: Connection reset by peer`. Proxy mode speaks the SSH channel protocol
/// over the socket instead, so output, stdin and the exit code all come back (OpenSSH 7.4+, every
/// platform). It needs a live master - there is no fallback inside `ssh` - hence the check first
/// (about 30 ms).
pub fn mux_options(ssh: &Ssh) -> Result<Vec<String>, ErrorObject> {
    if program().is_none() || !is_open(ssh) {
        return Ok(Vec::new());
    }

    Ok(vec![
        "-o".to_string(),
        format!("ControlPath={}", ssh_path(&control_path(ssh)?)),
        "-O".to_string(),
        "proxy".to_string(),
    ])
}

/// Is a master open for this host right now?
pub fn is_open(ssh: &Ssh) -> bool {
    let Some(program) = program() else {
        return false;
    };
    let Ok(path) = control_path(ssh) else {
        return false;
    };

    let mut command = std::process::Command::new(program);

    command
        .args(ssh.target.port_args())
        .args(["-o", &format!("ControlPath={}", ssh_path(&path)), "-O", "check"])
        .arg(&ssh.target.user_host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    hide_window(&mut command);

    command.status().map(|status| status.success()).unwrap_or(false)
}

/// Closes this host's master, if there is one (a removed host, a sign-out).
pub fn close(ssh: &Ssh) {
    let (Some(program), Ok(path)) = (program(), control_path(ssh)) else {
        return;
    };

    let mut command = std::process::Command::new(program);

    command
        .args(ssh.target.port_args())
        .args(["-o", &format!("ControlPath={}", ssh_path(&path)), "-O", "exit"])
        .arg(&ssh.target.user_host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    hide_window(&mut command);

    let _ = command.status();
}

/// Why a sign-in did not finish.
#[derive(Debug)]
pub enum SignInError {
    /// This machine has no `ssh` that can hold a master; the caller uses the key install instead.
    Unsupported,
    /// The host asked for a verification code and none was given. The window asks for one.
    NeedsCode,
    /// Anything else, in a sentence a person can act on.
    Failed(String),
}

/// What the askpass listener saw: which questions were asked (never the answers).
#[derive(Default)]
struct Seen {
    password: bool,
    code: bool,
    code_missing: bool,
    other: Vec<String>,
}

/// Opens the master for this host, answering its prompts with `password` and `code`.
///
/// `code` may be empty: a host that only asks for a password signs in with the password alone, and a
/// host that asks for a code it was not given ends in [`SignInError::NeedsCode`] - which is how the
/// window learns to show the code field.
pub fn sign_in(ssh: &Ssh, password: &str, code: &str) -> Result<String, SignInError> {
    let Some(program) = program() else {
        return Err(SignInError::Unsupported);
    };

    if is_open(ssh) {
        return Ok(format!("signed in to {} · the connection was already open", ssh.label()));
    }

    let failed = |error: ErrorObject| SignInError::Failed(error.message);
    let path = control_path(ssh).map_err(failed)?;
    let pins = super::hostkey::known_hosts_path().map_err(failed)?;

    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }

    /* A socket file left behind by a master that died (a reboot, a killed process) makes the new one
       refuse to bind. `-O check` above said nothing is listening on it, so it is safe to remove. */
    let _ = std::fs::remove_file(&path);

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| SignInError::Failed(format!("SDC could not open its sign-in helper: {error}")))?;
    let port = listener
        .local_addr()
        .map_err(|error| SignInError::Failed(error.to_string()))?
        .port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let deadline = Instant::now() + SIGN_IN_BUDGET;
    let done = Arc::new(AtomicBool::new(false));

    let answers = {
        let token = token.clone();
        let seen = seen.clone();
        let done = done.clone();
        let password = password.to_string();
        let code = code.trim().to_string();

        std::thread::spawn(move || serve_prompts(listener, &token, &password, &code, &seen, deadline, &done))
    };

    let helper = std::env::current_exe()
        .map_err(|error| SignInError::Failed(format!("SDC could not find its own program: {error}")))?;

    let mut command = std::process::Command::new(&program);

    command.args(ssh.target.port_args());

    for option in [
        "ConnectTimeout=10".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
        format!("UserKnownHostsFile={}", ssh_path(&pins)),
        "IdentitiesOnly=yes".to_string(),
        "LogLevel=ERROR".to_string(),
        "BatchMode=no".to_string(),
        "NumberOfPasswordPrompts=1".to_string(),
        "PreferredAuthentications=publickey,keyboard-interactive,password".to_string(),
        "ServerAliveInterval=15".to_string(),
        "ServerAliveCountMax=3".to_string(),
        "ControlMaster=yes".to_string(),
        format!("ControlPersist={PERSIST}"),
        format!("ControlPath={}", ssh_path(&path)),
    ] {
        command.arg("-o").arg(option);
    }

    if let Some(key) = key_path().filter(|key| key.exists()) {
        command.arg("-i").arg(ssh_path(&key));
    }

    /* `-f -N`: authenticate, then go to the background holding the connection. The foreground process
       exits 0 exactly when the sign-in succeeded, which is the answer this function waits for. */
    command
        .args(["-f", "-N"])
        .arg(&ssh.target.user_host)
        .env("SSH_ASKPASS", ssh_path(&helper))
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", "sdc:0")
        .env(ASKPASS_ENV, format!("{port}:{token}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    hide_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| SignInError::Failed(format!("`ssh` could not be started: {error}")))?;

    let stderr = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let mut text = String::new();

        if let Some(mut pipe) = stderr {
            let _ = pipe.read_to_string(&mut text);
        }

        let _ = tx.send(text);
    });

    let code_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            /* A question SDC could not answer ends the attempt now: `ssh` would otherwise sit on the
               cancelled prompt until the budget ran out. */
            Ok(None) if Instant::now() < deadline && !unanswerable(&seen) => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();

                break None;
            }
        }
    };

    done.store(true, Ordering::SeqCst);

    let stderr = rx.recv_timeout(Duration::from_millis(1500)).unwrap_or_default();
    let _ = answers.join();
    let seen = seen.lock().map(|mut seen| std::mem::take(&mut *seen)).unwrap_or_default();

    if code_status == Some(0) && is_open(ssh) {
        let how = match (seen.password, seen.code) {
            (true, true) => "password and verification code",
            (true, false) => "password",
            (false, true) => "verification code",
            (false, false) => "key",
        };

        return Ok(format!(
            "signed in to {} with the {how} · the connection stays open for {PERSIST} and nothing was stored",
            ssh.label()
        ));
    }

    if seen.code_missing {
        return Err(SignInError::NeedsCode);
    }

    let last = stderr
        .lines()
        .map(str::trim)
        .rev()
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string();

    if code_status.is_none() {
        return Err(SignInError::Failed(format!(
            "{} did not finish signing in within {} seconds",
            ssh.label(),
            SIGN_IN_BUDGET.as_secs()
        )));
    }

    if !seen.other.is_empty() {
        return Err(SignInError::Failed(format!(
            "{} asked something SDC does not know how to answer ({}); last line: {last}",
            ssh.label(),
            seen.other.join(" / ")
        )));
    }

    let what = match (seen.password, seen.code) {
        (true, true) => "the password or the verification code",
        (true, false) => "the password",
        (false, true) => "the verification code",
        (false, false) => "the sign-in",
    };

    Err(SignInError::Failed(format!(
        "{} refused {what}{}",
        ssh.label(),
        if last.is_empty() { String::new() } else { format!(": {last}") }
    )))
}

/// True once the helper has been asked something it had no answer for.
fn unanswerable(seen: &Mutex<Seen>) -> bool {
    seen.lock().map(|seen| seen.code_missing || !seen.other.is_empty()).unwrap_or(true)
}

/// The loopback side of the askpass helper: one connection per prompt, until `deadline` or `done`.
fn serve_prompts(
    listener: TcpListener,
    token: &str,
    password: &str,
    code: &str,
    seen: &Mutex<Seen>,
    deadline: Instant,
    done: &AtomicBool,
) {
    let _ = listener.set_nonblocking(true);

    while Instant::now() < deadline && !done.load(Ordering::SeqCst) {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(_) => return,
        };

        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

        let Some(prompt) = read_request(&stream, token) else {
            continue;
        };

        let answer = {
            let Ok(mut seen) = seen.lock() else {
                return;
            };

            match classify(&prompt) {
                Prompt::Password => {
                    seen.password = true;
                    Some(password.to_string())
                }
                Prompt::VerificationCode if !code.is_empty() => {
                    seen.code = true;
                    Some(code.to_string())
                }
                Prompt::VerificationCode => {
                    seen.code_missing = true;
                    None
                }
                _ => {
                    seen.other.push(prompt.trim().to_string());
                    None
                }
            }
        };

        let mut stream = stream;

        /* `ok <answer>` or `no`: an unanswered prompt makes the helper exit non-zero, which `ssh` takes
           as a cancelled question - the sign-in then fails with the reason recorded above. */
        let reply = match answer {
            Some(answer) => format!("ok {answer}\n"),
            None => "no\n".to_string(),
        };

        let _ = stream.write_all(reply.as_bytes());
    }
}

/// `<token>\n<prompt>\n` from the helper, or `None` for a caller that does not hold the token.
fn read_request(stream: &TcpStream, token: &str) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();

    reader.read_line(&mut first).ok()?;

    if first.trim_end() != token {
        return None;
    }

    let mut prompt = String::new();

    reader.read_line(&mut prompt).ok()?;

    Some(prompt.trim_end().to_string())
}

/// What a prompt asks for. `prompt_in` knows `password:` and `verification code`; hosts also say `OTP`,
/// `token` and `authenticator`.
fn classify(prompt: &str) -> Prompt {
    let lowered = prompt.to_lowercase();

    if lowered.contains("otp") || lowered.contains("authenticator") || lowered.contains("token") || lowered.contains("code") {
        return Prompt::VerificationCode;
    }

    prompt_in(prompt)
}

/// `sdcd` started by `ssh` as its `SSH_ASKPASS`: ask the daemon for the answer to `prompt`, print it,
/// and return the exit code. Nothing is written anywhere but `ssh`'s own pipe.
pub fn answer_prompt(spec: &str, prompt: &str) -> i32 {
    let Some((port, token)) = spec.split_once(':') else {
        return 1;
    };
    let Ok(port) = port.parse::<u16>() else {
        return 1;
    };
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return 1;
    };

    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

    /* The prompt is one line on the wire. */
    let one_line = prompt.replace(['\r', '\n'], " ");

    if stream.write_all(format!("{token}\n{one_line}\n").as_bytes()).is_err() {
        return 1;
    }

    let mut reply = String::new();

    if BufReader::new(&stream).read_line(&mut reply).is_err() {
        return 1;
    }

    match reply.trim_end_matches(['\r', '\n']).strip_prefix("ok ") {
        Some(answer) => {
            println!("{answer}");
            0
        }
        None => 1,
    }
}

#[cfg(windows)]
fn hide_window(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;

    /* CREATE_NO_WINDOW: the daemon has no console, and a console-subsystem `ssh` would flash one. */
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut std::process::Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompts_of_a_two_factor_host_are_told_apart() {
        assert_eq!(classify("(deploy@203.0.113.10) Verification code: "), Prompt::VerificationCode);
        assert_eq!(classify("(deploy@203.0.113.10) Password: "), Prompt::Password);
        assert_eq!(classify("deploy@203.0.113.10's password: "), Prompt::Password);
        assert_eq!(classify("One-time password (OTP): "), Prompt::VerificationCode);
        assert_eq!(classify("Enter passphrase for key: "), Prompt::None);
    }

    /// The whole helper round trip, without `ssh`: the listener answers the password and the code, and a
    /// caller without the token gets nothing.
    #[test]
    fn the_helper_is_answered_only_with_the_token() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let deadline = Instant::now() + Duration::from_secs(3);
        let server = {
            let seen = seen.clone();

            std::thread::spawn(move || serve_prompts(listener, "t0k3n", "hunter2", "123456", &seen, deadline, &AtomicBool::new(false)))
        };

        let spec = format!("{port}:t0k3n");

        assert_eq!(answer_prompt(&spec, "(u@h) Verification code: "), 0);
        assert_eq!(answer_prompt(&spec, "(u@h) Password: "), 0);
        assert_eq!(answer_prompt(&format!("{port}:wrong"), "(u@h) Password: "), 1);
        assert_eq!(answer_prompt(&spec, "Something else: "), 1);

        server.join().unwrap();

        let seen = seen.lock().unwrap();

        assert!(seen.password && seen.code && !seen.code_missing);
        assert_eq!(seen.other, vec!["Something else:".to_string()]);
    }

    #[test]
    fn a_missing_code_is_recorded_so_the_window_can_ask_for_one() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let deadline = Instant::now() + Duration::from_secs(2);
        let server = {
            let seen = seen.clone();

            std::thread::spawn(move || serve_prompts(listener, "k", "pw", "", &seen, deadline, &AtomicBool::new(false)))
        };

        assert_eq!(answer_prompt(&format!("{port}:k"), "Verification code: "), 1);

        server.join().unwrap();

        assert!(seen.lock().unwrap().code_missing);
    }

    #[test]
    fn a_control_path_is_short_and_per_host() {
        let one = Ssh::parse("deploy@203.0.113.10 -p 8443").unwrap();
        let two = Ssh::parse("deploy@203.0.113.10").unwrap();

        let (Ok(a), Ok(b)) = (control_path(&one), control_path(&two)) else {
            return;
        };

        assert_ne!(a, b);
        assert!(a.file_name().unwrap().to_string_lossy().len() <= 19);
    }
}
