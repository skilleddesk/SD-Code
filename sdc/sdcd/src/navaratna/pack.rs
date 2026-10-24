//! **N3-1 and N3-2 - the fresh step prompt and the skeleton Context Pack** (switches `nr.fresh_step`, `nr.skeleton_pack`).
//!
//! A model that reads one ever-growing conversation loses what is in the middle of it and what happened long ago. So every step is a **new,
//! short prompt built from the ledger**: the rules, then the goal with its requirement ids and what done means (at the top), the step's own
//! spec units word for word, the files as **skeletons** (signatures with line numbers) with the full body only of the symbols the step
//! changes, contracts and glossary, the last few handover notes, at most one worked example - and the goal and done-when once more as the
//! very last line. No earlier chat turn is ever in it, so its size does not depend on how many steps came before.
//!
//! Over budget, what goes first is the worked example, then the handover notes, then the bodies of related symbols (their skeletons
//! stay). A spec unit is **never** dropped: if the units themselves do not fit, the answer is [`NeedsSplit`].

use super::spec::Unit;
use super::symbols;

/// Handover notes kept in a pack, newest last.
pub const HANDOVER_NOTES: usize = 3;
/// A related file this small (tokens) is included whole; a larger one only as a skeleton.
pub const WHOLE_FILE_TOKENS: u64 = 120;

#[derive(Debug, Clone, Default)]
pub struct FileCtx {
    pub path: String,
    pub text: String,
    /// Qualified names of the symbols the step changes: their bodies are always in the pack.
    pub targets: Vec<String>,
}

#[derive(Default)]
pub struct StepInput<'a> {
    pub rules: &'a str,
    pub goal: &'a str,
    pub requirement_ids: Vec<String>,
    pub done_when: &'a str,
    /// The step's spec units, word for word.
    pub units: Vec<&'a Unit>,
    pub files: Vec<FileCtx>,
    pub contracts: Vec<String>,
    pub glossary: Vec<(String, String)>,
    /// All handover notes so far, oldest first; only the last few are used.
    pub handover: Vec<String>,
    pub example: Option<String>,
    /// The token budget of the whole prompt (the step target of the window).
    pub budget_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsSplit {
    pub required: u64,
    pub budget: u64,
}

impl std::fmt::Display for NeedsSplit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NeedsSplit: the step's rules, goal and spec units alone are about {} tokens and the budget is {}; the step must be split, nothing was cut", self.required, self.budget)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub text: String,
    pub tokens: u64,
    /// What was dropped to fit, in the order it was dropped.
    pub dropped: Vec<&'static str>,
}

fn tokens(text: &str) -> u64 {
    crate::tokens::estimate(text)
}

/// A file as the pack shows it: whole when small (or when the skeleton switch is off), otherwise its skeleton plus the full bodies of the
/// targets. `rank` scores a qualified symbol name (0.0..=1.0); a non-target symbol under `MIN_RANK` is left out of the skeleton.
fn render_file(file: &FileCtx, with_bodies: bool, skeleton_mode: bool, rank: &dyn Fn(&str) -> f64) -> String {
    let small = tokens(&file.text) <= WHOLE_FILE_TOKENS;

    if small || !skeleton_mode {
        return format!("--- {} ---\n{}\n", file.path, file.text.trim_end());
    }

    let Some(all) = symbols::index_text(&file.path, &file.text) else {
        /* no grammar: the honest fallback is the head of the file, marked as such */
        let head: String = file.text.lines().take(40).collect::<Vec<_>>().join("\n");

        return format!("--- {} (no grammar: first 40 lines) ---\n{head}\n", file.path);
    };
    let is_target = |symbol: &symbols::Symbol| file.targets.iter().any(|t| t == &symbol.qualified || t == &symbol.name);
    let kept: Vec<&symbols::Symbol> = all.iter().filter(|symbol| is_target(symbol) || rank(&symbol.qualified) >= MIN_RANK || symbol.depth == 0 && symbol.kind != "fn" && symbol.kind != "def").collect();
    let mut out = format!("--- {} (skeleton) ---\n", file.path);

    for symbol in &kept {
        out.push_str(&format!("{}L{}: {}\n", "  ".repeat(symbol.depth), symbol.start_line, symbol.signature));
    }

    if with_bodies {
        for symbol in kept.iter().filter(|symbol| is_target(symbol)) {
            out.push_str(&format!("\n# body of {} (lines {}-{})\n{}\n", symbol.qualified, symbol.start_line, symbol.end_line, &file.text[symbol.start_byte..symbol.end_byte]));
        }
    }

    out
}

/// A symbol ranked below this is a look-alike that is left out of a skeleton.
pub const MIN_RANK: f64 = 0.2;

/// A simple ranker: the share of the query's terms found in the symbol's name.
pub fn rank_by_terms(query: &str) -> impl Fn(&str) -> f64 {
    let terms = crate::sme::retrieval::terms(query);

    move |name: &str| {
        if terms.is_empty() {
            return 1.0;
        }

        let lower = name.to_lowercase();
        let hits = terms.iter().filter(|term| lower.contains(term.as_str())).count();

        hits as f64 / terms.len() as f64
    }
}

fn section(title: &str, body: &str) -> String {
    format!("## {title}\n{body}\n\n")
}

/// The final line that repeats what matters: the goal and what done means.
pub fn closing_line(goal: &str, done_when: &str) -> String {
    format!("Remember - goal: {} | done when: {}", goal.trim(), done_when.trim())
}

/// Builds the pack for one step.
pub fn build(input: &StepInput<'_>, rank: &dyn Fn(&str) -> f64) -> Result<Pack, NeedsSplit> {
    let skeleton_mode = crate::v3::on("nr.skeleton_pack");
    let ids = input.requirement_ids.join(", ");
    let top = section("Goal", &format!("{}\nRequirements: {}\nDone when: {}", input.goal.trim(), if ids.is_empty() { "-" } else { &ids }, input.done_when.trim()));
    let units = section("Spec (word for word)", &input.units.iter().map(|unit| format!("[{}] {}", unit.id, unit.text)).collect::<Vec<_>>().join("\n"));
    let closing = closing_line(input.goal, input.done_when);
    let contracts = if input.contracts.is_empty() && input.glossary.is_empty() {
        String::new()
    } else {
        let mut body = input.contracts.join("\n");

        if !input.glossary.is_empty() {
            body.push_str("\nGlossary: ");
            body.push_str(&input.glossary.iter().map(|(term, meaning)| format!("{term} = {meaning}")).collect::<Vec<_>>().join("; "));
        }

        section("Contracts and glossary", body.trim())
    };
    let notes: Vec<&String> = input.handover.iter().rev().take(HANDOVER_NOTES).collect::<Vec<_>>().into_iter().rev().collect();
    let notes_text = if notes.is_empty() { String::new() } else { section("Handover notes", &notes.iter().map(|n| n.as_str()).collect::<Vec<_>>().join("\n")) };
    let example_text = input.example.as_ref().map(|example| section("A similar problem that was solved", example)).unwrap_or_default();

    let assemble = |with_bodies: bool, with_notes: bool, with_example: bool| -> String {
        let files: String = input.files.iter().map(|file| render_file(file, with_bodies, skeleton_mode, rank)).collect::<Vec<_>>().join("\n");
        let mut text = String::new();

        text.push_str(input.rules.trim());
        text.push_str("\n\n");
        text.push_str(&top);
        text.push_str(&units);

        if !files.is_empty() {
            text.push_str(&section("Code", files.trim_end()));
        }

        text.push_str(&contracts);

        if with_notes {
            text.push_str(&notes_text);
        }

        if with_example {
            text.push_str(&example_text);
        }

        text.push_str(&closing);
        text
    };

    /* the target bodies are what the step is for: they are dropped last of all (only the bodies of related symbols could go, and a related symbol has no body here) */
    let mut dropped = Vec::new();
    let mut with_notes = true;
    let mut with_example = input.example.is_some();
    let mut text = assemble(true, with_notes, with_example);

    if tokens(&text) > input.budget_tokens && with_example {
        with_example = false;
        dropped.push("example");
        text = assemble(true, with_notes, with_example);
    }

    if tokens(&text) > input.budget_tokens && with_notes && !notes.is_empty() {
        with_notes = false;
        dropped.push("handover notes");
        text = assemble(true, with_notes, with_example);
    }

    if tokens(&text) > input.budget_tokens {
        /* last resort short of splitting: bodies of the target symbols go, their skeletons stay */
        let slim = assemble(false, with_notes, with_example);

        if tokens(&slim) <= input.budget_tokens && skeleton_mode {
            dropped.push("target bodies");
            text = slim;
        }
    }

    let used = tokens(&text);

    if used > input.budget_tokens {
        let fixed = tokens(&format!("{}{}{}{}", input.rules, top, units, closing));

        return Err(NeedsSplit { required: fixed.max(used), budget: input.budget_tokens });
    }

    Ok(Pack { tokens: used, text, dropped })
}

#[cfg(test)]
mod tests;
