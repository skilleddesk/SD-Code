//! The error translator - one of the five differentiators (master spec section 14.9).
//!
//! A tool error arrives as a wall of text: a stack trace, a compiler's caret diagram, a shell's exit
//! status. The translator turns it into **one plain sentence and a next step**, which is what the
//! error card shows and what `Fix this` seeds a turn with:
//!
//! | what the tool said                      | what the card says                            |
//! | --------------------------------------- | ---------------------------------------------- |
//! | `command not found` / `ENOENT`          | "X is not installed… the doctor can install it" |
//! | `EADDRINUSE` / `address already in use` | "A port is already in use"                      |
//! | `Cannot find module`                    | "A dependency is missing"                       |
//! | `error TS####`                          | "TypeScript rejected the code"                   |
//! | `AssertionError` / expected-received    | "A test expected something else"                 |
//! | `EACCES` / `permission denied`          | "The tool was not allowed to touch a file"       |
//! | `blocked_path`                          | "The file guard refused that path"               |
//! | `budget` / `rate limit`                 | "The provider stopped the turn"                  |
//!
//! It is deliberately a *table*: this is the one place a new failure becomes a friendly sentence, and
//! the `fixable` flag it returns is what decides whether the card offers `Fix this`.

use serde_json::{json, Value};

/// One translated failure, ready for an `ErrorRaised` event.
#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    pub title: String,
    pub explanation: String,
    /// True when an agent can plausibly act on this by itself.
    pub fixable: bool,
    /// The pattern that matched, so the UI - and a test - can see the reasoning.
    pub rule: &'static str,
}

impl Translation {
    pub fn to_event_payload(&self, session_id: &str, turn_id: Option<&str>, source: Option<&str>) -> Value {
        json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "title": self.title,
            "explanation": self.explanation,
            "source": source,
            "fixable": self.fixable,
        })
    }
}

fn rule(title: &str, explanation: String, fixable: bool, rule: &'static str) -> Translation {
    Translation { title: title.to_string(), explanation, fixable, rule }
}

/// Translates a tool's output. `tool` names the thing that failed (`npm test`, `tsc`, `claude`) and
/// only words the sentence; the text is what decides the rule.
pub fn translate(tool: &str, output: &str) -> Translation {
    let haystack = output.to_lowercase();
    let first_line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("the tool produced no output")
        /* The line is quoted in backticks below; one of its own (`gemini` said: …) broke the quote. */
        .replace('`', "");

    /* 0.11.8: an `ssh` that was refused at the door is a connection fact, not a file one. The log had
       `Permission denied (keyboard-interactive)` explained as "the path is not writable". */
    if haystack.contains("(keyboard-interactive)") || haystack.contains("permission denied (publickey") {
        return rule(
            "Not signed in to the host",
            format!("`{first_line}`. SDC's signed-in connection to that machine is closed, so nothing ran there. Sign in again (the banner's Reconnect, or the host's card), then send the message again."),
            false,
            "ssh-signed-out",
        );
    }

    if haystack.contains("host key verification failed") {
        return rule(
            "The host's key is not the pinned one",
            format!("`{first_line}`. The machine answered with a key SDC did not pin, so nothing was sent. Add the host again to see and pin the key it presents now."),
            false,
            "ssh-host-key",
        );
    }

    /* 0.12.4: the engine's own sign-in, not SSH's. `claude` on a VPS keeps its OAuth token in that
       machine's `~/.claude`, apart from this PC's - the report was signed in here, expired there. */
    if haystack.contains("failed to authenticate")
        || haystack.contains("oauth session expired")
        || haystack.contains("oauth token has expired")
        || haystack.contains("please run /login")
    {
        return rule(
            &format!("{tool} is signed out where this chat runs"),
            format!("`{first_line}`. The CLI's sign-in on the machine this chat runs on has expired. A sign-in on this PC does not reach a VPS - each machine keeps its own. Open a terminal on that machine, run `claude` and type `/login` (or pick another engine), then send the message again."),
            false,
            "engine-signed-out",
        );
    }

    /* 0.12.4: a provider that refused the stored key - Alibaba says `Invalid API-key provided`, OpenAI
       and DeepSeek `Incorrect API key`. Nothing an agent can fix: the key itself has to change. */
    if haystack.contains("invalid api-key")
        || haystack.contains("invalid api key")
        || haystack.contains("incorrect api key")
        || haystack.contains("invalid_api_key")
    {
        return rule(
            "The provider refused the API key",
            format!("`{first_line}`. The key saved for this provider is wrong, expired or was made for another region or workspace. Open Providers, paste a fresh key on that provider's card and press Test, then send the message again."),
            false,
            "api-key-rejected",
        );
    }

    /* 0.12.5: a turn on a provider with no key stored at all. */
    if haystack.contains("no api key for") {
        return rule(
            "No API key is saved for this provider",
            format!("`{first_line}`. Open Providers, paste the key on that provider's card and press Test - or pick a model from a provider that is connected."),
            false,
            "api-key-missing",
        );
    }

    /* `env: 'gemini': No such file or directory` - a CLI that is not installed on the machine the chat
       runs on (a VPS, usually). */
    if let Some(program) = haystack
        .split("env: '")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .filter(|_| haystack.contains("no such file or directory"))
    {
        return rule(
            &format!("{program} is not installed where this chat runs"),
            format!("`{first_line}`. The chat's folder is on a machine without `{program}`. Install it there, or pick an engine that is."),
            false,
            "missing-program-remote",
        );
    }

    if haystack.contains("command not found")
        || haystack.contains("enoent")
        || haystack.contains("is not recognized")
        || haystack.contains("is not installed")
        || haystack.contains("not on path")
    {
        return rule(
            &format!("{tool} could not find a program it needs"),
            format!("`{first_line}`. The program is not installed or not on PATH; the environment doctor can install it."),
            true,
            "missing-program",
        );
    }

    if haystack.contains("eaddrinuse") || haystack.contains("address already in use") {
        return rule(
            "A port is already in use",
            format!("`{first_line}`. Another process holds the port the server needs, so the server never started. Stop that process, or change the port."),
            true,
            "port-in-use",
        );
    }

    if haystack.contains("cannot find module") || haystack.contains("module_not_found") {
        return rule(
            "A dependency is missing",
            format!("`{first_line}`. The package is not installed in this project; the install step for its package manager will fix it."),
            true,
            "missing-module",
        );
    }

    if haystack.contains("error ts") {
        return rule(
            "TypeScript rejected the code",
            format!("`{first_line}`. The type checker stopped on a line it cannot prove; the fix belongs in the type, not in the check."),
            true,
            "typescript",
        );
    }

    if haystack.contains("assertionerror") || (haystack.contains("expected") && haystack.contains("received")) {
        return rule(
            "A test expected something else",
            format!("`{first_line}`. The behaviour and the expectation disagree; the failing assertion names the value that was produced."),
            true,
            "test-failure",
        );
    }

    if haystack.contains("eacces") || haystack.contains("permission denied") || haystack.contains("access is denied") {
        return rule(
            "The tool was not allowed to touch a file",
            format!("`{first_line}`. The path is not writable by this user; nothing was changed."),
            false,
            "permission-denied",
        );
    }

    if haystack.contains("blocked_path") {
        return rule(
            "The file guard refused that path",
            format!("`{first_line}`. `.env`, `*.pem`, `id_rsa` and the daemon's own directory are never read or written."),
            false,
            "blocked-path",
        );
    }

    if haystack.contains("budget") || haystack.contains("rate limit") {
        return rule(
            "The provider stopped the turn",
            format!("`{first_line}`. The request hit a limit rather than a bug; waiting or switching tier is the way forward."),
            false,
            "budget",
        );
    }

    /* The CLI adapter's own sentence for a CLI that stopped on an interactive question (a sign-in
       prompt, mostly). The sentence already names the fix, so it is kept whole. */
    if haystack.contains("stopped to ask") {
        return rule(
            if haystack.contains("not signed in") { "A sign-in is needed first" } else { "The engine stopped to ask a question" },
            first_line,
            false,
            "needs-person",
        );
    }

    rule(
        &format!("{tool} failed"),
        format!("`{first_line}`. This failure has no rule yet, so it is shown as the tool reported it."),
        false,
        "untranslated",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three sentences the report's own log got wrong (0.11.8).
    #[test]
    fn connection_failures_are_not_file_failures() {
        let signed_out = translate("gemini", "`gemini` said: deploy@203.0.113.10: Permission denied (keyboard-interactive).");

        assert_eq!(signed_out.rule, "ssh-signed-out");
        assert!(!signed_out.explanation.contains("``"), "{}", signed_out.explanation);

        let missing = translate("gemini", "`gemini` said: env: 'gemini': No such file or directory");

        assert_eq!(missing.rule, "missing-program-remote");
        assert!(missing.title.starts_with("gemini is not installed"), "{}", missing.title);

        /* A real file permission problem keeps its own sentence. */
        assert_eq!(translate("npm", "EACCES: permission denied, open '/srv/app/x'").rule, "permission-denied");
    }

    /// The 0.12.4 report: signed in to `claude` on the PC, the chat ran on the VPS where it had expired.
    #[test]
    fn an_expired_engine_sign_in_says_which_machine() {
        let translated = translate("claude_code", "Failed to authenticate: OAuth session expired and could not be refreshed");

        assert_eq!(translated.rule, "engine-signed-out");
        assert!(translated.explanation.contains("/login"), "{}", translated.explanation);
        assert!(translated.explanation.contains("VPS"), "{}", translated.explanation);

        /* The same round's second card: Alibaba refusing the stored key. */
        let refused = translate(
            "native_api",
            "Invalid API-key provided. For details, see: https://www.alibabacloud.com/help/en/model-studio/error-code#apikey-error (401)",
        );

        assert_eq!(refused.rule, "api-key-rejected");
    }

    #[test]
    fn a_missing_program_is_explained_and_fixable() {
        let translated = translate("claude", "claude: command not found");

        assert_eq!(translated.rule, "missing-program");
        assert!(translated.fixable);
        assert!(translated.explanation.contains("not installed"));
    }

    /// The wording the CLI adapters themselves use (`engines/cli.rs`), so a missing `claude` is not
    /// reported as "claude_code failed".
    #[test]
    fn the_adapters_own_missing_program_message_is_recognised() {
        let translated = translate(
            "claude_code",
            "`claude` is not installed or not on PATH. Install it, then run the environment doctor.",
        );

        assert_eq!(translated.rule, "missing-program");
        assert!(translated.title.contains("could not find a program"));
    }

    #[test]
    fn a_busy_port_is_explained() {
        let translated = translate("npm run dev", "Error: listen EADDRINUSE: address already in use :::3000");

        assert_eq!(translated.rule, "port-in-use");
        assert!(translated.title.contains("port"));
    }

    #[test]
    fn a_typescript_error_and_a_test_failure_are_told_apart() {
        let typescript = translate("tsc", "src/a.ts(3,5): error TS2322: Type 'string' is not assignable");

        assert_eq!(typescript.rule, "typescript");
        assert_eq!(translate("npm test", "AssertionError: expected 6 to be 5").rule, "test-failure");
    }

    #[test]
    fn the_blocked_path_rule_says_why() {
        let translated = translate("read", "blocked_path: /work/.env was refused");

        assert_eq!(translated.rule, "blocked-path");
        assert!(!translated.fixable);
        assert!(translated.explanation.contains(".env"));
    }

    #[test]
    fn a_rate_limit_is_explained_as_the_provider_stopping_the_turn() {
        let translated = translate("claude_code", "429: rate limit exceeded, please retry later");

        assert_eq!(translated.rule, "budget");
        assert!(!translated.fixable);
        assert!(translated.title.contains("provider stopped the turn"));
    }

    /// The other wording a provider uses for the same thing - a token or spend cap, not a request rate.
    #[test]
    fn a_budget_cap_matches_the_same_rule_as_a_rate_limit() {
        assert_eq!(translate("claude_code", "monthly budget exceeded").rule, "budget");
    }

    /// The adapter's sentence for a CLI stuck on its sign-in question (0.10.0) keeps its own words -
    /// they already name the fix - under a title that says what kind of problem this is.
    #[test]
    fn a_cli_waiting_for_a_sign_in_gets_its_own_rule() {
        let translated = translate(
            "gemini",
            &crate::engines::cli::waiting_for_a_person(
                "gemini",
                "Opening authentication page in your browser. Do you want to continue? [Y/n]:",
            ),
        );

        assert_eq!(translated.rule, "needs-person");
        assert!(translated.title.contains("sign-in"), "{}", translated.title);
        assert!(translated.explanation.contains("Settings"), "{}", translated.explanation);
    }

    #[test]
    fn an_unknown_failure_still_produces_a_sentence() {
        let translated = translate("mystery", "something nobody has seen before");

        assert_eq!(translated.rule, "untranslated");
        assert!(translated.explanation.contains("something nobody has seen before"));
    }

    #[test]
    fn an_empty_output_is_handled() {
        assert!(translate("silent", "   \n  ").explanation.contains("no output"));
    }

    #[test]
    fn the_event_payload_has_the_shape_the_card_reads() {
        let payload = translate("tsc", "error TS1").to_event_payload("s1", Some("t1"), Some("a.ts:3"));

        assert_eq!(payload["sessionId"], json!("s1"));
        assert_eq!(payload["turnId"], json!("t1"));
        assert_eq!(payload["fixable"], json!(true));
    }
}
