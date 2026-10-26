//! **N2-2 - language normalisation** (switch `nr.lang_spec`, off).
//!
//! A Bengali or Banglish prompt becomes a precise English spec **one unit at a time** (never the whole prompt in one go, which is where
//! meaning is lost). The original stays in the vault. Code, JSON, tables and quotes are not translated. After each unit is restated, a
//! back-check requires that every number, name, path and code identifier found in the original is also in the English line; a line
//! that lost one is flagged, not trusted.

use super::spec::{Kind, Unit};

/// Bengali digits to ASCII, so `৩.৫` and `3.5` are the same number.
pub fn ascii_digits(text: &str) -> String {
    text.chars().map(|c| if ('০'..='৯').contains(&c) { char::from(b'0' + (c as u32 - '০' as u32) as u8) } else { c }).collect()
}

fn has_bengali(text: &str) -> bool {
    text.chars().any(|c| ('\u{0980}'..='\u{09FF}').contains(&c))
}

fn trim_punct(token: &str) -> &str {
    let mut t = token.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '!' | '?' | '।' | '"' | '\'' | '[' | ']' | '{' | '}' | '“' | '”' | '‘' | '’' | '`' | '<' | '>'));

    /* a bracket that wraps the word is dropped; the `()` of `total()` is the word's own */
    if t.starts_with('(') {
        t = t.trim_start_matches('(');
        t = if t.contains('(') { t } else { t.trim_end_matches(')') };
    } else if t.ends_with(')') && !t.contains('(') {
        t = t.trim_end_matches(')');
    }

    t.trim_end_matches('.')
}

fn is_identifier_like(token: &str) -> bool {
    let ascii = token.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '\\' | '-' | ':' | '(' | ')' | '@' | '#' | '$' | '=' | '*'));

    if !ascii || token.is_empty() {
        return false;
    }

    let digit = token.chars().any(|c| c.is_ascii_digit());
    let separator = token.contains('/') || token.contains('\\') || token.contains("::") || token.contains('_') || token.starts_with("--") || token.starts_with('@') || token.starts_with('$') || token.contains('=');
    let call = token.ends_with("()") || token.contains("(");
    let dotted = token.split('.').count() > 1 && token.split('.').all(|part| !part.is_empty());
    let camel = token.chars().skip(1).any(|c| c.is_ascii_uppercase()) && token.chars().any(|c| c.is_ascii_lowercase());

    digit || separator || call || dotted || camel
}

/// The things in a text that must survive a restatement: numbers, paths, code identifiers, backticked spans, quoted strings and - in text with
/// Bengali script in it - capitalised Latin names. Normalised (Bengali digits become ASCII) and de-duplicated, in order of appearance.
pub fn anchors(original: &str) -> Vec<String> {
    let text = ascii_digits(original);
    let mixed = has_bengali(original);
    let mut found: Vec<String> = Vec::new();
    let mut add = |value: String| {
        if !value.is_empty() && !found.contains(&value) {
            found.push(value);
        }
    };

    /* backticked spans and "quoted strings" are kept exactly */
    let mut rest = text.as_str();

    while let Some(open) = rest.find('`') {
        let Some(close) = rest[open + 1..].find('`') else { break };

        add(rest[open + 1..open + 1 + close].to_string());
        rest = &rest[open + close + 2..];
    }

    let mut quoted = text.as_str();

    while let Some(open) = quoted.find('"') {
        let Some(close) = quoted[open + 1..].find('"') else { break };
        let inside = &quoted[open + 1..open + 1 + close];

        if !inside.is_empty() && inside.len() <= 80 {
            add(inside.to_string());
        }

        quoted = &quoted[open + close + 2..];
    }

    let mut first_latin = true;

    for raw in text.split_whitespace() {
        let token = trim_punct(raw);

        if token.is_empty() {
            continue;
        }

        let starts_alpha_upper = token.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        let latin = token.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));

        /* a path, a number, a code identifier - or, in text with Bengali script in it, a capitalised Latin word that is not the first Latin word (a name) */
        let a_name = mixed && latin && starts_alpha_upper && !first_latin && token.len() > 1;

        if is_identifier_like(token) || a_name || token.chars().all(|c| c.is_ascii_digit()) {
            add(token.to_string());
        }

        if token.chars().any(|c| c.is_ascii_alphabetic()) {
            first_latin = false;
        }
    }

    found
}

/// The anchors of `original` that `english` lost.
pub fn missing(original: &str, english: &str) -> Vec<String> {
    let english = ascii_digits(english);

    anchors(original).into_iter().filter(|anchor| !english.contains(anchor.as_str())).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Line {
    pub unit: String,
    pub original: String,
    pub english: String,
    /// Anchors of the original that the English line lost; empty means the back-check passed.
    pub missing: Vec<String>,
    /// Restated by the model (false for a unit that is passed through untouched: code, JSON, tables, quotes, text already in English).
    pub translated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Report {
    pub lines: Vec<Line>,
}

impl Report {
    pub fn mismatches(&self) -> Vec<&Line> {
        self.lines.iter().filter(|line| !line.missing.is_empty()).collect()
    }

    pub fn passed(&self) -> bool {
        self.mismatches().is_empty()
    }

    /// The English spec, one line per unit in order.
    pub fn english(&self) -> String {
        self.lines.iter().map(|line| line.english.as_str()).collect::<Vec<_>>().join("\n")
    }
}

/// Does a unit need restating: prose that has Bengali script in it, or Banglish.
pub fn needs_restating(unit: &Unit) -> bool {
    if unit.kind.is_block() {
        return false;
    }

    has_bengali(&unit.text) || looks_banglish(&unit.text)
}

const BANGLISH: &[&str] = &["korbe", "korte", "koro", "kora", "chai", "dekhao", "banao", "jodi", "tahole", "nahole", "na", "ache", "nai", "hobe", "hoy", "ekta", "amar", "tomar", "ki", "kothay", "dao", "daw", "kore", "theke", "shob", "sob"];

fn looks_banglish(text: &str) -> bool {
    let lower = text.to_lowercase();
    let hits = lower.split(|c: char| !c.is_alphabetic()).filter(|word| BANGLISH.contains(word)).count();

    hits >= 2
}

/// Restates each unit that needs it through `translate` (the local model), passes the rest through, and back-checks every line. With the
/// switch off nothing is restated: the original stays the spec.
pub fn restate(units: &[Unit], translate: &mut dyn FnMut(&Unit) -> String) -> Report {
    let on = crate::v3::on("nr.lang_spec");
    let lines = units
        .iter()
        .map(|unit| {
            if !on || !needs_restating(unit) || matches!(unit.kind, Kind::Code | Kind::Json | Kind::Table | Kind::Quote) {
                return Line { unit: unit.id.clone(), original: unit.text.clone(), english: unit.text.clone(), missing: Vec::new(), translated: false };
            }

            let english = translate(unit).trim().to_string();
            let missing = missing(&unit.text, &english);

            Line { unit: unit.id.clone(), original: unit.text.clone(), english, missing, translated: true }
        })
        .collect();

    Report { lines }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navaratna::spec::parse;

    #[test]
    fn numbers_paths_identifiers_and_names_are_the_anchors() {
        let anchors = anchors("shop.py ফাইলে total() ফাংশনে ৩.৫ শতাংশ কর যোগ করো, `tax_rate` আর Rahim এর জন্য। পথ src/a/b.rs, --force দিয়ে, \"out of stock\" দেখাও।");

        for expected in ["shop.py", "total()", "3.5", "tax_rate", "Rahim", "src/a/b.rs", "--force", "out of stock"] {
            assert!(anchors.contains(&expected.to_string()), "{expected} missing from {anchors:?}");
        }
    }

    #[test]
    fn plain_words_are_not_anchors() {
        assert!(anchors("এই কাজটা খুব সহজ করো").is_empty());
        assert!(anchors("Please make it simple").is_empty());
    }

    #[test]
    fn a_good_restatement_passes_and_a_lossy_one_is_flagged() {
        let units = parse("shop.py ফাইলে total() ফাংশনটা ঠিক করো, ৩.৫ শতাংশ কর ধরো।\n\nপ্রতি অর্ডারে ১০ টাকা ছাড় দাও।");
        /* 3.2: the switch is on by default now, so the untouched case is asked for rather than assumed. */
        let report = crate::v3::ablating(&["nr.lang_spec"], || {
            restate(&units, &mut |unit: &Unit| {
                if unit.text.contains("shop.py") {
                    "In shop.py, fix the function total(), using a tax of 3.5 percent.".to_string()
                } else {
                    "Give a discount on every order.".to_string()
                }
            })
        });

        crate::v3::forcing(&["nr.lang_spec"], || {
            let report = restate(&units, &mut |unit: &Unit| if unit.text.contains("shop.py") { "In shop.py, fix the function total(), using a tax of 3.5 percent.".to_string() } else { "Give a discount on every order.".to_string() });

            assert!(!report.passed());
            assert_eq!(report.mismatches().len(), 1);
            assert_eq!(report.mismatches()[0].missing, vec!["10".to_string()], "the lost number is named");
            assert!(report.lines[0].missing.is_empty());
        });

        assert!(report.lines.iter().all(|line| !line.translated), "switched off, the original stays");
    }

    #[test]
    fn the_back_check_passes_on_all_fixtures() {
        crate::v3::forcing(&["nr.lang_spec"], || {
            let fixtures = [
                ("config.toml এ port = 8080 সেট করো", "Set port = 8080 in config.toml"),
                ("`UserService.get_user()` ফাংশনে cache যোগ করো, মেয়াদ ৬০ সেকেন্ড", "Add a cache to `UserService.get_user()` with an expiry of 60 seconds"),
                ("amar app e login page ta banao, /login route e", "Create the login page in my app, on the /login route"),
            ];

            for (original, english) in fixtures {
                let units = parse(original);
                let report = restate(&units, &mut |_| english.to_string());

                assert!(report.passed(), "{original}: {:?}", report.mismatches());
                assert!(report.lines.iter().all(|line| line.translated));
            }
        });
    }

    #[test]
    fn code_blocks_and_english_prose_are_never_sent_to_the_model() {
        crate::v3::forcing(&["nr.lang_spec"], || {
            let units = parse("Add a button to the header.\n\n```js\nconst মান = 1;\n```\n");
            let mut calls = 0;
            let report = restate(&units, &mut |_| {
                calls += 1;
                String::new()
            });

            assert_eq!(calls, 0);
            assert_eq!(report.lines[1].english, units[1].text, "code is not translated");
        });
    }
}
