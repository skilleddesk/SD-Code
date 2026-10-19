//! **Working memory (P1-7, switch `sme.memory`)**: the few things a task established, held outside the conversation and put back into
//! every step's prompt - so a long turn stays where it is instead of unrolling its own history again.
//!
//! A note is one line with its kind, and a note that came from a file carries the path and the quote it rests on (P1-9 checks the quote
//! against the file, so a remembered "fact" cannot drift into an invention). The kinds are capped, because a small model reads a list
//! better than a log:
//!
//! | kind | kept | why |
//! |---|---|---|
//! | `fact` | 1 pinned | what the person said the task is: unchanged, and never dropped |
//! | `decision` | 5 | "the cache lives in `store/cache.rs`": the paths and choices the work rests on |
//! | `failure` | 8 | "`cargo test` fails on `x` because `y`": so the same dead end is not walked twice |
//!
//! A new note of a kind past its cap replaces the oldest of that kind. Pinned notes are never replaced, and the person can pin or drop a
//! note in the app - the table is theirs to edit. The rendered block is what the prompt carries: a header, the pinned note, then the
//! decisions and failures newest first, cut to the step's memory budget.

use serde_json::{json, Value};

pub use crate::store::memory::{cap_of, known_kind, Note};

/// The block a step's prompt carries: the pinned fact, then the newest decisions and failures, cut to `cap` characters. Empty when the
/// task has no notes.
pub fn render(notes: &[Note], cap: usize) -> String {
    let pinned: Vec<&Note> = notes.iter().filter(|note| note.pinned).collect();
    let rest: Vec<&Note> = notes.iter().filter(|note| !note.pinned).rev().collect();

    let mut lines = vec!["## This task, so far".to_string()];

    for note in pinned.into_iter().chain(rest) {
        lines.push(format!("- [{}] {}{}", note.kind, note.text, if note.source.is_empty() { String::new() } else { format!(" ({})", note.source) }));
    }

    let mut text = lines.join("\n");

    if text.len() > cap && cap > 0 {
        let mut kept = Vec::new();
        let mut spent = 0;

        for line in lines.iter() {
            if spent + line.len() + 1 > cap.saturating_sub(30) {
                break;
            }

            spent += line.len() + 1;
            kept.push(line.clone());
        }

        kept.push(format!("- ({} more not shown)", lines.len().saturating_sub(kept.len())));
        text = kept.join("\n");
    }

    /* a header with nothing under it is noise: say nothing instead */
    if lines.len() == 1 { String::new() } else { text }
}

/// Reads a note from "the person said ..." in a request: the line they wrote as a fact ("remember: ...", "the API is at ..."). Only an
/// explicit remembering is a note - guessing at importance is how a memory becomes noise.
pub fn requested_fact(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();

    for marker in ["remember that ", "remember: ", "note that ", "মনে রাখো: ", "মনে রাখো "] {
        if let Some(at) = lower.find(marker) {
            let said = text[at + marker.len()..].lines().next().unwrap_or_default().trim();

            if !said.is_empty() {
                return Some(said.trim_end_matches(['.', '।']).to_string());
            }
        }
    }

    None
}

/// The note the agent asked to keep, from the `remember` tool's input: `kind`, `text`, and optionally the `source` and `quote` it rests
/// on. `Err` is the sentence the model gets.
pub fn from_tool(input: &Value) -> Result<Note, String> {
    let text = input["text"].as_str().unwrap_or_default().trim();

    if text.is_empty() {
        return Err("`text` is required: the one line worth keeping".into());
    }

    if text.len() > 400 {
        return Err(format!("`text` is {} characters: a note is one line (under 400); split it or keep the part that matters", text.len()));
    }

    let kind = input["kind"].as_str().unwrap_or("decision");

    if !known_kind(kind) {
        return Err(format!("`{kind}` is not a kind of note: fact, decision, failure or open"));
    }

    let quote = input["quote"].as_str().unwrap_or_default();

    if quote.len() > 300 {
        return Err("`quote` is over 300 characters: quote the line the note rests on, not the file".into());
    }

    Ok(Note { seq: 0, kind: kind.to_string(), text: text.to_string(), source: input["source"].as_str().unwrap_or_default().trim().to_string(), quote: quote.to_string(), pinned: input["pin"].as_bool().unwrap_or(false) })
}

/// The note as the app's memory panel shows it.
pub fn as_json(note: &Note) -> Value {
    json!({ "seq": note.seq, "kind": note.kind, "text": note.text, "source": note.source, "quote": note.quote, "pinned": note.pinned })
}

#[cfg(test)]
mod tests;
