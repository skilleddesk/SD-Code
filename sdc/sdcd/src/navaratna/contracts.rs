//! **N6-3 - the contract lock and the glossary** (switch `nr.contract_lock`, off).
//!
//! Once a symbol's signature is **published** - the first time a verified step leaves it in the code - it is locked. A later patch that changes
//! a locked signature (or removes the symbol) is refused, unless the step is a **contract-change step**: it must list and update every call site
//! the index finds, and only then does the lock move to the new signature. Private helpers inside a function body are never locked. A
//! **glossary** of names, terms and constants rides in every Context Pack so later steps use the same words.

use std::collections::BTreeMap;
use std::path::Path;

use super::symbols;
use crate::graph::parse::{self, Lang};

/// `file::qualified` -> the locked signature (whitespace-normalised).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Lock {
    pub signatures: BTreeMap<String, Locked>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Locked {
    pub signature: String,
    pub step: String,
}

const FILE: &str = ".sdc/contracts.json";

impl Lock {
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = root.join(FILE);

        std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|error| error.to_string())?;
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(|error| error.to_string())?).map_err(|error| error.to_string())
    }

    /// Publishes the signatures of a verified file: a symbol not yet locked is locked now; a locked one is left alone.
    pub fn publish(&mut self, file: &str, text: &str, step: &str) -> usize {
        if !crate::v3::on("nr.contract_lock") {
            return 0;
        }

        let Some(all) = symbols::index_text(file, text) else { return 0 };
        let mut added = 0;

        for symbol in all.iter().filter(|s| !matches!(s.kind.as_str(), "impl" | "mod")) {
            let key = format!("{file}::{}", symbol.qualified);

            if let std::collections::btree_map::Entry::Vacant(slot) = self.signatures.entry(key) {
                slot.insert(Locked { signature: normalise(&symbol.signature), step: step.to_string() });
                added += 1;
            }
        }

        added
    }
}

fn normalise(signature: &str) -> String {
    signature.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Changed { symbol: String, locked: String, now: String, step: String },
    Removed { symbol: String, locked: String, step: String },
    /// A contract-change step left a call site as it was.
    CallSiteNotUpdated { symbol: String, site: String },
    /// A contract-change step did not list the symbol it changes.
    NotListed { symbol: String },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Changed { symbol, locked, now, step } => write!(f, "refused: `{symbol}` is a locked contract (published by step {step}): `{locked}` cannot become `{now}` outside a contract-change step"),
            Violation::Removed { symbol, locked, step } => write!(f, "refused: `{symbol}` is a locked contract (published by step {step}, `{locked}`) and this patch removes or renames it"),
            Violation::CallSiteNotUpdated { symbol, site } => write!(f, "refused: the contract change of `{symbol}` left the call site {site} unchanged"),
            Violation::NotListed { symbol } => write!(f, "refused: the contract-change step changes `{symbol}` but did not list it"),
        }
    }
}

/// What a patch to one file does to the locks it holds. `Ok` when nothing locked changed (or the switch is off).
pub fn check_patch(lock: &Lock, file: &str, new_text: &str) -> Result<(), Vec<Violation>> {
    if !crate::v3::on("nr.contract_lock") {
        return Ok(());
    }

    evaluate(lock, file, new_text)
}

/// What a new text of a file does to the locks it holds, whatever the switch says (a contract-change step is judged this way too).
fn evaluate(lock: &Lock, file: &str, new_text: &str) -> Result<(), Vec<Violation>> {
    let Some(now) = symbols::index_text(file, new_text) else { return Ok(()) };
    let mut violations = Vec::new();
    let prefix = format!("{file}::");

    for (key, locked) in lock.signatures.iter().filter(|(key, _)| key.starts_with(&prefix)) {
        let name = &key[prefix.len()..];

        match now.iter().find(|s| s.qualified == name && !matches!(s.kind.as_str(), "impl" | "mod")) {
            Some(symbol) if normalise(&symbol.signature) != locked.signature => violations.push(Violation::Changed { symbol: name.to_string(), locked: locked.signature.clone(), now: normalise(&symbol.signature), step: locked.step.clone() }),
            Some(_) => {}
            None => violations.push(Violation::Removed { symbol: name.to_string(), locked: locked.signature.clone(), step: locked.step.clone() }),
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Where a symbol is called, from the index's references: `file:line`.
pub fn call_sites(files: &[(String, String)], name: &str) -> Vec<String> {
    let simple = name.rsplit(['.', ':']).next().unwrap_or(name);
    let mut sites = Vec::new();

    for (path, text) in files {
        let Some(lang) = Lang::of(path).filter(|lang| lang.has_symbols()) else { continue };
        let Some(tree) = parse::parse(lang, text) else { continue };

        for reference in parse::references(&tree, text).into_iter().filter(|r| r.call && r.name == simple) {
            sites.push(format!("{path}:{}", reference.line));
        }
    }

    sites
}

/// A step that changes a locked contract on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractChange {
    /// `file::qualified` keys the step says it changes.
    pub symbols: Vec<String>,
}

/// Checks a contract-change step against the tree before and after: every changed locked symbol is listed, and every call site the index
/// found before is in a file that changed. On success the lock moves to the new signatures.
pub fn apply_contract_change(lock: &mut Lock, change: &ContractChange, before: &[(String, String)], after: &[(String, String)]) -> Result<(), Vec<Violation>> {
    let mut violations = Vec::new();
    let changed_files: Vec<&str> = after.iter().filter(|(path, text)| before.iter().find(|(p, _)| p == path).is_none_or(|(_, old)| old != text)).map(|(path, _)| path.as_str()).collect();

    for (file, text) in after {
        let Err(found) = evaluate(lock, file, text) else { continue };

        for violation in found {
            let (Violation::Changed { symbol, .. } | Violation::Removed { symbol, .. }) = &violation else { continue };
            let key = format!("{file}::{symbol}");

            if !change.symbols.contains(&key) {
                violations.push(Violation::NotListed { symbol: key.clone() });
                continue;
            }

            for site in call_sites(before, symbol) {
                let site_file = site.rsplit_once(':').map_or(site.as_str(), |(f, _)| f);

                if site_file != file && !changed_files.contains(&site_file) {
                    violations.push(Violation::CallSiteNotUpdated { symbol: symbol.clone(), site });
                }
            }
        }
    }

    if !violations.is_empty() {
        return Err(violations);
    }

    for (file, text) in after {
        let prefix = format!("{file}::");

        lock.signatures.retain(|key, _| !(key.starts_with(&prefix) && change.symbols.contains(key)));

        if let Some(all) = symbols::index_text(file, text) {
            for symbol in all.iter().filter(|s| !matches!(s.kind.as_str(), "impl" | "mod")) {
                let key = format!("{file}::{}", symbol.qualified);

                if change.symbols.contains(&key) {
                    lock.signatures.insert(key, Locked { signature: normalise(&symbol.signature), step: "contract-change".into() });
                }
            }
        }
    }

    Ok(())
}

/* ---- the glossary ------------------------------------------------------------------------------------------------------------------- */

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Glossary {
    pub entries: BTreeMap<String, String>,
}

const GLOSSARY_FILE: &str = ".sdc/glossary.json";

impl Glossary {
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(GLOSSARY_FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = root.join(GLOSSARY_FILE);

        std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|error| error.to_string())?;
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(|error| error.to_string())?).map_err(|error| error.to_string())
    }

    /// Adds a term. The same term with a different meaning is refused (the first meaning stands) and reported.
    pub fn add(&mut self, term: &str, meaning: &str) -> Result<(), String> {
        match self.entries.get(term) {
            Some(existing) if existing != meaning => Err(format!("`{term}` already means `{existing}`; it cannot also mean `{meaning}`")),
            Some(_) => Ok(()),
            None => {
                self.entries.insert(term.to_string(), meaning.to_string());
                Ok(())
            }
        }
    }

    /// The constants a file declares at its top level: Rust `const`/`static`, TypeScript/JavaScript `const NAME = ...`, Python `NAME = ...`.
    pub fn harvest(&mut self, file: &str, text: &str) -> Vec<String> {
        let mut conflicts = Vec::new();
        let rust = regex::Regex::new(r"(?m)^(?:pub(?:\([a-z]+\))?\s+)?(?:const|static)\s+([A-Z][A-Z0-9_]*)\s*:\s*[^=]+=\s*([^;]+);").expect("a valid pattern");
        let script = regex::Regex::new(r"(?m)^(?:export\s+)?const\s+([A-Z][A-Z0-9_]*)\s*(?::\s*[^=]+)?=\s*([^;\n]+)").expect("a valid pattern");
        let python = regex::Regex::new(r"(?m)^([A-Z][A-Z0-9_]*)\s*=\s*(.+)$").expect("a valid pattern");
        let pattern = match Lang::of(file) {
            Some(Lang::Rust) => &rust,
            Some(Lang::JavaScript | Lang::TypeScript | Lang::Tsx) => &script,
            Some(Lang::Python) => &python,
            _ => return conflicts,
        };

        for capture in pattern.captures_iter(text) {
            if let Err(conflict) = self.add(&capture[1], capture[2].trim()) {
                conflicts.push(conflict);
            }
        }

        conflicts
    }

    /// The pairs that go into every Context Pack.
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.entries.iter().map(|(term, meaning)| (term.clone(), meaning.clone())).collect()
    }
}

#[cfg(test)]
mod tests;
