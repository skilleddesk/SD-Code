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

/// The names a text asks `on(..)`, `on_for(..)`, `turn_on(.., ..)` or `technique(..)` about.
fn asked_names(text: &str) -> Vec<String> {
    let mut found = Vec::new();

    for marker in ["on(\"", "on_for(\"", "technique(\""] {
        let mut from = 0;

        while let Some(at) = text[from..].find(marker) {
            let start = from + at;
            let before = text[..start].chars().last();
            let name_from = start + marker.len();

            /* `on(` must be the function, not the tail of another name (`turn_on(`, `button(`): only a `::` or a space/paren/`!` before it */
            let function = marker != "on(\"" || !before.is_some_and(|c| c.is_alphanumeric() || c == '_');

            if function {
                if let Some(end) = text[name_from..].find('"') {
                    found.push(text[name_from..name_from + end].to_string());
                }
            }

            from = name_from;
        }
    }

    let mut from = 0;

    while let Some(at) = text[from..].find("turn_on(") {
        let args = &text[from + at + 8..(from + at + 8 + 80).min(text.len())];
        let end = args.find(')').unwrap_or(args.len());

        if let Some(open) = args[..end].rfind('"') {
            if let Some(close) = args[..open].rfind('"') {
                found.push(args[close + 1..open].to_string());
            }
        }

        from += at + 8;
    }

    found
}

/// Phase 0.6 caught a switch nothing reads. This catches the other half: code that asks about a switch **nobody registered** - `on()` answers
/// `false` for it, so the feature behind it is dead without any test failing (3.1: working memory, `sme.memory`, was exactly this).
#[test]
fn every_switch_the_code_asks_about_is_registered() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();

    rust_files(&root.join("src"), &mut files);

    let mut unknown: Vec<String> = Vec::new();

    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let (production, _) = split(file, &text);

        for name in asked_names(&production) {
            let plausible = !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.');

            if plausible && !crate::v3::FEATURES.iter().any(|feature| feature.name == name) && !unknown.contains(&name) {
                unknown.push(format!("{name} ({})", file.file_name().and_then(|name| name.to_str()).unwrap_or_default()));
            }
        }
    }

    assert!(unknown.is_empty(), "the code asks about switches that are not in v3::FEATURES (they are always off): {unknown:?}");
}

#[test]
fn the_name_reader_finds_what_the_code_asks_and_nothing_else() {
    let text = r#"if crate::v3::on("sme.zzz") && v3::on_for("a.b", p, m) { technique("t_1"); crate::v3::turn_on(&turn_id, "skel"); button("not.a.switch"); }"#;

    assert_eq!(asked_names(text), ["sme.zzz", "a.b", "t_1", "skel"]);
    assert!(!crate::v3::FEATURES.iter().any(|feature| feature.name == "sme.zzz"), "the sample name really is unregistered");
}

#[test]
fn an_inactive_switch_names_a_real_switch_and_gives_a_reason() {
    for (name, why) in crate::v3::INACTIVE {
        assert!(crate::v3::FEATURES.iter().any(|feature| feature.name == *name), "{name} is not a switch");
        assert!(why.len() > 40, "{name} needs a reason a person can read");
    }

    /* 3.1: `edit.transaction` was listed here until the Step Runtime wired it into the edit tools; it must not be listed while it is live. */
    assert!(crate::v3::inactive("edit.transaction").is_none());
    assert!(!crate::v3::FEATURES.iter().find(|feature| feature.name == "edit.transaction").unwrap().what.contains("INACTIVE"));
}

/// Q3 (M0-4): a switch that is **on** by default either needs no measurement (it protects, verifies or records), or was
/// measured on the bench, or is unmeasured and therefore applies to local models only and says so in its description. An
/// unmeasured change to a prompt must not be what a cloud model reads.
#[test]
fn every_default_on_switch_is_measured_safe_or_local_only_and_honest_about_it() {
    use crate::v3::{Evidence, Scope};

    let mut wrong = Vec::new();

    for feature in crate::v3::FEATURES.iter().filter(|feature| feature.default_on) {
        let honest = match feature.evidence {
            Evidence::Bench { label, models } => !label.is_empty() && !models.is_empty(),
            Evidence::Safety => true,
            Evidence::None => feature.scope == Scope::LocalOnly && feature.what.contains("unmeasured"),
        };

        if !honest {
            wrong.push(feature.name);
        }
    }

    assert!(wrong.is_empty(), "default-on switches with no evidence that are not local-only and marked unmeasured: {wrong:?}");

    /* The reverse: nothing that says it is unmeasured claims evidence, and a token technique is never `Safety` (it changes what a model is told). */
    for feature in crate::v3::FEATURES {
        if feature.what.contains("unmeasured") {
            assert!(matches!(feature.evidence, Evidence::None), "{} says unmeasured and claims evidence", feature.name);
        }

        if feature.token_technique {
            assert!(!matches!(feature.evidence, Evidence::Safety), "{} is a token technique: it needs a bench result, not a safety argument", feature.name);
        }
    }
}

/// M0-3: a local-only switch is off for a cloud, API or CLI model whatever it is set to, and a local model sees it as set.
#[test]
fn a_local_only_switch_never_reaches_a_cloud_model() {
    use crate::v3::{ModelCtx, Scope};

    let local = ModelCtx::new(true);
    let cloud = ModelCtx::cloud();

    for feature in crate::v3::FEATURES.iter().filter(|feature| feature.scope == Scope::LocalOnly) {
        assert!(!crate::v3::forcing(&[feature.name], || crate::v3::on_in(feature.name, &cloud)), "{} reached a cloud model", feature.name);
        assert!(crate::v3::forcing(&[feature.name], || crate::v3::on_in(feature.name, &local)), "{} did not reach a local model", feature.name);
    }

    /* An `All` switch is not narrowed. */
    assert!(crate::v3::forcing(&["edit.transaction"], || crate::v3::on_in("edit.transaction", &cloud)));
}

/// The classes a `Classes` scope names are the only local models it reaches.
#[test]
fn a_class_scope_reaches_only_its_classes() {
    use crate::sme::budget::Class;
    use crate::v3::{ModelCtx, Scope};

    let small_and_medium = Scope::Classes(&[Class::Small, Class::Medium]);

    assert!(small_and_medium.allows(&ModelCtx::local(8_192)));
    assert!(small_and_medium.allows(&ModelCtx::local(32_768)));
    assert!(!small_and_medium.allows(&ModelCtx::local(128_000)));
    assert!(!small_and_medium.allows(&ModelCtx::cloud()));
}
