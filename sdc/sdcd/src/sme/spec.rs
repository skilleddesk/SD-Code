//! **The spec brief (P1-14, switch `sme.spec_brief`)**: a message written in Bengali or Banglish is turned into a short English
//! *technical* spec - the goal, the files, the constraints, what "done" means - and the person confirms it before any file is touched.
//!
//! The reason it exists: a small local model does best with short English technical English, and the person writes in the language they
//! think in. Translating the whole message loses its shape; **restating it as a spec keeps the shape and drops the poetry**. So the
//! brief is built from the message, not from a translation of it:
//!
//! * the **goal** is the first sentence, in English, with the person's own names kept;
//! * the **files** are every path, file name and backticked identifier in the message, exactly as typed - a spec that renames a path is
//!   worse than no spec;
//! * the **constraints** are the sentences that say what must not happen ("টেস্ট ভাঙা যাবে না", "don't touch the css");
//! * **done when** is the sentence that says how the work will be judged ("build pass hote hobe").
//!
//! Every line of the brief is one sentence, at most a dozen lines in all, and the confirmation is asked in the person's own language
//! beside the English: `এই বোঝাটা ঠিক? / Is this right?` - because a spec nobody confirmed is a guess with a straight face.

use std::collections::HashMap;

use crate::understand::Reading;

/// The English technical spec of one message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spec {
    pub goal: String,
    pub files: Vec<String>,
    pub constraints: Vec<String>,
    pub done_when: Vec<String>,
    /// The language the brief is confirmed in, as the person writes it (`বাংলা`, `Banglish`).
    pub language: String,
}

/// Words that mark a sentence as a constraint, in the three ways people write them.
const CONSTRAINT: &[&str] = &[
    "cholbe na", "hobe na", "korba na", "koro na", "touch korio na", "bondho", "avoid", "don't", "dont", "must not", "should not", "without changing", "change koro na",
    "যাবে না", "হবে না", "করো না", "পরিবর্তন করো না", "বন্ধ",
];

/// Words that mark a sentence as the test of the work.
const DONE: &[&str] = &[
    "test", "cargo", "npm test", "build pass", "pass hote", "pass korte", "kaj korbe", "kaj korle", "verify", "green", "error jeno na", "kono error",
    "টেস্ট", "পাস", "চালালে", "কাজ করবে", "ঠিক হলে",
];

/// The technical tokens of a message: paths, file names, backticked identifiers and the programs people name, exactly as typed.
pub fn technical(request: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();

    /* what was written in backticks is technical by the person's own marking */
    let mut rest = request;

    while let Some(open) = rest.find('`') {
        let Some(close) = rest[open + 1..].find('`') else {
            break;
        };
        let span = rest[open + 1..open + 1 + close].trim();

        if !span.is_empty() && span.len() <= 120 && !found.contains(&span.to_string()) {
            found.push(span.to_string());
        }

        rest = &rest[open + 1 + close + 1..];
    }

    for word in request.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '"' | '\'' | '(' | ')' | '?' | '!' | '।' | '`'));

        if cleaned.is_empty() || cleaned.len() > 120 || found.contains(&cleaned.to_string()) {
            continue;
        }

        let looks_like_path = cleaned.contains('/') || cleaned.contains('\\') || (cleaned.contains('.') && cleaned.rsplit('.').next().is_some_and(|ext| (1..=6).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())));
        let is_program = ["cargo", "npm", "git", "python3", "node", "make", "docker"].contains(&cleaned);

        if looks_like_path || is_program {
            found.push(cleaned.to_string());
        }
    }

    found
}

/// Splits a message into sentences the way people end them: `.`, `!`, `?`, the Bengali `।`, or a line break - but not the dot inside
/// `src/ui/header.rs`, which is the person's path and must survive whole.
fn sentences(request: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = request.chars().collect();

    for (index, c) in chars.iter().enumerate() {
        let inside_token = *c == '.'
            && index > 0
            && index + 1 < chars.len()
            && chars[index - 1].is_alphanumeric()
            && (chars[index + 1].is_alphanumeric() || chars[index + 1] == '/');

        if matches!(c, '!' | '?' | '।' | '\n') || (*c == '.' && !inside_token) {
            if !current.trim().is_empty() {
                out.push(current.trim().to_string());
            }

            current.clear();
        } else {
            current.push(*c);
        }
    }

    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }

    out
}

fn mentions(sentence: &str, words: &[&str]) -> bool {
    let lower = sentence.to_ascii_lowercase();

    words.iter().any(|word| lower.contains(word) || sentence.contains(word))
}

/// The spec of a message, when the message is in a language other than English. `None` for English (the agent already works in it) and
/// for a message with nothing technical in it.
pub fn of(request: &str, reading: &Reading, glossary: &HashMap<String, String>) -> Option<Spec> {
    /* only the two languages SDC has a briefing rule for: for English the agent needs nothing, and a language SDC cannot restate is
       not guessed at */
    if !matches!(reading.code, "bn" | "bn-Latn" | "hi" | "hi-Latn") {
        return None;
    }

    let sentences = sentences(request);

    if sentences.is_empty() {
        return None;
    }

    let mut spec = Spec { language: reading.reply_label.to_string(), files: technical(request), ..Default::default() };
    let translated: Vec<String> = sentences.iter().map(|sentence| crate::v3::interlingua::brief(sentence, reading.reply_label, glossary)).collect();

    for (index, sentence) in sentences.iter().enumerate() {
        let english = translated[index].lines().last().unwrap_or(sentence).trim();

        if mentions(sentence, CONSTRAINT) {
            spec.constraints.push(trim_sentence(english));
        } else if mentions(sentence, DONE) {
            spec.done_when.push(trim_sentence(english));
        } else if spec.goal.is_empty() {
            spec.goal = trim_sentence(english);
        }
    }

    if spec.goal.is_empty() {
        spec.goal = trim_sentence(sentences[0].as_str());
    }

    /* a spec that says nothing is not worth a confirmation */
    if spec.goal.is_empty() {
        return None;
    }

    Some(spec)
}

fn trim_sentence(text: &str) -> String {
    let trimmed = text.trim().trim_start_matches('[').trim_end_matches(']').trim();

    /* the interlingua brief wraps its English in a bracket line; keep the English sentence only */
    let line = trimmed.lines().find(|line| !line.trim_start().starts_with('[')).unwrap_or(trimmed);

    line.trim().trim_end_matches(['.', '।']).to_string()
}

impl Spec {
    /// The English brief the model reads: four short sections, one line an item, the person's technical words untouched.
    pub fn brief(&self) -> String {
        let mut lines = vec!["## The request, as a technical spec".to_string(), format!("Goal: {}", self.goal)];

        if !self.files.is_empty() {
            lines.push(format!("Files: {}", self.files.join(", ")));
        }

        if !self.constraints.is_empty() {
            lines.push(format!("Constraints: {}", self.constraints.join("; ")));
        }

        if !self.done_when.is_empty() {
            lines.push(format!("Done when: {}", self.done_when.join("; ")));
        }

        lines.join("\n")
    }

    /// The question the person is asked before the work starts - in their language, and in English beside it.
    pub fn confirm(&self) -> String {
        if self.language.contains("Bangla") || self.language.contains("বাংলা") || self.language.contains("Banglish") {
            "এই বোঝাটা ঠিক? ঠিক থাকলে বলুন, ভুল থাকলে কোনটা ভুল বলুন. / Is this right? Say so, or name what is wrong.".to_string()
        } else {
            "Is this right? Say so, or name what is wrong.".to_string()
        }
    }
}

#[cfg(test)]
mod tests;
