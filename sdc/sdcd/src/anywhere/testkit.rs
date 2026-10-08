//! Test doubles for the Anywhere core: a fake daemon backend and a software browser.
//!
//! The browser here does what `web/src/crypto` does - HPKE hello, device-signed decisions, passkey
//! assertions - so the core is exercised with bytes it did not produce itself. The cross-language
//! check (the real TypeScript against the real Rust) is `_verify/remote` and `web/tests`.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::core::{Backend, Core, DecisionBy, Out, Settings, StreamEvent};
use super::crypto::{self, b64u, Initiator, RecvHalf, SendHalf, Signer, Welcome};
use super::identity::Identity;
use super::registry::{Device, Registry};
use super::router::{decision_message, Decision};
use super::webauthn::fake::Authenticator;
use super::webauthn::RelyingParty;
use crate::store::sqlite::Store;

pub const T0: i64 = 1_800_000_000_000;

#[derive(Default)]
pub struct FakeBackend {
    pub resolved: Mutex<Vec<(String, String, String)>>,
    pub notes: Mutex<Vec<(String, Value)>>,
    pub events: Mutex<Vec<(i64, Value)>>,
    pub kills: Mutex<u32>,
    /// Who decided each time, with what they said (labels and extras).
    pub details: Mutex<Vec<DecisionBy>>,
}

impl Backend for FakeBackend {
    fn resolve_permission(&self, permission_id: &str, decision: &str, by: &DecisionBy) -> bool {
        self.resolved.lock().unwrap().push((permission_id.to_string(), decision.to_string(), by.device_id.clone()));
        self.details.lock().unwrap().push(by.clone());

        true
    }

    fn events_since(&self, seq: i64, limit: usize) -> Vec<StreamEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| *s > seq)
            .take(limit)
            .map(|(seq, event)| StreamEvent { seq: *seq, event: event.clone(), session_id: Some("s1".into()), turn_id: None })
            .collect()
    }

    fn current_seq(&self) -> i64 {
        self.events.lock().unwrap().last().map(|(seq, _)| *seq).unwrap_or(0)
    }

    fn kill_all(&self) -> Value {
        *self.kills.lock().unwrap() += 1;

        json!({ "killed": 1 })
    }

    fn note(&self, what: &str, detail: Value) {
        self.notes.lock().unwrap().push((what.to_string(), detail));
    }
}

impl FakeBackend {
    pub fn resolved(&self) -> Vec<(String, String, String)> {
        self.resolved.lock().unwrap().clone()
    }

    pub fn noted(&self, what: &str) -> usize {
        self.notes.lock().unwrap().iter().filter(|(kind, _)| kind == what).count()
    }
}

pub struct Rig {
    pub core: Core,
    pub backend: Arc<FakeBackend>,
    pub registry: Arc<Registry>,
    pub clock: Arc<AtomicI64>,
    pub daemon_pub: Vec<u8>,
    pub kem_pub: Vec<u8>,
    pub daemon_id: String,
}

impl Default for Rig {
    fn default() -> Self {
        Self::new()
    }
}

impl Rig {
    pub fn new() -> Self {
        Self::with(Settings::default())
    }

    pub fn with(settings: Settings) -> Self {
        let store = Arc::new(Store::in_memory().unwrap());
        let registry = Arc::new(Registry::new(store));
        let backend = Arc::new(FakeBackend::default());
        let clock = Arc::new(AtomicI64::new(T0));
        let identity = Identity { signer: Signer::generate(), kem: crypto::KemKeys::generate() };
        let (daemon_pub, kem_pub) = (identity.public(), identity.kem.public());
        let ticking = clock.clone();
        let core = Core::new(identity, registry.clone(), backend.clone(), settings, Arc::new(move || ticking.load(Ordering::SeqCst)));
        let daemon_id = core.daemon_id().to_string();

        Self { core, backend, registry, clock, daemon_pub, kem_pub, daemon_id }
    }

    pub fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    /// A permission request as the gate emits it, wrapped as a broadcast notification line.
    pub fn permission_line(&self, permission_id: &str, risk: &str, action: &str, seq: i64) -> String {
        json!({
            "v": "0.1", "seq": seq, "ts": "now",
            "event": { "type": "PermissionRequested", "permissionId": permission_id, "sessionId": "s1", "turnId": "turn-1", "title": "Run a command",
                       "sub": "/srv/shop", "action": action, "target": "pnpm test", "risk": risk, "explain": "Tests first." }
        })
        .to_string()
    }
}

pub struct Browser {
    pub id: String,
    pub signer: Signer,
    pub passkey: Authenticator,
    pub guest: bool,
}

pub struct Session {
    pub conn: String,
    pub send: SendHalf,
    pub recv: RecvHalf,
    inbox: Vec<(u64, Value)>,
    rx: super::frame::Reassembler,
}

impl Browser {
    pub fn new(id: &str) -> Self {
        Self { id: id.into(), signer: Signer::generate(), passkey: Authenticator::new(), guest: false }
    }

    /// Registers this browser as already paired (what pairing leaves behind).
    pub fn register(&self, rig: &Rig) {
        rig.registry
            .add_device(&Device {
                id: self.id.clone(),
                name: format!("{} phone", self.id),
                user_agent: "test".into(),
                sign_pub: self.signer.public(),
                passkey_id: "cred".into(),
                passkey_pub: self.passkey.public(),
                passkey_counter: 0,
                guest: self.guest,
                created_at: T0,
                last_seen: None,
                revoked_at: None,
                expires_at: self.guest.then_some(T0 + 120 * 60_000),
            })
            .unwrap();
    }

    pub fn hello(&self, rig: &Rig, last_seq: i64) -> (Value, Initiator) {
        let (hello, initiator) = Initiator::start(&self.signer, &self.id, &rig.kem_pub, &rig.daemon_id, rig.now(), last_seq).unwrap();

        (json!({ "t": "hello", "device": hello.device, "enc": hello.enc, "ct": hello.ct, "sig": hello.sig }), initiator)
    }

    /// Opens a connection and completes the handshake. Returns the session and the frames the daemon
    /// sent along with the welcome (state, pending cards).
    pub fn connect(&self, rig: &mut Rig, conn: &str, last_seq: i64) -> (Session, Vec<Value>) {
        rig.core.open(conn, false);

        let (hello, initiator) = self.hello(rig, last_seq);
        let out = rig.core.on_message(conn, &hello);
        let welcome_msg = out
            .iter()
            .find_map(|item| match item {
                Out::Send { conn: target, msg } if target == conn && msg["t"] == "welcome" => Some(msg.clone()),
                _ => None,
            })
            .expect("the daemon answered with a welcome");
        let welcome = Welcome {
            enc: welcome_msg["enc"].as_str().unwrap().into(),
            ct: welcome_msg["ct"].as_str().unwrap().into(),
            sig: welcome_msg["sig"].as_str().unwrap().into(),
        };
        let (send, recv, _) = initiator.finish(&welcome, &rig.daemon_pub).expect("the welcome verifies");
        let mut session = Session::new(conn, send, recv);
        let frames = session.frames(&out);

        (session, frames)
    }

    /// The hello of a brand-new device, carrying the pairing body and a passkey proof bound to the
    /// hello's own nonce.
    pub fn pair_hello(&mut self, rig: &Rig, token: &str) -> (Value, Initiator) {
        let guest = self.guest;
        let signer_pub = b64u(&self.signer.public());
        let passkey_pub = b64u(&self.passkey.public());
        let rp = rig.core_rp();
        let passkey = std::cell::RefCell::new(&mut self.passkey);
        let build = |nonce: &str| {
            let assertion = (!guest).then(|| passkey.borrow_mut().assert(&rp, &crypto::pair_challenge(nonce)));

            crypto::PairBody {
                token: token.into(),
                name: "Test phone".into(),
                user_agent: "test".into(),
                sign_pub: signer_pub.clone(),
                passkey_id: if guest { String::new() } else { "cred".into() },
                passkey_pub: if guest { String::new() } else { passkey_pub.clone() },
                assertion: assertion.map(|a| crypto::AssertionWire {
                    authenticator_data: b64u(&a.authenticator_data),
                    client_data_json: b64u(&a.client_data_json),
                    signature: b64u(&a.signature),
                }),
            }
        };
        let (hello, initiator) = Initiator::start_with(&self.signer, &self.id, &rig.kem_pub, &rig.daemon_id, rig.now(), 0, Some(&build)).unwrap();

        (json!({ "t": "hello", "device": hello.device, "enc": hello.enc, "ct": hello.ct, "sig": hello.sig }), initiator)
    }

    /// A passkey assertion for the unlock challenge the daemon gave.
    pub fn assertion_json(&mut self, rig: &Rig, challenge: &[u8]) -> Value {
        let assertion = self.passkey.assert(&rig.core_rp(), challenge);

        json!({
            "authenticator_data": b64u(&assertion.authenticator_data),
            "client_data_json": b64u(&assertion.client_data_json),
            "signature": b64u(&assertion.signature),
        })
    }

    /// The signature the browser puts on a decision.
    pub fn sign_decision(&self, hash: &[u8; 32], decision: Decision) -> String {
        b64u(&self.signer.sign(&decision_message(hash, decision)))
    }
}

impl Rig {
    pub fn core_rp(&self) -> RelyingParty {
        RelyingParty::production()
    }
}

impl Session {
    pub fn new(conn: &str, send: SendHalf, recv: RecvHalf) -> Self {
        Self { conn: conn.into(), send, recv, inbox: Vec::new(), rx: super::frame::Reassembler::default() }
    }

    /// Seals one request frame.
    pub fn frame(&mut self, ch: &str, kind: &str, id: &str, body: Value) -> Value {
        let plain = serde_json::to_vec(&json!({ "ch": ch, "type": kind, "id": id, "body": body })).unwrap();
        let (n, ct) = self.send.seal(&super::frame::whole(&plain)).unwrap();

        json!({ "t": "f", "n": n, "ct": b64u(&ct) })
    }

    /// Sends a request and returns the daemon's outputs. The frames in them are opened at once and
    /// kept, because the receive counter must see every frame in order whether or not a test reads it.
    pub fn call(&mut self, rig: &mut Rig, kind: &str, id: &str, body: Value) -> Vec<Out> {
        let frame = self.frame("control", kind, id, body);
        let out = rig.core.on_message(&self.conn, &frame);

        self.absorb(&out);

        out
    }

    /// Opens the frames in `out` that this session has not opened yet.
    pub fn absorb(&mut self, out: &[Out]) {
        for item in out {
            if let Out::Send { conn, msg } = item {
                if conn == &self.conn && msg["t"] == "f" && msg["n"].as_u64().unwrap() >= self.recv.received() {
                    let ct = crypto::from_b64u(msg["ct"].as_str().unwrap()).unwrap();
                    let plain = self.recv.open(msg["n"].as_u64().unwrap(), &ct).expect("a frame the daemon sealed opens in order");

                    /* A piece of a longer message completes it only with its last piece; that piece's number is the message's. */
                    if let Some(message) = self.rx.push(&plain).expect("the daemon fragments correctly") {
                        self.inbox.push((msg["n"].as_u64().unwrap(), serde_json::from_slice(&message).unwrap()));
                    }
                }
            }
        }
    }


    /// Opens every frame in `out` addressed to this connection, and returns just those.
    pub fn frames(&mut self, out: &[Out]) -> Vec<Value> {
        self.absorb(out);

        let wanted: Vec<u64> = out
            .iter()
            .filter_map(|item| match item {
                Out::Send { conn, msg } if conn == &self.conn && msg["t"] == "f" => msg["n"].as_u64(),
                _ => None,
            })
            .collect();
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.inbox).into_iter().partition(|(n, _)| wanted.contains(n));

        self.inbox = rest;

        mine.into_iter().map(|(_, value)| value).collect()
    }

    /// The first frame of a type.
    pub fn find(frames: &[Value], kind: &str) -> Option<Value> {
        frames.iter().find(|frame| frame["type"] == kind).cloned()
    }

    /// Unlocks the session at `level` through the real challenge → assertion exchange.
    pub fn unlock(&mut self, browser: &mut Browser, rig: &mut Rig, level: &str) -> Vec<Value> {
        let out = self.call(rig, "capability.challenge", "ch", json!({ "level": level }));
        let frames = self.frames(&out);
        let reply = Session::find(&frames, "res").expect("a challenge reply");

        assert_eq!(reply["ok"], true, "challenge refused: {reply}");

        let challenge = crypto::from_b64u(reply["body"]["challenge"].as_str().unwrap()).unwrap();
        let assertion = browser.assertion_json(rig, &challenge);
        let out = self.call(rig, "capability.unlock", "un", json!({ "assertion": assertion }));

        self.frames(&out)
    }
}
