//! **Hallucinated-package guard (P3-3, switch `trust.slopguard`)**: a model sometimes installs a package that does not exist (it made the
//! name up), and an attacker can register exactly such names ("slopsquatting") with code that runs on install. Before an install
//! command (`npm install`, `pip install`, `cargo add` and their kin) is asked about or run, every package it names is looked up in its
//! registry. A package the registry says does not exist **refuses the command**, naming the package. A registry that cannot be reached
//! is never taken for "missing": the command goes on, with a note that the names were not checked.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    Npm,
    PyPi,
    Crates,
}

/// What a registry said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exists {
    Yes,
    No,
    /// No answer (offline, a timeout, a server error): not evidence either way.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub ecosystem: Ecosystem,
    pub name: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Packages the registry does not know: the command is refused.
    pub missing: Vec<Package>,
    /// Packages that could not be checked.
    pub unchecked: Vec<Package>,
}

impl Verdict {
    pub fn refusal(&self) -> Option<String> {
        if self.missing.is_empty() {
            return None;
        }

        let names: Vec<String> = self.missing.iter().map(|package| format!("`{}` ({})", package.name, registry_name(package.ecosystem))).collect();

        Some(format!("Refused: {} not found in the package registry - a name a model made up can be registered by an attacker with code that runs on install. Check the name (and the project's own dependency list), then try again.", names.join(", ")))
    }

    pub fn note(&self) -> Option<String> {
        (!self.unchecked.is_empty()).then(|| format!("[SDC: could not reach the registry to check {} - the install was not verified.]", self.unchecked.iter().map(|package| format!("`{}`", package.name)).collect::<Vec<_>>().join(", ")))
    }
}

fn registry_name(ecosystem: Ecosystem) -> &'static str {
    match ecosystem {
        Ecosystem::Npm => "npm",
        Ecosystem::PyPi => "PyPI",
        Ecosystem::Crates => "crates.io",
    }
}

/// Strips a version or extras from a requirement: `requests==2.0` -> `requests`, `left-pad@1.3` -> `left-pad`, `@scope/pkg@1` -> `@scope/pkg`.
fn bare(ecosystem: Ecosystem, token: &str) -> Option<String> {
    let token = token.trim_matches(|c| c == '"' || c == '\'');
    let name = match ecosystem {
        Ecosystem::Npm | Ecosystem::Crates => {
            let skip = usize::from(token.starts_with('@'));

            match token[skip..].find('@') {
                Some(at) => &token[..skip + at],
                None => token,
            }
        }
        Ecosystem::PyPi => token.split(['=', '<', '>', '~', '!', '[', ';']).next().unwrap_or_default(),
    };

    /* a path, a URL, a git source or a local archive is not a registry name */
    if name.is_empty() || name.contains("://") || name.starts_with('.') || name.starts_with('/') || name.starts_with('~') || name.contains(':') || name.ends_with(".whl") || name.ends_with(".tgz") || name.ends_with(".tar.gz") || name.ends_with(".txt") {
        return None;
    }

    let valid = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | '/'));

    valid.then(|| name.to_string())
}

/// The packages an install command names. Reads each `&&`/`;`/`|`-separated part; flags are skipped.
pub fn packages(line: &str) -> Vec<Package> {
    let mut found = Vec::new();

    for part in line.split(['&', ';', '|', '\n']) {
        let words: Vec<&str> = part.split_whitespace().collect();
        let Some(program_at) = words.iter().position(|word| matches!(*word, "npm" | "pnpm" | "yarn" | "pip" | "pip3" | "cargo" | "python" | "python3")) else {
            continue;
        };
        let program = words[program_at];
        let rest = &words[program_at + 1..];
        let (ecosystem, args): (Ecosystem, &[&str]) = match program {
            "npm" | "pnpm" if matches!(rest.first(), Some(&"install" | &"i" | &"add")) => (Ecosystem::Npm, &rest[1..]),
            "yarn" if rest.first() == Some(&"add") => (Ecosystem::Npm, &rest[1..]),
            "pip" | "pip3" if rest.first() == Some(&"install") => (Ecosystem::PyPi, &rest[1..]),
            "python" | "python3" if rest.len() >= 3 && rest[0] == "-m" && rest[1] == "pip" && rest[2] == "install" => (Ecosystem::PyPi, &rest[3..]),
            "cargo" if matches!(rest.first(), Some(&"add" | &"install")) => (Ecosystem::Crates, &rest[1..]),
            _ => continue,
        };
        let mut skip_next = false;

        for arg in args {
            if skip_next {
                skip_next = false;
                continue;
            }

            if arg.starts_with('-') {
                /* options that take a value as the next word */
                skip_next = matches!(*arg, "-r" | "--requirement" | "-c" | "--constraint" | "-e" | "--editable" | "-i" | "--index-url" | "--extra-index-url" | "--registry" | "--git" | "--path" | "--features" | "-F" | "--target" | "-t" | "--prefix" | "--root" | "--version" | "--vers" | "--rename" | "-p" | "--package");

                continue;
            }

            if let Some(name) = bare(ecosystem, arg) {
                if !found.contains(&Package { ecosystem, name: name.clone() }) {
                    found.push(Package { ecosystem, name });
                }
            }
        }
    }

    found
}

/// Looks a package up in its registry. Short timeout: this sits in front of a command the person is waiting for.
pub fn lookup(package: &Package) -> Exists {
    let encoded = package.name.replace('/', "%2F");
    let url = match package.ecosystem {
        Ecosystem::Npm => format!("https://registry.npmjs.org/{encoded}"),
        Ecosystem::PyPi => format!("https://pypi.org/pypi/{encoded}/json"),
        Ecosystem::Crates => format!("https://crates.io/api/v1/crates/{encoded}"),
    };
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(4)).build();

    match agent.get(&url).set("User-Agent", "sdc-slopguard (https://github.com/skilleddesk/SD-Code)").call() {
        Ok(_) => Exists::Yes,
        Err(ureq::Error::Status(404, _)) => Exists::No,
        Err(_) => Exists::Unknown,
    }
}

/// Checks every package an install command names with `ask` (the registry lookup, or a fake in tests).
pub fn check_with(line: &str, ask: impl Fn(&Package) -> Exists) -> Verdict {
    let mut verdict = Verdict::default();

    for package in packages(line) {
        match ask(&package) {
            Exists::Yes => {}
            Exists::No => verdict.missing.push(package),
            Exists::Unknown => verdict.unchecked.push(package),
        }
    }

    verdict
}

pub fn check(line: &str) -> Verdict {
    check_with(line, lookup)
}

#[cfg(test)]
mod tests;
