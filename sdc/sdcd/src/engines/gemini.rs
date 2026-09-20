//! Gemini CLI (master spec section 11.1).
//!
//! The third CLI, and the two ways it differs from the other two: its prompt goes in as an argument,
//! and it asks a trusted-folder question before it will do anything.

use async_trait::async_trait;

use crate::engines::cli::{CliAdapter, CliSpec, PromptPlacement};
use crate::engines::{Engine, EngineEvent, EngineStatus, Prompt};

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
/// The stream shape below is Gemini's published `stream-json` contract, **not** a capture: this machine
/// has no Gemini account signed in, and guessing which of its lines matter would be the same mistake
/// this file was written to fix. What is certain is the failure path - not signed in, it prints a plain
/// sentence on stdout (`Opening authentication page in your browser. Do you want to continue?`) and
/// exits 42, and `explain_failure` puts that sentence in the transcript instead of an empty answer.
pub const GEMINI_SPEC: CliSpec = CliSpec {
    program: "gemini",
    args: &["-p", "{prompt}", "--output-format", "stream-json", "--skip-trust"],
    prompt: PromptPlacement::Argument,
    model_flag: Some("-m"),
    env: &[("NO_COLOR", "1")],
};

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

    async fn start(&self, prompt: Prompt) -> Vec<EngineEvent> {
        self.cli.run(&prompt).await
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
}
