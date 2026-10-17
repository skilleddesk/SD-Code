//! **One token estimate for the whole daemon (Phase 0.9).**
//!
//! Before this there were four: `context::tokens_of` (a Bengali character = 1 token), `trust::cost::tokens_in` (= ½ token),
//! `guard::context::estimate_text` (= 2 tokens, ASCII at 3 bytes) and `chars / 4` for thinking and in the UI. A Bengali
//! prompt was therefore costed at about half of what a tokenizer counts (cost) and budgeted at twice it (guard) at the
//! same time.
//!
//! This is a **script-aware, upper-biased estimate calibrated against real tokenizers** (OpenAI `cl100k_base` and
//! `o200k_base`, measured with `tiktoken`; the samples and their real counts are `protocol/token-fixtures.json`, which the
//! Rust and the TypeScript tests both read). It is not a tokenizer - the providers' own are not all public - so it is an
//! estimate, and the one rule it keeps is the Runtime Guard's: **it never undercounts** (every fixture's estimate is at least
//! the larger real count; latin text and code are about 1.1-1.3x, Bengali about 1.1x of the worse tokenizer).
//!
//! The algorithm is the same in `app/src/lib/tokens.ts`; change both together (the fixtures fail otherwise).

/// What one character of each script costs, in tokens (measured on `cl100k_base`, the harsher of the two, plus margin).
const ARABIC: f64 = 0.9;
const BRAHMIC: f64 = 1.5;
const CJK: f64 = 1.1;
const CYRILLIC: f64 = 0.55;
const GREEK: f64 = 1.0;
const HEBREW: f64 = 1.15;
const EMOJI: f64 = 3.0;
const OTHER: f64 = 2.0;
/// A punctuation or symbol character (most merge with a neighbour in a real tokenizer; counted at less than one).
const PUNCT: f64 = 0.7;
/// A line break (tokenizers have tokens for runs of them).
const NEWLINE: f64 = 0.5;
/// Letters and digits (including accented Latin) run together into words; a word is about this many characters a token.
const WORD: f64 = 5.0;
/// The margin that keeps the estimate above the real count on every fixture and on this repository's own source.
const MARGIN: f64 = 1.08;

enum Class {
    Space,
    Newline,
    Word,
    Punct,
    Weighted(f64),
}

fn class(c: char) -> Class {
    let o = c as u32;

    if c == '\n' {
        return Class::Newline;
    }

    if c.is_whitespace() {
        return Class::Space;
    }

    if o < 128 {
        return if c.is_ascii_alphanumeric() || c == '_' { Class::Word } else { Class::Punct };
    }

    match o {
        0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Class::Weighted(ARABIC),
        0x0900..=0x0DFF | 0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF => Class::Weighted(BRAHMIC),
        0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF => Class::Weighted(CJK),
        0x00C0..=0x024F | 0x1E00..=0x1EFF => Class::Word,
        0x0400..=0x052F => Class::Weighted(CYRILLIC),
        0x0370..=0x03FF => Class::Weighted(GREEK),
        0x0590..=0x05FF => Class::Weighted(HEBREW),
        _ if o >= 0x1F000 || (0x2600..=0x27BF).contains(&o) => Class::Weighted(EMOJI),
        _ => Class::Weighted(OTHER),
    }
}

/// The estimated tokens of `text` - never fewer than a real tokenizer counts (see the module's fixtures).
pub fn estimate(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }

    let mut total = 0.0f64;
    let mut run = 0u64;

    for c in text.chars() {
        let kind = class(c);

        if matches!(kind, Class::Word) {
            run += 1;
            continue;
        }

        if run > 0 {
            total += (run as f64 / WORD).ceil();
            run = 0;
        }

        match kind {
            Class::Newline => total += NEWLINE,
            Class::Punct => total += PUNCT,
            Class::Weighted(weight) => total += weight,
            Class::Space | Class::Word => {}
        }
    }

    if run > 0 {
        total += (run as f64 / WORD).ceil();
    }

    (total * MARGIN).ceil() as u64 + 1
}

/// For a length with no text to look at (a byte count from a log line): ASCII-sized, so a third of it - upper-biased.
pub fn estimate_chars(chars: usize) -> u64 {
    (chars as u64).div_ceil(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixtures() -> Vec<Value> {
        let text = include_str!("../../protocol/token-fixtures.json");
        let doc: Value = serde_json::from_str(text).expect("the shared fixtures parse");

        doc["samples"].as_array().cloned().expect("samples")
    }

    /// The estimate equals the number the fixtures carry (so Rust and TypeScript agree), and never undercounts a real tokenizer.
    #[test]
    fn the_estimate_matches_the_shared_fixtures_and_never_undercounts_a_real_tokenizer() {
        for sample in fixtures() {
            let name = sample["name"].as_str().unwrap();
            let text = sample["text"].as_str().unwrap();
            let (cl, o2, expected) = (sample["cl100k"].as_u64().unwrap(), sample["o200k"].as_u64().unwrap(), sample["estimate"].as_u64().unwrap());
            let got = estimate(text);

            assert_eq!(got, expected, "{name}: Rust and the fixtures must agree");
            assert!(got >= cl.max(o2), "{name}: estimate {got} undercounts the real {cl}/{o2}");
            assert!(got as f64 <= cl.max(o2) as f64 * 2.0, "{name}: estimate {got} is more than twice the real {cl}/{o2}");
        }
    }

    /// The 2x error this module exists to remove: Bengali was costed at ~0.5 token a character, a tokenizer says ~1.2.
    #[test]
    fn bengali_is_no_longer_counted_at_half_of_what_a_tokenizer_counts() {
        let sample = fixtures().into_iter().find(|sample| sample["name"] == "bengali").unwrap();
        let text = sample["text"].as_str().unwrap();
        let real = sample["cl100k"].as_u64().unwrap();
        let old_cost = text.chars().filter(|c| c.is_ascii()).count() as u64 / 4 + text.chars().filter(|c| !c.is_ascii()).count() as u64 / 2 + 1;

        assert!(old_cost * 2 < real + 20, "the old cost.rs rule was about half of the real count: {old_cost} vs {real}");
        assert!(estimate(text) >= real);
    }

    #[test]
    fn empty_and_whitespace_texts_are_nearly_free() {
        assert_eq!(estimate(""), 0);
        assert!(estimate("   \t  ") <= 2);
    }

    #[test]
    fn an_estimate_over_the_repos_own_source_stays_within_bounds() {
        /* Real counts of these files (tiktoken, 2026-10-18) were 31.7k tokens for 124k chars of Rust: ours must not undercount. */
        let text = include_str!("tokens.rs");
        let chars = text.chars().count() as f64;

        assert!(estimate(text) as f64 >= chars / 4.5, "code is about 4 characters a token");
        assert!(estimate(text) as f64 <= chars / 2.0);
    }
}
