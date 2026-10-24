//! **N2-3 - the Hybrid Spec Compiler and the cloud Blueprint** (switch `nr.blueprint`, off).
//!
//! For a large or tangled prompt, one call to the chain's *architect* row returns the skeleton of the work: the folder tree, the data
//! models, the interface contracts as **empty signatures** and the requirement list with its unit mapping. The local 9B then does everything
//! else against it. The Blueprint is checked by code before it is used - the sentence-coverage gate, a syntax check of every contract, safe
//! paths, no implementation bodies - and when the call is not allowed, fails, or fails validation, the local path is used instead. With the
//! relay consent off, a model picked by hand, or `privacy = "local-only"`, **zero** cloud calls are made.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::spec::{self, Label, Map};
use crate::graph::parse::{self, Lang};

/// A prompt this long (characters), or with this many units or requirements, is "large or tangled".
pub const LARGE_PROMPT_CHARS: usize = 6_000;
pub const LARGE_UNITS: usize = 40;
pub const LARGE_REQUIREMENTS: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    pub file: String,
    /// An empty signature: `pub fn total(items: &[Item]) -> f64`, `def total(items):`, `function total(items: Item[]): number`.
    pub signature: String,
    #[serde(default)]
    pub requirements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub units: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blueprint {
    pub folders: Vec<String>,
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default)]
    pub contracts: Vec<Contract>,
    pub requirements: Vec<Requirement>,
    /// Units that are context, not work.
    #[serde(default)]
    pub background: Vec<String>,
}

/// Why the cloud was not asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    SwitchOff,
    ManualPin,
    LocalOnly,
    NoConsent,
    NoArchitect,
    TooSmall,
}

#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// `relay.auto`: the person's consent to the chain.
    pub relay_consent: bool,
    /// `.sdc/policy.toml` says `privacy = "local-only"`.
    pub local_only: bool,
    /// The person picked a cloud, API or CLI model by hand.
    pub manual_pin: bool,
    /// The chain has an enabled row with the architect role (or any role).
    pub architect_row: bool,
}

/// Is the prompt large or tangled enough?
pub fn large(prompt: &str, map: &Map) -> bool {
    prompt.len() >= LARGE_PROMPT_CHARS || map.units.len() >= LARGE_UNITS || map.requirements.len() >= LARGE_REQUIREMENTS
}

/// May the architect be asked? `Ok(())` only when every condition holds.
pub fn may_ask(context: &Context, prompt: &str, map: &Map) -> Result<(), Skip> {
    if !crate::v3::on("nr.blueprint") {
        return Err(Skip::SwitchOff);
    }

    if context.manual_pin {
        return Err(Skip::ManualPin);
    }

    if context.local_only {
        return Err(Skip::LocalOnly);
    }

    if !context.relay_consent {
        return Err(Skip::NoConsent);
    }

    if !context.architect_row {
        return Err(Skip::NoArchitect);
    }

    if !large(prompt, map) {
        return Err(Skip::TooSmall);
    }

    Ok(())
}

/// What the architect is asked: the parsed units (already redacted by the caller) and the shape of the answer.
pub fn request(units: &[spec::Unit], redact: &dyn Fn(&str) -> String) -> String {
    let mut out = String::from(
        "You are the architect. Below are the numbered units of a large request. Return ONE JSON object and nothing else, with: \"folders\" (the folder tree, relative paths), \"models\" (data models: name, fields), \"contracts\" (interface contracts as EMPTY signatures - no function bodies: file, signature, requirements), \"requirements\" (id R-001..., units: the unit ids it is made of) and \"background\" (unit ids that are context only). Every unit id must appear in exactly one requirement or in background. Do not write any implementation.\n\nUnits:\n",
    );

    for unit in units {
        out.push_str(&format!("{} [{:?}]: {}\n", unit.id, unit.kind, redact(&unit.text)));
    }

    out
}

/// Reads the architect's answer: the first JSON object in it.
pub fn parse_blueprint(answer: &str) -> Result<Blueprint, String> {
    let start = answer.find('{').ok_or("no JSON object in the answer")?;
    let end = answer.rfind('}').ok_or("no JSON object in the answer")?;

    serde_json::from_str(&answer[start..=end]).map_err(|error| format!("the Blueprint is not valid: {error}"))
}

fn safe_path(path: &str) -> bool {
    let p = path.trim();

    !p.is_empty() && !p.starts_with('/') && !p.starts_with('\\') && !p.contains("..") && !p.contains(':') && !p.contains('\0')
}

/// An empty signature has no body: no `{ ... }` with anything in it, and for Python nothing after the colon but `...` or `pass`.
pub fn is_empty_signature(signature: &str) -> bool {
    let text = signature.trim();

    if text.is_empty() {
        return false;
    }

    if let Some(open) = text.find('{') {
        let inside = text[open + 1..].trim_end().trim_end_matches('}').trim();

        /* `struct Item { price: f64 }` is a model, `fn x() { body }` is not a contract: only an empty brace pair may follow a signature */
        return inside.is_empty() && !text[..open].contains("fn ") && !text[..open].contains("function");
    }

    if text.contains('\n') {
        let mut lines = text.lines();
        let head = lines.next().unwrap_or_default();

        return head.trim_end().ends_with(':') && lines.all(|line| matches!(line.trim(), "" | "..." | "pass"));
    }

    true
}

/// The contract completed with a stub body, so the grammar can judge its syntax.
fn completed(file: &str, signature: &str) -> Option<(String, String)> {
    let lang = Lang::of(file)?;
    let signature = signature.trim();
    let text = match lang {
        Lang::Python => {
            let head = signature.lines().next().unwrap_or_default().trim_end().to_string();

            format!("{head}\n    ...\n")
        }
        Lang::Rust | Lang::Go | Lang::JavaScript | Lang::TypeScript | Lang::Tsx | Lang::Php => {
            let head = signature.trim_end_matches(';').trim_end_matches("{}").trim_end();

            /* a trait or interface method has no body at all, so it is wrapped in one for the check */
            match lang {
                Lang::Rust => format!("trait T {{ {head}; }}\n"),
                Lang::TypeScript | Lang::Tsx => format!("interface T {{ {head}; }}\n"),
                _ => format!("{head} {{}}\n"),
            }
        }
        _ => return None,
    };

    Some((file.to_string(), text))
}

/// Builds the map the coverage gate reads from a Blueprint, against the parsed units.
pub fn as_map(units: &[spec::Unit], blueprint: &Blueprint) -> Map {
    let mut labels: BTreeMap<String, Label> = BTreeMap::new();
    let mut requirements: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for requirement in &blueprint.requirements {
        for (n, id) in requirement.units.iter().enumerate() {
            if units.iter().any(|unit| &unit.id == id) {
                labels.entry(id.clone()).or_insert(if n == 0 { Label::Requirement } else { Label::PartOf(requirement.units[0].clone()) });
                requirements.entry(requirement.id.clone()).or_default().push(id.clone());
            }
        }
    }

    for id in &blueprint.background {
        if units.iter().any(|unit| &unit.id == id) {
            labels.entry(id.clone()).or_insert(Label::Background);
        }
    }

    Map { units: units.to_vec(), labels, requirements }
}

/// Checks a Blueprint. All the reasons it is not good are returned.
pub fn validate(units: &[spec::Unit], blueprint: &Blueprint) -> Result<Map, Vec<String>> {
    let mut problems = Vec::new();

    for folder in &blueprint.folders {
        if !safe_path(folder) {
            problems.push(format!("unsafe folder path `{folder}`"));
        }
    }

    let mut seen_requirements: Vec<&str> = Vec::new();
    let mut seen_units: Vec<&str> = Vec::new();

    for requirement in &blueprint.requirements {
        if !requirement.id.starts_with("R-") || requirement.units.is_empty() {
            problems.push(format!("requirement `{}` needs an id like R-001 and at least one unit", requirement.id));
        }

        if seen_requirements.contains(&requirement.id.as_str()) {
            problems.push(format!("requirement id {} is used twice", requirement.id));
        }

        seen_requirements.push(&requirement.id);

        for id in requirement.units.iter().chain(blueprint.background.iter()) {
            if !units.iter().any(|unit| &unit.id == id) {
                problems.push(format!("unit {id} does not exist in the request"));
            }
        }

        for id in &requirement.units {
            if seen_units.contains(&id.as_str()) {
                problems.push(format!("unit {id} belongs to more than one requirement"));
            }

            seen_units.push(id);
        }
    }

    for contract in &blueprint.contracts {
        if !safe_path(&contract.file) {
            problems.push(format!("unsafe contract path `{}`", contract.file));
            continue;
        }

        if !is_empty_signature(&contract.signature) {
            problems.push(format!("contract in {} has an implementation body, not an empty signature: `{}`", contract.file, contract.signature.lines().next().unwrap_or_default()));
            continue;
        }

        if let Some((file, text)) = completed(&contract.file, &contract.signature) {
            if parse::check(&file, &text).is_some_and(|errors| !errors.is_empty()) {
                problems.push(format!("contract in {} does not parse: `{}`", contract.file, contract.signature.lines().next().unwrap_or_default()));
            }
        }

        for r in &contract.requirements {
            if !blueprint.requirements.iter().any(|req| &req.id == r) {
                problems.push(format!("contract in {} names the unknown requirement {r}", contract.file));
            }
        }
    }

    let map = as_map(units, blueprint);

    /* the sentence-coverage gate, exactly as for a locally made map */
    let unmapped: Vec<String> = map.unmapped().iter().map(|unit| unit.id.clone()).collect();

    if !unmapped.is_empty() {
        problems.push(format!("units not mapped to a requirement or marked background: {}", unmapped.join(", ")));
    }

    if problems.is_empty() {
        Ok(map)
    } else {
        Err(problems)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The architect's Blueprint, validated; the map is what the rest of the work uses.
    Blueprint(Box<Blueprint>, Box<Map>),
    /// The cloud was not used (or failed); the local path goes on with the locally made map.
    Local(String),
}

/// The whole step: decide, ask once, validate, or fall back. `architect` is the one cloud call (a VCR fixture in the tests).
pub fn run(context: &Context, prompt: &str, local_map: &Map, redact: &dyn Fn(&str) -> String, architect: &mut dyn FnMut(&str) -> Result<String, String>) -> Outcome {
    if let Err(skip) = may_ask(context, prompt, local_map) {
        return Outcome::Local(format!("not asked: {skip:?}"));
    }

    let answer = match architect(&request(&local_map.units, redact)) {
        Ok(answer) => answer,
        Err(error) => return Outcome::Local(format!("the architect call failed: {error}")),
    };

    match parse_blueprint(&answer).map_err(|error| vec![error]).and_then(|blueprint| validate(&local_map.units, &blueprint).map(|map| (blueprint, map))) {
        Ok((blueprint, map)) => Outcome::Blueprint(Box::new(blueprint), Box::new(map)),
        Err(problems) => Outcome::Local(format!("the Blueprint failed validation: {}", problems.join("; "))),
    }
}

#[cfg(test)]
mod tests;
