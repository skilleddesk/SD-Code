//! Sign in once, run anywhere (0.12.5).
//!
//! A CLI turn on a host runs that host's `claude`, and until 0.12.5 it used that host's own sign-in: a
//! person signed in on this PC and saw `OAuth session expired` on the VPS, because each machine kept its
//! own login ("CLI and API gula SDC app a akbar connect korlei jano local and vps sob jaigai kaj kore").
//!
//! So a remote turn carries this machine's sign-in with it. The value travels on `ssh`'s stdin into a
//! `umask 077` file that the turn's shell reads and deletes before the CLI starts
//! (`ssh::ops::turn_line_with_secrets`): it is never in a command line, and never on the host's disk
//! after the turn began. The host's own `~/.claude` is not touched.
//!
//! API providers need none of this - their calls leave from this daemon for local and remote chats alike.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// A token with less than this left is refreshed first: a long turn must not outlive it.
const FRESH_FOR: Duration = Duration::from_secs(20 * 60);

/// The environment a remote turn of `program` carries, from this machine's sign-in. Empty when this
/// machine is not signed in - the host's own login is then used, as before.
pub fn forwarded(program: &str) -> Vec<(String, String)> {
    match program {
        "claude" => claude_token()
            .map(|token| vec![("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token)])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `~/.claude/.credentials.json`, or the one under `CLAUDE_CONFIG_DIR` - where Claude Code keeps its
/// sign-in on Windows and Linux. (macOS keeps it in the Keychain - see `read_claude_keychain`.)
fn claude_credentials_path() -> Option<PathBuf> {
    let folder = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => dirs::home_dir()?.join(".claude"),
    };

    Some(folder.join(".credentials.json"))
}

/// The access token and when it expires (ms since the epoch), if the file holds a sign-in.
fn read_claude() -> Option<(String, u64)> {
    let from_file = claude_credentials_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| parse_claude(&text));

    from_file.or_else(read_claude_keychain)
}

/// macOS: Claude Code keeps its sign-in in the login Keychain as `Claude Code-credentials`, holding the
/// same JSON the file holds elsewhere. Until 0.15.8 a Mac therefore forwarded nothing to a VPS. Read with
/// Apple's own `security` tool (the program Claude Code writes the item with), bounded so a Keychain
/// question nobody answers cannot hold a turn.
#[cfg(target_os = "macos")]
fn read_claude_keychain() -> Option<(String, u64)> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut child = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if started.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();

                return None;
            }
        }
    }

    let mut text = String::new();

    child.stdout.take()?.read_to_string(&mut text).ok()?;

    parse_claude(text.trim())
}

#[cfg(not(target_os = "macos"))]
fn read_claude_keychain() -> Option<(String, u64)> {
    None
}

fn parse_claude(text: &str) -> Option<(String, u64)> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    let oauth = parsed.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?.trim().to_string();
    let expires = oauth.get("expiresAt").and_then(Value::as_u64).unwrap_or(0);

    (!token.is_empty()).then_some((token, expires))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn fresh(expires_ms: u64) -> bool {
    expires_ms > now_ms() + FRESH_FOR.as_millis() as u64
}

/// This machine's Claude token, refreshed first when it is close to expiring.
///
/// The refresh is Claude Code's own: one tiny local `claude -p` makes the CLI renew its token and write
/// it back, exactly as any local use would. SDC never calls the sign-in endpoint itself.
fn claude_token() -> Option<String> {
    let (token, expires) = read_claude()?;

    if fresh(expires) {
        return Some(token);
    }

    refresh_claude_locally();

    read_claude().filter(|(_, expires)| fresh(*expires)).map(|(token, _)| token)
}

fn refresh_claude_locally() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let Some((executable, prefix)) = crate::host::program::launch("claude") else {
        return;
    };
    let mut command = Command::new(executable);

    command
        .args(prefix)
        .args(["-p", "--model", "haiku"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let Ok(mut child) = command.spawn() else {
        return;
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"Reply with: ok");
    }

    let started = std::time::Instant::now();

    while started.elapsed() < Duration::from_secs(90) {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_in_file_gives_its_token_and_expiry() {
        let text = r#"{"claudeAiOauth":{"accessToken":" tok-1 ","refreshToken":"r","expiresAt":1790000000000}}"#;

        assert_eq!(parse_claude(text), Some(("tok-1".to_string(), 1_790_000_000_000)));
        assert_eq!(parse_claude(r#"{"claudeAiOauth":{"accessToken":""}}"#), None);
        assert_eq!(parse_claude("{}"), None);
    }

    #[test]
    fn a_token_about_to_expire_is_not_fresh() {
        assert!(fresh(now_ms() + 60 * 60 * 1000));
        assert!(!fresh(now_ms() + 60 * 1000));
        assert!(!fresh(0));
    }

    #[test]
    fn only_claude_is_forwarded() {
        assert!(forwarded("codex").is_empty());
        assert!(forwarded("gemini").is_empty());
    }
}
