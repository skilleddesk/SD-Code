//! **Per-step tool subset (P2-3, switch `padma.tool_subset`, off until the bench says)**: a small model that is offered twenty tools picks the wrong
//! one, and every schema costs window. A recipe step names the 2-4 tools it needs; only those schemas go into that step's prompt. A tool the step did not
//! name is still found if the model calls it (the executor looks tools up by name, not through this list), so a subset can make a prompt smaller but
//! never turn a call into "tool not found". A step that names no known tool is offered everything.

use std::cell::RefCell;

use crate::agent::dialect::ToolSpec;

thread_local! {
    static ALLOWED: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Runs `step` with only `tools` offered to the model (a recipe step's declared tools).
pub fn with_tools<R>(tools: &[&str], step: impl FnOnce() -> R) -> R {
    struct Restore(Option<Vec<String>>);

    impl Drop for Restore {
        fn drop(&mut self) {
            let before = self.0.take();

            ALLOWED.with(|allowed| *allowed.borrow_mut() = before);
        }
    }

    let before = ALLOWED.with(|allowed| allowed.borrow_mut().replace(tools.iter().map(|tool| tool.to_string()).collect()));
    let _restore = Restore(before);

    step()
}

/// The specs a step is offered, or `None` when it is offered all of them (no subset declared, or none of the declared tools exists).
pub fn subset(specs: &[ToolSpec]) -> Option<Vec<ToolSpec>> {
    let chosen: Vec<ToolSpec> = ALLOWED.with(|allowed| allowed.borrow().as_ref().map(|names| specs.iter().filter(|spec| names.iter().any(|name| name == spec.name)).cloned().collect()))?;

    (!chosen.is_empty()).then_some(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(name: &'static str) -> ToolSpec {
        ToolSpec { name, description: "x", schema: json!({ "type": "object" }) }
    }

    #[test]
    fn a_declared_subset_keeps_only_those_tools_and_is_gone_after_the_step() {
        let all = [spec("read_file"), spec("edit_file"), spec("run_command"), spec("search")];

        assert!(subset(&all).is_none(), "no subset declared: everything is offered");

        with_tools(&["read_file", "search", "not_a_tool"], || {
            let names: Vec<&str> = subset(&all).unwrap().iter().map(|spec| spec.name).collect();

            assert_eq!(names, vec!["read_file", "search"]);
        });

        assert!(subset(&all).is_none());
    }

    #[test]
    fn a_step_that_names_no_known_tool_is_offered_all_of_them() {
        with_tools(&["nothing_here"], || assert!(subset(&[spec("read_file")]).is_none()));
    }

    #[test]
    fn steps_nest_and_the_outer_one_comes_back() {
        let all = [spec("a"), spec("b"), spec("c")];

        with_tools(&["a", "b"], || {
            with_tools(&["c"], || assert_eq!(subset(&all).unwrap().len(), 1));

            assert_eq!(subset(&all).unwrap().len(), 2);
        });
    }
}
