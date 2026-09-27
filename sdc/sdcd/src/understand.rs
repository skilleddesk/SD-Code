//! Reading a message the way a person meant it (0.11.8).
//!
//! The report: *"ami jevabe tmake sms kortasi ai vabe sms korle jano SDC bujte pare and promt make kore
//! automatic"*. People write to SDC the way they text - romanized Bengali, Bengali script, Hindi, a mix
//! with English, typos and all - and the engines behind SDC can read every one of those. What they were
//! never told is **how** to read it: that "kinah" is *whether*, that three requests are hiding in one
//! run-on line, and that the answer should come back in the person's own language.
//!
//! This module is that step, in one place for every engine:
//!
//! * [`Reading::of`] - which language and script a message is in. Scripts are counted; romanized
//!   Bengali and Hindi are recognised by their common words, which is what tells *Banglish* from English
//!   written in the same alphabet.
//! * [`shape`] - the prompt the engine actually receives: a short, fixed brief on how to read the message
//!   (find every request, read past the spelling, restate it, answer in the person's language, keep code
//!   as code), then the message **verbatim**. The brief asks for one first line, `Understood: …`, which
//!   the window draws as its own card - so the person sees at a glance whether SDC got it right, before
//!   any work is read.
//!
//! What is stored and replayed is the person's own text; only the turn in flight carries the brief.

use serde_json::{json, Value};

/// The marker the brief asks the answer to start with. The window looks for exactly this.
pub const UNDERSTOOD: &str = "Understood:";

/// What a message is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// BCP-47-ish: `bn-Latn` (Banglish), `bn`, `hi-Latn`, `hi`, `ar`, `en`, `other`.
    pub code: &'static str,
    /// What the window calls it: `Banglish`, `Bengali`, …
    pub label: &'static str,
    /// The language the answer should be written in, as the brief names it to the engine.
    pub reply_in: &'static str,
    /// The same, as the person would write it - the window's chip says `→ বাংলা`.
    pub reply_label: &'static str,
}

/// Common romanized-Bengali words - chosen so that none of them is an everyday English word.
const BANGLISH: &[&str] = &[
    "ami", "amake", "amar", "amra", "amader", "tumi", "tomar", "tomake", "tmake", "tmi", "apni", "apnar", "apnake",
    "kivabe", "kibhabe", "keno", "kothay", "kothaw", "kothao", "kobe", "koto", "koro", "kore", "korte", "korbo",
    "korben", "korsi", "korchi", "kortesi", "kortasi", "korse", "korche", "korle", "korlam", "koren", "hobe", "hoy",
    "hoi", "hoye", "hosse", "hocche", "hoyese", "hoyeche", "hoyesekinah", "hoise", "holo", "ache", "ase", "ashe",
    "nai", "nei", "nah", "kinah", "kina", "dakho", "dekho", "dekhao", "dekhte", "bujte", "bujhte", "buje", "bujhe",
    "bujhi", "jeno", "jano", "jevabe", "jeta", "jekono", "jodi", "sob", "shob", "kisu", "kichu", "aro", "onk",
    "onek", "aikhane", "ekhane", "oikhane", "okhane", "sekhane", "abar", "tahole", "kintu", "ebong", "ucit",
    "uchit", "lagbe", "lage", "parbe", "pari", "dorkar", "thakle", "thake", "likhle", "likhte", "bolo", "bolte",
    "diye", "dite", "nije", "nijei", "nijai", "tader", "oder", "eta", "ota", "sheta", "seta", "aita", "oita",
    "shathe", "songge", "sathe", "druto", "maje", "jonno", "jnno", "theke", "niye", "gese", "geche", "gelo",
    "chai", "cai", "caile", "chaile", "dile", "kaj", "kaje", "valo", "bhalo", "thikmoto", "somossa", "shomossha",
    "akdom", "akdomi", "ekdom", "ekhon", "akhon", "tarpor", "bole", "boli", "mone", "koira",
    "koiro", "korio", "hoile", "jabe", "jay", "jasse", "jacche", "vabe", "bhabe",
];

/// Common romanized-Hindi words, for the same reason.
const HINGLISH: &[&str] = &[
    "kya", "hai", "hain", "karo", "karna", "karke", "karta", "karti", "nahi", "nahin", "mujhe", "mera", "meri",
    "mere", "tum", "tumhe", "aap", "aapko", "kaise", "kyun", "kyon", "yeh", "woh", "bhai", "chahiye", "hoga",
    "raha", "rahi", "rahe", "kuch", "sab", "abhi", "bohot", "bahut", "accha", "acha", "theek", "thik", "kaha",
    "kahan", "batao", "bata", "dekh", "sakte", "sakta", "wala", "wali", "jaldi", "matlab",
];

impl Reading {
    /// Which language `text` is in. Script first (any non-Latin script that makes up a good share of the
    /// letters decides it); then, for Latin text, the romanized word lists.
    pub fn of(text: &str) -> Self {
        let (mut latin, mut bengali, mut devanagari, mut arabic, mut other) = (0usize, 0, 0, 0, 0);

        for c in text.chars().filter(|c| c.is_alphabetic()) {
            match c as u32 {
                0x0041..=0x024F => latin += 1,
                0x0980..=0x09FF => bengali += 1,
                0x0900..=0x097F => devanagari += 1,
                0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => arabic += 1,
                _ => other += 1,
            }
        }

        let letters = (latin + bengali + devanagari + arabic + other).max(1);
        let share = |count: usize| count * 100 / letters;

        if share(bengali) >= 20 {
            return Self::BENGALI;
        }

        if share(devanagari) >= 20 {
            return Self::HINDI;
        }

        if share(arabic) >= 20 {
            return Self::ARABIC;
        }

        if share(other) >= 20 {
            return Self::OTHER;
        }

        let words: Vec<String> = text
            .split(|c: char| !c.is_ascii_alphabetic())
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        let count = |list: &[&str]| words.iter().filter(|word| list.contains(&word.as_str())).count();
        let (bangla, hindi) = (count(BANGLISH), count(HINGLISH));
        let enough = |hits: usize| hits >= 2 && hits * 100 / words.len().max(1) >= 12;

        if bangla >= hindi && enough(bangla) {
            return Self::BANGLISH;
        }

        if enough(hindi) {
            return Self::HINGLISH;
        }

        Self::ENGLISH
    }

    pub const BANGLISH: Self = Self { code: "bn-Latn", label: "Banglish", reply_in: "Bengali, written in Bengali script", reply_label: "বাংলা" };
    pub const BENGALI: Self = Self { code: "bn", label: "Bengali", reply_in: "Bengali, written in Bengali script", reply_label: "বাংলা" };
    pub const HINGLISH: Self = Self { code: "hi-Latn", label: "Hinglish", reply_in: "Hindi, written in Devanagari script", reply_label: "हिन्दी" };
    pub const HINDI: Self = Self { code: "hi", label: "Hindi", reply_in: "Hindi, written in Devanagari script", reply_label: "हिन्दी" };
    pub const ARABIC: Self = Self { code: "ar", label: "Arabic script", reply_in: "the same language the person wrote in (Arabic, Urdu, Persian…), in its own script", reply_label: "same language" };
    pub const OTHER: Self = Self { code: "other", label: "their language", reply_in: "the same language the person wrote in", reply_label: "same language" };
    pub const ENGLISH: Self = Self { code: "en", label: "English", reply_in: "the language the person wrote in", reply_label: "English" };

    /// The chip the window shows on the person's message.
    pub fn to_json(&self) -> Value {
        json!({ "code": self.code, "label": self.label, "reply": self.reply_label })
    }
}

/// Is this message worth a brief? A slash command or an empty line is passed through as typed, and so is
/// a short English line ("hi", "run the tests") - a restatement of four words is noise, not help.
pub fn wants_brief(text: &str, reading: &Reading) -> bool {
    let trimmed = text.trim();

    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    reading.code != "en" || trimmed.split_whitespace().count() >= 12
}

/// The prompt the engine receives for `text`: the reading brief, then the message verbatim.
pub fn shape(text: &str, reading: &Reading) -> String {
    if !wants_brief(text, reading) {
        return text.to_string();
    }

    let written = match reading.code {
        "bn-Latn" => "in Banglish - Bengali typed with English letters, phonetically, often mixed with English words",
        "bn" => "in Bengali",
        "hi-Latn" => "in Hinglish - Hindi typed with English letters, often mixed with English words",
        "hi" => "in Hindi",
        "en" => "in English",
        _ => "in their own language",
    };

    format!(
        "[How to read this message - a note from SDC, not from the person]\n\
         The person wrote {written}. People write here the way they text: quickly, with typos, phonetic spelling, \
         missing punctuation and several requests run together.\n\
         1. First work out what they actually want. Read past the spelling, and find every separate request.\n\
         2. Start your answer with exactly one line: `{UNDERSTOOD} ` followed by their request restated clearly and \
         completely in {reply}, spoken to them directly as \"you\" (\"you want…\", never \"the user wants…\"; one or two \
         sentences; list the parts if there are several). Then a blank line.\n\
         3. Then do the work. When there are several parts, handle each one and say what was done for each.\n\
         4. Write the whole answer in {reply}. Keep code, commands, file paths, names and error messages exactly as they are.\n\
         5. If a wrong guess would be costly (deleting, deploying, spending money), ask one short question instead of guessing.\n\
         \n\
         [The person's message]\n\
         {text}",
        reply = reading.reply_in,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reports_own_messages_read_as_banglish() {
        for message in [
            "ar kothaw kono issue ashe kinah. and solve hoyesekinah amake double check kore dakho",
            "Jekono language a likle ai jeno seta buje promt nijai make kore kaj korte pare",
            "image dakho maje maje i mean onk druto ai vabe disconnect hoye jasse",
        ] {
            assert_eq!(Reading::of(message), Reading::BANGLISH, "{message}");
        }
    }

    #[test]
    fn scripts_and_english_are_told_apart() {
        assert_eq!(Reading::of("আমার সাইট কেন লোড হচ্ছে না?"), Reading::BENGALI);
        assert_eq!(Reading::of("मेरा सर्वर क्यों बंद है?"), Reading::HINDI);
        assert_eq!(Reading::of("mujhe batao yeh error kya hai bhai"), Reading::HINGLISH);
        assert_eq!(Reading::of("لماذا لا يعمل الخادم؟"), Reading::ARABIC);
        assert_eq!(Reading::of("Please fix the failing tests in the auth module and push"), Reading::ENGLISH);
        /* Code in a Bengali message does not make it English. */
        assert_eq!(Reading::of("`npm run build` চালাও আর error ঠিক করো"), Reading::BENGALI);
    }

    #[test]
    fn the_brief_keeps_the_message_verbatim_and_asks_for_the_understood_line() {
        let message = "ar kothaw kono issue ashe kinah, dakho";
        let shaped = shape(message, &Reading::of(message));

        assert!(shaped.ends_with(message), "{shaped}");
        assert!(shaped.contains("Understood: "), "{shaped}");
        assert!(shaped.contains("Bengali, written in Bengali script"), "{shaped}");
    }

    #[test]
    fn short_english_and_commands_pass_through_untouched() {
        for message in ["hi", "run the tests", "/compact", "  "] {
            assert_eq!(shape(message, &Reading::of(message)), message);
        }
    }
}
