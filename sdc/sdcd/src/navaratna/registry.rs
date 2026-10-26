//! **N6-1 - the Artifact Registry and duplicate detection** (switch `nr.registry`, on, record and warn only).
//!
//! Nothing is built twice and every requirement has exactly one owner:
//!
//! 1. **one owner** - when a plan is made, each requirement id is assigned to exactly one step; a plan that leaves one out or gives one to
//!    two steps is rejected, with the ids named;
//! 2. **the registry** - for each artifact (a file, a symbol, a document section): the requirement, the step, the model and a content hash;
//! 3. **create becomes edit** - before a create step, the registry and the symbol index are asked; if the name exists the step is turned into
//!    an edit and the reason is recorded;
//! 4. **duplicates** - after a step, same-name symbols and near-identical bodies (whitespace-normalised hash) are reported as a warning with
//!    both locations. Nothing is deleted automatically.
//!
//! The final receipt lists every requirement with its step, model and evidence (`file:line` or a test name).

use std::collections::BTreeMap;
use std::path::Path;

use super::symbols;
use crate::graph::parse::{self, Lang};

/* ---- 1. one owner ------------------------------------------------------------------------------------------------------------------ */

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProblems {
    pub unowned: Vec<String>,
    /// Requirement id and the steps that all claim it.
    pub shared: Vec<(String, Vec<String>)>,
    pub unknown: Vec<(String, String)>,
}

impl std::fmt::Display for OwnerProblems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();

        if !self.unowned.is_empty() {
            parts.push(format!("no step owns {}", self.unowned.join(", ")));
        }

        for (r, steps) in &self.shared {
            parts.push(format!("{r} is owned by {} steps ({})", steps.len(), steps.join(", ")));
        }

        for (r, step) in &self.unknown {
            parts.push(format!("step {step} owns {r}, which is not a requirement"));
        }

        write!(f, "the plan is rejected: {}", parts.join("; "))
    }
}

/// Checks a plan: `steps` is `(step id, the requirement ids it owns)`, `requirements` every requirement id of the request.
pub fn one_owner(requirements: &[String], steps: &[(String, Vec<String>)]) -> Result<(), OwnerProblems> {
    if !crate::v3::on("nr.registry") {
        return Ok(());
    }

    let mut owners: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut unknown = Vec::new();

    for (step, owned) in steps {
        for r in owned {
            if requirements.contains(r) {
                owners.entry(r.as_str()).or_default().push(step.clone());
            } else {
                unknown.push((r.clone(), step.clone()));
            }
        }
    }

    let unowned: Vec<String> = requirements.iter().filter(|r| !owners.contains_key(r.as_str())).cloned().collect();
    let shared: Vec<(String, Vec<String>)> = owners.into_iter().filter(|(_, steps)| steps.len() > 1).map(|(r, steps)| (r.to_string(), steps)).collect();

    if unowned.is_empty() && shared.is_empty() && unknown.is_empty() {
        Ok(())
    } else {
        Err(OwnerProblems { unowned, shared, unknown })
    }
}

/* ---- 2. the registry --------------------------------------------------------------------------------------------------------------- */

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// `src/a.rs`, `src/a.rs::Shape::area`, `doc:guide#3`.
    pub artifact: String,
    pub r_id: String,
    pub step: String,
    pub model: String,
    pub hash: String,
    /// `file:line` or a test name: what shows the requirement is met.
    pub evidence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Registry {
    pub entries: Vec<Entry>,
    /// Why a create step became an edit.
    pub conversions: Vec<String>,
}

const FILE: &str = ".sdc/registry.json";

impl Registry {
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = root.join(FILE);

        std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|error| error.to_string())?;
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(|error| error.to_string())?).map_err(|error| error.to_string())
    }

    /// Records an artifact for a requirement; the same artifact written again for the same requirement replaces its row (a retry never adds a second one),
    /// while a second requirement built in the same file keeps its own row - the receipt names evidence for every requirement.
    pub fn record(&mut self, entry: Entry) {
        if !crate::v3::on("nr.registry") {
            return;
        }

        self.entries.retain(|existing| existing.artifact != entry.artifact || existing.r_id != entry.r_id);
        self.entries.push(entry);
    }

    pub fn find(&self, artifact: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.artifact == artifact)
    }
}

/// The content hash recorded for an artifact: whitespace-normalised, so a reformat is not a change.
pub fn content_hash(text: &str) -> String {
    parse::sha(&parse::normalised(text))
}

/* ---- 3. create becomes edit -------------------------------------------------------------------------------------------------------- */

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateDecision {
    Create,
    /// The artifact exists: the step edits it. The reason is recorded in the registry.
    ConvertToEdit { existing: String, why: String },
}

/// Before a step creates `artifact` (a file path, or `path::Symbol`), look in the registry and the project.
pub fn before_create(registry: &mut Registry, root: &Path, artifact: &str) -> CreateDecision {
    if !crate::v3::on("nr.registry") {
        return CreateDecision::Create;
    }

    let why = if let Some(entry) = registry.find(artifact) {
        Some(format!("{artifact} is already in the registry (step {}, {})", entry.step, entry.r_id))
    } else if let Some((file, symbol)) = artifact.split_once("::") {
        std::fs::read_to_string(root.join(file)).ok().and_then(|text| symbols::index_text(file, &text)).and_then(|all| all.iter().find(|s| s.qualified == symbol || s.name == symbol).map(|s| format!("{symbol} already exists in {file}:{}", s.start_line)))
    } else if root.join(artifact).is_file() {
        Some(format!("{artifact} already exists on disk"))
    } else {
        None
    };

    match why {
        Some(why) => {
            registry.conversions.push(format!("create {artifact} -> edit: {why}"));
            CreateDecision::ConvertToEdit { existing: artifact.to_string(), why }
        }
        None => CreateDecision::Create,
    }
}

/* ---- 4. duplicates ----------------------------------------------------------------------------------------------------------------- */

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuplicateKind {
    SameName,
    NearIdentical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    pub kind: DuplicateKind,
    pub name: String,
    pub a: String,
    pub b: String,
}

impl std::fmt::Display for Duplicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            DuplicateKind::SameName => write!(f, "`{}` is declared twice: {} and {}", self.name, self.a, self.b),
            DuplicateKind::NearIdentical => write!(f, "`{}` has the same body as the one at {}: {} (a copy?)", self.name, self.b, self.a),
        }
    }
}

/// The most project files one duplicate check reads.
const MAX_FILES_READ: usize = 60;

/// The time one duplicate check may spend reading. Under `cfg(test)` it is generous: the whole suite runs at once on a loaded machine, and a
/// check that silently read no file would fail a test for the machine's load instead of for its own code.
fn read_budget() -> std::time::Duration {
    if cfg!(test) {
        std::time::Duration::from_secs(60)
    } else {
        std::time::Duration::from_millis(250)
    }
}

/// A body shorter than this many lines is not worth reporting as a copy.
const MIN_COPY_LINES: usize = 4;
/// Names every project has several of on purpose.
const COMMON: &[&str] = &["main", "new", "default", "fmt", "from", "into", "drop", "clone", "eq", "hash", "__init__", "constructor", "setup", "teardown", "index", "run", "test", "handler", "render", "get", "set"];

/// The duplicates between the symbols of one file and the other files. `files` is `(path, text)`; `edited` is the path being checked.
pub fn find_duplicates(files: &[(String, String)], edited: &str) -> Vec<Duplicate> {
    /* a copy is usually a copy with another name: the hash is of the declaration with its own name taken out */
    let defs_of = |path: &str, text: &str| -> Vec<parse::Def> {
        let Some(lang) = Lang::of(path).filter(|lang| lang.has_symbols()) else { return Vec::new() };
        let Some(tree) = parse::parse(lang, text) else { return Vec::new() };

        parse::definitions(lang, &tree, text)
            .into_iter()
            .filter(|def| matches!(def.kind.as_str(), "fn" | "def" | "method" | "class" | "struct" | "interface"))
            .map(|mut def| {
                let declaration = text.get(def.start_byte..def.end_byte).unwrap_or_default();

                def.norm_hash = parse::sha(&parse::normalised(&declaration.replace(def.name.as_str(), "\u{1}")));
                def
            })
            .collect()
    };
    let Some((_, edited_text)) = files.iter().find(|(path, _)| path == edited) else { return Vec::new() };
    let mine = defs_of(edited, edited_text);
    let mut found = Vec::new();

    for (path, text) in files.iter().filter(|(path, _)| path != edited) {
        let theirs = defs_of(path, text);
        let tests = crate::graph::index::is_test_path(path) || crate::graph::index::is_test_path(edited);

        for def in &mine {
            for other in &theirs {
                let here = format!("{edited}:{}", def.start_line);
                let there = format!("{path}:{}", other.start_line);
                let long = def.end_line.saturating_sub(def.start_line) + 1 >= MIN_COPY_LINES;

                if def.norm_hash == other.norm_hash && long && def.kind == other.kind {
                    found.push(Duplicate { kind: DuplicateKind::NearIdentical, name: def.qualified.clone(), a: here, b: there });
                } else if def.qualified == other.qualified && def.kind == other.kind && !tests && !COMMON.contains(&def.name.as_str()) && def.kind != "method" {
                    found.push(Duplicate { kind: DuplicateKind::SameName, name: def.qualified.clone(), a: here, b: there });
                }
            }
        }
    }

    found
}

/// The warning for an edited file, or `None`. Reads only the project files that mention one of the edited file's symbol names.
pub fn duplicate_note(root: &Path, path: &str, text: &str) -> Option<String> {
    if !crate::v3::on("nr.registry") {
        return None;
    }

    let mine = symbols::index_text(path, text)?;
    let names: Vec<&str> = mine.iter().filter(|s| s.depth == 0 && !COMMON.contains(&s.name.as_str())).map(|s| s.name.as_str()).collect();

    if names.is_empty() {
        return None;
    }

    let mut files = vec![(path.to_string(), text.to_string())];
    let started = std::time::Instant::now();

    for relative in crate::search::files(root, 4_000) {
        /* this runs after every edit: a bounded number of files and a bounded time, so a big project is not slowed down */
        if files.len() > MAX_FILES_READ || started.elapsed() > read_budget() {
            break;
        }

        if relative == path || symbols::no_grammar(&relative) {
            continue;
        }

        let Ok(other) = std::fs::read_to_string(root.join(&relative)) else { continue };

        if other.len() < 200_000 && names.iter().any(|name| other.contains(name)) {
            files.push((relative, other));
        }
    }

    let duplicates = find_duplicates(&files, path);

    if duplicates.is_empty() {
        return None;
    }

    Some(format!("[SDC check - a warning, nothing was changed or deleted] Possible duplicates:\n- {}\nIf it is a copy, edit the original instead.", duplicates.iter().take(5).map(|d| d.to_string()).collect::<Vec<_>>().join("\n- ")))
}

/* ---- the receipt ------------------------------------------------------------------------------------------------------------------- */

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptLine {
    pub r_id: String,
    pub step: String,
    pub model: String,
    pub evidence: String,
}

/// Every requirement with its step, model and evidence; one without a registry row says so plainly.
pub fn receipt(requirements: &[String], registry: &Registry) -> Vec<ReceiptLine> {
    requirements
        .iter()
        .map(|r| match registry.entries.iter().find(|entry| &entry.r_id == r) {
            Some(entry) => ReceiptLine { r_id: r.clone(), step: entry.step.clone(), model: entry.model.clone(), evidence: entry.evidence.clone() },
            None => ReceiptLine { r_id: r.clone(), step: "-".into(), model: "-".into(), evidence: "none recorded".into() },
        })
        .collect()
}

#[cfg(test)]
mod tests;
