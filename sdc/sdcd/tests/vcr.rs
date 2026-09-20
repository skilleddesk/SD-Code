//! The VCR replay test (master spec section 11.6).
//!
//! "Each engine's fixture replay produces identical UI events to a live run" is the acceptance item,
//! and this is it in testable form: every fixture in `tests/vcr/` is replayed through the *same*
//! parser the daemon uses for that engine, and the resulting event kinds are compared with the
//! kinds the fixture itself declares on its first line.
//!
//! The fixtures are therefore a contract, not a recording: if an adapter starts emitting an event it
//! did not before - or stops emitting one - the fixture has to be regenerated deliberately, and the
//! diff is the conversation about whether the change was wanted. Twelve ordinary turns are here
//! plus `native-13-user-pastes-screenshot.jsonl`, the vision path where a turn starts from a pasted
//! image rather than a typed prompt.

use std::fs;
use std::path::{Path, PathBuf};

use sdcd::engines::ollama::parse_chat_line;
use sdcd::engines::parse_stream_line;
use sdcd::engines::native_api::parse_sse;
use sdcd::engines::EngineEvent;

/// The fixture directory, relative to this test file's crate root.
fn fixtures() -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("vcr");
    let mut files: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("reading {}: {error}", directory.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().map(|extension| extension == "jsonl").unwrap_or(false))
        .collect();

    files.sort();

    files
}

/// The event kind, as the fixture's expectation spells it.
fn kind_of(event: &EngineEvent) -> &'static str {
    match event {
        EngineEvent::Delta(_) => "Delta",
        EngineEvent::Thinking(_) => "Thinking",
        EngineEvent::ToolStarted { .. } => "ToolStarted",
        EngineEvent::ToolOutput { .. } => "ToolOutput",
        EngineEvent::ToolCompleted { .. } => "ToolCompleted",
        EngineEvent::Failed(_) => "Failed",
        EngineEvent::Done { .. } => "Done",
    }
}

/// Replays one fixture with the parser its engine uses.
fn replay(path: &Path) -> Vec<String> {
    let raw = fs::read_to_string(path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    let mut lines = raw.lines().filter(|line| !line.trim().is_empty());
    let header: serde_json::Value = serde_json::from_str(lines.next().unwrap_or("{}")).expect("a vcr header line");
    let engine = header.get("engine").and_then(|value| value.as_str()).unwrap_or("claude_code");
    let body: Vec<String> = lines.map(str::to_string).collect();

    let events: Vec<EngineEvent> = match engine {
        "ollama" => body.iter().flat_map(|line| parse_chat_line(line)).collect(),
        "native_api" => parse_sse(&body),
        _ => body.iter().flat_map(|line| parse_stream_line(line)).collect(),
    };

    events.iter().map(kind_of).map(str::to_string).collect()
}

/// The kinds the fixture expects, from its header line.
fn expected(path: &Path) -> Vec<String> {
    let raw = fs::read_to_string(path).unwrap();

    serde_json::from_str::<serde_json::Value>(raw.lines().next().unwrap())
        .unwrap()["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kind| kind.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn there_are_thirteen_fixtures_including_the_vision_path() {
    let files = fixtures();

    assert_eq!(files.len(), 13, "twelve turns plus the screenshot path");
    assert!(files.iter().any(|path| path.file_name().unwrap().to_string_lossy().starts_with("native-13")));
}

#[test]
fn every_fixture_replays_to_the_kinds_it_declares() {
    for path in fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        assert_eq!(replay(&path), expected(&path), "fixture {name} replayed differently");
    }
}

#[test]
fn every_fixture_ends_at_a_terminal_event() {
    for path in fixtures() {
        let kinds = replay(&path);
        let last = kinds.last().map(String::as_str).unwrap_or_default();

        assert!(last == "Done" || last == "Failed", "{} ended at {last}", path.display());
    }
}

#[test]
fn the_vision_fixture_carries_both_text_and_thinking() {
    let kinds = replay(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vcr/native-13-user-pastes-screenshot.jsonl"));

    assert!(kinds.contains(&"Thinking".to_string()));
    assert!(kinds.contains(&"Delta".to_string()));
}

#[test]
fn a_changed_stream_shows_up_as_a_changed_replay() {
    /* The negative control: the fixtures cannot pass a fixture they do not describe. */
    let mutated: Vec<EngineEvent> = parse_stream_line(r#"{"type":"assistant_text","text":"extra"}"#);
    let kinds: Vec<String> = mutated.iter().map(kind_of).map(str::to_string).collect();

    assert_eq!(kinds, vec!["Delta".to_string()]);
    assert_ne!(kinds, expected(&fixtures()[0]));
}
