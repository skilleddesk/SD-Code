//! Signing a CLI in *from the app* (master spec sections 9.10 and 15.1).
//!
//! The quiet truth about the subscription providers is that the login belongs to the CLI: Claude Code,
//! Codex and Gemini each open a browser, ask the user to approve, and store the credential themselves.
//! SDC must not - and does not - touch that credential. What it can do is **drive the CLI's own login**,
//! which is what this module is: start the CLI, read the URL it prints, show it to the user with a copy
//! button, and hand back the code they paste. The token never leaves the CLI's own store.
//!
//! So the flow is:
//!
//! ```text
//!   UI                     sdcd                    the CLI
//!   cli.login        ->    pty.open(program)  ->   "Open https://…/authorize?code=…"
//!   cli.login.status <-    URL + tail + state
//!   (user copies the URL, approves, pastes the code)
//!   cli.login.code   ->    pty.write(code)    ->   "Successfully logged in"
//!   cli.login.status <-    state: authenticated
//! ```
//!
//! Two properties make this honest rather than clever:
//!
//! * **the recipes are data.** A CLI that changes its login command is a row in `RECIPES`, not a code
//!   change - and the UI always shows the CLI's raw output, so a recipe that goes stale is visible
//!   instead of silent;
//! * **nothing is auto-opened.** The URL is shown and copied; the app does not launch a browser on its
//!   own. A login link is a capability, and a capability is handed to the user to approve.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use serde_json::{json, Value};

use crate::pty::PtyManager;
use crate::sdcp::envelope::ErrorObject;

/// How one CLI signs in. `pump` is what the daemon types once the CLI is up, because two of the three
/// put their login behind an interactive prompt rather than a subcommand.
#[derive(Debug, Clone, Copy)]
pub struct LoginRecipe {
    pub provider_id: &'static str,
    pub label: &'static str,
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub pump: &'static [&'static str],
    /// A line that means the CLI finished signing in, matched case-insensitively.
    pub success: &'static [&'static str],
    /// What the UI says about this recipe, including anything the user has to know first.
    pub note: &'static str,
    /// Something the CLI needs *before* it will sign in, written to the CLI's own settings file.
    pub prepare: Prepare,
}

/// The step before the login, for a CLI that refuses to start one until it has been told how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prepare {
    /// Nothing: the CLI signs in when it is asked to.
    Nothing,
    /// Gemini CLI needs an auth method in its settings, or it exits with
    /// "Please set an Auth method in your settings.json". `oauth-personal` is its own name for
    /// `Login with Google` (verified in the installed package, `security.auth.selectedType`).
    ///
    /// Without this the flow cannot start at all in a pipe: the menu that would choose it needs a
    /// terminal, which the daemon does not have. Writing the choice is the smallest honest way to make
    /// the browser sign-in reachable from the app, and it is one key in a file the user owns - the
    /// CLI's own menu can change it back at any time.
    GeminiOauth,
}

/// The settings file the Gemini prepare step writes, under the user's home.
pub fn gemini_settings_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;

    Some(std::path::PathBuf::from(home).join(".gemini").join("settings.json"))
}

/// Runs a recipe's prepare step, returning the file it touched (for the answer the UI shows).
pub fn prepare(recipe: &LoginRecipe) -> Result<Option<String>, String> {
    match recipe.prepare {
        Prepare::Nothing => Ok(None),
        Prepare::GeminiOauth => {
            let path = gemini_settings_path()
                .ok_or_else(|| "no home directory to write the Gemini settings to".to_string())?;

            merge_gemini_oauth(&path)?;

            Ok(Some(path.display().to_string()))
        }
    }
}

/// Sets `security.auth.selectedType` to `oauth-personal`, keeping every other key the file has.
///
/// Merging rather than overwriting matters: this is the user's Gemini CLI configuration, which also
/// holds their theme, their MCP servers and their trusted folders.
pub fn merge_gemini_oauth(path: &std::path::Path) -> Result<(), String> {
    let mut settings: Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));

    let Some(root) = settings.as_object_mut() else {
        return Err(format!("{} is not a JSON object, so it was left alone", path.display()));
    };

    let security = root.entry("security").or_insert_with(|| json!({}));
    let Some(security) = security.as_object_mut() else {
        return Err(format!("security in {} is not an object, so it was left alone", path.display()));
    };

    let auth = security.entry("auth").or_insert_with(|| json!({}));
    let Some(auth) = auth.as_object_mut() else {
        return Err(format!("security.auth in {} is not an object, so it was left alone", path.display()));
    };

    auth.insert("selectedType".to_string(), json!("oauth-personal"));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }

    let text = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;

    std::fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// The three subscription providers, as data.
///
/// Two of these were corrected against the real CLIs (`_verify/cli-logins.mjs`, and the version strings
/// reported by `cli.recipes`):
///
///   * **Claude Code has a subcommand.** `claude auth login` signs in without a terminal, and that is
///     what this uses. The first version pumped `/login` into an interactive session instead, which
///     needs a TTY: over the daemon's pipes the CLI printed nothing, so the flow sat at
///     `waiting_for_url` forever with no URL to show;
///   * **Codex prints two URLs** (its own callback server, then the approval page) - `extract_url` now
///     prefers the `https://` one and trims the sentence's full stop.
///
/// `codex login` was already right. Gemini's recipe is the honest one: its first run asks how to sign
/// in, and that menu needs a terminal, so the note says what the user has to do once.
pub const RECIPES: &[LoginRecipe] = &[
    LoginRecipe {
        provider_id: "claude",
        label: "Claude Code",
        program: "claude",
        args: &["auth", "login"],
        pump: &[],
        success: &["successfully logged in", "login successful", "logged in as", "you are now logged in"],
        note: "Claude Code opens the approval page itself; the link is also printed here so it can be copied.",
        prepare: Prepare::Nothing,
    },
    LoginRecipe {
        provider_id: "openai",
        label: "Codex",
        program: "codex",
        args: &["login"],
        pump: &[],
        success: &["successfully logged in", "login successful", "authenticated"],
        note: "Codex waits for its browser callback; if the browser cannot reach it, paste the code from the page instead.",
        prepare: Prepare::Nothing,
    },
    LoginRecipe {
        provider_id: "gemini",
        label: "Gemini",
        program: "gemini",
        /* `--skip-trust`: without it Gemini CLI stops at its trusted-folder question before it reaches
           any sign-in, because the daemon drives it through pipes rather than a terminal. The flag is
           scoped to this one process, and the process exists to open a login page - it does not run a
           model action. */
        args: &["--skip-trust"],
        pump: &[],
        success: &["successfully logged in", "login successful", "authenticated", "signed in"],
        note: "Gemini CLI asks how to sign in the first time it runs, and that menu needs a terminal. SDC writes `security.auth.selectedType = oauth-personal` (Gemini's own name for Login with Google) into `~/.gemini/settings.json` so the browser sign-in can run here; the CLI's own menu can change it back.",
        prepare: Prepare::GeminiOauth,
    },
];

/// The recipe for a provider, or `None` when that provider does not sign in through a CLI.
pub fn recipe(provider_id: &str) -> Option<&'static LoginRecipe> {
    RECIPES.iter().find(|recipe| recipe.provider_id == provider_id)
}

/// The login URL in some output.
///
/// Two rules, both learned by running the real CLIs:
///
///   * **an `https://` page wins over `http://localhost`.** Codex prints both - first the callback
///     server it just started (`http://localhost:1455`), then the page the *user* has to open
///     (`https://auth.openai.com/oauth/authorize?...`). "The first URL wins" showed the user the
///     callback server, which is useless to them. A loopback address is still shown when it is all the
///     CLI printed (it is the browser's landing page); a plain `http://` page on a *remote* host never
///     is - that would be a login page over an unencrypted link;
///   * **sentence punctuation is not part of a URL.** The line reads `Starting local login server on
///     http://localhost:1455.` and the copy button handed over `http://localhost:1455.` - a link that
///     does not resolve. Trailing `.`, `,`, `;`, `:`, `!`, `?` and closing brackets/quotes are trimmed
///     repeatedly. (A URL ending in a media filename would lose its extension here. That is a trade
///     this function can make: its contract is "the CLI's login page", and no login page ends in
///     `.png`.)
pub fn extract_url(text: &str) -> Option<String> {
    let candidates: Vec<String> = text
        .split_whitespace()
        .filter_map(|token| {
            let trimmed = token.trim();

            if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
                return None;
            }

            Some(trimmed.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\'']).to_string())
        })
        .collect();

    candidates
        .iter()
        .find(|candidate| candidate.starts_with("https://"))
        .or_else(|| candidates.iter().find(|candidate| candidate.starts_with("http://localhost")))
        .cloned()
}

/// True when the CLI said it finished. Matched case-insensitively against the whole tail, because the
/// sentence wraps differently in each CLI and the wording is the part that matters.
pub fn is_authenticated(text: &str, recipe: &LoginRecipe) -> bool {
    let haystack = text.to_lowercase();

    recipe.success.iter().any(|pattern| haystack.contains(pattern))
}

/// One in-flight login, tracked by id so the UI can poll it and hand back a code.
#[derive(Debug, Clone)]
pub struct LoginSession {
    pub provider_id: String,
    pub pty_id: String,
    pub url: Option<String>,
    pub state: String,
    pub code_sent: bool,
    /// Whether this session's success has already been pushed as a `ProviderStatus`. See
    /// `status_announcing`.
    pub announced: bool,
    pub started: Instant,
}

/// The logins this daemon has started. One per provider is the normal case; the map allows a retry
/// after a cancel without a stale entry winning.
///
/// It holds the process registry rather than borrowing it, because the login's own work happens on a
/// thread (typing the CLI's prompt once its screen is up) and a thread cannot borrow a manager that a
/// request is also using.
pub struct LoginManager {
    pty: std::sync::Arc<PtyManager>,
    sessions: Mutex<HashMap<String, LoginSession>>,
    next_id: Mutex<u64>,
}

impl LoginManager {
    pub fn new(pty: std::sync::Arc<PtyManager>) -> Self {
        Self { pty, sessions: Mutex::new(HashMap::new()), next_id: Mutex::new(0) }
    }

    /// Starts a login. `provider_id` picks a recipe, and `program`/`args`/`pump` override it - which is
    /// what makes the mechanism testable without Claude installed, and what lets a user point SDC at a
    /// CLI of their own.
    pub fn start(
        &self,
        provider_id: &str,
        program: Option<&str>,
        args: Option<Vec<String>>,
        pump: Option<Vec<String>>,
    ) -> Result<Value, ErrorObject> {
        let known = recipe(provider_id);
        let (program, recipe_args, recipe_pump) = match (program, known) {
            (Some(program), _) => (program.to_string(), Vec::new(), Vec::new()),
            (None, Some(recipe)) => (
                recipe.program.to_string(),
                recipe.args.iter().map(|arg| (*arg).to_string()).collect(),
                recipe.pump.iter().map(|line| (*line).to_string()).collect(),
            ),
            (None, None) => {
                return Err(ErrorObject::bad_request(format!(
                    "`{provider_id}` does not sign in through a CLI; connect it with an API key instead"
                )))
            }
        };
        let args = args.unwrap_or(recipe_args);
        let pump = pump.unwrap_or(recipe_pump);
        let opened = self.pty.open(&program, &args, None, None, None).map_err(|error| {
            /* A CLI that is not installed is the common case, and it deserves the doctor's wording
               rather than "internal". */
            if error.message.contains("could not be started") {
                ErrorObject::not_found(format!(
                    "{} - install it first; Settings → Environment has the row",
                    error.message
                ))
            } else {
                error
            }
        })?;
        let pty_id = opened["ptyId"].as_str().unwrap_or_default().to_string();
        let id = {
            let mut next = self.next_id.lock().map_err(|_| ErrorObject::internal("login registry poisoned"))?;

            *next += 1;

            format!("login-{}", *next)
        };

        self.sessions
            .lock()
            .map_err(|_| ErrorObject::internal("login registry poisoned"))?
            .insert(
                id.clone(),
                LoginSession {
                    provider_id: provider_id.to_string(),
                    pty_id: pty_id.clone(),
                    url: None,
                    state: "starting".to_string(),
                    code_sent: false,
                    announced: false,
                    started: Instant::now(),
                },
            );

        /* Typing the CLI's prompt has to wait for its screen to be drawn, and the response must not
           wait with it: the UI wants the id immediately, and the URL a moment later. */
        if !pump.is_empty() {
            let pty = self.pty.clone();
            let pty_id = pty_id.clone();

            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(1200));

                for line in pump {
                    let _ = pty.write(&pty_id, &format!("{line}\n"));
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
            });
        }

        Ok(json!({ "loginId": id, "ptyId": pty_id, "program": program, "providerId": provider_id }))
    }

    /// The state of a login, derived from the CLI's own output every time it is asked.
    ///
    /// Deriving rather than remembering is deliberate: no background thread can fall out of step with
    /// reality, and a CLI that printed its success line while nobody was polling is still recognised on
    /// the next poll.
    pub fn status(&self, id: &str) -> Result<Value, ErrorObject> {
        let snapshot = self
            .sessions
            .lock()
            .map_err(|_| ErrorObject::internal("login registry poisoned"))?
            .get(id)
            .cloned()
            .ok_or_else(|| ErrorObject::not_found(format!("{id} is not a login this daemon started")))?;
        let output = self.pty.output(&snapshot.pty_id)?;
        let lines: Vec<String> = output["lines"]
            .as_array()
            .map(|lines| lines.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let text = lines.join("\n");
        let alive = output["state"].as_str().unwrap_or("gone") == "running";
        let recipe = recipe(&snapshot.provider_id);
        let authenticated = recipe.map(|recipe| is_authenticated(&text, recipe)).unwrap_or(false);
        let url = snapshot.url.clone().or_else(|| extract_url(&text));
        let state = if authenticated {
            "authenticated"
        } else if !alive {
            if snapshot.code_sent {
                /* The CLI took the code and finished; whether it *accepted* it is what its own success
                   line says, and the tail is shown so nobody has to guess. */
                "exited"
            } else {
                "failed"
            }
        } else if url.is_some() {
            "waiting_for_code"
        } else {
            "waiting_for_url"
        };

        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(session) = sessions.get_mut(id) {
                session.url = url.clone();
                session.state = state.to_string();
            }
        }

        Ok(json!({
            "loginId": id,
            "providerId": snapshot.provider_id,
            "providerLabel": recipe.map(|recipe| recipe.label).unwrap_or(&snapshot.provider_id),
            "program": output["command"],
            "url": url,
            "state": state,
            "note": recipe.map(|recipe| recipe.note),
            "lines": lines,
            "lineCount": lines.len(),
            "ms": snapshot.started.elapsed().as_millis() as u64,
            "authenticated": authenticated,
        }))
    }

    /// `cli_login.status`, plus whether *this* call is the first to see the CLI's success line.
    ///
    /// The flag is what makes a finished sign-in visible. Starting a login pushes
    /// `ProviderStatus{connecting}`, and only `cli.login.code` used to push `connected` - so a CLI that
    /// finishes on its own, with nobody pasting anything, left the card saying `connecting` for a login
    /// that had already succeeded. That is the report "even the one that succeeded isn't shown".
    /// Claude Code is exactly that case: it opens the browser itself and completes when the user
    /// approves the page.
    pub fn status_announcing(&self, id: &str) -> Result<(Value, bool), ErrorObject> {
        let answer = self.status(id)?;

        if answer["authenticated"] != Value::Bool(true) {
            return Ok((answer, false));
        }

        let first = self
            .sessions
            .lock()
            .map(|mut sessions| match sessions.get_mut(id) {
                Some(session) if !session.announced => {
                    session.announced = true;

                    true
                }
                _ => false,
            })
            .unwrap_or(false);

        Ok((answer, first))
    }

    /// Hands the pasted code to the CLI, which is the only thing that can use it. SDC does not store
    /// it, log it, or send it anywhere else - the credential stays in the CLI's own store.
    pub fn submit_code(&self, id: &str, code: &str) -> Result<Value, ErrorObject> {
        if code.trim().is_empty() {
            return Err(ErrorObject::bad_request("`code` is empty"));
        }

        /* The one paste that looks right and is not: the page SDC showed, handed back as the code.
           Refusing it here with a sentence beats letting the provider answer `400`. */
        if is_authorize_page(code) {
            return Err(ErrorObject::bad_request(
                "that is the page to open in the browser, not the code. Approve it there and paste the code the page shows - or the whole address the browser lands on afterwards",
            ));
        }

        let pty_id = {
            let mut sessions =
                self.sessions.lock().map_err(|_| ErrorObject::internal("login registry poisoned"))?;
            let session = sessions
                .get_mut(id)
                .ok_or_else(|| ErrorObject::not_found(format!("{id} is not a login this daemon started")))?;

            session.code_sent = true;

            session.pty_id.clone()
        };

        /* A pasted redirect URL is reduced to the code inside it, because pasting the whole address is
           what a user naturally does and the code is what the CLI asked for. */
        let value = match extract_code(code) {
            Some(candidate) if is_authorization_code(&candidate) => candidate,
            _ => code.trim().to_string(),
        };

        self.pty.write(&pty_id, &format!("{value}\n"))?;

        Ok(json!({ "submitted": true, "loginId": id }))
    }

    /// Kills the login process. A cancelled login leaves nothing behind: the CLI had not written a
    /// credential yet, so there is nothing to clean up.
    pub fn cancel(&self, id: &str) -> Result<Value, ErrorObject> {
        let pty_id = {
            let mut sessions =
                self.sessions.lock().map_err(|_| ErrorObject::internal("login registry poisoned"))?;
            let session = sessions
                .get_mut(id)
                .ok_or_else(|| ErrorObject::not_found(format!("{id} is not a login this daemon started")))?;

            session.state = "cancelled".to_string();

            session.pty_id.clone()
        };

        Ok(json!({ "cancelled": self.pty.close(&pty_id), "loginId": id }))
    }
}

/// The code inside a pasted URL (`…?code=abc123&…`), or `None` when there is none.
pub fn extract_code(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let start = trimmed.find("code=")?;
    let rest = &trimmed[start + 5..];
    let code = rest.split('&').next().unwrap_or(rest).trim();

    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

/// True when a value behind `code=` could be an authorization code rather than a query flag.
///
/// This is the trap that produced `Login failed: Request failed with status code 400`: Claude's
/// *authorize* page is `https://claude.com/cai/oauth/authorize?code=true&client_id=…`, so a user who
/// pasted the link SDC showed them handed the CLI the literal word `true`. A real code is a long
/// opaque string, so anything shorter than sixteen characters - or the words `true`/`false` - is not
/// treated as one, and the pasted text is passed through whole instead.
pub fn is_authorization_code(value: &str) -> bool {
    value.len() >= 16 && !value.eq_ignore_ascii_case("true") && !value.eq_ignore_ascii_case("false")
}

/// True when what was pasted is the page SDC itself showed, rather than the code the page displayed.
///
/// A callback address (`…/oauth/code/callback?code=…`) is a legitimate paste and is not caught here;
/// the authorize page is, because it is the one thing in this dialog that *looks* like a code and is
/// not.
pub fn is_authorize_page(input: &str) -> bool {
    let lowered = input.to_lowercase();

    (lowered.contains("response_type=code")
        || lowered.contains("client_id=")
        || lowered.contains("/authorize"))
        && !lowered.contains("/callback")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// The URL the user is shown, from the output the real CLIs print.
    ///
    /// The first assertion is Codex verbatim (trimmed): it starts a callback server and prints that
    /// address first, then the page to open. Before this rule the app offered
    /// `http://localhost:1455.` - the callback server, with the sentence's full stop attached.
    #[test]
    fn the_login_page_wins_over_a_callback_server_and_loses_the_full_stop() {
        let codex = "Starting local login server on http://localhost:1455.\n\n\
                     If your browser did not open, navigate to this URL to authenticate:\n\n\
                     https://auth.openai.com/oauth/authorize?response_type=code&state=abc\n";

        assert_eq!(
            extract_url(codex).as_deref(),
            Some("https://auth.openai.com/oauth/authorize?response_type=code&state=abc")
        );

        assert_eq!(
            extract_url("Starting local login server on http://localhost:1455.").as_deref(),
            Some("http://localhost:1455")
        );

        assert_eq!(extract_url("no link here at all"), None);
    }

    /// A stand-in CLI: prints a URL, waits for one line on stdin, then announces success. It is the
    /// whole login mechanism in three lines, and it runs on any machine - which is the point, because
    /// the real CLIs are not installed here.
    fn fake_cli() -> (String, Vec<String>) {
        let script = "echo https://example.com/authorize?code=abc123 & set /p answer= & echo Successfully logged in";

        if cfg!(windows) {
            ("cmd".to_string(), vec!["/C".to_string(), script.to_string()])
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "echo https://example.com/authorize?code=abc123; read answer; echo Successfully logged in".to_string()],
            )
        }
    }

    #[test]
    fn the_three_subscription_providers_have_recipes() {
        assert_eq!(RECIPES.len(), 3);

        for provider in ["claude", "openai", "gemini"] {
            let recipe = recipe(provider).unwrap();

            assert!(!recipe.program.is_empty());
            assert!(!recipe.success.is_empty());
            assert!(!recipe.note.is_empty(), "{provider} needs a note for the UI");
        }

        assert!(recipe("openai-api").is_none(), "an API provider signs in with a key");
    }

    #[test]
    fn finds_the_login_url_in_noisy_output() {
        let output = "Welcome to Claude Code v2.1\nOpen this link:\n  https://claude.ai/authorize?code=1&state=2 \nwaiting…";

        assert_eq!(extract_url(output).as_deref(), Some("https://claude.ai/authorize?code=1&state=2"));
        assert!(extract_url("nothing here").is_none());
        assert!(extract_url("http://example.com/insecure").is_none(), "a plain http login page is not shown");
    }

    #[test]
    fn recognises_a_success_line_whatever_the_case() {
        let recipe = recipe("claude").unwrap();

        assert!(is_authenticated("✔ Successfully logged in as me@example.com", recipe));
        assert!(!is_authenticated("waiting for approval", recipe));
    }

    #[test]
    fn reduces_a_pasted_redirect_url_to_its_code() {
        /* A pasted address is reduced to the code inside it; a code on its own has nothing to reduce,
           so `submit_code` passes it through unchanged. */
        assert_eq!(
            extract_code("http://localhost:1455/callback?code=xyz789&state=1").as_deref(),
            Some("xyz789")
        );
        assert_eq!(extract_code("?code=abc-123").as_deref(), Some("abc-123"));
        assert!(extract_code("abc-123").is_none());
        assert!(extract_code("code=").is_none());
    }

    /// The mechanism, end to end, against a CLI that behaves like the real ones: the URL is found, the
    /// state says it is waiting, the pasted code reaches the process, and the success line is seen.
    #[test]
    fn drives_a_login_from_url_to_authenticated() {
        let pty = Arc::new(PtyManager::new());
        let logins = LoginManager::new(pty.clone());
        let (program, args) = fake_cli();
        let started = logins.start("claude", Some(&program), Some(args), None).unwrap();
        let id = started["loginId"].as_str().unwrap().to_string();

        /* The URL arrives asynchronously, so the test polls the way the UI does. */
        let mut status = serde_json::json!({});

        for _ in 0..40 {
            status = logins.status(&id).unwrap();

            if status["url"].is_string() {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        assert_eq!(status["state"], serde_json::json!("waiting_for_code"));
        assert!(status["url"].as_str().unwrap().contains("authorize"));
        assert_eq!(status["authenticated"], serde_json::json!(false));

        logins.submit_code(&id, "the-code-from-the-page").unwrap();

        for _ in 0..40 {
            status = logins.status(&id).unwrap();

            if status["authenticated"] == serde_json::json!(true) {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        assert_eq!(status["authenticated"], serde_json::json!(true), "tail: {:?}", status["lines"]);
        assert!(status["lines"].as_array().unwrap().iter().any(|line| line.as_str().unwrap_or_default().contains("Successfully logged in")));
    }

    #[test]
    fn a_cli_that_is_not_installed_says_so() {
        let logins = LoginManager::new(Arc::new(PtyManager::new()));
        let error = logins.start("claude", Some("definitely-not-a-cli-sdcd"), None, None).unwrap_err();

        assert_eq!(error.code, "not_found");
        assert!(error.message.contains("install it first"));
    }

    #[test]
    fn an_api_provider_cannot_start_a_cli_login() {
        let logins = LoginManager::new(Arc::new(PtyManager::new()));
        let error = logins.start("openai-api", None, None, None).unwrap_err();

        assert_eq!(error.code, "bad_request");
        assert!(error.message.contains("API key"));
    }

    #[test]
    fn an_unknown_login_id_is_not_found() {
        let logins = LoginManager::new(Arc::new(PtyManager::new()));

        assert_eq!(logins.status("login-404").unwrap_err().code, "not_found");
        assert_eq!(logins.submit_code("login-404", "x").unwrap_err().code, "not_found");
        assert_eq!(logins.cancel("login-404").unwrap_err().code, "not_found");
    }

    /// The paste that produced `Login failed: Request failed with status code 400`.
    ///
    /// Claude's authorize page is `…?code=true&client_id=…`, so a user who pasted the very link SDC
    /// showed them handed the CLI the literal word `true`. The dialog now refuses that paste with a
    /// sentence, and the callback address - the one that really carries a code - still works.
    #[test]
    fn the_authorize_page_is_not_mistaken_for_a_code() {
        let shown = "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a&response_type=code&state=rBjx";

        assert!(is_authorize_page(shown));
        assert!(!is_authorization_code("true"));

        let landed = "https://platform.claude.com/oauth/code/callback?code=abc1234567890abcdef&state=xyz";

        assert!(!is_authorize_page(landed));
        assert_eq!(extract_code(landed).as_deref(), Some("abc1234567890abcdef"));
        assert!(is_authorization_code("abc1234567890abcdef"));

        /* A bare code - what the page prints - is passed through whole, `#state` and all. */
        assert_eq!(extract_code("AbC123#state-4f5e"), None);
    }

    #[test]
    fn a_submission_that_is_the_authorize_page_is_refused_before_the_cli_sees_it() {
        let logins = LoginManager::new(Arc::new(PtyManager::new()));

        /* The id does not exist, so the refusal has to happen before the registry lookup - which is
           also the order that matters: the user gets the sentence, not `not_found`. */
        let error = logins
            .submit_code(
                "login-404",
                "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a",
            )
            .unwrap_err();

        assert_eq!(error.code, "bad_request");
        assert!(error.message.contains("the page to open"), "{}", error.message);
    }
}
