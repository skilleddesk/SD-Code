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
}

/// The tools a sub-agent (`task`) may use: they read, search and look - they never change anything.
pub const READ_ONLY: &[&str] = &["read_file", "list_dir", "search", "grep", "glob", "git_diff", "web_fetch", "web_search", "view_image", "process_output"];

/// Every tool, for a turn whose model cannot see and which is not a sub-agent.
pub fn specs() -> Vec<ToolSpec> {
    specs_for(Caps::default())
}

/// The tools for a turn with `caps`.
pub fn specs_for(caps: Caps) -> Vec<ToolSpec> {
    let mut all = base_specs();

    all.extend(more_specs());

    if caps.vision {
        all.extend(vision_specs());
    }

    if caps.subagent {
        all.retain(|spec| READ_ONLY.contains(&spec.name));
    } else {
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
        description: "Use a real web browser (headless, on the person's machine) to try a page the way a user would: open it, read it, click, type into fields, press keys, and - for a model that can see - take a screenshot. read answers the page's text and a numbered list of what can be clicked or filled; click and type take that number, a CSS selector, or the words on the element. The browser stays open between calls in this chat. Use it to test the site or app you built (a dev server started with start_process, a file:/// page, or a public URL).",
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
        description: "Edit files with one patch in the Codex format - add, update, move and delete several files in one call. The patch starts with '*** Begin Patch' and ends with '*** End Patch'. Each file is '*** Add File: path' (every line prefixed with +), '*** Delete File: path', or '*** Update File: path' (optionally '*** Move to: newpath') followed by hunks: an '@@' line, then context lines (prefixed with a space), removed lines (-) and added lines (+). Context and removed lines must match the file exactly.",
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
        description: "Hand a self-contained research or exploration job to a sub-agent with its own fresh context and read-only tools (read, list, grep, glob, web). It answers with a report. Use it to explore a large codebase or look something up without filling your own context; several task calls in one reply run in parallel. Give it everything it needs to know in prompt - it has not seen this conversation.",
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
            description: "Render a web page in a headless browser on the person's machine and look at it - to check the page you built looks right. Works with http://localhost addresses of a dev server you started with start_process, public URLs, and file:/// paths.",
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
            description: "Search file contents with a regular expression (Rust/PCRE-like syntax). Answers path:line: text, up to 200 hits. Skips node_modules, .git, build output and secrets. Prefer it over reading files one by one to find where something is defined or used.",
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
            description: "Start a command that keeps running - a dev server, a watcher, a worker - in the background, in the project folder. Answers its id and its first output (for a server, usually the address). It keeps running after your answer; read it with process_output and end it with stop_process.",
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
            description: "Ask the person a question and wait for the answer - when a decision is theirs to make (which design, which data to keep, which of two readings they meant) and guessing wrong would waste the work. Offer 2-4 short options when you can; they can always write their own answer. Ask in the person's language. Do not use it for permission to run a tool: the tools ask for that themselves.",
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
            description: "Keep a fact for every later chat: a preference the person stated, a convention of the project, a decision. project scope writes .sdc/memory.md; global scope is SDC's memory for all projects. Only lasting facts - not the progress of this task.",
            schema: json!({
                "type": "object",
                "properties": {
                    "fact": { "type": "string", "description": "One short, self-contained sentence." },
                    "scope": { "type": "string", "enum": ["project", "global"], "description": "project (default) or global." },
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
    let outcome = execute_call(context, call);

    /* A local model's window holds less than one full answer (0.16.1): the cap follows the window. */
    if context.result_cap < RESULT_CAP && outcome.content.len() > context.result_cap {
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

    let number = |key: &str| call.input[key].as_u64();

    match call.name.as_str() {
        "read_file" if number("offset").is_some() || number("limit").is_some() => {
            read_range(context, &call_id, &text("path"), number("offset").unwrap_or(1) as usize, number("limit").map(|limit| limit as usize))
        }
        "read_file" => read_file(context, &call_id, &text("path")),
        "grep" => grep(context, &call_id, &text("pattern"), &text("path"), call.input["glob"].as_str(), call.input["ignore_case"].as_bool().unwrap_or(false)),
        "glob" => glob(context, &call_id, &text("pattern"), &text("path")),
        "web_fetch" => web_fetch(context, &call_id, &text("url")),
        "web_search" => web_search(context, &call_id, &text("query")),
        "start_process" => start_process(context, &call_id, &text("command"), number("wait_seconds").unwrap_or(6).min(60)),
        "process_output" => process_output(context, &call_id, &text("process_id"), number("lines").unwrap_or(60) as usize),
        "stop_process" => stop_process(context, &call_id, &text("process_id")),
        "ask_user" => ask_user(context, &call_id, &text("question"), &call.input["options"]),
        "remember" => remember(context, &call_id, &text("fact"), call.input["scope"].as_str().unwrap_or("project")),
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
    specs_for(Caps { vision: true, subagent: false, patch: true }).iter().map(|spec| spec.name).collect()
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
            let lines = text.lines().count();

            completed(context, call_id, true, &format!("done · {lines} ln{}", if truncated { " · cut" } else { "" }), None);

            let mut content = numbered(&text);

            if truncated {
                content.push_str("\n[The file is longer than 256 KB; only its beginning is shown.]");
            }

            Outcome::ok(content)
        }
        Err(error) => {
            completed(context, call_id, false, "failed", None);

            Outcome::error(error.message)
        }
    }
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

            Outcome::error(error.message)
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

fn write_file(context: &mut ToolContext, call_id: &str, path: &str, content: &str) -> Outcome {
    if path.trim().is_empty() {
        return Outcome::error("write_file needs a path.");
    }

    let existed = context.workspace.exists(path);
    let before = if existed { context.workspace.read(path).map(|(text, _)| text).unwrap_or_default() } else { String::new() };
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

    match context.workspace.write(path, content) {
        Ok(()) => {
            let hooks = after_edit(context, call_id, path);

            completed(context, call_id, true, &format!("done · +{added} −{removed}"), Some(card(&diff)));

            let said = format!("{} {path} ({} lines).", if existed { "Replaced" } else { "Created" }, content.lines().count());

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

fn edit_file(context: &mut ToolContext, call_id: &str, path: &str, old: &str, new: &str, all: bool) -> Outcome {
    if old.is_empty() {
        return Outcome::error("edit_file needs old_text. To create a file or replace all of it, use write_file.");
    }

    let before = match context.workspace.read(path) {
        Ok((_, true)) => return Outcome::error(format!("{path} is too large to edit in place; it is longer than 256 KB.")),
        Ok((text, false)) => text,
        Err(error) => return Outcome::error(error.message),
    };
    let found = before.matches(old).count();

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

    match context.workspace.write(path, &after) {
        Ok(()) => {
            let hooks = after_edit(context, call_id, path);

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

    Outcome::ok(if done == total {
        format!("Plan shown: all {total} steps done.")
    } else {
        format!("Plan shown: {done} of {total} steps done. Call update_plan again as each step starts or finishes.")
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

            Outcome::ok(if end < total { format!("{body}\n[lines {offset}-{end} of {total}]") } else { body })
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

            let dated = page.date.as_deref().map(|date| format!(" · {date}")).unwrap_or_default();

            match &context.research {
                Some(session) => {
                    let n = session.cite(&page.url, &page.title, page.date.clone(), true);

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

fn remember(context: &mut ToolContext, call_id: &str, fact: &str, scope: &str) -> Outcome {
    if fact.trim().is_empty() {
        return Outcome::error("remember needs a fact.");
    }

    let global = scope == "global";

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

        let written = match after {
            Some(text) => context.workspace.write(path, text),
            None => context.workspace.remove(path),
        };

        match written {
            Ok(()) => {
                completed(context, &card_id, true, &format!("done · +{added} −{removed}"), Some(card(&diff)));
                report.push(format!("{verb} {path} (+{added} −{removed})"));
            }
            Err(error) => {
                completed(context, &card_id, false, "failed", None);

                return Outcome::error(format!("{path}: {}. Done before it: {}", error.message, if report.is_empty() { "nothing".to_string() } else { report.join("; ") }));
            }
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

        assert!(read.content.len() < 3_200 && read.content.contains("characters cut"), "{}", read.content.len());

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
        let specs = specs_for(Caps { vision: true, subagent: false, patch: true });

        assert_eq!(specs.len(), 22);
        assert_eq!(specs_for(Caps::default()).len(), 19, "no image tools, no patch");
        assert_eq!(specs_for(Caps { vision: false, subagent: true, patch: true }).len(), 9, "a sub-agent only reads");
        assert!(specs_for(Caps { vision: false, subagent: true, patch: false }).iter().all(|spec| READ_ONLY.contains(&spec.name)));

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

        assert_eq!(outcome.content, "   10\tline 10\n   11\tline 11\n   12\tline 12\n[lines 10-12 of 50]");

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
