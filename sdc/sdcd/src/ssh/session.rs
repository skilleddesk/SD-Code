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
    /* A master that is busy (its `-O check` ran out of time) is still the only way in: falling back to
       the key on such a host is a certain `Permission denied (keyboard-interactive)`, which is what cut
       the report's agent off (0.15.2). Only a master that is really gone sends a call around it. */
    if program().is_none() || (!answered_recently(ssh) && state(ssh) == Master::Gone) {
        return Ok(Vec::new());
    }

    Ok(vec![
        "-o".to_string(),
        format!("ControlPath={}", ssh_path(&control_path(ssh)?)),
        "-O".to_string(),
        "proxy".to_string(),
    ])
}

/// The hosts this daemon signed in to and holds a master for (0.11.7), by [`Ssh::label`].
///
/// The watcher looks at these every few seconds with a local `-O check` - no network - so a master that
/// went away is on the host's card in seconds, not after the next 45 s remote probe.
fn signed() -> &'static Mutex<std::collections::HashSet<String>> {
    static SIGNED: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

    SIGNED.get_or_init(Default::default)
}

/// Did this daemon sign in to this host, and has nobody yet reported that sign-in gone?
pub fn was_signed_in(ssh: &Ssh) -> bool {
    signed().lock().map(|set| set.contains(&ssh.label())).unwrap_or(false)
}

/// Forgets a sign-in once its loss has been reported, so it is reported once.
pub fn forget(ssh: &Ssh) {
    if let Ok(mut set) = signed().lock() {
        set.remove(&ssh.label());
    }
}

/// Takes over a live master this daemon did not start (0.11.7) - one left by the daemon an update
/// replaced - so its loss is noticed as quickly as that of one it did.
pub fn adopt(ssh: &Ssh) {
    remember(ssh);
}

fn remember(ssh: &Ssh) {
    if let Ok(mut set) = signed().lock() {
        set.insert(ssh.label());
    }

    /* A master adopted from an older daemon has no spare name yet (0.15.2). */
    if let Ok(path) = control_path(ssh) {
        let spare = spare_path(&path);

        if path.exists() && !spare.exists() {
            let _ = std::fs::hard_link(&path, &spare);
        }
    }
}

/// How long a live `-O check` is trusted by the calls that route through the master (0.11.9).
///
/// Measured against the report's VPS: `-O check` is ~65 ms, a whole proxied command ~500 ms (the host is
/// 217 ms away). An agent runs commands in bursts, and each one paid the check again. Only a *yes* is
/// remembered, and only briefly: a master that died inside the window makes that one call fail with the
/// socket's own error, and the watcher - which always checks afresh - reports it within seconds.
const TRUST_OPEN: Duration = Duration::from_secs(2);

fn checked() -> &'static Mutex<std::collections::HashMap<String, Instant>> {
    static CHECKED: OnceLock<Mutex<std::collections::HashMap<String, Instant>>> = OnceLock::new();

    CHECKED.get_or_init(Default::default)
}

fn answered_recently(ssh: &Ssh) -> bool {
    checked()
        .lock()
        .ok()
        .and_then(|seen| seen.get(&ssh.label()).copied())
        .is_some_and(|at| at.elapsed() < TRUST_OPEN)
}

/// [`is_open`], answered from a *yes* seen in the last [`TRUST_OPEN`] when there is one.
pub fn is_open_recently(ssh: &Ssh) -> bool {
    answered_recently(ssh) || is_open(ssh)
}

/// What a local `-O check` says about a host's master (0.15.2).
///
/// Three answers, not two, and the third is the fix. The report: a VPS dropped to "signed out" the
/// moment a long agent turn ended, while its `ssh` master was still running and still holding the TCP
/// connection. A check that ran out of its budget on a busy machine was read as "the master is gone",
/// and what followed treated a live connection as a dead one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Master {
    /// The master answered.
    Open,
    /// Nothing is listening: no socket, or `ssh` said so.
    Gone,
    /// The check did not finish in time. The master may be busy; it is not reported lost for this.
    Unsure,
}

/// Is a master open for this host right now? Always asks; a *yes* is remembered for [`is_open_recently`].
pub fn is_open(ssh: &Ssh) -> bool {
    state(ssh) == Master::Open
}

/// The master's [`Master`] state, asked afresh. A *yes* is remembered for [`is_open_recently`], a *no*
/// clears it, and an unsure answer leaves it as it was.
pub fn state(ssh: &Ssh) -> Master {
    let answer = check_now(ssh);

    if let Ok(mut seen) = checked().lock() {
        match answer {
            Master::Open => {
                seen.insert(ssh.label(), Instant::now());
            }
            Master::Gone => {
                seen.remove(&ssh.label());
            }
            Master::Unsure => {}
        }
    }

    answer
}

/// The second name of a master's socket (0.15.2): a hard link made right after the sign-in.
///
/// The report's socket file vanished while its master kept running, and a master cannot be asked to
/// listen again - so the sign-in, and the verification code with it, was lost for good. A hard link is
/// the same socket under another name (it is how `ssh` itself puts the socket in place), so when the
/// first name goes, [`restore`] puts it back from this one and the connection is simply there again.
fn spare_path(path: &std::path::Path) -> PathBuf {
    path.with_extension("keep")
}

/// Puts a vanished socket name back from its spare. True when the socket is on disk afterwards.
fn restore(path: &std::path::Path) -> bool {
    if path.exists() {
        return true;
    }

    let spare = spare_path(path);

    spare.exists() && std::fs::hard_link(&spare, path).is_ok()
}

/// Is there a socket for this host on disk, under either name?
pub fn has_socket(ssh: &Ssh) -> bool {
    control_path(ssh).is_ok_and(|path| path.exists() || spare_path(&path).exists())
}

/// Removes both names of a socket whose master is known to be gone.
pub fn discard(ssh: &Ssh) {
    if let Ok(path) = control_path(ssh) {
        let _ = std::fs::remove_file(spare_path(&path));
        let _ = std::fs::remove_file(&path);
    }
}

fn check_now(ssh: &Ssh) -> Master {
    let Some(program) = program() else {
        return Master::Gone;
    };
    let Ok(path) = control_path(ssh) else {
        return Master::Gone;
    };

    restore(&path);

    /* No socket on disk, no master - and no process started. This is the common case (every host that
       never signed in, every test), and it is why a machine where `ssh` itself misbehaves cannot be
       made to wait here: 0.9.0's first Windows CI run sat in `cargo test` for four hours. Cygwin's
       emulated Unix socket is a real file, so the check holds on Windows too. */
    if !path.exists() {
        return Master::Gone;
    }

    let mut command = std::process::Command::new(program);

    command
        .args(ssh.target.port_args())
        .args(["-o", &format!("ControlPath={}", ssh_path(&path)), "-O", "check"])
        .arg(&ssh.target.user_host);

    match bounded(command, CHECK_BUDGET) {
        Some(true) => Master::Open,
        Some(false) => Master::Gone,
        None => Master::Unsure,
    }
}

/// How long `-O check` / `-O exit` may take. Measured at ~30 ms when the machine is quiet; at the end
/// of a long agent turn, with the window refreshing everything at once, it can take longer than the 3 s
/// this used to be - so running out of it now means [`Master::Unsure`], never "gone".
const CHECK_BUDGET: Duration = Duration::from_secs(5);

/// Runs a control command with no stdio and a hard deadline: `Some(success)`, or `None` when it had to
/// be killed. A control command talks to a local socket only, so it never has a reason to wait.
fn bounded(mut command: std::process::Command, budget: Duration) -> Option<bool> {
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    hide_window(&mut command);

    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + budget;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();

                return None;
            }
        }
    }
}

/// Closes this host's master, if there is one (a removed host, a sign-out).
pub fn close(ssh: &Ssh) {
    forget(ssh);

    let (Some(program), Ok(path)) = (program(), control_path(ssh)) else {
        return;
    };

    if !path.exists() {
        return;
    }

    let mut command = std::process::Command::new(program);

    command
        .args(ssh.target.port_args())
        .args(["-o", &format!("ControlPath={}", ssh_path(&path)), "-O", "exit"])
        .arg(&ssh.target.user_host);

    let _ = bounded(command, CHECK_BUDGET);
    let _ = std::fs::remove_file(spare_path(&path));
}

/// A port on the host, reachable from this machine through the signed-in master (0.14.4): the live
/// preview's answer to a dev server the agent started **on the VPS**. `localhost:3000` in that chat is
/// the VPS's own port, and a frame on this machine that loads it loads whatever this machine has there -
/// which is what the report's preview showed.
///
/// One `-O forward` to the master: no new login, no password, no code. The same host and port answer the
/// same local port while the master lives.
pub fn forward(ssh: &Ssh, remote_port: u16) -> Result<u16, ErrorObject> {
    let key = format!("{}#{remote_port}", ssh.label());

    if let Some(local) = forwards().lock().ok().and_then(|known| known.get(&key).copied()) {
        if std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], local).into(), Duration::from_millis(300)).is_ok() {
            return Ok(local);
        }
    }

    let program = program().ok_or_else(|| ErrorObject::internal("no `ssh` that can hold a connection open"))?;
    let path = control_path(ssh)?;

    if check_now(ssh) == Master::Gone {
        return Err(ErrorObject::bad_request(format!(
            "{} is not signed in, so its port {remote_port} cannot be opened here. Sign in to the host first.",
            ssh.label()
        )));
    }

    /* A free port, asked of the OS and released for `ssh` to take a moment later. */
    let local = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| ErrorObject::internal(error.to_string()))?
        .port();
    let mut command = std::process::Command::new(program);

    command
        .args(ssh.target.port_args())
        .args(["-o", &format!("ControlPath={}", ssh_path(&path)), "-O", "forward"])
        .args(["-L", &format!("127.0.0.1:{local}:127.0.0.1:{remote_port}")])
        .arg(&ssh.target.user_host);

    if bounded(command, CHECK_BUDGET) != Some(true) {
        return Err(ErrorObject::internal(format!("{} refused to forward port {remote_port}", ssh.label())));
    }

    if let Ok(mut known) = forwards().lock() {
        known.insert(key, local);
    }

    Ok(local)
}

fn forwards() -> &'static std::sync::Mutex<std::collections::HashMap<String, u16>> {
    static FORWARDS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u16>>> = std::sync::OnceLock::new();

    FORWARDS.get_or_init(Default::default)
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

    /* A master that is only slow to answer is asked again before anything is replaced: removing its
       socket below would cut a live sign-in off for good (0.15.2). */
    let mut master = state(ssh);

    for _ in 0..3 {
        if master != Master::Unsure {
            break;
        }

        std::thread::sleep(Duration::from_secs(1));
        master = state(ssh);
    }

    if master == Master::Open {
        remember(ssh);

        return Ok(format!("signed in to {} · the connection was already open", ssh.label()));
    }

    let failed = |error: ErrorObject| SignInError::Failed(error.message);
    let path = control_path(ssh).map_err(failed)?;
    let pins = super::hostkey::known_hosts_path().map_err(failed)?;

    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }

    /* A socket file left behind by a master that died (a reboot, a killed process) makes the new one
       refuse to bind. `-O check` above said nothing is listening on it, so it is safe to remove - both
       names of it. */
    discard(ssh);

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
        /* The master is the one connection that carries a sign-in nobody can redo without a fresh
           code, so it rides out a flaky network instead of giving up after 45 s (0.11.7): a keepalive
           every 15 s, and only 8 unanswered ones - two minutes of silence - end it. */
        "ServerAliveInterval=15".to_string(),
        "ServerAliveCountMax=8".to_string(),
        "TCPKeepAlive=yes".to_string(),
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
        remember(ssh);

        /* The spare name, so the socket can be put back if its file is ever removed (0.15.2). */
        let spare = spare_path(&path);
        let _ = std::fs::remove_file(&spare);
        let _ = std::fs::hard_link(&path, &spare);

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

    /// A host that never signed in is answered from the disk alone - no `ssh` is started, so nothing
    /// about the machine's `ssh` can make this wait (the 0.9.0 Windows CI hang).
    #[test]
    fn a_host_with_no_socket_is_not_open_and_starts_nothing() {
        let ssh = Ssh::parse("nobody@203.0.113.77 -p 2").unwrap();
        let started = Instant::now();

        assert!(!is_open(&ssh));
        assert!(mux_options(&ssh).unwrap().is_empty());
        assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    }

    /// The limiter (0.11.9): a *yes* seen moments ago is trusted without asking `ssh` again, even for a
    /// host with no real socket - and once [`TRUST_OPEN`] has passed, the cache stops overriding the real
    /// (negative) answer.
    #[test]
    fn a_recent_yes_is_trusted_until_the_window_passes() {
        let ssh = Ssh::parse("nobody@203.0.113.88 -p 3").unwrap();

        checked().lock().unwrap().insert(ssh.label(), Instant::now());

        assert!(is_open_recently(&ssh), "a fresh yes must be trusted without a real check");

        checked().lock().unwrap().insert(ssh.label(), Instant::now() - TRUST_OPEN - Duration::from_millis(50));

        assert!(
            !is_open_recently(&ssh),
            "a stale yes must fall back to the real check, which finds no socket"
        );
    }

    /// The limiter only ever *shortens* a real check into a skip - it must never keep a cached yes
    /// alive past a real check that comes back no, even while the window is still fresh. Otherwise a
    /// closed master would keep answering commands as if it were still open.
    #[test]
    fn a_real_no_clears_a_cached_yes_before_the_window_passes() {
        let ssh = Ssh::parse("nobody@203.0.113.99 -p 4").unwrap();

        checked().lock().unwrap().insert(ssh.label(), Instant::now());

        assert!(!is_open(&ssh), "a host with no real socket must never report open");
        assert!(
            !checked().lock().unwrap().contains_key(&ssh.label()),
            "a real no must remove the stale yes rather than leave it for the window to expire"
        );
        assert!(!is_open_recently(&ssh), "the cleared cache must not be trusted either");
    }

    /// The limiter's actual customer: [`mux_options`] must hand back the proxy options from a fresh
    /// cached yes alone, with no real socket behind it - it never re-checks `ssh` itself, it only asks
    /// [`is_open_recently`].
    #[test]
    fn a_fresh_cache_lets_mux_options_skip_the_real_check() {
        if program().is_none() {
            return;
        }

        let ssh = Ssh::parse("nobody@203.0.113.101 -p 5").unwrap();

        checked().lock().unwrap().insert(ssh.label(), Instant::now());

        let Ok(options) = mux_options(&ssh) else {
            return;
        };

        assert!(!options.is_empty(), "a fresh cached yes must produce proxy options");
        assert!(options.contains(&"proxy".to_string()));
        assert!(options.iter().any(|o| o.starts_with("ControlPath=")));
    }

    /// 0.15.2: the report's socket file vanished while its master kept running. The spare name made at
    /// sign-in puts it back - the same file, so a live master answers on it again.
    #[test]
    fn a_vanished_socket_name_is_put_back_from_its_spare() {
        let folder = std::env::temp_dir().join(format!("sdc-spare-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("cm-0123456789abcdef");

        std::fs::write(&path, "socket").unwrap();
        std::fs::hard_link(&path, spare_path(&path)).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert!(restore(&path), "the name must come back from the spare");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "socket");

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(spare_path(&path)).unwrap();

        assert!(!restore(&path), "with neither name there is nothing to put back");

        let _ = std::fs::remove_dir_all(&folder);
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
