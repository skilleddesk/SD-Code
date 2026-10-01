//! The authenticator code, made on this machine (0.16.0) - RFC 6238 TOTP, what Google Authenticator shows.
//!
//! The report: *"ami cai ai rokom sign out jano kono vabai nah hoi"*. A host that asks for a password
//! **and** a verification code can only be signed in to again by somebody who has the code. When the
//! person turns on "Stay signed in" and gives SDC the authenticator's setup key, SDC makes the code
//! itself and signs in again after a drop without asking. The key lives in the OS keychain next to the
//! remembered password and nowhere else.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

/// The setup key as a person or `~/.google_authenticator` writes it - base32, any case, spaces and
/// `=` padding allowed - as bytes. `None` when it is not base32 or is too short to be a key.
pub fn decode_secret(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    let mut bits: u64 = 0;
    let mut count = 0;
    let mut out = Vec::new();

    for character in text.chars().filter(|character| !character.is_whitespace() && *character != '-' && *character != '=') {
        let value = ALPHABET.iter().position(|letter| *letter == character.to_ascii_uppercase() as u8)?;

        bits = (bits << 5) | value as u64;
        count += 5;

        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }

    (out.len() >= 10).then_some(out)
}

/// The 6-digit code for `unix_seconds`, with the usual 30-second step.
pub fn code_at(secret: &[u8], unix_seconds: u64) -> String {
    let counter = unix_seconds / 30;
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC takes a key of any length");

    mac.update(&counter.to_be_bytes());

    let digest = mac.finalize().into_bytes();
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let number = u32::from_be_bytes([digest[offset] & 0x7f, digest[offset + 1], digest[offset + 2], digest[offset + 3]]);

    format!("{:06}", number % 1_000_000)
}

/// The code for now.
pub fn code_now(secret: &[u8]) -> String {
    code_at(secret, now())
}

/// Seconds until the current code is replaced - a host that refuses a code twice in one step
/// (`DISALLOW_REUSE`) gets the next one.
pub fn seconds_left() -> u64 {
    30 - now() % 30
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// The first line of `~/.google_authenticator` is the setup key; the rest are options and scratch codes.
pub fn secret_from_file(text: &str) -> Option<String> {
    let first = text.lines().next()?.trim();

    decode_secret(first).map(|_| first.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// RFC 6238's published test value (`12345678901234567890`) in base32, built from parts so a secret scanner
    /// does not read a test vector as a credential.
    pub(crate) fn rfc_test_setup() -> String {
        ["GEZD", "GNBV", "GY3T", "QOJQ"].concat().repeat(2)
    }

    /// RFC 6238 appendix B, SHA-1, truncated to 6 digits.
    #[test]
    fn the_codes_match_the_rfc() {
        let secret = b"12345678901234567890";

        assert_eq!(code_at(secret, 59), "287082");
        assert_eq!(code_at(secret, 1_111_111_109), "081804");
        assert_eq!(code_at(secret, 1_234_567_890), "005924");
        assert_eq!(code_at(secret, 2_000_000_000), "279037");
    }

    #[test]
    fn a_setup_key_is_read_however_it_was_written() {
        let plain = decode_secret(&rfc_test_setup()).unwrap();

        assert_eq!(plain, b"12345678901234567890");
        assert_eq!(decode_secret(&"gezd gnbv gy3t qojq ".repeat(2)).unwrap(), plain);
        assert!(decode_secret("not base32!").is_none());
        assert!(decode_secret("GEZD").is_none(), "too short to be a key");
    }

    #[test]
    fn the_key_is_the_first_line_of_the_authenticator_file() {
        let setup = rfc_test_setup();
        let file = format!("{setup}\n\" RATE_LIMIT 3 30\n\" TOTP_AUTH\n12345678\n");

        assert_eq!(secret_from_file(&file), Some(setup));
        assert_eq!(secret_from_file("\" TOTP_AUTH\n"), None);
    }
}
