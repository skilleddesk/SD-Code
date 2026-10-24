//! **The tool catalogue and `tool_search` (M2-1, switch `tools.catalog`, local models only, off)**: a small model is offered a few tools in full and
//! can still find the rest.
//!
//! The prompt diet gives a local model at most six tool schemas (`sme::diet`) because the schemas are the largest fixed cost of a small
//! window - but a model that is never told the other tools exist can never ask for them (I-8). With the catalogue on it gets:
//!
//! * the six core schemas, as before;
//! * **one line per remaining tool** - its name and what it is for - in the system prompt, in a fixed order, so the prefix is the same bytes on every
//!   step and the server's prompt cache keeps it;
//! * `tool_search(query)`: it answers with the full schema of the tools that match, and from the next step those tools are offered.
//!
//! A cloud model sees none of this (the switch is local-only and unmeasured).

use serde_json::json;

use crate::agent::dialect::ToolSpec;

/// The longest a catalogue line is.
const LINE_CHARS: usize = 96;

/// The first sentence of a tool's description, cut to a line.
pub fn one_line(spec: &ToolSpec) -> String {
    let description = spec.description.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = description.split(". ").next().unwrap_or(&description).trim_end_matches('.');
    let line: String = first.chars().take(LINE_CHARS).collect();

    format!("- {}: {}", spec.name, line)
}

/// The tools that exist but are not offered, in the order the prompt diet ranks tools (then by name), one line each. Empty when everything is offered.
pub fn block(offered: &[ToolSpec], all: &[ToolSpec]) -> String {
    let mut rest: Vec<&ToolSpec> = all.iter().filter(|spec| spec.name != "tool_search" && !offered.iter().any(|known| known.name == spec.name)).collect();

    rest.sort_by(|left, right| {
        let rank = |spec: &ToolSpec| crate::sme::diet::PRIORITY.iter().position(|name| *name == spec.name).unwrap_or(crate::sme::diet::PRIORITY.len());

        (rank(left), left.name).cmp(&(rank(right), right.name))
    });

    if rest.is_empty() {
        return String::new();
    }

    let lines: Vec<String> = rest.iter().map(|spec| one_line(spec)).collect();

    format!("\n\nOther tools exist but are not loaded. To use one, call tool_search with a few words about what you need - it hands you the tool for the next step:\n{}", lines.join("\n"))
}

/// The tools whose name or description share words with the query, best first (a name match counts most). At most `limit`.
pub fn search<'a>(all: &'a [ToolSpec], query: &str, limit: usize) -> Vec<&'a ToolSpec> {
    let words: Vec<String> = query.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '_').filter(|word| word.len() >= 3).map(str::to_string).collect();

    if words.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(usize, &ToolSpec)> = all
        .iter()
        .filter(|spec| spec.name != "tool_search")
        .map(|spec| {
            let name = spec.name.to_lowercase();
            let description = spec.description.to_lowercase();
            let score: usize = words.iter().map(|word| if name == *word { 6 } else if name.contains(word.as_str()) { 4 } else if description.contains(word.as_str()) { 1 } else { 0 }).sum();

            (score, spec)
        })
        .filter(|(score, _)| *score > 0)
        .collect();

    scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.name.cmp(right.1.name)));
    scored.into_iter().take(limit.max(1)).map(|(_, spec)| spec).collect()
}

/// What `tool_search` answers: the matching tools' full schemas, and which of them join the offered list.
pub fn answer(matches: &[&ToolSpec]) -> String {
    if matches.is_empty() {
        return "No tool matches those words. Try other words, or work with the tools you already have.".to_string();
    }

    let schemas: Vec<String> = matches.iter().map(|spec| json!({ "name": spec.name, "description": spec.description, "parameters": spec.schema }).to_string()).collect();

    format!("Found {}. {} available from your next step.\n{}", matches.iter().map(|spec| spec.name).collect::<Vec<_>>().join(", "), if matches.len() == 1 { "It is" } else { "They are" }, schemas.join("\n"))
}

/// The `tool_search` tool itself.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "tool_search",
        description: "Find a tool that is not loaded yet. Say in a few words what you need; the matching tools are handed to you for the next step.",
        schema: json!({
            "type": "object",
            "properties": { "query": { "type": "string", "description": "What you need, in a few words (e.g. `run the tests`, `fetch a web page`)." } },
            "required": ["query"],
            "additionalProperties": false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::{specs_for, Caps};

    fn all() -> Vec<ToolSpec> {
        specs_for(Caps { vision: true, subagent: false, patch: true, local: true })
    }

    #[test]
    fn every_registered_tool_is_either_offered_or_in_the_catalogue() {
        let all = all();
        let offered = crate::sme::diet::at_most(&crate::sme::diet::prioritise(&all, |spec| spec.name), crate::sme::diet::MAX_SCHEMAS);
        let text = block(&offered, &all);

        for spec in &all {
            assert!(offered.iter().any(|known| known.name == spec.name) || text.contains(&format!("- {}: ", spec.name)), "{} is neither offered nor listed", spec.name);
        }

        assert!(!offered.iter().any(|spec| text.contains(&format!("- {}: ", spec.name))), "a tool already offered is not listed again");
    }

    #[test]
    fn the_catalogue_is_the_same_bytes_every_time_so_the_prefix_stays_cacheable() {
        let all = all();
        let offered = crate::sme::diet::at_most(&crate::sme::diet::prioritise(&all, |spec| spec.name), crate::sme::diet::MAX_SCHEMAS);
        let reversed: Vec<ToolSpec> = all.iter().rev().cloned().collect();

        assert_eq!(block(&offered, &all), block(&offered, &reversed), "the order the tools were registered in does not matter");
        assert_eq!(block(&offered, &all), block(&offered, &all));
        assert!(block(&offered, &all).lines().skip(3).all(|line| line.chars().count() <= LINE_CHARS + 24), "one short line each");
        assert_eq!(block(&all, &all), "", "nothing left out, nothing to list");
    }

    #[test]
    fn a_search_finds_the_tool_by_its_name_or_by_what_it_is_for() {
        let all = all();

        assert_eq!(search(&all, "web_fetch", 3)[0].name, "web_fetch");
        assert!(search(&all, "open a page in the browser and click", 3).iter().any(|spec| spec.name == "browser"));
        assert!(search(&all, "the patch format", 4).iter().any(|spec| spec.name == "apply_patch"));
        assert!(search(&all, "zzz qqq", 3).is_empty());
        assert!(search(&all, "", 3).is_empty());
        assert!(search(&all, "tool_search", 3).is_empty(), "the search does not find itself");
    }

    #[test]
    fn the_answer_carries_the_full_schema_of_what_matched() {
        let all = all();
        let found = search(&all, "git_diff", 1);
        let text = answer(&found);

        assert!(text.starts_with("Found git_diff. It is available from your next step."), "{text}");
        assert!(text.contains("\"parameters\""));
        assert!(answer(&[]).contains("No tool matches"));
    }

    #[test]
    fn the_search_tool_is_a_strict_object_with_one_required_query() {
        let spec = spec();

        assert_eq!(spec.schema["required"], json!(["query"]));
        assert_eq!(spec.schema["additionalProperties"], json!(false));
    }
}
