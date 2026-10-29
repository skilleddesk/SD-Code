//! **Claude on every server, from one approval (1.0.6).**
//!
//! The report: "Claude Code is connected in SDC, but every VPS says `claude_code is signed out where this chat runs` -
//! connect it once and every server must work; nobody does the same work twice." A sign-in made on this PC cannot go
//! to a server by copying it: SDC never reads a CLI's own credential (the credential guard, P0-2), and a copied
//! sign-in would fight the original over its refresh token - one of the two ends `OAuth session expired`.
//!
//! Claude Code has a command made for exactly this: `claude setup-token` mints a **long-lived (one-year) token** for
//! the subscription, meant for machines that cannot open a browser, and prints it. SDC runs that command here, shows
//! its approval page in the Connect window the way every CLI sign-in is shown, hands back the code the person pastes,
//! and keeps the token the CLI prints in SDC's own key store - never on the screen, never in a log. A Claude turn on any
//! host then gets it as the CLI's own environment variable, over stdin into a file the turn reads and deletes
//! (`engines::cli`). One approval, every server, for a year.
//!
//! `setup-token` draws with Ink and needs a terminal: over pipes it prints nothing at all (measured, 2.1.312). So this
//! one login runs in a real pseudo-terminal (ConPTY on Windows, `openpty` elsewhere, through `portable-pty`), wide
//! enough that the token is never wrapped, with the cursor-position question a ConPTY asks first answered.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;

/// The id the Connect window and `cli.login` know this sign-in by.
pub const PROVIDER: &str = "claude-servers";
/// What the window calls it.
pub const LABEL: &str = "Claude Code on your servers";
/// The variable Claude Code reads a `setup-token` token from - the only place this name is written in SDC.
pub const ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// What the Connect window says under the link.
pub const NOTE: &str = "One approval makes Claude Code work on every server you connect, for a year: Claude Code's own `claude setup-token` makes a token for your subscription, and SDC keeps it in its key store and hands it to Claude on a server for each turn. Approve the page, paste the code it shows here.";

/// The error card's button that opens this approval.
pub const ACTION: &str = "connect:claude-servers";
/// What the card says when Claude on a server is signed out.
pub const SIGNED_OUT: &str = "Claude Code on this server has no sign-in that works. Press **Connect Claude for all servers** and approve once: SDC then hands Claude a token for your subscription on every server you connect - nothing to type on any of them. Send the message again afterwards.";

/// The key-store name the token is kept under.
fn key_name() -> String {
    crate::providers::key_ref(PROVIDER)
}

/// The token, when one was made.
pub fn stored() -> Option<String> {
    crate::auth::keychain::get(&key_name()).map(|token| healed(&token)).filter(|token| !token.trim().is_empty())
}

/// The token with what 1.0.6 glued to it taken off: the start of the next line, `Store this token securely`.
/// A token saved by 1.0.6 works again without a second approval.
pub fn healed(token: &str) -> String {
    let token = token.trim();

    match token.strip_suffix("Store") {
        Some(cut) if cut.ends_with("AA") => cut.to_string(),
        _ => token.to_string(),
    }
}

/// Forgets the token - a server that says it was refused gets a fresh approval, not the same refusal again.
pub fn forget() {
    let _ = crate::auth::keychain::delete(&key_name());
}

/// The environment a Claude turn on a host gets: the token, when there is one.
pub fn remote_env(program: &str) -> Vec<(String, String)> {
    if program != "claude" {
        return Vec::new();
    }

    stored().map(|token| vec![(ENV.to_string(), token)]).unwrap_or_default()
}

/// The text a terminal shows, without its escape sequences: CSI (`ESC [ … letter`), OSC (`ESC ] … BEL` or `ESC \`)
/// and the two-byte ones.
pub fn plain(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }

        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        /* 1.0.7: a cursor move is where one piece of the screen ends and the next begins. Dropped
                           without a trace, it glued the token to the word after it: Ink moves the cursor between the
                           token and "Store this token securely", and the token was saved as `…AAStore`. Only a
                           colour change (`m`) sits inside a run of text. */
                        if next != 'm' {
                            out.push('\n');
                        }

                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\u{7}' {
                        break;
                    }

                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    out
}

const TOKEN_PREFIX: &str = "sk-ant-oat";

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// The token `setup-token` printed, from its screen text.
pub fn extract_token(text: &str) -> Option<String> {
    let start = text.find(TOKEN_PREFIX)?;
    let token: String = text[start..].chars().take_while(|c| token_char(*c)).collect();
    let token = healed(&token);

    (token.len() >= 40).then_some(token)
}

/// The screen text with every token in it replaced - what the window and any log may see.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find(TOKEN_PREFIX) {
        out.push_str(&rest[..start]);

        let after = &rest[start..];
        let length = after.chars().take_while(|c| token_char(*c)).map(char::len_utf8).sum::<usize>();

        out.push_str("sk-ant-oat…(kept in SDC's key store, never shown)");
        rest = &after[length..];
    }

    out.push_str(rest);
    out
}

struct Session {
    output: Arc<Mutex<Vec<u8>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Kept so the terminal stays open while the CLI runs.
    _master: Box<dyn portable_pty::MasterPty + Send>,
    started: Instant,
    code_sent: bool,
    saved: bool,
    cancelled: bool,
}

fn sessions() -> &'static Mutex<HashMap<String, Session>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();

    SESSIONS.get_or_init(Default::default)
}

/// True for the ids this module hands out, so `cli_login` knows whose they are.
pub fn owns(login_id: &str) -> bool {
    login_id.starts_with("token-")
}

/// Starts `claude setup-token` in a terminal of its own.
pub fn start() -> Result<Value, ErrorObject> {
    start_program("claude", &["setup-token"])
}

fn start_program(program: &str, args: &[&str]) -> Result<Value, ErrorObject> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let (executable, prefix) = crate::host::program::launch(program)
        .ok_or_else(|| ErrorObject::not_found(format!("`{program}` is not installed on this computer - install it first; Settings → Environment has the row")))?;
    let pair = native_pty_system()
        /* Wide, so the 100-odd character token is one line and never broken by a wrap. */
        .openpty(PtySize { rows: 40, cols: 1000, pixel_width: 0, pixel_height: 0 })
        .map_err(|error| ErrorObject::internal(format!("a terminal for `{program}` could not be opened: {error}")))?;
    let mut command = CommandBuilder::new(executable);

    command.args(prefix);
    command.args(args);

    if let Ok(dir) = std::env::current_dir() {
        command.cwd(dir);
    }

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| ErrorObject::internal(format!("`{program}` could not be started: {error}")))?;

    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|error| ErrorObject::internal(error.to_string()))?;
    let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(pair.master.take_writer().map_err(|error| ErrorObject::internal(error.to_string()))?));
    let output = Arc::new(Mutex::new(Vec::new()));

    {
        let output = output.clone();
        let writer = writer.clone();

        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];

            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }

                let chunk = &buffer[..read];

                /* A ConPTY asks where the cursor is before it shows anything, and waits for the answer. */
                if chunk.windows(4).any(|window| window == b"\x1b[6n") {
                    if let Ok(mut writer) = writer.lock() {
                        let _ = writer.write_all(b"\x1b[1;1R");
                        let _ = writer.flush();
                    }
                }

                if let Ok(mut output) = output.lock() {
                    output.extend_from_slice(chunk);
                }
            }
        });
    }

    static NEXT: Mutex<u64> = Mutex::new(0);
    let id = {
        let mut next = NEXT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        *next += 1;

        format!("token-{}", *next)
    };

    sessions().lock().map_err(|_| ErrorObject::internal("token registry poisoned"))?.insert(
        id.clone(),
        Session { output, writer, child, _master: pair.master, started: Instant::now(), code_sent: false, saved: false, cancelled: false },
    );

    Ok(json!({ "loginId": id, "ptyId": id, "program": program, "providerId": PROVIDER }))
}

/// Where the approval has got to - the same answer `cli.login.status` gives for every CLI. The moment the CLI prints
/// its token, it is put in the key store and the CLI is closed; the answer only ever carries the redacted screen.
pub fn status(id: &str) -> Result<Value, ErrorObject> {
    status_saving(id, &|token| crate::auth::keychain::set(&key_name(), token).is_ok())
}

fn status_saving(id: &str, save: &dyn Fn(&str) -> bool) -> Result<Value, ErrorObject> {
    let mut sessions = sessions().lock().map_err(|_| ErrorObject::internal("token registry poisoned"))?;
    let session = sessions.get_mut(id).ok_or_else(|| ErrorObject::not_found(format!("{id} is not a login this daemon started")))?;
    let text = plain(&String::from_utf8_lossy(&session.output.lock().map(|output| output.clone()).unwrap_or_default()));

    if !session.saved {
        if let Some(token) = extract_token(&text) {
            session.saved = save(&token);

            if session.saved {
                let _ = session.child.kill();
            }
        }
    }

    let alive = !session.saved && !session.cancelled && matches!(session.child.try_wait(), Ok(None));
    let url = crate::auth::cli_login::extract_url(&text);
    let state = if session.saved {
        "authenticated"
    } else if session.cancelled {
        "cancelled"
    } else if !alive {
        if session.code_sent {
            "exited"
        } else {
            "failed"
        }
    } else if url.is_some() {
        "waiting_for_code"
    } else {
        "waiting_for_url"
    };
    let lines: Vec<String> = redact(&text).replace('\r', "\n").lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).map(str::to_string).collect();

    Ok(json!({
        "loginId": id,
        "providerId": PROVIDER,
        "providerLabel": LABEL,
        "program": "claude setup-token",
        "url": url,
        "state": state,
        "note": NOTE,
        "lineCount": lines.len(),
        "lines": lines,
        "ms": session.started.elapsed().as_millis() as u64,
        "authenticated": session.saved,
    }))
}

/// Types the pasted code into the CLI, followed by Enter - a terminal's Enter is a carriage return.
pub fn submit_code(id: &str, code: &str) -> Result<Value, ErrorObject> {
    let value = match crate::auth::cli_login::extract_code(code) {
        Some(candidate) if crate::auth::cli_login::is_authorization_code(&candidate) => candidate,
        _ => code.trim().to_string(),
    };
    let mut sessions = sessions().lock().map_err(|_| ErrorObject::internal("token registry poisoned"))?;
    let session = sessions.get_mut(id).ok_or_else(|| ErrorObject::not_found(format!("{id} is not a login this daemon started")))?;
    let mut writer = session.writer.lock().map_err(|_| ErrorObject::internal("token terminal poisoned"))?;

    writer.write_all(format!("{value}\r").as_bytes()).map_err(|error| ErrorObject::internal(format!("the code could not be typed: {error}")))?;
    let _ = writer.flush();
    session.code_sent = true;

    Ok(json!({ "submitted": true, "loginId": id }))
}

/// Ends the CLI. Nothing is kept: the token only exists once the CLI printed it, and then it is already stored.
pub fn cancel(id: &str) -> Result<Value, ErrorObject> {
    let mut sessions = sessions().lock().map_err(|_| ErrorObject::internal("token registry poisoned"))?;
    let session = sessions.get_mut(id).ok_or_else(|| ErrorObject::not_found(format!("{id} is not a login this daemon started")))?;

    session.cancelled = true;

    Ok(json!({ "cancelled": session.child.kill().is_ok(), "loginId": id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: &str = "\u{1b}[?25l\u{1b}[2mWelcome to Claude Code\u{1b}[0m\r\n\u{1b}]8;id=u-1;https://claude.com/cai/oauth/authorize?code=true&client_id=x&state=y\u{7}https://claude.com/cai/oauth/authorize?code=true&client_id=x&state=y\u{1b}]8;;\u{7}\r\nPaste code here if prompted >\r\n✓ Long-lived authentication token created successfully!\r\nYour OAuth token (valid for 1 year):\r\n\u{1b}[1msk-ant-oat01-test-test-test-test-test-test-test-test-test-test-test-test\u{1b}[22m\r\nStore this token securely.";

    #[test]
    fn the_screen_is_read_without_its_escapes_and_the_token_is_found_whole() {
        let text = plain(SCREEN);

        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("Welcome to Claude Code"));
        assert_eq!(crate::auth::cli_login::extract_url(&text).as_deref(), Some("https://claude.com/cai/oauth/authorize?code=true&client_id=x&state=y"));
        assert_eq!(extract_token(&text).as_deref(), Some("sk-ant-oat01-test-test-test-test-test-test-test-test-test-test-test-test"));
        assert_eq!(extract_token("Paste code here if prompted >"), None);
        assert_eq!(extract_token("sk-ant-oat01-short"), None, "a cut-off token is not a token");
    }

    /// 1.0.7, the report "after signing in, the same error again": the real screen moves the cursor between the
    /// token and the next sentence, and the saved token ended in `Store`.
    #[test]
    fn a_cursor_move_after_the_token_does_not_glue_the_next_word_to_it() {
        let screen = "Your OAuth token (valid for 1 year):\u{1b}[1B\u{1b}[2Gsk-ant-oat01-test-test-test-test-test-test-test-test-test-test-test-testAA\u{1b}[2B\u{1b}[2GStore this token securely.";

        assert_eq!(extract_token(&plain(screen)).as_deref(), Some("sk-ant-oat01-test-test-test-test-test-test-test-test-test-test-test-testAA"));
        assert_eq!(healed("sk-ant-oat01-test-test-test-test-test-test-testAAStore"), "sk-ant-oat01-test-test-test-test-test-test-testAA", "a 1.0.6 token works again");
        assert_eq!(healed("sk-ant-oat01-test-test-testAA"), "sk-ant-oat01-test-test-testAA");
    }

    #[test]
    fn the_token_never_reaches_the_window() {
        let shown = redact(&plain(SCREEN));

        assert!(!shown.contains("test-test"), "{shown}");
        assert!(shown.contains("never shown") && shown.contains("Store this token securely."));
    }

    #[test]
    fn only_claude_signed_out_on_a_server_gets_the_connect_button() {
        use crate::sdcp::methods::error_action;

        assert_eq!(error_action("engine-signed-out", "claude_code", true), Some(ACTION));
        assert_eq!(error_action("engine-signed-out", "claude_code", false), None, "on this PC the Providers card signs in");
        assert_eq!(error_action("codex-signed-out", "codex", true), Some(crate::sdcp::methods::CODEX_HOST_ACTION));
        assert_eq!(error_action("codex-signed-out", "codex", false), None);
        assert_eq!(error_action("missing-program-remote", "claude_code", true), None);
    }

    #[test]
    fn only_a_claude_turn_on_a_host_gets_the_token() {
        crate::auth::keychain::set(&key_name(), "sk-ant-oat01-test-token-for-the-environment-0123456789").unwrap();

        assert_eq!(remote_env("claude"), vec![(ENV.to_string(), "sk-ant-oat01-test-token-for-the-environment-0123456789".to_string())]);
        assert!(remote_env("codex").is_empty() && remote_env("gemini").is_empty());

        forget();

        assert!(remote_env("claude").is_empty(), "no approval yet, nothing sent");
    }

    /// The whole flow through a real terminal, with a stand-in for `claude setup-token`: the link is shown, the code
    /// typed back with Enter, and the token the program prints is saved - and only ever shown redacted.
    #[test]
    fn a_setup_token_run_in_a_terminal_is_driven_from_link_to_saved_token() {
        let Some(node) = crate::host::program::resolve("node") else {
            return;
        };
        let script = r#"process.stdout.write('Browser didn\'t open? Use the url below\r\nhttps://claude.com/cai/oauth/authorize?code=true&state=s\r\nPaste code here if prompted > ');process.stdin.setRawMode&&process.stdin.setRawMode(true);let got='';process.stdin.on('data',d=>{got+=d.toString();if(got.includes('\r')){process.stdout.write('\r\nYour OAuth token (valid for 1 year):\r\nsk-ant-oat01-'+got.trim().replace(/[^A-Za-z0-9]/g,'')+'-0123456789012345678901234567890123456789\r\n');setTimeout(()=>process.exit(0),300)}});"#;
        let started = start_program(&node.display().to_string(), &["-e", script]).unwrap();
        let id = started["loginId"].as_str().unwrap().to_string();
        let saved = Arc::new(Mutex::new(None::<String>));
        let keep = |token: &str| {
            *saved.lock().unwrap() = Some(token.to_string());
            true
        };
        let wait_for = |wanted: &str| {
            for _ in 0..100 {
                let answer = status_saving(&id, &keep).unwrap();

                if answer["state"] == wanted {
                    return answer;
                }

                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            panic!("never reached {wanted}: {}", status_saving(&id, &keep).unwrap());
        };

        let waiting = wait_for("waiting_for_code");

        assert_eq!(waiting["url"], "https://claude.com/cai/oauth/authorize?code=true&state=s");

        submit_code(&id, "abcdef0123456789#statepart").unwrap();

        let done = wait_for("authenticated");
        let token = saved.lock().unwrap().clone().unwrap();

        assert!(token.starts_with("sk-ant-oat01-abcdef0123456789statepart-"), "{token}");
        assert!(!done.to_string().contains(&token), "the window never sees the token");
    }
}
