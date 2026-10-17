//! **The promise check (Phase 0.6).** A switch that is on by default promises behaviour. This test reads the daemon's own
//! source and fails when a default-on switch
//!
//! * is named nowhere outside the switch table in production code (nothing reads it - the switch does nothing), unless
//!   `INACTIVE` says so out loud, or
//! * is named in no test (nobody checks that what it promises happens).
//!
//! It is a text check, not a coverage tool: it cannot prove a test *exercises* the path, only that one names the switch.
//! That is the cheapest guard that would have caught `edit.transaction` (on, described as active, called by nothing).

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();

        if name.starts_with("._") || name == "target" {
            continue;
        }

        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

/// Where a file's production code ends: its `#[cfg(test)] mod`. A file under `tests/` or named `tests.rs` is all test.
fn split(path: &Path, text: &str) -> (String, String) {
    let all_test = path.components().any(|part| part.as_os_str() == "tests") || path.file_name().is_some_and(|name| name == "tests.rs" || name == "promise.rs");

    if all_test {
        return (String::new(), text.to_string());
    }

    /* The test module, not a test-only helper above it. */
    let module = text.find("#[cfg(test)]\nmod ").or_else(|| text.find("#[cfg(test)]\r\nmod "));

    match module {
        Some(at) => (text[..at].to_string(), text[at..].to_string()),
        None => (text.to_string(), String::new()),
    }
}

fn names_switch(text: &str, feature: &str) -> bool {
    let quoted = format!("\"{feature}\"");

    text.lines().any(|line| line.contains(&quoted) && !line.trim_start().starts_with("//"))
}

#[test]
fn every_default_on_switch_has_a_production_path_and_a_test() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();

    rust_files(&root.join("src"), &mut files);
    rust_files(&root.join("tests"), &mut files);

    let mut production = String::new();
    let mut tests = String::new();

    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let (before, after) = split(file, &text);

        /* The switch table itself names every switch; it is not a use. */
        if file.ends_with("v3/mod.rs") {
            let table_end = before.find("];").unwrap_or(0);

            production.push_str(&before[table_end..]);
        } else {
            production.push_str(&before);
        }

        tests.push_str(&after);
    }

    let mut no_path = Vec::new();
    let mut no_test = Vec::new();

    for feature in crate::v3::FEATURES.iter().filter(|feature| feature.default_on) {
        if !names_switch(&production, feature.name) && crate::v3::inactive(feature.name).is_none() {
            no_path.push(feature.name);
        }

        if !names_switch(&tests, feature.name) && crate::v3::inactive(feature.name).is_none() {
            no_test.push(feature.name);
        }
    }

    assert!(no_path.is_empty(), "default-on switches that no production code reads (fix them, or list them in v3::INACTIVE): {no_path:?}");
    assert!(no_test.is_empty(), "default-on switches that no test names (add a test, or list them in v3::INACTIVE): {no_test:?}");
}

#[test]
fn an_inactive_switch_names_a_real_switch_and_gives_a_reason() {
    for (name, why) in crate::v3::INACTIVE {
        assert!(crate::v3::FEATURES.iter().any(|feature| feature.name == *name), "{name} is not a switch");
        assert!(why.len() > 40, "{name} needs a reason a person can read");
    }

    assert!(crate::v3::FEATURES.iter().find(|feature| feature.name == "edit.transaction").unwrap().what.contains("INACTIVE"));
}
