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
//! | `session limit` / `usage limit`         | "the plan's limit is reached, until the time it names" |
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

/// When a plan-limit line says the window refills, phrased to follow "It refills": `at 12:50am (Asia/Dhaka)`,
/// `after 2h`, `at Oct 27th, 2026 3:00 AM`. The card quotes whichever one arrived, because "wait a while" is not
/// what a person needs to hear when the hour is already in the text.
fn refills_at(output: &str) -> Option<String> {
    /* The marker, and the preposition the sentence needs back: `resets 12:50am` -> "at 12:50am", but
       `reset after 2h` -> "after 2h". */
    const MARKERS: [(&str, &str); 3] = [("resets ", "at "), ("reset after ", "after "), ("try again at ", "at ")];

    for line in output.lines() {
        let lower = line.to_lowercase();

        for (marker, preposition) in MARKERS {
            let Some(at) = lower.find(marker) else { continue };

            /* `get` rather than a slice: `to_lowercase` can change a byte length, and a rule that panicked on an
               unusual character would be worse than a card without a time. */
            let clause = line
                .get(at + marker.len()..)
                .unwrap_or("")
                .trim()
                .trim_end_matches(['.', ',', '!'])
                .trim();

            if !clause.is_empty() {
                return Some(format!("{preposition}{clause}"));
            }
        }
    }

    None
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
    /* PADMA P0-5 (G5): the request was longer than the model's window. SDC shortens it and goes on; this is never a failure without a rule. */
    if haystack.contains("contextoverflow") || haystack.contains("needssplit") {
        return rule(
            "The step was too large for the model's window",
            format!("`{first_line}`. SDC shortens the request in steps (older tool output first, then the project notes, then the answer's reserve) and splits the work; a step that still does not fit is marked blocked (too large) and the rest goes on."),
            true,
            "context-overflow",
        );
    }

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

    /* 0.15.6: Alibaba Model Studio checks every reply before it is sent, and blocks one its classifier
       dislikes: `<400> InternalError.Algo.DataInspectionFailed: Output data may contain inappropriate
       content.` It is the provider's filter, not the network or SDC. The agent already asked again three
       times with a shorter reply by the time this is shown. */
    if crate::agent::content_filtered(&haystack) {
        return rule(
            "The provider's content filter blocked the reply",
            format!("`{first_line}`. The model provider checks each reply before sending it, and it blocked this one - usually a long reply that quotes logs, error text or legal wording. Nothing is broken and the work so far is kept. SDC asked again three times with a shorter reply; send \"continue\" to go on, or pick a model from another provider (DeepSeek, Claude) for this chat."),
            false,
            "content-filter",
        );
    }

    /* 0.15.10: Codex run without a ChatGPT sign-in - it retries ten times and ends with this. The raw line
       named a websocket URL and a Cloudflare ray id; what the person has to do is sign in. */
    if haystack.contains("missing bearer or basic authentication") {
        return rule(
            "Codex is not signed in",
            format!("`{first_line}`. The Codex CLI on this machine has no ChatGPT sign-in. Open Providers → OpenAI (subscription) → Connect, approve the page in your browser, then send the message again. For a chat on a VPS, sign Codex in on that server (`codex login --device-auth`)."),
            false,
            "codex-signed-out",
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
            format!("`{first_line}`. The program is not installed; Settings → Environment installs it in one click."),
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

    /* 1.0.0: a CLI too old for the model that was picked. Measured 2026-10-28: Claude Code 2.1.278 answered
       `claude-opus-5-5` with "version 2.1.280 or newer is required". SDC updates the CLI itself (see
       `host::tools::refresh_clis`), so the sentence says it is already doing so. */
    if haystack.contains("or newer is required")
        || haystack.contains("does not support this model")
        || haystack.contains("isn't described by this version")
        || haystack.contains("please update the cli")
    {
        return rule(
            &format!("{tool} is too old for this model"),
            format!("`{first_line}`. SDC is updating the CLI now; send the message again in a minute."),
            true,
            "cli-outdated",
        );
    }

    /* 1.0.0: Google closed Gemini CLI's "Login with Google" for personal accounts (Google AI Pro included) -
       `IneligibleTierError: This client is no longer supported for Gemini Code Assist for individuals`. */
    if haystack.contains("ineligibletier") || haystack.contains("no longer supported for gemini code assist") {
        return rule(
            "Google no longer serves Gemini CLI to personal accounts",
            format!(
                "`{first_line}`. Google refuses this sign-in for every personal Google account, Google AI Pro included. \
                 The same models (gemini-4-argon too) work through the Google Gemini API card with a free key from \
                 aistudio.google.com/apikey."
            ),
            false,
            "gemini-refused",
        );
    }

    /* 3.3: a *plan's* window, not the API's. When the subscription's hours are spent the CLI refuses the turn with
       one line - `You've hit your session limit, resets 12:50am (Asia/Dhaka)` - and the card said "claude_code
       failed ... has no rule yet". Nothing is broken and nothing is lost: the window has to refill, and the line says
       when. `gemini` and `codex` word the same fact differently, so all their shapes are matched here. */
    if haystack.contains("session limit")
        || haystack.contains("usage limit")
        || haystack.contains("weekly limit")
        || haystack.contains("capacity on this model")
        || haystack.contains("quota will reset")
        || haystack.contains("exhausted your capacity")
    {
        let when = match refills_at(output) {
            Some(when) => format!("It refills {when}."),
            None => "The line names no time; that provider's usage page shows when it refills.".to_string(),
        };

        return rule(
            &format!("{tool}'s plan limit is reached"),
            format!(
                "`{first_line}`. The plan's window is spent, so the CLI stopped before the work began - nothing is \
                 broken and nothing was spent twice. {when} Send the message again after that, or pick another engine \
                 or model in the message box's model menu: the conversation stays where it is, and a `local` model \
                 keeps working meanwhile."
            ),
            false,
            "plan-limit",
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

    #[test]
    fn an_outdated_cli_and_googles_gemini_refusal_have_rules() {
        let old = translate("claude_code", "API Error: 400 Claude Code 2.1.278 does not support this model; version 2.1.280 or newer is required. Run 'claude update'");

        assert_eq!(old.rule, "cli-outdated");

        let refused = translate("gemini", "Error authenticating: IneligibleTierError: This client is no longer supported for Gemini Code Assist for individuals.");

        assert_eq!(refused.rule, "gemini-refused");
        assert!(refused.explanation.contains("aistudio.google.com"));
    }

    #[test]
    fn a_context_overflow_always_has_a_rule() {
        let got = translate("ollama", "ContextOverflow: this step is about 12397 tokens with 4096 reserved for the answer, and the model's window is 16384; SDC splits the work instead of sending it");

        assert_eq!(got.rule, "context-overflow");
        assert!(!got.explanation.contains("no rule yet"));
    }

    /// 0.15.10: a signed-out Codex says so, instead of a websocket URL and a ray id.
    #[test]
    fn a_signed_out_codex_is_named() {
        let translated = translate(
            "codex",
            "unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses",
        );

        assert_eq!(translated.rule, "codex-signed-out");
        assert!(translated.explanation.contains("Connect"), "{}", translated.explanation);
    }

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
            "`claude` is not installed. SDC installs it in one click: Settings → Environment → Install (or the provider's Connect card).",
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
    /// 3.3: the card that was reported - `claude_code failed ... has no rule yet` - on the line the CLI prints.
    #[test]
    fn a_plan_limit_names_the_time_it_refills() {
        let translated = translate("claude_code", "You've hit your session limit, resets 12:50am (Asia/Dhaka)");

        assert_eq!(translated.rule, "plan-limit");
        assert_eq!(translated.title, "claude_code's plan limit is reached");
        assert!(translated.explanation.contains("refills at 12:50am (Asia/Dhaka)"), "{}", translated.explanation);
        assert!(!translated.explanation.contains("no rule yet"));
        assert!(!translated.fixable, "waiting is not something an agent can do instead of the person");

        /* The line exactly as `claude` printed it, middle dot and all - this is the report. */
        let reported = translate("claude_code", "You've hit your session limit · resets 12:50am (Asia/Dhaka)");
        assert_eq!(reported.rule, "plan-limit");
        assert!(reported.explanation.contains("refills at 12:50am (Asia/Dhaka)"), "{}", reported.explanation);

        /* The same fact in the other two CLIs' words, and in a line that names no time at all. */
        let gemini = translate("gemini", "You have exhausted your capacity on this model. Your quota will reset after 2h.");
        assert_eq!(gemini.rule, "plan-limit");
        assert!(gemini.explanation.contains("refills after 2h"), "{}", gemini.explanation);

        let codex = translate("codex", "You've hit your usage limit. Upgrade to Pro, or try again at Oct 27th, 2026 3:00 AM.");
        assert_eq!(codex.rule, "plan-limit");
        assert!(codex.explanation.contains("refills at Oct 27th, 2026 3:00 AM"), "{}", codex.explanation);

        let untimed = translate("claude_code", "Claude AI usage limit reached|1729");
        assert_eq!(untimed.rule, "plan-limit");
        assert!(untimed.explanation.contains("usage page"), "{}", untimed.explanation);

        /* An API 429 is a different wait - seconds, not hours - and keeps the rule it already had. */
        assert_eq!(translate("deepseek", "429 Too Many Requests: rate limit reached, retry later").rule, "budget");
    }

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
