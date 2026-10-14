//! **Intent tool sets** (v3 W5, switch `v3.tool_pruning`, a token technique - off until it passes Q3): a request
//! that does not need the browser, the web or long-running processes is not sent their schemas on every step.
//!
//! The core set is always there (read, find, edit, run, plan, ask, sub-agents). The rest join by what the request
//! says. A model that calls a tool it was not offered still gets it: the call runs, and from then on the tool is
//! in the turn's list ("add missing tool on request").

use crate::agent::dialect::ToolSpec;

/// Always offered.
pub const CORE: &[&str] = &["read_file", "write_file", "edit_file", "apply_patch", "list_dir", "grep", "glob", "search", "run_command", "update_plan", "ask_user", "task", "git_diff"];

/// Extra tools, and the words that call for them (lowercase; a word matches anywhere in the request).
pub const GROUPS: &[(&[&str], &[&str])] = &[
    (
        &["web_search", "web_fetch"],
        &["http", "www", "web", "internet", "online", "search", "google", "latest", "news", "price", "docs", "documentation", "version", "release", "changelog", "api reference", "look up", "find out", "research"],
    ),
    (
        &["browser", "screenshot", "view_image", "start_process", "process_output", "stop_process"],
        &["browser", "page", "ui", "ux", "click", "screenshot", "screen", "layout", "css", "style", "design", "frontend", "front-end", "localhost", "button", "form", "mobile", "responsive", "image", "picture", "photo", "dev server", "preview"],
    ),
    (&["start_process", "process_output", "stop_process"], &["server", "serve", "watch", "dev ", "npm run", "start", "daemon", "background", "port", "docker", "compose", "deploy", "log"]),
    (&["remember"], &["remember", "always", "from now on", "prefer", "convention", "never do", "every time"]),
];

/// The tools a request needs, from the full list the turn would have offered.
pub fn select(all: &[ToolSpec], request: &str) -> Vec<ToolSpec> {
    let lower = request.to_lowercase();
    let mut wanted: Vec<&str> = CORE.to_vec();

    for (tools, words) in GROUPS {
        if words.iter().any(|word| lower.contains(word)) {
            wanted.extend(tools.iter());
        }
    }

    all.iter().filter(|spec| wanted.contains(&spec.name) || spec.name.starts_with("mcp__")).cloned().collect()
}

/// A call to a tool the turn was not offered: adds it to the list (answers whether it did).
pub fn add_missing(offered: &mut Vec<ToolSpec>, all: &[ToolSpec], name: &str) -> bool {
    if offered.iter().any(|spec| spec.name == name) {
        return false;
    }

    match all.iter().find(|spec| spec.name == name) {
        Some(spec) => {
            offered.push(spec.clone());
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<ToolSpec> {
        crate::agent::tools::specs_for(crate::agent::tools::Caps { vision: true, subagent: false, patch: false })
    }

    #[test]
    fn a_plain_code_change_gets_the_core_tools_only() {
        let all = all();
        let names: Vec<&str> = select(&all, "rename compute_total to sum_items in src/report.rs").iter().map(|spec| spec.name).collect();

        assert!(names.contains(&"edit_file") && names.contains(&"grep") && names.contains(&"run_command"));
        assert!(!names.contains(&"browser") && !names.contains(&"web_fetch") && !names.contains(&"remember"));
        assert!(names.len() < all.len());
    }

    #[test]
    fn the_request_brings_in_what_it_needs_and_a_call_adds_the_rest() {
        let all = all();
        let ui: Vec<&str> = select(&all, "the checkout page layout breaks on mobile").iter().map(|spec| spec.name).collect();

        assert!(ui.contains(&"browser") && ui.contains(&"start_process"));
        assert!(select(&all, "what changed in the latest React release?").iter().any(|spec| spec.name == "web_search"));

        let mut offered = select(&all, "fix the typo");

        assert!(add_missing(&mut offered, &all, "web_fetch"));
        assert!(!add_missing(&mut offered, &all, "web_fetch"), "once");
        assert!(!add_missing(&mut offered, &all, "no_such_tool"));
    }
}
