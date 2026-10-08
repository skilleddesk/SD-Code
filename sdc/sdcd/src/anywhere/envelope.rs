//! The action envelope and its hash (plan section 8, "Approval envelope").
//!
//! What the person sees on the phone is the envelope; what they sign is `action_hash`, the SHA-256 of
//! its canonical JSON. The daemon keeps its own copy of every pending request and recomputes the hash
//! when the answer arrives, so a relay (or a page served by one) that shows one thing and signs
//! another produces a hash the daemon does not have, and the answer is dropped.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::canonical;

/// The version of the envelope shape. Part of the hashed bytes, so it can never be downgraded.
pub const ENVELOPE_VERSION: u32 = 1;

/// How the effect of an action was estimated, so the card can say how much to trust it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlastRadius {
    pub files: u32,
    pub db_tables: u32,
    pub services: Vec<String>,
    /// Plain words for the cases the counts cannot express ("restarts nginx for every site").
    pub notes: Vec<String>,
}

/// Everything the card shows and the signature covers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionEnvelope {
    pub request_id: String,
    pub turn_id: String,
    pub session_id: String,
    /// `local`, or the SDC host id of the VPS the action runs on.
    pub host: String,
    pub cwd: String,
    /// `edit`, `run`, ... as the gate names it.
    pub action: String,
    /// What exactly would run: the command line, or the path being changed.
    pub target: String,
    pub args: Vec<String>,
    pub file_hashes: Map<String, Value>,
    /// The gate's risk word (`MUTATING`, `DANGEROUS`, ...).
    pub risk: String,
    pub blast_radius: BlastRadius,
    pub title: String,
    pub reason: String,
    /// A checkpoint id when a rewind can take the action back, else empty.
    pub rollback: String,
    /// Estimated cost in millionths of a dollar. Integer, because the canonical form has no floats.
    pub est_cost_micro_usd: u64,
    /// Milliseconds since the epoch after which the request is dead.
    pub expires_at: i64,
    /// Random per request, so two identical actions never share a hash.
    pub nonce: String,
}

impl ActionEnvelope {
    /// The value that is canonicalised: the fields above plus the envelope version.
    pub fn to_value(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("an envelope always serialises");

        if let Value::Object(map) = &mut value {
            map.insert("v".to_string(), json!(ENVELOPE_VERSION));
        }

        value
    }

    /// The canonical JSON that is hashed.
    pub fn canonical(&self) -> Result<String, canonical::CanonicalError> {
        canonical::to_string(&self.to_value())
    }

    /// `action_hash`: SHA-256 of the canonical bytes.
    pub fn action_hash(&self) -> Result<[u8; 32], canonical::CanonicalError> {
        Ok(Sha256::digest(self.canonical()?.as_bytes()).into())
    }

    /// Whether the request is still alive at `now_ms`.
    pub fn is_live(&self, now_ms: i64) -> bool {
        now_ms < self.expires_at
    }
}

#[cfg(test)]
pub fn sample() -> ActionEnvelope {
    ActionEnvelope {
        request_id: "apr_perm-turn-3-1".into(),
        turn_id: "turn-3".into(),
        session_id: "s1".into(),
        host: "client-vps-1".into(),
        cwd: "/var/www/shop".into(),
        action: "run".into(),
        target: "pnpm prisma migrate deploy".into(),
        args: vec!["pnpm".into(), "prisma".into(), "migrate".into(), "deploy".into()],
        file_hashes: {
            let mut map = Map::new();

            map.insert("prisma/schema.prisma".into(), json!("blake3:00"));

            map
        },
        risk: "DANGEROUS".into(),
        blast_radius: BlastRadius { files: 0, db_tables: 2, services: vec![], notes: vec![] },
        title: "Run a migration".into(),
        reason: "The checkout page needs the new column.".into(),
        rollback: "chk_812".into(),
        est_cost_micro_usd: 40_000,
        expires_at: 1_900_000_000_000,
        nonce: "AAAAAAAAAAAAAAAAAAAAAA".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_stable() {
        let one = sample().action_hash().unwrap();
        let two = sample().action_hash().unwrap();

        assert_eq!(one, two);
    }

    #[test]
    fn one_changed_character_changes_the_hash() {
        let original = sample();
        let mut changed = sample();

        changed.target.push('x');

        assert_ne!(original.action_hash().unwrap(), changed.action_hash().unwrap());

        let mut other_host = sample();

        other_host.host = "client-vps-2".into();

        assert_ne!(original.action_hash().unwrap(), other_host.action_hash().unwrap(), "the host is part of what is signed");
    }

    #[test]
    fn the_nonce_makes_identical_actions_differ() {
        let one = sample();
        let mut two = sample();

        two.nonce = "BBBBBBBBBBBBBBBBBBBBBB".into();

        assert_ne!(one.action_hash().unwrap(), two.action_hash().unwrap());
    }

    #[test]
    fn expiry_is_checked_against_the_clock_passed_in() {
        let envelope = sample();

        assert!(envelope.is_live(envelope.expires_at - 1));
        assert!(!envelope.is_live(envelope.expires_at));
    }
}

/// The vectors both implementations (this file and `web/src/crypto/canonical.ts`) must reproduce.
///
/// `protocol/remote-vectors.json` is committed. This test fails when the Rust output no longer matches it;
/// `UPDATE_VECTORS=1 cargo test vectors` rewrites it, and the TypeScript test then has to agree with the
/// new file or the two sides have drifted - which is exactly what a signature scheme cannot tolerate.
#[cfg(test)]
mod vectors {
    use super::*;

    fn cases() -> Vec<(&'static str, Value)> {
        vec![
            ("sorted keys, no whitespace", json!({ "b": 1, "a": [true, null, "x"], "c": { "z": 2, "y": 3 } })),
            ("utf-16 key order", json!({ "\u{fb33}": 1, "\u{1f600}": 2, "\u{0080}": 3, "a": 4 })),
            ("escapes", json!("a\"b\\c\n\t\u{08}\u{0c}\r\u{1f}\u{7f}é\u{2028}")),
            ("integers", json!([0, -5, 9007199254740991_i64, -9007199254740991_i64])),
            ("empty containers", json!({ "a": {}, "b": [], "c": "" })),
            ("nested arrays keep order", json!([[3, 2, 1], { "k": [ { "b": 1, "a": 2 } ] }])),
            ("non-latin text", json!({ "reason_localized": "ডাটাবেস migration চালানো হবে", "emoji": "🚀" })),
        ]
    }

    fn generated() -> Value {
        let mut canon = Vec::new();

        for (name, input) in cases() {
            let text = canonical::to_string(&input).unwrap();

            canon.push(json!({ "name": name, "input": input, "canonical": text, "sha256": hex::encode(Sha256::digest(text.as_bytes())) }));
        }

        let envelope = sample();

        let identity: Vec<u8> = std::iter::once(4_u8).chain((0..64).map(|n| n as u8)).collect();
        let device: Vec<u8> = std::iter::once(4_u8).chain((0..64).map(|n| (n as u8).wrapping_mul(3))).collect();
        let nonce32 = [7_u8; 32];
        let hash = [9_u8; 32];
        let derived = json!({
            "identity_public_hex": hex::encode(&identity),
            "device_public_hex": hex::encode(&device),
            "daemon_id": super::super::crypto::daemon_id(&identity),
            "fingerprint": super::super::crypto::fingerprint(&identity),
            "sas_token": "the-token",
            "sas_code": super::super::crypto::sas_code("the-token", &device, &identity),
            "pair_nonce": "AAAAAAAAAAAAAAAAAAAAAA",
            "pair_challenge_hex": hex::encode(super::super::crypto::pair_challenge("AAAAAAAAAAAAAAAAAAAAAA")),
            "unlock_nonce_hex": hex::encode(nonce32),
            "unlock_challenge_view_hex": hex::encode(super::super::core::unlock_challenge(super::super::session::Level::View, &nonce32)),
            "unlock_challenge_operate_hex": hex::encode(super::super::core::unlock_challenge(super::super::session::Level::Operate, &nonce32)),
            "decision_hash_hex": hex::encode(hash),
            "decision_allow_message_hex": hex::encode(super::super::router::decision_message(&hash, super::super::router::Decision::AllowOnce)),
            "decision_deny_message_hex": hex::encode(super::super::router::decision_message(&hash, super::super::router::Decision::Deny)),
            "hub_auth_message": String::from_utf8(super::super::core::hub_auth_message("NONCE", "DAEMONID")).unwrap(),
        });

        json!({
            "derived": derived,
            "note": "Generated by sdcd (UPDATE_VECTORS=1 cargo test vectors). The TypeScript side must reproduce every value.",
            "canonical": canon,
            "envelope": {
                "input": envelope.to_value(),
                "canonical": envelope.canonical().unwrap(),
                "action_hash_hex": hex::encode(envelope.action_hash().unwrap()),
                "action_hash_b64u": super::super::crypto::b64u(&envelope.action_hash().unwrap()),
            },
        })
    }

    fn path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("protocol").join("remote-vectors.json")
    }

    #[test]
    fn the_committed_vectors_match_what_this_code_produces() {
        let produced = generated();

        if std::env::var("UPDATE_VECTORS").is_ok() {
            std::fs::write(path(), serde_json::to_string_pretty(&produced).unwrap() + "\n").unwrap();

            return;
        }

        let committed: Value = serde_json::from_str(&std::fs::read_to_string(path()).expect("protocol/remote-vectors.json is committed")).unwrap();

        assert_eq!(committed, produced, "the canonical JSON or the envelope changed; rerun with UPDATE_VECTORS=1 and fix the TypeScript side too");
    }
}
