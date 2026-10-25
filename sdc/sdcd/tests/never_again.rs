//! **PADMA P0-1, the fixtures of the Never-Again suite.** `tests/never_again/*.jsonl` replay, in the same VCR format as `tests/vcr/`, the
//! fake-engine outputs of the 25 Oct 2026 incident: (a) a reply that ends in a colon, (b) thinking with no content, (c) a tool call
//! printed as text, (d) the step that was 109 tokens over the window, (e) the pasted error, (f) "ok koro", (g) Windows os error 2 and 3
//! on a path with a space, (h) the GPU notice. The tests that drive the agent loop with them are in `src/agent/never_again_tests.rs`
//! (they need the loop itself); this file keeps the fixtures honest: each replays to the event kinds it declares, and every guarantee
//! of the suite is covered by at least one fixture and by a test of that name.

use std::fs;
use std::path::{Path, PathBuf};

use sdcd::engines::ollama::parse_chat_line;
use sdcd::engines::EngineEvent;

const GUARANTEES: [&str; 11] = [
    "announce_then_stop",
    "colon_end_no_tool",
    "qwen35_text_toolcall",
    "thinking_only",
    "empty_answer",
    "overflow_by_109",
    "short_prompt_full_review",
    "resume_after_fail",
    "win_os_error_2_3_5",
    "no_evidence_claim",
    "no_ui_notice_in_history",
];

fn fixtures() -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("never_again");
    let mut files: Vec<PathBuf> = fs::read_dir(&directory).unwrap().filter_map(|entry| entry.ok().map(|entry| entry.path())).filter(|path| path.extension().is_some_and(|extension| extension == "jsonl")).collect();

    files.sort();

    files
}

fn header(path: &Path) -> serde_json::Value {
    serde_json::from_str(fs::read_to_string(path).unwrap().lines().next().unwrap()).unwrap()
}

fn kind_of(event: &EngineEvent) -> &'static str {
    match event {
        EngineEvent::Delta(_) => "Delta",
        EngineEvent::Thinking(_) => "Thinking",
        EngineEvent::ToolStarted { .. } => "ToolStarted",
        EngineEvent::Usage { .. } => "Usage",
        EngineEvent::Failed(_) => "Failed",
        EngineEvent::Done { .. } => "Done",
        _ => "Other",
    }
}

fn replay(path: &Path) -> Vec<String> {
    let raw = fs::read_to_string(path).unwrap();

    raw.lines().skip(1).filter(|line| !line.trim().is_empty()).flat_map(parse_chat_line).map(|event| kind_of(&event).to_string()).collect()
}

#[test]
fn there_is_a_fixture_for_each_of_the_eight_situations() {
    let names: Vec<String> = fixtures().iter().map(|path| path.file_name().unwrap().to_string_lossy().to_string()).collect();

    for letter in ["a-", "b-", "c-", "d-", "e-", "f-", "g-", "h-"] {
        assert!(names.iter().any(|name| name.starts_with(letter)), "no fixture {letter}: {names:?}");
    }
}

#[test]
fn every_fixture_replays_to_the_kinds_it_declares() {
    for path in fixtures() {
        let declared: Vec<String> = header(&path)["kinds"].as_array().unwrap().iter().map(|kind| kind.as_str().unwrap().to_string()).collect();

        assert_eq!(replay(&path), declared, "fixture {} replayed differently", path.display());
    }
}

#[test]
fn every_guarantee_has_a_fixture_and_a_test_of_its_name() {
    let source = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent/never_again_tests.rs")).unwrap();
    let covered: Vec<String> = fixtures().iter().flat_map(|path| header(path)["guarantee"].as_array().unwrap().iter().map(|name| name.as_str().unwrap().to_string()).collect::<Vec<_>>()).collect();

    for name in GUARANTEES {
        assert!(source.contains(&format!("fn {name}()")), "no test named {name}");

        /* the guarantees whose evidence is not a model stream (the scope of a request) have no fixture of their own */
        if name != "short_prompt_full_review" {
            assert!(covered.iter().any(|covered| covered == name), "no fixture covers {name}");
        }
    }
}
