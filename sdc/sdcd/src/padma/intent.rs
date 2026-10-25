//! **Intent-without-Action guard (P0-3, switch `padma.intent_guard`)**: a reply with no tool call that announces an action
//! ("now checking the build:", "দেখছি") is not an answer. Pure code: the phrases are data (`intent_phrases.toml`).

use std::sync::OnceLock;

use serde::Deserialize;

pub const PHRASES: &str = include_str!("intent_phrases.toml");

/// The note sent back to the model.
pub const REPROMPT: &str = "[SDC: You described an action but did not call a tool. Call the tool now, or reply FINAL: <answer> if the task is complete.]";

/// How many times one turn is re-prompted before the Recovery Ladder takes over.
pub const MAX_REPROMPTS: u32 = 2;

#[derive(Debug, Deserialize)]
struct Phrases {
    future_intent: Words,
    numbered_start: Numbered,
    final_marker: Final,
}

#[derive(Debug, Deserialize)]
struct Words {
    phrases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Numbered {
    first: Vec<String>,
    second: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Final {
    prefixes: Vec<String>,
}

fn phrases() -> &'static Phrases {
    static PARSED: OnceLock<Phrases> = OnceLock::new();

    PARSED.get_or_init(|| toml::from_str(PHRASES).expect("padma/intent_phrases.toml is valid"))
}

/// What a reply was found to hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Signals {
    pub colon: bool,
    pub future_phrase: bool,
    pub opened_list: bool,
}

impl Signals {
    pub fn score(&self) -> u32 {
        u32::from(self.colon) + u32::from(self.future_phrase) + u32::from(self.opened_list)
    }

    /// The colon alone, or any two signals.
    pub fn announces(&self) -> bool {
        self.colon || self.score() >= 2
    }

    pub fn names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();

        if self.colon {
            names.push("ends with a colon");
        }

        if self.future_phrase {
            names.push("future-intent phrase");
        }

        if self.opened_list {
            names.push("opened a numbered list");
        }

        names
    }
}

/// True when the reply is a deliberate end: it starts with `FINAL:`.
pub fn is_final(text: &str) -> bool {
    let head = text.trim_start().to_lowercase();

    phrases().final_marker.prefixes.iter().any(|prefix| head.starts_with(prefix.as_str()))
}

/// The reply without its `FINAL:` marker.
pub fn strip_final(text: &str) -> String {
    let trimmed = text.trim_start();
    let lower = trimmed.to_lowercase();

    for prefix in &phrases().final_marker.prefixes {
        if lower.starts_with(prefix.as_str()) {
            return trimmed[prefix.len()..].trim_start().to_string();
        }
    }

    text.to_string()
}

/// Reads a reply that made no tool call.
pub fn detect(text: &str) -> Signals {
    let trimmed = text.trim();

    if trimmed.is_empty() || is_final(trimmed) {
        return Signals::default();
    }

    let lower = trimmed.to_lowercase();
    let colon = trimmed.ends_with(':') || trimmed.ends_with('：');
    let future_phrase = phrases().future_intent.phrases.iter().any(|phrase| lower.contains(phrase.as_str()));
    let opened_list = {
        let has = |words: &[String]| lower.lines().any(|line| words.iter().any(|word| line.trim_start().starts_with(word.as_str())));

        has(&phrases().numbered_start.first) && !has(&phrases().numbered_start.second)
    };

    Signals { colon, future_phrase, opened_list }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colon_at_the_end_is_enough() {
        assert!(detect("now checking build:").announces());
        assert!(detect("Let me look at the files below:\n").announces());
    }

    #[test]
    fn a_future_phrase_alone_is_not_enough_but_two_signals_are() {
        assert!(!detect("I'll be happy to help. The answer is 4.").announces());
        assert!(detect("I'll start with the config.\n1. read the config").announces());
        assert!(detect("এখন আমি কোডটা দেখছি।\nধাপ ১: ফাইল পড়া").announces());
    }

    #[test]
    fn a_genuine_short_answer_is_left_alone() {
        assert!(!detect("2 + 2 = 4").announces());
        assert!(!detect("Yes. The function returns early when the list is empty.").announces());
        assert!(!detect("").announces());
    }

    #[test]
    fn a_two_item_list_is_not_an_opened_list() {
        assert!(!detect("1. read it\n2. patch it").opened_list);
    }

    #[test]
    fn final_marks_a_deliberate_end() {
        assert!(!detect("FINAL: the build passes:").announces());
        assert_eq!(strip_final("FINAL: all done"), "all done");
    }

    #[test]
    fn the_phrase_file_parses() {
        assert!(phrases().future_intent.phrases.len() > 10);
    }
}
