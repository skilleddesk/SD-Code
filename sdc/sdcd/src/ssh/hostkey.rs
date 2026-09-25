//! The host key: the one security decision a remote connection asks a person to make (0.7.13).
//!
//! `ssh`'s own answer to "is this the machine I think it is" is `known_hosts`, and the previous
//! release did not use it: `host.add` probed with `StrictHostKeyChecking=accept-new` against the
//! **user's** file, so the first key ever seen was trusted for ever, a *changed* key was invisible,
//! and nothing ever showed a fingerprint. That is the difference between a pin and a formality, and
//! it is the layer VS Code gets right: it refuses a changed host key and asks the person to verify
//! the fingerprint (`docs/REMOTE.md` §1).
//!
//! So this module keeps three small things, in this order:
//!
//! 1. **scan** - the key the machine presents, without authenticating. `ssh-keyscan` does exactly
//!    that (a key exchange and nothing else); where it is not installed, a throwaway handshake into
//!    a **temporary** known-hosts file does the same and the file is deleted.
//! 2. **fingerprint** - `SHA256:<base64 of the SHA-256 of the key blob>`, the string OpenSSH prints,
//!    so what a person sees in SDC can be compared with `ssh-keygen -lf` in their own terminal.
//! 3. **pin** - the line goes into SDC's own `known_hosts` (`<data>/ssh/known_hosts`, `0600`, in a
//!    `0700` directory), and every later connection is checked against it by `ssh` itself, because
//!    `Ssh::base_args` passes `StrictHostKeyChecking=yes` and that file.
//!
//! A pin is **not** removed by `host.remove`. Deleting a row is not a security decision: re-adding
//! the same machine must find the same key, and if it presents a different one that is the alarm this
//! entire module exists for.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::auth::remote::SshTarget;
use crate::sdcp::envelope::ErrorObject;

/// One key a machine presented, as it appears in a `known_hosts` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    /// `ssh-ed25519`, `ssh-rsa`, `ecdsa-sha2-nistp256`.
    pub key_type: String,
    /// The key blob, base64, as `known_hosts` carries it.
    pub base64: String,
    /// `SHA256:…` - the string a person compares.
    pub fingerprint: String,
    /// The whole line, ready to be written to `known_hosts`.
    pub line: String,
}

/// What is already pinned for a host, against what it presents now.
#[derive(Debug, Clone)]
pub enum Trust {
    /// A pin exists and one of the presented keys matches it. The connection may proceed.
    Pinned(HostKey),
    /// Nothing is pinned yet - the fingerprints a person has to decide about.
    Unknown(Vec<HostKey>),
    /// A pin exists and the machine presents something else. Nothing may proceed.
    Changed { pinned: Vec<String>, seen: Vec<HostKey> },
}

/// `<data>/ssh/known_hosts` - SDC's own pins, and only SDC's.
pub fn known_hosts_path() -> Result<PathBuf, ErrorObject> {
    let dir = crate::paths::data_dir()
        .map_err(ErrorObject::internal)?
        .join("ssh");

    std::fs::create_dir_all(&dir).map_err(|error| ErrorObject::internal(format!("{}: {error}", dir.display())))?;

    /* 0700 on the directory: the pins are not secret, but they are SDC's own decisions and nobody
       else's business. Unix only - `set_permissions` with a mode is not a thing on Windows, where
       the ACL of `%APPDATA%` already keeps it per-user. */
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }

    Ok(dir.join("known_hosts"))
}

/// `SHA256:<base64>`, the fingerprint OpenSSH prints for a key blob.
///
/// OpenSSH hashes the **decoded key blob**, not the base64 text of it, and it prints the digest
/// without the `=` padding - so a fingerprint shown here can be compared, character for character,
/// with what `ssh-keygen -lf <(ssh-keyscan host)` prints in the person's own terminal.
pub fn fingerprint(blob_base64: &str) -> String {
    let mut digest = Sha256::new();

    /* A malformed entry (a hand-edited `known_hosts`, a key type this build does not know) still
       produces a deterministic string rather than a panic: it simply matches nothing. */
    match base64_decode(blob_base64) {
        Some(bytes) => digest.update(&bytes),
        None => digest.update(blob_base64.as_bytes()),
    }

    format!("SHA256:{}", base64(&digest.finalize()).trim_end_matches('='))
}

/// base64 decoding, the inverse of [`base64`]. `None` for anything that is not base64.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u32;

    for character in text.chars() {
        let value = match character {
            'A'..='Z' => character as u32 - 'A' as u32,
            'a'..='z' => character as u32 - 'a' as u32 + 26,
            '0'..='9' => character as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => break,
            '\n' | '\r' => continue,
            _ => return None,
        };

        buffer = (buffer << 6) | value;
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }

    Some(out)
}

/// base64, standard alphabet with padding - twenty lines, so that a fingerprint needs no dependency.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::new();

    for chunk in bytes.chunks(3) {
        let window = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let packed = ((window[0] as u32) << 16) | ((window[1] as u32) << 8) | window[2] as u32;

        out.push(ALPHABET[(packed >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(packed >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(packed >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(packed & 63) as usize] as char } else { '=' });
    }

    out
}

/// One `known_hosts` line, split. `None` for a comment, a blank line or a hashed entry (`|1|…`),
/// which this daemon never writes and cannot interpret.
fn parse_line(line: &str) -> Option<(String, HostKey)> {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('|') {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    let field = parts.next()?.to_string();
    let key_type = parts.next()?.to_string();
    let base64_blob = parts.next()?.to_string();

    Some((
        field,
        HostKey {
            fingerprint: fingerprint(&base64_blob),
            key_type,
            base64: base64_blob,
            line: trimmed.to_string(),
        },
    ))
}

/// Every key in a `known_hosts` text, with the host field it belongs to.
fn parse_known_hosts(text: &str) -> Vec<(String, HostKey)> {
    text.lines().filter_map(parse_line).collect()
}

/// The host part of a `user@host` target - what `known_hosts` and `ssh-keyscan` name.
///
/// This is not cosmetic: `ssh-keyscan git@github.com` asks DNS for a machine *called* `git@github.com`
/// and fails with `getaddrinfo`, and a `known_hosts` field is `[host]:port`, never `[user@host]:port`.
/// A connection names the user; the machine's identity does not include one.
pub fn host_of(user_host: &str) -> String {
    user_host.rsplit('@').next().unwrap_or(user_host).to_string()
}

/// The fields `ssh` looks a host up under: `host` on the default port, `[host]:port` otherwise.
///
/// Both are returned even for the default port, because `ssh-keyscan -p 22` writes the bracketed form
/// on some builds and the plain one on others, and a pin that is present must be *found* rather than
/// asked for again.
pub fn fields_for(target: &SshTarget) -> Vec<String> {
    let host = host_of(&target.user_host);

    match target.port {
        Some(port) if port != 22 => vec![format!("[{host}]:{port}"), format!("{host}:{port}")],
        Some(port) => vec![host.clone(), format!("[{host}]:{port}"), format!("{host}:{port}")],
        None => vec![host],
    }
}

/// The keys already pinned for a host, read from a `known_hosts` file. `path` is a parameter so the
/// rule can be tested without touching the machine's own data directory.
pub fn pinned_in(path: &std::path::Path, target: &SshTarget) -> Result<Vec<HostKey>, ErrorObject> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        /* No file yet is not a failure: it is a host nothing has been pinned for. */
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(ErrorObject::internal(format!("{}: {error}", path.display()))),
    };
    let fields = fields_for(target);

    Ok(parse_known_hosts(&text)
        .into_iter()
        .filter(|(field, _)| fields.iter().any(|candidate| candidate == field))
        .map(|(_, key)| key)
        .collect())
}

/// Writes the keys into a `known_hosts` file, once each. Returns how many lines were added.
///
/// The file is created `0600` on unix: the pins are not secret, but a pin file a second user can edit
/// is a connection somebody else can redirect.
pub fn pin_into(path: &std::path::Path, keys: &[HostKey]) -> Result<usize, ErrorObject> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut added = 0_usize;
    let mut text = existing.clone();

    for key in keys {
        if existing.lines().any(|line| line.trim() == key.line) {
            continue;
        }

        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }

        text.push_str(&key.line);
        text.push('\n');
        added += 1;
    }

    if added > 0 {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| ErrorObject::internal(format!("{}: {error}", parent.display())))?;
        }

        std::fs::write(path, text).map_err(|error| ErrorObject::internal(format!("{}: {error}", path.display())))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }

    Ok(added)
}

/// The keys a machine presents right now, without authenticating.
///
/// Two ways to ask, and the order matters: `ssh-keyscan` performs a key exchange and **nothing else**,
/// which is exactly the property this needs - the fingerprint is decided before a password or a key is
/// ever offered. Where `ssh-keyscan` is missing or answers nothing, a throwaway handshake into a
/// **temporary** pin file does the same job, and the file is deleted either way.
pub fn scan(target: &SshTarget) -> Result<Vec<HostKey>, ErrorObject> {
    let keyscan = scan_with("ssh-keyscan", target);

    if let Ok(keys) = &keyscan {
        if !keys.is_empty() {
            return Ok(keys.clone());
        }
    }

    match scan_by_handshake(target) {
        Ok(keys) if !keys.is_empty() => Ok(keys),
        Ok(_) => Err(ErrorObject::bad_request(format!(
            "{} did not present a host key: neither `ssh-keyscan` nor a handshake answered with one.{}",
            target.user_host,
            super::port_hint(target)
        ))),
        /*
         * Both failed. The sentence is the **handshake's**, because that call is a real `ssh` connection:
         * `Connection refused`, `Connection timed out` and `Permission denied` are its words, and they are
         * what a person can act on. `ssh-keyscan` can fail where a real `ssh` succeeds - against OpenSSH
         * 10.2 on Ubuntu it prints `choose_kex: unsupported KEX method …` while `ssh` completes the same
         * exchange - so reporting it first would send somebody to fix a client that is working. It is
         * appended when it says something *different*, because on the hosts where it is the real problem
         * (a fingerprint wanted without offering a key) that is worth knowing.
         */
        Err(handshake) => {
            let message = match keyscan {
                Err(keyscan_error) if !keyscan_error.message.contains(&handshake.message) => format!(
                    "{} (`ssh-keyscan` said: {})",
                    handshake.message, keyscan_error.message
                ),
                _ => handshake.message,
            };

            Err(ErrorObject::bad_request(format!("{message}{}", super::port_hint(target))))
        }
    }
}

/// `ssh-keyscan -p <port> -T 10 <host>`, whose stdout *is* `known_hosts` lines.
fn scan_with(program: &str, target: &SshTarget) -> Result<Vec<HostKey>, ErrorObject> {
    let Some(mut command) = crate::host::program::command(program) else {
        return Err(ErrorObject::not_found(format!("`{program}` is not on this machine")));
    };

    let mut args: Vec<String> = Vec::new();

    if let Some(port) = target.port {
        args.push("-p".to_string());
        args.push(port.to_string());
    }

    args.push("-T".to_string());
    args.push("10".to_string());
    /* The **host**, not `user@host`: `ssh-keyscan` resolves whatever it is given, and a user name in
       front of a hostname makes DNS look for a machine called `git@github.com` (verified against
       OpenSSH 9.5: `getaddrinfo git@github.com: A non-recoverable error`). */
    args.push(host_of(&target.user_host));

    let output = command
        .args(&args)
        .output()
        .map_err(|error| ErrorObject::internal(format!("`{program}` could not be started: {error}")))?;
    let keys: Vec<HostKey> = parse_known_hosts(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .map(|(_, key)| key)
        .collect();

    if keys.is_empty() {
        return Err(ErrorObject::bad_request(format!(
            "{} did not present a host key: {}",
            target.user_host,
            reason_line(&String::from_utf8_lossy(&output.stderr))
        )));
    }

    Ok(keys)
}

/// The fallback: one handshake into a temporary pin file, read, deleted.
///
/// This is the only `ssh` in the daemon that does **not** use [`super::Ssh::base_args`], and for a
/// good reason: it is how a pin is *discovered*, so it must not require one. It authenticates with
/// `PreferredAuthentications=none`, so no key and no password is ever offered - the key exchange
/// happens, the host key goes into the throwaway file, and the connection is dropped.
fn scan_by_handshake(target: &SshTarget) -> Result<Vec<HostKey>, ErrorObject> {
    let Some(mut command) = crate::host::program::command("ssh") else {
        return Err(ErrorObject::not_found("`ssh` is not on this machine's PATH"));
    };

    let scratch = std::env::temp_dir().join(format!("sdc-scan-{}.known_hosts", uuid::Uuid::new_v4()));
    let mut args = target.port_args();

    for option in [
        "BatchMode=yes",
        "ConnectTimeout=10",
        "StrictHostKeyChecking=accept-new",
        "PreferredAuthentications=none",
    ] {
        args.push("-o".to_string());
        args.push(option.to_string());
    }

    args.push("-o".to_string());
    args.push(format!("UserKnownHostsFile={}", scratch.display()));
    args.push(target.user_host.clone());
    args.push("true".to_string());

    let output = command.args(&args).output();
    let keys: Vec<HostKey> = std::fs::read_to_string(&scratch)
        .map(|text| parse_known_hosts(&text).into_iter().map(|(_, key)| key).collect())
        .unwrap_or_default();

    let _ = std::fs::remove_file(&scratch);

    if keys.is_empty() {
        let reason = output
            .ok()
            .map(|output| reason_line(&String::from_utf8_lossy(&output.stderr)))
            .unwrap_or_else(|| "no answer".to_string());

        return Err(ErrorObject::bad_request(format!(
            "{} did not present a host key: {reason}",
            target.user_host
        )));
    }

    Ok(keys)
}

/// The line of an `ssh-keyscan` transcript that actually says something.
///
/// `first_line` is not good enough for `ssh-keyscan`, and this was a real sentence a person read:
///
/// ```text
/// deploy@203.0.113.10:8443 did not present a host key: # 203.0.113.10:8443
/// ```
///
/// The first line of its stderr is its own **header comment** (`# host:port`), the second and third are
/// the server's banner, and the reason - `choose_kex: unsupported KEX method …` - is last. So the header
/// comments and the banner go, and what is left is taken from the end, where a reason normally is.
fn reason_line(text: &str) -> String {
    let useful: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with('#'))
        .filter(|line| !line.starts_with("SSH-2.0-"))
        .collect();

    useful
        .last()
        .map(|line| line.to_string())
        .unwrap_or_else(|| "no answer".to_string())
}




/// What is pinned for this host, against what it presents now.
pub fn inspect(target: &SshTarget) -> Result<Trust, ErrorObject> {
    let seen = scan(target)?;
    let pinned = pinned_in(&known_hosts_path()?, target)?;

    if pinned.is_empty() {
        return Ok(Trust::Unknown(seen));
    }

    if let Some(key) = seen.iter().find(|key| pinned.iter().any(|pinned| pinned.base64 == key.base64)) {
        return Ok(Trust::Pinned(key.clone()));
    }

    Ok(Trust::Changed {
        pinned: pinned.iter().map(|key| key.fingerprint.clone()).collect(),
        seen,
    })
}

/// Confirms that one fingerprint a person accepted is still what the machine presents, and answers
/// with the key to pin.
///
/// The second scan is the point: a fingerprint is shown, a person reads it, and only then is it
/// stored. Trusting the earlier scan would leave a window in which the machine could have changed,
/// and the whole value of a pin is that what was *confirmed* is what is *stored*.
///
/// **Only the confirmed key is returned**, never every key the machine offers: pinning an RSA key
/// nobody looked at because the Ed25519 one was on screen would be a trust decision made by this
/// function on a person's behalf.
pub fn confirm(target: &SshTarget, fingerprint_wanted: &str) -> Result<Vec<HostKey>, ErrorObject> {
    let seen = scan(target)?;

    if !seen.iter().any(|key| key.fingerprint == fingerprint_wanted) {
        return Err(ErrorObject::bad_request(format!(
            "{} now presents {} - not the {} that was confirmed. Nothing was pinned: a key that changes while it is being checked is exactly the case a pin is for.",
            target.user_host,
            seen.iter().map(|key| key.fingerprint.clone()).collect::<Vec<_>>().join(", "),
            fingerprint_wanted
        )));
    }

    Ok(seen.into_iter().filter(|key| key.fingerprint == fingerprint_wanted).collect())
}

/// The fingerprint a window should show: the Ed25519 key where there is one (OpenSSH's own default),
/// then ECDSA, then RSA - so two machines' dialogs say the same thing.
pub fn primary(keys: &[HostKey]) -> Option<&HostKey> {
    ["ssh-ed25519", "ecdsa-sha2-nistp256", "ssh-rsa"]
        .iter()
        .find_map(|wanted| keys.iter().find(|key| key.key_type == *wanted))
        .or_else(|| keys.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::remote::parse_target;

    /// A real-shaped Ed25519 blob, for the tests that only need the string to be well formed.
    const SAMPLE: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIL6T0T1dJm6hxKt3fX1lTk7q0b9hIY5gWq1Y3f5e6d7a";

    #[test]
    fn the_fingerprint_is_of_the_decoded_key_and_has_no_padding() {
        /* `base64("abc")` is `YWJj`, and the digest is of the *decoded* bytes - the same value
           OpenSSH prints, with the `=` dropped. */
        let printed = fingerprint("YWJj");

        assert!(printed.starts_with("SHA256:"), "{printed}");
        assert!(!printed.contains('='), "{printed}");
        assert_eq!(printed.len(), "SHA256:".len() + 43, "{printed}");

        /* The shapes that must not panic - an empty entry, and something that is not base64 at all. */
        assert!(fingerprint("").starts_with("SHA256:"));
        assert!(fingerprint("not base64!").starts_with("SHA256:"));
        assert_eq!(fingerprint(SAMPLE), fingerprint(SAMPLE));
    }

    /// base64 both ways: the encoder and the decoder must agree, padding included.
    #[test]
    fn base64_encodes_and_decodes_what_it_should() {
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64_decode("YWJj").unwrap(), b"abc");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        assert_eq!(base64_decode("YWI=").unwrap(), b"ab");
        assert_eq!(base64_decode("YWJj\n").unwrap(), b"abc");
        assert_eq!(base64_decode("!!"), None);
    }

    #[test]
    fn a_known_hosts_line_is_read_for_the_host_and_the_port_it_names() {
        let line = "[vps.example]:8443 ssh-ed25519 AAAAB3NzaC1lZDI1NTE5AAAAI";
        let (field, key) = parse_line(line).expect("a line");

        assert_eq!(field, "[vps.example]:8443");
        assert_eq!(key.key_type, "ssh-ed25519");
        assert_eq!(key.line, line);
        assert!(key.fingerprint.starts_with("SHA256:"));

        /* Comments, blanks and hashed entries are ignored rather than misread. */
        assert!(parse_line("# a comment").is_none());
        assert!(parse_line("").is_none());
        assert!(parse_line("|1|abcdef=|xyz= ssh-ed25519 AAAA").is_none());
    }

    #[test]
    fn the_lookup_names_the_port_the_way_ssh_does() {
        let plain = parse_target("root@vps.example").unwrap();
        let other = parse_target("ssh -p 8443 root@vps.example").unwrap();

        /* The **host**, without the user: `ssh`'s `known_hosts` field is `[host]:port`, and
           `ssh-keyscan root@vps.example` fails in DNS with `getaddrinfo` - which this test caught when
           it still expected `root@vps.example`. */
        assert_eq!(fields_for(&plain), vec!["vps.example"]);
        assert_eq!(fields_for(&other), vec!["[vps.example]:8443", "vps.example:8443"]);
        assert_eq!(host_of("root@vps.example"), "vps.example");
        assert_eq!(host_of("vps.example"), "vps.example");
    }

    /// The pin: written once, read back for its own host, never for another.
    #[test]
    fn a_pin_is_written_once_and_read_back_only_for_its_own_host() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ssh/known_hosts");
        let mine = parse_target("ssh -p 8443 root@vps.example").unwrap();
        let other = parse_target("ssh -p 8443 root@other.example").unwrap();
        let line = "[vps.example]:8443 ssh-ed25519 AAAAB3NzaC1lZDI1NTE5AAAAI";
        let keys = vec![
            HostKey {
                key_type: "ssh-ed25519".into(),
                base64: "AAAAB3NzaC1lZDI1NTE5AAAAI".into(),
                fingerprint: fingerprint("AAAAB3NzaC1lZDI1NTE5AAAAI"),
                line: "[vps.example]:8443 ssh-rsa AAAAB3NzaC1yc2E".into(),
            },
            HostKey {
                key_type: "ssh-ed25519".into(),
                base64: "AAAAB3NzaC1lZDI1NTE5AAAAI".into(),
                fingerprint: fingerprint("AAAAB3NzaC1lZDI1NTE5AAAAI"),
                line: line.to_string(),
            },
        ];

        assert_eq!(pin_into(&file, &keys).unwrap(), 2, "both lines go in");
        assert_eq!(pin_into(&file, &keys).unwrap(), 0, "and a second call adds nothing");

        let pinned = pinned_in(&file, &mine).unwrap();

        assert_eq!(pinned.len(), 2);
        assert!(pinned_in(&file, &other).unwrap().is_empty(), "another host reads nothing");

        /* 0600 on unix: the file a connection decision is stored in is not for everyone. */
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;

            assert_eq!(mode, 0o600, "{mode:o}");
        }
    }

    /// The line of a failed `ssh-keyscan` that a person should read - **not** its header comment.
    ///
    /// The transcript is verbatim from the machine in the bug report (OpenSSH 10.2 on Ubuntu, port 8443),
    /// where `ssh-keyscan` failed and a real `ssh` to the same port completed the key exchange. Before this
    /// the daemon answered `… did not present a host key: # 203.0.113.10:8443` - its own header - which
    /// names neither the problem nor the fix.
    #[test]
    fn a_failed_scan_reports_the_reason_and_not_the_header() {
        let transcript = "# 203.0.113.10:8443\nSSH-2.0-OpenSSH_10.2p1\nUbuntu-2ubuntu3.6\n\
                          choose_kex: unsupported KEX method sntrup761x25519-sha512@openssh.com\n\
                          # 203.0.113.10:8443\nSSH-2.0-OpenSSH_10.2p1\nUbuntu-2ubuntu3.6\n\
                          choose_kex: unsupported KEX method sntrup761x25519-sha512@openssh.com\n";

        let reason = reason_line(transcript);

        assert!(reason.starts_with("choose_kex:"), "{reason}");
        assert!(!reason.contains('#'), "no header comment: {reason}");
        assert!(!reason.contains("SSH-2.0-"), "{reason}");

        /* `ssh`'s own failures are one line, and it is taken whole. */
        assert_eq!(
            reason_line("\nPermission denied (keyboard-interactive).\n"),
            "Permission denied (keyboard-interactive)."
        );
        assert_eq!(reason_line("   \n\n"), "no answer");
    }

    /// The one test that asks OpenSSH itself: a fingerprint is only useful if it is *the* fingerprint.
    /// Skipped where `ssh-keygen` is absent (the same rule the git tests use), because the point is to
    /// compare with the tool a person would compare against - and a build that printed its own
    /// fingerprints in its own format would be worse than no fingerprint at all.
    #[test]
    fn the_fingerprint_is_the_string_openssh_prints() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("sdc-probe_ed25519");
        let made = std::process::Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-C", "sdc-probe", "-f", &key.display().to_string()])
            .output();

        if made.is_err() {
            return;
        }

        let public = std::fs::read_to_string(dir.path().join("sdc-probe_ed25519.pub")).unwrap();
        let blob = public.split_whitespace().nth(1).expect("the key blob").to_string();
        let printed = std::process::Command::new("ssh-keygen")
            .args(["-lf", &dir.path().join("sdc-probe_ed25519.pub").display().to_string(), "-E", "sha256"])
            .output()
            .expect("ssh-keygen -lf");
        let text = String::from_utf8_lossy(&printed.stdout);
        let expected = text.split_whitespace().nth(1).unwrap_or_default().to_string();

        assert!(expected.starts_with("SHA256:"), "ssh-keygen printed {text}");
        assert_eq!(fingerprint(&blob), expected);
    }
}

