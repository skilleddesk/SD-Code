//! Gemini CLI (master spec section 11.1).
//!
//! The third CLI, and the two ways it differs from the other two: its prompt goes in as an argument,
//! and it asks a trusted-folder question before it will do anything.

use async_trait::async_trait;

use crate::engines::cli::{CliAdapter, CliSpec, PromptPlacement};
use crate::engines::{Engine, EngineStatus, EventSink, Prompt};

/// `gemini -p <prompt> --output-format stream-json --skip-trust` is the headless form.
///
/// The old spec asked for `gemini --output json`, and this CLI answers it by not recognising the flag:
///
/// ```text
/// $ gemini --output json
/// Unknown argument: output        (exit 1, empty stdout)
/// ```
///
/// Its own `--help` says what the right shape is: *"Defaults to interactive mode. Use -p/--prompt for
/// non-interactive (headless) mode"*, and `-p` takes the prompt as its value - *"Appended to input on
/// stdin (if any)"* - which is why `PromptPlacement::Argument` exists: piped, the prompt would be read
/// as the answer to Gemini's own questions (its sign-in prompt, its trusted-folder prompt).
/// `--skip-trust` answers the trusted-folder one, and `stream-json` is one of the three values its
/// `-o` accepts.
///
/// The stream shape below is Gemini's published `stream-json` contract, **not** a capture: no account
/// signed in on this machine can produce one any more - Google refuses the CLI itself, and
/// [`refusal_sentence`] below has that sentence word for word. What *is* measured is the failure path:
/// signed out, it prints a plain sentence on stdout (`Opening authentication page in your browser. Do
/// you want to continue?`) and exits 42, and `explain_failure` puts that sentence in the transcript
/// instead of an empty answer.
pub const GEMINI_SPEC: CliSpec = CliSpec {
    program: "gemini",
    args: &["-p", "{prompt}", "--output-format", "stream-json", "--skip-trust"],
    prompt: PromptPlacement::Argument,
    model_flag: Some("-m"),
    env: &[("NO_COLOR", "1")],
    /* `--approval-mode auto_edit` accepts edits only; `--yolo` is Gemini's full-autonomy switch. */
    autonomy: [
        &["--approval-mode", "auto_edit"],
        &["--approval-mode", "auto_edit"],
        &["--yolo"],
    ],
};

/// What a local Gemini turn should do about signing in, decided from three facts.
///
/// The matrix was measured on Gemini CLI 0.60.0, not imagined:
///
/// | oauth credential | stored Google key | `selectedType`           | what happens                   |
/// | ---------------- | ----------------- | ------------------------ | ------------------------------ |
/// | present          | -                 | -                        | leave alone: OAuth works       |
/// | absent           | none              | -                        | leave alone: the turn's own    |
/// |                  |                   |                          | failure sentence names the fix |
/// | absent           | present           | none / `gemini-api-key`  | hand the CLI the key           |
/// | absent           | present           | `oauth-personal`         | rewrite to `gemini-api-key`,   |
/// |                  |                   |                          | then hand the CLI the key -    |
/// |                  |                   |                          | `oauth-personal` makes it      |
/// |                  |                   |                          | **ignore** `GEMINI_API_KEY`    |
/// | absent           | present           | anything else (vertex…)  | leave alone: that is the       |
/// |                  |                   |                          | person's own setup             |
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthPlan {
    LeaveAlone,
    UseKey { rewrite: bool, key: String },
}

pub fn auth_plan(credentials: bool, key: Option<String>, selected: Option<&str>) -> AuthPlan {
    if credentials {
        return AuthPlan::LeaveAlone;
    }

    let Some(key) = key else {
        return AuthPlan::LeaveAlone;
    };

    match selected {
        None | Some("gemini-api-key") => AuthPlan::UseKey { rewrite: false, key },
        /* The dead end SDC itself can have written: a sign-in that chose Login with Google and never
           finished leaves `oauth-personal` behind, and with it the CLI hangs on its auth question
           while a perfectly good key sits in the keychain. No credential means that choice never
           produced anything worth keeping. */
        Some("oauth-personal") => AuthPlan::UseKey { rewrite: true, key },
        Some(_) => AuthPlan::LeaveAlone,
    }
}

/// The environment a **local** turn of `program` needs - today only Gemini has one: `GEMINI_API_KEY`
/// from the connected Google provider, when OAuth has nothing better. A remote turn is left alone
/// (the key would travel through a command line into another machine's process list).
pub fn turn_env_for(program: &str) -> Vec<(&'static str, String)> {
    if program != "gemini" {
        return Vec::new();
    }

    let credentials = crate::providers::gemini_credentials()
        .map(|path| path.exists())
        .unwrap_or(false);
    let key = crate::auth::keychain::get(&crate::providers::key_ref("google"));
    let selected = crate::auth::cli_login::gemini_selected_type();

    match auth_plan(credentials, key, selected.as_deref()) {
        AuthPlan::LeaveAlone => Vec::new(),
        AuthPlan::UseKey { rewrite, key } => {
            if rewrite {
                if let Some(path) = crate::auth::cli_login::gemini_settings_path() {
                    let _ = crate::auth::cli_login::merge_gemini_auth(&path, "gemini-api-key");
                }
            }

            vec![("GEMINI_API_KEY", key)]
        }
    }
}

/// Google's own refusal of this CLI, turned into the way out (0.11.3).
///
/// Measured on 2026-09-26, with a sign-in that is **finished** - `~/.gemini/oauth_creds.json` is there,
/// `gemini --version` answers `0.60.0`, `settings.json` names `oauth-personal` - so this is not the
/// signed-out case [`auth_plan`] handles:
///
/// ```text
/// $ gemini -p OK --output-format stream-json --skip-trust
/// Error authenticating: IneligibleTierError: This client is no longer supported for Gemini Code Assist
/// for individuals. To continue using Gemini, please migrate to the Antigravity suite of products:
/// https://antigravity.google                            (exit 1, empty stdout, that line on stderr)
/// ```
///
/// Google has cut this client off for individual accounts. Nothing SDC can send changes that: the
/// *subscription* route is closed from the far end, and the sentence the transcript would otherwise
/// carry (`` `gemini` said: Error authenticating: … ``) names no way out of an account SDC is not
/// allowed to touch. The route that does work is the API, which SDC has had since 0.9.0 - the `google`
/// provider on `native_api`, the same models over `generativelanguage.googleapis.com`.
///
/// `None` for every other sentence, deliberately: a failure this function does not know is left in the
/// CLI's own words rather than rewritten into a story about Google.
pub fn refusal_sentence(program: &str, said: &str) -> Option<String> {
    if program != GEMINI_SPEC.program {
        return None;
    }

    let lowered = said.to_lowercase();
    let refused_by_google = lowered.contains("ineligibletiererror")
        || lowered.contains("no longer supported for gemini code assist")
        || lowered.contains("migrate to the antigravity suite");

    if !refused_by_google {
        return None;
    }

    Some(format!(
        "Google refused the `{program}` CLI itself: `{said}` - this is not a sign-in SDC can finish, and signing in again will not change it. The same models are reachable with a key instead, which is a different route to the same place: Providers → Google Gemini API (a Google AI Studio key), then pick a Gemini model from that row - every turn runs over `generativelanguage.googleapis.com` rather than through the CLI."
    ))
}

pub struct Gemini {
    cli: CliAdapter,
}

impl Gemini {
    pub fn new() -> Self {
        Self { cli: CliAdapter::new(GEMINI_SPEC) }
    }
}

impl Default for Gemini {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }

    async fn start(&self, prompt: Prompt, sink: &EventSink) {
        self.cli.run(&prompt, sink).await
    }

    async fn cancel(&self, turn_id: &str) -> bool {
        self.cli.kill(turn_id)
    }

    fn status(&self, turn_id: &str) -> EngineStatus {
        if self.cli.is_running(turn_id) {
            EngineStatus::Running
        } else {
            EngineStatus::Idle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::claude_code::{CLAUDE_SPEC, PARTIAL_MESSAGES_FLAG};
    use crate::engines::codex::CODEX_SPEC;

    /// The one assertion this file exists for (spec section 11.2: the flag is mandatory).
    #[test]
    fn claude_asks_for_partial_messages() {
        assert!(CLAUDE_SPEC.args.contains(&PARTIAL_MESSAGES_FLAG));
    }

    #[test]
    fn every_cli_is_its_own_program() {
        assert_eq!(GEMINI_SPEC.program, "gemini");
        assert_eq!(CODEX_SPEC.program, "codex");
    }

    /// The measured matrix (0.60.0): a finished OAuth wins, a stored key rescues the `oauth-personal`
    /// dead end, and anyone who chose Vertex or GCA on purpose is left alone.
    #[test]
    fn the_auth_plan_matches_what_the_cli_was_measured_to_do() {
        let key = || Some("AIza-real".to_string());

        /* OAuth finished: nothing to do, whatever else is true. */
        assert_eq!(auth_plan(true, key(), Some("oauth-personal")), AuthPlan::LeaveAlone);

        /* No key: nothing to hand over; the turn's failure sentence names the fix. */
        assert_eq!(auth_plan(false, None, Some("oauth-personal")), AuthPlan::LeaveAlone);
        assert_eq!(auth_plan(false, None, None), AuthPlan::LeaveAlone);

        /* A key with no auth choice, or with the matching one: hand it over as-is. */
        assert_eq!(auth_plan(false, key(), None), AuthPlan::UseKey { rewrite: false, key: "AIza-real".into() });
        assert_eq!(
            auth_plan(false, key(), Some("gemini-api-key")),
            AuthPlan::UseKey { rewrite: false, key: "AIza-real".into() }
        );

        /* The dead end: oauth chosen, never finished, key available - rewrite and hand it over.
           `oauth-personal` makes the CLI ignore `GEMINI_API_KEY`, so without the rewrite the turn
           hangs on the auth question while a good key sits in the keychain. */
        assert_eq!(
            auth_plan(false, key(), Some("oauth-personal")),
            AuthPlan::UseKey { rewrite: true, key: "AIza-real".into() }
        );

        /* Somebody's own Vertex or GCA setup is not SDC's to rewrite. */
        assert_eq!(auth_plan(false, key(), Some("vertex-ai")), AuthPlan::LeaveAlone);
    }

    /// Only Gemini has turn environment; the other CLIs must get nothing extra.
    #[test]
    fn no_other_cli_gets_a_key() {
        assert!(turn_env_for("claude").is_empty());
        assert!(turn_env_for("codex").is_empty());
    }

    /// Google's refusal, word for word from the capture in `refusal_sentence`'s doc. Two properties,
    /// and both matter: the sentence names the route that still works (the `google` API key), and it
    /// keeps Google's own words - a failure quietly rewritten would be the same lie in the other
    /// direction, and this one is the *only* failure that is allowed to be rewritten at all.
    #[test]
    fn googles_refusal_names_the_route_that_still_works() {
        let captured = "Error authenticating: IneligibleTierError: This client is no longer supported for Gemini Code Assist for individuals. To continue using Gemini, please migrate to the Antigravity suite of products: https://antigravity.google";

        let sentence = refusal_sentence("gemini", captured).expect("the measured refusal is recognised");

        assert!(sentence.contains("IneligibleTierError"), "{sentence}");
        assert!(sentence.contains("Google Gemini API"), "{sentence}");
        assert!(sentence.contains("generativelanguage.googleapis.com"), "{sentence}");

        /* Another CLI, and any other Gemini failure, stay in their own words. */
        assert_eq!(refusal_sentence("claude", captured), None);
        assert_eq!(refusal_sentence("codex", captured), None);
        assert_eq!(refusal_sentence("gemini", "Unknown argument: output"), None);
    }
}
