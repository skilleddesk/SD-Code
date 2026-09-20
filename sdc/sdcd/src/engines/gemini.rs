//! Gemini CLI (master spec section 11.1).
//!
//! The third CLI, and the reason `cli.rs` exists: the only thing that differs from Codex is the
//! program name and the flag that asks for the structured stream.

use async_trait::async_trait;

use crate::engines::cli::{CliAdapter, CliSpec};
use crate::engines::{Engine, EngineEvent, EngineStatus, Prompt};

/// `gemini --output json` streams one object per line.
pub const GEMINI_SPEC: CliSpec = CliSpec {
    program: "gemini",
    args: &["--output", "json"],
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
