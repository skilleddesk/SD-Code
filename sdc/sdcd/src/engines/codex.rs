//! Codex CLI (master spec section 11.1).
//!
//! `codex` streams JSON lines natively, so the adapter is the shared CLI one with a different
//! program. Its model list is single-model (`default`), which is why the app's tier mapping clamps
//! Fast and Balanced onto the same name (spec section 9.3).

use async_trait::async_trait;

use crate::engines::cli::{CliAdapter, CliSpec, PromptPlacement};
use crate::engines::{Engine, EngineEvent, EngineStatus, Prompt};

/// `codex exec --json -` is the non-interactive form, measured against the installed CLI.
///
/// The old spec asked for `codex --json --quiet`, which this CLI does not have at all:
///
/// ```text
/// $ codex --json --quiet
/// error: unexpected argument '--json' found        (exit 2, empty stdout)
/// ```
///
/// `exec` is Codex's "run non-interactively, without the TUI" subcommand and `-` is how it reads the
/// prompt from stdin. `--skip-git-repo-check` keeps a folder that is not a git repository from ending
/// the turn before the model is asked anything. What it answers, captured line for line:
///
/// ```text
/// {"type":"thread.started","thread_id":"…"}
/// {"type":"turn.started"}
/// {"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}
/// {"type":"turn.completed","usage":{"input_tokens":12926,…}}
/// ```
pub const CODEX_SPEC: CliSpec = CliSpec {
    program: "codex",
    args: &["exec", "--json", "--skip-git-repo-check", "-"],
    prompt: PromptPlacement::Stdin,
    model_flag: Some("-m"),
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
