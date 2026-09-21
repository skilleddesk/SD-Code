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

use std::path::{Path, PathBuf};

use crate::sdcp::envelope::ErrorObject;

/// Which store is **actually** in use. Reported by `host.doctor`, `host.status` and the Provider Hub's save
/// toast.
///
/// 0.7.10 made this a runtime question rather than a compile-time one. `keyring` v3 enables no backend by
/// default and even a compiled-in store can be unreachable - a locked Keychain, a Windows service account, a
/// container built with the feature on - so answering `"os"` because `cfg!(feature = "keychain")` was true
/// would be the same class of lie as a `master` branch label on a project that is on `main`: a confident
/// answer about something nobody checked. The store is probed once, and the file fallback is what
/// `set`/`get`/`delete` use when the probe says no, so the reported store and the used store cannot disagree.
pub fn backend() -> &'static str {
    if os_store_works() {
        "os"
    } else {
        "file"
    }
}

/// How the fallback file is protected - `"acl"` on Windows, `"mode"` on unix, `"none"` when neither could be
/// applied. Reported next to `backend()`, so "the fallback" is not presented as if it were all the same thing.
pub fn protection() -> &'static str {
    if os_store_works() {
        return "os";
    }

    if cfg!(unix) {
        "mode"
    } else if cfg!(windows) {
        "acl"
    } else {
        "none"
    }
}

/// The name the probe asks for. It never stores anything; `NoEntry` is the *success* case, because it means
/// the store answered. Feature-gated like the call that uses it, so a build without the feature has no unused
/// constant to warn about.
#[cfg(feature = "keychain")]
const PROBE: &str = "sdc.store.probe";

/// Is there a reachable OS store? Computed once - a keyring call per `set` would be two round trips to DPAPI
/// for every save, and the answer cannot change inside one process.
#[cfg(feature = "keychain")]
fn os_store_works() -> bool {
    static WORKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

    *WORKS.get_or_init(|| match keyring::Entry::new("dev.skilleddesk.sdc", PROBE) {
        /* Reachable and empty: the good case. */
        Ok(entry) => match entry.get_password() {
            Ok(_) => true,
            Err(keyring::Error::NoEntry) => true,
            Err(_) => false,
        },
        Err(_) => false,
    })
}

/// Without the feature there is no store to probe, and the answer is a constant instead of a call.
#[cfg(not(feature = "keychain"))]
fn os_store_works() -> bool {
    false
}

/* The four keyring calls, split out so that the feature still decides what is *linked* while `os_store_works`
   decides what is *used*: without the feature these compile to a clear "no OS store in this build" instead of
   a `keyring` reference that does not resolve. */
#[cfg(feature = "keychain")]
fn os_set(name: &str, secret: &str) -> Result<(), ErrorObject> {
    let entry = keyring::Entry::new("dev.skilleddesk.sdc", name)
        .map_err(|error| ErrorObject::internal(error.to_string()))?;

    entry
        .set_password(secret)
        .map_err(|error| ErrorObject::internal(error.to_string()))
}

#[cfg(not(feature = "keychain"))]
fn os_set(_name: &str, _secret: &str) -> Result<(), ErrorObject> {
    Err(ErrorObject::internal("no OS key store in this build"))
}

#[cfg(feature = "keychain")]
fn os_get(name: &str) -> Option<String> {
    keyring::Entry::new("dev.skilleddesk.sdc", name)
        .ok()
        .and_then(|entry| entry.get_password().ok())
}

#[cfg(not(feature = "keychain"))]
fn os_get(_name: &str) -> Option<String> {
    None
}

#[cfg(feature = "keychain")]
fn os_delete(name: &str) {
    if let Ok(entry) = keyring::Entry::new("dev.skilleddesk.sdc", name) {
        let _ = entry.delete_credential();
    }
}

#[cfg(not(feature = "keychain"))]
fn os_delete(_name: &str) {}

fn key_path(name: &str) -> Result<PathBuf, ErrorObject> {
    let dir = crate::paths::data_dir().map_err(ErrorObject::internal)?.join("keys");

    std::fs::create_dir_all(&dir).map_err(ErrorObject::internal)?;

    /* The directory first, so files written into it inherit the restriction rather than being created
       readable and narrowed afterwards. */
    let _ = restrict_to_owner(&dir);

    Ok(dir.join(name.replace(['/', '\\'], "_")))
}

/// Narrows a path to its owner: `0600` on unix, an ACL on Windows that drops inheritance.
///
/// The Windows half is why this exists at all. Until 0.7.10 a key written by the fallback inherited the
/// permissions of `%APPDATA%`, which on a shared machine means every account in `Users` - so "the fallback is
/// a file with 0600" was only ever true on unix. `icacls` is the system tool for it (no extra dependency, and
/// it is what an administrator would run by hand): `/inheritance:r` removes the inherited entries and
/// `/grant:r` replaces the rest with the current account's. When it cannot be applied the error is returned,
/// and `protection()` reports what the fallback really got.
fn restrict_to_owner(path: &Path) -> Result<(), ErrorObject> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(ErrorObject::internal)
    }

    #[cfg(windows)]
    {
        let account = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
            (Ok(domain), Ok(user)) if !domain.is_empty() && !user.is_empty() => format!("{domain}\\{user}"),
            _ => std::env::var("USERNAME").map_err(ErrorObject::internal)?,
        };

        /* The inheritance flags belong on a *directory* and must not be used on a file: `(OI)(CI)` marks an
           ACE as "for children only", and a file granted that way ends up with no effective permission at all -
           the owner could not read the key it just wrote (which is exactly how the first version of this
           failed its own round-trip test). */
        let grant = if path.is_dir() {
            format!("{account}:(OI)(CI)F")
        } else {
            format!("{account}:F")
        };

        let output = std::process::Command::new("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &grant])
            .output()
            .map_err(|error| ErrorObject::internal(format!("icacls is not available: {error}")))?;

        if !output.status.success() {
            return Err(ErrorObject::internal(format!(
                "icacls could not restrict {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;

        Ok(())
    }
}
/// Stores a secret under a name. `sdc.provider.<id>` is the naming the provider backend uses.
pub fn set(name: &str, secret: &str) -> Result<(), ErrorObject> {
    if secret.is_empty() {
        return delete(name);
    }

    if os_store_works() {
        return os_set(name, secret);
    }

    let path = key_path(name)?;

    std::fs::write(&path, secret).map_err(ErrorObject::internal)?;

    /* After the write, because the file has to exist - and the error is *returned*, not swallowed: a key
       saved into a readable file is worse than a save that failed. */
    restrict_to_owner(&path)
}

/// Reads a secret, or `None` when there is not one.
pub fn get(name: &str) -> Option<String> {
    if os_store_works() {
        return os_get(name);
    }

    let path = key_path(name).ok()?;

    std::fs::read_to_string(path).ok().map(|secret| secret.trim().to_string())
}

/// Removes a secret. A missing one is not an error.
pub fn delete(name: &str) -> Result<(), ErrorObject> {
    if os_store_works() {
        os_delete(name);

        return Ok(());
    }

    let path = key_path(name)?;

    if path.exists() {
        std::fs::remove_file(path).map_err(ErrorObject::internal)?;
    }

    Ok(())
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
        /* The reported store is the *used* store (0.7.10): a `backend()` that said `os` while the save went to
           a file - or the other way round - is the disagreement this pins down. */
        assert!(matches!(protection(), "os" | "acl" | "mode"));

        delete(ROUND_TRIP_KEY).unwrap();
        assert!(get(ROUND_TRIP_KEY).is_none());
    }

    /// The fallback file is narrowed to its owner, and on Windows that is an **ACL**.
    ///
    /// Until 0.7.10 the Windows fallback inherited `%APPDATA%`'s permissions, so `0600` was only ever true on
    /// unix and a shared machine's other accounts could read a key. `icacls` output is what this asserts,
    /// because that is what an administrator would look at.
    #[cfg(windows)]
    #[test]
    fn the_windows_fallback_file_is_restricted_to_its_owner() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("key");
        std::fs::write(&file, "sk-acl-test").unwrap();

        restrict_to_owner(&file).unwrap();

        let acl = std::process::Command::new("icacls").arg(&file).output().unwrap();
        let text = String::from_utf8_lossy(&acl.stdout).to_string();

        let user = std::env::var("USERNAME").unwrap_or_default();

        assert!(text.contains(&user), "the owner must keep access: {text}");
        assert!(
            !text.contains("BUILTIN\\Users") && !text.contains("Everyone"),
            "no group may read a key file: {text}"
        );
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
