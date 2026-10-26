//! **N7-2 - the tool envelope lock and the repeated-call block** (switch `nr.tool_envelope`, off).
//!
//! Forcing a whole answer into JSON makes a model reason worse. So only the **envelope** of a tool call - the tool's name and the shape of its
//! arguments - is bound to a schema; the thinking before it and the code in the message stay free text. Three guards sit on it:
//!
//! * an arguments object that does not match the tool's schema is *malformed* and never reaches the tool;
//! * the **same call twice in a row** is blocked, and the model is told the first result is already in its context;
//! * when a step needs a tool and the model answered in prose, it is asked once more with the tool forced (the envelope schema applied to that
//!   one request).

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

/// What the guard remembers: the call just made.
#[derive(Debug, Default, Clone)]
pub struct Guard {
    last: Option<(String, String)>,
    reasked: bool,
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
    /// Judges one call. A call that passes becomes "the last call".
    pub fn judge(&mut self, specs: &[ToolSpec], name: &str, arguments: &Value) -> Verdict {
        if !crate::v3::on("nr.tool_envelope") {
            return Verdict::Run;
        }

        /* a tool the turn's own list does not hold (an MCP tool, `tool_search`) is judged by its executor; only a repeat of it is blocked here */
        if let Some(spec) = specs.iter().find(|spec| spec.name == name) {
            if let Err(problem) = rescue::validate(spec, arguments) {
                return Verdict::Malformed(problem);
            }
        }

        let key = (name.to_string(), canonical(arguments));

        if self.last.as_ref() == Some(&key) {
            return Verdict::Repeat(format!("You just made exactly this `{name}` call and its result is already above in this conversation; read it instead of asking again, or do something different."));
        }

        self.last = Some(key);
        Verdict::Run
    }

    /// A different call (or the end of the step) forgets the last one: only back-to-back repeats are blocked.
    pub fn forget(&mut self) {
        self.last = None;
    }

    /// The step required a tool and the model answered in prose: ask once more with the tool forced. `false` for the second time on - the answer
    /// is accepted rather than looping.
    pub fn reask_with_tool(&mut self, tool_required: bool, reply_has_tool_call: bool) -> bool {
        if !crate::v3::on("nr.tool_envelope") || !tool_required || reply_has_tool_call || self.reasked {
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

            assert_eq!(guard.judge(&specs, "read_file", &json!({ "path": "a.rs" })), Verdict::Run);
            assert!(matches!(guard.judge(&specs, "read_file", &json!({ "path": "a.rs" })), Verdict::Repeat(why) if why.contains("already above")));
            assert_eq!(guard.judge(&specs, "read_file", &json!({ "path": "b.rs" })), Verdict::Run);
            assert_eq!(guard.judge(&specs, "read_file", &json!({ "path": "a.rs" })), Verdict::Run, "not back to back any more");
        });
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
