//! Claude Code (master spec section 11.2).
//!
//! `claude` is the app's default engine, and `--include-partial-messages` is **mandatory**: without
//! it the CLI buffers the whole answer and prints it once, which would make the app's streaming turn
//! a comfortable fiction. With it, every chunk arrives as its own JSON line and the turn stream
//! grows the way the prototype shows.

use async_trait::async_trait;

use crate::engines::cli::{CliAdapter, CliSpec};
use crate::engines::{Engine, EngineEvent, EngineStatus, Prompt};

/// The one flag that makes this adapter honest.
pub const PARTIAL_MESSAGES_FLAG: &str = "--include-partial-messages";

/// The spec `claude` is invoked with. Exported so a test can assert the flag is present.
pub const CLAUDE_SPEC: CliSpec = CliSpec {
    program: "claude",
    args: &[PARTIAL_MESSAGES_FLAG],
    env: &[("NO_COLOR", "1"), ("CLAUDE_NO_UPDATE_CHECK", "1")],
};

pub struct ClaudeCode {
    cli: CliAdapter,
}

impl ClaudeCode {
    pub fn new() -> Self {
        Self { cli: CliAdapter::new(CLAUDE_SPEC) }
    }
}

impl Default for ClaudeCode {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude_code"
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
