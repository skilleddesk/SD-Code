//! Verifying a passkey assertion on the daemon (threat T28).
//!
//! The server is not trusted to say "the passkey checked out": the daemon checks the assertion itself.
//! What it checks, in the order the WebAuthn specification gives (section 7.2, authentication):
//!
//! 1. `clientDataJSON` is `webauthn.get`, for the challenge the daemon expects, from the origin the
//!    daemon expects, and not cross-origin;
//! 2. the authenticator data starts with SHA-256 of the relying-party id, and both "user present" and
//!    "user verified" are set (a passkey that did not check a fingerprint or a PIN is refused);
//! 3. the signature over `authenticator data || SHA-256(clientDataJSON)` verifies with the key stored
//!    at pairing (ES256, P-256);
//! 4. the signature counter went up, when the authenticator keeps one.
//!
//! The challenge is the `action_hash` of the request being approved (or the daemon's unlock nonce), so
//! a valid assertion is a signature over exactly one thing.

use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::Deserialize;

use super::crypto::{b64u, sha256};

/// Where the web app lives. The relying-party id is the host; the origin is what the browser reports.
#[derive(Debug, Clone)]
pub struct RelyingParty {
    pub id: String,
    pub origin: String,
}

impl RelyingParty {
    pub fn production() -> Self {
        Self { id: "sdc.skilleddesk.com".into(), origin: "https://sdc.skilleddesk.com".into() }
    }
}

/// What the browser hands over after `navigator.credentials.get`. All fields are raw bytes here; the
/// wire carries them base64url.
#[derive(Debug, Clone)]
pub struct Assertion {
    pub authenticator_data: Vec<u8>,
    pub client_data_json: Vec<u8>,
    /// DER-encoded ECDSA signature, as WebAuthn returns it.
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebAuthnError {
    ClientData(&'static str),
    Challenge,
    Origin,
    RpIdHash,
    NotVerified,
    Signature,
    Counter,
    Key,
}

impl std::fmt::Display for WebAuthnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClientData(what) => write!(f, "client data: {what}"),
            Self::Challenge => write!(f, "the assertion answers a different challenge"),
            Self::Origin => write!(f, "the assertion came from another origin"),
            Self::RpIdHash => write!(f, "the assertion is for another relying party"),
            Self::NotVerified => write!(f, "the passkey did not verify the user (fingerprint, face or PIN)"),
            Self::Signature => write!(f, "the passkey signature is wrong"),
            Self::Counter => write!(f, "the passkey's counter did not increase (a cloned authenticator?)"),
            Self::Key => write!(f, "the stored passkey public key is invalid"),
        }
    }
}

impl std::error::Error for WebAuthnError {}

#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin", default)]
    cross_origin: bool,
}

/// Verifies `assertion` for `expected_challenge` against the stored SEC1 public key.
///
/// `stored_counter` is the last counter seen for this credential. On success the new counter is
/// returned and must be stored. An authenticator that never counts (always 0) is accepted as long as it
/// stays at 0.
pub fn verify(
    rp: &RelyingParty,
    credential_public: &[u8],
    stored_counter: u32,
    expected_challenge: &[u8],
    assertion: &Assertion,
) -> Result<u32, WebAuthnError> {
    let client: ClientData =
        serde_json::from_slice(&assertion.client_data_json).map_err(|_| WebAuthnError::ClientData("not JSON"))?;

    if client.kind != "webauthn.get" {
        return Err(WebAuthnError::ClientData("type is not webauthn.get"));
    }

    if client.challenge != b64u(expected_challenge) {
        return Err(WebAuthnError::Challenge);
    }

    if client.origin != rp.origin || client.cross_origin {
        return Err(WebAuthnError::Origin);
    }

    let data = &assertion.authenticator_data;

    if data.len() < 37 {
        return Err(WebAuthnError::ClientData("authenticator data is too short"));
    }

    if data[..32] != sha256(&[rp.id.as_bytes()]) {
        return Err(WebAuthnError::RpIdHash);
    }

    let flags = data[32];

    /* Bit 0 user present, bit 2 user verified. */
    if flags & 0x01 == 0 || flags & 0x04 == 0 {
        return Err(WebAuthnError::NotVerified);
    }

    let counter = u32::from_be_bytes([data[33], data[34], data[35], data[36]]);

    if (counter != 0 || stored_counter != 0) && counter <= stored_counter {
        return Err(WebAuthnError::Counter);
    }

    let key = VerifyingKey::from_sec1_bytes(credential_public).map_err(|_| WebAuthnError::Key)?;
    let signature = Signature::from_der(&assertion.signature).map_err(|_| WebAuthnError::Signature)?;
    let signed = [data.as_slice(), &sha256(&[&assertion.client_data_json])].concat();

    key.verify(&signed, &signature).map_err(|_| WebAuthnError::Signature)?;

    Ok(counter)
}

/// A software authenticator for tests (here and in the session tests): signs the way a browser's
/// passkey does, so the verifier is exercised against bytes it did not write itself.
#[cfg(test)]
pub mod fake {
    use super::*;
    use crate::anywhere::crypto::Signer;
    use p256::ecdsa::signature::Signer as _;

    pub struct Authenticator {
        pub key: Signer,
        pub counter: u32,
        pub flags: u8,
    }

    impl Default for Authenticator {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Authenticator {
        pub fn new() -> Self {
            Self { key: Signer::generate(), counter: 0, flags: 0x05 }
        }

        pub fn public(&self) -> Vec<u8> {
            self.key.public()
        }

        pub fn assert(&mut self, rp: &RelyingParty, challenge: &[u8]) -> Assertion {
            self.counter += 1;

            let client = serde_json::json!({ "type": "webauthn.get", "challenge": b64u(challenge), "origin": rp.origin, "crossOrigin": false });
            let client_data_json = serde_json::to_vec(&client).unwrap();
            let mut data = sha256(&[rp.id.as_bytes()]).to_vec();

            data.push(self.flags);
            data.extend_from_slice(&self.counter.to_be_bytes());

            let signed = [data.as_slice(), &sha256(&[&client_data_json])].concat();
            let signature: p256::ecdsa::Signature = self.key.signing_key().sign(&signed);

            Assertion { authenticator_data: data, client_data_json, signature: signature.to_der().as_bytes().to_vec() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Authenticator;
    use super::*;

    fn rp() -> RelyingParty {
        RelyingParty::production()
    }

    #[test]
    fn a_good_assertion_verifies_and_the_counter_moves() {
        let mut key = Authenticator::new();
        let assertion = key.assert(&rp(), b"challenge-bytes");

        assert_eq!(verify(&rp(), &key.public(), 0, b"challenge-bytes", &assertion), Ok(1));
    }

    #[test]
    fn another_challenge_is_refused() {
        let mut key = Authenticator::new();
        let assertion = key.assert(&rp(), b"what the card showed");

        assert_eq!(verify(&rp(), &key.public(), 0, b"something else", &assertion), Err(WebAuthnError::Challenge));
    }

    #[test]
    fn another_origin_is_refused() {
        let mut key = Authenticator::new();
        let phishing = RelyingParty { id: "sdc.skilleddesk.com".into(), origin: "https://sdc-skilleddesk.example".into() };
        let assertion = key.assert(&phishing, b"c");

        assert_eq!(verify(&rp(), &key.public(), 0, b"c", &assertion), Err(WebAuthnError::Origin));
    }

    #[test]
    fn another_relying_party_id_is_refused() {
        let mut key = Authenticator::new();
        let other = RelyingParty { id: "evil.example".into(), origin: rp().origin };
        let assertion = key.assert(&other, b"c");

        assert_eq!(verify(&rp(), &key.public(), 0, b"c", &assertion), Err(WebAuthnError::RpIdHash));
    }

    #[test]
    fn a_passkey_that_did_not_verify_the_user_is_refused() {
        let mut key = Authenticator::new();

        key.flags = 0x01;

        let assertion = key.assert(&rp(), b"c");

        assert_eq!(verify(&rp(), &key.public(), 0, b"c", &assertion), Err(WebAuthnError::NotVerified));
    }

    #[test]
    fn a_counter_that_does_not_increase_is_refused() {
        let mut key = Authenticator::new();
        let assertion = key.assert(&rp(), b"c");

        assert_eq!(verify(&rp(), &key.public(), 1, b"c", &assertion), Err(WebAuthnError::Counter));
        assert_eq!(verify(&rp(), &key.public(), 5, b"c", &assertion), Err(WebAuthnError::Counter));
    }

    #[test]
    fn an_authenticator_that_never_counts_is_accepted_while_it_stays_at_zero() {
        let mut key = Authenticator::new();
        let mut assertion = key.assert(&rp(), b"c");

        /* Rewrite the counter to 0 and re-sign, as a platform authenticator without a counter does. */
        let len = assertion.authenticator_data.len();

        assertion.authenticator_data[len - 4..].copy_from_slice(&0_u32.to_be_bytes());

        let signed = [assertion.authenticator_data.as_slice(), &sha256(&[&assertion.client_data_json])].concat();
        let signature: p256::ecdsa::Signature = p256::ecdsa::signature::Signer::sign(key.key.signing_key(), &signed);

        assertion.signature = signature.to_der().as_bytes().to_vec();

        assert_eq!(verify(&rp(), &key.public(), 0, b"c", &assertion), Ok(0));
    }

    #[test]
    fn a_signature_by_another_key_is_refused() {
        let mut real = Authenticator::new();
        let mut thief = Authenticator::new();
        let assertion = thief.assert(&rp(), b"c");

        assert_eq!(verify(&rp(), &real.public(), 0, b"c", &assertion), Err(WebAuthnError::Signature));

        let _ = real.assert(&rp(), b"c");
    }

    #[test]
    fn a_tampered_client_data_breaks_the_signature() {
        let mut key = Authenticator::new();
        let mut assertion = key.assert(&rp(), b"c");

        /* Same meaning, different bytes: the signature covers the exact bytes. */
        assertion.client_data_json.push(b' ');

        assert_eq!(verify(&rp(), &key.public(), 0, b"c", &assertion), Err(WebAuthnError::Signature));
    }
}
