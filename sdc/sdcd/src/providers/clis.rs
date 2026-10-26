//! **Which models the installed CLI accepts** - model discovery for the subscription providers (3.2).
//!
//! The three subscription cards (Claude Code, Codex, Gemini CLI) had no live model list at all. `models.json` gave
//! each of them a block with **no `live` endpoint**, so `models::live` answered *"this provider does not list its
//! models"* every time and the menu fell back to whatever this build was shipped with. That is the report: *"after
//! adding CLIs, SDC shows old/bad model lists"*, and *"the newer Gemini models the CLI offers never appear"*.
//!
//! The honest reason is that **none of the three CLIs has a `models list` command.** Measured here on the installed
//! ones:
//!
//! ```text
//! $ claude --help     ... --model <model>  "an alias for the latest model (e.g. 'fable', 'opus', 'sonnet')
//!                                           or a model's full name (e.g. 'claude-fable-5')"
//!                         subcommands: agents attach auth doctor gateway import install logs mcp plugin
//!                                      project respawn rm setup-token stop ultrareview update   (no `models`)
//! $ codex --help      ... subcommands: agents exec review login logout mcp plugin app-server ...  (no `models`)
//! $ gemini --help     ... -m, --model                                                            (no `models`)
//! ```
//!
//! So the list is taken from where the CLI itself keeps it: **its own program files.** A CLI that accepts
//! `--model gemini-3.8-flash` has that string inside it, and the moment the person updates the CLI the new ids are
//! there - which is the property that was asked for, "new models appear automatically with zero manual work".
//!
//! This is a *scan*, and a scan needs discipline or it invents models. Three rules keep it honest:
//!
//! 1. **a tight shape per provider.** Claude's ids are `claude-<line>-<major>[-<minor>][-<date>]` with a one-digit
//!    minor, which is why `claude-haiku-3-55` (the bytes `claude-haiku-3-5` followed by a `5`) is not accepted;
//! 2. **delimited on both sides.** The byte after the id must not be one an id can contain, so a string that runs
//!    into the next one in a binary (`gpt-5.6-terraglobal.openai.`) is thrown away rather than truncated;
//! 3. **[`catalog::plausible`] and [`catalog::chatty`] on top**, which drop the CLI's own fixtures, its
//!    documentation filenames and the models a coding chat cannot run.
//!
//! Codex's binary is a Rust executable whose string table runs the literals together, so the first loose scan of it
//! produced `gpt-5.1-codex-ma`, `gpt-6-as` and `gpt-5.6-lerra` beside the real ids. Rules 2 and 3 above remove the
//! first and the third (neither appears delimited even once), and a fourth rule removes the second: **every word after
//! the version is at least three letters, and an id has at most four parts.** Measured on the installed
//! `codex-cli 0.155.1`, that leaves exactly the real ones - `gpt-5.1-codex-max`, `gpt-5.3-codex`, `gpt-5.6-luna`,
//! `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-6`, `gpt-6-astra`, `gpt-6-pro`, `gpt-5.2`, `gpt-5.4`, `gpt-5.5`, `gpt-4.1` -
//! and nothing invented. The person's own `~/.codex/config.toml` is read as well, because a model they already run is
//! one they can pick.

use std::path::{Path, PathBuf};

use super::catalog;

/// How one CLI's models are found.
struct Scan {
    /// The provider card this CLI belongs to - `openai` is the card, `codex` the program.
    provider: &'static str,
    /// The program, as `host::program::resolve` knows it.
    program: &'static str,
    /// Where the ids live, relative to the directory the program resolved in (an npm shim's own folder) or to the
    /// program itself. The first candidate that exists is used.
    payloads: &'static [&'static str],
    /// The prefix every id of this provider starts with.
    prefix: &'static str,
    /// Does the text after the prefix have this provider's shape?
    shape: fn(&str) -> bool,
}

/// `claude-<line>-<major>[-<minor>][-<yyyymmdd>]`, and nothing else.
///
/// The lines are not hard-coded as a closed set by accident: Anthropic adds them (`fable`, `mythos` came after
/// `opus`), and an unknown word here would let any `claude-something-4` string through. A new line is one word in this
/// list; until then its models still arrive from the curated bundle and from the API's own list.
fn claude_shape(rest: &str) -> bool {
    let mut parts = rest.split('-');
    let line = parts.next().unwrap_or_default();

    if !["opus", "sonnet", "haiku", "fable", "mythos"].contains(&line) {
        return false;
    }

    let numeric = |part: &str, len: std::ops::RangeInclusive<usize>| {
        !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) && len.contains(&part.len())
    };
    let Some(major) = parts.next() else {
        return false;
    };

    if !numeric(major, 1..=2) {
        return false;
    }

    match (parts.next(), parts.next(), parts.next()) {
        (None, _, _) => true,
        /* A one-digit minor, or a dated snapshot straight after the major. */
        (Some(next), None, _) => numeric(next, 1..=1) || numeric(next, 8..=8),
        (Some(minor), Some(date), None) => numeric(minor, 1..=1) && numeric(date, 8..=8),
        _ => false,
    }
}

/// `gemini-<version>[-<word>...]`, where the version comes **first**.
///
/// Looser than Claude's on purpose - Google's ids are `3.1-pro`, `4-argon`, `omni-1.1-flash` and `3.8-flash-lite`,
/// four different shapes - and safe because the file being scanned is **text** (the CLI ships its bundle as
/// JavaScript), so a string does not run into its neighbour the way it does in a compiled binary.
///
/// "The version comes first" is what tells a model from the CLI's other `gemini-` names: the installed bundle holds
/// `gemini-cli-a2a`, which has a digit in it and is a protocol, not a model.
fn gemini_shape(rest: &str) -> bool {
    let first = rest.split('-').next().unwrap_or_default();
    let number = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let versioned = match first.split_once('.') {
        Some((major, minor)) => number(major) && number(minor),
        None => number(first),
    };

    (versioned || first == "omni")
        && rest.len() >= 2
        && rest.bytes().any(|byte| byte.is_ascii_digit())
        && rest.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-')
        && !rest.contains("..")
        && !rest.ends_with('-')
}

/// `gpt-<major>[.<minor>]` and at most two words after it, each at least three letters.
///
/// The three-letter floor is what tells `gpt-6-astra` from `gpt-6-as`, and the four-part ceiling what tells
/// `gpt-5.1-codex-max` from `gpt-5.6-lunacodex-auto-review`. Both of those were real strings in the installed binary.
fn codex_shape(rest: &str) -> bool {
    let mut parts = rest.split('-');
    let Some(version) = parts.next() else {
        return false;
    };
    let number = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());

    let versioned = match version.split_once('.') {
        Some((major, minor)) => number(major) && number(minor),
        None => number(version),
    };

    if !versioned {
        return false;
    }

    let words: Vec<&str> = parts.collect();

    words.len() <= 2 && words.iter().all(|word| word.len() >= 3 && word.bytes().all(|byte| byte.is_ascii_lowercase()))
}

const SCANS: &[Scan] = &[
    Scan {
        provider: "claude",
        program: "claude",
        /* The npm shim is a batch file next to the package; the native build is the program itself. */
        payloads: &["node_modules/@anthropic-ai/claude-code/bin/claude.exe", "node_modules/@anthropic-ai/claude-code/bin/claude", "../lib/node_modules/@anthropic-ai/claude-code/bin/claude", ""],
        prefix: "claude-",
        shape: claude_shape,
    },
    Scan {
        provider: "gemini",
        program: "gemini",
        payloads: &["node_modules/@google/gemini-cli/bundle", "../lib/node_modules/@google/gemini-cli/bundle", ""],
        prefix: "gemini-",
        shape: gemini_shape,
    },
    Scan {
        provider: "openai",
        program: "codex",
        /* npm installs the native binary in a per-platform package; `*` stands for the platform and the target triple. */
        payloads: &[
            "node_modules/@openai/codex/node_modules/@openai/codex-*/vendor/*/bin/codex.exe",
            "node_modules/@openai/codex/node_modules/@openai/codex-*/vendor/*/bin/codex",
            "../lib/node_modules/@openai/codex/node_modules/@openai/codex-*/vendor/*/bin/codex",
            "",
        ],
        prefix: "gpt-",
        shape: codex_shape,
    },
];

/// The bytes an id may contain. Anything else ends it - and the byte that ends it must be one of these *not*, which is
/// how a string that runs into the next one in a binary is recognised and dropped.
fn id_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
}

/// Every `prefix…` run in `bytes` that is delimited on both sides and passes `shape`.
///
/// Byte-level rather than a regex over a string: the files are a 230 MB executable and a 70 MB bundle, and neither is
/// valid UTF-8 as a whole.
fn ids_in(bytes: &[u8], prefix: &str, shape: fn(&str) -> bool) -> Vec<String> {
    let needle = prefix.as_bytes();
    let mut found: Vec<String> = Vec::new();
    let mut at = 0usize;

    while at + needle.len() <= bytes.len() {
        if &bytes[at..at + needle.len()] != needle {
            at += 1;

            continue;
        }

        /* The byte before must not be part of an id either, or `xclaude-opus-5` would count. */
        let clean_start = at == 0 || !id_byte(bytes[at - 1]);
        let mut end = at + needle.len();

        while end < bytes.len() && id_byte(bytes[end]) {
            end += 1;
        }

        if clean_start && end > at + needle.len() {
            if let Ok(rest) = std::str::from_utf8(&bytes[at + needle.len()..end]) {
                let id = format!("{prefix}{rest}");

                if shape(rest) && catalog::plausible(&id) && !found.contains(&id) {
                    found.push(id);
                }
            }
        }

        at = end.max(at + 1);
    }

    found
}

/// Does `name` match a one-`*` pattern? (`codex-*` matches `codex-win32-x64`; `*` matches anything.)
fn matches(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, tail)) => name.len() >= head.len() + tail.len() && name.starts_with(head) && name.ends_with(tail),
    }
}

/// `folder` joined with a relative path whose components may hold a `*`, as every path that exists.
///
/// A star matches one directory name, which is all the layouts here need: npm puts a CLI's native binary in a
/// per-platform package (`@openai/codex-win32-x64`) under a per-target-triple folder (`x86_64-pc-windows-msvc`), and
/// naming either of them in this file would make the scan work on one computer and not on the next.
fn expand(folder: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut here = vec![folder.to_path_buf()];

    for component in pattern.split('/').filter(|part| !part.is_empty()) {
        let mut next = Vec::new();

        for path in here {
            if component.contains('*') {
                if let Ok(entries) = std::fs::read_dir(&path) {
                    let mut children: Vec<PathBuf> = entries
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|child| child.file_name().and_then(|name| name.to_str()).is_some_and(|name| matches(component, name)))
                        .collect();

                    children.sort();
                    next.extend(children);
                }
            } else {
                next.push(path.join(component));
            }
        }

        here = next;
    }

    here.into_iter().filter(|path| path.exists()).collect()
}

/// The payload to scan for a CLI: the file itself, or every `.js` of a bundle directory.
fn payload_files(scan: &Scan) -> Vec<PathBuf> {
    let Some(program) = crate::host::program::resolve(scan.program) else {
        return Vec::new();
    };
    let folder = program.parent().map(Path::to_path_buf).unwrap_or_default();

    for candidate in scan.payloads {
        let paths = if candidate.is_empty() { vec![program.clone()] } else { expand(&folder, candidate) };

        for path in paths {
            if path.is_file() {
                return vec![path];
            }

            if path.is_dir() {
                let mut files: Vec<PathBuf> = std::fs::read_dir(&path)
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|entry| entry.path())
                            .filter(|path| path.extension().is_some_and(|extension| extension == "js"))
                            .collect()
                    })
                    .unwrap_or_default();

                files.sort();

                if !files.is_empty() {
                    return files;
                }
            }
        }
    }

    Vec::new()
}

/// The models the person's own Codex configuration names (`~/.codex/config.toml`): `model = "gpt-5.6-codex"`, and the
/// same key inside a `[profiles.*]` table. Nothing is written, and a configuration that is not there is not an error.
fn codex_configured() -> Vec<String> {
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(PathBuf::from(home).join(".codex").join("config.toml")).unwrap_or_default();
    let mut found: Vec<String> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("model") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let id = value.trim().trim_matches(['"', '\'']).to_string();

        if !id.is_empty() && catalog::plausible(&id) && catalog::chatty(&id) && !found.contains(&id) {
            found.push(id);
        }
    }

    found
}

/// The model ids the installed CLI of `provider_id` accepts, newest-looking first.
///
/// `Err` when the CLI is not installed, or installed in a shape this build cannot read - and that sentence is what the
/// model list shows as its note, so a stale menu always says why.
pub fn model_ids(provider_id: &str) -> Result<Vec<String>, String> {
    let Some(scan) = SCANS.iter().find(|scan| scan.provider == provider_id) else {
        return Err(format!("{provider_id} does not sign in through a CLI"));
    };
    /* The models the person already runs count, whether or not the scan finds them. */
    let mut found: Vec<String> = if provider_id == "openai" { codex_configured() } else { Vec::new() };

    if crate::host::program::resolve(scan.program).is_none() {
        return Err(format!("`{}` is not installed on this computer, so its model list is the bundled one", scan.program));
    }

    let files = payload_files(scan);

    if files.is_empty() {
        return Err(format!("`{}` is installed but its program files could not be found, so its model list is the bundled one", scan.program));
    }

    for file in files {
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };

        for id in ids_in(&bytes, scan.prefix, scan.shape) {
            if catalog::chatty(&id) && !found.contains(&id) {
                found.push(id);
            }
        }
    }

    if found.is_empty() {
        return Err(format!("`{}` named no model this build recognises, so its model list is the bundled one", scan.program));
    }

    /* Newest first, so the menu opens on the model a person most likely wants. */
    found.sort_by(|left, right| {
        catalog::version(&catalog::key(right))
            .cmp(&catalog::version(&catalog::key(left)))
            .then_with(|| left.len().cmp(&right.len()))
            .then_with(|| left.cmp(right))
    });

    Ok(found)
}

/// Do we have a way to list this provider's models from the machine itself?
pub fn can_list(provider_id: &str) -> bool {
    matches!(provider_id, "claude" | "gemini" | "openai")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude's shape, against the strings a scan of the real `claude` program turns up.
    ///
    /// The last group is the reason the shape is this tight: those are not models, they are the bytes of a real id
    /// running into whatever the compiler put next to it.
    #[test]
    fn claudes_shape_accepts_its_ids_and_nothing_near_them() {
        for real in ["opus-5-5", "opus-5", "sonnet-5-5", "sonnet-5", "haiku-5-5", "haiku-4-5", "haiku-4-5-20251001", "fable-5-1", "opus-4-1-20250805", "opus-4", "mythos-5-1"] {
            assert!(claude_shape(real), "claude-{real} is a model");
        }

        for junk in ["haiku-3-55", "opus-4-5-20251101-v1", "opus", "code", "sonnet-x", "sonnet-5-5-5", "dev-4-5", ""] {
            assert!(!claude_shape(junk), "claude-{junk} is not a model");
        }
    }

    /// The scanner: delimited on both sides, or not taken at all.
    #[test]
    fn an_id_that_runs_into_the_next_string_is_dropped() {
        let bytes = b"\x00claude-opus-5-5\x00xclaude-opus-4\x00claude-sonnet-5-5globalclaude\x00claude-haiku-4-5-20251001\"";
        let found = ids_in(bytes, "claude-", claude_shape);

        assert_eq!(found, ["claude-opus-5-5", "claude-haiku-4-5-20251001"]);
    }

    /// Gemini's shape over the bundle's own text, including the fixture it ships.
    #[test]
    fn geminis_shape_accepts_the_ids_the_cli_offers() {
        let bytes = b"'gemini-3.1-pro-preview','gemini-4-argon',\"gemini-3.8-flash\" gemini-omni-1.1-flash gemini-9001-super-duper gemini-3.md gemini-3.8-flash-tts gemini-cli-a2a";
        let found: Vec<String> = ids_in(bytes, "gemini-", gemini_shape).into_iter().filter(|id| catalog::chatty(id)).collect();

        assert_eq!(found, ["gemini-3.1-pro-preview", "gemini-4-argon", "gemini-3.8-flash", "gemini-omni-1.1-flash"]);
    }

    /// Codex's shape, against the strings the installed `codex.exe` really holds.
    ///
    /// `gpt-6-as` and `gpt-5.6-lunacodex-auto-review` are both in that binary, and both are a real id with its
    /// neighbour's bytes attached. They are the reason this shape has a floor and a ceiling.
    #[test]
    fn codexs_shape_tells_a_model_from_its_neighbours_bytes() {
        for real in ["5.1-codex-max", "5.3-codex", "5.6-luna", "5.6-sol", "5.6-terra", "6", "6-astra", "6-pro", "5.2", "4.1"] {
            assert!(codex_shape(real), "gpt-{real} is a model");
        }

        for junk in ["6-as", "5.6-lunacodex-auto-review", "5.1-codex-ma", "4o-mini-transcribebackg", "", "x", "5.6-I3E"] {
            assert!(!codex_shape(junk), "gpt-{junk} is not a model");
        }
    }

    /// A provider with no CLI says so in a sentence the model list can show.
    #[test]
    fn a_provider_without_a_cli_is_refused_by_name() {
        let error = model_ids("deepseek").unwrap_err();

        assert!(error.contains("deepseek") && error.contains("does not sign in through a CLI"), "{error}");
        assert!(can_list("claude") && can_list("gemini") && can_list("openai"));
        assert!(!can_list("anthropic-api"));
    }

    /// On this machine the answer is either the ids or a sentence saying why not - never an empty success.
    ///
    /// It runs wherever the suite runs, which is the point: a CI runner has no CLI installed and must get the sentence,
    /// and a developer's machine that has one must get ids that pass every filter.
    #[test]
    fn the_answer_is_either_ids_or_a_reason() {
        for provider in ["claude", "gemini", "openai"] {
            match model_ids(provider) {
                Ok(ids) => {
                    /* Printed because the list is the thing being claimed: `cargo test -- --nocapture` shows what
                       this computer's CLIs really offer, which is how the scan was checked against the three. */
                    println!("{provider}: {}", ids.join(", "));

                    assert!(!ids.is_empty(), "{provider} answered Ok with nothing");

                    for id in &ids {
                        assert!(catalog::plausible(id), "{provider} offered {id}");
                        assert!(catalog::chatty(id), "{provider} offered {id}, which cannot take a turn");
                    }
                }
                Err(reason) => {
                    println!("{provider}: {reason}");

                    assert!(reason.len() > 20, "{provider}: {reason}");
                }
            }
        }
    }
}

