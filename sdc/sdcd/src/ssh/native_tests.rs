//! SDC's own connection against a real SSH server on loopback (0.16.0).
//!
//! The server is `russh`'s, run in-process: it asks `Password:` and `Verification code:` like the
//! report's VPS, runs `echo`-like commands, echoes stdin back for `cat`, and can be cut off from the
//! test - which is how "the connection dropped and SDC signed in again by itself" is proven without a
//! network or a person typing a code.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::PrivateKey;
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet};

use super::native::{self, Credentials, Phase, CHANNELS};
use super::{totp, Ssh};

/// RFC 6238's test key (`12345678901234567890`) in base32, built here rather than written out whole.
fn secret_text() -> String {
    super::totp::tests::rfc_test_setup()
}

#[derive(Clone)]
struct Server {
    signed_in: Arc<AtomicUsize>,
    /// `cat` channels: what arrived on stdin is sent back.
    cats: Arc<Mutex<HashMap<ChannelId, ()>>>,
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Reject { proceed_with_methods: Some(MethodSet::from(&[MethodKind::KeyboardInteractive][..])), partial_success: false })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(response) = response else {
            return Ok(Auth::Partial {
                name: Cow::Borrowed(""),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false), (Cow::Borrowed("Verification code: "), false)]),
            });
        };
        let answers: Vec<String> = response.map(|bytes| String::from_utf8_lossy(&bytes).to_string()).collect();
        let secret = totp::decode_secret(&secret_text()).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let good_code = [now.saturating_sub(30), now, now + 30].iter().any(|at| answers.get(1) == Some(&totp::code_at(&secret, *at)));

        if answers.first().map(String::as_str) == Some("pw") && good_code {
            self.signed_in.fetch_add(1, Ordering::SeqCst);

            return Ok(Auth::Accept);
        }

        Ok(Auth::reject())
    }

    async fn channel_open_session(&mut self, _channel: Channel<Msg>, reply: server::ChannelOpenHandle, _session: &mut Session) -> Result<(), Self::Error> {
        reply.accept().await;

        Ok(())
    }

    async fn exec_request(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).to_string();

        session.channel_success(channel)?;

        if command == "cat" {
            self.cats.lock().unwrap().insert(channel, ());

            return Ok(());
        }

        if let Some(seconds) = command.strip_prefix("sleep ") {
            let handle = session.handle();
            let seconds: u64 = seconds.parse().unwrap_or(1);

            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(seconds)).await;
                let _ = handle.data(channel, bytes_of("slept\n")).await;
                let _ = handle.exit_status_request(channel, 0).await;
                let _ = handle.eof(channel).await;
                let _ = handle.close(channel).await;
            });

            return Ok(());
        }

        session.data(channel, bytes_of(&format!("ran: {command}\n")))?;
        session.extended_data(channel, 1, bytes_of("a warning\n"))?;
        session.exit_status_request(channel, 3)?;
        session.eof(channel)?;
        session.close(channel)?;

        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        if self.cats.lock().unwrap().contains_key(&channel) {
            session.data(channel, bytes_of(&String::from_utf8_lossy(data)))?;
        }

        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        if self.cats.lock().unwrap().remove(&channel).is_some() {
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            session.close(channel)?;
        }

        Ok(())
    }
}

fn bytes_of(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// A running test server: its port, its key's blob, and a switch that cuts every connection.
struct Running {
    port: u16,
    blob: String,
    signed_in: Arc<AtomicUsize>,
    sessions: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
}

impl Running {
    /// Drops every connection, the way a network failure does.
    fn cut(&self) {
        for session in self.sessions.lock().unwrap().drain(..) {
            session.abort();
        }
    }
}

fn start() -> Running {
    let key = PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&[7; 32])), "test").unwrap();
    let blob = key.public_key().to_openssh().unwrap().split_whitespace().nth(1).unwrap().to_string();
    let config = Arc::new(server::Config { keys: vec![key], auth_rejection_time: Duration::from_millis(10), ..Default::default() });
    let signed_in = Arc::new(AtomicUsize::new(0));
    let sessions = Arc::new(Mutex::new(Vec::new()));
    let handler = Server { signed_in: signed_in.clone(), cats: Default::default() };
    let kept = sessions.clone();

    let port = native::block(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let (config, handler) = (config.clone(), handler.clone());
                /* The server reads a pipe, and a pump joins the pipe to the TCP socket: aborting the pump
                   drops the socket with no goodbye, which is what a network failure looks like. */
                let (inner, mut outer) = tokio::io::duplex(256 * 1024);
                let mut socket = socket;
                let pump = tokio::spawn(async move {
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut outer).await;
                });

                kept.lock().unwrap().push(pump.abort_handle());
                tokio::spawn(async move {
                    if let Ok(session) = server::run_stream(config, inner, handler).await {
                        let _ = session.await;
                    }
                });
            }
        });

        port
    })
    .unwrap();

    Running { port, blob, signed_in, sessions }
}

fn connect(server: &Running, user: &str) -> Ssh {
    let ssh = Ssh::parse(&format!("ssh -p {} {user}@127.0.0.1", server.port)).unwrap();

    native::test_pins().lock().unwrap().insert(ssh.label(), vec![server.blob.clone()]);

    ssh
}

fn stay_signed_in() -> Credentials {
    Credentials { password: "pw".into(), totp: totp::decode_secret(&secret_text()) }
}

#[test]
fn it_signs_in_with_the_password_and_a_code_it_makes_itself_and_runs_commands() {
    let server = start();
    let ssh = connect(&server, "first");

    let used = native::sign_in(&ssh, stay_signed_in(), "").unwrap();

    assert!(used.password && used.code);
    assert!(native::is_live(&ssh));

    let output = ssh.run("uptime", Duration::from_secs(10)).unwrap();

    assert_eq!(output.stdout, "ran: uptime\n");
    assert_eq!(output.stderr, "a warning\n");
    assert_eq!(output.code, Some(3));

    let echoed = ssh.run_with_stdin("cat", "line one\nline two\n", Duration::from_secs(10)).unwrap();

    assert_eq!(echoed.stdout, "line one\nline two\n");
    assert_eq!(echoed.code, Some(0));

    native::close(&ssh);
    assert!(!native::manages(&ssh));
}

#[test]
fn a_wrong_code_is_refused_and_a_missing_one_is_asked_for() {
    let server = start();
    let ssh = connect(&server, "second");

    let wrong = native::sign_in(&ssh, Credentials { password: "pw".into(), totp: None }, "000000");

    assert!(matches!(wrong, Err(super::session::SignInError::Failed(ref sentence)) if sentence.contains("refused")), "{:?}", wrong.err().map(|e| format!("{e:?}")));

    let missing = native::sign_in(&ssh, Credentials { password: "pw".into(), totp: None }, "");

    assert!(matches!(missing, Err(super::session::SignInError::NeedsCode)));
    assert!(!native::manages(&ssh));
}

#[test]
fn a_dropped_connection_is_signed_in_again_by_itself_and_the_next_command_just_works() {
    let server = start();
    let ssh = connect(&server, "third");

    native::sign_in(&ssh, stay_signed_in(), "").unwrap();
    assert_eq!(server.signed_in.load(Ordering::SeqCst), 1);

    server.cut();

    /* The command is sent while the connection is down: it waits for the new sign-in and runs. */
    let output = ssh.run("whoami", Duration::from_secs(60)).unwrap();

    assert_eq!(output.stdout, "ran: whoami\n", "{output:?}");
    assert_eq!(server.signed_in.load(Ordering::SeqCst), 2, "signed in again, once");
    assert_eq!(native::phase(&ssh), Some(Phase::Up));

    native::close(&ssh);
}

#[test]
fn without_stay_signed_in_a_drop_needs_a_person() {
    let server = start();
    let ssh = connect(&server, "fourth");

    native::sign_in(&ssh, Credentials { password: "pw".into(), totp: None }, &totp::code_now(&totp::decode_secret(&secret_text()).unwrap())).unwrap();
    assert!(!native::stays_signed_in(&ssh));

    server.cut();

    for _ in 0..50 {
        if native::phase(&ssh) == Some(Phase::Down) {
            break;
        }

        std::thread::sleep(Duration::from_millis(200));
    }

    assert_eq!(native::phase(&ssh), Some(Phase::Down));

    let output = ssh.run("true", Duration::from_secs(5)).unwrap();

    assert_eq!(output.code, Some(255));
    assert!(output.stderr.contains("(keyboard-interactive)"), "the sentence every caller reads as 'sign in again': {}", output.stderr);
}

#[test]
fn many_commands_at_once_share_the_connection_without_being_refused() {
    let server = start();
    let ssh = connect(&server, "fifth");

    native::sign_in(&ssh, stay_signed_in(), "").unwrap();

    let runs: Vec<_> = (0..24)
        .map(|index| {
            let ssh = ssh.clone();

            std::thread::spawn(move || ssh.run(&format!("job {index}"), Duration::from_secs(30)).unwrap())
        })
        .collect();

    for (index, run) in runs.into_iter().enumerate() {
        assert_eq!(run.join().unwrap().stdout, format!("ran: job {index}\n"));
    }

    native::close(&ssh);
}

/// The channel limiter (0.16.0): a caller past [`CHANNELS`] concurrent commands is told the host is
/// busy instead of being queued forever, and the next command goes straight through once a slot frees.
#[test]
fn a_caller_past_channels_many_is_told_the_host_is_busy_until_one_frees() {
    let server = start();
    let ssh = connect(&server, "seventh");

    native::sign_in(&ssh, stay_signed_in(), "").unwrap();

    /* Hold every slot with a command that only finishes once the server says so. */
    let holding: Vec<_> = (0..CHANNELS)
        .map(|_| {
            let ssh = ssh.clone();

            std::thread::spawn(move || ssh.run("sleep 1", Duration::from_secs(10)).unwrap())
        })
        .collect();

    /* Give the holders a moment to actually open their channels before the next one is attempted. */
    std::thread::sleep(Duration::from_millis(200));

    let busy = ssh.run("true", Duration::from_millis(200)).unwrap();

    assert_eq!(busy.code, Some(255));
    assert!(busy.stderr.contains(&format!("{CHANNELS} commands are already running")), "{}", busy.stderr);

    for held in holding {
        let output = held.join().unwrap();

        assert_eq!(output.stdout, "slept\n");
        assert_eq!(output.code, Some(0));
    }

    /* Every slot freed: the next command goes through immediately. */
    let output = ssh.run("true", Duration::from_secs(5)).unwrap();

    assert_eq!(output.code, Some(3));

    native::close(&ssh);
}

#[test]
fn a_bridged_process_behaves_like_ssh() {
    let server = start();
    let ssh = connect(&server, "sixth");

    native::sign_in(&ssh, stay_signed_in(), "").unwrap();

    let (program, args) = ssh.launcher(None).unwrap();

    assert_eq!(args[0], super::bridge::FLAG, "{program} {args:?}");

    let run = |command: &str, input: &str| {
        let mut full = args[1..].to_vec();

        full.push(command.to_string());

        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = super::bridge::client_run(&full, std::io::Cursor::new(input.as_bytes().to_vec()), &mut out, &mut err);

        (code, String::from_utf8_lossy(&out).to_string(), String::from_utf8_lossy(&err).to_string())
    };

    assert_eq!(run("deploy", ""), (3, "ran: deploy\n".to_string(), "a warning\n".to_string()));
    assert_eq!(run("cat", "piped through\n").1, "piped through\n");
    assert_eq!(run("sleep 1", "").1, "slept\n");

    native::close(&ssh);

    let (code, _, err) = run("deploy", "");

    assert_eq!(code, 255);
    assert!(err.contains("(keyboard-interactive)"), "{err}");
}
