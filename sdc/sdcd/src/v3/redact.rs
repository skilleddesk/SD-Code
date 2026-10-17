//! **Secret redaction** (v3 W9, switch `v3.redact`): secrets in what goes to a model become `‹secret:N›`, and
//! come back as the real value in what the model asks SDC to write or run.
//!
//! The file guard already keeps key files (`.env`, `*.pem`, …) away from the agent; this catches the secrets that
//! arrive another way - printed by a command, inside a config file, in a log, pasted into a message. A finding is
//! either a known token shape (cloud, Git host, AI provider, chat keys, private-key blocks, JWTs) or the value of
//! an assignment whose name says secret (`password = …`, `API_KEY: …`) that looks random enough (entropy).
//!
//! The vault is per chat and in memory: `‹secret:3›` in a later message of the same chat still turns back into
//! the real value on write. A placeholder SDC does not know is left as it is.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use regex::Regex;
use serde_json::Value;

#[derive(Default)]
struct Vault {
    by_value: HashMap<String, usize>,
    by_number: HashMap<usize, String>,
}

fn vaults() -> &'static Mutex<HashMap<String, Vault>> {
    static VAULTS: OnceLock<Mutex<HashMap<String, Vault>>> = OnceLock::new();

    VAULTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Known token shapes. The whole match is the secret.
fn shapes() -> &'static [Regex] {
    static SHAPES: OnceLock<Vec<Regex>> = OnceLock::new();

    SHAPES.get_or_init(|| {
        [
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"\bgh[pousr]_[A-Za-z0-9]{36,}\b",
            r"\bgithub_pat_[A-Za-z0-9_]{50,}\b",
            r"\bglpat-[A-Za-z0-9_\-]{20,}\b",
            r"\bsk-ant-[A-Za-z0-9_\-]{20,}",
            r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{32,}",
            r"\bxox[abprs]-[A-Za-z0-9\-]{10,}",
            r"\bAIza[0-9A-Za-z_\-]{35}\b",
            r"\bsk_live_[0-9A-Za-z]{24,}\b",
            r"\beyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
            r"\b[a-z]+://[^\s:/@]+:[^\s@/]{6,}@[^\s/]+",
        ]
        .iter()
        .map(|pattern| Regex::new(pattern).expect("a valid secret pattern"))
        .collect()
    })
}

/// `NAME = value` where the name says secret; group 2 is the value.
fn assignment() -> &'static Regex {
    static ASSIGNMENT: OnceLock<Regex> = OnceLock::new();

    ASSIGNMENT.get_or_init(|| {
        Regex::new(r#"(?i)\b([A-Z0-9_.\-]*(?:password|passwd|secret|token|api[_\-]?key|access[_\-]?key|private[_\-]?key|auth|credential)[A-Z0-9_.\-]*)["']?\s*[:=]\s*["']?([^\s"'`,;]{8,})"#)
            .expect("a valid assignment pattern")
    })
}

/// Shannon entropy in bits per character.
fn entropy(text: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();

    for c in text.chars() {
        *counts.entry(c).or_default() += 1;
    }

    let length = text.chars().count() as f64;

    counts.values().map(|&count| count as f64 / length).map(|p| -p * p.log2()).sum()
}

/// Does an assigned value look like a secret rather than a word or a reference (`${DB_PASSWORD}`, `changeme`)?
fn random_enough(value: &str) -> bool {
    !value.starts_with('$') && !value.starts_with('<') && !value.contains("secret:") && value.chars().count() >= 8 && entropy(value) >= 3.0
}

fn placeholder(number: usize) -> String {
    format!("‹secret:{number}›")
}

/// Replaces the secrets in `text`; answers the new text and how many were found.
pub fn scrub(session: &str, text: &str) -> (String, usize) {
    let mut found: Vec<(usize, usize)> = Vec::new();

    for shape in shapes() {
        found.extend(shape.find_iter(text).map(|hit| (hit.start(), hit.end())));
    }

    for captures in assignment().captures_iter(text) {
        if let Some(value) = captures.get(2).filter(|value| random_enough(value.as_str())) {
            found.push((value.start(), value.end()));
        }
    }

    if found.is_empty() {
        return (text.to_string(), 0);
    }

    /* Overlaps merge; replaced from the end so earlier offsets stay right. */
    found.sort();

    let mut merged: Vec<(usize, usize)> = Vec::new();

    for (start, end) in found {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }

    let mut vaults = vaults().lock().unwrap_or_else(|poison| poison.into_inner());
    let vault = vaults.entry(session.to_string()).or_default();
    let mut out = text.to_string();

    for (start, end) in merged.iter().rev() {
        let secret = &text[*start..*end];
        let next = vault.by_value.len() + 1;
        let number = *vault.by_value.entry(secret.to_string()).or_insert(next);

        vault.by_number.insert(number, secret.to_string());
        out.replace_range(*start..*end, &placeholder(number));
    }

    (out, merged.len())
}

/// Puts one secret value in the chat's vault and answers its placeholder - for a guard that found a secret by
/// another rule (Phase 0.3) and still wants the model's later write to carry the real value back.
pub fn hide(session: &str, secret: &str) -> String {
    let mut vaults = vaults().lock().unwrap_or_else(|poison| poison.into_inner());
    let vault = vaults.entry(session.to_string()).or_default();
    let next = vault.by_value.len() + 1;
    let number = *vault.by_value.entry(secret.to_string()).or_insert(next);

    vault.by_number.insert(number, secret.to_string());
    placeholder(number)
}

fn restore_text(vault: &Vault, text: &str) -> String {
    static PLACEHOLDER: OnceLock<Regex> = OnceLock::new();

    let pattern = PLACEHOLDER.get_or_init(|| Regex::new(r"‹secret:(\d+)›").expect("a valid placeholder pattern"));

    pattern
        .replace_all(text, |captures: &regex::Captures| captures[1].parse::<usize>().ok().and_then(|number| vault.by_number.get(&number).cloned()).unwrap_or_else(|| captures[0].to_string()))
        .to_string()
}

/// A chat's vault as `(number, secret)` pairs - saved in the job journal so a resumed turn's placeholders keep
/// their meaning after a restart (R2).
pub fn export(session: &str) -> Vec<(usize, String)> {
    let vaults = vaults().lock().unwrap_or_else(|poison| poison.into_inner());
    let mut pairs: Vec<(usize, String)> = vaults.get(session).map(|vault| vault.by_number.iter().map(|(number, secret)| (*number, secret.clone())).collect()).unwrap_or_default();

    pairs.sort();
    pairs
}

/// Loads a saved vault back (on resume).
pub fn import(session: &str, pairs: &[(usize, String)]) {
    let mut vaults = vaults().lock().unwrap_or_else(|poison| poison.into_inner());
    let vault = vaults.entry(session.to_string()).or_default();

    for (number, secret) in pairs {
        vault.by_number.insert(*number, secret.clone());
        vault.by_value.insert(secret.clone(), *number);
    }
}

/// Puts the real values back into a tool call's input (every string in it).
pub fn restore(session: &str, input: &Value) -> Value {
    let vaults = vaults().lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(vault) = vaults.get(session) else {
        return input.clone();
    };

    fn walk(vault: &Vault, value: &Value) -> Value {
        match value {
            Value::String(text) if text.contains("‹secret:") => Value::String(restore_text(vault, text)),
            Value::Array(items) => Value::Array(items.iter().map(|item| walk(vault, item)).collect()),
            Value::Object(map) => Value::Object(map.iter().map(|(key, item)| (key.clone(), walk(vault, item))).collect()),
            other => other.clone(),
        }
    }

    walk(vault, input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tokens_and_secret_assignments_are_replaced_and_restored() {
        /* The tokens are built from pieces so no scanner takes this fixture for a real credential. */
        let (openai, github) = (format!("{}{}", "sk-proj-", "AbCdEf0123456789AbCdEf0123456789xyz"), format!("{}{}", "ghp_", "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij12"));
        let text = &format!("export OPENAI_API_KEY={openai}\nDB_PASSWORD=\"t9#Lq2!vZp8@wK\"\ngh = {github}\nuser = admin\npassword = ${{DB_PASSWORD}}\n");
        let (scrubbed, found) = scrub("chat-r1", text);

        assert_eq!(found, 3, "{scrubbed}");
        assert!(!scrubbed.contains("sk-proj-") && !scrubbed.contains("t9#Lq2") && !scrubbed.contains("ghp_"));
        assert!(scrubbed.contains("user = admin") && scrubbed.contains("${DB_PASSWORD}"), "words and references stay");

        let call = serde_json::json!({ "path": ".env.example", "content": scrubbed });
        let restored = restore("chat-r1", &call);

        assert_eq!(restored["content"], text.as_str(), "the real values come back on write");
        assert_eq!(restore("another-chat", &call)["content"], call["content"], "another chat's vault does not");
    }

    #[test]
    fn the_same_secret_keeps_its_number_and_private_keys_go_whole() {
        /* Built from pieces so no scanner mistakes this test fixture for a real key in the source. */
        let (begin, end, kind) = ("-----BEGIN", "-----END", "OPENSSH PRIVATE KEY-----");
        let key = &format!("{begin} {kind}\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU\n{end} {kind}");
        let (first, _) = scrub("chat-r2", &format!("a {key} b"));
        let (second, _) = scrub("chat-r2", &format!("again {key}"));

        assert_eq!(first, "a ‹secret:1› b");
        assert_eq!(second, "again ‹secret:1›");
        assert_eq!(scrub("chat-r2", "nothing here, password: short").1, 0);

        /* After a restart: the saved vault gives the same numbers back. */
        import("chat-r2-after-restart", &export("chat-r2"));
        assert_eq!(scrub("chat-r2-after-restart", key).0, "‹secret:1›");
    }
}
