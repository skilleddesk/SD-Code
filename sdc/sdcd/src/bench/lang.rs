//! `sdcd bench lang` (v3 W1, A-14): what the same request costs in tokens in each language, per provider -
//! measured with the provider's own count, not estimated.
//!
//! Each sentence of `bench/lang/parallel.toml` is sent once per language as a one-message request (no tools);
//! the count of a one-character message is subtracted, which leaves the sentence's own tokens. The ratio to
//! English is reported per language, beside SDC's own estimate (`context::tokens_of`) so I8 can calibrate it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The languages of the parallel corpus; `en` is the reference.
pub const LANGUAGES: [&str; 5] = ["en", "bn", "banglish", "hi", "ar"];

#[derive(Debug, Clone, Deserialize)]
pub struct Corpus {
    pub sentence: Vec<BTreeMap<String, String>>,
}

impl Corpus {
    pub fn parse(text: &str) -> Result<Self, String> {
        let corpus: Corpus = toml::from_str(text).map_err(|error| error.to_string())?;

        for (index, sentence) in corpus.sentence.iter().enumerate() {
            if let Some(missing) = LANGUAGES.iter().find(|language| sentence.get(**language).is_none_or(|text| text.trim().is_empty())) {
                return Err(format!("sentence {}: no `{missing}`", index + 1));
            }
        }

        Ok(corpus)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LanguageCost {
    pub language: String,
    /// The provider's tokens for the whole corpus in this language.
    pub tokens: u64,
    /// Over English's.
    pub ratio: f64,
    /// SDC's estimate for the same text, and its ratio to the provider's count (1.0 = exact).
    pub estimated: u64,
    pub estimate_error: f64,
}

/// Turns per-language totals into ratios. `measured[language] = (provider tokens, SDC's estimate)`.
pub fn summarise(measured: &BTreeMap<String, (u64, u64)>) -> Vec<LanguageCost> {
    let english = measured.get("en").map(|(tokens, _)| *tokens).unwrap_or(0).max(1) as f64;

    LANGUAGES
        .iter()
        .filter_map(|language| {
            let (tokens, estimated) = *measured.get(*language)?;

            Some(LanguageCost {
                language: language.to_string(),
                tokens,
                ratio: tokens as f64 / english,
                estimated,
                estimate_error: if tokens == 0 { 0.0 } else { estimated as f64 / tokens as f64 },
            })
        })
        .collect()
}

/// Measures the corpus on one model: `probe` answers the provider's input tokens for a message.
pub fn measure(corpus: &Corpus, probe: &dyn Fn(&str) -> Result<u64, String>) -> Result<Vec<LanguageCost>, String> {
    let floor = probe(".")?;
    let mut measured: BTreeMap<String, (u64, u64)> = BTreeMap::new();

    for sentence in &corpus.sentence {
        for language in LANGUAGES {
            let text = &sentence[language];
            /* The floor message's "." is taken as one token and added back; if a tokenizer differs, the error is
               the same per sentence in every language, so it barely moves a ratio. */
            let tokens = probe(text)?.saturating_sub(floor) + 1;
            let entry = measured.entry(language.to_string()).or_default();

            entry.0 += tokens;
            entry.1 += crate::context::tokens_of(text);
        }
    }

    Ok(summarise(&measured))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_corpus_has_every_language_for_every_sentence() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/lang/parallel.toml");
        let corpus = Corpus::parse(&std::fs::read_to_string(path).unwrap()).unwrap();

        assert!(corpus.sentence.len() >= 10);
    }

    #[test]
    fn ratios_are_against_english_from_the_providers_own_count() {
        let corpus = Corpus::parse(
            r#"
[[sentence]]
en = "fix the bug"
bn = "বাগটা ঠিক করো"
banglish = "bug ta thik koro"
hi = "बग ठीक करो"
ar = "أصلح الخطأ"
"#,
        )
        .unwrap();
        /* A fake provider: 10 tokens of overhead, then one token per byte / 4, rounded up. */
        let probe = |text: &str| Ok(10 + text.len().div_ceil(4) as u64);
        let costs = measure(&corpus, &probe).unwrap();
        let english = costs.iter().find(|cost| cost.language == "en").unwrap();
        let bengali = costs.iter().find(|cost| cost.language == "bn").unwrap();

        assert_eq!(english.ratio, 1.0);
        assert!(bengali.ratio > 2.0, "Bengali's bytes cost more than English's: {}", bengali.ratio);
        assert!(Corpus::parse("[[sentence]]\nen = \"x\"").unwrap_err().contains("no `bn`"));
    }
}
