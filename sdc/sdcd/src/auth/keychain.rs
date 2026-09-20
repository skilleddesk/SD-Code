//! The OS keychain, with an honest fallback (master spec sections 9.10 and 17.4).
//!
//! "Keys are stored in the OS keychain, never on disk" is the promise the Provider Hub makes. This
//! module is where that promise is kept or, on a machine without a keychain, where it is *reported*
//! as not being kept:
//!
//! * with the `keychain` feature, `keyring` talks to DPAPI (Windows), the Keychain (macOS) or the
//!   Secret Service (Linux);
//! * without it, the fallback is a file under `<data>/keys/<name>` with 0600 permissions on unix,
//!   and `backend()` says `"file"` so the doctor can warn about it.
//!
//! Two things never happen here: a key is never written into the SQLite file, and a key is never
//! returned in an event. `mask()` is what a card shows, and it is the only transformation of a secret
//! that leaves this module.

use std::path::PathBuf;

use crate::sdcp::envelope::ErrorObject;

/// Which store is actually in use. Reported by `host.doctor` and by the Provider Hub's save toast.
pub fn backend() -> &'static str {
    if cfg!(feature = "keychain") {
        "os"
    } else {
        "file"
    }
}

fn key_path(name: &str) -> Result<PathBuf, ErrorObject> {
    let dir = crate::paths::data_dir().map_err(ErrorObject::internal)?.join("keys");

    std::fs::create_dir_all(&dir).map_err(ErrorObject::internal)?;

    Ok(dir.join(name.replace(['/', '\\'], "_")))
}

/// Stores a secret under a name. `sdc.provider.<id>` is the naming the provider backend uses.
pub fn set(name: &str, secret: &str) -> Result<(), ErrorObject> {
    if secret.is_empty() {
        return delete(name);
    }

    #[cfg(feature = "keychain")]
    {
        let entry = keyring::Entry::new("dev.skilleddesk.sdc", name)
            .map_err(|error| ErrorObject::internal(error.to_string()))?;

        entry
            .set_password(secret)
            .map_err(|error| ErrorObject::internal(error.to_string()))?;

        return Ok(());
    }

    #[cfg(not(feature = "keychain"))]
    {
        let path = key_path(name)?;

        std::fs::write(&path, secret).map_err(ErrorObject::internal)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }
}

/// Reads a secret, or `None` when there is not one.
pub fn get(name: &str) -> Option<String> {
    #[cfg(feature = "keychain")]
    {
        if let Ok(entry) = keyring::Entry::new("dev.skilleddesk.sdc", name) {
            if let Ok(secret) = entry.get_password() {
                return Some(secret);
            }
        }

        return None;
    }

    #[cfg(not(feature = "keychain"))]
    {
        let path = key_path(name).ok()?;

        std::fs::read_to_string(path).ok().map(|secret| secret.trim().to_string())
    }
}

/// Removes a secret. A missing one is not an error.
pub fn delete(name: &str) -> Result<(), ErrorObject> {
    #[cfg(feature = "keychain")]
    {
        if let Ok(entry) = keyring::Entry::new("dev.skilleddesk.sdc", name) {
            let _ = entry.delete_credential();
        }

        return Ok(());
    }

    #[cfg(not(feature = "keychain"))]
    {
        let path = key_path(name)?;

        if path.exists() {
            std::fs::remove_file(path).map_err(ErrorObject::internal)?;
        }

        Ok(())
    }
}

/// The masked label a card shows: `sk-…4f8a`. Never the secret, and short enough to read.
pub fn mask(secret: &str) -> String {
    let trimmed = secret.trim();

    if trimmed.len() <= 8 {
        return "••••".to_string();
    }

    format!("{}…{}", &trimmed[..4], &trimmed[trimmed.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct names per test: the suite runs in parallel, and one test deleting another's entry
    /// would be a flake that only shows up on a busy machine.
    const ROUND_TRIP_KEY: &str = "sdc.test.keychain-round-trip";
    const EMPTY_KEY: &str = "sdc.test.keychain-empty";

    #[test]
    fn round_trips_a_secret_and_reports_which_store_it_used() {
        set(ROUND_TRIP_KEY, "sk-test-1234").unwrap();

        assert_eq!(get(ROUND_TRIP_KEY).as_deref(), Some("sk-test-1234"));
        assert!(matches!(backend(), "os" | "file"));

        delete(ROUND_TRIP_KEY).unwrap();
        assert!(get(ROUND_TRIP_KEY).is_none());
    }

    #[test]
    fn masks_a_key_without_revealing_it() {
        assert_eq!(mask("sk-ant-api03-4f8a"), "sk-a…4f8a");
        assert_eq!(mask("short"), "••••");
    }

    #[test]
    fn an_empty_secret_deletes_the_entry() {
        set(EMPTY_KEY, "sk-test-1234").unwrap();
        set(EMPTY_KEY, "").unwrap();

        assert!(get(EMPTY_KEY).is_none());
    }
}
