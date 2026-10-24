//! **N2-1 - the prompt parser, sentence coverage and requirement ids** (switch `nr.sentence_cover`, on, verification only).
//!
//! A big prompt is cut by code, not by a model, into **units**: headings, list items, tables, fenced code, JSON and quoted blocks stay whole
//! as one unit each, and a conditional ("if ... then ... else", "unless", "যদি ... তবে") is never cut in the middle. Units are numbered
//! `S-001`...; a labeller (the local 9B, later the cloud Blueprint) says for each one whether it is a *requirement* or *background*; each
//! requirement gets an id `R-001`... and keeps its byte span in the prompt. The **coverage gate** then requires that every unit is
//! labelled - an unlabelled one blocks planning, by name. Nothing is ever dropped or summarised: the prompt and the map are kept whole in the
//! vault, and every non-blank character of the prompt lies in exactly one unit (a test checks it).

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Heading,
    Sentence,
    ListItem,
    Table,
    Code,
    Json,
    Quote,
}

impl Kind {
    /// A block that is never cut and never read as prose.
    pub fn is_block(self) -> bool {
        matches!(self, Kind::Table | Kind::Code | Kind::Json | Kind::Quote)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Unit {
    /// `S-001`...
    pub id: String,
    pub kind: Kind,
    pub text: String,
    /// Byte span in the prompt: `prompt[start..end] == text`.
    pub start: usize,
    pub end: usize,
    /// 1-based line numbers.
    pub line_from: usize,
    pub line_to: usize,
}

struct Line<'a> {
    text: &'a str,
    start: usize,
}

fn lines_of(prompt: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut start = 0;

    for piece in prompt.split_inclusive('\n') {
        out.push(Line { text: piece.trim_end_matches(['\n', '\r']), start });
        start += piece.len();
    }

    out
}

fn indent(text: &str) -> usize {
    text.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn is_fence(text: &str) -> Option<&'static str> {
    let t = text.trim_start();

    if t.starts_with("```") {
        Some("```")
    } else if t.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn list_marker(text: &str) -> bool {
    let t = text.trim_start();
    let mut chars = t.chars();

    match chars.next() {
        Some('-' | '*' | '+' | '•') => chars.next().is_some_and(char::is_whitespace),
        Some(c) if c.is_ascii_digit() => {
            let rest = t.trim_start_matches(|c: char| c.is_ascii_digit());

            (rest.starts_with(". ") || rest.starts_with(") ")) && rest.len() < t.len()
        }
        _ => false,
    }
}

fn is_table(text: &str) -> bool {
    text.trim_start().starts_with('|')
}

fn is_heading(text: &str) -> bool {
    let t = text.trim_start();
    let hashes = t.chars().take_while(|c| *c == '#').count();

    (1..=6).contains(&hashes) && t[hashes..].starts_with(' ')
}

fn starts_block(text: &str) -> bool {
    is_fence(text).is_some() || is_table(text) || is_heading(text) || text.trim_start().starts_with('>') || list_marker(text)
}

const IF_WORDS: &[&str] = &["if", "when", "unless", "whenever", "in case", "provided", "যদি", "যখন", "যেহেতু", "জদি", "jodi", "jokhon", "jodi", "যদিও"];
const ELSE_WORDS: &[&str] = &["otherwise", "else", "or else", "if not", "if so", "then", "in that case", "নাহলে", "না হলে", "নইলে", "অন্যথায়", "তাহলে", "তবে", "nahole", "na hole", "tahole", "tobe", "noile"];

fn lower_starts(text: &str, words: &[&str]) -> bool {
    let lower = text.trim_start().trim_start_matches(['-', '*', '+', '•', '"', '\'', '(']).trim_start().to_lowercase();
    let lower = lower.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')').trim_start();

    words.iter().any(|word| {
        lower.starts_with(word) && lower[word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() || !word.is_ascii())
    })
}

/// Does a text open a condition?
pub fn is_conditional(text: &str) -> bool {
    lower_starts(text, IF_WORDS)
}

fn is_else(text: &str) -> bool {
    lower_starts(text, ELSE_WORDS)
}

/// A JSON value that starts at `lines[i]`: the index after its last line, if it is balanced and parses.
fn json_block(lines: &[Line<'_>], i: usize, prompt: &str) -> Option<usize> {
    let first = lines[i].text.trim_start();

    if !(first.starts_with('{') || first.starts_with('[')) {
        return None;
    }

    let mut depth = 0i64;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, line) in lines[i..].iter().enumerate().take(2_000) {
        for c in line.text.chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }

                continue;
            }

            match c {
                '"' => in_string = true,
                '{' | '[' => depth += 1,
                '}' | ']' => depth -= 1,
                _ => {}
            }
        }

        if depth == 0 && !in_string {
            let end_line = i + offset;
            let end = lines[end_line].start + lines[end_line].text.len();
            let candidate = &prompt[lines[i].start..end];

            return serde_json::from_str::<serde_json::Value>(candidate.trim()).ok().map(|_| end_line + 1);
        }
    }

    None
}

fn push(units: &mut Vec<Unit>, prompt: &str, kind: Kind, start: usize, end: usize) {
    let slice = &prompt[start..end];
    let trimmed_start = start + (slice.len() - slice.trim_start().len());
    let trimmed_end = end - (slice.len() - slice.trim_end().len());

    if trimmed_start >= trimmed_end {
        return;
    }

    let line_from = prompt[..trimmed_start].matches('\n').count() + 1;
    let line_to = prompt[..trimmed_end].matches('\n').count() + 1;

    units.push(Unit { id: String::new(), kind, text: prompt[trimmed_start..trimmed_end].to_string(), start: trimmed_start, end: trimmed_end, line_from, line_to });
}

/// Splits prose into sentences, as spans `(start, end)` relative to `text`. A terminator inside `code`, a "quoted string", a (bracket) or
/// a number such as 3.5 does not end a sentence; the Bengali danda always does.
pub fn sentence_spans(text: &str) -> Vec<(usize, usize)> {
    const ABBREVIATIONS: [&str; 10] = ["e.g", "i.e", "etc", "vs", "mr", "mrs", "dr", "fig", "no", "approx"];

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut tick = false;
    let mut quote: Option<char> = None;
    let mut paren = 0i32;
    let mut i = 0;

    while i < chars.len() {
        let (at, c) = chars[i];

        if c == '`' {
            tick = !tick;
        } else if !tick {
            match (quote, c) {
                (None, '"') | (None, '“') => quote = Some(if c == '"' { '"' } else { '”' }),
                (Some(q), _) if c == q => quote = None,
                (None, '(' | '[') => paren += 1,
                (None, ')' | ']') => paren = (paren - 1).max(0),
                _ => {}
            }
        }

        let free = !tick && quote.is_none() && paren == 0;

        if free && matches!(c, '.' | '!' | '?' | '।' | '॥') {
            let after = chars.get(i + 1).map(|(_, n)| *n);
            let terminator_ends = after.is_none_or(char::is_whitespace);
            let mut boundary = c == '।' || c == '॥' || terminator_ends;

            if boundary && c == '.' {
                let word_start = text[start..at].rfind(char::is_whitespace).map_or(start, |p| start + p + 1);
                let word = text[word_start..at].trim_start_matches(['(', '"']).to_lowercase();
                let next_word_lower = text[at + 1..].trim_start().chars().next().is_some_and(|n| n.is_lowercase() && n.is_ascii());

                if ABBREVIATIONS.contains(&word.as_str()) || next_word_lower || word.is_empty() && at == start {
                    boundary = false;
                }
            }

            if boundary {
                let mut end_i = i;

                while chars.get(end_i + 1).is_some_and(|(_, n)| matches!(n, '"' | '\'' | ')' | ']' | '”' | '`')) {
                    end_i += 1;
                }

                let end = chars.get(end_i + 1).map_or(text.len(), |(p, _)| *p);

                spans.push((start, end));

                while chars.get(end_i + 1).is_some_and(|(_, n)| n.is_whitespace()) {
                    end_i += 1;
                }

                start = chars.get(end_i + 1).map_or(text.len(), |(p, _)| *p);
                i = end_i;
            }
        }

        i += 1;
    }

    if start < text.len() && !text[start..].trim().is_empty() {
        spans.push((start, text.len()));
    }

    /* a condition and its "otherwise" stay one unit, however many sentences it took to say; once an else is in, the unit is closed */
    let mut merged: Vec<(usize, usize)> = Vec::new();
    let mut closed = true;

    for span in spans {
        let sentence = &text[span.0..span.1];

        if let Some(last) = merged.last_mut() {
            let previous = &text[last.0..last.1];

            if !closed && is_conditional(previous) && is_else(sentence) && !is_conditional(sentence) {
                last.1 = span.1;
                closed = true;
                continue;
            }
        }

        closed = !is_conditional(sentence);
        merged.push(span);
    }

    merged
}

/// Cuts a prompt into units. Every non-blank character of the prompt is in exactly one unit.
pub fn parse(prompt: &str) -> Vec<Unit> {
    let lines = lines_of(prompt);
    let mut units: Vec<Unit> = Vec::new();
    let mut i = 0;

    let line_end = |index: usize| lines[index].start + lines[index].text.len();

    while i < lines.len() {
        let line = &lines[i];

        if line.text.trim().is_empty() {
            i += 1;
            continue;
        }

        if let Some(fence) = is_fence(line.text) {
            let mut j = i + 1;

            while j < lines.len() && !lines[j].text.trim_start().starts_with(fence) {
                j += 1;
            }

            let last = j.min(lines.len() - 1);

            push(&mut units, prompt, Kind::Code, line.start, line_end(last));
            i = last + 1;
            continue;
        }

        if is_table(line.text) {
            let mut j = i;

            while j + 1 < lines.len() && is_table(lines[j + 1].text) {
                j += 1;
            }

            push(&mut units, prompt, Kind::Table, line.start, line_end(j));
            i = j + 1;
            continue;
        }

        if is_heading(line.text) {
            push(&mut units, prompt, Kind::Heading, line.start, line_end(i));
            i += 1;
            continue;
        }

        if line.text.trim_start().starts_with('>') {
            let mut j = i;

            while j + 1 < lines.len() && lines[j + 1].text.trim_start().starts_with('>') {
                j += 1;
            }

            push(&mut units, prompt, Kind::Quote, line.start, line_end(j));
            i = j + 1;
            continue;
        }

        if let Some(next) = json_block(&lines, i, prompt) {
            push(&mut units, prompt, Kind::Json, line.start, line_end(next - 1));
            i = next;
            continue;
        }

        if list_marker(line.text) {
            let base = indent(line.text);
            let conditional = is_conditional(line.text) || line.text.trim_end().ends_with(':');
            let mut j = i;

            while j + 1 < lines.len() {
                let next = &lines[j + 1];

                if next.text.trim().is_empty() {
                    break;
                }

                let deeper = indent(next.text) > base;

                if deeper && (conditional || !list_marker(next.text)) && is_fence(next.text).is_none() {
                    j += 1;
                } else {
                    break;
                }
            }

            /* an "else" item at the same level belongs to the condition before it */
            if conditional {
                while j + 1 < lines.len() && indent(lines[j + 1].text) == base && list_marker(lines[j + 1].text) && is_else(lines[j + 1].text) {
                    j += 1;

                    while j + 1 < lines.len() && !lines[j + 1].text.trim().is_empty() && indent(lines[j + 1].text) > base {
                        j += 1;
                    }
                }
            }

            push(&mut units, prompt, Kind::ListItem, line.start, line_end(j));
            i = j + 1;
            continue;
        }

        /* a paragraph: lines up to a blank line or the start of a block */
        let mut j = i;

        while j + 1 < lines.len() && !lines[j + 1].text.trim().is_empty() && !starts_block(lines[j + 1].text) && json_block(&lines, j + 1, prompt).is_none() {
            j += 1;
        }

        let from = line.start;
        let to = line_end(j);
        let paragraph = &prompt[from..to];

        for (a, b) in sentence_spans(paragraph) {
            push(&mut units, prompt, Kind::Sentence, from + a, from + b);
        }

        i = j + 1;
    }

    for (n, unit) in units.iter_mut().enumerate() {
        unit.id = format!("S-{:03}", n + 1);
    }

    units
}

/* ---- labels, requirement ids, the coverage gate ------------------------------------------------------------------------------------ */

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Label {
    Requirement,
    Background,
    /// Belongs to an earlier requirement unit (a code block that shows what the sentence before it asked for).
    PartOf(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Map {
    pub units: Vec<Unit>,
    pub labels: BTreeMap<String, Label>,
    /// `R-001` -> the unit ids it is made of, in order.
    pub requirements: BTreeMap<String, Vec<String>>,
}

impl Map {
    /// Units no one has labelled.
    pub fn unmapped(&self) -> Vec<&Unit> {
        self.units.iter().filter(|unit| !self.labels.contains_key(&unit.id)).collect()
    }

    /// The requirement a unit belongs to.
    pub fn requirement_of(&self, unit_id: &str) -> Option<&str> {
        self.requirements.iter().find(|(_, units)| units.iter().any(|id| id == unit_id)).map(|(r, _)| r.as_str())
    }
}

/// Runs a labeller over the units in order. The labeller may return `None` for a unit it cannot judge - that unit stays unmapped.
pub fn label(units: Vec<Unit>, labeller: &mut dyn FnMut(&Unit, Option<&str>) -> Option<Label>) -> Map {
    let mut map = Map { units, ..Map::default() };
    let mut last_requirement: Option<String> = None;
    let units = map.units.clone();

    for unit in &units {
        let Some(answer) = labeller(unit, last_requirement.as_deref().and_then(|r| map.requirements.get(r).and_then(|u| u.first()).map(String::as_str))) else {
            continue;
        };

        match &answer {
            Label::Requirement => {
                let r_id = format!("R-{:03}", map.requirements.len() + 1);

                map.requirements.insert(r_id.clone(), vec![unit.id.clone()]);
                last_requirement = Some(r_id);
            }
            Label::PartOf(owner) => {
                let r_id = map.requirement_of(owner).map(str::to_string);

                match r_id {
                    Some(r_id) => map.requirements.entry(r_id).or_default().push(unit.id.clone()),
                    /* a part of nothing is not covered: it stays unmapped */
                    None => continue,
                }
            }
            Label::Background => {}
        }

        map.labels.insert(unit.id.clone(), answer);
    }

    map
}

/// Words that make a unit a requirement when no model is there to say.
const CUES: &[&str] = &[
    "must", "should", "shall", "need", "needs", "add ", "create", "implement", "build", "make ", "ensure", "write", "support", "allow", "return", "show", "fix", "update", "remove", "use ", "validate", "handle", "store", "send", "read", "generate",
    "করো", "কর ", "করতে", "করবে", "চাই", "দরকার", "বানাও", "বানাতে", "যোগ", "দেখাও", "লেখো", "ঠিক", "korbe", "korte", "chai", "banao", "dekhao",
];

/// A deterministic stand-in for the model's labelling (and the fallback when none is available): prose with an instruction in it is a
/// requirement, a heading is background, a block that follows a requirement belongs to it.
pub fn heuristic(unit: &Unit, last_requirement_unit: Option<&str>) -> Option<Label> {
    match unit.kind {
        Kind::Heading => Some(Label::Background),
        Kind::Code | Kind::Json | Kind::Table | Kind::Quote => Some(last_requirement_unit.map_or(Label::Background, |owner| Label::PartOf(owner.to_string()))),
        Kind::Sentence | Kind::ListItem => {
            let lower = unit.text.to_lowercase();

            Some(if CUES.iter().any(|cue| lower.contains(cue)) { Label::Requirement } else { Label::Background })
        }
    }
}

/// The result of the coverage gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    pub unmapped: Vec<String>,
}

impl std::fmt::Display for Blocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "planning is blocked: {} unit(s) of the prompt are not mapped to a requirement or marked background: {}", self.unmapped.len(), self.unmapped.join(", "))
    }
}

/// 100% of the units must be labelled, or planning is blocked with the list. With the switch off the gate lets everything through.
pub fn coverage_gate(map: &Map) -> Result<(), Blocked> {
    if !crate::v3::on("nr.sentence_cover") {
        return Ok(());
    }

    let unmapped: Vec<String> = map.unmapped().iter().map(|unit| unit.id.clone()).collect();

    if unmapped.is_empty() {
        Ok(())
    } else {
        Err(Blocked { unmapped })
    }
}

/// Every non-blank character of the prompt lies in exactly one unit - nothing was dropped, nothing counted twice.
pub fn accounts_for_everything(prompt: &str, units: &[Unit]) -> bool {
    let mut covered = vec![0u8; prompt.len()];

    for unit in units {
        if unit.end > prompt.len() || prompt.get(unit.start..unit.end) != Some(unit.text.as_str()) {
            return false;
        }

        for slot in &mut covered[unit.start..unit.end] {
            *slot += 1;
        }
    }

    prompt.char_indices().all(|(at, c)| c.is_whitespace() || covered[at] == 1) && covered.iter().all(|n| *n <= 1)
}

/// Stores the prompt and the map in the vault; answers the two ids.
pub fn store(root: &Path, prompt: &str, map: &Map) -> Result<(String, String), String> {
    let spec = super::vault::put(root, "spec", prompt)?;
    let json = serde_json::to_string_pretty(map).map_err(|error| error.to_string())?;
    let map_id = super::vault::put(root, "specmap", &json)?;

    Ok((spec, map_id))
}

/// A whole parse-label-check pass with the deterministic labeller.
pub fn analyse(prompt: &str) -> Map {
    label(parse(prompt), &mut |unit, last| heuristic(unit, last))
}

/// A prompt shorter than this is not a "big prompt": it is not parsed, stored or checked.
pub const BIG_PROMPT_CHARS: usize = 1_500;

/// What the agent does with a big prompt at the start of a turn: parse it, label it, run the coverage gate and keep the prompt and the map
/// in the vault. Verification only - the model is told nothing, unless the gate blocks (then it is told which units are unmapped). `None`
/// for a short prompt or with the switch off.
pub fn check_big_prompt(root: &Path, prompt: &str) -> Option<Result<(String, String), Blocked>> {
    if !crate::v3::on("nr.sentence_cover") || prompt.len() < BIG_PROMPT_CHARS {
        return None;
    }

    let map = analyse(prompt);

    if let Err(blocked) = coverage_gate(&map) {
        return Some(Err(blocked));
    }

    store(root, prompt, &map).ok().map(Ok)
}

#[cfg(test)]
mod tests;
