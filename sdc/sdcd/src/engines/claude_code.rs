//! Claude Code (master spec section 11.2).
//!
//! `claude` is the app's default engine, and `--include-partial-messages` is **mandatory**: without
//! it the CLI buffers the whole answer and prints it once, which would make the app's streaming turn
//! a comfortable fiction. With it, every chunk arrives as its own JSON line and the turn stream
//! grows the way the prototype shows.

use async_trait::async_trait;

use crate::engines::cli::{CliAdapter, CliSpec, PromptPlacement};
use crate::engines::{Engine, EngineStatus, EventSink, Prompt};

/// The one flag that makes this adapter honest.
pub const PARTIAL_MESSAGES_FLAG: &str = "--include-partial-messages";

/// The spec `claude` is invoked with. Exported so a test can assert the flags are present.
///
/// **`-p` and `--output-format stream-json` are not optional next to `--include-partial-messages`**,
/// and this is the defect that made every chat turn produce nothing (found by asking the CLI, not by
/// reading this file):
///
/// ```text
/// $ claude --include-partial-messages
/// Error: --include-partial-messages requires --print and --output-format=stream-json.
/// ```
///
/// The old spec was exactly that one flag. The CLI exited 1 with that sentence on **stderr**, the
/// adapter sent stderr to `Stdio::null()`, and the turn ended with an empty transcript - so a signed-in
/// Claude Code looked like a broken chat. Measured working form, which is what the daemon runs now:
///
/// ```text
/// $ claude -p --output-format stream-json --include-partial-messages --verbose
/// {"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"O"}}}
/// …
/// {"type":"result","subtype":"success","result":"OK","total_cost_usd":0.029,…}
/// ```
///
/// `--verbose` is what Claude Code asks for when `-p` and `stream-json` are combined.
pub const CLAUDE_SPEC: CliSpec = CliSpec {
    program: "claude",
    args: &["-p", "--output-format", "stream-json", PARTIAL_MESSAGES_FLAG, "--verbose"],
    prompt: PromptPlacement::Stdin,
    model_flag: Some("--model"),
    env: &[("NO_COLOR", "1"), ("CLAUDE_NO_UPDATE_CHECK", "1")],
    /* `--permission-mode acceptEdits` lets the checkpointed folder be edited; `--allowedTools Bash`
       adds commands; `--dangerously-skip-permissions` is the CLI's own full-autonomy switch. */
    autonomy: [
        &["--permission-mode", "acceptEdits"],
        &["--permission-mode", "acceptEdits", "--allowedTools", "Bash"],
        &["--dangerously-skip-permissions"],
    ],
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
