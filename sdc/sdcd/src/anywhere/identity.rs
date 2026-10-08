//! The daemon's identity: an ECDSA key that signs `welcome`s and an X25519 key that `hello`s are sealed to.
//!
//! The private halves are kept in the OS keychain (`auth::keychain`) under fixed names and nowhere else;
//! the SQLite file never holds them. DESIGN.md OQ-7: if the keychain backend is the file fallback, SDC
//! Anywhere refuses to turn on unless the person has explicitly accepted a file-protected key, and the
//! storage kind is shown in Settings.

use crate::auth::keychain;

use super::crypto::{b64u, daemon_id, fingerprint, from_b64u, KemKeys, Signer};

const SIGN_NAME: &str = "sdc.anywhere.identity";
const KEM_NAME: &str = "sdc.anywhere.kem";

/// Where the private keys live, in words the Settings page can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Storage {
    /// `os` (DPAPI, Keychain, Secret Service) or `file`.
    pub backend: &'static str,
    /// For the file store: `acl`, `mode` or `none`.
    pub protection: &'static str,
}

pub fn storage() -> Storage {
    Storage { backend: keychain::backend(), protection: keychain::protection() }
}

/// Whether the keys may be created here: the OS store is reachable, or the person accepted a file.
pub fn storage_allowed(accepted_file_key: bool) -> bool {
    storage().backend == "os" || accepted_file_key
}

pub struct Identity {
    pub signer: Signer,
    pub kem: KemKeys,
}

impl Identity {
    /// Loads the keys, creating them on first use.
    pub fn load_or_create() -> Result<Self, String> {
        if let (Some(sign), Some(kem)) = (keychain::get(SIGN_NAME), keychain::get(KEM_NAME)) {
            let signer = from_b64u(&sign).ok().and_then(|bytes| Signer::from_secret(&bytes).ok());
            let kem = from_b64u(&kem).ok().and_then(|bytes| KemKeys::from_secret(&bytes).ok());

            if let (Some(signer), Some(kem)) = (signer, kem) {
                return Ok(Self { signer, kem });
            }
        }

        let identity = Self { signer: Signer::generate(), kem: KemKeys::generate() };

        keychain::set(SIGN_NAME, &b64u(&identity.signer.secret())).map_err(|error| error.message)?;
        keychain::set(KEM_NAME, &b64u(&identity.kem.secret())).map_err(|error| error.message)?;

        Ok(identity)
    }

    /// Replaces both keys. Every paired device must pair again, because they pinned the old identity.
    pub fn reset() -> Result<(), String> {
        keychain::delete(SIGN_NAME).map_err(|error| error.message)?;
        keychain::delete(KEM_NAME).map_err(|error| error.message)
    }

    pub fn public(&self) -> Vec<u8> {
        self.signer.public()
    }

    pub fn id(&self) -> String {
        daemon_id(&self.public())
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, because the test keychain is process-wide and two tests resetting it would race.
    #[test]
    fn the_identity_is_stable_until_it_is_reset() {
        let _ = Identity::reset();

        let first = Identity::load_or_create().unwrap();
        let second = Identity::load_or_create().unwrap();

        assert_eq!(first.public(), second.public());
        assert_eq!(first.kem.public(), second.kem.public());
        assert_eq!(first.id(), second.id());

        Identity::reset().unwrap();

        let third = Identity::load_or_create().unwrap();

        assert_ne!(first.public(), third.public());
    }
}
