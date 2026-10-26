//! **N7-2 - the tool envelope lock and the repeated-call block** (switch `nr.tool_envelope`, on for local models since 3.2).
//!
//! Forcing a whole answer into JSON makes a model reason worse. So only the **envelope** of a tool call - the tool's name and the shape of its
//! arguments - is bound to a schema; the thinking before it and the code in the message stay free text. Three guards sit on it:
//!
//! * an arguments object that does not match the tool's schema is *malformed* and never reaches the tool;
//! * the **same call twice** is blocked, and the model is told the first result is already in its context;
//! * when a step needs a tool and the model answered in prose, it is asked once more with the tool forced (the envelope schema applied to that
//!   one request).
//!
//! ## Why "twice in a row" was not enough (3.2)
//!
//! Until 3.2 only a **back-to-back** repeat was blocked, which catches `read a.rs, read a.rs` and misses the shape a small model actually gets
//! stuck in: `read a.rs, grep foo, read a.rs, grep foo, …`. The second `read a.rs` is a different call from the one just before it, so it ran,
//! and its result arrived identical - a loop that costs the whole window and tells the model nothing it did not have.
//!
//! So the guard now remembers every call whose answer **cannot have changed**, for as long as that is true:
//!
//! * a **search** - `grep`, `glob`, `search`, `list_dir`, `code_graph`, `find_symbol` - may not be repeated with the same arguments at all.
//!   Its result is already above in the conversation, word for word. Reading a *file* is the one look left out entirely, because
//!   `dedupe_reads` answers a re-read better than a block could (see [`deduped_elsewhere`]);
//! * anything that **changes the project or the world** (`edit_file`, `write_file`, `apply_patch`, `run_command`, a browser action, …) clears
//!   that memory, because after a change a re-read is not a repeat - it is the right thing to do. `read a.rs, edit a.rs, read a.rs` passes,
//!   which is the case a blanket "never read the same file twice" would have broken;
//! * a non-pure call is still blocked back-to-back, as before: running the same test command twice with nothing in between is a stutter,
//!   running it again after an edit is work.

use serde_json::Value;

use crate::agent::dialect::ToolSpec;
use crate::agent::rescue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Let the call through.
    Run,
    /// The same call as the one just made.
    Repeat(String),
    /// Arguments that do not fit the tool's schema.
    Malformed(String),
}

/// What the guard remembers: the call just made, and every look-only call since the last change.
///
/// It also carries **whose turn it is**. `nr.tool_envelope` is a `LocalOnly` switch, and until 3.2 this file asked
/// `v3::on` rather than `v3::on_in`, which ignores that scope - so once the switch was on by default it began judging
/// a *cloud* model's calls too. The first thing that showed was a plan-mode test: the cloud model's deliberately
/// wrong `edit_file` arguments were refused by this schema check before plan mode could refuse them as a write, and
/// the turn's own reason for saying no was replaced by a different one.
#[derive(Debug, Clone)]
pub struct Guard {
    last: Option<(String, String)>,
    /// Pure calls whose answer is already in the conversation. Cleared by anything that changes the project.
    looked: Vec<(String, String)>,
    reasked: bool,
    /// The model this turn runs on, so the switch's `LocalOnly` scope is honoured.
    model: crate::v3::ModelCtx,
}

impl Default for Guard {
    /// A guard for a **local** model - the only kind the switch applies to, and what a test means by `default()`.
    fn default() -> Self {
        Self::for_model(crate::v3::ModelCtx::new(true))
    }
}

/// Tools that only **look**: called twice with the same arguments they answer the same thing, so the second call is
/// waste however many steps lie between them.
///
/// Deliberately a list of names rather than a flag on the tool: a tool that is wrongly called pure would block work
/// the model needs to do, so each name is here because its answer cannot change while the project does not. Anything
/// not named here - every edit, every command, every browser action, `web_fetch` and `web_search` (the web changes),
/// `ask_user`, `remember`, `done` - is not pure and is only guarded against a back-to-back stutter.
pub fn pure(tool: &str) -> bool {
    matches!(
        tool,
        "read_file"
            | "read"
            | "list_dir"
            | "glob"
            | "grep"
            | "search"
            | "ranked_search"
            | "code_graph"
            | "find_symbol"
            | "symbols"
            | "callers"
            | "where"
            | "spec_read"
            | "obs_read"
            | "view_image"
            | "tool_search"
    )
}

/// Is a repeat of this call somebody else's job? **Reading a file is**, and that exception is deliberate.
///
/// The agent loop already handles a repeated read better than a block could. `dedupe_reads` hashes what it gave the
/// model and answers a re-read with a twenty-token pointer saying the file is unchanged and its content is above,
/// and with the **whole file** when it is not. That is strictly better than refusing the call: it costs the same few
/// tokens, it says the same thing, and it still works when the file changed by some means SDC never saw (a build, a
/// `git pull`, another agent in the same folder). Blocking the read as well would only replace a useful answer with
/// an error.
///
/// So the repeat block owns what has no such mechanism, and that is what a stuck model really repeats: the searches.
/// `grep`, `glob`, `search`, `list_dir`, `code_graph`, `find_symbol` - the same query gives the same answer, and
/// `grep foo / read a.rs / grep foo / read a.rs` is the loop that eats a small window.
fn deduped_elsewhere(tool: &str) -> bool {
    matches!(tool, "read_file" | "read")
}

/// Is this call one the guard remembers for the whole task, rather than only back-to-back?
fn tracked(tool: &str) -> bool {
    pure(tool) && !deduped_elsewhere(tool)
}

/// Does this call change the project or the world, so that an earlier look may now answer differently?
///
/// The opposite of [`pure`] with the cases that are *neither* left out.
///
/// Asking the person something, writing a note, reading a web page or reading a running process's output changes
/// neither the files nor the world, so none of them makes an earlier read worth repeating. They are not pure either:
/// `web_fetch` can answer differently for the same address, and `process_output` is expected to.
fn changes_things(tool: &str) -> bool {
    !pure(tool)
        && !matches!(
            tool,
            "ask_user" | "remember" | "done" | "in_progress" | "update_plan" | "web_search" | "web_fetch" | "process_output" | "screenshot"
        )
}

/// A canonical form of a JSON value (object keys sorted), so two spellings of the same arguments compare equal.
pub fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();

            keys.sort();

            format!("{{{}}}", keys.iter().map(|key| format!("{}:{}", Value::String((*key).clone()), canonical(&map[*key]))).collect::<Vec<_>>().join(","))
        }
        Value::Array(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

impl Guard {
    /// A guard for one turn's model.
    pub fn for_model(model: crate::v3::ModelCtx) -> Self {
        Self { last: None, looked: Vec::new(), reasked: false, model }
    }

    /// Is the envelope lock on for this turn's model? `LocalOnly`, so never for a model the person picked by hand.
    fn on(&self) -> bool {
        crate::v3::on_in("nr.tool_envelope", &self.model)
    }

    /// Judges one call. A call that passes becomes "the last call".
    pub fn judge(&mut self, specs: &[ToolSpec], name: &str, arguments: &Value) -> Verdict {
        if !self.on() {
            return Verdict::Run;
        }

        /* a tool the turn's own list does not hold (an MCP tool, `tool_search`) is judged by its executor; only a repeat of it is blocked here */
        if let Some(spec) = specs.iter().find(|spec| spec.name == name) {
            if let Err(problem) = rescue::validate(spec, arguments) {
                return Verdict::Malformed(problem);
            }
        }

        let key = (name.to_string(), canonical(arguments));
        let said_again = |what: &str| {
            Verdict::Repeat(format!(
                "You already made exactly this `{name}` call {what} and its result is above in this conversation, word for word; read it there instead of asking again, and then do the next thing the task needs."
            ))
        };

        if self.last.as_ref() == Some(&key) && !deduped_elsewhere(name) {
            return said_again("just now");
        }

        /* A look whose answer cannot have changed since it was answered. */
        if tracked(name) && self.looked.contains(&key) {
            return said_again("earlier in this task");
        }

        if changes_things(name) {
            /* The project moved: every earlier look may answer differently now, so looking again is work. */
            self.looked.clear();
        } else if tracked(name) {
            self.looked.push(key.clone());
        }

        self.last = Some(key);
        Verdict::Run
    }

    /// Forgets the last call, so a back-to-back repeat after it is allowed. The looks are kept: they are still above in
    /// the conversation, and nothing has changed to make them worth asking again.
    pub fn forget(&mut self) {
        self.last = None;
    }

    /// The step required a tool and the model answered in prose: ask once more with the tool forced. `false` for the second time on - the answer
    /// is accepted rather than looping.
    pub fn reask_with_tool(&mut self, tool_required: bool, reply_has_tool_call: bool) -> bool {
        if !self.on() || !tool_required || reply_has_tool_call || self.reasked {
            return false;
        }

        self.reasked = true;
        true
    }

    pub fn new_step(&mut self) {
        self.reasked = false;
    }
}

/// The schema applied to the one forced request: the envelope of every tool, nothing about the text around it.
pub fn forced_envelope(specs: &[ToolSpec]) -> Value {
    rescue::call_schema(specs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn specs() -> Vec<ToolSpec> {
        vec![
            ToolSpec { name: "read_file", description: "read", schema: json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"], "additionalProperties": false }) },
            ToolSpec { name: "grep", description: "search", schema: json!({ "type": "object", "properties": { "pattern": { "type": "string" }, "limit": { "type": "integer" } }, "required": ["pattern"] }) },
        ]
    }

    #[test]
    fn a_call_repeated_back_to_back_is_blocked_with_the_reason() {
        crate::v3::forcing(&["nr.tool_envelope"], || {
            let mut guard = Guard::default();
            let specs = specs();

            assert_eq!(guard.judge(&specs, "grep", &json!({ "pattern": "foo" })), Verdict::Run);
            assert!(matches!(guard.judge(&specs, "grep", &json!({ "pattern": "foo" })), Verdict::Repeat(why) if why.contains("above in this conversation")));

            /* 3.2: and still blocked with another call in between - `grep foo / read a.rs / grep foo` is the loop a
               small model really gets stuck in, and "back to back" let it through. */
            assert_eq!(guard.judge(&specs, "read_file", &json!({ "path": "c.rs" })), Verdict::Run);
            assert!(matches!(guard.judge(&specs, "grep", &json!({ "pattern": "foo" })), Verdict::Repeat(why) if why.contains("earlier in this task")));

            /* A repeated *read* is never blocked here at all: `dedupe_reads` answers it with a pointer, and notices
               when the file changed - see `deduped_elsewhere`. */
            assert_eq!(guard.judge(&specs, "read_file", &json!({ "path": "a.rs" })), Verdict::Run);
            assert_eq!(guard.judge(&specs, "read_file", &json!({ "path": "a.rs" })), Verdict::Run);
        });
    }

    /// The case a blanket "never read the same file twice" would break: after a change, reading again is the right move.
    #[test]
    fn a_read_after_a_change_is_work_not_a_repeat() {
        crate::v3::forcing(&["nr.tool_envelope"], || {
            let mut guard = Guard::default();
            let specs = vec![
                ToolSpec { name: "read_file", description: "read", schema: json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] }) },
                ToolSpec { name: "edit_file", description: "edit", schema: json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] }) },
                ToolSpec { name: "run_command", description: "run", schema: json!({ "type": "object", "properties": { "command": { "type": "string" } }, "required": ["command"] }) },
                ToolSpec { name: "remember", description: "note", schema: json!({ "type": "object", "properties": { "fact": { "type": "string" } }, "required": ["fact"] }) },
                ToolSpec { name: "grep", description: "search", schema: json!({ "type": "object", "properties": { "pattern": { "type": "string" } }, "required": ["pattern"] }) },
            ];

            assert_eq!(guard.judge(&specs, "grep", &json!({ "pattern": "total" })), Verdict::Run);
            assert_eq!(guard.judge(&specs, "edit_file", &json!({ "path": "a.rs" })), Verdict::Run);
            assert_eq!(guard.judge(&specs, "grep", &json!({ "pattern": "total" })), Verdict::Run, "the project changed");

            /* A command is not pure, so running it twice in a row is a stutter - but once more after an edit is work. */
            assert_eq!(guard.judge(&specs, "run_command", &json!({ "command": "cargo test" })), Verdict::Run);
            assert!(matches!(guard.judge(&specs, "run_command", &json!({ "command": "cargo test" })), Verdict::Repeat(_)));
            assert_eq!(guard.judge(&specs, "edit_file", &json!({ "path": "b.rs" })), Verdict::Run);
            assert_eq!(guard.judge(&specs, "run_command", &json!({ "command": "cargo test" })), Verdict::Run, "after an edit the checks run again");

            /* Writing a note changes neither the files nor the world, so it does not make an earlier search repeatable. */
            assert_eq!(guard.judge(&specs, "grep", &json!({ "pattern": "cart" })), Verdict::Run);
            assert_eq!(guard.judge(&specs, "remember", &json!({ "fact": "c.rs holds the parser" })), Verdict::Run);
            assert!(matches!(guard.judge(&specs, "grep", &json!({ "pattern": "cart" })), Verdict::Repeat(_)));
        });
    }

    /// Which tools only look. A tool wrongly called pure would block work, so the list is asserted rather than trusted.
    #[test]
    fn only_the_tools_that_look_are_pure() {
        for looks in ["read_file", "list_dir", "grep", "glob", "search", "ranked_search", "code_graph", "find_symbol", "symbols", "obs_read"] {
            assert!(pure(looks), "{looks} only looks");
            assert!(!changes_things(looks), "{looks} changes nothing");
        }

        /* Every look is remembered for the task except reading a file, which `dedupe_reads` answers better. */
        for searched in ["list_dir", "grep", "glob", "search", "ranked_search", "code_graph", "find_symbol", "symbols", "obs_read"] {
            assert!(tracked(searched), "{searched} is remembered for the task");
        }

        assert!(!tracked("read_file") && !tracked("read"), "a re-read belongs to `dedupe_reads`");
        assert!(deduped_elsewhere("read_file") && deduped_elsewhere("read"));
        assert!(!deduped_elsewhere("grep") && !deduped_elsewhere("run_command"));

        for acts in ["edit_file", "write_file", "apply_patch", "patch_symbol", "replace_symbol", "run_command", "start_process", "click", "type", "browser"] {
            assert!(!pure(acts), "{acts} does something");
            assert!(changes_things(acts), "{acts} makes an earlier look worth repeating");
        }

        /* Neither: they are not repeat-free looks, and they do not change what a look would answer. The web is not the
           project (the same address can answer differently), and a running process's output is expected to grow. */
        for neither in ["ask_user", "remember", "done", "web_search", "web_fetch", "process_output", "screenshot"] {
            assert!(!pure(neither) && !changes_things(neither), "{neither}");
        }
    }

    #[test]
    fn two_spellings_of_the_same_arguments_are_the_same_call() {
        assert_eq!(canonical(&json!({ "b": 1, "a": [1, { "y": 2, "x": 1 }] })), canonical(&json!({ "a": [1, { "x": 1, "y": 2 }], "b": 1 })));

        crate::v3::forcing(&["nr.tool_envelope"], || {
            let mut guard = Guard::default();

            assert_eq!(guard.judge(&specs(), "grep", &json!({ "pattern": "x", "limit": 5 })), Verdict::Run);
            assert!(matches!(guard.judge(&specs(), "grep", &json!({ "limit": 5, "pattern": "x" })), Verdict::Repeat(_)));
        });
    }

    #[test]
    fn malformed_arguments_never_reach_a_tool() {
        crate::v3::forcing(&["nr.tool_envelope"], || {
            let mut guard = Guard::default();

            for (name, arguments) in [("read_file", json!({})), ("read_file", json!({ "path": 5 })), ("read_file", json!({ "path": "a", "extra": 1 })), ("grep", json!("not an object"))] {
                assert!(matches!(guard.judge(&specs(), name, &arguments), Verdict::Malformed(_)), "{name} {arguments}");
            }

            assert_eq!(guard.judge(&specs(), "grep", &json!({ "pattern": "x" })), Verdict::Run);
        });
    }

    #[test]
    fn prose_where_a_tool_was_required_is_re_asked_once_with_the_tool_forced() {
        crate::v3::forcing(&["nr.tool_envelope"], || {
            let mut guard = Guard::default();

            assert!(!guard.reask_with_tool(false, false), "no tool was required");
            assert!(!guard.reask_with_tool(true, true), "it did call one");
            assert!(guard.reask_with_tool(true, false));
            assert!(!guard.reask_with_tool(true, false), "only once, then the answer stands");
            guard.new_step();
            assert!(guard.reask_with_tool(true, false));
        });

        let schema = forced_envelope(&specs());

        assert_eq!(schema["anyOf"].as_array().unwrap().len(), 2);
        assert_eq!(schema["anyOf"][0]["properties"]["name"]["enum"][0], "read_file");
        assert!(schema["anyOf"][0]["required"].as_array().unwrap().iter().any(|r| r == "arguments"), "only the envelope is bound");
    }

    /// 3.2: the switch is `LocalOnly`, and this file must honour that - a cloud model's calls are not judged here.
    #[test]
    fn a_cloud_models_calls_are_never_judged() {
        crate::v3::forcing(&["nr.tool_envelope"], || {
            let mut cloud = Guard::for_model(crate::v3::ModelCtx::cloud());

            assert_eq!(cloud.judge(&specs(), "read_file", &json!({})), Verdict::Run, "a malformed call is the turn's own to refuse");
            assert_eq!(cloud.judge(&specs(), "grep", &json!({ "pattern": "x" })), Verdict::Run);
            assert_eq!(cloud.judge(&specs(), "grep", &json!({ "pattern": "x" })), Verdict::Run, "and a repeat is not blocked");
            assert!(!cloud.reask_with_tool(true, false));

            let mut local = Guard::default();

            assert!(matches!(local.judge(&specs(), "read_file", &json!({})), Verdict::Malformed(_)), "a local model is judged");
        });
    }

    #[test]
    fn with_the_switch_off_everything_runs_as_before() {
        crate::v3::ablating(&["nr.tool_envelope"], || {
            let mut guard = Guard::default();

            assert_eq!(guard.judge(&specs(), "read_file", &json!({})), Verdict::Run);
            assert_eq!(guard.judge(&specs(), "read_file", &json!({})), Verdict::Run);
            assert!(!guard.reask_with_tool(true, false));
        });
    }
}
