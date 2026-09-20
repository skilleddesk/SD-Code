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
}

/// The three subscription providers, as data.
///
/// `codex` takes a subcommand; `claude` and `gemini` put login behind their own prompt, so the daemon
/// types the command for the user (`/login`, and the menu's first choice). If a CLI changes any of
/// this, this table is the only thing that has to change - and the raw output the UI shows is what
/// tells you it has.
pub const RECIPES: &[LoginRecipe] = &[
    LoginRecipe {
        provider_id: "claude",
        label: "Claude Code",
        program: "claude",
        args: &[],
        pump: &["/login"],
        success: &["successfully logged in", "login successful", "logged in as", "you are now logged in"],
        note: "Claude Code opens the approval page itself; the link is also printed here so it can be copied.",
    },
    LoginRecipe {
        provider_id: "openai",
        label: "Codex",
        program: "codex",
        args: &["login"],
        pump: &[],
        success: &["successfully logged in", "login successful", "authenticated"],
        note: "Codex waits for its browser callback; if the browser cannot reach it, paste the code from the page instead.",
    },
    LoginRecipe {
        provider_id: "gemini",
        label: "Gemini",
        program: "gemini",
        args: &[],
        pump: &["1"],
        success: &["successfully logged in", "login successful", "authenticated", "signed in"],
        note: "Gemini asks how to sign in first; the daemon answers with the browser option, which is the first choice.",
    },
];

/// The recipe for a provider, or `None` when that provider does not sign in through a CLI.
pub fn recipe(provider_id: &str) -> Option<&'static LoginRecipe> {
    RECIPES.iter().find(|recipe| recipe.provider_id == provider_id)
}

/// The first URL in some output. A login page is the only URL a CLI prints on purpose, and the first
/// one wins because a later address is usually the redirect that already carried the code.
pub fn extract_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|token| token.trim_matches(|character: char| "()[]<>,;\"'".contains(character)))
        .find(|token| token.starts_with("https://") || token.starts_with("http://localhost"))
        .map(str::to_string)
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
        let opened = self.pty.open(&program, &args, None).map_err(|error| {
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

    /// Hands the pasted code to the CLI, which is the only thing that can use it. SDC does not store
    /// it, log it, or send it anywhere else - the credential stays in the CLI's own store.
    pub fn submit_code(&self, id: &str, code: &str) -> Result<Value, ErrorObject> {
        if code.trim().is_empty() {
            return Err(ErrorObject::bad_request("`code` is empty"));
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
        let value = extract_code(code).unwrap_or_else(|| code.trim().to_string());

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
}
