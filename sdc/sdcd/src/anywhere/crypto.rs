//! The end-to-end session between a browser device and this daemon (plan section 4.1).
//!
//! ## Shape
//!
//! Two directions, two independent HPKE contexts (RFC 9180, base mode, DHKEM(X25519, HKDF-SHA256),
//! HKDF-SHA256, AES-256-GCM). An HPKE context numbers its own messages: the AEAD nonce is the base
//! nonce XOR a counter that both ends advance, so a dropped, repeated or reordered frame fails to
//! open. The relay therefore cannot replay or reorder anything without the session noticing.
//!
//! ```text
//! device                                                     daemon
//!   | hello { device, enc, ct, sig }  --------------------->  |  ct = Seal_c2d(HelloPlain{device_pk, ts, nonce, last_seq})
//!   |        sig = ECDSA(device key, label | enc | ct)          |  daemon: open, check device is trusted, check sig,
//!   |                                                           |          check ts window and nonce unused
//!   | <-----------------  welcome { enc, ct, sig }              |  ct = Seal_d2c(WelcomePlain{nonce echo, level, seq})
//!   |        sig = ECDSA(daemon key, label | hello.sig | enc | ct)
//! ```
//!
//! * the device is authenticated by its ECDSA P-256 key (a WebCrypto non-extractable key in the
//!   browser), registered when the device was paired;
//! * the daemon is authenticated by its ECDSA P-256 identity key, whose fingerprint the device
//!   pinned at pairing (the QR code carries it);
//! * the X25519 keys are per session, so a later compromise of a device's signing key does not open
//!   old traffic in the daemon-to-device direction; the device-to-daemon direction is sealed to the
//!   daemon's long-term KEM key, which is rotated on every rekey of the daemon identity.
//!
//! A session is rekeyed by running the handshake again (`SendHalf::needs_rekey`).
//!
//! The server never holds any of these keys. It forwards the `hello`, `welcome` and frame JSON.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hpke::aead::{AeadCtxR, AeadCtxS, AesGcm256};
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem as KemTrait, OpModeR, OpModeS, Serializable};
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type Kem = X25519HkdfSha256;
type Aead = AesGcm256;
type Kdf = HkdfSha256;

pub const LABEL_HELLO: &[u8] = b"sdc-anywhere/v1/hello";
pub const LABEL_WELCOME: &[u8] = b"sdc-anywhere/v1/welcome";
pub const LABEL_DECISION: &[u8] = b"sdc-anywhere/v1/decision";
const INFO_C2D: &[u8] = b"sdc-anywhere/v1/c2d";
const INFO_D2C: &[u8] = b"sdc-anywhere/v1/d2c";

/// A session is rekeyed after this many messages or bytes in one direction, whichever comes first
/// (plan 4.1: one hour or 1 GB; the hour is checked by the caller's clock).
pub const REKEY_AFTER_MESSAGES: u64 = 1 << 20;
pub const REKEY_AFTER_BYTES: u64 = 1 << 30;

/// How far a `hello`'s timestamp may differ from this machine's clock (ms).
pub const HELLO_WINDOW_MS: i64 = 120_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoError {
    Malformed(&'static str),
    BadSignature(&'static str),
    Decrypt,
    Stale,
    UnknownDevice,
    Limit,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(what) => write!(f, "malformed {what}"),
            Self::BadSignature(what) => write!(f, "bad signature on {what}"),
            Self::Decrypt => write!(f, "a frame did not decrypt (dropped, repeated, reordered or altered)"),
            Self::Stale => write!(f, "the hello is outside the allowed time window"),
            Self::UnknownDevice => write!(f, "the device is not registered"),
            Self::Limit => write!(f, "the session reached its message limit and must be rekeyed"),
        }
    }
}

impl std::error::Error for CryptoError {}

// --- encoding helpers ---------------------------------------------------------------------------

pub fn b64u(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn from_b64u(text: &str) -> Result<Vec<u8>, CryptoError> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).map_err(|_| CryptoError::Malformed("base64url"))
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];

    getrandom::fill(&mut bytes).expect("the operating system has a random source");

    bytes
}

pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();

    for part in parts {
        hasher.update(part);
    }

    hasher.finalize().into()
}

// --- signing keys -------------------------------------------------------------------------------

/// An ECDSA P-256 signing key: the daemon's identity, or (in tests) a stand-in for a browser device.
pub struct Signer {
    key: SigningKey,
}

impl Signer {
    pub fn generate() -> Self {
        loop {
            let bytes: [u8; 32] = random();

            /* A random 32-byte string is a valid P-256 scalar except with probability about 2^-32. */
            if let Ok(key) = SigningKey::from_slice(&bytes) {
                return Self { key };
            }
        }
    }

    pub fn from_secret(bytes: &[u8]) -> Result<Self, CryptoError> {
        SigningKey::from_slice(bytes).map(|key| Self { key }).map_err(|_| CryptoError::Malformed("signing key"))
    }

    /// The 32 secret bytes. Callers put these in the keychain and nowhere else.
    pub fn secret(&self) -> Vec<u8> {
        self.key.to_bytes().to_vec()
    }

    /// The underlying key, for test code that needs to sign in another format (WebAuthn's DER).
    #[cfg(test)]
    pub fn signing_key(&self) -> &SigningKey {
        &self.key
    }

    /// SEC1 uncompressed public key (65 bytes), the form WebCrypto exports as `raw`.
    pub fn public(&self) -> Vec<u8> {
        self.key.verifying_key().to_sec1_point(false).as_bytes().to_vec()
    }

    /// ECDSA P-256 / SHA-256 signature, raw `r || s` (64 bytes), the form WebCrypto produces.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let signature: Signature = self.key.sign(message);

        signature.to_bytes().into()
    }
}

/// Verifies a raw 64-byte ECDSA P-256 signature against a SEC1 public key.
pub fn verify(public: &[u8], message: &[u8], signature: &[u8]) -> Result<(), CryptoError> {
    let key = VerifyingKey::from_sec1_bytes(public).map_err(|_| CryptoError::Malformed("public key"))?;
    let signature = Signature::from_slice(signature).map_err(|_| CryptoError::Malformed("signature"))?;

    key.verify(message, &signature).map_err(|_| CryptoError::BadSignature("message"))
}

/// The id a daemon is known by: the first 16 bytes of SHA-256 of its identity public key.
pub fn daemon_id(identity_public: &[u8]) -> String {
    b64u(&sha256(&[identity_public])[..16])
}

/// The fingerprint shown to a person, hex in groups, so two can be compared by eye.
pub fn fingerprint(public: &[u8]) -> String {
    let digest = sha256(&[public]);

    digest[..10].chunks(2).map(|pair| format!("{:02x}{:02x}", pair[0], pair[1])).collect::<Vec<_>>().join("-")
}

// --- the daemon's key-encapsulation key ----------------------------------------------------------

/// The X25519 key the device-to-daemon direction is sealed to.
pub struct KemKeys {
    secret: <Kem as KemTrait>::PrivateKey,
    public: <Kem as KemTrait>::PublicKey,
}

impl KemKeys {
    pub fn generate() -> Self {
        let (secret, public) = Kem::gen_keypair();

        Self { secret, public }
    }

    pub fn from_secret(bytes: &[u8]) -> Result<Self, CryptoError> {
        let secret = <Kem as KemTrait>::PrivateKey::from_bytes(bytes).map_err(|_| CryptoError::Malformed("kem key"))?;
        let public = Kem::sk_to_pk(&secret);

        Ok(Self { secret, public })
    }

    pub fn secret(&self) -> Vec<u8> {
        self.secret.to_bytes().as_slice().to_vec()
    }

    pub fn public(&self) -> Vec<u8> {
        self.public.to_bytes().as_slice().to_vec()
    }
}

// --- directional halves ---------------------------------------------------------------------------

/// The sending half of a session: seals one message at a time and numbers it.
pub struct SendHalf {
    ctx: AeadCtxS<Aead, Kdf, Kem>,
    messages: u64,
    bytes: u64,
}

/// The receiving half: opens messages strictly in the order they were sealed.
pub struct RecvHalf {
    ctx: AeadCtxR<Aead, Kdf, Kem>,
    messages: u64,
}

impl SendHalf {
    fn new(ctx: AeadCtxS<Aead, Kdf, Kem>) -> Self {
        Self { ctx, messages: 0, bytes: 0 }
    }

    /// Seals `plain`. The message number is bound as associated data, so a frame that claims a
    /// different number than its position fails to open.
    pub fn seal(&mut self, plain: &[u8]) -> Result<(u64, Vec<u8>), CryptoError> {
        if self.needs_rekey() {
            return Err(CryptoError::Limit);
        }

        let number = self.messages;
        let ct = self.ctx.seal(plain, &number.to_be_bytes()).map_err(|_| CryptoError::Limit)?;

        self.messages += 1;
        self.bytes += plain.len() as u64;

        Ok((number, ct))
    }

    pub fn needs_rekey(&self) -> bool {
        self.messages >= REKEY_AFTER_MESSAGES || self.bytes >= REKEY_AFTER_BYTES
    }

    pub fn sent(&self) -> u64 {
        self.messages
    }
}

impl RecvHalf {
    fn new(ctx: AeadCtxR<Aead, Kdf, Kem>) -> Self {
        Self { ctx, messages: 0 }
    }

    /// Opens message number `number`, which must be exactly the next one.
    pub fn open(&mut self, number: u64, ct: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if number != self.messages {
            return Err(CryptoError::Decrypt);
        }

        let plain = self.ctx.open(ct, &number.to_be_bytes()).map_err(|_| CryptoError::Decrypt)?;

        self.messages += 1;

        Ok(plain)
    }

    pub fn received(&self) -> u64 {
        self.messages
    }
}

// --- the handshake --------------------------------------------------------------------------------

/// What a device sends first. All binary fields are base64url.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hello {
    pub device: String,
    pub enc: String,
    pub ct: String,
    pub sig: String,
}

/// What the daemon answers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Welcome {
    pub enc: String,
    pub ct: String,
    pub sig: String,
}

/// The sealed part of a `hello`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloPlain {
    pub device: String,
    /// The device's per-session X25519 public key, base64url. The daemon seals its answers to it.
    pub pk: String,
    pub ts: i64,
    /// Random, single use: the daemon remembers it so the same hello cannot be played twice.
    pub nonce: String,
    /// The last event the device already has; the daemon replays what came after.
    pub last_seq: i64,
    /// Present only in the first hello of a brand-new device (pairing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<PairBody>,
}

/// What a device that is not paired yet tells the daemon, sealed inside its first hello.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairBody {
    /// The secret from the QR code. The daemon stores only its hash and spends it once.
    pub token: String,
    pub name: String,
    pub user_agent: String,
    /// The device's ECDSA P-256 public key (SEC1, base64url). Also proves possession: the hello is
    /// signed with the matching private key.
    pub sign_pub: String,
    /// Empty for a guest session, which has no passkey.
    #[serde(default)]
    pub passkey_id: String,
    #[serde(default)]
    pub passkey_pub: String,
    /// A passkey assertion whose challenge is `pair_challenge(nonce)`: proves the browser holds the
    /// passkey and verified the user while creating it.
    #[serde(default)]
    pub assertion: Option<AssertionWire>,
}

/// A WebAuthn assertion on the wire (base64url fields).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssertionWire {
    pub authenticator_data: String,
    pub client_data_json: String,
    pub signature: String,
}

/// The challenge a new device's passkey must sign: bound to this hello and no other.
pub fn pair_challenge(nonce: &str) -> [u8; 32] {
    sha256(&[b"sdc-anywhere/v1/pair-challenge", nonce.as_bytes()])
}

/// The six digits both screens show during pairing, from everything the two sides agreed on. A person
/// who compares them notices a device that is not the one in their hand.
pub fn sas_code(token: &str, device_sign_pub: &[u8], daemon_identity_pub: &[u8]) -> String {
    let digest = sha256(&[b"sdc-anywhere/v1/sas", token.as_bytes(), device_sign_pub, daemon_identity_pub]);
    let number = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;

    format!("{:03} {:03}", number / 1000, number % 1000)
}

/// The sealed part of a `welcome`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomePlain {
    pub device: String,
    pub ts: i64,
    /// The hello's nonce, so the device knows this welcome answers *its* hello.
    pub nonce: String,
    pub level: String,
    pub seq: i64,
}

fn info(label: &[u8], daemon_id: &str, device: &str) -> Vec<u8> {
    [label, b"|", daemon_id.as_bytes(), b"|", device.as_bytes()].concat()
}

/// The device side of a handshake in progress.
pub struct Initiator {
    send: SendHalf,
    secret: <Kem as KemTrait>::PrivateKey,
    daemon_id: String,
    device: String,
    nonce: String,
    hello_sig: Vec<u8>,
}

impl Initiator {
    /// Builds the `hello`. `daemon_kem_public` and `daemon_id` come from the pairing the device did.
    pub fn start(
        device_signer: &Signer,
        device: &str,
        daemon_kem_public: &[u8],
        daemon_id: &str,
        now_ms: i64,
        last_seq: i64,
    ) -> Result<(Hello, Self), CryptoError> {
        Self::start_with(device_signer, device, daemon_kem_public, daemon_id, now_ms, last_seq, None)
    }

    /// `start`, for the first hello of a new device, which carries the pairing body.
    pub fn start_with(
        device_signer: &Signer,
        device: &str,
        daemon_kem_public: &[u8],
        daemon_id: &str,
        now_ms: i64,
        last_seq: i64,
        pair: Option<&dyn Fn(&str) -> PairBody>,
    ) -> Result<(Hello, Self), CryptoError> {
        let recipient = <Kem as KemTrait>::PublicKey::from_bytes(daemon_kem_public).map_err(|_| CryptoError::Malformed("daemon kem key"))?;
        let (secret, public) = Kem::gen_keypair();
        let nonce = b64u(&random::<16>());
        let plain = HelloPlain { device: device.to_string(), pk: b64u(public.to_bytes().as_slice()), ts: now_ms, nonce: nonce.clone(), last_seq, pair: pair.map(|build| build(&nonce)) };
        let (enc, mut ctx) = hpke::setup_sender::<Aead, Kdf, Kem>(&OpModeS::Base, &recipient, &info(INFO_C2D, daemon_id, device))
            .map_err(|_| CryptoError::Malformed("hello setup"))?;
        let ct = ctx.seal(&serde_json::to_vec(&plain).expect("a hello serialises"), &[]).map_err(|_| CryptoError::Malformed("hello seal"))?;
        let enc_bytes = enc.to_bytes();
        let sig = device_signer.sign(&[LABEL_HELLO, enc_bytes.as_slice(), &ct].concat());
        let hello = Hello { device: device.to_string(), enc: b64u(enc_bytes.as_slice()), ct: b64u(&ct), sig: b64u(&sig) };

        Ok((
            hello,
            Self { send: SendHalf::new(ctx), secret, daemon_id: daemon_id.to_string(), device: device.to_string(), nonce, hello_sig: sig.to_vec() },
        ))
    }

    /// Verifies the daemon's `welcome` against the identity key pinned at pairing and returns the
    /// two halves. Fails if the welcome answers a different hello.
    pub fn finish(self, welcome: &Welcome, daemon_identity_public: &[u8]) -> Result<(SendHalf, RecvHalf, WelcomePlain), CryptoError> {
        let enc = from_b64u(&welcome.enc)?;
        let ct = from_b64u(&welcome.ct)?;
        let sig = from_b64u(&welcome.sig)?;

        verify(daemon_identity_public, &[LABEL_WELCOME, &self.hello_sig, &enc, &ct].concat(), &sig)
            .map_err(|_| CryptoError::BadSignature("welcome"))?;

        let encapped = <Kem as KemTrait>::EncappedKey::from_bytes(&enc).map_err(|_| CryptoError::Malformed("welcome enc"))?;
        let mut ctx = hpke::setup_receiver::<Aead, Kdf, Kem>(&OpModeR::Base, &self.secret, &encapped, &info(INFO_D2C, &self.daemon_id, &self.device))
            .map_err(|_| CryptoError::Decrypt)?;
        let plain = ctx.open(&ct, &[]).map_err(|_| CryptoError::Decrypt)?;
        let body: WelcomePlain = serde_json::from_slice(&plain).map_err(|_| CryptoError::Malformed("welcome body"))?;

        if body.nonce != self.nonce || body.device != self.device {
            return Err(CryptoError::BadSignature("welcome (answers another hello)"));
        }

        Ok((self.send, RecvHalf::new(ctx), body))
    }
}

/// The daemon side after it has opened and verified a `hello`.
pub struct Pending {
    recv: RecvHalf,
    device: String,
    device_pk: <Kem as KemTrait>::PublicKey,
    hello_sig: Vec<u8>,
    daemon_id: String,
}

/// The device named in a hello, so the caller can look up its registered public key before the
/// hello is opened.
pub fn hello_device(hello: &Hello) -> &str {
    &hello.device
}

/// The daemon's half of the handshake for a device that is **not registered yet** (its first hello).
///
/// The hello is opened first, because the device's key is inside it; then the signature is checked
/// against that key, which proves the sender holds the private half. Whether the pairing token is
/// valid is the caller's job (`registry::consume_pairing`) and so is checking the passkey assertion.
pub fn accept_pairing(
    kem: &KemKeys,
    daemon_id: &str,
    hello: &Hello,
    now_ms: i64,
) -> Result<(HelloPlain, PairBody, Pending), CryptoError> {
    let enc = from_b64u(&hello.enc)?;
    let ct = from_b64u(&hello.ct)?;
    let sig = from_b64u(&hello.sig)?;
    let encapped = <Kem as KemTrait>::EncappedKey::from_bytes(&enc).map_err(|_| CryptoError::Malformed("hello enc"))?;
    let mut ctx = hpke::setup_receiver::<Aead, Kdf, Kem>(&OpModeR::Base, &kem.secret, &encapped, &info(INFO_C2D, daemon_id, &hello.device))
        .map_err(|_| CryptoError::Decrypt)?;
    let plain = ctx.open(&ct, &[]).map_err(|_| CryptoError::Decrypt)?;
    let body: HelloPlain = serde_json::from_slice(&plain).map_err(|_| CryptoError::Malformed("hello body"))?;
    let pair = body.pair.clone().ok_or(CryptoError::Malformed("pairing body"))?;

    if body.device != hello.device {
        return Err(CryptoError::BadSignature("hello (device mismatch)"));
    }

    verify(&from_b64u(&pair.sign_pub)?, &[LABEL_HELLO, &enc, &ct].concat(), &sig).map_err(|_| CryptoError::BadSignature("pairing hello"))?;

    if (now_ms - body.ts).abs() > HELLO_WINDOW_MS {
        return Err(CryptoError::Stale);
    }

    let device_pk = <Kem as KemTrait>::PublicKey::from_bytes(&from_b64u(&body.pk)?).map_err(|_| CryptoError::Malformed("device session key"))?;

    Ok((
        body,
        pair,
        Pending { recv: RecvHalf::new(ctx), device: hello.device.clone(), device_pk, hello_sig: sig, daemon_id: daemon_id.to_string() },
    ))
}

/// The daemon's half of the handshake.
///
/// `device_public` is the ECDSA key registered for `hello.device`; the caller must already have
/// established that the device exists and is not revoked. `now_ms` is this machine's clock. The
/// caller still has to check that `HelloPlain::nonce` has not been seen (it owns the nonce store).
pub fn accept(
    kem: &KemKeys,
    daemon_id: &str,
    hello: &Hello,
    device_public: &[u8],
    now_ms: i64,
) -> Result<(HelloPlain, Pending), CryptoError> {
    let enc = from_b64u(&hello.enc)?;
    let ct = from_b64u(&hello.ct)?;
    let sig = from_b64u(&hello.sig)?;

    /* The signature first: nothing is decrypted for a device that did not sign these bytes. */
    verify(device_public, &[LABEL_HELLO, &enc, &ct].concat(), &sig).map_err(|_| CryptoError::BadSignature("hello"))?;

    let encapped = <Kem as KemTrait>::EncappedKey::from_bytes(&enc).map_err(|_| CryptoError::Malformed("hello enc"))?;
    let mut ctx = hpke::setup_receiver::<Aead, Kdf, Kem>(&OpModeR::Base, &kem.secret, &encapped, &info(INFO_C2D, daemon_id, &hello.device))
        .map_err(|_| CryptoError::Decrypt)?;
    let plain = ctx.open(&ct, &[]).map_err(|_| CryptoError::Decrypt)?;
    let body: HelloPlain = serde_json::from_slice(&plain).map_err(|_| CryptoError::Malformed("hello body"))?;

    if body.device != hello.device {
        return Err(CryptoError::BadSignature("hello (device mismatch)"));
    }

    if (now_ms - body.ts).abs() > HELLO_WINDOW_MS {
        return Err(CryptoError::Stale);
    }

    let device_pk = <Kem as KemTrait>::PublicKey::from_bytes(&from_b64u(&body.pk)?).map_err(|_| CryptoError::Malformed("device session key"))?;

    Ok((
        body.clone(),
        Pending { recv: RecvHalf::new(ctx), device: hello.device.clone(), device_pk, hello_sig: sig, daemon_id: daemon_id.to_string() },
    ))
}

impl Pending {
    /// Answers the hello. `plain.nonce` must be the hello's nonce.
    pub fn welcome(self, identity: &Signer, plain: &WelcomePlain) -> Result<(Welcome, SendHalf, RecvHalf), CryptoError> {
        let (enc, mut ctx) = hpke::setup_sender::<Aead, Kdf, Kem>(&OpModeS::Base, &self.device_pk, &info(INFO_D2C, &self.daemon_id, &self.device))
            .map_err(|_| CryptoError::Malformed("welcome setup"))?;
        let ct = ctx.seal(&serde_json::to_vec(plain).expect("a welcome serialises"), &[]).map_err(|_| CryptoError::Limit)?;
        let enc_bytes = enc.to_bytes();
        let sig = identity.sign(&[LABEL_WELCOME, &self.hello_sig, enc_bytes.as_slice(), &ct].concat());
        let welcome = Welcome { enc: b64u(enc_bytes.as_slice()), ct: b64u(&ct), sig: b64u(&sig) };

        Ok((welcome, SendHalf::new(ctx), self.recv))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct World {
        identity: Signer,
        kem: KemKeys,
        daemon_id: String,
        device: Signer,
    }

    fn world() -> World {
        let identity = Signer::generate();
        let daemon_id = daemon_id(&identity.public());

        World { identity, kem: KemKeys::generate(), daemon_id, device: Signer::generate() }
    }

    const NOW: i64 = 1_800_000_000_000;

    fn connect(w: &World) -> ((SendHalf, RecvHalf), (SendHalf, RecvHalf)) {
        let (hello, initiator) = Initiator::start(&w.device, "dev1", &w.kem.public(), &w.daemon_id, NOW, 7).unwrap();
        let (plain, pending) = accept(&w.kem, &w.daemon_id, &hello, &w.device.public(), NOW + 50).unwrap();

        assert_eq!(plain.last_seq, 7);

        let welcome_plain = WelcomePlain { device: "dev1".into(), ts: NOW, nonce: plain.nonce.clone(), level: "view".into(), seq: 9 };
        let (welcome, d_send, d_recv) = pending.welcome(&w.identity, &welcome_plain).unwrap();
        let (c_send, c_recv, body) = initiator.finish(&welcome, &w.identity.public()).unwrap();

        assert_eq!(body.seq, 9);

        ((c_send, c_recv), (d_send, d_recv))
    }

    #[test]
    fn a_session_carries_messages_both_ways() {
        let w = world();
        let ((mut c_send, mut c_recv), (mut d_send, mut d_recv)) = connect(&w);

        let (n, ct) = c_send.seal(b"approve").unwrap();

        assert_eq!(d_recv.open(n, &ct).unwrap(), b"approve");

        let (n, ct) = d_send.seal(b"card").unwrap();

        assert_eq!(c_recv.open(n, &ct).unwrap(), b"card");
        assert_ne!(ct, b"card");
    }

    #[test]
    fn a_replayed_frame_does_not_open() {
        let w = world();
        let (( mut c_send, _), (_, mut d_recv)) = connect(&w);
        let (n, ct) = c_send.seal(b"once").unwrap();

        assert!(d_recv.open(n, &ct).is_ok());
        assert_eq!(d_recv.open(n, &ct), Err(CryptoError::Decrypt));
    }

    #[test]
    fn a_dropped_or_reordered_frame_is_noticed() {
        let w = world();
        let ((mut c_send, _), (_, mut d_recv)) = connect(&w);
        let (n0, ct0) = c_send.seal(b"zero").unwrap();
        let (n1, ct1) = c_send.seal(b"one").unwrap();

        /* Frame 1 arrives first: the relay reordered, or frame 0 was dropped. */
        assert_eq!(d_recv.open(n1, &ct1), Err(CryptoError::Decrypt));
        assert_eq!(d_recv.open(n0, &ct0).unwrap(), b"zero");
    }

    #[test]
    fn an_altered_frame_does_not_open() {
        let w = world();
        let ((mut c_send, _), (_, mut d_recv)) = connect(&w);
        let (n, mut ct) = c_send.seal(b"allow ls").unwrap();

        ct[0] ^= 1;

        assert_eq!(d_recv.open(n, &ct), Err(CryptoError::Decrypt));
    }

    #[test]
    fn a_frame_that_claims_another_number_does_not_open() {
        let w = world();
        let ((mut c_send, _), (_, mut d_recv)) = connect(&w);
        let (_, ct) = c_send.seal(b"x").unwrap();

        assert_eq!(d_recv.open(5, &ct), Err(CryptoError::Decrypt));
    }

    #[test]
    fn a_hello_from_an_unregistered_key_is_refused() {
        let w = world();
        let stranger = Signer::generate();
        let (hello, _) = Initiator::start(&stranger, "dev1", &w.kem.public(), &w.daemon_id, NOW, 0).unwrap();

        /* The daemon has dev1 registered with w.device's key, so it verifies against that. */
        assert!(matches!(accept(&w.kem, &w.daemon_id, &hello, &w.device.public(), NOW), Err(CryptoError::BadSignature(_))));
    }

    #[test]
    fn a_tampered_hello_is_refused_before_it_is_opened() {
        let w = world();
        let (mut hello, _) = Initiator::start(&w.device, "dev1", &w.kem.public(), &w.daemon_id, NOW, 0).unwrap();
        let mut ct = from_b64u(&hello.ct).unwrap();

        ct[3] ^= 0x40;
        hello.ct = b64u(&ct);

        assert!(matches!(accept(&w.kem, &w.daemon_id, &hello, &w.device.public(), NOW), Err(CryptoError::BadSignature(_))));
    }

    #[test]
    fn a_stale_hello_is_refused() {
        let w = world();
        let (hello, _) = Initiator::start(&w.device, "dev1", &w.kem.public(), &w.daemon_id, NOW, 0).unwrap();

        assert!(matches!(accept(&w.kem, &w.daemon_id, &hello, &w.device.public(), NOW + HELLO_WINDOW_MS + 1), Err(CryptoError::Stale)));
        assert!(matches!(accept(&w.kem, &w.daemon_id, &hello, &w.device.public(), NOW - HELLO_WINDOW_MS - 1), Err(CryptoError::Stale)));
    }

    #[test]
    fn a_hello_for_another_daemon_does_not_open() {
        let w = world();
        let other = KemKeys::generate();
        let (hello, _) = Initiator::start(&w.device, "dev1", &w.kem.public(), &w.daemon_id, NOW, 0).unwrap();

        assert!(accept(&other, &w.daemon_id, &hello, &w.device.public(), NOW).is_err());
        assert!(accept(&w.kem, "another-daemon", &hello, &w.device.public(), NOW).is_err());
    }

    #[test]
    fn a_welcome_from_the_wrong_daemon_is_refused_by_the_device() {
        let w = world();
        let impostor = Signer::generate();
        let (hello, initiator) = Initiator::start(&w.device, "dev1", &w.kem.public(), &w.daemon_id, NOW, 0).unwrap();
        let (plain, pending) = accept(&w.kem, &w.daemon_id, &hello, &w.device.public(), NOW).unwrap();
        let welcome_plain = WelcomePlain { device: "dev1".into(), ts: NOW, nonce: plain.nonce, level: "view".into(), seq: 0 };
        let (welcome, _, _) = pending.welcome(&impostor, &welcome_plain).unwrap();

        /* The device pinned the real daemon's identity key; the impostor's signature does not match. */
        assert!(matches!(initiator.finish(&welcome, &w.identity.public()), Err(CryptoError::BadSignature(_))));
    }

    #[test]
    fn a_welcome_for_a_different_hello_is_refused() {
        let w = world();
        let (hello_a, initiator_a) = Initiator::start(&w.device, "dev1", &w.kem.public(), &w.daemon_id, NOW, 0).unwrap();
        let (hello_b, _) = Initiator::start(&w.device, "dev1", &w.kem.public(), &w.daemon_id, NOW, 0).unwrap();
        let (plain_b, pending_b) = accept(&w.kem, &w.daemon_id, &hello_b, &w.device.public(), NOW).unwrap();
        let (_, _) = accept(&w.kem, &w.daemon_id, &hello_a, &w.device.public(), NOW).unwrap();
        let welcome_plain = WelcomePlain { device: "dev1".into(), ts: NOW, nonce: plain_b.nonce, level: "view".into(), seq: 0 };
        let (welcome_b, _, _) = pending_b.welcome(&w.identity, &welcome_plain).unwrap();

        /* A relay that answers A's hello with B's welcome. */
        assert!(initiator_a.finish(&welcome_b, &w.identity.public()).is_err());
    }

    #[test]
    fn rekey_is_demanded_at_the_message_limit() {
        let w = world();
        let ((mut c_send, _), _) = connect(&w);

        c_send.messages = REKEY_AFTER_MESSAGES;

        assert!(c_send.needs_rekey());
        assert_eq!(c_send.seal(b"x"), Err(CryptoError::Limit));
    }

    #[test]
    fn identity_keys_survive_storage() {
        let one = Signer::generate();
        let two = Signer::from_secret(&one.secret()).unwrap();

        assert_eq!(one.public(), two.public());

        let kem = KemKeys::generate();
        let again = KemKeys::from_secret(&kem.secret()).unwrap();

        assert_eq!(kem.public(), again.public());
    }

    #[test]
    fn fingerprints_are_short_and_stable() {
        let key = Signer::generate().public();

        assert_eq!(fingerprint(&key), fingerprint(&key));
        assert_eq!(fingerprint(&key).len(), 24);
    }
}

/// `cargo test --release bench_seal_and_open -- --ignored --nocapture` (PERF.md row 16). Ignored by default: a
/// timing test in the normal run would fail on a busy machine for no reason a person could fix.
#[cfg(test)]
mod bench {
    use super::*;

    fn percentile(sorted: &[f64], q: f64) -> f64 {
        sorted[((sorted.len() as f64 * q).ceil() as usize).saturating_sub(1).min(sorted.len() - 1)]
    }

    #[test]
    #[ignore]
    fn bench_seal_and_open() {
        let identity = Signer::generate();
        let kem = KemKeys::generate();
        let daemon_id = daemon_id(&identity.public());
        let device = Signer::generate();
        let (hello, initiator) = Initiator::start(&device, "bench-device", &kem.public(), &daemon_id, 1_800_000_000_000, 0).unwrap();
        let (plain, pending) = accept(&kem, &daemon_id, &hello, &device.public(), 1_800_000_000_000).unwrap();
        let welcome_plain = WelcomePlain { device: "bench-device".into(), ts: 0, nonce: plain.nonce, level: "view".into(), seq: 0 };
        let (welcome, _, mut d_recv) = pending.welcome(&identity, &welcome_plain).unwrap();
        let (mut c_send, _, _) = initiator.finish(&welcome, &identity.public()).unwrap();

        for size in [256_usize, 1024, 16 * 1024, 256 * 1024] {
            let message = vec![0x5a_u8; size];
            let mut seal_us = Vec::new();
            let mut open_us = Vec::new();

            for _ in 0..2000 {
                let t = std::time::Instant::now();
                let (n, ct) = c_send.seal(&message).unwrap();

                seal_us.push(t.elapsed().as_secs_f64() * 1e6);

                let t = std::time::Instant::now();

                d_recv.open(n, &ct).unwrap();
                open_us.push(t.elapsed().as_secs_f64() * 1e6);
            }

            seal_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
            open_us.sort_by(|a, b| a.partial_cmp(b).unwrap());

            println!(
                "BENCH {size:>7} B  seal p50 {:8.1} us p95 {:8.1} us   open p50 {:8.1} us p95 {:8.1} us",
                percentile(&seal_us, 0.5),
                percentile(&seal_us, 0.95),
                percentile(&open_us, 0.5),
                percentile(&open_us, 0.95)
            );
        }

        let mut handshakes = Vec::new();

        for _ in 0..200 {
            let t = std::time::Instant::now();
            let (hello, initiator) = Initiator::start(&device, "bench-device", &kem.public(), &daemon_id, 1_800_000_000_000, 0).unwrap();
            let (plain, pending) = accept(&kem, &daemon_id, &hello, &device.public(), 1_800_000_000_000).unwrap();
            let wp = WelcomePlain { device: "bench-device".into(), ts: 0, nonce: plain.nonce, level: "view".into(), seq: 0 };
            let (welcome, _, _) = pending.welcome(&identity, &wp).unwrap();

            initiator.finish(&welcome, &identity.public()).unwrap();
            handshakes.push(t.elapsed().as_secs_f64() * 1e3);
        }

        handshakes.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("BENCH full handshake (both sides, one process)  p50 {:.2} ms p95 {:.2} ms", percentile(&handshakes, 0.5), percentile(&handshakes, 0.95));
    }
}
