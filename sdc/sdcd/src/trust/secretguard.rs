//! Content-based secret guard (Phase 0.3).
//!
//! `secret_paths` protects files a person *named*. This module protects the files nobody thought to name:
//! when the agent reads a file, its content is scanned **before** anything goes to a model, and a value that
//! looks like a secret is masked and the person is asked first. Three signals:
//!
//! 1. known key shapes (`scan`'s rules: `sk-`, `ghp_`, `AIza`, private keys, database URLs ...),
//! 2. a line that assigns a value to `password`, `api_key`, `token` ... with or without quotes (`pass=hunter2`),
//! 3. a long, high-entropy token (random-looking, mixed letters and digits).
//!
//! A file *name* that says what it holds (`pass.txt`, `githubapi.rtf`, `service-token.json`) asks first as well,
//! when it is a data/config file; source code is judged by its content only, so `api.ts` does not nag.

use std::sync::OnceLock;

use regex::Regex;

/// One value found in a text: the line (1-based), what it is, and the byte range of the value itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub line: usize,
    pub what: String,
    pub start: usize,
    pub end: usize,
}

/// Words in a file name that mean it holds a secret.
const NAME_WORDS: &[&str] = &["api", "apikey", "key", "keys", "secret", "secrets", "token", "tokens", "pass", "passwd", "password", "passwords", "credential", "credentials", "creds"];

/// Extensions of files that hold data or settings (not source code) - where a name like `pass.txt` is a signal.
const DATA_EXTENSIONS: &[&str] = &["txt", "rtf", "md", "json", "toml", "yaml", "yml", "ini", "cfg", "conf", "csv", "env", "properties", "xml", "plist", "log", "pem", "key", "text", "doc", "docx", "pdf"];

/// The word-like parts of a file name: split on separators and on camelCase edges.
fn name_parts(stem: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;

    for c in stem.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }

            previous_lower = false;
            continue;
        }

        if c.is_uppercase() && previous_lower && !current.is_empty() {
            parts.push(std::mem::take(&mut current));
        }

        previous_lower = c.is_lowercase();
        current.extend(c.to_lowercase());
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// A trailing `api` or `key` glued to a longer word (`deepseakapi`, `TAVILY_API_KEY`) still counts.
fn glued(part: &str) -> bool {
    part.len() > 3 && ["api", "apikey", "secret", "token", "password", "credentials"].iter().any(|word| part.ends_with(word))
}

/// The word in this file's name that makes it ask-first, when its kind is a data/config file.
pub fn sensitive_name(path: &str) -> Option<String> {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (stem, extension) = match file.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, extension.to_lowercase()),
        _ => (file.trim_start_matches('.'), String::new()),
    };

    if !extension.is_empty() && !DATA_EXTENSIONS.contains(&extension.as_str()) {
        return None;
    }

    name_parts(stem).into_iter().find(|part| NAME_WORDS.contains(&part.as_str()) || glued(part))
}

fn assignment() -> &'static Regex {
    static COMPILED: OnceLock<Regex> = OnceLock::new();

    COMPILED.get_or_init(|| {
        Regex::new(r#"(?im)\b(?:[A-Za-z0-9_.\-]*(?:password|passwd|pwd|pass|secret|api[_-]?key|apikey|token|credential)[A-Za-z0-9_.\-]*)["']?\s*[:=]\s*["']?([^\s"']{6,})"#).expect("assignment rule")
    })
}

fn candidate() -> &'static Regex {
    static COMPILED: OnceLock<Regex> = OnceLock::new();

    COMPILED.get_or_init(|| Regex::new(r"[A-Za-z0-9_\-+/]{24,}={0,2}").expect("candidate rule"))
}

/// The same placeholders `scan` ignores, plus common non-secret words.
fn placeholder(value: &str) -> bool {
    let lowered = value.to_lowercase();

    ["xxx", "your", "example", "changeme", "placeholder", "dummy", "<", "${", "{{", "process.env", "getenv", "env(", "****", "redacted", "sample", "null", "none", "undefined", "true", "false"]
        .iter()
        .any(|marker| lowered.contains(marker))
}

/// A value that is code (a call, a block) or plain letters, not something typed in as a password.
fn code_like(value: &str) -> bool {
    value.contains(['(', ')', ';', '{', '}']) || !value.chars().any(|c| c.is_ascii_digit() || (!c.is_alphanumeric() && c != '_'))
}

/// Shannon entropy in bits per character.
pub fn entropy(text: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let total = text.chars().count() as f64;

    for c in text.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }

    counts.values().map(|n| {
        let p = *n as f64 / total;

        -p * p.log2()
    }).sum()
}

/// A token that looks random: long, mixed letters and digits, high entropy, and not a path or a hash of a lock file.
fn random_looking(token: &str) -> bool {
    let letters = token.chars().any(|c| c.is_ascii_alphabetic());
    let digits = token.chars().any(|c| c.is_ascii_digit());
    let lower = token.chars().any(|c| c.is_ascii_lowercase());
    let upper = token.chars().any(|c| c.is_ascii_uppercase());

    if !letters || !digits || token.matches('/').count() > 1 || token.starts_with("sha") || placeholder(token) {
        return false;
    }

    let needed = if lower && upper { 3.9 } else { 4.2 };

    entropy(token) >= needed
}

/// Every secret-looking value in `text`, in order, without overlaps.
pub fn scan(file: &str, text: &str) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    let lowered = file.to_lowercase();

    if ["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "cargo.lock", "composer.lock", ".min.js", ".map", ".svg"].iter().any(|suffix| lowered.ends_with(suffix)) {
        return found;
    }

    let mut offset = 0;

    for (index, line) in text.split_inclusive('\n').enumerate() {
        let number = index + 1;

        /* known shapes: the value is what the rule's own capture says */
        if let Some((start, end, what)) = crate::trust::scan::secret_span(line) {
            found.push(Found { line: number, what, start: offset + start, end: offset + end });
        }

        for captured in assignment().captures_iter(line) {
            let value = captured.get(1).expect("value group");

            if code_like(value.as_str()) || placeholder(value.as_str()) {
                continue;
            }

            found.push(Found { line: number, what: "a password, key or token assigned to a name".into(), start: offset + value.start(), end: offset + value.end() });
        }

        for token in candidate().find_iter(line) {
            if random_looking(token.as_str()) {
                found.push(Found { line: number, what: "a long random-looking string".into(), start: offset + token.start(), end: offset + token.end() });
            }
        }

        offset += line.len();
    }

    found.sort_by_key(|item| (item.start, item.end));

    let mut merged: Vec<Found> = Vec::new();

    for item in found {
        match merged.last_mut() {
            Some(last) if item.start < last.end => last.end = last.end.max(item.end),
            _ => merged.push(item),
        }
    }

    merged
}

/// The text with every found value replaced by a mask that keeps only its length.
pub fn mask(text: &str, found: &[Found]) -> String {
    mask_with(text, found, |value| format!("[masked by SDC: {} chars]", value.chars().count()))
}

/// The text with every found value replaced by what `replacement` makes of it.
pub fn mask_with(text: &str, found: &[Found], mut replacement: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;

    for item in found {
        if item.start < at || item.end > text.len() || !text.is_char_boundary(item.start) || !text.is_char_boundary(item.end) {
            continue;
        }

        out.push_str(&text[at..item.start]);
        out.push_str(&replacement(&text[item.start..item.end]));
        at = item.end;
    }

    out.push_str(&text[at..]);
    out
}

/// What the person is shown: kinds and line numbers, never a value.
pub fn describe(found: &[Found]) -> String {
    let mut lines: Vec<String> = found.iter().take(5).map(|item| format!("line {}: {}", item.line, item.what)).collect();

    if found.len() > 5 {
        lines.push(format!("and {} more", found.len() - 5));
    }

    lines.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fake secrets only: shaped like real ones, valid nowhere.
    const FAKE_OPENAI: &str = "sk-proj-FAKEfakeFAKEfakeFAKE1234567890abcd";
    const FAKE_RANDOM: &str = "q8Zr3vLk9XmP2wTn7BcY5dHs1JfG4aEu";

    #[test]
    fn a_known_key_shape_is_found_and_masked_whole() {
        let text = format!("# config\nOPENAI={FAKE_OPENAI}\nname=sdc\n");
        let found = scan("notes.txt", &text);

        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].line, 2);
        assert!(!mask(&text, &found).contains("FAKEfake"));
        assert!(mask(&text, &found).contains("name=sdc"));
    }

    #[test]
    fn an_unquoted_password_line_is_found() {
        let text = "user=admin\npassword=Tr0ub4dor&3xyz\n";
        let found = scan("pass.txt", text);

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(!mask(text, &found).contains("Tr0ub4dor"));
        assert!(mask(text, &found).contains("user=admin"));
    }

    #[test]
    fn api_key_and_token_assignments_are_found_in_json_and_env_styles() {
        for text in ["API_KEY=abcd1234efgh5678", "{\"token\": \"abcd1234efgh5678\"}", "export GITHUB_TOKEN=abcd1234efgh5678", "client_secret: abcd1234efgh5678"] {
            let found = scan("settings.txt", text);

            assert!(!found.is_empty(), "{text}");
            assert!(!mask(text, &found).contains("abcd1234efgh5678"), "{text}");
        }
    }

    #[test]
    fn a_high_entropy_string_without_a_label_is_found() {
        let text = format!("some note\n{FAKE_RANDOM}\n");
        let found = scan("note.txt", &text);

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(!mask(&text, &found).contains(FAKE_RANDOM));
    }

    #[test]
    fn ordinary_code_and_prose_are_not_flagged() {
        let text = "fn read_configuration_for_the_current_user() {}\nlet password_field = input();\nThe quick brown fox jumps over the lazy dog.\nconst path = \"src/components/very/long/path/name.ts\";\npassword = \"changeme\"\ntoken: ${TOKEN}\n";

        assert!(scan("main.rs", text).is_empty(), "{:?}", scan("main.rs", text));
    }

    #[test]
    fn lock_files_are_skipped() {
        assert!(scan("pnpm-lock.yaml", &format!("integrity: {FAKE_RANDOM}")).is_empty());
    }

    #[test]
    fn a_file_name_that_says_secret_asks_first_for_data_files_only() {
        for name in ["pass.txt", "githubapi.rtf", "TAVILY_API_KEY.txt", "deepseakapi.txt", "service-token.json", "credentials.json", "dir/my_secret.yaml", "apiKey.json", "passwords.csv"] {
            assert!(sensitive_name(name).is_some(), "{name}");
        }

        for name in ["api.ts", "keyboard.rs", "passport.js", "token_counter.rs", "README.md", "monkey.txt", "capital.txt", "src/keys.rs"] {
            assert!(sensitive_name(name).is_none(), "{name}");
        }
    }

    #[test]
    fn masking_keeps_the_rest_of_the_file_and_only_the_length_of_the_value() {
        let text = "a=1\nsecret = \"zzzz9999yyyy\"\nb=2\n";
        let found = scan("x.txt", text);
        let masked = mask(text, &found);

        assert!(masked.starts_with("a=1\n") && masked.ends_with("b=2\n"));
        assert!(masked.contains("[masked by SDC: 12 chars]"), "{masked}");
        assert!(!describe(&found).contains("zzzz"));
    }
}
