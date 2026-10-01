//! SDC's own SSH connection (0.16.0): the daemon signs in and holds the connection itself.
//!
//! The report: *"ami cai ai rokom sign out jano kono vabai nah hoi"* - the VPS was signed out six times
//! in one day, about every 90 minutes. The event log and the process list said why: the connection was
//! an `ssh -f -N` ControlMaster from Git for Windows, and that process was found at 89% of a core,
//! alive, holding its TCP connection and serving nothing (`muxclient: master hello exchange failed`).
//! 0.15.4 and 0.15.6 could only notice it and kill it, and every kill was a sign-out.
//!
//! So the connection is no longer a process SDC watches from the outside. It is a [`russh`] client
//! inside the daemon:
//!
//! * **one connection per host**, signed in once (password, code, or SDC's key), carrying every command
//!   as its own channel - what the master did, without Cygwin's Unix-socket emulation in the middle;
//! * **keepalives** every 15 s and two minutes of silence before the connection is called dead, the
//!   same tolerance the master had;
//! * **at most [`CHANNELS`] channels at once**, below OpenSSH's `MaxSessions 10`, so a burst of agent
//!   commands waits for a slot instead of being refused with `administratively prohibited`;
//! * **it signs in again by itself** when the connection drops and SDC has what it needs: the password
//!   (kept in memory for the daemon's life) and, for a host that also asks for a code, the
//!   authenticator key the person gave to "Stay signed in" ([`super::totp`]). A command that arrives
//!   while that happens waits for it instead of failing.
//!
//! Every caller still speaks `ssh`: [`exec`] answers [`super::Ssh::run`], [`super::bridge`] gives the
//! streaming callers (a turn's CLI, an MCP server, the Terminal) a process that behaves like `ssh`, and
//! [`forward`] answers the live preview. A host that never signed in this way keeps the old path.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use russh::client;
use russh::keys::PublicKey;
use russh::{ChannelMsg, MethodKind};
use tokio::sync::Semaphore;

use super::session::SignInError;
use super::{Ssh, SshOutput};
use crate::auth::remote::{prompt_in, Prompt};

/// A keepalive every this long...
const KEEPALIVE: Duration = Duration::from_secs(15);
/// ...and this many unanswered ones end the connection: two minutes, as the master had.
const KEEPALIVE_MAX: usize = 8;
/// Channels open at once. OpenSSH's default `MaxSessions` is 10; one is left for a person's own use.
pub const CHANNELS: usize = 9;
/// The TCP connection and the key exchange.
const CONNECT_BUDGET: Duration = Duration::from_secs(20);
/// How long a command waits for a connection that is being signed in again.
const RECONNECT_WAIT: Duration = Duration::from_secs(40);
/// How often the supervisor looks at a connection.
const LOOK_EVERY: Duration = Duration::from_secs(2);
/// The longest gap between two attempts to sign in again after a network failure.
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// How much of one stream [`exec`] keeps (the same limit the `ssh` path has).
const CAPTURE_LIMIT: usize = 256 * 1024;

/// The runtime every connection lives on. Its own, so a caller on any thread - a blocking task, a
/// plain thread, a test - can wait for an answer without being inside the daemon's runtime.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("sdc-ssh")
            .enable_all()
            .build()
            .expect("the SSH runtime starts")
    })
}

/// Runs `future` on [`runtime`] and waits for it, from any thread.
pub(crate) fn block<F, T>(future: F) -> Option<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();

    runtime().spawn(async move {
        let _ = tx.send(future.await);
    });

    rx.recv().ok()
}

/// Spawns `future` on [`runtime`] without waiting.
pub(crate) fn spawn<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    runtime().spawn(future);
}

/// What SDC may use to sign in to a host again without asking.
#[derive(Clone, Default)]
pub struct Credentials {
    pub password: String,
    /// The authenticator's key, decoded ([`super::totp::decode_secret`]).
    pub totp: Option<Vec<u8>>,
}

/// Where a host's status goes: its row in the store and a `HostStatus` for the window (`watch.rs`).
pub trait Reporter: Send + Sync {
    fn report(&self, ssh: &Ssh, status: &str, detail: &str);
}

static REPORTER: OnceLock<Box<dyn Reporter>> = OnceLock::new();

/// Connects the status of every connection to the daemon's store and window. Called once at start.
pub fn attach(reporter: Box<dyn Reporter>) {
    let _ = REPORTER.set(reporter);
}

fn report(ssh: &Ssh, status: &str, detail: &str) {
    if let Some(reporter) = REPORTER.get() {
        reporter.report(ssh, status, detail);
    }
}

/// The server's key, checked against SDC's own pins - the same rule `StrictHostKeyChecking=yes` was.
struct Client {
    pins: Vec<String>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        let line = key.to_openssh().unwrap_or_default();
        let blob = line.split_whitespace().nth(1).unwrap_or_default();

        Ok(!blob.is_empty() && self.pins.iter().any(|pin| pin == blob))
    }
}

type Handle = client::Handle<Client>;

/// A live connection and its channel slots.
#[derive(Clone)]
struct Conn {
    handle: Arc<Handle>,
    slots: Arc<Semaphore>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Signed in and answering.
    Up,
    /// The connection dropped and SDC is signing in again by itself.
    Reconnecting,
    /// Gone, and a person has to sign in.
    Down,
}

struct Host {
    ssh: Ssh,
    creds: Credentials,
    /// The host asked for a code at the last sign-in, so a sign-in without one cannot succeed.
    wants_code: bool,
    /// SDC's key was accepted, so it can sign in again without a password.
    with_key: bool,
    conn: Option<Conn>,
    phase: Phase,
    /// Bumped by every sign-in and every close, so an old supervisor knows it is no longer in charge.
    generation: u64,
    signed_in_at: Instant,
}

fn hosts() -> &'static Mutex<HashMap<String, Host>> {
    static HOSTS: OnceLock<Mutex<HashMap<String, Host>>> = OnceLock::new();

    HOSTS.get_or_init(Default::default)
}

/// Does SDC hold (or is it re-establishing) its own connection to this host?
pub fn manages(ssh: &Ssh) -> bool {
    hosts().lock().is_ok_and(|hosts| hosts.get(&ssh.label()).is_some_and(|host| host.phase != Phase::Down))
}

/// Has SDC signed in to this host its own way - up, being signed in again, or dropped and waiting for a
/// person? A dropped one is still SDC's: its calls are answered "sign in again" here instead of dialing
/// the host with a key it refuses (each refusal is a penalty against this PC).
pub fn known(ssh: &Ssh) -> bool {
    hosts().lock().is_ok_and(|hosts| hosts.contains_key(&ssh.label()))
}

/// The connection's phase, when SDC has one for this host.
pub fn phase(ssh: &Ssh) -> Option<Phase> {
    let hosts = hosts().lock().ok()?;
    let host = hosts.get(&ssh.label())?;

    match (&host.conn, host.phase) {
        (Some(conn), Phase::Up) if conn.handle.is_closed() => Some(Phase::Reconnecting),
        (_, phase) => Some(phase),
    }
}

/// Is the connection up right now?
pub fn is_live(ssh: &Ssh) -> bool {
    phase(ssh) == Some(Phase::Up)
}

/// [`is_live`] by [`Ssh::label`].
pub fn is_live_label(label: &str) -> bool {
    hosts().lock().is_ok_and(|hosts| {
        hosts.get(label).is_some_and(|host| host.phase == Phase::Up && host.conn.as_ref().is_some_and(|conn| !conn.handle.is_closed()))
    })
}

/// Can this host be signed in to again without a person - and is "Stay signed in" on?
pub fn stays_signed_in(ssh: &Ssh) -> bool {
    hosts()
        .lock()
        .is_ok_and(|hosts| hosts.get(&ssh.label()).is_some_and(can_resume))
}

fn can_resume(host: &Host) -> bool {
    (!host.creds.password.is_empty() || host.with_key) && (!host.wants_code || host.creds.totp.is_some())
}

/// Gives a signed-in host the authenticator key, so a drop is signed in again without a code.
pub fn set_totp(ssh: &Ssh, secret: Option<Vec<u8>>) {
    if let Ok(mut hosts) = hosts().lock() {
        if let Some(host) = hosts.get_mut(&ssh.label()) {
            host.creds.totp = secret;
        }
    }
}

/// What the last sign-in used, for the sentences: `(password, code)`.
#[derive(Default, Clone, Copy)]
pub struct Used {
    pub password: bool,
    pub code: bool,
    pub key: bool,
}

/// Signs in and keeps the connection. `code` may be empty when `creds.totp` can make one, or when the
/// host does not ask.
pub fn sign_in(ssh: &Ssh, creds: Credentials, code: &str) -> Result<Used, SignInError> {
    let (signing, code_owned) = (ssh.clone(), code.trim().to_string());
    let attempt_creds = creds.clone();

    let (handle, used) = block(async move { open(&signing, &attempt_creds, &code_owned).await })
        .unwrap_or_else(|| Err(SignInError::Failed("the sign-in could not be run".into())))?;

    install(ssh, creds, used, handle);

    Ok(used)
}

/// Records a new connection for `ssh` and starts the supervisor that keeps it.
fn install(ssh: &Ssh, creds: Credentials, used: Used, handle: Handle) {
    let generation = {
        let Ok(mut hosts) = hosts().lock() else {
            return;
        };
        let previous = hosts.remove(&ssh.label());
        let generation = previous.as_ref().map(|host| host.generation + 1).unwrap_or(1);

        if let Some(old) = previous.and_then(|host| host.conn) {
            spawn(async move {
                let _ = old.handle.disconnect(russh::Disconnect::ByApplication, "replaced", "en").await;
            });
        }

        hosts.insert(
            ssh.label(),
            Host {
                ssh: ssh.clone(),
                creds,
                wants_code: used.code,
                with_key: used.key,
                conn: Some(Conn { handle: Arc::new(handle), slots: Arc::new(Semaphore::new(CHANNELS)) }),
                phase: Phase::Up,
                generation,
                signed_in_at: Instant::now(),
            },
        );

        generation
    };

    spawn(supervise(ssh.label(), generation));
}

/// Ends SDC's connection to a host on purpose (Sign out, host removed). Nothing signs it in again.
pub fn close(ssh: &Ssh) {
    let conn = hosts().lock().ok().and_then(|mut hosts| hosts.remove(&ssh.label())).and_then(|host| host.conn);

    if let Some(conn) = conn {
        block(async move {
            let _ = conn.handle.disconnect(russh::Disconnect::ByApplication, "signed out", "en").await;
        });
    }
}

/// Opens a connection and authenticates.
async fn open(ssh: &Ssh, creds: &Credentials, code: &str) -> Result<(Handle, Used), SignInError> {
    let failed = |message: String| SignInError::Failed(message);
    let pins = pins_for(ssh).map_err(failed)?;

    if pins.is_empty() {
        return Err(failed(format!("{}: Host key verification failed. SDC has no pinned key for it.", ssh.label())));
    }

    let config = Arc::new(client::Config {
        keepalive_interval: Some(KEEPALIVE),
        keepalive_max: KEEPALIVE_MAX,
        inactivity_timeout: None,
        nodelay: true,
        ..Default::default()
    });
    let host = super::hostkey::host_of(&ssh.target.user_host);
    let port = ssh.target.port.unwrap_or(22);

    let mut handle = match tokio::time::timeout(CONNECT_BUDGET, client::connect(config, (host.as_str(), port), Client { pins })).await {
        Err(_) => return Err(failed(format!("{} did not answer within {} s", ssh.label(), CONNECT_BUDGET.as_secs()))),
        Ok(Err(russh::Error::UnknownKey)) => {
            return Err(failed(format!(
                "{}: Host key verification failed. It presents a key that is not the one SDC pinned.",
                ssh.label()
            )))
        }
        Ok(Err(error)) => return Err(failed(format!("{} could not be reached: {error}", ssh.label()))),
        Ok(Ok(handle)) => handle,
    };

    let used = authenticate(&mut handle, ssh, creds, code).await?;

    Ok((handle, used))
}

/// The key blobs SDC pinned for this host.
fn pins_for(ssh: &Ssh) -> Result<Vec<String>, String> {
    #[cfg(test)]
    if let Some(pins) = test_pins().lock().ok().and_then(|pins| pins.get(&ssh.label()).cloned()) {
        return Ok(pins);
    }

    let path = super::hostkey::known_hosts_path().map_err(|error| error.message)?;

    Ok(super::hostkey::pinned_in(&path, &ssh.target)
        .map_err(|error| error.message)?
        .into_iter()
        .map(|key| key.base64)
        .collect())
}

/// Pins for the test server, so a test never writes to the real `known_hosts`.
#[cfg(test)]
pub(crate) fn test_pins() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static PINS: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();

    PINS.get_or_init(Default::default)
}

fn user_of(ssh: &Ssh) -> String {
    match ssh.target.user_host.rsplit_once('@') {
        Some((user, _)) => user.to_string(),
        None => std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "root".to_string()),
    }
}

/// `none` first, to learn the methods; then SDC's key, the host's questions, or the password - as many
/// rounds as the host asks for (a host can want a key *and* a code).
async fn authenticate(handle: &mut Handle, ssh: &Ssh, creds: &Credentials, code: &str) -> Result<Used, SignInError> {
    let user = user_of(ssh);
    let fail = |error: russh::Error| SignInError::Failed(format!("{}: {error}", ssh.label()));
    let mut used = Used::default();

    let mut methods = match handle.authenticate_none(&user).await.map_err(fail)? {
        client::AuthResult::Success => return Ok(used),
        client::AuthResult::Failure { remaining_methods, .. } => remaining_methods,
    };

    let (mut tried_key, mut tried_questions, mut tried_password) = (false, 0, false);

    loop {
        let has = |kind: MethodKind| methods.contains(&kind);
        let key = crate::auth::remote::key_path().filter(|path| path.exists());

        let answer = if has(MethodKind::PublicKey) && !tried_key && key.is_some() {
            tried_key = true;

            let Some(secret) = key.and_then(|path| russh::keys::load_secret_key(path, None).ok()) else {
                continue;
            };
            let hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
            let result = handle
                .authenticate_publickey(&user, russh::keys::PrivateKeyWithHashAlg::new(Arc::new(secret), hash))
                .await
                .map_err(fail)?;

            if matches!(result, client::AuthResult::Success | client::AuthResult::Failure { partial_success: true, .. }) {
                used.key = true;
            }

            result
        } else if has(MethodKind::KeyboardInteractive) && tried_questions < 2 {
            tried_questions += 1;

            answer_questions(handle, ssh, &user, creds, code, &mut used).await?
        } else if has(MethodKind::Password) && !tried_password && !creds.password.is_empty() {
            tried_password = true;
            used.password = true;

            handle.authenticate_password(&user, &creds.password).await.map_err(fail)?
        } else {
            return Err(refused(ssh, used));
        };

        match answer {
            client::AuthResult::Success => return Ok(used),
            client::AuthResult::Failure { remaining_methods, partial_success } => {
                /* A refused password or code is final: asking again with the same answers only adds a
                   penalty against this PC on the host. */
                if !partial_success && (used.password || used.code) {
                    return Err(refused(ssh, used));
                }

                methods = remaining_methods;
            }
        }
    }
}

fn refused(ssh: &Ssh, used: Used) -> SignInError {
    let what = match (used.password, used.code) {
        (true, true) => "the password or the verification code",
        (true, false) => "the password",
        (false, true) => "the verification code",
        (false, false) => "the sign-in",
    };

    SignInError::Failed(format!("{} refused {what}: Permission denied (keyboard-interactive).", ssh.label()))
}

/// The host's own questions (keyboard-interactive): `Password:`, `Verification code:`.
async fn answer_questions(
    handle: &mut Handle,
    ssh: &Ssh,
    user: &str,
    creds: &Credentials,
    code: &str,
    used: &mut Used,
) -> Result<client::AuthResult, SignInError> {
    use client::KeyboardInteractiveAuthResponse as Reply;

    let fail = |error: russh::Error| SignInError::Failed(format!("{}: {error}", ssh.label()));
    let mut reply = handle
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await
        .map_err(fail)?;

    loop {
        match reply {
            Reply::Success => return Ok(client::AuthResult::Success),
            Reply::Failure { remaining_methods, partial_success } => {
                return Ok(client::AuthResult::Failure { remaining_methods, partial_success })
            }
            Reply::InfoRequest { prompts, .. } => {
                let mut answers = Vec::with_capacity(prompts.len());

                for prompt in &prompts {
                    answers.push(answer_for(ssh, &prompt.prompt, creds, code, used)?);
                }

                reply = handle.authenticate_keyboard_interactive_respond(answers).await.map_err(fail)?;
            }
        }
    }
}

/// One question's answer, or the reason there is none.
fn answer_for(ssh: &Ssh, question: &str, creds: &Credentials, code: &str, used: &mut Used) -> Result<String, SignInError> {
    let lowered = question.to_lowercase();
    let kind = match prompt_in(question) {
        Prompt::None if lowered.contains("code") || lowered.contains("token") || lowered.contains("otp") => Prompt::VerificationCode,
        Prompt::None if lowered.contains("password") => Prompt::Password,
        other => other,
    };

    match kind {
        Prompt::Password if !creds.password.is_empty() => {
            used.password = true;

            Ok(creds.password.clone())
        }
        Prompt::Password => Err(SignInError::Failed(format!("{} asks for a password. Type it in the Sign in card.", ssh.label()))),
        Prompt::VerificationCode => {
            used.code = true;

            if !code.is_empty() {
                Ok(code.to_string())
            } else if let Some(secret) = &creds.totp {
                Ok(super::totp::code_now(secret))
            } else {
                Err(SignInError::NeedsCode)
            }
        }
        _ => Err(SignInError::Failed(format!(
            "{} asked something SDC does not know how to answer ({})",
            ssh.label(),
            question.trim()
        ))),
    }
}

/// Watches one connection and signs it in again when it drops, for as long as SDC has what it needs.
async fn supervise(label: String, generation: u64) {
    loop {
        tokio::time::sleep(LOOK_EVERY).await;

        let (ssh, closed, resumable) = {
            let Ok(hosts) = hosts().lock() else {
                return;
            };
            let Some(host) = hosts.get(&label).filter(|host| host.generation == generation) else {
                return;
            };
            let closed = host.conn.as_ref().is_none_or(|conn| conn.handle.is_closed());

            (host.ssh.clone(), closed, can_resume(host))
        };

        if !closed {
            continue;
        }

        let lasted = hosts()
            .lock()
            .ok()
            .and_then(|hosts| hosts.get(&label).map(|host| host.signed_in_at.elapsed()))
            .unwrap_or_default();

        if !resumable {
            mark(&label, generation, Phase::Down);
            report(
                &ssh,
                "offline",
                &format!(
                    "the connection to {} dropped after {} (the network or the host ended it). Sign in again to go on - turn on \"Stay signed in\" in the Sign in card and SDC signs in again by itself next time.",
                    ssh.label(),
                    human(lasted)
                ),
            );

            return;
        }

        mark(&label, generation, Phase::Reconnecting);
        report(&ssh, "connecting", &format!("the connection to {} dropped - signing in again by itself…", ssh.label()));

        let mut wait = Duration::from_secs(2);

        loop {
            let creds = match hosts().lock().ok().and_then(|hosts| {
                hosts.get(&label).filter(|host| host.generation == generation).map(|host| host.creds.clone())
            }) {
                Some(creds) => creds,
                None => return,
            };

            match open(&ssh, &creds, "").await {
                Ok((handle, _)) => {
                    let Ok(mut hosts) = hosts().lock() else {
                        return;
                    };
                    let Some(host) = hosts.get_mut(&label).filter(|host| host.generation == generation) else {
                        return;
                    };

                    host.conn = Some(Conn { handle: Arc::new(handle), slots: Arc::new(Semaphore::new(CHANNELS)) });
                    host.phase = Phase::Up;
                    host.signed_in_at = Instant::now();
                    drop(hosts);

                    report(&ssh, "connected", &format!("{} is reachable · signed in again by itself", ssh.label()));

                    break;
                }
                /* A code used once cannot be used again in the same 30 s (`DISALLOW_REUSE`): the next one. */
                Err(SignInError::Failed(sentence)) if sentence.contains("refused") && creds.totp.is_some() && wait < Duration::from_secs(40) => {
                    tokio::time::sleep(Duration::from_secs(super::totp::seconds_left() + 1)).await;
                    wait = Duration::from_secs(40);
                }
                Err(SignInError::Failed(sentence)) if sentence.contains("refused") || sentence.contains("Host key verification") => {
                    mark(&label, generation, Phase::Down);
                    report(&ssh, "offline", &format!("SDC could not sign in to {} again by itself: {sentence}", ssh.label()));

                    return;
                }
                Err(SignInError::NeedsCode) => {
                    mark(&label, generation, Phase::Down);
                    report(
                        &ssh,
                        "offline",
                        &format!("the connection to {} dropped, and signing in again needs a verification code. Sign in, and turn on \"Stay signed in\" so SDC can do this by itself.", ssh.label()),
                    );

                    return;
                }
                /* The network: keep trying, more slowly, for as long as it takes. */
                Err(_) => {
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(BACKOFF_MAX);
                }
            }
        }
    }
}

fn mark(label: &str, generation: u64, phase: Phase) {
    if let Ok(mut hosts) = hosts().lock() {
        if let Some(host) = hosts.get_mut(label).filter(|host| host.generation == generation) {
            host.phase = phase;

            if phase == Phase::Down {
                host.conn = None;
            }
        }
    }
}

fn human(lasted: Duration) -> String {
    let minutes = lasted.as_secs() / 60;

    if minutes < 2 {
        format!("{} s", lasted.as_secs())
    } else if minutes < 120 {
        format!("{minutes} min")
    } else {
        format!("{} h {} min", minutes / 60, minutes % 60)
    }
}

/// The live connection, waiting for one that is being signed in again. `None` when there is none to
/// wait for.
async fn connection(label: &str) -> Option<Conn> {
    let deadline = Instant::now() + RECONNECT_WAIT;

    loop {
        let (conn, phase) = {
            let hosts = hosts().lock().ok()?;
            let host = hosts.get(label)?;

            (host.conn.clone(), host.phase)
        };

        match (conn, phase) {
            (Some(conn), Phase::Up) if !conn.handle.is_closed() => return Some(conn),
            (_, Phase::Down) => return None,
            _ if Instant::now() >= deadline => return None,
            _ => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}

/// A channel on the host's connection, with its slot held for as long as the channel is.
pub(crate) async fn open_channel(label: &str, budget: Duration) -> Result<(russh::Channel<client::Msg>, tokio::sync::OwnedSemaphorePermit), String> {
    for attempt in 0..3 {
        let Some(conn) = connection(label).await else {
            return Err(format!("{label} is not signed in"));
        };
        let permit = tokio::time::timeout(budget, conn.slots.clone().acquire_owned())
            .await
            .map_err(|_| format!("{label} is busy: {CHANNELS} commands are already running on it"))?
            .map_err(|_| format!("{label} closed"))?;

        match conn.handle.channel_open_session().await {
            Ok(channel) => return Ok((channel, permit)),
            /* The host said no to one more session (its `MaxSessions` is lower than ours): wait a little. */
            Err(russh::Error::ChannelOpenFailure(_)) if attempt < 2 => {
                drop(permit);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            /* The connection itself is gone: the supervisor notices within seconds; wait for it. */
            Err(_) if attempt < 2 => {
                drop(permit);
                tokio::time::sleep(LOOK_EVERY * 2).await;
            }
            Err(error) => return Err(format!("{label}: {error}")),
        }
    }

    Err(format!("{label} could not open a channel"))
}

/// Runs `script` on the host over SDC's own connection. `None` when SDC does not manage this host, so
/// the caller uses the `ssh` path.
pub fn exec(ssh: &Ssh, script: &str, input: Option<&str>, timeout: Duration) -> Option<SshOutput> {
    if !known(ssh) {
        return None;
    }

    let (label, script, input, user_host) = (ssh.label(), script.to_string(), input.map(str::to_string), ssh.target.user_host.clone());

    block(async move {
        let started = Instant::now();

        match open_channel(&label, timeout).await {
            Ok((channel, permit)) => {
                let left = timeout.saturating_sub(started.elapsed()).max(Duration::from_secs(1));
                let output = collect(channel, script, input, left).await;

                drop(permit);
                output
            }
            Err(reason) => SshOutput {
                stdout: String::new(),
                stderr: format!("{user_host}: Permission denied (keyboard-interactive). SDC's connection: {reason}"),
                code: Some(255),
                timed_out: false,
                truncated: false,
            },
        }
    })
}

async fn collect(mut channel: russh::Channel<client::Msg>, script: String, input: Option<String>, timeout: Duration) -> SshOutput {
    let mut output = SshOutput { stdout: String::new(), stderr: String::new(), code: None, timed_out: false, truncated: false };

    if let Err(error) = channel.exec(true, script.into_bytes()).await {
        output.stderr = format!("the command could not be started: {error}");
        output.code = Some(255);

        return output;
    }

    if let Some(text) = input {
        let _ = channel.data_bytes(text.into_bytes()).await;
    }

    /* Like `ssh` with stdin at its end: a command that reads stdin gets EOF instead of waiting. */
    let _ = channel.eof().await;

    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        match tokio::time::timeout_at(deadline, channel.wait()).await {
            Err(_) => {
                output.timed_out = true;
                let _ = channel.close().await;

                break;
            }
            Ok(None) | Ok(Some(ChannelMsg::Close)) => break,
            Ok(Some(ChannelMsg::Data { data })) => keep(&mut stdout, &data, &mut output.truncated),
            Ok(Some(ChannelMsg::ExtendedData { data, ext: 1 })) => keep(&mut stderr, &data, &mut output.truncated),
            Ok(Some(ChannelMsg::ExitStatus { exit_status })) => output.code = Some(exit_status as i32),
            Ok(Some(ChannelMsg::ExitSignal { signal_name, .. })) => {
                output.code = Some(255);
                stderr.extend_from_slice(format!("killed by signal {signal_name:?}\n").as_bytes());
            }
            Ok(Some(_)) => {}
        }
    }

    output.stdout = String::from_utf8_lossy(&stdout).to_string();
    output.stderr = String::from_utf8_lossy(&stderr).to_string();

    if output.code.is_none() && !output.timed_out {
        output.code = Some(255);
    }

    output
}

fn keep(buffer: &mut Vec<u8>, data: &[u8], truncated: &mut bool) {
    let room = CAPTURE_LIMIT.saturating_sub(buffer.len());

    if data.len() > room {
        *truncated = true;
    }

    buffer.extend_from_slice(&data[..data.len().min(room)]);
}

/// A port on the host, answered on a local port through SDC's connection (the live preview).
pub fn forward(ssh: &Ssh, remote_port: u16) -> Option<Result<u16, String>> {
    if !known(ssh) {
        return None;
    }

    let key = format!("{}#{remote_port}", ssh.label());

    if let Some(local) = forwards().lock().ok().and_then(|known| known.get(&key).copied()) {
        return Some(Ok(local));
    }

    let label = ssh.label();

    let bound = block(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.map_err(|error| error.to_string())?;
        let local = listener.local_addr().map_err(|error| error.to_string())?.port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let label = label.clone();

                tokio::spawn(async move {
                    let Some(conn) = connection(&label).await else {
                        return;
                    };
                    let Ok(permit) = conn.slots.clone().acquire_owned().await else {
                        return;
                    };

                    if let Ok(channel) = conn
                        .handle
                        .channel_open_direct_tcpip("127.0.0.1", remote_port as u32, "127.0.0.1", local as u32)
                        .await
                    {
                        let mut stream = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                    }

                    drop(permit);
                });
            }
        });

        Ok::<u16, String>(local)
    })
    .unwrap_or_else(|| Err("the SSH runtime did not answer".to_string()));

    if let Ok(local) = &bound {
        if let Ok(mut known) = forwards().lock() {
            known.insert(key, *local);
        }
    }

    Some(bound)
}

fn forwards() -> &'static Mutex<HashMap<String, u16>> {
    static FORWARDS: OnceLock<Mutex<HashMap<String, u16>>> = OnceLock::new();

    FORWARDS.get_or_init(Default::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials(password: &str, totp: Option<Vec<u8>>) -> Credentials {
        Credentials { password: password.to_string(), totp }
    }

    #[test]
    fn each_question_gets_the_answer_it_asks_for() {
        let ssh = Ssh::parse("ssh -p 8443 deploy@203.0.113.10").unwrap();
        let mut used = Used::default();
        let creds = credentials("hunter2", None);

        assert_eq!(answer_for(&ssh, "Password: ", &creds, "123456", &mut used).unwrap(), "hunter2");
        assert_eq!(answer_for(&ssh, "Verification code: ", &creds, "123456", &mut used).unwrap(), "123456");
        assert!(used.password && used.code);

        assert!(matches!(answer_for(&ssh, "Verification code: ", &creds, "", &mut used), Err(SignInError::NeedsCode)));
        assert!(matches!(answer_for(&ssh, "Favourite colour? ", &creds, "", &mut used), Err(SignInError::Failed(_))));
    }

    #[test]
    fn with_the_authenticator_key_the_code_is_made_here() {
        let ssh = Ssh::parse("deploy@203.0.113.10").unwrap();
        let secret = super::super::totp::decode_secret(&super::super::totp::tests::rfc_test_setup()).unwrap();
        let mut used = Used::default();
        let code = answer_for(&ssh, "Verification code: ", &credentials("pw", Some(secret.clone())), "", &mut used).unwrap();

        assert_eq!(code.len(), 6);
        assert_eq!(code, super::super::totp::code_now(&secret));
    }

    #[test]
    fn only_a_host_with_everything_it_needs_is_signed_in_again_by_itself() {
        let ssh = Ssh::parse("deploy@203.0.113.10").unwrap();
        let host = |password: &str, wants_code: bool, totp: Option<Vec<u8>>| Host {
            ssh: ssh.clone(),
            creds: credentials(password, totp),
            wants_code,
            with_key: false,
            conn: None,
            phase: Phase::Up,
            generation: 1,
            signed_in_at: Instant::now(),
        };

        assert!(can_resume(&host("pw", false, None)), "a password-only host");
        assert!(!can_resume(&host("pw", true, None)), "a code nobody can make");
        assert!(can_resume(&host("pw", true, Some(vec![1; 10]))), "Stay signed in");
        assert!(!can_resume(&host("", false, None)), "a key-only host has nothing to type, and is not this path");
    }

    #[test]
    fn a_host_sdc_never_signed_in_to_keeps_the_ssh_path() {
        let ssh = Ssh::parse("ssh -p 2201 nobody@203.0.113.99").unwrap();

        assert!(!manages(&ssh));
        assert!(exec(&ssh, "true", None, Duration::from_secs(1)).is_none());
        assert!(forward(&ssh, 3000).is_none());
    }

    /// The channel limiter (0.16.0): [`open_channel`] hands out at most [`CHANNELS`] slots per
    /// connection - what keeps a burst of commands under OpenSSH's `MaxSessions` instead of being
    /// refused outright. A caller past that waits for one to free up, as [`connection`]'s own timeout
    /// does around it.
    #[tokio::test]
    async fn only_channels_many_slots_are_free_at_once() {
        let slots = Arc::new(Semaphore::new(CHANNELS));
        let held: Vec<_> = (0..CHANNELS).map(|_| slots.clone().try_acquire_owned().unwrap()).collect();

        assert!(
            tokio::time::timeout(Duration::from_millis(50), slots.clone().acquire_owned()).await.is_err(),
            "no slot must be free while all CHANNELS are held"
        );

        drop(held.into_iter().next().unwrap());

        assert!(
            tokio::time::timeout(Duration::from_millis(50), slots.acquire_owned()).await.is_ok(),
            "freeing one slot must let the next caller in"
        );
    }
}
