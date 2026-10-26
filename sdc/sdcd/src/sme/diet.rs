//! **The prompt diet (P1-5, switch `sme.prompt_diet`)**: what a small local model is sent is a budget, so the prompt is built in a
//! fixed shape and cut to fit - never grown until it overflows.
//!
//! Four rules, and each is a function here:
//!
//! * **Few tools.** A step is offered at most [`MAX_SCHEMAS`] tool schemas ([`at_most`]): a small model given twenty tools picks the
//!   wrong one, and the schemas themselves are the largest fixed cost of a small window.
//! * **Short system prompt.** [`slim`] cuts the rules at a line boundary to the class's cap (see [`sme::budget`]) and says `[shortened]`,
//!   so a 4K window keeps the rules that matter instead of the prose around them. It is deterministic: the same input is the same bytes.
//! * **The goal at both ends.** [`bookend`] puts the goal above the evidence and repeats it under it. A small model loses the middle of
//!   a long prompt; it does not lose the edges. Calling it twice changes nothing.
//! * **A stable prefix.** [`prefix`] is built from the parts that do not change inside a task (the rules, the map, the memory's header)
//!   in a fixed order, so every step of a task sends the same first bytes and the server's prompt cache can keep them. Nothing in it
//!   carries a clock, a counter or a step number - that is the point of it.

use serde_json::Value;

use super::budget::{Budget, Class, Section};

/// The most tool schemas a step is offered.
pub const MAX_SCHEMAS: usize = 6;

/// The system prompt's cap in characters, per class: S keeps the rules that fit a small window, L is barely touched.
pub fn system_cap(class: Class) -> usize {
    match class {
        Class::Small => 2_400,
        Class::Medium => 5_000,
        Class::Large => 14_000,
    }
}

/// The tools a small model needs first, most important first: look (read, search), change (edit, write), RUN (the tests), then the rest.
/// A step capped at six keeps these; the order the tools happened to be registered in says nothing about how much they matter.
pub const PRIORITY: &[&str] = &["read_file", "grep", "edit_file", "write_file", "run_command", "ranked_search", "code_graph", "glob", "list_dir", "search", "git_diff", "update_plan", "find_symbol", "symbols", "patch_symbol", "replace_symbol", "apply_patch", "ask_user", "task", "remember"];

/// The specs in order of importance: the `PRIORITY` ones first (in that order), every other one after them in the order given.
pub fn prioritise<T: Clone>(specs: &[T], name_of: impl Fn(&T) -> &str) -> Vec<T> {
    let rank = |spec: &T| PRIORITY.iter().position(|name| *name == name_of(spec)).unwrap_or(PRIORITY.len());
    let mut ordered: Vec<T> = specs.to_vec();

    ordered.sort_by_key(rank);

    ordered
}

/// At most `max` schemas, in the order given (put the best first with `prioritise`).
pub fn at_most<T: Clone>(specs: &[T], max: usize) -> Vec<T> {
    specs.iter().take(max.max(1)).cloned().collect()
}

/// The share of a local model's window the tool schemas may take before they are cut: the hard floor under every
/// switch.
pub const SCHEMA_SHARE: u64 = 40;

/// The fewest schemas a step is left with, however small the window: look, change, run.
pub const MIN_SCHEMAS: usize = 4;

/// **As many schemas as the window can hold** (3.2) - a floor that no switch can lower.
///
/// [`at_most`] is the *technique*: six schemas, because a small model given twenty picks the wrong one. This is the
/// *physics*: a window is a hard limit, and the schemas are the largest fixed part of the prompt. Measured on a local
/// model with an 8,192-token window and the diet switched off: 21 tool schemas fitted, and the 22nd did not - so every
/// call of the turn was refused as ContextOverflow before it was sent, and the turn spent its whole step limit saying
/// nothing. A prompt that cannot be sent is worse than a prompt with four tools in it.
///
/// `tokens_of` is handed the chosen prefix rather than one spec, because what matters is the total: the caller owns
/// the estimator (the daemon has exactly one), and this function owns the rule.
pub fn within<T: Clone>(specs: &[T], room: u64, tokens_of: impl Fn(&[T]) -> u64) -> Vec<T> {
    if room == 0 || specs.len() <= MIN_SCHEMAS || tokens_of(specs) <= room {
        return specs.to_vec();
    }

    /* Down from the whole list one schema at a time, so a window that holds nineteen keeps nineteen. */
    let mut kept = specs.len();

    while kept > MIN_SCHEMAS && tokens_of(&specs[..kept]) > room {
        kept -= 1;
    }

    specs[..kept].to_vec()
}

/// The system prompt cut to the class's cap, at a line boundary, marked `[shortened]`. Unchanged below the cap.
pub fn slim(system: &str, class: Class) -> String {
    let cap = system_cap(class);

    if system.len() <= cap || cap == 0 {
        return system.to_string();
    }

    let mut cut = cap;

    while cut > 0 && !system.is_char_boundary(cut) {
        cut -= 1;
    }

    let at = system[..cut].rfind('\n').unwrap_or(cut);
    let kept = system[..at].trim_end();

    format!("{kept}\n[shortened: the rest of the rules are unchanged - smallest step first, read before you write, verify before you answer]")
}

/// The goal above the evidence and again under it. Idempotent: a text that already carries both bookends is returned as it is.
pub fn bookend(text: &str, goal: &str) -> String {
    let goal = goal.trim();

    if goal.is_empty() || text.trim_start().starts_with(&format!("Goal: {goal}")) {
        return text.to_string();
    }

    format!("Goal: {goal}\n\n{}\n\nThe goal, again: {goal}", text.trim_end())
}

/// The stable prefix of a task's prompts: the parts that do not change between steps, in a fixed order and with a fixed separator.
/// Empty parts are left out entirely (two empty parts must not become two blank lines).
pub fn prefix(parts: &[(&str, &str)]) -> String {
    parts.iter().filter(|(_, body)| !body.trim().is_empty()).map(|(title, body)| format!("## {title}\n{}", body.trim_end())).collect::<Vec<_>>().join("\n\n")
}

/// A hash of the prefix, for the trace and for the test that proves it is stable: it must be the same for every step of a task.
pub fn prefix_key(parts: &[(&str, &str)]) -> String {
    crate::graph::parse::sha(&prefix(parts))
}

/// Trims the prompt's notes to the class's card section, so the evidence above the person's words never eats the window alone.
pub fn fit_notes(notes: &str, budget: &Budget) -> String {
    budget.trim(notes, Section::Card).0
}

/// Parts of a prompt that are worth keeping byte-stable, gathered in one place so the caller cannot order them differently by accident:
/// the rules first, then the project's map, then this task's memory. `None` means "this step has none of it".
pub type StableParts<'a> = [(&'a str, Option<&'a str>); 3];

pub fn stable(parts: &StableParts<'_>) -> String {
    let named: Vec<(&str, &str)> = parts.iter().filter_map(|(title, body)| body.map(|body| (*title, body))).collect();

    prefix(&named)
}

/// Appends `note` to the text of the last message, when that message's content is plain text. Used for the goal bookend at the end of a
/// conversation: the model reads the last thing it was told, so the goal goes there too. `false` when the shape was not plain text (a
/// tool-call message), and nothing is changed.
pub fn append_to_last(messages: &mut [Value], note: &str) -> bool {
    let Some(last) = messages.last_mut() else {
        return false;
    };

    let Some(content) = last.get("content").and_then(Value::as_str).map(str::to_string) else {
        return false;
    };

    last["content"] = Value::String(format!("{content}\n\n{note}"));
    true
}

#[cfg(test)]
mod tests;
