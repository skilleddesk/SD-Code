//! The heart of SDC Anywhere: every browser connection as a state machine.
//!
//! `Core` is synchronous and owns no socket and no clock. It is fed what arrived (a relay message, a
//! daemon event, a tick, the answer to a tunnelled call) and returns what to do ([`Out`]: send these
//! relay messages, close this connection, run this call). The async driver (`relay`) only moves bytes
//! and runs the calls. That split is what lets every rule here be tested without a network.
//!
//! ## What one connection goes through
//!
//! ```text
//!   open ──hello──► Locked ──capability.unlock (passkey)──► View ──(passkey)──► Operate (5 min)
//!    │                                                        │
//!    └──pair hello──► Pairing ──desktop confirms──► device saved, "pair.done", close
//! ```
//!
//! ## Ordering and priority
//!
//! Frames are sealed with a counter-numbered HPKE context, so they must leave in the order they are
//! sealed. Priority is therefore applied *before* sealing: each link has one queue per channel and
//! [`Core::flush`] seals from the highest channel down (`control` > `pty` > `stream` > `fs` >
//! `preview` > `xfer`). A big download queued on `xfer` can never delay an approval or a Kill.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use super::frame::{self, Outgoing, Reassembler};
use super::gateway::{self, PathsHandle};
use super::crypto::{self, accept, accept_pairing, b64u, from_b64u, random, sha256, Hello, RecvHalf, SendHalf, WelcomePlain};
use super::identity::Identity;
use super::registry::{Device, Registry};
use super::router::{edit_instruction, envelope_from_event, Accepted, Decision, OnTimeout, Proof, Refusal, Router, PAUSE_INSTRUCTION};
use super::session::{need_for, Capability, Level, Limits, Need};
use super::webauthn::{self, Assertion, RelyingParty};

/// Channels, highest priority first.
pub const CHANNELS: [&str; 6] = ["control", "pty", "stream", "fs", "preview", "xfer"];

/// How many events a `stream.subscribe` replays at most.
pub const REPLAY_LIMIT: usize = 500;

/// A pairing request that nobody confirmed on the desktop is dropped after this long.
pub const PAIR_WAIT_MS: i64 = 2 * 60 * 1000;

/// After a connection drops (a network blip, the relay restarting), the same device may reopen the session it
/// had, at the level it had, for this long. Past it, the passkey is asked for again. An explicit lock, a revoke
/// and a guest session never resume.
pub const RESUME_GRACE_MS: i64 = 2 * 60 * 1000;

/// The most connections the core tracks at once, waiting for a hello or open. The relay is not trusted to be polite: a
/// flood of `open` messages must not grow the daemon's memory.
pub const MAX_CONNECTIONS: usize = 128;

/// Failed unlocks and refused decisions a session may cause before it is closed. A passkey cannot be guessed, but a
/// session that keeps sending bad proofs is either broken or hostile, and each one costs a signature check.
pub const MAX_FAILURES: u8 = 5;

/// A pairing token (the QR code) is valid this long.
pub const PAIR_TOKEN_TTL_MS: i64 = 10 * 60 * 1000;

const LABEL_UNLOCK: &[u8] = b"sdc-anywhere/v1/unlock";

/// What the core needs from the daemon. A trait so tests do not need a database or an engine.
pub trait Backend: Send + Sync {
    /// Answers the gate. `by` names the device, for the audit trail. `true` when something was waiting.
    fn resolve_permission(&self, permission_id: &str, decision: &str, by: &DecisionBy) -> bool;
    /// Notifications after `seq`, oldest first.
    fn events_since(&self, seq: i64, limit: usize) -> Vec<StreamEvent>;
    fn current_seq(&self) -> i64;
    /// The kill switch. Returns the daemon's own answer for the card.
    fn kill_all(&self) -> Value;
    /// Records a decision that was refused or an unlock that failed, for the audit trail.
    fn note(&self, what: &str, detail: Value);
}

/// One event of the daemon's log, as a browser is shown it.
#[derive(Debug, Clone)]
pub struct StreamEvent {
    pub seq: i64,
    pub event: Value,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
}

fn stream_frame(event: &StreamEvent) -> Value {
    json!({ "type": "stream.event", "body": { "seq": event.seq, "event": event.event, "session_id": event.session_id, "turn_id": event.turn_id } })
}

/// Who answered a request, and what they did, for the ledger.
#[derive(Debug, Clone)]
pub struct DecisionBy {
    pub device_id: String,
    pub device_name: String,
    /// `allow_once` or `deny`: what the gate was told, in the words the event log uses.
    pub label: String,
    /// The person's own words and choices: `reason`, `edited_to`, `scoped_minutes`, `paused`.
    pub extra: Value,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub limits: Limits,
    /// The longest an "allow for a while" may last.
    pub max_scoped_grant_ms: i64,
    pub approval_timeout_ms: i64,
    pub on_timeout: OnTimeout,
    pub rp: RelyingParty,
    /// How long the relay waits after a push before it sends the email (`escalate_to_email_after_sec`).
    pub escalate_email_sec: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            max_scoped_grant_ms: 60 * 60_000,
            approval_timeout_ms: super::router::DEFAULT_TIMEOUT_MS,
            on_timeout: OnTimeout::Pause,
            rp: RelyingParty::production(),
            escalate_email_sec: DEFAULT_ESCALATE_EMAIL_SEC,
        }
    }
}

/// Push first; email this long after if nobody has answered (plan 5.10).
pub const DEFAULT_ESCALATE_EMAIL_SEC: i64 = 60;

/// A relay that asks for pairing offers faster than this is refused: a sign-in link is spent by a person, a few times at most.
const LINK_OFFER_GAP_MS: i64 = 10_000;

/// What the driver must do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// A relay message for one connection.
    Send { conn: String, msg: Value },
    /// Drop the connection.
    Close { conn: String },
    /// Run a call on the blocking pool and report back with `Core::rpc_done`. A gateway method (`gateway::is_gateway_method`)
    /// carries the connection's path table; `critical` says the core verified a fresh passkey for exactly this read.
    Rpc { conn: String, id: String, method: String, params: Value, paths: Option<PathsHandle>, critical: bool },
    /// A message for the relay itself (device registry, "something is waiting").
    Ctl(Value),
}

/// A request to pair, waiting for the person at the desktop.
#[derive(Debug, Clone)]
pub struct PairRequest {
    pub conn: String,
    pub device_id: String,
    pub name: String,
    pub user_agent: String,
    pub guest: bool,
    /// The six digits both screens show.
    pub sas: String,
    pub asked_at: i64,
}

/// What `begin_pairing` returns for the QR code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairOffer {
    /// Goes after the `#` in the pairing URL. Never sent to a server: browsers do not send fragments.
    pub fragment: String,
    pub fingerprint: String,
    pub expires_at: i64,
}

struct Queued {
    out: Outgoing,
}

/// Most frames sealed for one connection per call. What is left waits for the next call, so a higher-priority message
/// that arrives meanwhile goes first (at the next piece boundary).
pub const FLUSH_BUDGET: usize = 16;

/// How many bytes of non-control frames a connection may have sent that the browser has not yet acknowledged. Past it
/// the daemon waits: the relay cannot slow a fast sender down, so the receiver's own acknowledgements do.
pub const WINDOW_BYTES: usize = 1024 * 1024;

/// Calls one connection may have running at once. The rest wait their turn; Kill never waits.
pub const MAX_INFLIGHT: usize = 4;

/// Calls that may wait for a slot. Past it the browser is told the daemon is busy.
pub const MAX_WAITING: usize = 64;

struct WaitingCall {
    id: String,
    method: String,
    params: Value,
    critical: bool,
    gateway: bool,
}

struct Link {
    device: Device,
    send: SendHalf,
    recv: RecvHalf,
    cap: Capability,
    queues: [VecDeque<Queued>; 6],
    subscribed: bool,
    last_seq: i64,
    challenge: Option<Challenge>,
    /// The level last reported to the browser, so a change that happens by itself is reported once.
    announced: &'static str,
    /// Bad proofs so far (a failed unlock, a refused decision); at `MAX_FAILURES` the session is closed.
    failures: u8,
    /// The ids this connection has been given for files and folders (`gateway`).
    paths: PathsHandle,
    /// A challenge for a one-file passkey proof (a protected file), if one is outstanding.
    critical: Option<CriticalChallenge>,
    /// Puts fragments back together.
    rx: Reassembler,
    next_message: u32,
    inflight: usize,
    waiting: VecDeque<WaitingCall>,
    /// Frames sent and not yet acknowledged: `(frame number, bytes)`, oldest first.
    unacked: VecDeque<(u64, usize)>,
    unacked_bytes: usize,
    /// Which channel each running call's answer goes on: a file read's answer is bulk (`fs`), the rest is control.
    answer_on: HashMap<String, &'static str>,
}

struct CriticalChallenge {
    action: &'static str,
    host: String,
    path: String,
    nonce: [u8; 32],
    expires_at: i64,
}

struct Challenge {
    level: Level,
    nonce: [u8; 32],
    expires_at: i64,
}

struct PairLink {
    send: SendHalf,
    request: PairRequest,
    device: Device,
}

enum Conn {
    /// Opened by the relay, waiting for its hello.
    Waiting { pairing: bool, opened_at: i64 },
    Linked(Box<Link>),
    Pairing(Box<PairLink>),
}

/// What a dropped session left behind.
struct Resumable {
    cap: Capability,
    dropped_at: i64,
    paths: PathsHandle,
}

pub struct Core {
    resumable: HashMap<String, Resumable>,
    identity: Identity,
    registry: Arc<Registry>,
    backend: Arc<dyn Backend>,
    settings: Settings,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    conns: HashMap<String, Conn>,
    router: Router,
    id: String,
    /// When the last pairing offer for a sign-in link was made.
    last_link_offer: Option<i64>,
}

#[derive(Deserialize)]
struct WireAssertion {
    authenticator_data: String,
    client_data_json: String,
    signature: String,
}

impl WireAssertion {
    fn decode(&self) -> Option<Assertion> {
        Some(Assertion {
            authenticator_data: from_b64u(&self.authenticator_data).ok()?,
            client_data_json: from_b64u(&self.client_data_json).ok()?,
            signature: from_b64u(&self.signature).ok()?,
        })
    }
}

/// What the daemon signs to prove itself to the relay.
pub fn hub_auth_message(nonce: &str, daemon_id: &str) -> Vec<u8> {
    [b"sdc-anywhere/v1/hub-auth".as_slice(), b"|", nonce.as_bytes(), b"|", daemon_id.as_bytes()].concat()
}

fn ok_body(id: &str, body: Value) -> Value {
    json!({ "type": "res", "id": id, "ok": true, "body": body })
}

fn err_body(id: &str, code: &str, message: impl std::fmt::Display) -> Value {
    json!({ "type": "res", "id": id, "ok": false, "error": { "code": code, "message": message.to_string() } })
}

fn channel_index(ch: &str) -> usize {
    CHANNELS.iter().position(|known| *known == ch).unwrap_or(0)
}

impl Core {
    pub fn new(identity: Identity, registry: Arc<Registry>, backend: Arc<dyn Backend>, settings: Settings, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        let id = identity.id();
        let router = Router::new(settings.approval_timeout_ms);

        Self { resumable: HashMap::new(), identity, registry, backend, settings, clock, conns: HashMap::new(), router, id, last_link_offer: None }
    }

    pub fn daemon_id(&self) -> &str {
        &self.id
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// The daemon's identity public key (SEC1), for the relay handshake and the pairing QR code.
    pub fn identity_public(&self) -> Vec<u8> {
        self.identity.public()
    }

    /// Signs the relay's challenge, proving this daemon holds the key its id is the hash of.
    pub fn sign_hub_auth(&self, nonce: &str) -> Vec<u8> {
        self.identity.signer.sign(&hub_auth_message(nonce, &self.id)).to_vec()
    }

    /// Every active device, for re-teaching the relay after it restarted.
    pub fn devices_for_sync(&self) -> Vec<Value> {
        let now = self.now();

        self.registry
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|device| device.is_active(now))
            .map(|device| json!({ "id": device.id, "pub": b64u(&device.sign_pub), "guest": device.guest }))
            .collect()
    }

    /// The relay connection dropped: every browser connection went with it. Open requests stay, because
    /// the daemon, not the relay, is where they live.
    pub fn reset_connections(&mut self) {
        for (_, state) in std::mem::take(&mut self.conns) {
            if let Conn::Linked(link) = state {
                self.remember(link);
            }
        }
    }

    /// The path table of a linked connection (tests put places into it the way a listing would).
    #[cfg(test)]
    pub fn paths_for_test(&self, conn: &str) -> Option<PathsHandle> {
        match self.conns.get(conn) {
            Some(Conn::Linked(link)) => Some(link.paths.clone()),
            _ => None,
        }
    }

    /// How many time-limited allows are live (tests).
    #[cfg(test)]
    pub fn grants_for_test(&self) -> usize {
        self.router.grants(self.now()).len()
    }

    pub fn open_requests(&self) -> usize {
        self.router.open().len()
    }

    pub fn connections(&self) -> usize {
        self.conns.values().filter(|conn| matches!(conn, Conn::Linked(_))).count()
    }

    pub fn pair_requests(&self) -> Vec<PairRequest> {
        self.conns
            .values()
            .filter_map(|conn| match conn {
                Conn::Pairing(pairing) => Some(pairing.request.clone()),
                _ => None,
            })
            .collect()
    }

    /// The relay opened a connection. `pairing` is true for the unauthenticated pairing route.
    pub fn open(&mut self, conn: &str, pairing: bool) {
        let now = self.now();

        if self.conns.len() >= MAX_CONNECTIONS && !self.conns.contains_key(conn) {
            return;
        }

        self.conns.insert(conn.to_string(), Conn::Waiting { pairing, opened_at: now });
    }

    /// The relay closed a connection. A session that was open and unlocked can be picked up again for a short while.
    pub fn closed(&mut self, conn: &str) {
        if let Some(Conn::Linked(link)) = self.conns.remove(conn) {
            self.remember(link);
        }
    }

    fn remember(&mut self, link: Box<Link>) {
        let now = self.now();

        if !link.cap.is_guest() && link.cap.level(now) != Level::Locked {
            self.resumable.insert(link.device.id.clone(), Resumable { cap: link.cap, dropped_at: now, paths: link.paths });
        }
    }

    // --- pairing, from the desktop's side ----------------------------------------------------------

    /// Starts a pairing: a single-use token, valid ten minutes. The fragment goes into the QR code.
    pub fn begin_pairing(&mut self, guest: bool) -> anyhow::Result<PairOffer> {
        let now = self.now();
        let token = b64u(&random::<32>());

        self.registry.create_pairing(&token, now, PAIR_TOKEN_TTL_MS, guest)?;

        Ok(PairOffer {
            fragment: format!("v1.{}.{}.{}", token, b64u(&self.identity.public()), b64u(&self.identity.kem.public())),
            fingerprint: self.identity.fingerprint(),
            expires_at: now + PAIR_TOKEN_TTL_MS,
        })
    }

    /// A sign-in link was spent at the relay, and the relay asks for a pairing offer for the browser that holds it. The browser
    /// then pairs the normal way: it shows a six-digit code, and this computer (or a trusted device) must approve it. So the link
    /// by itself grants nothing; the offer is an ordinary, single-use, ten-minute pairing token, never a guest one.
    pub fn offer_for_link(&mut self, ticket: &str) -> Vec<Out> {
        let now = self.now();
        let refuse = |why: &str| vec![Out::Ctl(json!({ "ctl": "offer", "ticket": ticket, "error": why }))];

        if self.last_link_offer.is_some_and(|at| (0..LINK_OFFER_GAP_MS).contains(&(now - at))) {
            return refuse("busy");
        }

        self.last_link_offer = Some(now);

        match self.begin_pairing(false) {
            Ok(offer) => {
                self.backend.note("link.offer", json!({}));

                vec![Out::Ctl(json!({ "ctl": "offer", "ticket": ticket, "fragment": offer.fragment, "fingerprint": offer.fingerprint, "expires_at": offer.expires_at }))]
            }
            Err(_) => refuse("failed"),
        }
    }

    /// The person compared the codes and said yes (`accept`) or no.
    pub fn confirm_pairing(&mut self, device_id: &str, accept: bool) -> anyhow::Result<Vec<Out>> {
        let conn = self
            .conns
            .iter()
            .find_map(|(id, conn)| match conn {
                Conn::Pairing(pairing) if pairing.device.id == device_id => Some(id.clone()),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("no pairing request from device {device_id} is waiting"))?;
        let Some(Conn::Pairing(mut pairing)) = self.conns.remove(&conn) else { unreachable!("found above") };
        let mut out = Vec::new();

        if accept {
            let mut device = pairing.device.clone();

            device.created_at = self.now();
            self.registry.add_device(&device)?;
            out.push(Out::Ctl(json!({ "ctl": "device.add", "id": device.id, "pub": b64u(&device.sign_pub), "guest": device.guest })));
            self.send_pair(&mut pairing, &conn, json!({ "type": "pair.done", "device": device.id, "daemon": self.id, "guest": device.guest }), &mut out);
        } else {
            self.send_pair(&mut pairing, &conn, json!({ "type": "pair.rejected" }), &mut out);
        }

        out.push(Out::Close { conn });

        Ok(out)
    }

    fn send_pair(&self, pairing: &mut PairLink, conn: &str, value: Value, out: &mut Vec<Out>) {
        if let Ok((n, ct)) = pairing.send.seal(&frame::whole(&serde_json::to_vec(&value).expect("a value serialises"))) {
            out.push(Out::Send { conn: conn.to_string(), msg: json!({ "t": "f", "n": n, "ct": b64u(&ct) }) });
        }
    }

    /// Revokes a device: it can never open a session again, and any open one is closed now.
    pub fn revoke_device(&mut self, id: &str) -> anyhow::Result<Vec<Out>> {
        let now = self.now();
        let existed = self.registry.revoke(id, now)?;
        let mut out = Vec::new();

        if existed {
            let doomed: Vec<String> = self
                .conns
                .iter()
                .filter(|(_, state)| matches!(state, Conn::Linked(link) if link.device.id == id))
                .map(|(conn, _)| conn.clone())
                .collect();

            for conn in doomed {
                self.conns.remove(&conn);
                out.push(Out::Close { conn });
            }

            self.router.revoke_grants(id);
            out.push(Out::Ctl(json!({ "ctl": "device.remove", "id": id })));
            self.backend.note("device.revoked", json!({ "device": id }));
        }

        Ok(out)
    }

    // --- relay messages ------------------------------------------------------------------------------

    /// A message arrived on `conn` from the browser side.
    pub fn on_message(&mut self, conn: &str, msg: &Value) -> Vec<Out> {
        let kind = msg.get("t").and_then(Value::as_str).unwrap_or_default();
        let mut out = Vec::new();

        match (self.conns.get(conn), kind) {
            (Some(Conn::Waiting { pairing, .. }), "hello") => {
                let pairing = *pairing;

                self.on_hello(conn, pairing, msg, &mut out);
            }
            (Some(Conn::Linked(_)), "f") => self.on_frame(conn, msg, &mut out),
            (Some(Conn::Pairing(_)), "f") => {
                /* A pairing session has nothing to say to the daemon; a frame from it is dropped. */
            }
            _ => {
                self.conns.remove(conn);
                out.push(Out::Close { conn: conn.to_string() });
            }
        }

        self.flush_into(&mut out);

        out
    }

    fn drop_conn(&mut self, conn: &str, out: &mut Vec<Out>) {
        self.conns.remove(conn);
        out.push(Out::Close { conn: conn.to_string() });
    }

    fn on_hello(&mut self, conn: &str, pairing: bool, msg: &Value, out: &mut Vec<Out>) {
        let now = self.now();
        let Ok(hello) = serde_json::from_value::<Hello>(msg.clone()) else {
            return self.drop_conn(conn, out);
        };

        if pairing {
            return self.on_pair_hello(conn, &hello, now, out);
        }

        /* A registered, active device, or nothing. The same answer for "unknown" and "revoked": the
           relay (and anyone watching it) learns nothing about which devices exist. */
        let Some(device) = self.registry.device(crypto::hello_device(&hello)).ok().flatten().filter(|device| device.is_active(now)) else {
            self.backend.note("hello.refused", json!({ "device": hello.device, "why": "unknown or revoked" }));

            return self.drop_conn(conn, out);
        };
        let accepted = accept(&self.identity.kem, &self.id, &hello, &device.sign_pub, now);
        let (plain, pending) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                self.backend.note("hello.refused", json!({ "device": hello.device, "why": error.to_string() }));

                return self.drop_conn(conn, out);
            }
        };

        /* The replay check comes after the signature and the open: a stranger cannot burn nonces. */
        match self.registry.claim_nonce(&plain.nonce, &device.id, now) {
            Ok(true) => {}
            _ => {
                self.backend.note("hello.refused", json!({ "device": device.id, "why": "replayed hello" }));

                return self.drop_conn(conn, out);
            }
        }

        let guest = device.guest;
        /* A device that dropped a moment ago and is unlocked picks up where it was; otherwise it starts locked
           (a guest starts at View). The grace never extends a window: the Operate deadline is the old one. */
        let resumed = self.resumable.remove(&device.id).filter(|old| now - old.dropped_at <= RESUME_GRACE_MS && old.cap.level(now) != Level::Locked);
        let (cap, paths) = match resumed {
            Some(old) => (old.cap, old.paths),
            None => (Capability::new(now, guest, self.settings.limits), PathsHandle::default()),
        };
        let level = cap.level(now);
        let welcome_plain = WelcomePlain { device: device.id.clone(), ts: now, nonce: plain.nonce.clone(), level: level.name().into(), seq: self.backend.current_seq() };
        let Ok((welcome, send, recv)) = pending.welcome(&self.identity.signer, &welcome_plain) else {
            return self.drop_conn(conn, out);
        };
        let _ = self.registry.touch(&device.id, now);
        let mut link = Link {
            cap,
            device,
            send,
            recv,
            queues: Default::default(),
            subscribed: false,
            last_seq: plain.last_seq,
            challenge: None,
            announced: "locked",
            failures: 0,
            paths,
            critical: None,
            rx: Reassembler::default(),
            next_message: 0,
            inflight: 0,
            waiting: VecDeque::new(),
            unacked: VecDeque::new(),
            unacked_bytes: 0,
            answer_on: HashMap::new(),
        };

        out.push(Out::Send { conn: conn.to_string(), msg: json!({ "t": "welcome", "enc": welcome.enc, "ct": welcome.ct, "sig": welcome.sig }) });
        self.announce_state(&mut link, now);
        self.show_open_requests(&mut link, now);
        self.conns.insert(conn.to_string(), Conn::Linked(Box::new(link)));
    }

    fn on_pair_hello(&mut self, conn: &str, hello: &Hello, now: i64, out: &mut Vec<Out>) {
        let (plain, pair, pending) = match accept_pairing(&self.identity.kem, &self.id, hello, now) {
            Ok(opened) => opened,
            Err(error) => {
                self.backend.note("pair.refused", json!({ "why": error.to_string() }));

                return self.drop_conn(conn, out);
            }
        };
        let (Ok(sign_pub), Ok(passkey_pub)) = (from_b64u(&pair.sign_pub), from_b64u(&pair.passkey_pub)) else {
            return self.drop_conn(conn, out);
        };

        if !matches!(self.registry.claim_nonce(&plain.nonce, &hello.device, now), Ok(true)) {
            return self.drop_conn(conn, out);
        }

        /* The token is spent only now, after the signature proved the sender holds the key and the nonce
           was fresh: a stranger who guesses nothing cannot use up someone's QR code. */
        let Ok(Some(guest)) = self.registry.consume_pairing(&pair.token, now) else {
            self.backend.note("pair.refused", json!({ "why": "token unknown, used or expired" }));

            return self.drop_conn(conn, out);
        };
        let mut counter = 0;

        if !guest {
            let assertion = pair.assertion.as_ref().and_then(|wire| {
                Some(Assertion {
                    authenticator_data: from_b64u(&wire.authenticator_data).ok()?,
                    client_data_json: from_b64u(&wire.client_data_json).ok()?,
                    signature: from_b64u(&wire.signature).ok()?,
                })
            });
            let verified = assertion.and_then(|assertion| {
                webauthn::verify(&self.settings.rp, &passkey_pub, 0, &crypto::pair_challenge(&plain.nonce), &assertion).ok()
            });

            match verified {
                Some(new_counter) => counter = new_counter,
                None => {
                    self.backend.note("pair.refused", json!({ "why": "the passkey did not prove itself" }));

                    return self.drop_conn(conn, out);
                }
            }
        }

        let welcome_plain = WelcomePlain { device: hello.device.clone(), ts: now, nonce: plain.nonce.clone(), level: "pairing".into(), seq: 0 };
        let Ok((welcome, send, _recv)) = pending.welcome(&self.identity.signer, &welcome_plain) else {
            return self.drop_conn(conn, out);
        };
        let device = Device {
            id: hello.device.clone(),
            name: pair.name.chars().take(60).collect(),
            user_agent: pair.user_agent.chars().take(200).collect(),
            sign_pub: sign_pub.clone(),
            passkey_id: pair.passkey_id.clone(),
            passkey_pub,
            passkey_counter: counter,
            guest,
            created_at: now,
            last_seen: None,
            revoked_at: None,
            expires_at: guest.then(|| now + self.settings.limits.clamp().guest_max_ms),
        };
        let request = PairRequest {
            conn: conn.to_string(),
            device_id: device.id.clone(),
            name: device.name.clone(),
            user_agent: device.user_agent.clone(),
            guest,
            sas: crypto::sas_code(&pair.token, &sign_pub, &self.identity.public()),
            asked_at: now,
        };

        out.push(Out::Send { conn: conn.to_string(), msg: json!({ "t": "welcome", "enc": welcome.enc, "ct": welcome.ct, "sig": welcome.sig }) });
        self.conns.insert(conn.to_string(), Conn::Pairing(Box::new(PairLink { send, request, device })));
    }

    fn on_frame(&mut self, conn: &str, msg: &Value, out: &mut Vec<Out>) {
        let now = self.now();
        let Some(Conn::Linked(mut link)) = self.conns.remove(conn) else { return };
        let number = msg.get("n").and_then(Value::as_u64);
        let ct = msg.get("ct").and_then(Value::as_str).and_then(|text| from_b64u(text).ok());
        let opened = match (number, ct) {
            (Some(number), Some(ct)) => link.recv.open(number, &ct),
            _ => Err(crypto::CryptoError::Malformed("frame")),
        };
        let Ok(plain) = opened else {
            /* A frame that does not open means a drop, a replay or an alteration: the session is over. */
            self.backend.note("frame.refused", json!({ "device": link.device.id }));
            out.push(Out::Close { conn: conn.to_string() });

            return;
        };
        /* A piece of a message, or a whole one: put back together before anything reads it. */
        let message = match link.rx.push(&plain) {
            Ok(Some(message)) => message,
            Ok(None) => {
                self.conns.insert(conn.to_string(), Conn::Linked(link));

                return;
            }
            Err(_) => {
                self.backend.note("frame.refused", json!({ "device": link.device.id, "why": "bad fragment" }));
                out.push(Out::Close { conn: conn.to_string() });

                return;
            }
        };
        let Ok(frame) = serde_json::from_slice::<Value>(&message) else {
            out.push(Out::Close { conn: conn.to_string() });

            return;
        };

        /* A revoked device is cut off at its next frame even if the revoke came from elsewhere. */
        let still_active = self.registry.device(&link.device.id).ok().flatten().is_some_and(|device| device.is_active(now));

        if !still_active || link.cap.is_over(now) {
            out.push(Out::Close { conn: conn.to_string() });

            return;
        }

        let kind = frame.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
        let id = frame.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let body = frame.get("body").cloned().unwrap_or(Value::Null);

        if kind != "ping" && kind != "ack" {
            link.cap.touch(now);
        }

        self.handle(conn, &mut link, &kind, &id, &body, now, out);

        if link.failures >= MAX_FAILURES {
            self.backend.note("session.closed", json!({ "device": link.device.id, "why": "too many bad proofs" }));
            out.push(Out::Close { conn: conn.to_string() });

            return;
        }

        self.conns.insert(conn.to_string(), Conn::Linked(link));
    }

    #[allow(clippy::too_many_arguments)]
    fn handle(&mut self, conn: &str, link: &mut Link, kind: &str, id: &str, body: &Value, now: i64, out: &mut Vec<Out>) {
        let level = link.cap.level(now);

        match kind {
            "ping" => self.reply(link, id, ok_body(id, json!({ "now": now }))),
            /* The browser has opened every frame before number `n`: they no longer count against the window. */
            "ack" => {
                let upto = body.get("n").and_then(Value::as_u64).unwrap_or(0);

                while let Some(&(number, bytes)) = link.unacked.front() {
                    if number >= upto {
                        break;
                    }

                    link.unacked.pop_front();
                    link.unacked_bytes = link.unacked_bytes.saturating_sub(bytes);
                }
            }
            "capability.challenge" => {
                let wanted = body.get("level").and_then(Value::as_str).and_then(Level::parse);

                match wanted {
                    Some(Level::View) | Some(Level::Operate) if !(link.cap.is_guest() && wanted == Some(Level::Operate)) => {
                        let nonce = random::<32>();
                        let expires_at = now + self.settings.limits.clamp().challenge_ms;
                        let level = wanted.expect("matched");

                        link.challenge = Some(Challenge { level, nonce, expires_at });
                        self.reply(link, id, ok_body(id, json!({ "level": level.name(), "challenge": b64u(&unlock_challenge(level, &nonce)), "rp_id": self.settings.rp.id, "expires_at": expires_at })));
                    }
                    _ => self.reply(link, id, err_body(id, "forbidden", "that level cannot be requested")),
                }
            }
            "capability.unlock" => self.on_unlock(link, id, body, now),
            "capability.lock" => {
                link.cap.lock();
                self.announce_state(link, now);
                self.reply(link, id, ok_body(id, json!({})));
            }
            "approval.decision" => self.on_decision(conn, link, id, body, level, now, out),
            "control.kill" => {
                /* No step-up, no unlock: stopping is always easy (plan 5.8). It is still logged. */
                self.backend.note("kill", json!({ "device": link.device.id }));

                let answer = self.backend.kill_all();

                self.reply(link, id, ok_body(id, answer));
            }
            "stream.subscribe" => {
                if level < Level::View {
                    return self.reply(link, id, err_body(id, "locked", "unlock the session first"));
                }

                let since = body.get("last_seq").and_then(Value::as_i64).unwrap_or(link.last_seq);

                link.subscribed = true;

                for item in self.backend.events_since(since, REPLAY_LIMIT) {
                    if streamable(&item.event) {
                        enqueue(link, "stream", stream_frame(&item));
                        link.last_seq = link.last_seq.max(item.seq);
                    }
                }

                self.reply(link, id, ok_body(id, json!({ "seq": self.backend.current_seq() })));
            }
            "rpc" => {
                let method = body.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
                let params = body.get("params").cloned().unwrap_or_else(|| json!({}));

                match need_for(&method) {
                    Need::Never => {
                        self.backend.note("rpc.refused", json!({ "device": link.device.id, "method": method, "why": "not available remotely" }));
                        self.reply(link, id, err_body(id, "forbidden", format!("{method} is not available from a browser")));
                    }
                    Need::Level(needed) if level < needed => {
                        let (code, text) = if level == Level::Locked { ("locked", "unlock the session first") } else { ("needs_operate", "open the Operate window with your passkey") };

                        self.reply(link, id, err_body(id, code, text));
                    }
                    Need::Level(_) if gateway::is_gateway_method(&method) => self.gateway_call(conn, link, id, method, params, now, out),
                    Need::Level(_) => self.start_call(conn, link, id, method, params, false, false, out),
                }
            }
            _ => self.reply(link, id, err_body(id, "unknown", format!("unknown message {kind}"))),
        }
    }

    /// A call for the file gateway. The browser's ids are looked up here only to ask one question: is this a protected
    /// file? If so, the read waits for a passkey assertion over *that file* (and no other), which is spent once.
    #[allow(clippy::too_many_arguments)]
    fn gateway_call(&mut self, conn: &str, link: &mut Link, id: &str, method: String, mut params: Value, now: i64, out: &mut Vec<Out>) {
        let mut critical = false;

        /* Whatever the browser said about `critical`, only this function decides it. */
        if let Some(map) = params.as_object_mut() {
            map.remove("critical");
        }

        let protected = (method == "fs.read")
            .then(|| params.get("path_id").and_then(Value::as_str).and_then(|path_id| gateway::protection_of(&link.paths, path_id)))
            .flatten();

        if let Some((host, path, _pattern)) = protected {
            let proof = params.get("assertion").and_then(|value| serde_json::from_value::<WireAssertion>(value.clone()).ok()).and_then(|wire| wire.decode());

            match (proof, link.critical.take()) {
                (Some(assertion), Some(challenge)) if challenge.host == host && challenge.path == path && challenge.action == "fs.read" && now < challenge.expires_at => {
                    let device = self.registry.device(&link.device.id).ok().flatten().unwrap_or_else(|| link.device.clone());

                    match webauthn::verify(&self.settings.rp, &device.passkey_pub, device.passkey_counter, &critical_challenge(challenge.action, &host, &path, &challenge.nonce), &assertion) {
                        Ok(counter) => {
                            let _ = self.registry.set_passkey_counter(&device.id, counter);

                            link.device.passkey_counter = counter;
                            critical = true;
                            self.backend.note("protected.read", json!({ "device": device.id, "host": host, "path": path }));
                        }
                        Err(error) => {
                            link.failures = link.failures.saturating_add(1);
                            self.backend.note("protected.refused", json!({ "device": device.id, "path": path, "why": error.to_string() }));

                            return self.reply(link, id, err_body(id, "refused", error));
                        }
                    }
                }
                _ => {
                    /* No proof yet, or one for something else: ask for a fresh one for this file. */
                    let nonce = random::<32>();
                    let expires_at = now + self.settings.limits.clamp().challenge_ms;
                    let digest = critical_challenge("fs.read", &host, &path, &nonce);

                    link.critical = Some(CriticalChallenge { action: "fs.read", host, path, nonce, expires_at });

                    return self.reply(
                        link,
                        id,
                        json!({ "type": "res", "ok": false, "error": { "code": "needs_critical", "message": "this file is protected; confirm with your passkey", "challenge": b64u(&digest), "rp_id": self.settings.rp.id, "expires_at": expires_at } }),
                    );
                }
            }
        }

        self.start_call(conn, link, id, method, params, true, critical, out);
    }

    /// Runs a call now if the connection has a free slot, else keeps it in line. Kill never waits.
    #[allow(clippy::too_many_arguments)]
    fn start_call(&mut self, conn: &str, link: &mut Link, id: &str, method: String, params: Value, gateway: bool, critical: bool, out: &mut Vec<Out>) {
        let urgent = method == "kill.all" || method == "kill.list";

        if !urgent && link.inflight >= MAX_INFLIGHT {
            if link.waiting.len() >= MAX_WAITING {
                return self.reply(link, id, err_body(id, "busy", "too many requests at once; try again in a moment"));
            }

            link.waiting.push_back(WaitingCall { id: id.to_string(), method, params, critical, gateway });

            return;
        }

        /* Counted either way, so its completion frees a slot like any other; it just did not have to wait for one. */
        link.inflight += 1;
        link.answer_on.insert(id.to_string(), if gateway && method.starts_with("fs.") { "fs" } else { "control" });
        out.push(Self::call_out(conn, link, WaitingCall { id: id.to_string(), method, params, critical, gateway }));
    }

    fn call_out(conn: &str, link: &Link, call: WaitingCall) -> Out {
        Out::Rpc { conn: conn.to_string(), id: call.id, method: call.method, params: call.params, paths: call.gateway.then(|| link.paths.clone()), critical: call.critical }
    }

    fn on_unlock(&mut self, link: &mut Link, id: &str, body: &Value, now: i64) {
        let Some(challenge) = link.challenge.take() else {
            return self.reply(link, id, err_body(id, "no_challenge", "ask for a challenge first"));
        };

        if now >= challenge.expires_at {
            return self.reply(link, id, err_body(id, "expired", "the challenge expired; ask again"));
        }

        let assertion = body.get("assertion").and_then(|value| serde_json::from_value::<WireAssertion>(value.clone()).ok()).and_then(|wire| wire.decode());
        let Some(assertion) = assertion else {
            return self.reply(link, id, err_body(id, "bad_request", "no assertion"));
        };
        let device = self.registry.device(&link.device.id).ok().flatten().unwrap_or_else(|| link.device.clone());
        let verified = webauthn::verify(&self.settings.rp, &device.passkey_pub, device.passkey_counter, &unlock_challenge(challenge.level, &challenge.nonce), &assertion);

        match verified {
            Ok(counter) => {
                let _ = self.registry.set_passkey_counter(&device.id, counter);

                link.device.passkey_counter = counter;

                match challenge.level {
                    Level::Operate => {
                        let _ = link.cap.unlock_operate(now);
                    }
                    _ => link.cap.unlock_view(now),
                }

                self.backend.note("unlock", json!({ "device": device.id, "level": challenge.level.name() }));
                self.announce_state(link, now);
                self.show_open_requests(link, now);
                self.reply(link, id, ok_body(id, json!({ "level": link.cap.level(now).name() })));
            }
            Err(error) => {
                link.failures = link.failures.saturating_add(1);
                self.backend.note("unlock.refused", json!({ "device": device.id, "why": error.to_string() }));
                self.reply(link, id, err_body(id, "refused", error));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_decision(&mut self, conn: &str, link: &mut Link, id: &str, body: &Value, level: Level, now: i64, out: &mut Vec<Out>) {
        let text = |key: &str| body.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let request_id = text("request_id");
        let proof = Proof {
            device_signature: from_b64u(&text("device_sig")).unwrap_or_default(),
            assertion: body.get("assertion").and_then(|value| serde_json::from_value::<WireAssertion>(value.clone()).ok()).and_then(|wire| wire.decode()),
        };
        let device = self.registry.device(&link.device.id).ok().flatten().unwrap_or_else(|| link.device.clone());
        let result = self.router.check(&request_id, &text("decision"), &text("action_hash"), &proof, &device, level, &self.settings.rp, now);

        match result {
            Ok(accepted) => {
                self.apply(link, &accepted);
                self.reply(link, id, ok_body(id, json!({ "request_id": accepted.request_id, "decision": accepted.decision.wire() })));
                self.announce_resolved(&accepted, &link.device.name, conn, now, out);
            }
            Err(refusal) => {
                /* A wrong or missing proof counts against the session; "too late" and "already answered" are not misbehaviour. */
                if matches!(refusal, Refusal::BadDeviceSignature | Refusal::HashMismatch | Refusal::Passkey(_) | Refusal::BadDecision) {
                    link.failures = link.failures.saturating_add(1);
                }

                self.backend.note("decision.refused", json!({ "device": link.device.id, "request": request_id, "why": refusal.to_string() }));

                let code = match refusal {
                    Refusal::Locked => "locked",
                    Refusal::NeedsOperate => "needs_operate",
                    Refusal::NeedsFreshPasskey => "needs_passkey",
                    Refusal::Expired => "expired",
                    Refusal::AlreadyResolved => "already_resolved",
                    _ => "refused",
                };

                self.reply(link, id, err_body(id, code, refusal));
            }
        }
    }

    /// Hands an accepted decision to the gate and records it.
    fn apply(&mut self, link: &mut Link, accepted: &Accepted) {
        if let Some(counter) = accepted.new_passkey_counter {
            let _ = self.registry.set_passkey_counter(&link.device.id, counter);

            link.device.passkey_counter = counter;
        }

        let now = self.now();
        let (gate, label, extra) = match (accepted.decision, &accepted.reason) {
            (Decision::AllowOnce, _) => ("allow_once".to_string(), "allow_once", json!({})),
            (Decision::AllowScoped(minutes), _) => ("allow_once".to_string(), "allow_once", json!({ "scoped_minutes": minutes })),
            (Decision::Deny, Some(reason)) => (format!("deny:{reason}"), "deny", json!({ "reason": reason })),
            (Decision::Deny, None) => ("deny".to_string(), "deny", json!({})),
            (Decision::DenyPause, _) => (format!("deny:{PAUSE_INSTRUCTION}"), "deny", json!({ "paused": true })),
            (Decision::Edit, command) => {
                let command = command.clone().unwrap_or_default();

                (format!("deny:{}", edit_instruction(&command)), "deny", json!({ "edited_to": command }))
            }
        };
        let by = DecisionBy { device_id: link.device.id.clone(), device_name: link.device.name.clone(), label: label.to_string(), extra };

        if let Decision::AllowScoped(minutes) = accepted.decision {
            if let Some(pending) = self.router.get(&accepted.request_id) {
                let ms = (i64::from(minutes) * 60_000).min(self.settings.max_scoped_grant_ms);
                let envelope = pending.envelope.clone();

                self.router.grant(&link.device.id, &envelope, now + ms);
                self.backend.note("scoped.granted", json!({ "device": link.device.id, "pattern": super::router::pattern_of(&envelope), "host": envelope.host, "cwd": envelope.cwd, "minutes": ms / 60_000 }));
            }
        }

        self.backend.resolve_permission(&accepted.permission_id, &gate, &by);
        self.router.close(&accepted.request_id);
    }

    /// Tells every other session (and the relay) that a request is closed.
    fn announce_resolved(&mut self, accepted: &Accepted, by: &str, except: &str, now: i64, out: &mut Vec<Out>) {
        let message = json!({ "type": "approval.resolved", "body": { "request_id": accepted.request_id, "decision": accepted.decision.wire(), "by": by } });

        for (conn, state) in self.conns.iter_mut() {
            if let Conn::Linked(other) = state {
                if conn != except && other.cap.level(now) >= Level::View {
                    enqueue(other, "control", message.clone());
                }
            }
        }

        out.push(Out::Ctl(json!({ "ctl": "clear", "request": accepted.request_id })));
    }

    // --- events from the daemon ------------------------------------------------------------------------

    /// A notification the daemon broadcast (the JSON line SDCP clients get).
    pub fn on_event(&mut self, line: &str) -> Vec<Out> {
        let Ok(notification) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        let event = notification.get("event").cloned().unwrap_or(Value::Null);
        let seq = notification.get("seq").and_then(Value::as_i64).unwrap_or(0);
        let session_id = notification.get("sessionId").and_then(Value::as_str).map(str::to_string);
        let turn_id = notification.get("turnId").and_then(Value::as_str).map(str::to_string);
        let now = self.now();
        let mut out = Vec::new();

        match event.get("type").and_then(Value::as_str) {
            Some("PermissionRequested") => {
                if let Some(envelope) = envelope_from_event(&event, now, self.settings.approval_timeout_ms) {
                    let permission_id = event.get("permissionId").and_then(Value::as_str).unwrap_or_default().to_string();

                    /* "Allow for a while": a live grant for this kind of action answers it without waking anyone. */
                    if let Some(grant) = self.router.covering(&envelope, now).cloned() {
                        let by = DecisionBy {
                            device_id: grant.device.clone(),
                            device_name: "a time-limited allow".into(),
                            label: "allow_once".into(),
                            extra: json!({ "scoped_use": true, "granted_by": grant.device, "pattern": grant.pattern }),
                        };

                        self.backend.note("scoped.used", json!({ "device": grant.device, "pattern": grant.pattern, "target": envelope.target }));
                        self.backend.resolve_permission(&permission_id, "allow_once", &by);

                        return Vec::new();
                    }

                    if let Some(pending) = self.router.add(&permission_id, envelope) {
                        let request = pending.envelope.request_id.clone();
                        let expires = pending.envelope.expires_at;

                        for state in self.conns.values_mut() {
                            if let Conn::Linked(link) = state {
                                show_request(link, &pending, now);
                            }
                        }

                        out.push(Out::Ctl(json!({ "ctl": "notify", "request": request, "expires_at": expires, "escalate_after_sec": self.settings.escalate_email_sec })));
                    }
                }
            }
            Some("PermissionResolved") => {
                /* Answered on the desktop (or by us a moment ago, in which case it is already closed). */
                let permission_id = event.get("permissionId").and_then(Value::as_str).unwrap_or_default();

                if let Some(request) = self.router.resolved_elsewhere(permission_id) {
                    let message = json!({ "type": "approval.resolved", "body": { "request_id": request, "decision": event.get("decision").cloned().unwrap_or(Value::Null), "by": "desktop" } });

                    for state in self.conns.values_mut() {
                        if let Conn::Linked(link) = state {
                            if link.cap.level(now) >= Level::View {
                                enqueue(link, "control", message.clone());
                            }
                        }
                    }

                    out.push(Out::Ctl(json!({ "ctl": "clear", "request": request })));
                }
            }
            _ => {}
        }

        if streamable(&event) {
            for state in self.conns.values_mut() {
                if let Conn::Linked(link) = state {
                    if link.subscribed && link.cap.level(now) >= Level::View {
                        enqueue(link, "stream", stream_frame(&StreamEvent { seq, event: event.clone(), session_id: session_id.clone(), turn_id: turn_id.clone() }));
                        link.last_seq = link.last_seq.max(seq);
                    }
                }
            }
        }

        self.flush_into(&mut out);

        out
    }

    /// A tunnelled call finished.
    pub fn rpc_done(&mut self, conn: &str, id: &str, result: Result<Value, (String, String)>) -> Vec<Out> {
        let mut out = Vec::new();

        if let Some(Conn::Linked(mut link)) = self.conns.remove(conn) {
            let body = match result {
                Ok(value) => ok_body(id, value),
                Err((code, message)) => err_body(id, &code, message),
            };
            link.inflight = link.inflight.saturating_sub(1);

            let channel = link.answer_on.remove(id).unwrap_or("control");

            enqueue(&mut link, channel, with_id(body, id));

            /* A slot is free: the next call in line starts. */
            while link.inflight < MAX_INFLIGHT {
                let Some(next) = link.waiting.pop_front() else { break };

                link.inflight += 1;
                link.answer_on.insert(next.id.clone(), if next.gateway && next.method.starts_with("fs.") { "fs" } else { "control" });
                out.push(Self::call_out(conn, &link, next));
            }

            self.conns.insert(conn.to_string(), Conn::Linked(link));
        }

        self.flush_into(&mut out);

        out
    }

    /// Time passed: expire requests, end guest sessions, drop stale pairing attempts, lock idle views.
    pub fn tick(&mut self) -> Vec<Out> {
        let now = self.now();
        let mut out = Vec::new();

        for pending in self.router.expire(now) {
            let request = pending.envelope.request_id.clone();

            if self.settings.on_timeout == OnTimeout::Deny {
                let by = DecisionBy { device_id: "timeout".into(), device_name: "timeout".into(), label: "deny".into(), extra: json!({ "timed_out": true }) };

                self.backend.resolve_permission(&pending.permission_id, "deny", &by);
            }

            for state in self.conns.values_mut() {
                if let Conn::Linked(link) = state {
                    if link.cap.level(now) >= Level::View {
                        enqueue(link, "control", json!({ "type": "approval.expired", "body": { "request_id": request } }));
                    }
                }
            }

            out.push(Out::Ctl(json!({ "ctl": "clear", "request": request })));
        }

        let mut dead = Vec::new();

        for (conn, state) in self.conns.iter_mut() {
            match state {
                Conn::Linked(link) => {
                    if link.cap.is_over(now) {
                        dead.push(conn.clone());
                    }
                }
                Conn::Pairing(pairing) if now - pairing.request.asked_at > PAIR_WAIT_MS => dead.push(conn.clone()),
                Conn::Waiting { opened_at, .. } if now - *opened_at > 30_000 => dead.push(conn.clone()),
                _ => {}
            }
        }

        for conn in dead {
            self.conns.remove(&conn);
            out.push(Out::Close { conn });
        }

        let ids: Vec<String> = self.conns.keys().cloned().collect();

        for conn in ids {
            if let Some(Conn::Linked(link)) = self.conns.get_mut(&conn) {
                let level = link.cap.level(now).name();

                if link.announced != level {
                    link.announced = level;
                    enqueue(link, "control", json!({ "type": "capability.state", "body": { "level": level, "operate_until": link.cap.operate_until(now) } }));
                }
            }
        }

        self.flush_into(&mut out);

        out
    }

    // --- helpers -------------------------------------------------------------------------------------

    fn reply(&self, link: &mut Link, id: &str, body: Value) {
        enqueue(link, "control", with_id(body, id));
    }

    fn announce_state(&self, link: &mut Link, now: i64) {
        let level = link.cap.level(now);

        link.announced = level.name();
        enqueue(link, "control", json!({ "type": "capability.state", "body": { "level": level.name(), "operate_until": link.cap.operate_until(now), "guest": link.cap.is_guest() } }));
    }

    /// What a session is shown of the open requests: the cards when it may view, a bare count when not.
    fn show_open_requests(&self, link: &mut Link, now: i64) {
        if link.cap.level(now) >= Level::View {
            for pending in self.router.open() {
                show_request(link, pending, now);
            }
        } else {
            enqueue(link, "control", json!({ "type": "approval.pending", "body": self.router.blind_summary() }));
        }
    }

    /// Seals everything queued, highest channel first, and appends the relay messages.
    fn flush_into(&mut self, out: &mut Vec<Out>) {
        for (conn, state) in self.conns.iter_mut() {
            if let Conn::Linked(link) = state {
                flush_link(conn, link, out);
            }
        }
    }

    /// Whether any connection still has frames waiting to be sealed (the flush budget stopped short of them).
    pub fn has_backlog(&self) -> bool {
        self.conns.values().any(|conn| matches!(conn, Conn::Linked(link) if can_send(link)))
    }

    /// Seals the next round of waiting frames. The driver calls it again while [`Core::has_backlog`] is true and the
    /// relay link has room.
    pub fn pump(&mut self) -> Vec<Out> {
        let mut out = Vec::new();

        self.flush_into(&mut out);

        out
    }

    /// Sealing and queue length of one connection, for the driver's back-pressure and for tests.
    pub fn queued(&self, conn: &str) -> usize {
        match self.conns.get(conn) {
            Some(Conn::Linked(link)) => link.queues.iter().map(VecDeque::len).sum(),
            _ => 0,
        }
    }

    /// Queues a message on a channel of one connection (used by later phases: fs, xfer, pty).
    pub fn push(&mut self, conn: &str, ch: &'static str, value: Value) -> Vec<Out> {
        self.push_many(conn, vec![(ch, value)])
    }

    /// Queues several messages and seals them in one flush, so priority decides their order.
    pub fn push_many(&mut self, conn: &str, items: Vec<(&'static str, Value)>) -> Vec<Out> {
        let mut out = Vec::new();

        if let Some(Conn::Linked(link)) = self.conns.get_mut(conn) {
            for (ch, value) in items {
                enqueue(link, ch, value);
            }
        }

        self.flush_into(&mut out);

        out
    }
}

fn with_id(mut body: Value, id: &str) -> Value {
    if let Some(map) = body.as_object_mut() {
        map.insert("id".into(), json!(id));
    }

    body
}

/// What a passkey signs to open one protected thing: bound to the action, the exact file on its host, and a nonce.
fn critical_challenge(action: &str, host: &str, path: &str, nonce: &[u8; 32]) -> [u8; 32] {
    sha256(&[b"sdc-anywhere/v1/critical", action.as_bytes(), b"|", host.as_bytes(), b"|", path.as_bytes(), b"|", nonce])
}

/// The bytes a passkey signs to open a level: bound to the level and to the daemon's nonce.
pub(crate) fn unlock_challenge(level: Level, nonce: &[u8; 32]) -> [u8; 32] {
    sha256(&[LABEL_UNLOCK, level.name().as_bytes(), nonce])
}

/// Events worth sending to a browser. The permission events travel as approval messages instead.
fn streamable(event: &Value) -> bool {
    !matches!(event.get("type").and_then(Value::as_str), Some("PermissionRequested" | "PermissionResolved"))
}

fn enqueue(link: &mut Link, ch: &'static str, mut value: Value) {
    if let Some(map) = value.as_object_mut() {
        map.insert("ch".into(), json!(ch));
    }

    let Ok(bytes) = serde_json::to_vec(&value) else { return };
    let id = link.next_message;

    link.next_message = link.next_message.wrapping_add(1);
    link.queues[channel_index(ch)].push_back(Queued { out: Outgoing::new(bytes, id) });
}

fn show_request(link: &mut Link, pending: &super::router::Pending, now: i64) {
    if link.cap.level(now) >= Level::View {
        enqueue(link, "control", json!({ "type": "approval.requested", "body": { "envelope": pending.envelope.to_value(), "action_hash": b64u(&pending.hash) } }));
    } else {
        enqueue(link, "control", json!({ "type": "approval.pending", "body": { "count": 1 } }));
    }
}

/// Whether a frame could be sealed now: control frames always can; the rest only while the window is open.
fn can_send(link: &Link) -> bool {
    link.queues.iter().enumerate().find(|(_, queue)| !queue.is_empty()).is_some_and(|(index, _)| index == 0 || link.unacked_bytes < WINDOW_BYTES)
}

fn flush_link(conn: &str, link: &mut Link, out: &mut Vec<Out>) {
    for _ in 0..FLUSH_BUDGET {
        /* The highest-priority channel that has anything, every time: a Kill that arrives while a big reply is
           being sent goes out at the next piece, not after the whole reply. */
        let Some((index, queue)) = link.queues.iter_mut().enumerate().find(|(_, queue)| !queue.is_empty()) else { return };

        /* Bulk waits for the browser to catch up; a control message never does. */
        if index > 0 && link.unacked_bytes >= WINDOW_BYTES {
            return;
        }

        let Some(front) = queue.front_mut() else { return };
        let (plain, done) = front.out.next_piece();

        if done {
            queue.pop_front();
        }

        match link.send.seal(&plain) {
            Ok((n, ct)) => {
                if index > 0 {
                    link.unacked.push_back((n, plain.len()));
                    link.unacked_bytes += plain.len();
                }

                out.push(Out::Send { conn: conn.to_string(), msg: json!({ "t": "f", "n": n, "ct": b64u(&ct) }) });
            }
            Err(_) => {
                /* The send half is spent: the session must be rekeyed, which means a new hello. */
                out.push(Out::Close { conn: conn.to_string() });

                return;
            }
        }
    }
}
