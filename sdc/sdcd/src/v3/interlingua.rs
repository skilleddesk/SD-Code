//! **Interlingua** (v3 I8, switch `v3.interlingua`, a token technique - off until it passes Q3): SDC thinks and
//! works in English, and the person writes their own language or mixes the two. Instead of a model guessing what
//! Banglish meant, the message is turned into one English brief before it travels: the person's technical words
//! (paths, identifiers, flags, quoted code) are kept exactly, and the words SDC has learned from the person are
//! translated. The answer still comes back in the person's language - the brief says so.
//!
//! The vocabulary is the person's own: SDC learns their words when they gloss them (`kaj (work)`, `kaj = work`) and
//! keeps them in `.sdc/interlingua.json`, which they can also edit by hand. A message already in English is sent
//! untouched - nothing is spent on a brief that would say the same thing.

use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const FILE: &str = ".sdc/interlingua.json";

/// Words SDC knows without being taught: the everyday words of the languages its people write alongside English.
/// The list is deliberately small and only ever *adds* to what the person taught - a wrong guess costs tokens.
const WORDS: &[(&str, &str)] = &[
    ("kaj", "work"), ("kajta", "the work"), ("koro", "do"), ("korbo", "will do"), ("kori", "do"), ("kore", "done"), ("kora", "do"), ("korte", "to do"),
    ("korle", "if you do"), ("koro na", "do not"), ("banao", "create"), ("banate", "to create"), ("baniye", "created"), ("thik", "correct"), ("thik koro", "fix"),
    ("bhul", "wrong"), ("bhul ta", "the mistake"), ("dekho", "look at"), ("dekhe", "looked at"), ("dekhte", "to look at"), ("cholo", "let us"), ("chalu", "running"),
    ("bondho", "stop"), ("suru", "start"), ("suru koro", "start"), ("shesh", "finished"), ("hoyeche", "has happened"), ("hobe", "will be"), ("hoy", "happens"),
    ("hoyni", "did not happen"), ("hoyto", "maybe"), ("lagbe", "is needed"), ("lage", "is needed"), ("mitthe", "false"), ("mile", "matches"), ("mile na", "does not match"),
    ("ekta", "a"), ("shob", "all"), ("onek", "many"), ("aro", "more"), ("ektu", "a little"), ("khub", "very"), ("valo", "good"), ("bhalo", "good"), ("kharap", "bad"),
    ("kintu", "but"), ("jodi", "if"), ("nahole", "otherwise"), ("ar", "and"), ("tahole", "then"), ("keno", "why"), ("kothay", "where"), ("kemon", "how"), ("kemon kore", "how"),
    ("age", "before"), ("pore", "after"), ("theke", "from"), ("diye", "with"), ("jonno", "for"), ("moddhe", "in"), ("upore", "above"), ("niche", "below"),
    ("ta", "the"), ("tar", "its"), ("ei", "this"), ("oi", "that"), ("ami", "I"), ("tumi", "you"), ("apni", "you"), ("amake", "me"), ("dekhao", "show me"), ("bolo", "tell me"),
    ("dao", "give"), ("niye", "take"), ("chai", "want"), ("dorkar", "need"), ("sompurno", "complete"), ("puro", "the whole"), ("sob", "all"), ("kisu", "something"),
    ("karo", "do"), ("kaam", "work"), ("theek", "correct"), ("theek karo", "fix"), ("bana", "make"), ("chalao", "run"), ("chal", "run"), ("band", "stop"), ("shuru", "start"),
    ("dekho na", "look at"), ("likho", "write"), ("likho na", "do not write"), ("choto", "small"), ("boro", "big"), ("notun", "new"), ("purano", "old"), ("jodi", "if"),
];

/// The script a message is written in, when it is not Latin - the language can then be named without a dictionary.
fn script_language(text: &str) -> Option<&'static str> {
    let name = text.chars().find_map(|c| match c as u32 {
        0x0980..=0x09FF => Some("Bangla"),
        0x0900..=0x097F => Some("Hindi"),
        0x0600..=0x06FF => Some("Arabic"),
        0x4E00..=0x9FFF => Some("Chinese"),
        0x3040..=0x30FF => Some("Japanese"),
        0xAC00..=0xD7AF => Some("Korean"),
        0x0400..=0x04FF => Some("Russian"),
        0x0E00..=0x0E7F => Some("Thai"),
        _ => None,
    })?;

    Some(name)
}

/// How many of the person's own words the message uses.
pub fn known_words(text: &str, glossary: &HashMap<String, String>) -> usize {
    let lower = format!(" {} ", text.to_lowercase());
    let mut hits: HashSet<&str> = HashSet::new();

    let taught: Vec<&str> = glossary.keys().map(String::as_str).collect();

    for word in WORDS.iter().map(|(word, _)| *word).chain(taught) {
        if lower.contains(&format!(" {word} ")) || lower.contains(&format!(" {word},")) || lower.contains(&format!(" {word}.")) {
            hits.insert(word);
        }
    }

    hits.len()
}

/// The language the person wrote in, or `None` when the message is English (then nothing needs briefing).
pub fn detect(text: &str, glossary: &HashMap<String, String>) -> Option<&'static str> {
    if let Some(language) = script_language(text) {
        return Some(language);
    }

    /* One word common to both languages is a coincidence; two in one message is how these are written - or one in
       a short message ("kaj ta koro" is three words and plainly not English). */
    let hits = known_words(text, glossary);

    (hits >= 2 || hits == 1 && text.split_whitespace().count() <= 12).then_some("Banglish")
}

/// The person's vocabulary: what they taught SDC, with the built-in words underneath it.
pub fn glossary(root: &Path) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = WORDS.iter().map(|(word, english)| (word.to_string(), english.to_string())).collect();

    if let Ok(text) = std::fs::read_to_string(root.join(FILE)) {
        if let Ok(taught) = serde_json::from_str::<HashMap<String, String>>(&text) {
            map.extend(taught);
        }
    }

    map
}

/// Learns from the person's own glosses: `kaj (work)` or `kaj = work`. Answers how many pairs were learned.
pub fn learn(root: &Path, text: &str) -> usize {
    let mut taught: HashMap<String, String> = std::fs::read_to_string(root.join(FILE)).ok().and_then(|existing| serde_json::from_str(&existing).ok()).unwrap_or_default();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut learned = 0;
    let pair = |word: &str, english: &str, taught: &mut HashMap<String, String>| -> bool {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        let english: String = english.trim_matches(|c: char| !c.is_alphanumeric() && c != ' ').to_lowercase();

        if word.chars().count() >= 3 && !english.is_empty() && english.chars().count() <= 24 && english.chars().all(|c| c.is_alphanumeric() || c == ' ') && !english.contains(&word) {
            return taught.insert(word, english).is_none();
        }

        false
    };

    for (index, token) in tokens.iter().enumerate() {
        let trimmed = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '(' && c != ')' && c != '=');
        let learned_pair = if trimmed == "=" {
            (index > 0 && index + 1 < tokens.len()).then(|| (tokens[index - 1], tokens[index + 1])).is_some_and(|(word, english)| pair(word, english, &mut taught))
        } else if let Some(rest) = trimmed.strip_prefix('(') {
            /* `kaj (work)`: the word is the token before the gloss. */
            (index > 0).then(|| (tokens[index - 1], rest.trim_end_matches([')', ',', '.']))).is_some_and(|(word, english)| pair(word, english, &mut taught))
        } else if let Some((word, english)) = token.split_once('(') {
            pair(word, english, &mut taught)
        } else {
            false
        };

        if learned_pair {
            learned += 1;
        }
    }

    if learned > 0 {
        if let Some(parent) = root.join(FILE).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let _ = std::fs::write(root.join(FILE), serde_json::to_string_pretty(&taught).unwrap_or_default() + "\n");
    }

    learned
}

/// One token of the message as the brief writes it: the person's technical words stay exactly as they are.
fn token(word: &str, glossary: &HashMap<String, String>) -> String {
    let plain = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '/' && c != '-');

    if plain.is_empty() {
        return word.to_string();
    }

    /* A path, an identifier, an option or something in a version: the person's own word, never translated. */
    let technical = plain.contains('.') || plain.contains('/') || plain.contains('_') || plain.contains('-') || plain.chars().any(|c| c.is_ascii_digit()) || plain.to_lowercase().starts_with("--") || word.chars().any(|c| c.is_ascii_uppercase());

    if technical {
        return word.to_string();
    }

    let lower = plain.to_lowercase();

    if let Some(english) = glossary.get(&lower) {
        return word.replacen(plain, english, 1);
    }

    /* Not a known word: keep it, so nothing is lost - the model reads the person's own word beside the English. */
    word.to_string()
}

/// The English brief of a message: the same instructions, with the person's words translated and their technical
/// words untouched.
pub fn brief(text: &str, language: &str, glossary: &HashMap<String, String>) -> String {
    let translated: Vec<String> = text.split_whitespace().map(|word| token(word, glossary)).collect();
    let translated = translated.join(" ");

    if translated == text.trim() {
        return format!("[English brief: the message is already in English apart from a word or two. The person wrote in {language} - answer them in {language}. The person's own words, unchanged:]\n{text}");
    }

    format!(
        "[English brief of the person's message: SDC works in English, the person wrote in {language}. Every path, name and option below is exactly theirs; the rest is translated from their words. Answer the person in {language}.]\n{translated}\n[The person's own words, for anything the translation got wrong:]\n{}",
        text.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_language_is_told_from_english() {
        let empty = HashMap::new();

        assert_eq!(detect("kaj ta thik koro", &empty), Some("Banglish"));
        assert_eq!(detect("ফাইলটা ঠিক করো", &empty), Some("Bangla"));
        assert_eq!(detect("rename the total column in the sales report", &empty), None);
        assert_eq!(detect("fix the build", &empty), None);
    }

    #[test]
    fn the_brief_translates_the_learned_words_and_keeps_the_technical_ones() {
        let root = std::env::temp_dir().join(format!("sdc-interlingua-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        assert_eq!(learn(&root, "kaj (work) ta amake dekhao"), 1);

        let glossary = glossary(&root);

        assert_eq!(glossary["kaj"], "work");

        let briefed = brief("kaj ta koro src/orders.ts e --verbose diye", "Banglish", &glossary);

        assert!(briefed.contains("work"), "{briefed}");
        assert!(briefed.contains("src/orders.ts") && briefed.contains("--verbose"), "the person's own technical words are untouched:\n{briefed}");
        assert!(briefed.contains("Answer the person in Banglish") && briefed.contains("The person's own words"));
        assert_eq!(learn(&root, "nothing to learn here"), 0);

        let _ = std::fs::remove_dir_all(&root);
    }
}
