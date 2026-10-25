//! The tools the agent has, and what each one does to the window.
//!
//! v4 had eight, deliberately (docs/ROADMAP-v4.md, "risks"): every tool is one more thing a model can get
//! wrong and one more thing a person has to be able to read in a tool card. 0.13 adds the ones whose
//! absence cost more than their risk - each measured against what Claude Code's agent does with its own:
//! `grep`/`glob` (find by pattern, not by a dozen listings), `read_file` by line range, `web_fetch`/
//! `web_search` (read the docs instead of guessing), `start_process`/`process_output`/`stop_process`
//! (a dev server that keeps running), `ask_user` (a question with choices, mid-task), `remember`,
//! `task` (sub-agents that explore in parallel), and - for a model that can see - `view_image` and
//! `screenshot`. The first eight:
//!
//! | tool           | card   | asks first                      |
//! | -------------- | ------ | ------------------------------- |
//! | `read_file`    | Read   | never                           |
//! | `list_dir`     | Read   | never                           |
//! | `search`       | Read   | never                           |
//! | `git_diff`     | Read   | never                           |
//! | `write_file`   | Edit   | in Simple (`ask`)               |
//! | `edit_file`    | Edit   | in Simple (`ask`)               |
//! | `run_command`  | Run    | in Simple and Pro, always when the line looks dangerous |
//! | `update_plan`  | plan   | never                           |
//!
//! A checkpoint is written before the first Edit or Run of a turn by the daemon's turn loop (P5), not
//! here: the card's `ToolStarted` is what triggers it, so the checkpoint exists before the change does.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{json, Value};

use super::dialect::{ToolSpec, ToolUse};
use super::gate::{self, Autonomy, Decision};
use super::workspace::{Workspace, DEFAULT_COMMAND};
use crate::engines::{EngineEvent, EventSink};

/// The most text one tool answer hands back to the model. Longer output keeps its head and its tail,
/// which is where a build's command and its error are.
pub const RESULT_CAP: usize = 24_000;

/// Lines of a diff drawn on an Edit card; the rest is counted, not dropped silently.
const DIFF_CARD_LINES: usize = 160;

/// What the model of a turn can be given (0.13): a model that sees gets the image tools, and a
/// sub-agent gets only the tools that read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caps {
    pub vision: bool,
    pub subagent: bool,
    /// The model writes Codex's patch format (GPT, o-series, Codex): it gets `apply_patch` (0.14).
    pub patch: bool,
    /// The model runs on a local server (Ollama). A switch that only applies to local models (`v3::Scope::LocalOnly`) adds its tools
    /// for such a model and for no other: `false` - the default - is any cloud, API or CLI model.
    pub local: bool,
}

impl Caps {
    /// The model these caps are for, as a switch's scope sees it.
    pub fn model(&self) -> crate::v3::ModelCtx {
        /* The class matters to a `Classes` scope only; the tool list is built before the window is known to be small or large. */
        crate::v3::ModelCtx::new(self.local)
    }
}

/// The tools a sub-agent (`task`) may use: they read, search and look - they never change anything.
/// Tools whose result is shown on their card (0.19): what the model read, found or fetched. Commands
/// already stream their own output, and edits carry a diff.
const SEEN: &[&str] = &["read_file", "list_dir", "search", "grep", "glob", "git_diff", "web_fetch", "web_search", "process_output"];

pub const READ_ONLY: &[&str] = &["read_file", "list_dir", "search", "grep", "glob", "git_diff", "web_fetch", "web_search", "view_image", "process_output", "spec_read", "obs_read"];

/// Every tool, for a turn whose model cannot see and which is not a sub-agent.
pub fn specs() -> Vec<ToolSpec> {
    specs_for(Caps::default())
}

/// The tools for a turn with `caps`.
pub fn specs_for(caps: Caps) -> Vec<ToolSpec> {
    let mut all = base_specs();

    all.extend(more_specs());

    /* v3 W4: where the project's declarations are (switch `v3.intel` - two tools are added). */
    if crate::v3::on("intel") {
        all.extend(intel_specs());
    }

    if crate::v3::on("intel") && crate::v3::on("graph.v1") {
        all.push(code_graph_spec());
    }

    if crate::v3::on_in("sme.retrieval", &caps.model()) {
        all.push(ranked_search_spec());
    }

    /* N3-1 / N3-3: what was kept in the vault (a whole build log, a long prompt) is fetched by id and line range */
    if crate::v3::on_in("nr.error_slice", &caps.model()) || crate::v3::on_in("nr.fresh_step", &caps.model()) {
        all.push(spec_read_spec());
    }

    /* PADMA P0-5: the rest of a tool output that was cut to the window's 10% is read back by its `‹obs:N›` number. */
    if crate::v3::on("padma.obs_cap") {
        all.push(obs_read_spec());
    }

    if caps.vision {
        all.extend(vision_specs());
    }

    if caps.subagent {
        all.retain(|spec| READ_ONLY.contains(&spec.name));
    } else {
        if crate::v3::on("edit.symbol_patch") && crate::v3::on("graph.v1") {
            all.push(patch_symbol_spec());
        }

        /* N4-2: the new definition comes in the message as a plain code block, not in the arguments */
        if crate::v3::on_in("nr.symbol_patch", &caps.model()) {
            all.push(replace_symbol_spec());
        }

        all.push(task_spec());
        all.push(browser_spec());

        if caps.patch {
            all.push(patch_spec());
        }
    }

    all
}

/// `browser` (0.14): a real headless browser the agent drives like a person testing a page.
pub fn browser_spec() -> ToolSpec {
    ToolSpec {
        name: "browser",
        description: "Use a real browser (headless) to try a page as a user: open, read, click, type, press keys, screenshot. It stays open between calls. For testing the page you built, not as a web fetch.",
        schema: json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["open", "read", "click", "type", "press", "screenshot", "close"], "description": "What to do." },
                "url": { "type": "string", "description": "open: the address (http://localhost:5173, https://…, file:///…)." },
                "target": { "type": "string", "description": "click/type: the element's number from read, a CSS selector, or the words on it." },
                "text": { "type": "string", "description": "type: what to type into the element." },
                "key": { "type": "string", "description": "press: Enter, Tab, Escape, Backspace, ArrowDown, ArrowUp, Space." },
                "width": { "type": "integer", "description": "open: viewport width in pixels when the browser starts (default 1280; 390 for a phone)." },
            },
            "required": ["action"],
            "additionalProperties": false,
        }),
    }
}

/// `apply_patch` (0.14): Codex's patch format, for the models trained on it.
pub fn patch_spec() -> ToolSpec {
    ToolSpec {
        name: "apply_patch",
        description: "Edit several files with one Codex-format patch: '*** Begin Patch' to '*** End Patch', with '*** Add File: path' (+ lines), '*** Delete File: path', or '*** Update File: path' and an '@@' hunk of context, removed (-) and added (+) lines that must match exactly.",
        schema: json!({
            "type": "object",
            "properties": { "patch": { "type": "string", "description": "The whole patch." } },
            "required": ["patch"],
            "additionalProperties": false,
        }),
    }
}

/// The `task` tool: a sub-agent with its own fresh context, read-only, for exploring and researching.
pub fn task_spec() -> ToolSpec {
    ToolSpec {
        name: "task",
        description: "Hand a self-contained exploration or research job to a sub-agent with a fresh context and read-only tools; it answers with a report. Several task calls in one reply run in parallel. Give it everything it needs: it has not seen this conversation.",
        schema: json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "Three to six words: what the sub-agent is doing (shown on its card)." },
                "prompt": { "type": "string", "description": "The full job: what to find out, where to look, and what the report must contain." },
            },
            "required": ["description", "prompt"],
            "additionalProperties": false,
        }),
    }
}

fn vision_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "view_image",
            description: "Look at an image file in the project folder (png, jpg, gif, webp): a mock-up, a screenshot the person saved, an icon.",
            schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Image path, relative to the project folder." } },
                "required": ["path"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "screenshot",
            description: "Render a page in a headless browser and look at it - to check the page you built looks right. Works with a dev server's localhost address, a public URL, and file:/// paths.",
            schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The address to render." },
                    "width": { "type": "integer", "description": "Viewport width in pixels (default 1280; 390 for a phone)." },
                    "height": { "type": "integer", "description": "Viewport height in pixels (default 900)." },
                },
                "required": ["url"],
                "additionalProperties": false,
            }),
        },
    ]
}

fn more_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "grep",
            description: "Search file contents with a regular expression (Rust/PCRE-like). Answers path:line: text, up to 200 hits; skips node_modules, .git and build output. Prefer it to reading files one by one to find where something is used.",
            schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "The regular expression, for example function\\s+charge\\w+ or TODO|FIXME." },
                    "path": { "type": "string", "description": "Directory to search in, relative to the folder. Defaults to the whole folder." },
                    "glob": { "type": "string", "description": "Only files matching this pattern, for example *.tsx or src/**/*.php." },
                    "ignore_case": { "type": "boolean", "description": "Match without regard to case." },
                },
                "required": ["pattern"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "glob",
            description: "Find files by name pattern: **/*.tsx, src/**/test_*.py, *.config.js. Answers paths relative to the folder, most recently changed first, up to 200.",
            schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "The pattern. ** crosses folders, * and ? do not, {a,b} is either." },
                    "path": { "type": "string", "description": "Directory to look in, relative to the folder. Defaults to the whole folder." },
                },
                "required": ["pattern"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "web_fetch",
            description: "Read a public web page (documentation, an API reference, an error's discussion) as text. http(s) only; not this machine or a private network.",
            schema: json!({
                "type": "object",
                "properties": { "url": { "type": "string", "description": "The full address, starting with https://." } },
                "required": ["url"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "web_search",
            description: "Search the web. Answers titles, addresses and snippets; read a result with web_fetch.",
            schema: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "What to search for." } },
                "required": ["query"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "start_process",
            description: "Start a command that keeps running (a dev server, a watcher) in the background. Answers its id and first output - a server's address, usually. It keeps running after your answer: read it with process_output, end it with stop_process.",
            schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command line, for the shell named in the system prompt." },
                    "wait_seconds": { "type": "integer", "description": "How long to wait for its first output before answering (default 6, max 60)." },
                },
                "required": ["command"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "process_output",
            description: "The latest output of a background process, and whether it is still running.",
            schema: json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The id start_process answered." },
                    "lines": { "type": "integer", "description": "How many of the last lines (default 60)." },
                },
                "required": ["process_id"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "stop_process",
            description: "Stop a background process.",
            schema: json!({
                "type": "object",
                "properties": { "process_id": { "type": "string", "description": "The id start_process answered." } },
                "required": ["process_id"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "ask_user",
            description: "Ask the person and wait for the answer - when a decision is theirs (which design, which data to keep, which reading) and a wrong guess would waste the work. Offer 2-4 short options, in their language. Not for permission to run a tool.",
            schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The question, complete and short." },
                    "options": { "type": "array", "items": { "type": "string" }, "description": "Two to four short choices." },
                },
                "required": ["question"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "remember",
            description: "Keep a lasting fact for later chats (a preference, a convention, a decision) - not this task's progress. project writes .sdc/memory.md; global is for all projects. A quote must be the file's own line.",
            schema: json!({
                "type": "object",
                "properties": {
                    "fact": { "type": "string", "description": "One short, self-contained sentence." },
                    "scope": { "type": "string", "enum": ["project", "global"], "description": "project (default) or global." },
                    "source": { "type": "string", "description": "The file the fact rests on, if it rests on code." },
                    "quote": { "type": "string", "description": "That line, exactly as the file has it." },
                },
                "required": ["fact"],
                "additionalProperties": false,
            }),
        },
    ]
}

fn base_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file",
            description: "Read a text file in the project folder, with line numbers. Paths are relative to the folder. For a large file, read a range with offset and limit; a whole read is cut at 256 KB and says so.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path, relative to the project folder." },
                    "offset": { "type": "integer", "description": "The first line to read, counting from 1. Omit to start at the top." },
                    "limit": { "type": "integer", "description": "How many lines to read from offset. Omit to read to the end." },
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "list_dir",
            description: "List one level of a directory in the project folder. Directories end with a slash.",
            schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Directory, relative to the project folder. Use \".\" for the folder itself." } },
                "required": ["path"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "search",
            description: "Find a literal string in the project's files. Answers path:line: text, up to 100 hits.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The exact text to find (not a regex)." },
                    "path": { "type": "string", "description": "Directory to search in, relative to the folder. Defaults to the whole folder." },
                    "glob": { "type": "string", "description": "Optional file name filter, for example *.ts" },
                },
                "required": ["query"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "git_diff",
            description: "Show the project's uncommitted changes (git diff HEAD). Use it to review your own work before you finish.",
            schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolSpec {
            name: "write_file",
            description: "Create a file, or replace a whole file, with the given content. Parent directories are created. Prefer edit_file for changing part of an existing file.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path, relative to the project folder." },
                    "content": { "type": "string", "description": "The complete new content of the file." },
                },
                "required": ["path", "content"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "edit_file",
            description: "Replace an exact piece of text in an existing file. old_text must appear exactly once (include enough surrounding lines to make it unique), unless replace_all is true.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path, relative to the project folder." },
                    "old_text": { "type": "string", "description": "The exact text to replace, including whitespace." },
                    "new_text": { "type": "string", "description": "The text to put in its place." },
                    "replace_all": { "type": "boolean", "description": "Replace every occurrence instead of exactly one." },
                },
                "required": ["path", "old_text", "new_text"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "run_command",
            description: "Run a shell command in the project folder and get its exit code and output. Use it to install, build, test and run. Commands that never exit (servers, watchers) must not be started here.",
            schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command line, for the shell named in the system prompt." },
                    "timeout_seconds": { "type": "integer", "description": "Stop the command after this many seconds (default 120, max 600)." },
                },
                "required": ["command"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "update_plan",
            description: "Show the person your plan as a checklist, and update it as you go. Call it before a multi-step task and whenever a step starts or finishes. Keep one step in_progress at a time.",
            schema: json!({
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": { "type": "string" },
                                "status": { "type": "string", "enum": ["pending", "in_progress", "done"] },
                            },
                            "required": ["text", "status"],
                            "additionalProperties": false,
                        },
                    },
                },
                "required": ["steps"],
                "additionalProperties": false,
            }),
        },
    ]
}

/// What a turn's tools share: the folder, the window, the permission rule and what was already allowed.
pub struct ToolContext<'a> {
    pub workspace: &'a Workspace,
    /// The chat the turn belongs to - a background process is listed under it.
    pub session_id: &'a str,
    /// A sub-agent's context: tools that change anything are refused, whatever the model asks.
    pub read_only: bool,
    /// The model can look at images: a browser screenshot goes to it only then (0.14).
    pub vision: bool,
    pub sink: &'a EventSink,
    pub turn_id: &'a str,
    pub autonomy: Autonomy,
    /// Kinds of action the person said `Always allow` to, for the rest of this turn.
    pub always: HashSet<&'static str>,
    pub calls: usize,
    /// Writes the turn's checkpoint **now**, before the change - see `Checkpointer`.
    pub checkpoint: Option<Checkpointer>,
    /// Whether this turn's checkpoint exists yet: one per turn, before its first change.
    pub checkpointed: bool,
    /// The folder's policy (0.12): protected paths and command rules are asked about **before** acting.
    pub policy: &'a crate::trust::policy::Policy,
    /// The files this turn has changed - the blast radius.
    pub changed: HashSet<String>,
    /// The person allowed this turn to go past the blast radius.
    pub radius_allowed: bool,
    /// The agent's checklist as it last sent it (0.13): the completion gate reads its open steps.
    pub plan: Option<Value>,
    /// How many file changes this turn has made - the completion gate asks again only after new ones.
    pub edits: usize,
    /// The edit count when the agent last ran a command: equal to `edits`, it ran something after its last change.
    pub ran_at: Option<usize>,
    /// A `/research` turn's limits and sources (0.16.1), shared with its sub-agents - `None` otherwise.
    pub research: Option<std::sync::Arc<super::research::Session>>,
    /// The model runs on this machine (Ollama): its window is small, and the web is its only by `/research`.
    pub local: bool,
    /// The most one tool answer hands back - `RESULT_CAP`, or less for a local model's window.
    pub result_cap: usize,
    /// The words the model wrote in the message that carries this step's tool calls: where `replace_symbol` finds the new definition (a plain
    /// fenced code block - model-written code is never inside a JSON string, N3).
    pub reply_text: String,
}

impl ToolContext<'_> {
    /// The plan's steps that are not done, one per line - `None` when there is no plan or all are done.
    pub fn open_plan_steps(&self) -> Option<String> {
        let steps = self.plan.as_ref()?.as_array()?;
        let open: Vec<String> = steps
            .iter()
            .filter(|step| step["status"] != "done")
            .map(|step| format!("- {}", step["text"].as_str().unwrap_or_default()))
            .collect();

        (!open.is_empty()).then(|| open.join("\n"))
    }
}

/// The Trust Kernel's word on a file change before it happens: a protected path, a turn past its blast
/// radius, or text that carries a secret each wait for the person - in every mode, Auto included.
/// `Some(outcome)` is the refusal to hand the model.
fn guard_edit(context: &mut ToolContext, path: &str, content: Option<&str>) -> Option<Outcome> {
    let root = context.workspace.root().to_string();

    if let Some(pattern) = context.policy.protected(path, Some(&root)) {
        if let Some(refused) = ask_always(
            context,
            &format!("Change a protected file: {path}"),
            &format!("The policy protects `{pattern}`. Changing it can break the site or leak a secret."),
            path,
            "This path is in .sdc/policy.toml's protected list (or SDC's defaults). Allow only if you meant the agent to touch it.",
        ) {
            return Some(refused);
        }
    }

    /* N6-3 (`nr.contract_lock`): a signature a verified step published cannot be changed or removed by a patch */
    if let (Some(text), false) = (content, context.workspace.is_remote()) {
        if crate::v3::on("nr.contract_lock") {
            let relative = context.workspace.resolve(path).ok().map(|absolute| context.workspace.relative(&absolute)).unwrap_or_else(|| path.to_string());
            let lock = crate::navaratna::contracts::Lock::load(std::path::Path::new(&root));

            if let Err(violations) = crate::navaratna::contracts::check_patch(&lock, &relative, text) {
                return Some(Outcome::error(violations.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")));
            }
        }
    }

    if let Some(text) = content {
        let found = crate::trust::scan::secrets_in_text(path, text);

        if let Some(first) = found.first() {
            if let Some(refused) = ask_always(
                context,
                &format!("Write a secret into {path}"),
                first["message"].as_str().unwrap_or("The content looks like a secret"),
                path,
                "A key written into a file ends up in commits and backups. The usual place is an environment variable.",
            ) {
                return Some(refused);
            }
        }
    }

    let max = context.policy.max_files_per_turn;

    if !context.changed.contains(path) && max > 0 && context.changed.len() >= max && !context.radius_allowed {
        if let Some(refused) = ask_always(
            context,
            "Go past the blast radius",
            &format!("This turn has already changed {} files, the policy's limit.", context.changed.len()),
            path,
            "A wide change is harder to review and to undo. Allow to let this turn continue past the limit.",
        ) {
            return Some(refused);
        }

        context.radius_allowed = true;
    }

    context.changed.insert(path.to_string());
    context.edits += 1;

    None
}

/// What the model is told when the person refused and said why. The reason is their own words (from the desktop
/// or a signed message from their browser), so it is passed on as their instruction, quoted.
fn declined_with(reason: &str) -> String {
    format!("The person declined this action and said: \"{reason}\". Follow what they said; do not try the action again in another way.")
}

/// A question the kernel asks whatever the autonomy level: `DANGEROUS` is the risk the gate never skips.
fn ask_always(context: &mut ToolContext, title: &str, sub: &str, target: &str, explain: &str) -> Option<Outcome> {
    match gate::ask(context.sink, context.turn_id, context.calls, "edit", title, sub, target, "DANGEROUS", explain) {
        Decision::Allow | Decision::AlwaysAllow => None,
        Decision::Deny => Some(Outcome::error(
            "The person declined this change: it touches something the project's policy protects. Do not try it another way; explain what you wanted to do.",
        )),
        Decision::DenyWith(reason) => Some(Outcome::error(declined_with(&reason))),
        Decision::ShowMe => Some(Outcome::error(
            "The person wants to see exactly what this changes before allowing it. Stop calling tools now and show the change in your answer.",
        )),
        Decision::Stopped => Some(Outcome::error("The turn was stopped.")),
    }
}

/**
 * Takes the checkpoint of the chat's folder and pushes `CheckpointSaved`, and returns when both are done.
 *
 * The daemon's turn loop used to take the checkpoint when it *received* a mutating `ToolStarted` - but the
 * agent does not wait for the loop, so the file was written first and the "before" checkpoint recorded the
 * change it was meant to undo (found by Verify: "no changes since the checkpoint"). The agent now calls this
 * itself, synchronously, between the permission and the write.
 */
pub type Checkpointer = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// The turn's checkpoint, taken once, before its first change.
fn checkpoint_first(context: &mut ToolContext, title: &str) {
    if context.checkpointed {
        return;
    }

    if let Some(checkpoint) = &context.checkpoint {
        checkpoint(title);
    }

    context.checkpointed = true;
}

/// One tool's answer to the model: the text, and whether it is an error the model should react to.
pub struct Outcome {
    pub content: String,
    pub is_error: bool,
    /// An image the model is shown with the answer (0.13): `(media type, base64)`.
    pub image: Option<(String, String)>,
}

impl Outcome {
    pub fn ok(content: impl Into<String>) -> Self {
        Self { content: cap(&content.into()), is_error: false, image: None }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self { content: cap(&content.into()), is_error: true, image: None }
    }

    fn picture(content: impl Into<String>, media_type: &str, bytes: &[u8]) -> Self {
        use base64::Engine as _;

        Self {
            content: content.into(),
            is_error: false,
            image: Some((media_type.to_string(), base64::engine::general_purpose::STANDARD.encode(bytes))),
        }
    }
}

/// Runs one tool call, drawing its card as it goes.
pub fn execute(context: &mut ToolContext, call: &ToolUse) -> Outcome {
    let before = context.calls;
    let outcome = execute_call(context, call);

    /* 0.19: what the model *saw* - the file it read, the matches, the page - goes on its card too, so the
       person watches the same material the model reasons over. Only for tools that print nothing of their
       own while they run, only the first lines, and never for an error the card already says. */
    if SEEN.contains(&call.name.as_str()) && context.calls == before + 1 && !outcome.content.trim().is_empty() {
        let call_id = format!("{}-{}", context.turn_id, context.calls);

        for event in crate::engines::result_lines(&call_id, &outcome.content, outcome.is_error) {
            context.sink.send(event);
        }
    }

    /* A local model's window holds less than one full answer (0.16.1): the cap follows the window. */
    if context.result_cap < RESULT_CAP && outcome.content.len() > context.result_cap {
        /* PADMA P0-5: what is cut is kept in the vault, so the model can read the rest with `obs_read` instead of losing it. */
        if crate::v3::on("padma.obs_cap") && call.name != "obs_read" {
            return Outcome { content: crate::padma::budget::vault_cut(&context.session_id, &outcome.content, context.result_cap), ..outcome };
        }

        return Outcome { content: cap_to(&outcome.content, context.result_cap), ..outcome };
    }

    outcome
}

/// The answer of one call, before the window's cap.
fn execute_call(context: &mut ToolContext, call: &ToolUse) -> Outcome {
    if let Some(raw) = call.input.get("__invalid_json") {
        return Outcome::error(format!(
            "The input for {} was not valid JSON ({}). Send the call again with a complete JSON object.",
            call.name,
            raw.as_str().unwrap_or_default().chars().take(200).collect::<String>()
        ));
    }

    context.calls += 1;

    let call_id = format!("{}-{}", context.turn_id, context.calls);
    let text = |key: &str| call.input[key].as_str().unwrap_or_default().to_string();

    if context.read_only && !READ_ONLY.contains(&call.name.as_str()) {
        return Outcome::error(format!("`{}` is not available to a sub-agent: it only reads. Report what should be changed instead.", call.name));
    }

    /* A research turn has its own small toolbox; a model that reaches past it is told what it has. */
    if context.research.is_some() && !super::research::TOOLS.contains(&call.name.as_str()) {
        return Outcome::error(format!(
            "`{}` is not part of a research turn. The tools are: {}.",
            call.name,
            super::research::TOOLS.join(", ")
        ));
    }

    /* v3 W7b: in plan mode the turn may read anything and change nothing. */
    if crate::v3::plan::active() && crate::v3::plan::changes_something(&call.name) {
        started(context, &call_id, "read", "Plan mode", &call.name);
        completed(context, &call_id, true, "not run · plan mode", None);

        return Outcome::ok(crate::v3::plan::refusal(&call.name));
    }

    let number = |key: &str| call.input[key].as_u64();

    match call.name.as_str() {
        "read_file" if number("offset").is_some() || number("limit").is_some() => {
            read_range(context, &call_id, &text("path"), number("offset").unwrap_or(1) as usize, number("limit").map(|limit| limit as usize))
        }
        "read_file" => read_file(context, &call_id, &text("path")),
        "grep" => grep(context, &call_id, &text("pattern"), &text("path"), call.input["glob"].as_str(), call.input["ignore_case"].as_bool().unwrap_or(false)),
        "glob" => glob(context, &call_id, &text("pattern"), &text("path")),
        "fleet_run" => fleet_run(context, &call_id, &call.input),
        "find_symbol" => find_symbol(context, &call_id, &text("name")),
        "symbols" => file_symbols(context, &call_id, &text("path")),
        "code_graph" => code_graph(context, &call_id, &text("query"), &text("name")),
        "ranked_search" => ranked_search(context, &call_id, &text("query"), call.input["k"].as_u64()),
        "patch_symbol" => patch_symbol(context, &call_id, &call.input),
        "replace_symbol" => replace_symbol(context, &call_id, &call.input),
        "spec_read" => spec_read(context, &call.input),
        "obs_read" => obs_read(context, &call.input),
        "web_fetch" => web_fetch(context, &call_id, &text("url")),
        "web_search" => web_search(context, &call_id, &text("query")),
        "start_process" => start_process(context, &call_id, &text("command"), number("wait_seconds").unwrap_or(6).min(60)),
        "process_output" => process_output(context, &call_id, &text("process_id"), number("lines").unwrap_or(60) as usize),
        "stop_process" => stop_process(context, &call_id, &text("process_id")),
        "ask_user" => ask_user(context, &call_id, &text("question"), &call.input["options"]),
        "remember" => remember(context, &call_id, &text("fact"), call.input["scope"].as_str().unwrap_or("project"), &call.input),
        "view_image" => view_image(context, &call_id, &text("path")),
        "screenshot" => screenshot(context, &call_id, &text("url"), number("width").unwrap_or(1280) as u32, number("height").unwrap_or(900) as u32),
        "browser" => browser(context, &call_id, &call.input),
        "apply_patch" => apply_patch(context, &call_id, &text("patch")),
        "list_dir" => list_dir(context, &call_id, &text("path")),
        "search" => search(context, &call_id, &text("query"), &text("path"), call.input["glob"].as_str()),
        "git_diff" => git_diff(context, &call_id),
        "write_file" => write_file(context, &call_id, &text("path"), &text("content")),
        "edit_file" => edit_file(
            context,
            &call_id,
            &text("path"),
            &text("old_text"),
            &text("new_text"),
            call.input["replace_all"].as_bool().unwrap_or(false),
        ),
        "run_command" => run_command(
            context,
            &call_id,
            &text("command"),
            call.input["timeout_seconds"].as_u64().map(Duration::from_secs).unwrap_or(DEFAULT_COMMAND),
        ),
        "update_plan" => update_plan(context, &call.input),
        other => Outcome::error(format!("There is no tool called `{other}`. The tools are: {}.", names().join(", "))),
    }
}

fn names() -> Vec<&'static str> {
    specs_for(Caps { vision: true, subagent: false, patch: true, local: true }).iter().map(|spec| spec.name).collect()
}

fn started(context: &ToolContext, call_id: &str, tool: &str, name: &str, target: &str) {
    context.sink.send(EngineEvent::ToolStarted {
        call_id: call_id.to_string(),
        tool: tool.to_string(),
        name: name.to_string(),
        target: target.to_string(),
    });
}

fn completed(context: &ToolContext, call_id: &str, ok: bool, meta: &str, diff: Option<Value>) {
    context.sink.send(EngineEvent::ToolCompleted {
        call_id: call_id.to_string(),
        status: if ok { "done" } else { "failed" }.to_string(),
        meta: meta.to_string(),
        diff,
    });
}

fn read_file(context: &mut ToolContext, call_id: &str, path: &str) -> Outcome {
    started(context, call_id, "read", "Read", path);

    match context.workspace.read(path) {
        Ok((text, truncated)) => {
            /* Phase 0.3: a secret in the content, or a name that says one, is asked about before any model sees the
               file; once the person allows it, the model still gets the text with the values masked. */
            /* A file the policy marks secret is withheld whole by the privacy filter; there is nothing to ask. */
            let marked = context.policy.secret(path, Some(context.workspace.root())).is_some();
            let found = if marked { Vec::new() } else { crate::trust::secretguard::scan(path, &text) };
            let named = if marked { None } else { crate::trust::secretguard::sensitive_name(path) };

            let text = if found.is_empty() && named.is_none() {
                text
            } else {
                let detail = if found.is_empty() {
                    format!("The file name says `{}`; no secret value was recognised in it.", named.clone().unwrap_or_default())
                } else {
                    format!("Found {}.", crate::trust::secretguard::describe(&found))
                };

                if let Some(refused) = ask_always(
                    context,
                    &format!("Read a file that may hold a secret: {path}"),
                    &detail,
                    path,
                    "Nothing has been sent to a model. If you allow it, the model gets the file with every secret value masked (only its length is kept).",
                ) {
                    completed(context, call_id, false, "withheld", None);

                    return refused;
                }

                if crate::v3::on("redact") {
                    /* the vault's placeholder: the model can still write the file back with the real value */
                    crate::trust::secretguard::mask_with(&text, &found, |value| crate::v3::redact::hide(context.session_id, value))
                } else {
                    crate::trust::secretguard::mask(&text, &found)
                }
            };
            let lines = text.lines().count();

            /* v3 I2: a long file's outline first (switch `v3.skeleton`); the agent reads the part it needs. */
            if crate::v3::turn_on(context.turn_id, "skeleton") && !truncated {
                if let Some(outline) = crate::v3::outline::outline(path, &text) {
                    completed(context, call_id, true, &format!("outline · {lines} ln"), None);

                    return Outcome::ok(outline);
                }
            }

            completed(context, call_id, true, &format!("done · {lines} ln{}", if truncated { " · cut" } else { "" }), None);

            let mut content = numbered(&text);

            if truncated {
                content.push_str("\n[The file is longer than 256 KB; only its beginning is shown. Read the rest in pieces: read_file with offset and limit, or find the part with ranked_search.]");
            }

            Outcome::ok(content)
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(file_error(context, path, error.message))
        }
    }
}

/// v3 I7: one command on many servers, the answers clustered. A command that changes anything asks first; with
/// `canary` it runs on the first server alone, and goes on to the rest only if that one exits 0.
fn fleet_run(context: &mut ToolContext, call_id: &str, input: &Value) -> Outcome {
    let command = input["command"].as_str().unwrap_or_default().trim().to_string();
    let selector: Vec<String> = input["hosts"].as_array().map(|hosts| hosts.iter().filter_map(|host| host.as_str().map(str::to_string)).collect()).unwrap_or_default();

    if command.is_empty() || selector.is_empty() {
        return Outcome::error("fleet_run needs a command and hosts (names, ids or [\"all\"]).");
    }

    let servers = crate::v3::fleet::select_hosts(&selector);

    if servers.is_empty() {
        return Outcome::error(format!("No connected server matches {}. The servers SDC knows are in the Hosts list.", selector.join(", ")));
    }

    if !crate::v3::fleet::read_only(&command) {
        if let Some(refused) = ask_always(
            context,
            &format!("Run on {} server(s): {command}", servers.len()),
            "A command that may change the servers",
            &servers.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", "),
            "This runs on every server listed. Use canary to try one server first.",
        ) {
            return refused;
        }
    }

    started(context, call_id, "run", "Fleet", &format!("{} server(s) · {command}", servers.len()));

    let mut answers = Vec::new();
    let rest = if input["canary"].as_bool().unwrap_or(false) {
        answers = crate::v3::fleet::run_on(&servers[..1], &command);

        if answers.first().is_some_and(|answer| answer.exit != 0) {
            completed(context, call_id, false, "canary failed", None);

            return Outcome::ok(format!("Canary {} failed (exit {}), so the other {} server(s) were not touched:\n{}", servers[0].0, answers[0].exit, servers.len() - 1, answers[0].output));
        }

        &servers[1..]
    } else {
        &servers[..]
    };

    answers.extend(crate::v3::fleet::run_on(rest, &command));

    let failed = answers.iter().filter(|answer| answer.exit != 0).count();

    completed(context, call_id, failed == 0, &format!("{} server(s) · {failed} failed", answers.len()), None);
    Outcome::ok(crate::v3::fleet::render(&command, &answers))
}

/// v3 W4: the two code-intelligence tools' specs.
fn intel_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "find_symbol",
            description: "Find where something is declared in this project (a function, struct, class, type, section). Answers file:line and the kind. Use it before reading files to find the right place.",
            schema: serde_json::json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "The declaration's name, or part of it." } },
                "required": ["name"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "symbols",
            description: "List one file's declarations (name, kind, line) without reading its code. Use it to see a long file's shape before reading the part you need.",
            schema: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "The file, relative to the project folder." } },
                "required": ["path"],
                "additionalProperties": false,
            }),
        },
    ]
}

/// P1-1: the Code Graph's other two questions - who uses a name, and which tests touch a file - with no model behind them.
fn code_graph_spec() -> ToolSpec {
    ToolSpec {
        name: "code_graph",
        description: "The project's code graph: `callers` where a name is used, `tests` which tests touch a file, `where` where it is declared.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "enum": ["callers", "tests", "where"] },
                "name": { "type": "string", "description": "A name (callers, where) or a file path relative to the project (tests)." },
            },
            "required": ["query", "name"],
            "additionalProperties": false,
        }),
    }
}

fn code_graph(context: &mut ToolContext, call_id: &str, query: &str, name: &str) -> Outcome {
    started(context, call_id, "read", "Code graph", &format!("{query} {name}"));

    if context.workspace.is_remote() {
        completed(context, call_id, false, "remote folder", None);

        return Outcome::error("The code graph reads this computer's folders; this chat's folder is on a server.");
    }

    let (index, _) = crate::graph::index::refreshed(std::path::Path::new(context.workspace.root()));
    let lines: Vec<String> = match query {
        "callers" => index.callers(name).into_iter().take(60).map(|(file, line, call)| format!("{file}:{line}{}", if call { " (call)" } else { "" })).collect(),
        "tests" => index.tests_for(name.trim_start_matches("./")),
        "where" => index.where_is(name).into_iter().map(|(file, def)| format!("{file}:{}: {} {} #{}", def.start_line, def.kind, def.qualified, &def.hash[..12])).collect(),
        other => {
            completed(context, call_id, false, "bad query", None);

            return Outcome::error(format!("`{other}` is not a question the code graph answers: callers, tests or where."));
        }
    };

    completed(context, call_id, true, &format!("{} found", lines.len()), None);

    if lines.is_empty() {
        return Outcome::ok(format!("The code graph found nothing for {query} {name}. (It reads Rust, Python, JavaScript, TypeScript, Go and PHP; for other files use grep.)"));
    }

    Outcome::ok(lines.join("\n"))
}

/// P1-11: edit a declaration by what it is, not by where its text happens to be.
fn patch_symbol_spec() -> ToolSpec {
    ToolSpec {
        name: "patch_symbol",
        description: "Change one declaration by name: file, qualified name (`Class.method`), the #hash find_symbol printed. Applied only if the hash still matches and the name is unique. `new_body` or `edits` ({old,new}).",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "symbol": { "type": "string", "description": "Qualified name, e.g. Store.add or Ledger::total" },
                "old_hash": { "type": "string", "description": "At least 8 hex digits of the symbol's #hash as find_symbol/symbols showed it" },
                "new_body": { "type": "string", "description": "The complete new declaration, from its first line to its last (symbols up to 60 lines)." },
                "edits": { "type": "array", "items": { "type": "object", "properties": { "old": { "type": "string" }, "new": { "type": "string" } }, "required": ["old", "new"] } },
            },
            "required": ["path", "symbol", "old_hash"],
            "additionalProperties": false,
        }),
    }
}

/// The part of the file that is one symbol, after the checks that make a symbol edit safe. `Err` is the sentence for the model.
pub fn plan_symbol_patch(path: &str, text: &str, symbol: &str, old_hash: &str, new_body: Option<&str>, edits: &[(String, String)]) -> Result<String, String> {
    use crate::graph::parse;

    let lang = parse::Lang::of(path).filter(|lang| lang.has_symbols()).ok_or_else(|| format!("{path} is not a language the code graph reads (Rust, Python, JavaScript, TypeScript, Go, PHP); use edit_file."))?;
    let tree = parse::parse(lang, text).ok_or("the file could not be parsed; use edit_file")?;
    let defs = parse::definitions(lang, &tree, text);
    let exact: Vec<&parse::Def> = defs.iter().filter(|def| def.qualified == symbol).collect();
    let found: Vec<&parse::Def> = if exact.is_empty() { defs.iter().filter(|def| def.name == symbol).collect() } else { exact };

    match found.len() {
        0 => {
            let known: Vec<String> = defs.iter().take(25).map(|def| def.qualified.clone()).collect();

            return Err(format!("{path} has no declaration called `{symbol}`. Its declarations: {}. Read the file again.", known.join(", ")));
        }
        1 => {}
        _ => {
            let lines: Vec<String> = found.iter().map(|def| format!("{} (line {}, #{})", def.qualified, def.start_line, &def.hash[..12])).collect();

            return Err(format!("`{symbol}` names {} declarations in {path}: {}. Nothing was changed - say which one by its qualified name and its hash.", found.len(), lines.join("; ")));
        }
    }

    let def = found[0];
    let wanted = old_hash.trim().trim_start_matches('#').to_ascii_lowercase();

    if wanted.len() < 8 || !wanted.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("old_hash must be at least 8 hex digits of the symbol's #hash.".into());
    }

    if !def.hash.starts_with(&wanted) && !def.norm_hash.starts_with(&wanted) {
        return Err(format!("`{symbol}` is not what you read: its hash is now #{} (you gave #{wanted}). Nothing was changed - read it again with find_symbol or read_file, then patch.", &def.hash[..12]));
    }

    let current = &text[def.start_byte..def.end_byte];
    let replaced = match (new_body, edits.is_empty()) {
        (Some(body), true) => {
            if def.end_line - def.start_line + 1 > 60 {
                return Err(format!("`{symbol}` is {} lines: too large to rewrite whole. Send `edits` ({{old, new}} replacements inside it) instead.", def.end_line - def.start_line + 1));
            }

            body.trim_start().to_string()
        }
        (None, false) => {
            let mut working = current.to_string();

            for (old, new) in edits {
                if old.trim().is_empty() {
                    return Err("an edit's `old` is empty".into());
                }

                let exact_hits = working.matches(old.as_str()).count();

                working = match exact_hits {
                    1 => working.replacen(old.as_str(), new, 1),
                    0 => {
                        /* the same tokens in the same order, whitespace aside - never fuzzier than that */
                        let pattern = old.split_whitespace().map(regex::escape).collect::<Vec<_>>().join(r"\s+");
                        let matcher = regex::Regex::new(&pattern).map_err(|error| error.to_string())?;
                        let hits: Vec<_> = matcher.find_iter(&working).collect();

                        match hits.len() {
                            1 => format!("{}{}{}", &working[..hits[0].start()], new, &working[hits[0].end()..]),
                            0 => return Err(format!("`old` was not found inside `{symbol}`: {:?}. Copy it from the symbol exactly. Nothing was changed.", old.chars().take(80).collect::<String>())),
                            n => return Err(format!("`old` matches {n} places inside `{symbol}`; add surrounding text so it is unique. Nothing was changed.")),
                        }
                    }
                    n => return Err(format!("`old` appears {n} times inside `{symbol}`; add surrounding text so it is unique. Nothing was changed.")),
                };
            }

            working
        }
        (Some(_), false) => return Err("send either new_body or edits, not both".into()),
        (None, true) => return Err("send new_body (the whole declaration) or edits (replacements inside it)".into()),
    };

    Ok(format!("{}{}{}", &text[..def.start_byte], replaced, &text[def.end_byte..]))
}

/// A body that opens more brackets than it closes was most likely cut off in the middle (the model ran out of room).
pub fn looks_cut_off(body: &str) -> bool {
    let open = body.chars().filter(|c| matches!(c, '(' | '[' | '{')).count();
    let close = body.chars().filter(|c| matches!(c, ')' | ']' | '}')).count();

    open > close
}

fn patch_symbol(context: &mut ToolContext, call_id: &str, input: &Value) -> Outcome {
    let text_of = |key: &str| input[key].as_str().unwrap_or_default().to_string();
    let (path, symbol, old_hash) = (text_of("path"), text_of("symbol"), text_of("old_hash"));
    let new_body = input["new_body"].as_str();
    let edits: Vec<(String, String)> = input["edits"].as_array().into_iter().flatten().map(|edit| (edit["old"].as_str().unwrap_or_default().to_string(), edit["new"].as_str().unwrap_or_default().to_string())).collect();

    if context.workspace.is_remote() {
        return Outcome::error("patch_symbol works on this computer's folders; use edit_file for a server's.");
    }

    let before = match context.workspace.read(&path) {
        Ok((_, true)) => return Outcome::error(format!("{path} is larger than 256 KB; use edit_file on a range.")),
        Ok((text, false)) => text,
        Err(error) => return Outcome::error(error.message),
    };

    let after = match plan_symbol_patch(&path, &before, &symbol, &old_hash, new_body, &edits) {
        Ok(after) => after,
        Err(reason) => return Outcome::error(reason),
    };

    keep_pre_image(context, &path, Some(before.as_bytes()));

    let diff = diff_lines(&before, &after);
    let (added, removed) = counts(&diff);

    started(context, call_id, "edit", "Patch symbol", &format!("{path} · {symbol}"));

    if let Some(refused) = guard_edit(context, &path, Some(&after)) {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    if let Some(refused) = ask(context, "edit", &format!("Patch {symbol} in {path}"), &format!("The agent wants to change one declaration (+{added} −{removed} lines)"), &path, "MUTATING", "A checkpoint is taken before the first change of this turn, so Rewind can put the file back.") {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    checkpoint_first(context, &format!("Before patch_symbol {path}"));

    match commit_edits(context, &[(path.as_str(), Some(after.as_str()))]) {
        Ok(()) => {
            let now = crate::intel::symbols(&path, &after).into_iter().find(|candidate| candidate.qualified == symbol || candidate.name == symbol).map(|found| format!(" It is now #{}.", &found.hash[..12.min(found.hash.len())])).unwrap_or_default();

            completed(context, call_id, true, &format!("done · +{added} −{removed}"), Some(card(&diff)));

            let said = format!("Patched {symbol} in {path} (+{added} −{removed}).{now}");

            Outcome::ok(match language_server_note(context, &path, &after) {
                Some(note) => format!("{said}\n\n{note}"),
                None => said,
            })
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            let cut = new_body.is_some_and(looks_cut_off);

            Outcome::error(if cut { format!("{} The new body opens more brackets than it closes, so it looks cut off: send the whole declaration, or smaller `edits`.", error.message) } else { error.message })
        }
    }
}

/// `obs_read` (PADMA P0-5): lines of a tool output that was cut to fit the window.
fn obs_read_spec() -> ToolSpec {
    ToolSpec {
        name: "obs_read",
        description: "Read part of a tool output that was cut short or shortened to a line `‹obs:N›`. Give N and the line range.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "n": { "type": "integer", "description": "The number N in ‹obs:N›." },
                "range": { "type": "string", "description": "Lines, like \"120-240\". Default: the first 120." },
            },
            "required": ["n"],
            "additionalProperties": false,
        }),
    }
}

fn obs_read(context: &ToolContext, input: &Value) -> Outcome {
    let n = input["n"].as_u64().unwrap_or(0) as u32;

    match crate::padma::budget::read(&context.session_id, n, input["range"].as_str().unwrap_or_default(), context.result_cap.saturating_sub(400).max(1_500)) {
        Ok(text) => Outcome::ok(text),
        Err(error) => Outcome::error(error),
    }
}

/// `spec_read` (N3): lines of something SDC kept outside the conversation - the whole log behind an error slice, the full request.
fn spec_read_spec() -> ToolSpec {
    ToolSpec {
        name: "spec_read",
        description: "Read part of something SDC kept for you: the whole log behind a cut-down error (kind `log`), the full request (`spec`). Give the kind and the id that was printed, and the line range.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["log", "spec", "specmap", "section"] },
                "id": { "type": "string", "description": "The id printed next to `spec_read`." },
                "from": { "type": "integer", "description": "First line (1-based, default 1)." },
                "to": { "type": "integer", "description": "Last line (default: 200 lines on)." },
            },
            "required": ["kind", "id"],
            "additionalProperties": false,
        }),
    }
}

fn spec_read(context: &ToolContext, input: &Value) -> Outcome {
    let (kind, id) = (input["kind"].as_str().unwrap_or_default(), input["id"].as_str().unwrap_or_default());
    let from = input["from"].as_u64().unwrap_or(1).max(1) as usize;
    let to = input["to"].as_u64().map_or(from + 199, |to| to as usize).min(from + 399);

    if context.workspace.is_remote() {
        return Outcome::error("spec_read works on this computer's folders.");
    }

    match crate::navaratna::vault::read_lines(std::path::Path::new(context.workspace.root()), kind, id, from, to) {
        Some(text) => Outcome::ok(cap_to(&text, RESULT_CAP)),
        None => Outcome::error(format!("Nothing is kept under {kind} {id}. Use the id exactly as it was printed.")),
    }
}

/// N4-2 (`nr.symbol_patch`): replace a whole declaration with the definition the model wrote as a code block in its message.
fn replace_symbol_spec() -> ToolSpec {
    ToolSpec {
        name: "replace_symbol",
        description: "Replace one whole declaration, or add one after it. Name the file and the symbol (qualified, e.g. Cart.add or Ledger::total). Write the COMPLETE new definition in a fenced ``` code block in the same message, before this call - not in the arguments; nothing may be left out (no `// ...`, no `rest unchanged`) and it must define exactly that one symbol. mode `insert_after` adds a new symbol after the named one instead.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "symbol": { "type": "string", "description": "Qualified name of the symbol to replace (or to insert after)." },
                "mode": { "type": "string", "enum": ["replace", "insert_after"] },
            },
            "required": ["path", "symbol"],
            "additionalProperties": false,
        }),
    }
}

fn replace_symbol(context: &mut ToolContext, call_id: &str, input: &Value) -> Outcome {
    use crate::navaratna::patcher;

    let text_of = |key: &str| input[key].as_str().unwrap_or_default().to_string();
    let (path, symbol) = (text_of("path"), text_of("symbol"));
    let insert = input["mode"].as_str() == Some("insert_after");

    if context.workspace.is_remote() {
        return Outcome::error("replace_symbol works on this computer's folders; use edit_file for a server's.");
    }

    if !context.reply_text.contains("```") {
        return Outcome::error("Write the new definition in a fenced ``` code block in your message first, then call replace_symbol with the path and the symbol. The code goes in the message, not in the arguments.");
    }

    let before = match context.workspace.read(&path) {
        Ok((_, true)) => return Outcome::error(format!("{path} is larger than 256 KB; use edit_file on a range.")),
        Ok((text, false)) => text,
        Err(error) => return Outcome::error(error.message),
    };
    let planned = if insert { patcher::insert_after(&path, &before, &symbol, &context.reply_text) } else { patcher::replace_symbol(&path, &before, &symbol, &context.reply_text) };
    let after = match planned {
        Ok(after) => after,
        Err(refused) => return Outcome::error(refused.to_string()),
    };

    keep_pre_image(context, &path, Some(before.as_bytes()));

    let diff = diff_lines(&before, &after);
    let (added, removed) = counts(&diff);

    started(context, call_id, "edit", "Replace symbol", &format!("{path} · {symbol}"));

    if let Some(refused) = guard_edit(context, &path, Some(&after)) {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    if let Some(refused) = ask(context, "edit", &format!("Replace {symbol} in {path}"), &format!("The agent wants to change one declaration (+{added} −{removed} lines)"), &path, "MUTATING", "A checkpoint is taken before the change, so it can be undone.") {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    checkpoint_first(context, &format!("Before replace_symbol {path}"));

    match commit_edits(context, &[(path.as_str(), Some(after.as_str()))]) {
        Ok(()) => {
            completed(context, call_id, true, &format!("done · +{added} −{removed}"), Some(card(&diff)));

            let said = format!("{} {symbol} in {path} (+{added} −{removed}).", if insert { "Added a symbol after" } else { "Replaced" });

            Outcome::ok(match language_server_note(context, &path, &after) {
                Some(note) => format!("{said}\n\n{note}"),
                None => said,
            })
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(error.message)
        }
    }
}

/// ` #1a2b3c4d5e6f` after a declaration the Code Graph read: the hash `patch_symbol` asks for (empty for a line-pattern symbol).
fn hash_tag(symbol: &crate::intel::Symbol) -> String {
    if symbol.hash.is_empty() { String::new() } else { format!(" #{}", &symbol.hash[..12]) }
}

/// P1-8: the ranked search - which chunks of the project answer this question, from the names, the declarations and BM25, with no model.
fn ranked_search_spec() -> ToolSpec {
    ToolSpec {
        name: "ranked_search",
        description: "Find the code that answers a question, ranked and with no model: the best chunks as `path:lines` and a skim of the best one.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "What you are looking for, in words (e.g. `where the header is drawn`)" },
                "k": { "type": "integer", "description": "How many candidates (default 6, at most 20)" },
            },
            "required": ["query"],
            "additionalProperties": false,
        }),
    }
}

fn ranked_search(context: &mut ToolContext, call_id: &str, query: &str, k: Option<u64>) -> Outcome {
    if query.trim().is_empty() {
        return Outcome::error("ranked_search needs a query.");
    }

    started(context, call_id, "read", "Ranked search", query);

    if context.workspace.is_remote() {
        completed(context, call_id, false, "remote folder", None);

        return Outcome::error("The ranked search reads this computer's folders; this chat's folder is on a server - use grep.");
    }

    let root = std::path::Path::new(context.workspace.root());
    let k = k.unwrap_or(6).clamp(1, 20) as usize;
    let index = crate::sme::retrieval::build(root, |_, _| {});
    let hits = index.search(query, k);
    let skimmed = crate::sme::retrieval::skim(&hits, 24);

    completed(context, call_id, true, &format!("{} candidates", hits.len()), None);

    Outcome::ok(if skimmed.is_empty() { format!("Nothing in this project matches `{query}`. Try grep with a word you saw.") } else { skimmed })
}

/// v3 W4: where a name is declared, across the project (the newest-changed files are looked at first).
fn find_symbol(context: &mut ToolContext, call_id: &str, name: &str) -> Outcome {
    let name = name.trim();

    if name.chars().count() < 2 {
        return Outcome::error("find_symbol needs a name (two letters or more).");
    }

    started(context, call_id, "read", "Find symbol", name);

    let root = std::path::PathBuf::from(context.workspace.root());
    let mut found: Vec<String> = Vec::new();

    for path in crate::search::files(&root, 4_000) {
        let Ok(text) = std::fs::read_to_string(root.join(&path)) else {
            continue;
        };

        for symbol in crate::intel::matching(&crate::intel::symbols(&path, &text), name) {
            found.push(format!("{path}:{}: {} {}{}", symbol.line, symbol.kind, if symbol.qualified.is_empty() { &symbol.name } else { &symbol.qualified }, hash_tag(&symbol)));
        }

        if found.len() >= 40 {
            break;
        }
    }

    completed(context, call_id, !found.is_empty(), &format!("{} found", found.len()), None);

    if found.is_empty() {
        return Outcome::ok(format!("No declaration of {name} in this project. It may be in a dependency, or named differently - grep for the text instead."));
    }

    Outcome::ok(format!("{name}:\n{}", found.join("\n")))
}

/// v3 W4: one file's declarations.
fn file_symbols(context: &mut ToolContext, call_id: &str, path: &str) -> Outcome {
    let path = path.trim();
    let Ok(full) = context.workspace.resolve(path) else {
        return failed_with(context, call_id, format!("{path} is outside the project folder"));
    };
    let Ok(text) = std::fs::read_to_string(&full) else {
        return failed_with(context, call_id, format!("cannot read {path}"));
    };

    let symbols = crate::intel::symbols(path, &text);

    started(context, call_id, "read", "Symbols", path);
    completed(context, call_id, true, &format!("{} declarations", symbols.len()), None);

    if symbols.is_empty() {
        return Outcome::ok(format!("{path} has no declarations SDC recognises ({} lines) - read it instead.", text.lines().count()));
    }

    Outcome::ok(symbols.iter().map(|symbol| format!("{}: {} {}{}", symbol.line, symbol.kind, if symbol.qualified.is_empty() { &symbol.name } else { &symbol.qualified }, hash_tag(symbol))).collect::<Vec<_>>().join("\n"))
}

/// v3 W7a: before a file is changed, its old content goes into the content-addressed store, filed under this turn -
/// so the turn can be taken back in a project with no git at all (switch `v3.cas`).
fn keep_pre_image(context: &ToolContext, path: &str, before: Option<&[u8]>) {
    if !crate::v3::on("cas") {
        return;
    }

    let root = std::path::Path::new(context.workspace.root());

    crate::v3::cas::record(root, context.turn_id, path, before);
}

fn list_dir(context: &mut ToolContext, call_id: &str, path: &str) -> Outcome {
    let shown = if path.trim().is_empty() { "." } else { path };

    started(context, call_id, "read", "List", shown);

    match context.workspace.list(shown) {
        Ok((entries, hidden)) => {
            completed(context, call_id, true, &format!("done · {} entries", entries.len()), None);

            let mut content = if entries.is_empty() { "(empty)".to_string() } else { entries.join("\n") };

            if hidden > 0 {
                content.push_str(&format!("\n[{hidden} name(s) hidden by the file guard: secrets and keys are never shown]"));
            }

            Outcome::ok(content)
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(file_error(context, shown, error.message))
        }
    }
}

fn search(context: &mut ToolContext, call_id: &str, query: &str, path: &str, glob: Option<&str>) -> Outcome {
    if query.is_empty() {
        return Outcome::error("search needs a non-empty query.");
    }

    started(context, call_id, "read", "Search", query);

    match context.workspace.search(query, if path.is_empty() { "." } else { path }, glob, 100) {
        Ok(hits) => {
            completed(context, call_id, true, &format!("done · {} hits", hits.len()), None);

            Outcome::ok(if hits.is_empty() { format!("No match for `{query}`.") } else { hits.join("\n") })
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(error.message)
        }
    }
}

fn git_diff(context: &mut ToolContext, call_id: &str) -> Outcome {
    started(context, call_id, "read", "Diff", ".");

    match context.workspace.diff() {
        Ok(patch) => {
            completed(context, call_id, true, &format!("done · {} ln", patch.lines().count()), None);

            Outcome::ok(if patch.trim().is_empty() { "No uncommitted changes.".to_string() } else { patch })
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(error.message)
        }
    }
}

/// The project's `after_edit` hooks for one written file (0.13): each runs in the folder, its last lines
/// go on the card, and a failure is added to what the model is told - a formatter that rewrote the file
/// or a linter that objects is something it has to know before its next edit.
fn after_edit(context: &mut ToolContext, call_id: &str, path: &str) -> Option<String> {
    let mut failures = Vec::new();

    for hook in context.policy.after_edit.clone() {
        let quoted = if context.workspace.is_remote() || !cfg!(windows) { crate::ssh::sh_quote(path) } else { format!("\"{path}\"") };
        let line = hook.replace("{file}", &quoted);

        if crate::pty::denied_reason_line(&line).is_some() || context.policy.denies(&line).is_some() {
            continue;
        }

        match context.workspace.run(&line, Duration::from_secs(120)) {
            Ok(report) => {
                let combined = format!("{}{}", report.stdout, report.stderr);

                context.sink.send(EngineEvent::ToolOutput {
                    call_id: call_id.to_string(),
                    level: if report.ok { "dim" } else { "fail" }.to_string(),
                    text: format!("hook: {line} → {}", if report.ok { "ok".to_string() } else { format!("exit {}", report.exit_code.unwrap_or(-1)) }),
                });

                if !report.ok {
                    let tail: Vec<&str> = combined.lines().rev().take(25).collect::<Vec<_>>().into_iter().rev().collect();

                    failures.push(format!("The project's after_edit hook `{line}` failed:\n{}", tail.join("\n")));
                }
            }
            Err(error) => failures.push(format!("The project's after_edit hook `{line}` could not run: {}", error.message)),
        }
    }

    (!failures.is_empty()).then(|| failures.join("\n\n"))
}

/// Writes the edits of one step (P1-2). With `edit.transaction` on and a local folder they go through the Step Runtime: a syntax gate that
/// refuses newly broken code before anything changes, then an all-or-nothing journaled commit. SDC's own `.sdc/` and `.git/` are not
/// project sources and are written as before; a remote folder is written file by file. `edits` are `(path, Some(text) | None to delete)`.
fn commit_edits(context: &ToolContext, edits: &[(&str, Option<&str>)]) -> Result<(), crate::sdcp::envelope::ErrorObject> {
    let own = |path: &str| path.trim_start_matches("./").split('/').any(|part| part == ".sdc" || part == ".git");

    if !crate::v3::on("edit.transaction") || context.workspace.is_remote() {
        return write_directly(context, edits);
    }

    let mut staged: Vec<(String, Option<String>)> = Vec::new();
    let mut direct: Vec<(&str, Option<&str>)> = Vec::new();

    for (path, text) in edits {
        let absolute = context.workspace.resolve(path)?;

        crate::fs::guard(std::path::Path::new(&absolute))?;

        let relative = context.workspace.relative(&absolute);

        if own(&relative) {
            direct.push((path, *text));
        } else {
            staged.push((relative, text.map(str::to_string)));
        }
    }

    if !staged.is_empty() {
        crate::runtime::step::apply(std::path::Path::new(context.workspace.root()), &format!("{}-{}", context.turn_id, context.calls), &staged).map_err(|refusal| crate::sdcp::envelope::ErrorObject::bad_request(refusal.0))?;
    }

    write_directly(context, &direct)
}

fn write_directly(context: &ToolContext, edits: &[(&str, Option<&str>)]) -> Result<(), crate::sdcp::envelope::ErrorObject> {
    for (path, text) in edits {
        match text {
            Some(text) => context.workspace.write(path, text)?,
            None => context.workspace.remove(path)?,
        }
    }

    Ok(())
}

/// P1-12 (switch `verify.lsp`): what the language server says about the file just written, with the declarations of the names its
/// errors mention. `None` when the switch is off, the folder is remote, no server is installed for the language, or there is nothing wrong.
fn language_server_note(context: &ToolContext, path: &str, text: &str) -> Option<String> {
    if !crate::v3::on("verify.lsp") || context.workspace.is_remote() {
        return None;
    }

    let root = std::path::Path::new(context.workspace.root());
    let relative = context.workspace.resolve(path).ok().map(|absolute| context.workspace.relative(&absolute))?;
    let found = crate::verify::lsp::check(root, &relative, text, None).ok().flatten()?;
    let note = crate::verify::lsp::repair_note(root, &relative, &found);

    (!note.is_empty()).then_some(note)
}

/// M4-1 (`verify.api_exists`): the imports of the edited text looked up in the installed dependencies. A warning for the model, never a refusal.
fn api_exists_note(context: &ToolContext, path: &str, text: &str) -> Option<String> {
    if !crate::v3::on("verify.api_exists") || context.workspace.is_remote() {
        return None;
    }

    let relative = context.workspace.resolve(path).ok().map(|absolute| context.workspace.relative(&absolute))?;

    crate::verify::api_exists::warning(&crate::verify::api_exists::check(std::path::Path::new(context.workspace.root()), &relative, text))
}

/// N5-2 (`nr.exist_gate`): members of this project's own types that the symbol index does not have. A warning, never a refusal.
fn exist_gate_note(context: &ToolContext, path: &str, text: &str) -> Option<String> {
    if context.workspace.is_remote() {
        return None;
    }

    let relative = context.workspace.resolve(path).ok().map(|absolute| context.workspace.relative(&absolute))?;

    let root = std::path::Path::new(context.workspace.root());
    /* N6-1 (`nr.registry`): the same name or the same body declared in two places */
    let notes: Vec<String> = crate::navaratna::exist::note(root, &relative, text).into_iter().chain(crate::navaratna::registry::duplicate_note(root, &relative, text)).collect();

    (!notes.is_empty()).then(|| notes.join("\n\n"))
}

/// The text after a successful edit: the hooks' failure, and the language server's errors, each only when there is one.
fn after_edit_notes(context: &ToolContext, path: &str, text: &str, hooks: Option<String>) -> Option<String> {
    let notes: Vec<String> = hooks.into_iter().chain(language_server_note(context, path, text)).chain(api_exists_note(context, path, text)).chain(exist_gate_note(context, path, text)).collect();

    (!notes.is_empty()).then(|| notes.join("\n\n"))
}

fn write_file(context: &mut ToolContext, call_id: &str, path: &str, content: &str) -> Outcome {
    if path.trim().is_empty() {
        return Outcome::error("write_file needs a path.");
    }

    let existed = context.workspace.exists(path);
    let before = if existed { context.workspace.read(path).map(|(text, _)| text).unwrap_or_default() } else { String::new() };

    keep_pre_image(context, path, existed.then_some(before.as_bytes()));
    let diff = diff_lines(&before, content);
    let (added, removed) = counts(&diff);

    started(context, call_id, "edit", if existed { "Edit" } else { "Create" }, path);

    if let Some(refused) = guard_edit(context, path, Some(content)) {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    let verb = if existed { "replace" } else { "create" };

    if let Some(refused) = ask(
        context,
        "edit",
        &format!("{} {path}", if existed { "Replace" } else { "Create" }),
        &format!("The agent wants to {verb} a file (+{added} −{removed} lines)"),
        path,
        "MUTATING",
        "A checkpoint is taken before the first change of this turn, so Rewind can put the file back.",
    ) {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    checkpoint_first(context, &format!("Before {} {path}", if existed { "Edit" } else { "Create" }));

    match commit_edits(context, &[(path, Some(content))]) {
        Ok(()) => {
            let hook_failure = after_edit(context, call_id, path);
            let hooks = after_edit_notes(context, path, content, hook_failure);

            completed(context, call_id, true, &format!("done · +{added} −{removed}"), Some(card(&diff)));

            let said = format!("{} {path} ({} lines).", if existed { "Replaced" } else { "Created" }, content.lines().count());

            Outcome::ok(match hooks {
                Some(failed) => format!("{said}\n\n{failed}"),
                None => said,
            })
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(file_error(context, path, error.message))
        }
    }
}

fn edit_file(context: &mut ToolContext, call_id: &str, path: &str, old: &str, new: &str, all: bool) -> Outcome {
    if old.is_empty() {
        return Outcome::error("edit_file needs old_text. To create a file or replace all of it, use write_file.");
    }

    let before = match context.workspace.read(path) {
        Ok((_, true)) => return Outcome::error(format!("{path} is too large to edit in place; it is longer than 256 KB.")),
        Ok((text, false)) => text,
        Err(error) => return Outcome::error(file_error(context, path, error.message)),
    };
    let found = before.matches(old).count();

    keep_pre_image(context, path, Some(before.as_bytes()));

    if found == 0 {
        return Outcome::error(format!(
            "old_text was not found in {path}. Read the file again and copy the text exactly, including indentation."
        ));
    }

    if found > 1 && !all {
        return Outcome::error(format!(
            "old_text appears {found} times in {path}. Add surrounding lines so it is unique, or set replace_all."
        ));
    }

    let after = if all { before.replace(old, new) } else { before.replacen(old, new, 1) };
    let diff = diff_lines(&before, &after);
    let (added, removed) = counts(&diff);

    started(context, call_id, "edit", "Edit", path);

    if let Some(refused) = guard_edit(context, path, Some(new)) {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    if let Some(refused) = ask(
        context,
        "edit",
        &format!("Edit {path}"),
        &format!("The agent wants to change a file (+{added} −{removed} lines)"),
        path,
        "MUTATING",
        "A checkpoint is taken before the first change of this turn, so Rewind can put the file back.",
    ) {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    checkpoint_first(context, &format!("Before Edit {path}"));

    match commit_edits(context, &[(path, Some(&after))]) {
        Ok(()) => {
            let hook_failure = after_edit(context, call_id, path);
            let hooks = after_edit_notes(context, path, &after, hook_failure);

            completed(context, call_id, true, &format!("done · +{added} −{removed}"), Some(card(&diff)));

            let said = format!("Edited {path}: {} replacement(s).", if all { found } else { 1 });

            Outcome::ok(match hooks {
                Some(failed) => format!("{said}\n\n{failed}"),
                None => said,
            })
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(error.message)
        }
    }
}

fn run_command(context: &mut ToolContext, call_id: &str, line: &str, timeout: Duration) -> Outcome {
    if line.trim().is_empty() {
        return Outcome::error("run_command needs a command.");
    }

    started(context, call_id, "run", "Run", line);

    if let Some(reason) = crate::pty::denied_reason_line(line) {
        completed(context, call_id, false, "refused", None);

        return Outcome::error(format!("Refused: {reason}. This command is on SDC's deny list and cannot be run."));
    }

    if let Some(rule) = context.policy.denies(line) {
        completed(context, call_id, false, "refused", None);

        return Outcome::error(format!(
            "Refused: this project's policy (.sdc/policy.toml) denies commands matching `{rule}`. Do not run it another way."
        ));
    }

    /* P3-3: an install that names a package no registry knows is refused before anyone is asked. */
    let mut unverified_note = None;

    if crate::v3::on("trust.slopguard") && !context.workspace.is_remote() {
        let verdict = crate::trust::slopguard::check(line);

        if let Some(refusal) = verdict.refusal() {
            completed(context, call_id, false, "refused", None);

            return Outcome::error(refusal);
        }

        unverified_note = verdict.note();
    }

    /* A command the policy lists under `always_ask` waits for the person in every mode, like a dangerous one. */
    let risk = if gate::looks_dangerous(line) || context.policy.always_asks(line).is_some() { "DANGEROUS" } else { "MUTATING" };

    if let Some(refused) = ask(
        context,
        "run",
        "Run a command",
        &format!("The agent wants to run a command on {}", context.workspace.place()),
        line,
        risk,
        &format!(
            "It runs in {} with a {}s limit, and its output goes back to the agent. A checkpoint is taken before the first change of this turn.",
            context.workspace.root(),
            timeout.as_secs()
        ),
    ) {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    checkpoint_first(context, &format!("Before Run {line}"));
    context.ran_at = Some(context.edits);

    match context.workspace.run(line, timeout) {
        Ok(report) => {
            let combined = format!("{}{}", report.stdout, report.stderr);
            let lines: Vec<&str> = combined.lines().collect();

            /* The card's output window: the last lines, where a test runner prints its verdict. */
            for text in lines.iter().rev().take(30).rev() {
                context.sink.send(EngineEvent::ToolOutput {
                    call_id: call_id.to_string(),
                    level: "dim".to_string(),
                    text: text.to_string(),
                });
            }

            let exit = report.exit_code.map(|code| code.to_string()).unwrap_or_else(|| "none".to_string());
            let meta = if report.timed_out {
                format!("timed out · {}s", timeout.as_secs())
            } else {
                format!("exit {exit} · {}ms", report.duration_ms)
            };

            context.sink.send(EngineEvent::ToolOutput {
                call_id: call_id.to_string(),
                level: if report.ok { "ok" } else { "fail" }.to_string(),
                text: meta.clone(),
            });
            completed(context, call_id, report.ok, &meta, None);

            let mut content = format!("exit code: {exit}{}\n", if report.timed_out { " (stopped: timed out)" } else { "" });

            if !report.stdout.is_empty() {
                content.push_str(&format!("--- stdout ---\n{}\n", report.stdout));
            }

            if !report.stderr.is_empty() {
                content.push_str(&format!("--- stderr ---\n{}\n", report.stderr));
            }

            if let Some(note) = &unverified_note {
                content.push_str(note);
                content.push('\n');
            }

            /* N3-3 (`nr.error_slice`): a failed build or test is shown as its first error, where it is, and the code around it; the whole log is
               kept in the vault. A log the slicer does not read is shown as it always was. */
            if !report.ok && !report.timed_out && !context.workspace.is_remote() && crate::v3::on_in("nr.error_slice", &tool_model(context)) {
                if let Some(slice) = crate::navaratna::errslice::slice(std::path::Path::new(context.workspace.root()), &combined, None, 800) {
                    content = format!("exit code: {exit}\n--- the failure, cut to its first error ---\n{}\n", slice.text);

                    if let Some(note) = &unverified_note {
                        content.push_str(note);
                        content.push('\n');
                    }
                }
            }

            /* A failed command is information, not a tool error: the model reads the output and fixes
               the code. Only a command that could not be run at all is an error. */
            Outcome::ok(content)
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(error.message)
        }
    }
}

/// A file tool's error in words (PADMA P0-8, `padma.tool_hints`): what the operating system said, the path SDC looked at, and what is near it.
fn file_error(context: &ToolContext, path: &str, message: String) -> String {
    if !crate::v3::on("padma.tool_hints") || context.workspace.is_remote() {
        return message;
    }

    match context.workspace.resolve(path) {
        Ok(resolved) => crate::padma::hints::explain(context.workspace.root(), &resolved, &message),
        Err(_) => message,
    }
}

/// The model a tool context belongs to, for a switch's scope.
fn tool_model(context: &ToolContext) -> crate::v3::ModelCtx {
    crate::v3::ModelCtx::new(context.local)
}

fn update_plan(context: &mut ToolContext, input: &Value) -> Outcome {
    let steps: Vec<Value> = input["steps"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|step| {
            let text = step["text"].as_str()?.trim().to_string();
            let status = match step["status"].as_str().unwrap_or("pending") {
                "done" => "done",
                "in_progress" => "in_progress",
                _ => "pending",
            };

            (!text.is_empty()).then(|| json!({ "text": text, "status": status }))
        })
        .take(20)
        .collect();

    if steps.is_empty() {
        return Outcome::error("update_plan needs at least one step with text.");
    }

    let done = steps.iter().filter(|step| step["status"] == "done").count();

    context.sink.send(EngineEvent::Plan(json!(steps)));
    context.plan = Some(json!(steps));

    let total = steps.len();
    let said = if done == total {
        format!("Plan shown: all {total} steps done.")
    } else {
        format!("Plan shown: {done} of {total} steps done. Call update_plan again as each step starts or finishes.")
    };

    /* P1-10: a step naming more than two files is larger than the step budget, and the answer says which ones - split by file, so no
       step carries more context than it can hold. */
    let large = if crate::v3::on_in("runtime.decompose", &tool_model(context)) {
        crate::runtime::decompose::too_large(&steps.iter().filter_map(|step| step["text"].as_str().map(str::to_string)).collect::<Vec<_>>())
    } else {
        Vec::new()
    };

    Outcome::ok(if large.is_empty() {
        said
    } else {
        let named: Vec<String> = large.iter().map(|(index, files)| format!("step {} names {} files ({})", index + 1, files.len(), files.join(", "))).collect();

        format!("{said} But {} - a step holds {}: split each into one step per file.", named.join("; "), crate::runtime::decompose::FILES_PER_STEP)
    })
}

/// One call to a tool of the project's MCP servers (0.12): a `run` for the permission rules, because an
/// MCP tool can do anything its server can - with the card, the checkpoint and the ledger row of one.
pub fn mcp_call(context: &mut ToolContext, servers: &mut super::mcp::McpTools, call: &ToolUse) -> Outcome {
    context.calls += 1;

    let call_id = format!("{}-{}", context.turn_id, context.calls);
    let label = servers.label(&call.name);

    started(context, &call_id, "run", "MCP", &label);

    if let Some(refused) = ask(
        context,
        "run",
        &format!("Use {label}"),
        "The agent wants to call a tool of this project's MCP server",
        &label,
        "MUTATING",
        "The server is listed in .sdc/mcp.json and runs on this machine; its tool can do whatever the server can.",
    ) {
        completed(context, &call_id, false, "declined", None);

        return refused;
    }

    checkpoint_first(context, &format!("Before MCP {label}"));

    match servers.call(&call.name, &call.input) {
        Ok(text) => {
            completed(context, &call_id, true, &format!("done · {} ln", text.lines().count()), None);

            Outcome::ok(if text.trim().is_empty() { "The tool answered with nothing.".to_string() } else { text })
        }
        Err(error) => {
            completed(context, &call_id, false, "failed", None);

            Outcome::error(error)
        }
    }
}

fn failed_with(context: &ToolContext, call_id: &str, error: impl Into<String>) -> Outcome {
    completed(context, call_id, false, "failed", None);

    Outcome::error(error)
}

/// `read_file` with a range: the lines from `offset` (1-based), `limit` of them, with their real numbers.
fn read_range(context: &mut ToolContext, call_id: &str, path: &str, offset: usize, limit: Option<usize>) -> Outcome {
    let offset = offset.max(1);

    started(context, call_id, "read", "Read", &format!("{path}:{offset}{}", limit.map(|limit| format!("+{limit}")).unwrap_or_default()));

    match context.workspace.read_lines(path, offset, limit.unwrap_or(2_000).clamp(1, 5_000)) {
        Ok((lines, total)) => {
            completed(context, call_id, true, &format!("done · {} of {total} ln", lines.len()), None);

            if lines.is_empty() {
                return Outcome::ok(format!("{path} has {total} lines; there is nothing from line {offset}."));
            }

            let body = lines
                .iter()
                .enumerate()
                .map(|(index, line)| format!("{:>5}\t{line}", offset + index))
                .collect::<Vec<_>>()
                .join("\n");
            let end = offset + lines.len() - 1;

            Outcome::ok(if end < total { format!("{body}\n[lines {offset}-{end} of {total} · more: read_file with offset={}]", end + 1) } else { body })
        }
        Err(error) => failed_with(context, call_id, error.message),
    }
}

fn grep(context: &mut ToolContext, call_id: &str, pattern: &str, path: &str, glob: Option<&str>, ignore_case: bool) -> Outcome {
    if pattern.is_empty() {
        return Outcome::error("grep needs a pattern.");
    }

    started(context, call_id, "read", "Grep", pattern);

    match super::search::grep(context.workspace.root(), context.workspace.remote(), path, pattern, glob, ignore_case, 200) {
        Ok(hits) => {
            completed(context, call_id, true, &format!("done · {} hits", hits.len()), None);

            if hits.is_empty() {
                return Outcome::ok(format!("No line matches `{pattern}`."));
            }

            let mut lines: Vec<String> = hits.iter().map(|hit| format!("{}:{}: {}", hit.path, hit.line, hit.text)).collect();

            if hits.len() >= 200 {
                lines.push("[200 hits shown - narrow the pattern, the path or the glob]".to_string());
            }

            Outcome::ok(lines.join("\n"))
        }
        Err(error) => failed_with(context, call_id, error.message),
    }
}

fn glob(context: &mut ToolContext, call_id: &str, pattern: &str, path: &str) -> Outcome {
    if pattern.is_empty() {
        return Outcome::error("glob needs a pattern.");
    }

    started(context, call_id, "read", "Glob", pattern);

    match super::search::glob(context.workspace.root(), context.workspace.remote(), path, pattern, 200) {
        Ok((files, total)) => {
            completed(context, call_id, true, &format!("done · {total} files"), None);

            if files.is_empty() {
                return Outcome::ok(format!("No file matches `{pattern}`."));
            }

            let mut text = files.join("\n");

            if total > files.len() {
                text.push_str(&format!("\n[{} of {total} shown, newest first]", files.len()));
            }

            Outcome::ok(text)
        }
        Err(error) => failed_with(context, call_id, error.message),
    }
}

/// Why this turn may not use the web, or `None` when it may (0.16.1).
///
/// A folder whose policy keeps everything local opens the web for a `/research` turn alone - the command
/// is the person's permission. A local model uses the web only through `/research` unless Settings →
/// Research says otherwise; an API model keeps the web it always had.
fn web_closed(context: &ToolContext, what: &str) -> Option<String> {
    if context.research.is_some() {
        return None;
    }

    if context.policy.privacy_local() {
        return Some(format!("This project's policy keeps everything on this machine (privacy = local), so {what}. Ask with /research to look something up on the web."));
    }

    if context.local && super::research::config().local_web_only_research {
        return Some(format!("On a local model {what} unless the person asks with /research (Settings → Research). Answer from what you know and what is in the folder, or tell the person to start the question with /research."));
    }

    None
}

fn web_fetch(context: &mut ToolContext, call_id: &str, url: &str) -> Outcome {
    started(context, call_id, "read", "Fetch", url);

    if let Some(reason) = web_closed(context, "the web is not read") {
        return failed_with(context, call_id, reason);
    }

    if let Some(session) = context.research.clone() {
        if let Err(reason) = session.take_page() {
            return failed_with(context, call_id, reason);
        }
    }

    let page_cap = if context.local { context.result_cap.saturating_sub(400).max(2_000) } else { super::web::PAGE_CAP };

    match super::web::fetch_page(url, page_cap) {
        Ok(page) => {
            completed(context, call_id, true, &format!("done · {} ln", page.text.lines().count()), None);

            /* v3 L3: what this page said is worth keeping - the next turn about it need not fetch again. */
            if crate::v3::on("knowledge") {
                crate::v3::knowledge::remember(std::path::Path::new(context.workspace.root()), &page.url, &page.title, &page.text);
            }

            let dated = page.date.as_deref().map(|date| format!(" · {date}")).unwrap_or_default();

            match &context.research {
                Some(session) => {
                    let n = session.cite(&page.url, &page.title, page.date.clone(), true);

                    /* M4-2: with sentence citations the page is shown as numbered sentences, and the same sentences are kept to check the pointers against. */
                    if crate::v3::on("research.sentence_cite") {
                        let sentences = crate::sme::cite::sentences(&page.text);
                        let shown = crate::sme::cite::numbered(n, &sentences);

                        session.keep_sentences(n, sentences);

                        return Outcome::ok(format!("[{n}] {}{dated}\n{}\n\n{}", page.title, page.url, shown));
                    }

                    Outcome::ok(format!("[{n}] {}{dated}\n{}\n\n{}", page.title, page.url, page.text))
                }
                None => Outcome::ok(format!("[{}]{dated}\n{}", page.url, page.text)),
            }
        }
        Err(error) => failed_with(context, call_id, error),
    }
}

fn web_search(context: &mut ToolContext, call_id: &str, query: &str) -> Outcome {
    started(context, call_id, "read", "Search web", query);

    if let Some(reason) = web_closed(context, "the web is not searched") {
        return failed_with(context, call_id, reason);
    }

    if let Some(session) = context.research.clone() {
        if let Err(reason) = session.take_search() {
            return failed_with(context, call_id, reason);
        }
    }

    /* Fewer results for a small window: eight snippets are a quarter of a 4K context. */
    let limit = if context.local { 5 } else { 8 };

    match super::research::search(query, limit) {
        Ok((results, _)) if results.is_empty() => {
            completed(context, call_id, true, "done · 0 results", None);

            Outcome::ok(format!("No results for `{query}`."))
        }
        Ok((results, service)) => {
            completed(context, call_id, true, &format!("done · {} results · {service}", results.len()), None);

            let lines: Vec<String> = results
                .iter()
                .enumerate()
                .map(|(index, hit)| {
                    let n = match &context.research {
                        Some(session) => session.cite(&hit.url, &hit.title, hit.date.clone(), false),
                        None => index + 1,
                    };
                    let dated = hit.date.as_deref().map(|date| format!(" · {date}")).unwrap_or_default();

                    format!("[{n}] {}{dated}\n   {}\n   {}", hit.title, hit.url, hit.snippet)
                })
                .collect();

            Outcome::ok(lines.join("\n"))
        }
        Err(error) => failed_with(context, call_id, error),
    }
}

fn start_process(context: &mut ToolContext, call_id: &str, line: &str, wait: u64) -> Outcome {
    if line.trim().is_empty() {
        return Outcome::error("start_process needs a command.");
    }

    started(context, call_id, "run", "Start", line);

    if let Some(reason) = crate::pty::denied_reason_line(line) {
        completed(context, call_id, false, "refused", None);

        return Outcome::error(format!("Refused: {reason}. This command is on SDC's deny list and cannot be run."));
    }

    if let Some(rule) = context.policy.denies(line) {
        completed(context, call_id, false, "refused", None);

        return Outcome::error(format!("Refused: this project's policy (.sdc/policy.toml) denies commands matching `{rule}`."));
    }

    let risk = if gate::looks_dangerous(line) || context.policy.always_asks(line).is_some() { "DANGEROUS" } else { "MUTATING" };

    if let Some(refused) = ask(
        context,
        "run",
        "Start a background process",
        &format!("The agent wants to start a command that keeps running on {}", context.workspace.place()),
        line,
        risk,
        "It keeps running after the turn (a dev server, a watcher) until it is stopped - from the agent, or from the chat's process list.",
    ) {
        completed(context, call_id, false, "declined", None);

        return refused;
    }

    checkpoint_first(context, &format!("Before Start {line}"));

    let id = match super::background::start(context.session_id, context.workspace.root(), context.workspace.remote(), line) {
        Ok(id) => id,
        Err(error) => return failed_with(context, call_id, error.message),
    };

    /* Its first words, which for a server are the address: waited for, up to `wait` seconds. */
    let deadline = std::time::Instant::now() + Duration::from_secs(wait);
    let (mut text, mut running, mut exit) = (String::new(), true, None);

    while std::time::Instant::now() < deadline && !crate::engines::cancel::requested(context.turn_id) {
        std::thread::sleep(Duration::from_millis(if context.workspace.is_remote() { 1_500 } else { 300 }));

        if let Ok((now, alive, code)) = super::background::output(&id, 40) {
            (text, running, exit) = (now, alive, code);
        }

        if !running || text.to_lowercase().contains("http://") || text.to_lowercase().contains("listening") || text.to_lowercase().contains("ready") {
            break;
        }
    }

    for line in text.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
        context.sink.send(EngineEvent::ToolOutput { call_id: call_id.to_string(), level: "dim".to_string(), text: line.to_string() });
    }

    completed(context, call_id, running, &if running { format!("running · {id}") } else { format!("exited {}", exit.map(|code| code.to_string()).unwrap_or_default()) }, None);

    Outcome::ok(format!(
        "process_id: {id}\nstatus: {}\n--- first output ---\n{}",
        if running { "running".to_string() } else { format!("exited with code {}", exit.map(|code| code.to_string()).unwrap_or_else(|| "?".into())) },
        if text.trim().is_empty() { "(nothing yet)" } else { text.as_str() }
    ))
}

fn process_output(context: &mut ToolContext, call_id: &str, id: &str, lines: usize) -> Outcome {
    started(context, call_id, "read", "Output", id);

    match super::background::output(id, lines.clamp(1, 500)) {
        Ok((text, running, exit)) => {
            completed(context, call_id, true, if running { "running" } else { "exited" }, None);

            Outcome::ok(format!(
                "status: {}\n{}",
                if running { "running".to_string() } else { format!("exited with code {}", exit.map(|code| code.to_string()).unwrap_or_else(|| "?".into())) },
                if text.trim().is_empty() { "(no output)" } else { text.as_str() }
            ))
        }
        Err(error) => failed_with(context, call_id, error.message),
    }
}

fn stop_process(context: &mut ToolContext, call_id: &str, id: &str) -> Outcome {
    started(context, call_id, "run", "Stop", id);

    if super::background::stop(id) {
        completed(context, call_id, true, "stopped", None);

        Outcome::ok(format!("Stopped {id}."))
    } else {
        failed_with(context, call_id, format!("There is no running process `{id}`."))
    }
}

fn ask_user(context: &mut ToolContext, call_id: &str, question: &str, options: &Value) -> Outcome {
    if question.trim().is_empty() {
        return Outcome::error("ask_user needs a question.");
    }

    /* Some models escape twice, and "\n" arrives as a backslash and an n (measured on DeepSeek). */
    let question = question.replace("\\n", "\n");
    let question = question.trim();
    let options: Vec<String> = options
        .as_array()
        .map(|items| items.iter().filter_map(|item| item.as_str()).map(str::trim).filter(|item| !item.is_empty()).take(6).map(str::to_string).collect())
        .unwrap_or_default();

    started(context, call_id, "read", "Question", question);

    match gate::ask_question(context.sink, context.turn_id, context.calls, question, &options) {
        Some(answer) if !answer.is_empty() => {
            /* The answer stays on the card, whole, as the record of what was decided. */
            context.sink.send(EngineEvent::ToolOutput { call_id: call_id.to_string(), level: "ok".to_string(), text: answer.clone() });
            completed(context, call_id, true, &format!("answered · {}", answer.chars().take(40).collect::<String>()), None);

            Outcome::ok(format!("The person answered: {answer}"))
        }
        Some(_) => {
            completed(context, call_id, true, "no answer", None);

            Outcome::ok("The person closed the question without answering. Use your best judgement, and say which choice you made and why.")
        }
        None => {
            completed(context, call_id, false, "stopped", None);

            Outcome::error("The turn was stopped.")
        }
    }
}

fn remember(context: &mut ToolContext, call_id: &str, fact: &str, scope: &str, input: &Value) -> Outcome {
    if fact.trim().is_empty() {
        return Outcome::error("remember needs a fact.");
    }

    /* P1-9: a fact that rests on a line of code carries the line, and the line is checked against the file before anything is kept -
       a small model quotes from memory, and a quote that is not in the file carries an authority nobody wrote. */
    let (source, quote) = (input["source"].as_str().unwrap_or_default().trim().to_string(), input["quote"].as_str().unwrap_or_default().trim().to_string());

    if crate::v3::on("sme.quote_verify") && (!source.is_empty() || !quote.is_empty()) {
        if context.workspace.is_remote() {
            return Outcome::error("A quote is checked against the file on this computer; this chat's folder is on a server - keep the fact without the quote.");
        }

        if let Err(refusal) = crate::sme::quote::verify(std::path::Path::new(context.workspace.root()), &source, &quote) {
            completed(context, call_id, false, "the quote is not in the file", None);

            return Outcome::error(refusal);
        }
    }

    let fact = if quote.is_empty() {
        fact.to_string()
    } else {
        format!("{fact} ({source}: `{quote}`)")
    };
    let global = scope == "global";

    /* M3: with Memory 2.0 a fact a model keeps says where it came from - checked against a file's own line, it was observed; otherwise it is a guess -
       and a fact that is only true today is not memory (it belongs in the plan). */
    let fact = if crate::v3::on("memory.v2") && !global {
        if !crate::v3::memory_v2::lasts_a_month(&fact) {
            return Outcome::error("That sounds like how things are today, not something that will still be true in a month, so it is not kept in memory. Put it in the plan (update_plan), or keep only the lasting part.");
        }

        let origin = if quote.is_empty() { crate::v3::memory_v2::Origin::Inferred } else { crate::v3::memory_v2::Origin::Observed };

        format!("{} {fact}", origin.tag())
    } else {
        fact
    };
    let fact = fact.as_str();

    /* SDC's own memory, not a change to the project: no checkpoint, not in the turn's changed files. */
    started(context, call_id, "read", "Remember", fact);

    let result = if global {
        crate::sdcp::methods::agent_methods::global_memory_path().and_then(|path| {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();

            std::fs::write(&path, crate::sdcp::methods::agent_methods::append_fact(&existing, fact)).map_err(crate::sdcp::envelope::ErrorObject::internal)
        })
    } else {
        context.workspace.append_memory(fact)
    };

    match result {
        Ok(()) => {
            completed(context, call_id, true, if global { "global memory" } else { ".sdc/memory.md" }, None);

            Outcome::ok(format!("Remembered in {}.", if global { "SDC's global memory" } else { ".sdc/memory.md" }))
        }
        Err(error) => failed_with(context, call_id, error.message),
    }
}

/// The media type of an image file, from its name.
fn image_type(path: &str) -> Option<&'static str> {
    let lowered = path.to_ascii_lowercase();

    [(".png", "image/png"), (".jpg", "image/jpeg"), (".jpeg", "image/jpeg"), (".gif", "image/gif"), (".webp", "image/webp")]
        .iter()
        .find(|(extension, _)| lowered.ends_with(extension))
        .map(|(_, media)| *media)
}

fn view_image(context: &mut ToolContext, call_id: &str, path: &str) -> Outcome {
    let Some(media_type) = image_type(path) else {
        return Outcome::error("view_image reads .png, .jpg, .gif and .webp files.");
    };

    started(context, call_id, "read", "View", path);

    match context.workspace.read_bytes(path, 5 * 1024 * 1024) {
        Ok(bytes) => {
            completed(context, call_id, true, &format!("done · {} KB", bytes.len() / 1024), None);

            Outcome::picture(format!("The image {path} is attached."), media_type, &bytes)
        }
        Err(error) => failed_with(context, call_id, error.message),
    }
}

fn screenshot(context: &mut ToolContext, call_id: &str, url: &str, width: u32, height: u32) -> Outcome {
    started(context, call_id, "read", "Screenshot", url);

    match super::browser::screenshot(url, width.clamp(320, 2560), height.clamp(320, 4000)) {
        Ok(bytes) => {
            completed(context, call_id, true, &format!("done · {width}×{height}"), None);

            Outcome::picture(format!("A screenshot of {url} at {width}×{height} is attached."), "image/png", &bytes)
        }
        Err(error) => failed_with(context, call_id, error),
    }
}

/// Whether an address is this machine's own (a dev server, a file): the browser goes there without asking.
fn local_address(url: &str) -> bool {
    let lowered = url.trim().to_ascii_lowercase();

    lowered.starts_with("file:///")
        || ["http://localhost", "http://127.0.0.1", "http://0.0.0.0", "http://[::1]", "https://localhost", "https://127.0.0.1"].iter().any(|prefix| lowered.starts_with(prefix))
}

/// `browser` (0.14): one chat's headless browser, driven step by step.
fn browser(context: &mut ToolContext, call_id: &str, input: &Value) -> Outcome {
    let action = input["action"].as_str().unwrap_or_default().to_string();
    let url = input["url"].as_str().unwrap_or_default().to_string();
    let target = input["target"].as_str().unwrap_or_default().to_string();
    let words = input["text"].as_str().unwrap_or_default().to_string();
    let key = input["key"].as_str().unwrap_or_default().to_string();
    let width = input["width"].as_u64().unwrap_or(1280).clamp(320, 2560) as u32;
    let shown = match action.as_str() {
        "open" => url.clone(),
        "click" => target.clone(),
        "type" => format!("{target} ← {}", words.chars().take(40).collect::<String>()),
        "press" => key.clone(),
        _ => String::new(),
    };

    started(context, call_id, if action == "read" || action == "screenshot" { "read" } else { "run" }, &format!("Browser {action}"), &shown);

    if action == "close" {
        let closed = super::browser::close(context.session_id);

        completed(context, call_id, true, if closed { "closed" } else { "no browser was open" }, None);

        return Outcome::ok(if closed { "The browser is closed." } else { "No browser was open." });
    }

    /* A public site can take real actions (a form sent, an order placed): asked like a command, unless the
       mode says otherwise. The person's own dev server and files are not asked about. */
    if action == "open" && !local_address(&url) {
        if let Some(reason) = web_closed(context, "the browser does not open public sites").filter(|_| !context.policy.privacy_local()) {
            return failed_with(context, call_id, reason);
        }

        if context.policy.privacy_local() {
            return failed_with(context, call_id, "This project's policy keeps everything on this machine (privacy = local-only), so the browser does not open public sites.");
        }

        if let Some(refused) = ask(
            context,
            "run",
            "Open a website in the agent's browser",
            "The agent wants to open a public page and may click or type on it",
            &url,
            "MUTATING",
            "Its clicks and typing are real: a form it sends is sent. Your own dev server and local files are opened without asking.",
        ) {
            completed(context, call_id, false, "declined", None);

            return refused;
        }
    }

    if action == "screenshot" && !context.vision {
        return failed_with(context, call_id, "This model cannot see images; use action read to get the page's text and its clickable elements.");
    }

    let session = context.session_id.to_string();
    let result = super::browser::with_session(&session, width, 900, |page| match action.as_str() {
        "open" => page.open(&url).and_then(|_| page.read()).map(|text| (text, None)),
        "read" => page.read().map(|text| (text, None)),
        "click" => page.click(&target).and_then(|what| page.read().map(|text| (format!("Clicked {what}.\n\n{text}"), None))),
        "type" => page.type_text(&target, &words).and_then(|what| page.read().map(|text| (format!("Typed into {what}.\n\n{text}"), None))),
        "press" => page.press(&key).and_then(|_| page.read().map(|text| (format!("Pressed {key}.\n\n{text}"), None))),
        "screenshot" => page.screenshot().map(|png| ("A screenshot of the page is attached.".to_string(), Some(png))),
        other => Err(format!("`{other}` is not a browser action: open, read, click, type, press, screenshot, close")),
    });

    match result {
        Ok((text, Some(png))) => {
            completed(context, call_id, true, &format!("done · {} KB", png.len() / 1024), None);

            Outcome::picture(text, "image/png", &png)
        }
        Ok((text, None)) => {
            completed(context, call_id, true, "done", None);

            Outcome::ok(text)
        }
        Err(reason) => failed_with(context, call_id, reason),
    }
}

/// `apply_patch` (0.14): every file of a Codex patch, through the same guard, question, checkpoint and card
/// as an edit. Files are checked before any is written, so a hunk that does not fit changes nothing.
fn apply_patch(context: &mut ToolContext, call_id: &str, patch: &str) -> Outcome {
    let changes = match super::patch::parse(patch) {
        Ok(changes) if !changes.is_empty() => changes,
        Ok(_) => return Outcome::error("The patch changes no file."),
        Err(reason) => return Outcome::error(format!("The patch could not be read: {reason}")),
    };

    /* Everything worked out first: (path, before, after); after None = delete. */
    let mut planned: Vec<(String, String, Option<String>)> = Vec::new();

    for change in &changes {
        match change {
            super::patch::Change::Add { path, text } => {
                if context.workspace.exists(path) {
                    return Outcome::error(format!("`Add File: {path}` - the file already exists; update it instead."));
                }

                planned.push((path.clone(), String::new(), Some(text.clone())));
            }
            super::patch::Change::Delete { path } => match context.workspace.read(path) {
                Ok((before, _)) => planned.push((path.clone(), before, None)),
                Err(error) => return Outcome::error(format!("`Delete File: {path}`: {}", error.message)),
            },
            super::patch::Change::Update { path, move_to, hunks } => {
                let before = match context.workspace.read(path) {
                    Ok((_, true)) => return Outcome::error(format!("{path} is larger than 256 KB; patch it with edit_file on a range.")),
                    Ok((text, false)) => text,
                    Err(error) => return Outcome::error(format!("`Update File: {path}`: {}", error.message)),
                };
                let after = match super::patch::apply(&before, hunks) {
                    Ok(after) => after,
                    Err(reason) => return Outcome::error(format!("{path}: {reason} Nothing was changed.")),
                };

                match move_to {
                    Some(target) => {
                        planned.push((path.clone(), before, None));
                        planned.push((target.clone(), String::new(), Some(after)));
                    }
                    None => planned.push((path.clone(), before, Some(after))),
                }
            }
        }
    }

    let mut report = Vec::new();
    let mut cards: Vec<(String, String, String, Value)> = Vec::new();

    for (index, (path, before, after)) in planned.iter().enumerate() {
        let card_id = format!("{call_id}-{index}");
        let (verb, diff) = match after {
            Some(text) => (if before.is_empty() && !context.workspace.exists(path) { "Create" } else { "Edit" }, diff_lines(before, text)),
            None => ("Delete", diff_lines(before, "")),
        };
        let (added, removed) = counts(&diff);

        started(context, &card_id, "edit", verb, path);

        if let Some(refused) = guard_edit(context, path, after.as_deref()) {
            completed(context, &card_id, false, "declined", None);

            return refused;
        }

        if let Some(refused) = ask(
            context,
            "edit",
            &format!("{verb} {path}"),
            &format!("The agent's patch changes a file (+{added} −{removed} lines)"),
            path,
            "MUTATING",
            "A checkpoint is taken before the first change of this turn, so Rewind can put the file back.",
        ) {
            completed(context, &card_id, false, "declined", None);

            return refused;
        }

        checkpoint_first(context, &format!("Before patch {path}"));

        cards.push((card_id, format!("{verb} {path} (+{added} −{removed})"), format!("done · +{added} −{removed}"), card(&diff)));
    }

    /* P1-2: every file of the patch reaches the project in one step - all of them, or (a syntax error, a file that changed, a crash)
       none. It used to be written file by file, so a failure on the third left two of them changed. */
    let edits: Vec<(&str, Option<&str>)> = planned.iter().map(|(path, _, after)| (path.as_str(), after.as_deref())).collect();

    match commit_edits(context, &edits) {
        Ok(()) => {
            for (card_id, line, meta, shown) in cards {
                completed(context, &card_id, true, &meta, Some(shown));
                report.push(line);
            }
        }
        Err(error) => {
            for (card_id, ..) in &cards {
                completed(context, card_id, false, "failed", None);
            }

            return Outcome::error(format!("{}. The patch was not applied; nothing was changed.", error.message.trim_end_matches('.')));
        }
    }

    Outcome::ok(format!("Patch applied:\n{}", report.join("\n")))
}

/// Asks the person when the autonomy level says so; `Some(outcome)` is the refusal to hand the model.
fn ask(
    context: &mut ToolContext,
    kind: &'static str,
    title: &str,
    sub: &str,
    target: &str,
    risk: &str,
    explain: &str,
) -> Option<Outcome> {
    if !gate::needs_approval(context.autonomy, kind, risk) || context.always.contains(kind) && risk != "DANGEROUS" {
        return None;
    }

    match gate::ask(context.sink, context.turn_id, context.calls, kind, title, sub, target, risk, explain) {
        Decision::Allow => None,
        Decision::AlwaysAllow => {
            context.always.insert(kind);

            None
        }
        Decision::Deny => Some(Outcome::error(
            "The person declined this action. Do not try it again in another way; explain what you wanted to do, or continue without it.",
        )),
        Decision::DenyWith(reason) => Some(Outcome::error(declined_with(&reason))),
        Decision::ShowMe => Some(Outcome::error(
            "The person wants to see exactly what this does before allowing it. Stop calling tools now: show the exact change or command in your answer, say why it is needed, and wait for their reply.",
        )),
        Decision::Stopped => Some(Outcome::error("The turn was stopped.")),
    }
}

/// The text with line numbers, the way the model refers back to places in a file.
fn numbered(text: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(index, line)| format!("{:>5}\t{line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One line of a change: `(line number, text, "add" | "rem")`.
type DiffLine = (usize, String, &'static str);

/// The lines that changed between two texts: the common head and tail are skipped and the middle is
/// reported as removed-then-added, which is exactly what an `edit_file` did.
fn diff_lines(before: &str, after: &str) -> Vec<DiffLine> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let head = old.iter().zip(&new).take_while(|(left, right)| left == right).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let mut lines = Vec::new();

    for (offset, text) in old[head..old.len() - tail].iter().enumerate() {
        lines.push((head + offset + 1, text.to_string(), "rem"));
    }

    for (offset, text) in new[head..new.len() - tail].iter().enumerate() {
        lines.push((head + offset + 1, text.to_string(), "add"));
    }

    lines
}

fn counts(diff: &[DiffLine]) -> (usize, usize) {
    (
        diff.iter().filter(|line| line.2 == "add").count(),
        diff.iter().filter(|line| line.2 == "rem").count(),
    )
}

/// The Edit card's rows, in the shape `ToolCallCompleted.diff` carries.
fn card(diff: &[DiffLine]) -> Value {
    json!(diff
        .iter()
        .take(DIFF_CARD_LINES)
        .map(|(number, text, change)| json!({ "lineNumber": number.to_string(), "text": text, "change": change }))
        .collect::<Vec<_>>())
}

/// Head and tail of a long answer, with the cut said out loud.
fn cap(text: &str) -> String {
    cap_to(text, RESULT_CAP)
}

/// `cap` at another size - a local model's window.
fn cap_to(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }

    let head: String = text.chars().take(limit * 2 / 3).collect();
    let tail: String = {
        let reversed: String = text.chars().rev().take(limit / 3).collect();

        reversed.chars().rev().collect()
    };

    format!("{head}\n\n[… {} characters cut …]\n\n{tail}", text.len() - head.len() - tail.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::Recorder;

    fn context<'a>(workspace: &'a Workspace, sink: &'a EventSink, autonomy: Autonomy) -> ToolContext<'a> {
        static POLICY: std::sync::OnceLock<crate::trust::policy::Policy> = std::sync::OnceLock::new();

        ToolContext {
            workspace,
            session_id: "s-test",
            read_only: false,
            vision: false,
            sink,
            turn_id: "turn-t",
            autonomy,
            always: HashSet::new(),
            calls: 0,
            checkpoint: None,
            checkpointed: false,
            policy: POLICY.get_or_init(Default::default),
            changed: HashSet::new(),
            radius_allowed: false,
            plan: None,
            edits: 0,
            ran_at: None,
            research: None,
            local: false,
            result_cap: RESULT_CAP,
            reply_text: String::new(),
        }
    }

    /// The kernel's guard runs before the gate: a protected file waits for the person even in Auto,
    /// and a denied answer leaves the file untouched.
    #[test]
    fn a_protected_file_is_asked_about_in_auto_and_left_alone_when_declined() {
        let (root, workspace) = folder("protected");
        let recorder = Recorder::new();
        let sink = recorder.sink();

        std::fs::write(root.join("wp-config.php"), "<?php define('DB_NAME','x');").unwrap();

        let answering = std::thread::spawn(|| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);

            while !gate::resolve("perm-turn-t-1", "deny") {
                assert!(std::time::Instant::now() < deadline, "the protected path was never asked about");
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let mut context = context(&workspace, &sink, Autonomy::Auto);
        let call = ToolUse {
            id: "c1".into(),
            name: "write_file".into(),
            input: json!({ "path": "wp-config.php", "content": "<?php // replaced" }),
        };
        let outcome = execute(&mut context, &call);

        answering.join().unwrap();

        assert!(outcome.is_error);
        assert!(std::fs::read_to_string(root.join("wp-config.php")).unwrap().contains("DB_NAME"), "the file was not changed");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Phase 0.3: a file whose content holds a secret is asked about before the model sees it; allowed, the model
    /// gets the text with the value masked; declined, it gets nothing of the file.
    #[test]
    fn a_file_with_a_secret_inside_is_asked_about_and_reaches_the_model_masked() {
        let (root, workspace) = folder("content-guard");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let fake = "FAKEvalue9Zq4Xw7Lm2Pt8";

        std::fs::write(root.join("notes.txt"), format!("deploy host: example.org\npassword={fake}\n")).unwrap();

        let answer = |decision: &'static str, id: &'static str| {
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);

                while !gate::resolve(id, decision) {
                    assert!(std::time::Instant::now() < deadline, "the secret question was never asked");
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };

        let mut context = context(&workspace, &sink, Autonomy::Auto);

        /* its own turn id: permission questions are keyed by it, and other tests in this module answer "perm-turn-t-1" */
        context.turn_id = "turn-secret-guard";

        let allowing = answer("allow", "perm-turn-secret-guard-1");
        let allowed = execute(&mut context, &call("read_file", json!({ "path": "notes.txt" })));

        allowing.join().unwrap();

        assert!(!allowed.is_error, "{}", allowed.content);
        assert!(allowed.content.contains("deploy host") && allowed.content.contains("password=") && (allowed.content.contains("[masked by SDC") || allowed.content.contains("‹secret:")), "{}", allowed.content);
        assert!(!allowed.content.contains(fake), "the value never reaches the model");

        let declining = answer("deny", "perm-turn-secret-guard-2");
        let declined = execute(&mut context, &call("read_file", json!({ "path": "notes.txt" })));

        declining.join().unwrap();

        assert!(declined.is_error);
        assert!(!declined.content.contains(fake) && !declined.content.contains("deploy host"));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// P1-2: a write that would newly break a file's syntax is refused before anything changes, and says where.
    #[test]
    fn a_write_that_would_break_the_syntax_is_refused_and_the_file_is_untouched() {
        let (root, workspace) = folder("syntax-gate");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        std::fs::write(root.join("lib.rs"), "fn good() -> u32 { 1 }\n").unwrap();

        let broken = execute(&mut context, &call("write_file", json!({ "path": "lib.rs", "content": "fn good( -> u32 {\n" })));

        assert!(broken.is_error, "{}", broken.content);
        assert!(broken.content.contains("syntax error at line 1") && broken.content.contains("nothing in this step was changed"), "{}", broken.content);
        assert_eq!(std::fs::read_to_string(root.join("lib.rs")).unwrap(), "fn good() -> u32 { 1 }\n");

        let fine = execute(&mut context, &call("write_file", json!({ "path": "lib.rs", "content": "fn good() -> u32 { 2 }\n" })));

        assert!(!fine.is_error, "{}", fine.content);
        assert_eq!(std::fs::read_to_string(root.join("lib.rs")).unwrap(), "fn good() -> u32 { 2 }\n");
        assert!(!root.join(".sdc/tx").read_dir().map(|mut dir| dir.next().is_some()).unwrap_or(false), "no transaction was left behind");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// P1-2: a patch of two files reaches the project as a whole or not at all - the second file's syntax error leaves the first untouched.
    #[test]
    fn a_patch_whose_second_file_is_broken_changes_neither_file() {
        let (root, workspace) = folder("patch-atomic");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        std::fs::write(root.join("a.py"), "def a():\n    return 1\n").unwrap();

        let patch = "*** Begin Patch\n*** Update File: a.py\n@@\n def a():\n-    return 1\n+    return 2\n*** Add File: b.py\n+def b(:\n+    pass\n*** End Patch";
        let outcome = execute(&mut context, &call("apply_patch", json!({ "patch": patch })));

        assert!(outcome.is_error, "{}", outcome.content);
        assert!(outcome.content.contains("b.py") && outcome.content.contains("nothing was changed"), "{}", outcome.content);
        assert_eq!(std::fs::read_to_string(root.join("a.py")).unwrap(), "def a():\n    return 1\n", "the valid half was not applied alone");
        assert!(!root.join("b.py").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// P1-2: SDC's own `.sdc/` files (the /init command writes `.sdc/rules.md`) are not project sources and are still writable.
    #[test]
    fn files_under_sdc_are_still_written() {
        let (root, workspace) = folder("own-files");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);
        let outcome = execute(&mut context, &call("write_file", json!({ "path": ".sdc/rules.md", "content": "# rules\n" })));

        assert!(!outcome.is_error, "{}", outcome.content);
        assert_eq!(std::fs::read_to_string(root.join(".sdc/rules.md")).unwrap(), "# rules\n");

        let _ = std::fs::remove_dir_all(&root);
    }

    fn python_with_two_adds() -> String {
        "class Store:\n    def add(self, x):\n        return x\n\n    def remove(self, x):\n        return None\n\n\nclass Cart:\n    def add(self, x):\n        return x + 1\n".to_string()
    }

    fn hash_of(path: &str, text: &str, qualified: &str) -> String {
        crate::intel::symbols(path, text).into_iter().find(|symbol| symbol.qualified == qualified).unwrap().hash
    }

    /// P1-11: a symbol is found by its qualified name and hash; a changed, missing or duplicated symbol changes nothing.
    #[test]
    fn a_symbol_patch_is_applied_only_to_the_one_symbol_with_the_hash_that_was_read() {
        let text = python_with_two_adds();
        let hash = hash_of("s.py", &text, "Store.add");

        /* the whole declaration, for a small symbol */
        let patched = plan_symbol_patch("s.py", &text, "Store.add", &hash[..10], Some("def add(self, x):\n        return x * 2"), &[]).unwrap();

        assert!(patched.contains("return x * 2") && patched.contains("class Cart:\n    def add(self, x):\n        return x + 1"), "{patched}");
        assert!(patched.contains("def remove(self, x):"), "the neighbours are untouched");

        /* the plain name is ambiguous: two `add`s */
        let ambiguous = plan_symbol_patch("s.py", &text, "add", &hash[..10], Some("def add(self): pass"), &[]).unwrap_err();

        assert!(ambiguous.contains("names 2 declarations") && ambiguous.contains("Cart.add") && ambiguous.contains("Nothing was changed"), "{ambiguous}");

        /* a stale hash */
        let stale = plan_symbol_patch("s.py", &text, "Store.add", "deadbeef00", Some("def add(self): pass"), &[]).unwrap_err();

        assert!(stale.contains("is not what you read") && stale.contains(&hash[..12]), "{stale}");

        /* a name that is not there lists what is */
        let missing = plan_symbol_patch("s.py", &text, "Store.nothing", &hash[..10], Some("x"), &[]).unwrap_err();

        assert!(missing.contains("Store.remove") && missing.contains("Cart.add"), "{missing}");
        assert!(plan_symbol_patch("s.py", &text, "Store.add", "xyz", Some("x"), &[]).unwrap_err().contains("8 hex digits"));
        assert!(plan_symbol_patch("notes.txt", "x", "a", "00000000", Some("x"), &[]).unwrap_err().contains("not a language"));
    }

    #[test]
    fn edits_inside_a_symbol_must_match_exactly_once_or_by_whitespace_alone() {
        let text = "def f(a):\n    x = a + 1\n    y = a + 1\n    return x\n";
        let hash = hash_of("f.py", text, "f");
        let edit = |old: &str, new: &str| vec![(old.to_string(), new.to_string())];

        /* twice: refused, with the count */
        let twice = plan_symbol_patch("f.py", text, "f", &hash, None, &edit("a + 1", "a + 2")).unwrap_err();

        assert!(twice.contains("appears 2 times"), "{twice}");

        /* exactly once */
        let once = plan_symbol_patch("f.py", text, "f", &hash, None, &edit("x = a + 1", "x = a + 5")).unwrap();

        assert!(once.contains("x = a + 5") && once.contains("y = a + 1"), "{once}");

        /* whitespace drift: the model typed one space where the file has more */
        let drifted = "def g(a):\n    total  =   a   +  1\n    return total\n";
        let ok = plan_symbol_patch("g.py", drifted, "g", &hash_of("g.py", drifted, "g"), None, &edit("total = a + 1", "total = a + 9")).unwrap();

        assert!(ok.contains("total = a + 9"), "{ok}");

        /* not fuzzy: a different token is not found */
        assert!(plan_symbol_patch("g.py", drifted, "g", &hash_of("g.py", drifted, "g"), None, &edit("total = a - 1", "x")).unwrap_err().contains("was not found"));
    }

    #[test]
    fn a_large_symbol_cannot_be_rewritten_whole_and_both_forms_cannot_be_mixed() {
        let big = format!("def big():\n{}    return 0\n", "    x = 1\n".repeat(70));
        let hash = hash_of("b.py", &big, "big");

        assert!(plan_symbol_patch("b.py", &big, "big", &hash, Some("def big():\n    return 1"), &[]).unwrap_err().contains("too large to rewrite whole"));
        assert!(plan_symbol_patch("b.py", &big, "big", &hash, Some("x"), &[("a".into(), "b".into())]).unwrap_err().contains("not both"));
        assert!(plan_symbol_patch("b.py", &big, "big", &hash, None, &[]).unwrap_err().contains("new_body"));
    }

    /// P1-11 fuzz: duplicate names and drifted whitespace never produce a silent misapply - every accepted patch changes exactly the
    /// intended symbol's bytes, and every refused one changes nothing.
    #[test]
    fn fuzzed_duplicate_names_and_drifted_whitespace_never_misapply() {
        let mut seed = 0x9e37_79b9_u64;
        let mut next = |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };

        for round in 0..60 {
            let classes = 2 + next(3) as usize;
            let spaces = |n: u64| " ".repeat(1 + n as usize);
            let mut text = String::new();

            for class in 0..classes {
                text.push_str(&format!("class C{class}:\n    def run(self, v):\n        return v{}+{}{class}\n\n", spaces(next(3)), spaces(next(3))));
            }

            let target = next(classes as u64) as usize;
            let qualified = format!("C{target}.run");
            let hash = hash_of("m.py", &text, &qualified);

            /* the right symbol, the right hash: only C{target}.run changes */
            let patched = plan_symbol_patch("m.py", &text, &qualified, &hash, Some(&format!("def run(self, v):\n        return 'patched{round}'")), &[]).unwrap();

            for class in 0..classes {
                let still = patched.contains(&format!("class C{class}:"));

                assert!(still, "round {round}: class {class} vanished");
            }

            assert_eq!(patched.matches("patched").count(), 1, "round {round}: exactly one symbol changed");

            let definition = crate::graph::parse::definitions(crate::graph::Lang::Python, &crate::graph::parse::parse(crate::graph::Lang::Python, &patched).unwrap(), &patched);

            for def in definitions_other_than(&definition, &qualified) {
                let original = hash_of("m.py", &text, &def);
                let now = hash_of("m.py", &patched, &def);

                assert_eq!(original, now, "round {round}: {def} must be byte-identical");
            }

            /* the bare name, a wrong qualifier and a wrong hash are all refused */
            assert!(plan_symbol_patch("m.py", &text, "run", &hash, Some("def run(self): pass"), &[]).is_err());
            assert!(plan_symbol_patch("m.py", &text, "Nope.run", &hash, Some("def run(self): pass"), &[]).is_err());
            assert!(plan_symbol_patch("m.py", &text, &qualified, "0123456789ab", Some("def run(self): pass"), &[]).is_err());
        }
    }

    fn definitions_other_than(defs: &[crate::graph::Def], qualified: &str) -> Vec<String> {
        defs.iter().map(|def| def.qualified.clone()).filter(|name| name != qualified && name.ends_with(".run")).collect()
    }

    #[test]
    fn the_patch_symbol_tool_writes_through_the_gate_and_refuses_a_cut_off_body() {
        let (root, workspace) = folder("patch-symbol");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);
        let text = python_with_two_adds();
        let hash = hash_of("s.py", &text, "Cart.add");

        std::fs::write(root.join("s.py"), &text).unwrap();

        let cut = execute(&mut context, &call("patch_symbol", json!({ "path": "s.py", "symbol": "Cart.add", "old_hash": hash, "new_body": "def add(self, x):\n        return foo(x," })));

        assert!(cut.is_error && cut.content.contains("looks cut off"), "{}", cut.content);
        assert_eq!(std::fs::read_to_string(root.join("s.py")).unwrap(), text, "a cut-off body changed nothing");

        let good = execute(&mut context, &call("patch_symbol", json!({ "path": "s.py", "symbol": "Cart.add", "old_hash": hash, "new_body": "def add(self, x):\n        return x + 100" })));

        assert!(!good.is_error, "{}", good.content);
        assert!(good.content.contains("It is now #"), "{}", good.content);
        assert!(std::fs::read_to_string(root.join("s.py")).unwrap().contains("return x + 100"));

        let again = execute(&mut context, &call("patch_symbol", json!({ "path": "s.py", "symbol": "Cart.add", "old_hash": hash, "new_body": "def add(self, x):\n        return 0" })));

        assert!(again.is_error && again.content.contains("is not what you read"), "the same hash cannot be used twice: {}", again.content);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// N6-3 end to end: a signature a verified step published cannot be changed by a later write; a body change is fine; off, nothing is locked.
    #[test]
    fn a_locked_signature_cannot_be_changed_by_a_later_write() {
        let (root, workspace) = folder("contract-lock");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);
        let shop = "pub fn total(items: &[u64]) -> u64 {\n    items.iter().sum()\n}\n";

        std::fs::write(root.join("shop.rs"), shop).unwrap();

        let mut lock = crate::navaratna::contracts::Lock::default();

        crate::v3::forcing(&["nr.contract_lock"], || lock.publish("shop.rs", shop, "s1"));
        lock.save(&root).unwrap();

        let changed = shop.replace("(items: &[u64])", "(items: &[u64], tax: u64)");
        let refused = crate::v3::forcing(&["nr.contract_lock"], || execute(&mut context, &call("write_file", json!({ "path": "shop.rs", "content": changed }))));

        assert!(refused.is_error && refused.content.contains("locked contract"), "{}", refused.content);
        assert_eq!(std::fs::read_to_string(root.join("shop.rs")).unwrap(), shop, "nothing was written");

        let body = shop.replace("items.iter().sum()", "items.iter().copied().sum()");
        let fine = crate::v3::forcing(&["nr.contract_lock"], || execute(&mut context, &call("write_file", json!({ "path": "shop.rs", "content": body }))));

        assert!(!fine.is_error, "{}", fine.content);

        let off = execute(&mut context, &call("write_file", json!({ "path": "shop.rs", "content": changed })));

        assert!(!off.is_error, "with the switch off the old behaviour stands: {}", off.content);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// N3-3 end to end: a failing command's output is cut to its first error with the code around it and a pointer to the whole log; with the
    /// switch off the model gets the whole output, as before.
    #[cfg(unix)]
    #[test]
    fn a_failing_command_shows_its_first_error_and_keeps_the_whole_log() {
        let (root, workspace) = folder("error-slice");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), (1..=20).map(|n| format!("let line_{n} = {n};\n")).collect::<String>()).unwrap();
        std::fs::write(root.join("fail.sh"), "for n in 1 2 3 4 5 6 7 8; do echo \"warning: noise $n\"; done\necho 'error[E0308]: mismatched types'\necho ' --> src/a.rs:7:5'\necho 'error[E0425]: cannot find value'\necho ' --> src/a.rs:9:5'\nexit 1\n").unwrap();

        let plain = execute(&mut context, &call("run_command", json!({ "command": "sh fail.sh" })));

        assert!(plain.content.contains("warning: noise 8"), "off: the whole output, as it always was: {}", plain.content);

        let cloud = crate::v3::forcing(&["nr.error_slice"], || execute(&mut context, &call("run_command", json!({ "command": "sh fail.sh" }))));

        assert!(cloud.content.contains("warning: noise 8"), "a cloud model is untouched whatever the switch says");

        context.local = true;

        let sliced = crate::v3::forcing(&["nr.error_slice"], || execute(&mut context, &call("run_command", json!({ "command": "sh fail.sh" }))));

        assert!(sliced.content.contains("src/a.rs:7") && sliced.content.contains("mismatched types") && sliced.content.contains("let line_7 = 7;"), "{}", sliced.content);
        assert!(sliced.content.contains("(+1 more error") && !sliced.content.contains("warning: noise 8"), "{}", sliced.content);

        let id = sliced.content.split("spec_read log ").nth(1).and_then(|rest| rest.split(')').next()).unwrap();

        assert!(crate::navaratna::vault::get(&root, "log", id).unwrap().contains("warning: noise 8"), "the whole log is in the vault");

        /* and the model can fetch any part of it with spec_read */
        let part = execute(&mut context, &call("spec_read", json!({ "kind": "log", "id": id, "from": 1, "to": 2 })));

        assert!(!part.is_error && part.content.contains("warning: noise 1") && !part.content.contains("noise 3"), "{}", part.content);
        assert!(execute(&mut context, &call("spec_read", json!({ "kind": "log", "id": "ffff" }))).is_error);
        assert!(!crate::v3::forcing(&["nr.error_slice"], || specs_for(Caps { vision: false, subagent: false, patch: false, local: true })).iter().all(|spec| spec.name != "spec_read"), "offered when the slicer is on");
        assert!(specs_for(Caps { vision: false, subagent: false, patch: false, local: true }).iter().all(|spec| spec.name != "spec_read"), "and not otherwise");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// N4-2 end to end: the definition comes from the message as a code block; elision and a rename are refused; everything else in the file is
    /// byte-identical afterwards; with the switch off the model is not offered the tool.
    #[test]
    fn the_replace_symbol_tool_takes_the_definition_from_the_message_and_changes_nothing_else() {
        let (root, workspace) = folder("replace-symbol");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);
        let text = python_with_two_adds();

        std::fs::write(root.join("s.py"), &text).unwrap();

        let silent = execute(&mut context, &call("replace_symbol", json!({ "path": "s.py", "symbol": "Cart.add" })));

        assert!(silent.is_error && silent.content.contains("fenced"), "{}", silent.content);

        context.reply_text = "Here it is:\n```python\ndef add(self, x):\n    # rest unchanged\n    return x\n```".to_string();

        let lazy = execute(&mut context, &call("replace_symbol", json!({ "path": "s.py", "symbol": "Cart.add" })));

        assert!(lazy.is_error && lazy.content.contains("leaves code out"), "{}", lazy.content);

        context.reply_text = "```python\ndef remove(self, x):\n    return x\n```".to_string();

        let renamed = execute(&mut context, &call("replace_symbol", json!({ "path": "s.py", "symbol": "Cart.add" })));

        assert!(renamed.is_error && renamed.content.contains("exactly the one symbol"), "{}", renamed.content);
        assert_eq!(std::fs::read_to_string(root.join("s.py")).unwrap(), text, "refusals changed nothing");

        context.reply_text = "```python\ndef add(self, x):\n    return x + 100\n```".to_string();

        let good = execute(&mut context, &call("replace_symbol", json!({ "path": "s.py", "symbol": "Cart.add" })));

        assert!(!good.is_error, "{}", good.content);

        let after = std::fs::read_to_string(root.join("s.py")).unwrap();

        assert!(after.contains("return x + 100") && !after.contains("# rest unchanged"));
        assert!(after.lines().count().abs_diff(text.lines().count()) <= 2);

        let names = |on: bool| -> Vec<&'static str> {
            let caps = Caps { vision: false, subagent: false, patch: false, local: true };

            if on { crate::v3::forcing(&["nr.symbol_patch"], || specs_for(caps)) } else { specs_for(caps) }.into_iter().map(|spec| spec.name).collect()
        };

        assert!(names(true).contains(&"replace_symbol") && !names(false).contains(&"replace_symbol"), "off by default: the model is told nothing new");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// P1-12 end to end: with `verify.lsp` on, an edit's answer carries the language server's errors and the declaration of the name they
    /// mention; with it off the answer is the plain "Replaced" it always was.
    #[test]
    fn an_edit_answers_with_the_language_servers_errors_when_the_switch_is_on() {
        let (root, workspace) = folder("lsp-note");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp/mock_server.py");

        std::fs::write(root.join("pay.py"), "def BAD(a):\n    return a\n").unwrap();
        std::env::set_var("SDC_LSP_COMMAND", serde_json::to_string(&[if cfg!(windows) { "python" } else { "python3" }, "-I", &script.to_string_lossy()]).unwrap());

        let on = crate::v3::forcing(&["verify.lsp"], || execute(&mut context, &call("write_file", json!({ "path": "app.py", "content": "import pay\nx = BAD(1)\n" }))));
        let off = execute(&mut context, &call("write_file", json!({ "path": "app2.py", "content": "import pay\nx = BAD(1)\n" })));

        std::env::remove_var("SDC_LSP_COMMAND");
        crate::verify::lsp::stop_all();

        assert!(!on.is_error, "{}", on.content);
        assert!(on.content.contains("language server reports 1 error(s) in app.py") && on.content.contains("line 2:5"), "{}", on.content);
        assert!(on.content.contains("`BAD` is declared at pay.py:1: def BAD(a):"), "{}", on.content);
        assert!(!off.content.contains("language server"), "off: the plain answer: {}", off.content);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_new_tools_are_offered_only_when_their_switches_are_on() {
        let names = |on: &[&str]| crate::v3::forcing(on, || specs_for(Caps { vision: false, subagent: false, patch: false, local: true }).iter().map(|spec| spec.name).collect::<Vec<_>>());

        assert!(!names(&[]).contains(&"patch_symbol") && !names(&[]).contains(&"code_graph"), "off by default: the model is told nothing new");
        assert!(names(&["edit.symbol_patch"]).contains(&"patch_symbol"));
        assert!(names(&["intel"]).contains(&"code_graph") && names(&["intel"]).contains(&"find_symbol"));
        assert!(!crate::v3::forcing(&["edit.symbol_patch"], || specs_for(Caps { vision: false, subagent: true, patch: false, local: true })).iter().any(|spec| spec.name == "patch_symbol"), "a read-only sub-agent never gets an editing tool");
    }

    /// P1-8: the ranked search answers from the project's own names and declarations, with no model call.
    #[test]
    fn the_ranked_search_finds_the_declaration_and_says_where_it_is() {
        let (root, workspace) = folder("ranked-search");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        std::fs::create_dir_all(root.join("billing")).unwrap();
        std::fs::write(root.join("billing/pay.py"), "def charge(amount):\n    return amount * 2\n").unwrap();
        std::fs::write(root.join("app.py"), "from pay import charge\n\ndef run():\n    return charge(1)\n").unwrap();

        let found = execute(&mut context, &call("ranked_search", json!({ "query": "where is charge defined", "k": 3 })));

        assert!(!found.is_error, "{}", found.content);
        assert!(found.content.contains("billing/pay.py:1-2 [declares]"), "{}", found.content);
        assert!(found.content.contains("def charge(amount):"));
        assert!(execute(&mut context, &call("ranked_search", json!({ "query": "nothing like this exists", "k": 2 }))).content.contains("Nothing in this project matches"));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// M0-5: with the whole SME group off (the plain arm of Gate 1) nothing crashes - the tools that depend on the graph are not offered, and
    /// a call to one anyway is answered, not trusted to work.
    #[test]
    fn with_the_sme_group_off_no_tool_is_offered_that_needs_it_and_a_stray_call_does_not_crash() {
        let (root, workspace) = folder("plain-arm");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        std::fs::write(root.join("app.py"), "def run():\n    return 1\n").unwrap();

        let group = crate::bench::group_features("sme").unwrap();

        crate::v3::ablating(&group, || {
            crate::v3::forcing(&["intel"], || {
                let offered: Vec<&str> = specs_for(Caps { vision: false, subagent: false, patch: false, local: true }).iter().map(|spec| spec.name).collect();

                assert!(!offered.contains(&"ranked_search") && !offered.contains(&"code_graph"), "{offered:?}");
            });

            for stray in [call("ranked_search", json!({ "query": "run" })), call("code_graph", json!({ "query": "where", "name": "run" }))] {
                let answered = execute(&mut context, &stray);

                assert!(!answered.content.is_empty(), "a call to a tool that is off still gets an answer");
            }
        });

        let _ = std::fs::remove_dir_all(&root);
    }

    /// M4-1: an edit that imports a name the installed package does not have is answered with a warning and the nearest real name - and the edit is still made.
    #[test]
    fn an_edit_that_uses_a_name_the_installed_package_lacks_gets_a_warning_and_is_not_blocked() {
        let (root, workspace) = folder("api-exists");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        std::fs::create_dir_all(root.join("node_modules/react-query")).unwrap();
        std::fs::write(root.join("node_modules/react-query/index.d.ts"), "export declare function useQuery(): void;\n").unwrap();

        let invented = execute(&mut context, &call("write_file", json!({ "path": "app.ts", "content": "import { useQuerry } from 'react-query';\n" })));

        assert!(!invented.is_error, "{}", invented.content);
        assert!(invented.content.contains("`useQuerry` is not declared by `react-query`") && invented.content.contains("did you mean `useQuery`"), "{}", invented.content);
        assert!(root.join("app.ts").exists(), "nothing was blocked");

        let real = execute(&mut context, &call("write_file", json!({ "path": "ok.ts", "content": "import { useQuery } from 'react-query';\n" })));

        assert!(!real.content.contains("SDC check"), "{}", real.content);

        let off = crate::v3::ablating(&["verify.api_exists"], || execute(&mut context, &call("write_file", json!({ "path": "off.ts", "content": "import { nope } from 'react-query';\n" }))));

        assert!(!off.content.contains("SDC check"), "switched off, it says nothing");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_ranked_search_is_offered_only_when_its_switch_is_on() {
        let names = |on: &[&str]| crate::v3::forcing(on, || specs_for(Caps { vision: false, subagent: false, patch: false, local: true }).iter().map(|spec| spec.name).collect::<Vec<_>>());

        assert!(names(&[]).contains(&"ranked_search"), "on by default: it costs no model call");
        assert!(!crate::v3::ablating(&["sme.retrieval"], || names(&[])).contains(&"ranked_search"));
    }

    /// P1-9: a remembered fact that quotes a file is checked against the file; a quote the file does not have is refused and nothing
    /// is written.
    #[test]
    fn a_remembered_quote_that_is_not_in_the_file_is_refused() {
        let (root, workspace) = folder("quote-guard");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        std::fs::create_dir_all(root.join("billing")).unwrap();
        std::fs::write(root.join("billing/pay.py"), "def charge(amount, currency):\n    return amount\n").unwrap();

        let fake = execute(&mut context, &call("remember", json!({ "fact": "charge takes one argument", "source": "billing/pay.py", "quote": "def charge(amount):" })));

        assert!(fake.is_error, "{}", fake.content);
        assert!(fake.content.contains("is not in billing/pay.py") && fake.content.contains("was not kept"), "{}", fake.content);
        assert!(!root.join(".sdc/memory.md").exists(), "nothing was remembered");

        let real = execute(&mut context, &call("remember", json!({ "fact": "charge takes an amount and a currency", "source": "billing/pay.py", "quote": "def charge(amount, currency):" })));

        assert!(!real.is_error, "{}", real.content);

        let remembered = std::fs::read_to_string(root.join(".sdc/memory.md")).unwrap_or_default();

        assert!(remembered.contains("billing/pay.py: `def charge(amount, currency):`"), "{remembered}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_command_the_policy_denies_is_refused_without_running() {
        let (root, workspace) = folder("denied");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let policy = crate::trust::policy::Policy::parse("deny_commands = [\"drop database\"]", ".sdc/policy.toml");
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        context.policy = Box::leak(Box::new(policy));

        let call = ToolUse { id: "c1".into(), name: "run_command".into(), input: json!({ "command": "mysql -e 'DROP DATABASE shop'" }) };
        let outcome = execute(&mut context, &call);

        assert!(outcome.is_error);
        assert!(outcome.content.contains("denies"), "{}", outcome.content);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The blast radius (0.12): a turn that would change more files than the policy allows is asked
    /// about even in Auto, and the file that would cross the limit is left unwritten when declined.
    #[test]
    fn a_turn_past_the_blast_radius_is_asked_about_and_the_extra_file_is_left_alone() {
        let (root, workspace) = folder("blast-radius");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let policy = crate::trust::policy::Policy::parse("max_files_per_turn = 2", ".sdc/policy.toml");
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        context.policy = Box::leak(Box::new(policy));

        let answering = std::thread::spawn(|| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);

            while !gate::resolve("perm-turn-t-3", "deny") {
                assert!(std::time::Instant::now() < deadline, "the blast-radius question was never asked");
                std::thread::sleep(Duration::from_millis(10));
            }
        });

        assert!(!execute(&mut context, &call("write_file", json!({ "path": "a.txt", "content": "1" }))).is_error);
        assert!(!execute(&mut context, &call("write_file", json!({ "path": "b.txt", "content": "2" }))).is_error);

        let third = execute(&mut context, &call("write_file", json!({ "path": "c.txt", "content": "3" })));

        answering.join().unwrap();

        assert!(third.is_error);
        assert!(root.join("a.txt").exists());
        assert!(root.join("b.txt").exists());
        assert!(!root.join("c.txt").exists(), "the file over the limit must not be written");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 0.16.1: a local model reads the web only through `/research`; an API model keeps its web tools,
    /// and a long answer is cut to a local model's window.
    #[test]
    fn a_local_model_is_kept_off_the_web_outside_research() {
        let (root, workspace) = folder("webgate");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut local = context(&workspace, &sink, Autonomy::Auto);

        local.local = true;

        /* The gate is a setting, off by default since the live check; turned on, the tool refuses. */
        let store = crate::store::sqlite::Store::in_memory().unwrap();

        store.set_setting("research.localWebOnly", "on").unwrap();
        crate::agent::research::configure(&store);

        let search = ToolUse { id: "w1".into(), name: "web_search".into(), input: json!({ "query": "anything" }) };
        let refused = execute(&mut local, &search);

        crate::agent::research::configure(&crate::store::sqlite::Store::in_memory().unwrap());
        assert!(refused.is_error && refused.content.contains("/research"), "{}", refused.content);

        local.result_cap = 3_000;
        std::fs::write(root.join("long.txt"), "line of text\n".repeat(2_000)).unwrap();

        let read = execute(&mut local, &ToolUse { id: "r1".into(), name: "read_file".into(), input: json!({ "path": "long.txt" }) });

        assert!(read.content.len() < 3_200 && read.content.contains("obs_read"), "{}", read.content.len());

        let _ = std::fs::remove_dir_all(&root);
    }

    fn folder(name: &str) -> (std::path::PathBuf, Workspace) {
        let root = std::env::temp_dir().join(format!("sdc-agent-tools-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::new(root.to_str().unwrap(), None);

        (root, workspace)
    }

    fn call(name: &str, input: Value) -> ToolUse {
        ToolUse { id: "x".into(), name: name.into(), input }
    }

    #[test]
    fn every_tool_has_a_strict_object_schema() {
        let specs = specs_for(Caps { vision: true, subagent: false, patch: true, local: true });

        assert_eq!(specs.len(), 24);
        assert_eq!(specs_for(Caps { local: true, ..Caps::default() }).len(), 21, "no image tools, no patch");
        assert_eq!(specs_for(Caps::default()).len(), 20, "a cloud model is not offered ranked_search: it is unmeasured, so it is local-only (M0-3)");
        assert!(!specs_for(Caps::default()).iter().any(|spec| spec.name == "ranked_search"));
        assert_eq!(specs_for(Caps { vision: false, subagent: true, patch: true, local: true }).len(), 10, "a sub-agent only reads");
        assert!(specs_for(Caps { vision: false, subagent: true, patch: false, local: true }).iter().all(|spec| READ_ONLY.contains(&spec.name)));

        for spec in specs {
            assert_eq!(spec.schema["type"], "object", "{}", spec.name);
            assert_eq!(spec.schema["additionalProperties"], false, "{}", spec.name);
        }
    }

    #[test]
    fn an_edit_replaces_exactly_one_occurrence_and_draws_its_diff() {
        let (root, workspace) = folder("edit");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        workspace.write("pay.js", "a\nconst x = Math.round(v);\nb\n").unwrap();

        let outcome = execute(
            &mut context,
            &call("edit_file", json!({ "path": "pay.js", "old_text": "Math.round(v)", "new_text": "Math.round(v * 100) / 100" })),
        );

        assert!(!outcome.is_error, "{}", outcome.content);
        assert_eq!(workspace.read("pay.js").unwrap().0, "a\nconst x = Math.round(v * 100) / 100;\nb\n");

        let completed = recorder.events().into_iter().find_map(|event| match event {
            EngineEvent::ToolCompleted { meta, diff, .. } => Some((meta, diff)),
            _ => None,
        });
        let (meta, diff) = completed.unwrap();

        assert_eq!(meta, "done · +1 −1");
        assert_eq!(diff.unwrap()[0], json!({ "lineNumber": "2", "text": "const x = Math.round(v);", "change": "rem" }));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_edit_that_is_not_unique_or_not_found_is_an_error_the_model_can_fix() {
        let (root, workspace) = folder("edit-err");
        let sink = EventSink::discarding();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        workspace.write("a.txt", "same\nsame\n").unwrap();

        let twice = execute(&mut context, &call("edit_file", json!({ "path": "a.txt", "old_text": "same", "new_text": "x" })));
        let missing = execute(&mut context, &call("edit_file", json!({ "path": "a.txt", "old_text": "nope", "new_text": "x" })));
        let all = execute(
            &mut context,
            &call("edit_file", json!({ "path": "a.txt", "old_text": "same", "new_text": "x", "replace_all": true })),
        );

        assert!(twice.is_error && twice.content.contains("2 times"));
        assert!(missing.is_error && missing.content.contains("not found"));
        assert!(!all.is_error);
        assert_eq!(workspace.read("a.txt").unwrap().0, "x\nx\n");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failing_command_is_information_for_the_model_not_a_tool_error() {
        let (root, workspace) = folder("run");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);
        let line = if cfg!(windows) { "echo broken 1>&2 && exit 3" } else { "echo broken >&2; exit 3" };
        let outcome = execute(&mut context, &call("run_command", json!({ "command": line })));

        assert!(!outcome.is_error);
        assert!(outcome.content.contains("exit code: 3"), "{}", outcome.content);
        assert!(outcome.content.contains("broken"));
        assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::ToolCompleted { status, .. } if status == "failed")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_plan_is_sent_to_the_window_and_empty_steps_are_refused() {
        let (root, workspace) = folder("plan");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Ask);
        let outcome = execute(
            &mut context,
            &call("update_plan", json!({ "steps": [{ "text": "Read", "status": "done" }, { "text": "Fix", "status": "in_progress" }, { "text": " ", "status": "pending" }] })),
        );

        assert!(outcome.content.starts_with("Plan shown: 1 of 2 steps done."), "{}", outcome.content);
        assert_eq!(
            recorder.events(),
            vec![EngineEvent::Plan(json!([{ "text": "Read", "status": "done" }, { "text": "Fix", "status": "in_progress" }]))]
        );
        assert!(execute(&mut context, &call("update_plan", json!({ "steps": [] }))).is_error);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_tools_and_broken_input_get_a_sentence() {
        let (root, workspace) = folder("unknown");
        let sink = EventSink::discarding();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        assert!(execute(&mut context, &call("delete_everything", json!({}))).content.contains("no tool called"));
        assert!(execute(&mut context, &call("read_file", json!({ "__invalid_json": "{\"pa" }))).content.contains("not valid JSON"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_range_read_numbers_its_lines_and_says_where_it_stopped() {
        let (root, workspace) = folder("range");
        let sink = EventSink::discarding();
        let mut context = context(&workspace, &sink, Autonomy::Auto);
        let text: String = (1..=50).map(|n| format!("line {n}\n")).collect();

        workspace.write("big.txt", &text).unwrap();

        let outcome = execute(&mut context, &call("read_file", json!({ "path": "big.txt", "offset": 10, "limit": 3 })));

        assert_eq!(outcome.content, "   10\tline 10\n   11\tline 11\n   12\tline 12\n[lines 10-12 of 50 · more: read_file with offset=13]");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 0.19: a read's card shows what the model read - the file's first lines, numbered - after its end.
    #[test]
    fn a_read_shows_the_person_what_the_model_read() {
        let (root, workspace) = folder("seen");
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        workspace.write("notes.txt", "first\nsecond\n").unwrap();
        execute(&mut context, &call("read_file", json!({ "path": "notes.txt" })));

        let shown: Vec<String> = recorder
            .events()
            .into_iter()
            .filter_map(|event| match event {
                EngineEvent::ToolOutput { text, .. } => Some(text),
                _ => None,
            })
            .collect();

        assert_eq!(shown, vec!["    1\tfirst".to_string(), "    2\tsecond".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_patch_changes_every_file_or_none() {
        let (root, workspace) = folder("patch");
        let sink = EventSink::discarding();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        workspace.write("src/math.js", "function add(a, b) {\n  return a - b;\n}\n").unwrap();
        workspace.write("old.js", "bye\n").unwrap();

        let good = "*** Begin Patch\n*** Update File: src/math.js\n@@\n function add(a, b) {\n-  return a - b;\n+  return a + b;\n*** Add File: src/mul.js\n+module.exports = (a, b) => a * b;\n*** Delete File: old.js\n*** End Patch";
        let outcome = execute(&mut context, &call("apply_patch", json!({ "patch": good })));

        assert!(!outcome.is_error, "{}", outcome.content);
        assert!(workspace.read("src/math.js").unwrap().0.contains("a + b"));
        assert!(workspace.exists("src/mul.js"));
        assert!(!workspace.exists("old.js"));

        /* A hunk that does not fit leaves every file as it was, the ones before it included. */
        let bad = "*** Begin Patch\n*** Add File: new.js\n+x\n*** Update File: src/math.js\n-  return nothing;\n+  return 1;\n*** End Patch";
        let refused = execute(&mut context, &call("apply_patch", json!({ "patch": bad })));

        assert!(refused.is_error && refused.content.contains("does not match"), "{}", refused.content);
        assert!(!workspace.exists("new.js"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_sub_agent_cannot_change_anything() {
        let (root, workspace) = folder("readonly");
        let sink = EventSink::discarding();
        let mut context = context(&workspace, &sink, Autonomy::Auto);

        context.read_only = true;

        let outcome = execute(&mut context, &call("write_file", json!({ "path": "a.txt", "content": "x" })));

        assert!(outcome.is_error && outcome.content.contains("sub-agent"));
        assert!(!root.join("a.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remember_writes_the_project_memory() {
        let (root, workspace) = folder("remember");
        let sink = EventSink::discarding();
        let mut context = context(&workspace, &sink, Autonomy::Ask);
        let outcome = execute(&mut context, &call("remember", json!({ "fact": "Use pnpm, never npm" })));

        assert!(!outcome.is_error, "{}", outcome.content);
        assert!(std::fs::read_to_string(root.join(".sdc/memory.md")).unwrap().contains("- Use pnpm, never npm"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_diff_is_the_changed_middle_only() {
        assert_eq!(
            diff_lines("a\nb\nc\n", "a\nB\nB2\nc\n"),
            vec![(2, "b".to_string(), "rem"), (2, "B".to_string(), "add"), (3, "B2".to_string(), "add")]
        );
        assert!(diff_lines("same\n", "same\n").is_empty());
    }

    #[test]
    fn a_long_answer_keeps_its_head_and_tail() {
        let long = format!("START{}END", "x".repeat(RESULT_CAP * 2));
        let capped = cap(&long);

        assert!(capped.starts_with("START") && capped.ends_with("END"));
        assert!(capped.contains("characters cut"));
        assert!(capped.len() < long.len());
    }
}
