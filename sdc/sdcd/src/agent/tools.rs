//! The eight tools the agent has, and what each one does to the window.
//!
//! Eight, deliberately (docs/ROADMAP-v4.md, "risks"): every tool is one more thing a model can get
//! wrong and one more thing a person has to be able to read in a tool card. These are the four verbs of
//! a coding loop - read, write, run, observe - plus the plan:
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
const RESULT_CAP: usize = 24_000;

/// Lines of a diff drawn on an Edit card; the rest is counted, not dropped silently.
const DIFF_CARD_LINES: usize = 160;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file",
            description: "Read a text file in the project folder. Paths are relative to the folder. Large files are cut at 256 KB and the answer says so.",
            schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "File path, relative to the project folder." } },
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
}

impl Outcome {
    fn ok(content: impl Into<String>) -> Self {
        Self { content: cap(&content.into()), is_error: false }
    }

    fn error(content: impl Into<String>) -> Self {
        Self { content: cap(&content.into()), is_error: true }
    }
}

/// Runs one tool call, drawing its card as it goes.
pub fn execute(context: &mut ToolContext, call: &ToolUse) -> Outcome {
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

    match call.name.as_str() {
        "read_file" => read_file(context, &call_id, &text("path")),
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
    specs().iter().map(|spec| spec.name).collect()
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

fn write_file(context: &mut ToolContext, call_id: &str, path: &str, content: &str) -> Outcome {
    if path.trim().is_empty() {
        return Outcome::error("write_file needs a path.");
    }

    let existed = context.workspace.exists(path);
    let before = if existed { context.workspace.read(path).map(|(text, _)| text).unwrap_or_default() } else { String::new() };
    let diff = diff_lines(&before, content);
    let (added, removed) = counts(&diff);

    started(context, call_id, "edit", if existed { "Edit" } else { "Create" }, path);

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
            completed(context, call_id, true, &format!("done · +{added} −{removed}"), Some(card(&diff)));

            Outcome::ok(format!("{} {path} ({} lines).", if existed { "Replaced" } else { "Created" }, content.lines().count()))
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
            completed(context, call_id, true, &format!("done · +{added} −{removed}"), Some(card(&diff)));

            Outcome::ok(format!("Edited {path}: {} replacement(s).", if all { found } else { 1 }))
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

    let risk = if gate::looks_dangerous(line) { "DANGEROUS" } else { "MUTATING" };

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

    let total = steps.len();

    Outcome::ok(if done == total {
        format!("Plan shown: all {total} steps done.")
    } else {
        format!("Plan shown: {done} of {total} steps done. Call update_plan again as each step starts or finishes.")
    })
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
    if text.len() <= RESULT_CAP {
        return text.to_string();
    }

    let head: String = text.chars().take(RESULT_CAP * 2 / 3).collect();
    let tail: String = {
        let reversed: String = text.chars().rev().take(RESULT_CAP / 3).collect();

        reversed.chars().rev().collect()
    };

    format!("{head}\n\n[… {} characters cut …]\n\n{tail}", text.len() - head.len() - tail.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::Recorder;

    fn context<'a>(workspace: &'a Workspace, sink: &'a EventSink, autonomy: Autonomy) -> ToolContext<'a> {
        ToolContext { workspace, sink, turn_id: "turn-t", autonomy, always: HashSet::new(), calls: 0, checkpoint: None, checkpointed: false }
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
        let specs = specs();

        assert_eq!(specs.len(), 8);

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
