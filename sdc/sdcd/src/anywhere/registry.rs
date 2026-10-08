//! The paired devices, the hello nonces already seen and the pairing tokens (SQLite, public data only).
//!
//! Nothing secret is stored here: a device's *public* signing key, its passkey's *public* key and
//! counter. The daemon's own private keys are in the keychain (`identity`).

use std::sync::Arc;

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::store::sqlite::Store;

use super::crypto::{b64u, from_b64u, sha256};

/// How long a hello nonce is remembered. Longer than the hello window, so a replay is caught for as long
/// as the hello would have been accepted.
pub const NONCE_TTL_MS: i64 = 10 * 60 * 1000;

/// A browser device the person has paired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub user_agent: String,
    /// SEC1 ECDSA P-256 public key of the device's non-extractable signing key.
    pub sign_pub: Vec<u8>,
    pub passkey_id: String,
    /// SEC1 public key of the passkey.
    pub passkey_pub: Vec<u8>,
    pub passkey_counter: u32,
    /// A guest session: View only, ends at `expires_at`, and the key is gone when the tab closes.
    pub guest: bool,
    pub created_at: i64,
    pub last_seen: Option<i64>,
    pub revoked_at: Option<i64>,
    pub expires_at: Option<i64>,
}

impl Device {
    /// Whether this device may open a session at `now_ms`.
    pub fn is_active(&self, now_ms: i64) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|end| now_ms < end)
    }
}

pub struct Registry {
    store: Arc<Store>,
}

const COLUMNS: &str =
    "id, name, user_agent, sign_pub, passkey_id, passkey_pub, passkey_counter, guest, created_at, last_seen, revoked_at, expires_at";

fn row_to_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    let decode = |text: String| from_b64u(&text).unwrap_or_default();

    Ok(Device {
        id: row.get(0)?,
        name: row.get(1)?,
        user_agent: row.get(2)?,
        sign_pub: decode(row.get(3)?),
        passkey_id: row.get(4)?,
        passkey_pub: decode(row.get(5)?),
        passkey_counter: row.get::<_, i64>(6)? as u32,
        guest: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
        last_seen: row.get(9)?,
        revoked_at: row.get(10)?,
        expires_at: row.get(11)?,
    })
}

impl Registry {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    pub fn add_device(&self, device: &Device) -> Result<()> {
        let connection = self.store.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO anywhere_devices (id, name, user_agent, sign_pub, passkey_id, passkey_pub, passkey_counter, guest, created_at, last_seen, revoked_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                device.id,
                device.name,
                device.user_agent,
                b64u(&device.sign_pub),
                device.passkey_id,
                b64u(&device.passkey_pub),
                device.passkey_counter as i64,
                device.guest as i64,
                device.created_at,
                device.last_seen,
                device.revoked_at,
                device.expires_at
            ],
        )?;

        Ok(())
    }

    pub fn device(&self, id: &str) -> Result<Option<Device>> {
        let connection = self.store.connection.lock().unwrap();

        Ok(connection
            .query_row(&format!("SELECT {COLUMNS} FROM anywhere_devices WHERE id = ?1"), params![id], row_to_device)
            .optional()?)
    }

    /// Every device, newest first, revoked ones included (the list shows them as revoked).
    pub fn list(&self) -> Result<Vec<Device>> {
        let connection = self.store.connection.lock().unwrap();
        let mut statement = connection.prepare(&format!("SELECT {COLUMNS} FROM anywhere_devices ORDER BY created_at DESC"))?;
        let rows = statement.query_map([], row_to_device)?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Marks a device revoked. Idempotent. `true` when the device exists.
    pub fn revoke(&self, id: &str, now_ms: i64) -> Result<bool> {
        let connection = self.store.connection.lock().unwrap();
        let changed = connection.execute(
            "UPDATE anywhere_devices SET revoked_at = COALESCE(revoked_at, ?2) WHERE id = ?1",
            params![id, now_ms],
        )?;

        Ok(changed > 0)
    }

    pub fn touch(&self, id: &str, now_ms: i64) -> Result<()> {
        let connection = self.store.connection.lock().unwrap();

        connection.execute("UPDATE anywhere_devices SET last_seen = ?2 WHERE id = ?1", params![id, now_ms])?;

        Ok(())
    }

    pub fn set_passkey_counter(&self, id: &str, counter: u32) -> Result<()> {
        let connection = self.store.connection.lock().unwrap();

        connection.execute("UPDATE anywhere_devices SET passkey_counter = ?2 WHERE id = ?1", params![id, counter as i64])?;

        Ok(())
    }

    /// Records a hello nonce. `false` when it was already there: the hello is a replay.
    pub fn claim_nonce(&self, nonce: &str, device: &str, now_ms: i64) -> Result<bool> {
        let connection = self.store.connection.lock().unwrap();

        connection.execute("DELETE FROM anywhere_nonces WHERE seen_at < ?1", params![now_ms - NONCE_TTL_MS])?;

        let inserted = connection.execute(
            "INSERT OR IGNORE INTO anywhere_nonces (nonce, device_id, seen_at) VALUES (?1, ?2, ?3)",
            params![nonce, device, now_ms],
        )?;

        Ok(inserted > 0)
    }

    /// Stores the hash of a pairing token. The token itself is only in the QR code.
    pub fn create_pairing(&self, token: &str, now_ms: i64, ttl_ms: i64, guest: bool) -> Result<()> {
        let connection = self.store.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO anywhere_pairings (token_hash, created_at, expires_at, guest) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash(token), now_ms, now_ms + ttl_ms, guest as i64],
        )?;

        Ok(())
    }

    /// Spends a pairing token: valid once, before it expires. Returns whether it was a guest token.
    pub fn consume_pairing(&self, token: &str, now_ms: i64) -> Result<Option<bool>> {
        let connection = self.store.connection.lock().unwrap();
        let hash = token_hash(token);
        let found: Option<(i64, i64, Option<i64>)> = connection
            .query_row(
                "SELECT expires_at, guest, used_at FROM anywhere_pairings WHERE token_hash = ?1",
                params![hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        match found {
            Some((expires_at, guest, None)) if now_ms < expires_at => {
                connection.execute("UPDATE anywhere_pairings SET used_at = ?2 WHERE token_hash = ?1", params![hash, now_ms])?;

                Ok(Some(guest != 0))
            }
            _ => Ok(None),
        }
    }
}

fn token_hash(token: &str) -> String {
    b64u(&sha256(&[b"sdc-anywhere/v1/pairing-token", token.as_bytes()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::new(Arc::new(Store::in_memory().unwrap()))
    }

    fn device(id: &str) -> Device {
        Device {
            id: id.into(),
            name: "Pixel".into(),
            user_agent: "Chrome".into(),
            sign_pub: vec![4; 65],
            passkey_id: "cred".into(),
            passkey_pub: vec![4; 65],
            passkey_counter: 0,
            guest: false,
            created_at: 1000,
            last_seen: None,
            revoked_at: None,
            expires_at: None,
        }
    }

    #[test]
    fn a_device_round_trips() {
        let registry = registry();

        registry.add_device(&device("d1")).unwrap();

        assert_eq!(registry.device("d1").unwrap().unwrap(), device("d1"));
        assert!(registry.device("nope").unwrap().is_none());
        assert_eq!(registry.list().unwrap().len(), 1);
    }

    #[test]
    fn a_revoked_device_is_not_active_and_stays_listed() {
        let registry = registry();

        registry.add_device(&device("d1")).unwrap();

        assert!(registry.device("d1").unwrap().unwrap().is_active(5000));
        assert!(registry.revoke("d1", 6000).unwrap());
        assert!(!registry.device("d1").unwrap().unwrap().is_active(7000));
        assert!(registry.revoke("d1", 9000).unwrap(), "revoking twice is harmless");
        assert_eq!(registry.device("d1").unwrap().unwrap().revoked_at, Some(6000), "the first revocation time is kept");
        assert!(!registry.revoke("ghost", 1).unwrap());
        assert_eq!(registry.list().unwrap().len(), 1);
    }

    #[test]
    fn a_guest_device_expires() {
        let mut guest = device("g1");

        guest.guest = true;
        guest.expires_at = Some(10_000);

        assert!(guest.is_active(9_999));
        assert!(!guest.is_active(10_000));
    }

    #[test]
    fn a_hello_nonce_can_be_claimed_once() {
        let registry = registry();

        assert!(registry.claim_nonce("n1", "d1", 1000).unwrap());
        assert!(!registry.claim_nonce("n1", "d1", 2000).unwrap(), "the same hello twice is a replay");
        assert!(registry.claim_nonce("n2", "d1", 2000).unwrap());
    }

    #[test]
    fn old_nonces_are_forgotten_only_after_the_window() {
        let registry = registry();

        assert!(registry.claim_nonce("n1", "d1", 1000).unwrap());
        assert!(!registry.claim_nonce("n1", "d1", 1000 + NONCE_TTL_MS - 1).unwrap());
        /* After the TTL the hello's own timestamp check rejects it, so forgetting is safe. */
        assert!(registry.claim_nonce("n1", "d1", 1000 + NONCE_TTL_MS + 1).unwrap());
    }

    #[test]
    fn a_nonce_survives_a_reopened_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sdc.db");

        {
            let registry = Registry::new(Arc::new(Store::open(&path).unwrap()));

            assert!(registry.claim_nonce("n1", "d1", 1000).unwrap());
        }

        let registry = Registry::new(Arc::new(Store::open(&path).unwrap()));

        assert!(!registry.claim_nonce("n1", "d1", 1500).unwrap(), "a restart must not reopen the replay window");
    }

    #[test]
    fn a_pairing_token_works_once_and_expires() {
        let registry = registry();

        registry.create_pairing("tok", 1000, 600_000, false).unwrap();

        assert_eq!(registry.consume_pairing("tok", 2000).unwrap(), Some(false));
        assert_eq!(registry.consume_pairing("tok", 2001).unwrap(), None, "single use");

        registry.create_pairing("late", 1000, 600_000, true).unwrap();

        assert_eq!(registry.consume_pairing("late", 1000 + 600_000).unwrap(), None, "expired");

        registry.create_pairing("guest", 1000, 600_000, true).unwrap();

        assert_eq!(registry.consume_pairing("guest", 1001).unwrap(), Some(true));
        assert_eq!(registry.consume_pairing("unknown", 1001).unwrap(), None);
    }

    #[test]
    fn the_token_itself_is_not_stored() {
        let registry = registry();

        registry.create_pairing("super-secret-token", 1000, 1000, false).unwrap();

        let connection = registry.store.connection.lock().unwrap();
        let stored: String = connection.query_row("SELECT token_hash FROM anywhere_pairings", [], |row| row.get(0)).unwrap();

        assert!(!stored.contains("super-secret-token"));
    }
}
