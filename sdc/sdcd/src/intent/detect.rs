//! Which language, dialect and script a message is in (the Intent Engine's first step).
//!
//! There is no list of supported languages: any script is recognised by its Unicode block, and any
//! language a model reads is read by the model. What this module adds is the part a model is not asked
//! for - a fast, offline first look that names the script, notices romanized writing (Banglish, Hinglish,
//! Arabizi, Romanized Urdu), mixed writing (Taglish, Spanglish) and the regional forms whose words are
//! distinctive enough to spot (Sylheti, Chittagonian, Noakhali, Bhojpuri, Egyptian / Gulf / Levantine /
//! Maghrebi Arabic, Swiss German). The model's reading of the message then confirms or corrects it, and
//! the person's own correction of the card is the last word.
//!
//! Every answer carries a confidence, because a guess that says it is a guess is honest (P4).

use serde_json::{json, Value};

/// What a message is written in.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    /// BCP-47-ish language code: `bn`, `hi`, `ar`, `es`, `ur`, `tl`, `en`, `und` …
    pub code: String,
    /// The regional form when one is recognised: `sylheti`, `chittagonian`, `egyptian` …
    pub dialect: Option<String>,
    /// ISO 15924 script: `Beng`, `Deva`, `Arab`, `Latn`, `Hans`, `Jpan`, `Kore`, `Cyrl` …
    pub script: String,
    /// Written in Latin letters although the language has its own script (Banglish, Hinglish, Arabizi).
    pub romanized: bool,
    /// Two languages mixed in one message (Taglish, Spanglish, Banglish with English terms).
    pub mixed: bool,
    /// 0.0 - 1.0: how sure the offline look is.
    pub confidence: f64,
    /// What the window's chip says: `Sylheti (Bengali script)`, `Banglish`, `Arabizi` …
    pub label: String,
    /// The language the answer should be in, as the engine is told.
    pub reply_in: String,
    /// The same, as the person would name it: `বাংলা`, `हिन्दी`, `العربية` …
    pub reply_label: String,
}

impl Detection {
    pub fn to_json(&self) -> Value {
        json!({
            "code": self.code,
            "dialect": self.dialect,
            "script": self.script,
            "romanized": self.romanized,
            "mixed": self.mixed,
            "confidence": (self.confidence * 100.0).round() / 100.0,
            "label": self.label,
            "replyIn": self.reply_in,
            "reply": self.reply_label,
        })
    }
}

/// Scripts by Unicode block: `(script, first, last)`.
const SCRIPTS: &[(&str, u32, u32)] = &[
    ("Beng", 0x0980, 0x09FF),
    ("Deva", 0x0900, 0x097F),
    ("Arab", 0x0600, 0x06FF),
    ("Arab", 0x0750, 0x077F),
    ("Arab", 0xFB50, 0xFDFF),
    ("Arab", 0xFE70, 0xFEFF),
    ("Guru", 0x0A00, 0x0A7F),
    ("Gujr", 0x0A80, 0x0AFF),
    ("Orya", 0x0B00, 0x0B7F),
    ("Taml", 0x0B80, 0x0BFF),
    ("Telu", 0x0C00, 0x0C7F),
    ("Knda", 0x0C80, 0x0CFF),
    ("Mlym", 0x0D00, 0x0D7F),
    ("Sinh", 0x0D80, 0x0DFF),
    ("Thai", 0x0E00, 0x0E7F),
    ("Laoo", 0x0E80, 0x0EFF),
    ("Tibt", 0x0F00, 0x0FFF),
    ("Mymr", 0x1000, 0x109F),
    ("Geor", 0x10A0, 0x10FF),
    ("Hang", 0x1100, 0x11FF),
    ("Ethi", 0x1200, 0x137F),
    ("Khmr", 0x1780, 0x17FF),
    ("Cyrl", 0x0400, 0x04FF),
    ("Grek", 0x0370, 0x03FF),
    ("Armn", 0x0530, 0x058F),
    ("Hebr", 0x0590, 0x05FF),
    ("Kana", 0x3040, 0x30FF),
    ("Hani", 0x4E00, 0x9FFF),
    ("Hani", 0x3400, 0x4DBF),
    ("Hang", 0xAC00, 0xD7AF),
    ("Latn", 0x0041, 0x024F),
    ("Latn", 0x1E00, 0x1EFF),
];

fn script_of(character: char) -> Option<&'static str> {
    let code = character as u32;

    SCRIPTS.iter().find(|(_, first, last)| (*first..=*last).contains(&code)).map(|(script, _, _)| *script)
}

/// A language's own name for itself, for the reply chip - and the name the engine is told.
fn names(code: &str) -> (&'static str, &'static str) {
    match code {
        "bn" => ("Bengali, written in Bengali script", "বাংলা"),
        "hi" => ("Hindi, written in Devanagari script", "हिन्दी"),
        "bho" => ("Bhojpuri, written in Devanagari script", "भोजपुरी"),
        "ur" => ("Urdu, written in Urdu (Arabic) script", "اردو"),
        "ar" => ("Arabic, in Arabic script", "العربية"),
        "fa" => ("Persian, in its own script", "فارسی"),
        "pa" => ("Punjabi, in Gurmukhi script", "ਪੰਜਾਬੀ"),
        "gu" => ("Gujarati, in Gujarati script", "ગુજરાતી"),
        "or" => ("Odia, in Odia script", "ଓଡ଼ିଆ"),
        "ta" => ("Tamil, in Tamil script", "தமிழ்"),
        "te" => ("Telugu, in Telugu script", "తెలుగు"),
        "kn" => ("Kannada, in Kannada script", "ಕನ್ನಡ"),
        "ml" => ("Malayalam, in Malayalam script", "മലയാളം"),
        "si" => ("Sinhala, in Sinhala script", "සිංහල"),
        "th" => ("Thai", "ไทย"),
        "lo" => ("Lao", "ລາວ"),
        "my" => ("Burmese", "မြန်မာ"),
        "km" => ("Khmer", "ខ្មែរ"),
        "ka" => ("Georgian", "ქართული"),
        "am" => ("Amharic, in Ethiopic script", "አማርኛ"),
        "ru" => ("the Cyrillic-script language the person wrote in (Russian, Ukrainian, …)", "русский"),
        "el" => ("Greek", "Ελληνικά"),
        "hy" => ("Armenian", "Հայերեն"),
        "he" => ("Hebrew", "עברית"),
        "ja" => ("Japanese", "日本語"),
        "zh" => ("Chinese", "中文"),
        "ko" => ("Korean", "한국어"),
        "es" => ("Spanish", "Español"),
        "pt" => ("Portuguese", "Português"),
        "fr" => ("French", "Français"),
        "de" => ("German", "Deutsch"),
        "gsw" => ("Swiss German (answer in standard German unless the person prefers the dialect)", "Schwiizerdütsch"),
        "it" => ("Italian", "Italiano"),
        "tr" => ("Turkish", "Türkçe"),
        "id" => ("Indonesian", "Bahasa Indonesia"),
        "ms" => ("Malay", "Bahasa Melayu"),
        "tl" => ("Filipino (Tagalog), mixed with English the way the person wrote", "Filipino"),
        "vi" => ("Vietnamese", "Tiếng Việt"),
        "sw" => ("Swahili", "Kiswahili"),
        "nl" => ("Dutch", "Nederlands"),
        "pl" => ("Polish", "Polski"),
        "en" => ("the language the person wrote in", "English"),
        _ => ("the same language the person wrote in", "same language"),
    }
}

/// The language an answer should be written in, as an engine is told, for a language code.
pub fn detect_label_reply(code: &str) -> String {
    names(code).0.to_string()
}

fn words_of(text: &str) -> Vec<String> {
    text.split(|character: char| !(character.is_alphanumeric() || character == '\''))
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn hits(words: &[String], list: &[&str]) -> usize {
    words.iter().filter(|word| list.contains(&word.as_str())).count()
}

/* ---- Word lists: common, distinctive, and never everyday English ------------------------------------ */

const BANGLISH: &[&str] = &[
    "ami", "amake", "amar", "amra", "amader", "tumi", "tomar", "tomake", "tmake", "tmi", "apni", "apnar", "apnake",
    "kivabe", "kibhabe", "keno", "kothay", "kothaw", "kothao", "kobe", "koto", "koro", "kore", "korte", "korbo",
    "korben", "korsi", "korchi", "kortesi", "kortasi", "korse", "korche", "korle", "korlam", "koren", "hobe", "hoy",
    "hoi", "hoye", "hosse", "hocche", "hoyese", "hoyeche", "hoise", "holo", "ache", "ase", "ashe", "nai", "nei", "nah",
    "kinah", "kina", "dakho", "dekho", "dekhao", "dekhte", "bujte", "bujhte", "buje", "bujhe", "jeno", "jano", "jevabe",
    "jeta", "jekono", "jodi", "sob", "shob", "kisu", "kichu", "aro", "onk", "onek", "ekhane", "oikhane", "abar",
    "tahole", "kintu", "ebong", "lagbe", "lage", "parbe", "pari", "dorkar", "thake", "bolo", "bolte", "diye", "dite",
    "nije", "nijei", "eta", "ota", "seta", "aita", "oita", "sathe", "maje", "jonno", "theke", "niye", "gese", "geche",
    "chai", "caile", "chaile", "dile", "kaj", "kaje", "valo", "bhalo", "thik", "somossa", "ekdom", "ekhon", "akhon",
    "tarpor", "bole", "mone", "jabe", "jasse", "jacche", "vabe", "dao", "daw", "kortese", "koro", "hoche",
    /* 0.12, from the live check: short everyday words a romanized request is built from. */
    "ei", "oi", "er", "ekta", "ekti", "likho", "lekho", "likhe", "ar", "jog", "korun", "kori", "dilam", "ache", "gula", "gulo",
    "kichui", "thik", "banao", "banai", "dekhao", "chalao", "khulo", "moto", "shudhu", "sudhu",
];

const HINGLISH: &[&str] = &[
    "kya", "hai", "hain", "karo", "karna", "karke", "karta", "karti", "nahi", "nahin", "mujhe", "mera", "meri", "mere",
    "tum", "tumhe", "aap", "aapko", "kaise", "kyun", "kyon", "yeh", "woh", "bhai", "chahiye", "hoga", "raha", "rahi",
    "rahe", "kuch", "sab", "abhi", "bohot", "bahut", "accha", "acha", "theek", "kaha", "kahan", "batao", "bata", "dekh",
    "sakte", "sakta", "wala", "wali", "jaldi", "matlab", "ka", "ki", "ko", "se", "par",
];

/// Words Romanized Urdu uses that Hinglish writers mostly do not.
const ROMAN_URDU: &[&str] = &["kyun", "bohat", "hum", "ap", "apka", "shukriya", "zaroor", "masla", "theek", "kr", "rha", "rhi", "hy", "ha", "nhi", "mjhe", "krna", "krdo", "kardo"];

/// Arabizi: Arabic in Latin letters, digits for sounds Latin lacks (3 = ع, 7 = ح, 2 = ء, 5 = خ, 8 = غ, 9 = ق).
const ARABIZI: &[&str] = &[
    "mesh", "msh", "ana", "enta", "enti", "howa", "heya", "ezay", "izay", "leh", "keda", "kda", "dah", "di", "3ayez",
    "3ayz", "3awez", "sha8al", "shaghal", "sale7", "sal7", "7aga", "7elw", "ya3ni", "yalla", "inshallah", "habibi",
    "fe", "fi", "mafish", "mish", "ba2a", "2ool", "shway", "shwaya", "wallah", "bas", "kaman", "7abibi", "3ala",
];

const TAGALOG: &[&str] = &["ang", "ng", "mga", "po", "naman", "yung", "ba", "sige", "hindi", "gawin", "ayusin", "paki", "pakiayos", "gumagana", "yan", "ito", "sa", "ko", "mo", "na", "lang", "talaga"];

const SPANISH: &[&str] = &["el", "la", "los", "las", "de", "que", "no", "funciona", "arreglalo", "arréglalo", "formulario", "por", "favor", "para", "con", "una", "este", "esta", "sitio", "página", "pagina", "hacer", "quiero", "necesito", "está"];
const PORTUGUESE: &[&str] = &["não", "nao", "funciona", "conserta", "está", "esta", "você", "voce", "fazer", "preciso", "quero", "formulário", "página", "obrigado", "então", "isso", "uma", "um"];
const FRENCH: &[&str] = &["le", "la", "les", "est", "ne", "pas", "fonctionne", "marche", "répare", "repare", "formulaire", "merci", "je", "veux", "avec", "pour", "une", "ce", "site", "s'il", "vous", "plaît"];
const GERMAN: &[&str] = &["der", "die", "das", "ist", "nicht", "funktioniert", "bitte", "reparieren", "formular", "und", "ich", "möchte", "kannst", "du", "mit", "ein", "eine"];
const SWISS: &[&str] = &["isch", "nöd", "nid", "chunnt", "gsi", "öppis", "mached", "mach", "bitte", "funktioniert", "gaht", "gohts", "chasch", "hüt", "eifach", "wämmer", "de", "d"];
const INDONESIAN: &[&str] = &["yang", "tidak", "bisa", "tolong", "perbaiki", "formulir", "kontak", "ini", "itu", "saya", "dengan", "untuk", "ada", "sudah", "belum", "gak", "nggak", "dong"];
const TURKISH: &[&str] = &["bu", "ve", "bir", "çalışmıyor", "calismiyor", "lütfen", "lutfen", "düzelt", "duzelt", "form", "iletişim", "sitenin", "değil", "için"];
const SWAHILI: &[&str] = &["hii", "haifanyi", "kazi", "tafadhali", "rekebisha", "fomu", "ya", "na", "kwa", "wa", "mawasiliano"];

/* ---- Regional forms ------------------------------------------------------------------------------ */

/// Sylheti in Bengali script: `করর` (does), `কিতা` (what), `খান` as a classifier, `অখন`, `বাইক্কা`.
const SYLHETI: &[&str] = &["করর", "কিতা", "খান", "অখন", "বাইক্কা", "কিলা", "ইতা", "হকল", "কইরা", "সাইটর", "ওউ", "মাতো", "যাইবায়", "আইবায়"];
/// Chittagonian: `গরি` (doing), `আঁই` (I), `তুঁই` (you), `গরঅ`, `ন` for not.
const CHITTAGONIAN: &[&str] = &["গরি", "আঁই", "তুঁই", "গরঅ", "গরো", "ইবা", "হন", "ক্যানে", "গরন", "লাগিবো", "যাইবো", "আঁরে"];
/// Noakhali: `কিল্লাই` (why), `আঁর` (my), `হেতে`, `কইত্তাম`.
const NOAKHALI: &[&str] = &["কিল্লাই", "আঁর", "হেতে", "কইত্তাম", "হইছে", "কইচ্ছে", "হেগুন", "কিয়ারে"];
/// Bhojpuri in Devanagari: `बा`, `हवे`, `रहल`, `कइसे`, `हमार`, `तोहार`, `करीं`.
const BHOJPURI: &[&str] = &["बा", "हवे", "रहल", "कइसे", "हमार", "तोहार", "करीं", "नइखे", "बाटे", "कहाँ", "गइल", "कर", "द"];
const EGYPTIAN: &[&str] = &["مش", "عايز", "عاوز", "ازاي", "إزاي", "كده", "دلوقتي", "بتاع", "بتاعة", "شغال", "إيه", "ايه", "ليه", "خالص", "صلحه", "صلّحه"];
const GULF: &[&str] = &["وايد", "شلون", "ابي", "أبي", "زين", "حق", "الحين", "يبي", "شنو", "ليش"];
const LEVANTINE: &[&str] = &["شو", "هلق", "بدي", "كتير", "منيح", "هيك", "ليش", "عم", "مشان", "هلأ"];
const MAGHREBI: &[&str] = &["بزاف", "واش", "ديال", "بغيت", "دابا", "كيفاش", "مزيان", "شنو", "علاش"];
/// Words that tell Urdu from Arabic in Arabic script.
const URDU: &[&str] = &["ہے", "کیا", "نہیں", "میں", "کر", "کو", "یہ", "وہ", "ٹھیک", "کریں", "ہیں", "کے", "کی", "سائٹ"];
const PERSIAN: &[&str] = &["است", "نمی", "کار", "کنید", "لطفا", "این", "درست", "نیست", "می"];

fn detection(code: &str, dialect: Option<&str>, script: &str, romanized: bool, mixed: bool, confidence: f64, label: String) -> Detection {
    let (reply_in, reply_label) = names(code);

    Detection {
        code: code.to_string(),
        dialect: dialect.map(str::to_string),
        script: script.to_string(),
        romanized,
        mixed,
        confidence: confidence.clamp(0.05, 0.99),
        label,
        reply_in: reply_in.to_string(),
        reply_label: reply_label.to_string(),
    }
}

/// The offline first look at a message.
pub fn detect(text: &str) -> Detection {
    let mut counts: std::collections::HashMap<&'static str, usize> = Default::default();
    let mut letters = 0usize;

    for character in text.chars().filter(|character| character.is_alphabetic()) {
        letters += 1;

        if let Some(script) = script_of(character) {
            *counts.entry(script).or_default() += 1;
        }
    }

    let words = words_of(text);
    let latin = counts.get("Latn").copied().unwrap_or(0);
    let dominant = counts
        .iter()
        .filter(|(script, _)| **script != "Latn")
        .max_by_key(|(_, count)| **count)
        .map(|(script, count)| (*script, *count));
    let share = |count: usize| count as f64 / letters.max(1) as f64;
    /* English words inside a non-Latin message make it mixed (code and file names excepted by the
       threshold: a message is mixed when a real share of its letters are Latin). */
    let mixed_latin = latin > 0 && share(latin) >= 0.25;

    if let Some((script, count)) = dominant.filter(|(_, count)| share(*count) >= 0.2) {
        let confidence = 0.6 + share(count) * 0.35;

        return match script {
            "Beng" => {
                let (dialect, label) = if hits(&words, SYLHETI) >= 1 && hits(&words, SYLHETI) >= hits(&words, CHITTAGONIAN) {
                    (Some("sylheti"), "Sylheti (Bengali script)")
                } else if hits(&words, CHITTAGONIAN) >= 1 {
                    (Some("chittagonian"), "Chittagonian (Bengali script)")
                } else if hits(&words, NOAKHALI) >= 1 {
                    (Some("noakhali"), "Noakhali (Bengali script)")
                } else {
                    (None, "Bengali")
                };

                detection("bn", dialect, "Beng", false, mixed_latin, confidence, label.to_string())
            }
            "Deva" => {
                if hits(&words, BHOJPURI) >= 2 {
                    detection("bho", Some("bhojpuri"), "Deva", false, mixed_latin, confidence * 0.9, "Bhojpuri (Devanagari)".to_string())
                } else {
                    detection("hi", None, "Deva", false, mixed_latin, confidence, "Hindi".to_string())
                }
            }
            "Arab" => {
                let (urdu, persian) = (hits(&words, URDU), hits(&words, PERSIAN));

                if urdu >= 2 && urdu > persian {
                    detection("ur", None, "Arab", false, mixed_latin, confidence, "Urdu".to_string())
                } else if persian >= 2 {
                    detection("fa", None, "Arab", false, mixed_latin, confidence * 0.9, "Persian".to_string())
                } else {
                    let regional = [
                        ("egyptian", "Egyptian Arabic", hits(&words, EGYPTIAN)),
                        ("gulf", "Gulf Arabic", hits(&words, GULF)),
                        ("levantine", "Levantine Arabic", hits(&words, LEVANTINE)),
                        ("maghrebi", "Maghrebi Arabic (Darija)", hits(&words, MAGHREBI)),
                    ];
                    let best = regional.iter().max_by_key(|(_, _, count)| *count).filter(|(_, _, count)| *count >= 1);

                    match best {
                        Some((dialect, label, _)) => detection("ar", Some(dialect), "Arab", false, mixed_latin, confidence * 0.9, label.to_string()),
                        None => detection("ar", None, "Arab", false, mixed_latin, confidence, "Arabic".to_string()),
                    }
                }
            }
            "Hani" if counts.get("Kana").copied().unwrap_or(0) > 0 => detection("ja", None, "Jpan", false, mixed_latin, confidence, "Japanese".to_string()),
            "Kana" => detection("ja", None, "Jpan", false, mixed_latin, confidence, "Japanese".to_string()),
            "Hani" => detection("zh", None, "Hans", false, mixed_latin, confidence, "Chinese".to_string()),
            "Hang" => detection("ko", None, "Kore", false, mixed_latin, confidence, "Korean".to_string()),
            other => {
                let code = match other {
                    "Guru" => "pa",
                    "Gujr" => "gu",
                    "Orya" => "or",
                    "Taml" => "ta",
                    "Telu" => "te",
                    "Knda" => "kn",
                    "Mlym" => "ml",
                    "Sinh" => "si",
                    "Thai" => "th",
                    "Laoo" => "lo",
                    "Mymr" => "my",
                    "Khmr" => "km",
                    "Geor" => "ka",
                    "Ethi" => "am",
                    "Cyrl" => "ru",
                    "Grek" => "el",
                    "Armn" => "hy",
                    "Hebr" => "he",
                    "Tibt" => "bo",
                    _ => "und",
                };
                let (_, native) = names(code);

                detection(code, None, other, false, mixed_latin, confidence, native.to_string())
            }
        };
    }

    /* Latin letters: romanized forms first (they borrow English words, so English last). */
    let total = words.len().max(1);
    let rate = |count: usize| count as f64 / total as f64;
    let arabizi_digits = words
        .iter()
        .filter(|word| word.chars().any(|c| c.is_ascii_alphabetic()) && word.chars().any(|c| matches!(c, '2' | '3' | '5' | '7' | '8' | '9')))
        .count();
    let candidates: Vec<(&str, Option<&str>, &str, bool, f64)> = vec![
        ("bn", None, "Banglish", true, rate(hits(&words, BANGLISH))),
        ("hi", None, "Hinglish", true, rate(hits(&words, HINGLISH)) * 0.95),
        ("ur", None, "Romanized Urdu", true, rate(hits(&words, ROMAN_URDU)) * 0.9 + if hits(&words, ROMAN_URDU) >= 2 { 0.02 } else { 0.0 }),
        ("ar", None, "Arabizi", true, rate(hits(&words, ARABIZI)) + rate(arabizi_digits) * 1.5),
        ("tl", None, "Taglish", false, rate(hits(&words, TAGALOG)) * 0.9),
        ("es", None, "Spanish", false, rate(hits(&words, SPANISH)) * 0.9),
        ("pt", None, "Portuguese", false, rate(hits(&words, PORTUGUESE)) * 0.9),
        ("fr", None, "French", false, rate(hits(&words, FRENCH)) * 0.85),
        ("gsw", Some("swiss"), "Swiss German", false, rate(hits(&words, SWISS)) * if hits(&words, &["isch", "nöd", "chunnt", "gsi", "öppis", "chasch", "gaht"]) > 0 { 1.2 } else { 0.3 }),
        ("de", None, "German", false, rate(hits(&words, GERMAN)) * 0.85),
        ("id", None, "Indonesian", false, rate(hits(&words, INDONESIAN)) * 0.9),
        ("tr", None, "Turkish", false, rate(hits(&words, TURKISH)) * 0.9),
        ("sw", None, "Swahili", false, rate(hits(&words, SWAHILI)) * 0.9),
    ];
    let best = candidates.iter().cloned().max_by(|a, b| a.4.partial_cmp(&b.4).unwrap_or(std::cmp::Ordering::Equal));

    if let Some((code, dialect, label, romanized, score)) = best {
        let raw_hits = (score * total as f64).round() as usize;

        if score >= 0.12 && raw_hits >= 2 {
            /* Vietnamese diacritics are a script of their own in all but name. */
            let english_words = hits(&words, &["the", "is", "and", "please", "fix", "form", "site", "not", "working", "contact", "error", "page"]);
            let mixed = english_words >= 2 || (label == "Taglish") || (code == "es" && english_words >= 1);
            let label = if code == "es" && english_words >= 2 { "Spanglish".to_string() } else { label.to_string() };

            return detection(code, dialect, "Latn", romanized, mixed, (0.45 + score).min(0.92), label);
        }
    }

    if text.chars().any(|c| "ăâđêôơưạảấầẩẫậắằẳẵặẹẻẽếềểễệỉịọỏốồổỗộớờởỡợụủứừửữựỳỵỷỹ".contains(c)) {
        return detection("vi", None, "Latn", false, false, 0.85, "Vietnamese".to_string());
    }

    if letters == 0 {
        return detection("und", None, "Zyyy", false, false, 0.1, "no words".to_string());
    }

    detection("en", None, "Latn", false, false, if total >= 4 { 0.7 } else { 0.45 }, "English".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plan's own examples - one task, five ways of writing it.
    #[test]
    fn the_plans_examples_are_read_as_their_languages() {
        let banglish = detect("ei site er contact form ta kaj kortese na, thik kore dao");
        let sylheti = detect("ই সাইটর contact form খান কাম করর না, বাইক্কা কইরা দাও");
        let hinglish = detect("is site ka contact form kaam nahi kar raha, fix karo");
        let arabizi = detect("el contact form mesh sha8al, sale7o");
        let spanish = detect("el formulario de contacto no funciona, arréglalo");

        assert_eq!((banglish.code.as_str(), banglish.romanized), ("bn", true), "{banglish:?}");
        assert_eq!((sylheti.code.as_str(), sylheti.dialect.as_deref()), ("bn", Some("sylheti")), "{sylheti:?}");
        assert_eq!((hinglish.code.as_str(), hinglish.romanized), ("hi", true), "{hinglish:?}");
        assert_eq!((arabizi.code.as_str(), arabizi.label.as_str()), ("ar", "Arabizi"), "{arabizi:?}");
        assert_eq!(spanish.code, "es", "{spanish:?}");
    }

    /// The golden set's offline half: thirty-odd ways of writing, each to its language (and dialect where
    /// the words give it away). The model's half is measured live by `_verify/intent-golden.mjs`.
    #[test]
    fn the_golden_set_reads_every_script_and_romanization() {
        let cases: &[(&str, &str, Option<&str>)] = &[
            ("আমার সাইটের কন্টাক্ট ফর্ম কাজ করছে না, ঠিক করে দাও", "bn", None),
            ("আঁই কইলাম ফর্মটা গরি দেও, কাম ন গরের", "bn", Some("chittagonian")),
            ("কিল্লাই ফর্মটা কাম করে না? আঁর সাইট ঠিক কইত্তাম", "bn", Some("noakhali")),
            ("मेरी साइट का कॉन्टैक्ट फॉर्म काम नहीं कर रहा, ठीक करो", "hi", None),
            ("हमार साइट के फॉर्म काम नइखे करत, ठीक कर द बा", "bho", Some("bhojpuri")),
            ("نموذج الاتصال في الموقع لا يعمل، أصلحه من فضلك", "ar", None),
            ("الفورم مش شغال خالص، صلحه دلوقتي", "ar", Some("egyptian")),
            ("الفورم ما يشتغل وايد مشاكل، ابي تصلحه الحين", "ar", Some("gulf")),
            ("شو القصة الفورم ما عم يشتغل، بدي تصلحه هلق", "ar", Some("levantine")),
            ("الفورم ما خدامش، بغيت تصلحو دابا بزاف مهم", "ar", Some("maghrebi")),
            ("میری سائٹ کا فارم کام نہیں کر رہا، ٹھیک کریں", "ur", None),
            ("mera form kaam nhi kr rha, isko theek kr do bohat zaroor", "ur", None),
            ("yung contact form ng site hindi gumagana, pakiayos naman po", "tl", None),
            ("o formulário de contato não funciona, conserta por favor", "pt", None),
            ("le formulaire de contact ne marche pas, répare-le s'il vous plaît", "fr", None),
            ("das Kontaktformular funktioniert nicht, bitte reparieren", "de", None),
            ("s Kontaktformular gaht nöd, chasch das bitte flicke? es isch dringend", "gsw", Some("swiss")),
            ("formulir kontak di situs ini tidak bisa, tolong perbaiki dong", "id", None),
            ("sitenin iletişim formu çalışmıyor, lütfen düzelt", "tr", None),
            ("fomu ya mawasiliano haifanyi kazi, tafadhali rekebisha", "sw", None),
            ("biểu mẫu liên hệ không hoạt động, sửa giúp tôi", "vi", None),
            ("サイトのお問い合わせフォームが動きません、直してください", "ja", None),
            ("网站的联系表单不工作，请修复", "zh", None),
            ("사이트 문의 양식이 작동하지 않아요, 고쳐 주세요", "ko", None),
            ("Контактная форма на сайте не работает, исправь", "ru", None),
            ("தளத்தின் தொடர்பு படிவம் வேலை செய்யவில்லை, சரி செய்யுங்கள்", "ta", None),
            ("సైట్ సంప్రదింపు ఫారం పనిచేయడం లేదు, సరిచేయండి", "te", None),
            ("ਸਾਈਟ ਦਾ ਫਾਰਮ ਕੰਮ ਨਹੀਂ ਕਰ ਰਿਹਾ, ਠੀਕ ਕਰੋ", "pa", None),
            ("แบบฟอร์มติดต่อในเว็บไม่ทำงาน ช่วยแก้ด้วย", "th", None),
            ("טופס יצירת הקשר באתר לא עובד, תקן בבקשה", "he", None),
            ("Η φόρμα επικοινωνίας δεν λειτουργεί, διόρθωσέ την", "el", None),
            ("the contact form on the site is not working, please fix it", "en", None),
            ("ami chai site er form ta thik hok, amake janao kivabe hobe", "bn", None),
            ("ei project er math file e ekta multiply function add koro ar test o likho", "bn", None),
            ("bhai mujhe batao yeh error kya hai aur kaise theek hoga", "hi", None),
        ];

        let mut wrong = Vec::new();

        for (text, code, dialect) in cases {
            let found = detect(text);

            if found.code != *code || (dialect.is_some() && found.dialect.as_deref() != *dialect) {
                wrong.push(format!("{text} → {} {:?} (wanted {code} {dialect:?})", found.code, found.dialect));
            }
        }

        assert!(cases.len() >= 30);
        assert!(wrong.is_empty(), "{} of {} misread:\n{}", wrong.len(), cases.len(), wrong.join("\n"));
    }

    #[test]
    fn a_short_line_is_english_with_low_confidence_and_nothing_is_undetermined() {
        assert_eq!(detect("hi").code, "en");
        assert!(detect("hi").confidence < 0.6);
        assert_eq!(detect("1234 !!").code, "und");
    }
}
