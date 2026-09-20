//! Getting into a VPS (0.7.0).
//!
//! The report this module answers, word for word: *"SSH connect korte gele ai rokom notification asbe -
//! akhono vps remotely connect korar jinis add hoi nai"*. The notification was the daemon's honest
//! sentence - `answered, but it asks for a password or a verification code - and SDC runs ssh without a
//! terminal, so it cannot type it` - and it was honest and useless at the same time: a VPS whose owner
//! logs in from a terminal every day was told to go and edit `authorized_keys` by hand.
//!
//! Three things happen here instead:
//!
//! * **the target is parsed**, not assumed. A person pastes what they type into their own shell -
//!   `ssh -p 8443 mehedi105117@109.199.108.216` - and the port comes out of it, so the probe dials the
//!   port they actually use. (Before this, that string was treated as a hostname, `-p` and all, and the
//!   port was never used: `ssh` was asked for `8443` as a host.)
//! * **SDC owns a key**, generated on first use under `~/.ssh/sdc_ed25519` - the same place `ssh-keygen`
//!   would put a key a person made for this purpose;
//! * **the password the user already has is used once**, through a real terminal this daemon *does*
//!   have: `ssh` runs on the daemon's PTY, its `password:` prompt is answered, and the key is appended
//!   to `authorized_keys`. Nothing is stored: not the password, and not the host's key material.
//!
//! A verification code (2FA) is the one case this cannot automate, and it says that instead of
//! pretending: a code is a *second* factor, and a daemon holding one would defeat it.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::pty::PtyManager;
use crate::sdcp::envelope::ErrorObject;

/// Printed by the install command so the daemon can tell "the key is in place" from "the shell
/// answered something".
pub const MARKER: &str = "SDC-KEY-INSTALLED";

/// An SSH target as the daemon understands one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    /// `user@host`, without a port and without the `ssh` command around it.
    pub user_host: String,
    /// The port, when the target named one.
    pub port: Option<u16>,
}

impl SshTarget {
    /// `-p PORT`, or nothing: the arguments every `ssh` call for this host shares.
    pub fn port_args(&self) -> Vec<String> {
        match self.port {
            Some(port) => vec!["-p".to_string(), port.to_string()],
            None => Vec::new(),
        }
    }
}

/// Parses what a person types for a host.
///
/// Accepted, because all four are things people paste:
///
/// ```text
/// mehedi105117@109.199.108.216
/// ssh mehedi105117@109.199.108.216
/// ssh -p 8443 mehedi105117@109.199.108.216
/// mehedi105117@109.199.108.216 -p 8443
/// ```
///
/// A bare hostname is an error rather than a guess: `ssh` needs a user to try, and inventing `root`
/// for somebody would be a change of meaning, not a convenience.
pub fn parse_target(input: &str) -> Result<SshTarget, String> {
    let mut words: Vec<String> = input
        .split_whitespace()
        .map(|word| word.trim().to_string())
        .filter(|word| !word.is_empty())
        .collect();

    if words.is_empty() {
        return Err("`target` is empty".to_string());
    }

    /* The `ssh` in front is part of the command, not part of the address. */
    if words[0].eq_ignore_ascii_case("ssh") {
        words.remove(0);
    }

    let mut port: Option<u16> = None;
    let mut user_host: Option<String> = None;
    let mut index = 0;

    while index < words.len() {
        let word = words[index].clone();

        if word == "-p" || word == "--port" {
            let value = words.get(index + 1).ok_or_else(|| format!("`{word}` needs a port after it"))?;

            port = Some(value.parse::<u16>().map_err(|error| format!("`{value}` is not a port: {error}"))?);
            index += 2;
            continue;
        }

        /* `-p8443` is how a shell user writes it when they are in a hurry. */
        if word.len() > 2 && word.starts_with("-p") {
            let value = &word[2..];

            port = Some(value.parse::<u16>().map_err(|error| format!("`{value}` is not a port: {error}"))?);
            index += 1;
            continue;
        }

        /* Flags `ssh` is commonly given that carry no address: dropped, so the address is what is left.
           An unknown flag is left alone, and the address check below is what reports it. */
        if word.starts_with('-') && matches!(word.as_str(), "-v" | "-q" | "-4" | "-6" | "-A" | "-T") {
            index += 1;
            continue;
        }

        if user_host.is_none() {
            user_host = Some(word);
        }

        index += 1;
    }

    let user_host = user_host.ok_or_else(|| format!("no `user@host` in `{input}`"))?;

    if !user_host.contains('@') {
        return Err(format!(
            "`{user_host}` has no user: write it as `user@host` (SDC cannot guess who to log in as)"
        ));
    }

    Ok(SshTarget { user_host, port })
}

/// The path of the key SDC uses for hosts it adds: `~/.ssh/sdc_ed25519`.
pub fn key_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;

    Some(PathBuf::from(home).join(".ssh").join("sdc_ed25519"))
}

/// The public half of the key, or `None` when there is no key yet.
pub fn public_key() -> Option<String> {
    let path = key_path()?;
    let text = std::fs::read_to_string(path.with_extension("pub")).ok()?;
    let line = text.lines().next().unwrap_or_default().trim().to_string();

    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

/// The public half, generated if it does not exist yet.
///
/// `ssh-keygen -t ed25519 -N ""` - the same command a person runs, so the key is a normal key: they can
/// see it with `ssh-add -l`, revoke it by deleting one line from `authorized_keys`, and use it from a
/// terminal too. No passphrase: a passphrase is something a *person* types, and an automated installer
/// that stored one would be storing a secret for no reason.
pub fn ensure_key() -> Result<String, String> {
    if let Some(existing) = public_key() {
        return Ok(existing);
    }

    let path = key_path().ok_or_else(|| "no home directory to put a key in".to_string())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }

    let Some(mut command) = crate::host::program::command("ssh-keygen") else {
        return Err("`ssh-keygen` is not on PATH, so SDC cannot make a key".to_string());
    };

    let output = command
        .args(["-t", "ed25519", "-N", "", "-C", "sdc", "-f", &path.display().to_string()])
        .output()
        .map_err(|error| format!("`ssh-keygen` could not be started: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "`ssh-keygen` refused: {}",
            String::from_utf8_lossy(&output.stderr).lines().next().unwrap_or("no reason given")
        ));
    }

    public_key().ok_or_else(|| format!("`ssh-keygen` ran but {} has no public half", path.display()))
}

/// Copies SDC's public key onto a host, using the password once, through the daemon's own PTY.
///
/// Returns the sentence the UI shows on success. The password is passed to `ssh` and never written
/// anywhere: not to the store, not to the event log, not into the sentence that comes back - which is
/// why this function takes it as an argument and has no way to keep it.
pub fn install_key(pty: &PtyManager, target: &SshTarget, password: &str) -> Result<String, ErrorObject> {
    let public_key = ensure_key().map_err(ErrorObject::internal)?;
    let command = install_command(&public_key);

    let mut args = target.port_args();

    args.extend([
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-o".to_string(),
        "NumberOfPasswordPrompts=1".to_string(),
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        target.user_host.clone(),
        command,
    ]);

    let opened = pty
        .open("ssh", &args, None)
        .map_err(|error| ErrorObject::not_found(format!("`ssh` could not be started: {}", error.message)))?;
    let pty_id = opened["ptyId"].as_str().unwrap_or_default().to_string();

    let mut typed = false;
    let mut transcript = String::new();
    let mut status = String::new();

    /* Up to ~30 seconds: a handshake, a prompt, an answer, and a one-line shell command. */
    for _ in 0..150 {
        std::thread::sleep(Duration::from_millis(200));

        let snapshot = pty.output(&pty_id).unwrap_or_else(|_| serde_json::json!({}));
        let lines: Vec<String> = snapshot["lines"]
            .as_array()
            .map(|lines| lines.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();

        transcript = lines.join("\n");
        status = snapshot["state"].as_str().unwrap_or("gone").to_string();

        if transcript.contains(MARKER) {
            break;
        }

        match prompt_in(&transcript) {
            Prompt::VerificationCode => {
                let _ = pty.close(&pty_id);

                return Err(ErrorObject::bad_request(format!(
                    "{} asks for a verification code, which SDC cannot answer for you - a one-time code is a second factor and a daemon holding one would defeat it. Two ways forward: sign in the way you would from a terminal and add this key once, or turn 2FA off for the install and back on after. Public key: {public_key}",
                    target.user_host
                )));
            }
            Prompt::Password if !typed && !password.is_empty() => {
                pty.write(&pty_id, &format!("{password}\n")).map_err(|error| {
                    ErrorObject::internal(format!("the password could not be typed: {}", error.message))
                })?;

                typed = true;
            }
            _ => {}
        }

        if status != "running" {
            break;
        }
    }

    let _ = pty.close(&pty_id);

    if transcript.contains(MARKER) {
        return Ok(format!(
            "SDC's key is on {} · the password was used once and was not stored",
            target.user_host
        ));
    }

    Err(ErrorObject::bad_request(refusal(&target.user_host, &transcript, &status)))
}

/// The sentence for an install that did not finish - `ssh`'s own line where there is one.
fn refusal(user_host: &str, transcript: &str, status: &str) -> String {
    let last = transcript
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output");

    match prompt_in(transcript) {
        Prompt::VerificationCode => format!(
            "{user_host} wants a verification code. A one-time code is a second factor; SDC will not ask you for it. Sign in from a terminal once and add the key by hand, or sign in without 2FA once to set this up."
        ),
        Prompt::Password => format!(
            "{user_host} refused that password: {last}. Check it - and note that a host which *only* offers a verification code cannot be set up from here."
        ),
        _ if status == "running" => format!("{user_host} did not finish setting up the key; last line: {last}"),
        _ => format!("{user_host} refused the key install: {last}"),
    }
}

/// The shell command that appends SDC's key, once, and prints the marker.
fn install_command(public_key: &str) -> String {
    format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
         (grep -qF '{public_key}' ~/.ssh/authorized_keys 2>/dev/null || echo '{public_key}' >> ~/.ssh/authorized_keys) && \
         chmod 600 ~/.ssh/authorized_keys && echo {MARKER}"
    )
}

/// What an `ssh` transcript is waiting for.
#[derive(Debug, PartialEq, Eq)]
pub enum Prompt {
    /// A password, which this can answer.
    Password,
    /// A one-time code, which it cannot - a code is a second factor.
    VerificationCode,
    /// A host key question, which `accept-new` answers.
    HostKey,
    /// Nothing: the process is working or finished.
    None,
}

/// Reads an `ssh` transcript and says what it is waiting for.
pub fn prompt_in(text: &str) -> Prompt {
    let lowered = text.to_lowercase();

    /* The password question first: OpenSSH asks `user@host's password:`, and a keyboard-interactive
       host asks the same thing in its own words. */
    if lowered.contains("password:") || lowered.contains("password for ") {
        return Prompt::Password;
    }

    if lowered.contains("verification code") || lowered.contains("one-time") || lowered.contains("passcode") {
        return Prompt::VerificationCode;
    }

    if lowered.contains("are you sure you want to continue connecting") || lowered.contains("host key verification") {
        return Prompt::HostKey;
    }

    Prompt::None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact string from the report: a full `ssh` command with a port.
    #[test]
    fn a_pasted_ssh_command_yields_the_address_and_the_port() {
        let parsed = parse_target("ssh -p 8443 mehedi105117@109.199.108.216").unwrap();

        assert_eq!(parsed.user_host, "mehedi105117@109.199.108.216");
        assert_eq!(parsed.port, Some(8443));
        assert_eq!(parsed.port_args(), vec!["-p", "8443"]);
    }

    #[test]
    fn the_other_shapes_a_person_types_are_accepted() {
        assert_eq!(
            parse_target("mehedi105117@109.199.108.216").unwrap(),
            SshTarget { user_host: "mehedi105117@109.199.108.216".into(), port: None }
        );
        assert_eq!(parse_target("ssh root@vps.example").unwrap().port, None);
        assert_eq!(parse_target("root@vps.example -p2222").unwrap().port, Some(2222));
        assert_eq!(parse_target("ssh -q -p 22 root@vps.example").unwrap().port, Some(22));
        assert_eq!(parse_target("  ssh   -p   2200   root@vps.example  ").unwrap().port, Some(2200));
    }

    #[test]
    fn a_host_without_a_user_is_explained_rather_than_guessed() {
        let error = parse_target("109.199.108.216").unwrap_err();

        assert!(error.contains("user@host"), "{error}");
        assert!(parse_target("").is_err());
        assert!(parse_target("ssh -p not-a-port root@h").is_err());
        assert!(parse_target("ssh -p").is_err());
    }

    #[test]
    fn the_prompts_are_told_apart() {
        assert_eq!(prompt_in("mehedi105117@109.199.108.216's password:"), Prompt::Password);
        assert_eq!(prompt_in("Password for root@vps:"), Prompt::Password);
        assert_eq!(prompt_in("Verification code:"), Prompt::VerificationCode);
        assert_eq!(prompt_in("Enter passcode: "), Prompt::VerificationCode);
        assert_eq!(prompt_in("Are you sure you want to continue connecting (yes/no)?"), Prompt::HostKey);
        assert_eq!(prompt_in("Welcome to Ubuntu 24.04"), Prompt::None);
    }

    /// The install command has to be one shell line, quote the key, and be safe to run twice.
    #[test]
    fn the_install_command_is_idempotent_and_prints_the_marker() {
        let command = install_command("ssh-ed25519 AAAA sdc");

        assert!(command.contains("grep -qF 'ssh-ed25519 AAAA sdc' ~/.ssh/authorized_keys"));
        assert!(command.contains("echo 'ssh-ed25519 AAAA sdc' >> ~/.ssh/authorized_keys"));
        assert!(command.contains("chmod 600 ~/.ssh/authorized_keys"));
        assert!(command.contains(MARKER));
        assert!(!command.contains('\n'), "one line, because ssh runs it through a shell");
    }

    /// The one test that touches the machine: it makes SDC's key the way the app does.
    ///
    /// `--ignored`, because a test suite should not write to `~/.ssh` on every run - but it is here, and
    /// it is the only way to know that `ssh-keygen -t ed25519 -N ""` is spelled correctly on this
    /// platform (PowerShell, for instance, cannot pass that empty argument at all).
    #[test]
    #[ignore = "creates ~/.ssh/sdc_ed25519 on the machine that runs it"]
    fn the_key_is_generated_and_readable() {
        let public = ensure_key().expect("ssh-keygen should produce a key");

        assert!(public.starts_with("ssh-ed25519 "), "{public}");
        assert!(public.ends_with(" sdc"), "{public}");
        assert_eq!(public_key().as_deref(), Some(public.as_str()));

        /* And the second call is the same key, not a new one. */
        assert_eq!(ensure_key().unwrap(), public);
    }
}
