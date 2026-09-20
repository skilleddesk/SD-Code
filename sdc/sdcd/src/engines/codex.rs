//! Codex CLI (master spec section 11.1).
//!
//! `codex` streams JSON lines natively, so the adapter is the shared CLI one with a different
//! program. Its model list is single-model (`default`), which is why the app's tier mapping clamps
//! Fast and Balanced onto the same name (spec section 9.3).

use async_trait::async_trait;

use crate::engines::cli::{CliAdapter, CliSpec};
use crate::engines::{Engine, EngineEvent, EngineStatus, Prompt};

/// `codex --json` asks for the structured stream; `--quiet` drops the interactive banner.
pub const CODEX_SPEC: CliSpec = CliSpec {
    program: "codex",
    args: &["--json", "--quiet"],
    env: &[("NO_COLOR", "1")],
};

pub struct Codex {
    cli: CliAdapter,
}

impl Codex {
    pub fn new() -> Self {
        Self { cli: CliAdapter::new(CODEX_SPEC) }
    }
}

impl Default for Codex {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for Codex {
    fn id(&self) -> &'static str {
        "codex"
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
