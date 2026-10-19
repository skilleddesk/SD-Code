use super::*;

fn note(seq: i64, kind: &str, text: &str, pinned: bool) -> Note {
    Note { seq, kind: kind.into(), text: text.into(), source: String::new(), quote: String::new(), pinned }
}

fn daemon() -> std::sync::Arc<crate::DaemonState> {
    crate::DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon")
}

#[test]
fn a_task_keeps_a_capped_number_of_each_kind_and_the_oldest_goes_first() {
    let state = daemon();
    let store = &state.store;

    for n in 1..=7 {
        store.add_note("t1", "decision", &format!("decision {n}"), "a.rs", "", false).unwrap();
    }

    let kept: Vec<String> = store.notes("t1").unwrap().into_iter().map(|note| note.text).collect();

    assert_eq!(kept, vec!["decision 3", "decision 4", "decision 5", "decision 6", "decision 7"], "the newest five decisions are kept");
    assert_eq!(cap_of("decision"), 5);

    for n in 1..=10 {
        store.add_note("t1", "failure", &format!("failure {n}"), "", "", false).unwrap();
    }

    assert_eq!(store.notes("t1").unwrap().iter().filter(|note| note.kind == "failure").count(), 8);
    assert_eq!(store.notes("t1").unwrap()[0].seq, 3, "the seq keeps counting: a note's identity does not move");
}

#[test]
fn a_pinned_note_is_never_replaced_and_the_person_can_pin_drop_and_clear() {
    let state = daemon();
    let store = &state.store;
    let first = store.add_note("t2", "fact", "the person wants the tests to keep passing", "", "", true).unwrap();

    store.add_note("t2", "fact", "a second fact", "", "", false).unwrap();

    let facts: Vec<String> = store.notes("t2").unwrap().into_iter().filter(|note| note.kind == "fact").map(|note| note.text).collect();

    assert_eq!(facts, vec!["the person wants the tests to keep passing"], "the pinned fact outlives the newer one");

    /* unpinned, the old fact is an ordinary note again: the cap of one replaces it with the newer one */
    store.set_note_pinned("t2", first, false).unwrap();

    let second = store.add_note("t2", "fact", "now this is the fact", "", "", false).unwrap();
    let facts: Vec<String> = store.notes("t2").unwrap().into_iter().filter(|note| note.kind == "fact").map(|note| note.text).collect();

    assert_eq!(facts, vec!["now this is the fact"], "one fact, and it is the newest");
    assert!(!store.drop_note("t2", first).unwrap(), "the old fact was already replaced");
    assert!(store.drop_note("t2", second).unwrap(), "a note the person points at is dropped");
    assert!(!store.drop_note("t2", 999).unwrap(), "nothing to drop");
    assert_eq!(store.clear_notes("t2").unwrap(), 0);
    assert!(store.notes("t2").unwrap().is_empty());

    store.add_note("t2", "decision", "kept", "x.rs", "", false).unwrap();
    assert_eq!(store.clear_notes("t2").unwrap(), 1);
    assert!(store.clear_notes("t2").unwrap() == 0, "clearing an empty memory is not an error");
}

#[test]
fn one_task_never_sees_anothers_memory() {
    let state = daemon();
    let store = &state.store;

    store.add_note("a", "decision", "for a", "x.rs", "", false).unwrap();
    store.add_note("b", "decision", "for b", "y.rs", "", false).unwrap();

    assert_eq!(store.notes("a").unwrap()[0].text, "for a");
    assert_eq!(store.notes("b").unwrap()[0].text, "for b");
    assert!(store.notes("c").unwrap().is_empty());
}

#[test]
fn the_block_a_step_reads_puts_the_pinned_note_first_then_the_newest_and_says_what_the_source_is() {
    let notes = vec![
        Note { seq: 1, kind: "fact".into(), text: "make the header stop flickering".into(), source: String::new(), quote: String::new(), pinned: true },
        Note { seq: 2, kind: "decision".into(), text: "the header is drawn in `ui/header.rs`".into(), source: "ui/header.rs".into(), quote: "fn draw(".into(), pinned: false },
        Note { seq: 3, kind: "failure".into(), text: "`cargo test` fails on `draws_the_header`".into(), source: String::new(), quote: String::new(), pinned: false },
    ];
    let block = render(&notes, 4_000);

    assert!(block.starts_with("## This task, so far"), "{block}");
    assert!(block.find("make the header stop flickering").unwrap() < block.find("`cargo test` fails").unwrap(), "the pinned fact is first");
    assert!(block.find("fails on").unwrap() < block.find("is drawn in").unwrap(), "then the newest first");
    assert!(block.contains("(ui/header.rs)"), "{block}");

    /* no notes, no header */
    assert_eq!(render(&[], 4_000), "");

    /* a cap smaller than the list keeps the head and says how many went */
    let tight = render(&notes, 120);

    assert!(tight.len() <= 120, "{}: {tight}", tight.len());
    assert!(tight.contains("more not shown"), "{tight}");
}

#[test]
fn an_explicit_remembering_in_the_persons_words_becomes_a_fact_and_nothing_else_does() {
    assert_eq!(requested_fact("Please remember: the API key is in the settings screen."), Some("the API key is in the settings screen".to_string()));
    assert_eq!(requested_fact("remember that the port is 8080"), Some("the port is 8080".to_string()));
    assert_eq!(requested_fact("মনে রাখো: টেস্ট সবুজ থাকতে হবে"), Some("টেস্ট সবুজ থাকতে হবে".to_string()));
    assert_eq!(requested_fact("fix the header, it flickers"), None, "importance is not guessed at");
    assert_eq!(requested_fact("remember: "), None);
}

#[test]
fn a_note_from_the_agents_tool_is_validated_before_it_is_kept() {
    let good = from_tool(&serde_json::json!({ "kind": "decision", "text": "the cache is in `store/cache.rs`", "source": "store/cache.rs", "quote": "pub struct Cache" })).unwrap();

    assert_eq!((good.kind.as_str(), good.source.as_str()), ("decision", "store/cache.rs"));
    assert_eq!(from_tool(&serde_json::json!({ "text": "" })).unwrap_err(), "`text` is required: the one line worth keeping");
    assert!(from_tool(&serde_json::json!({ "kind": "poem", "text": "x" })).unwrap_err().contains("not a kind of note"));
    assert!(from_tool(&serde_json::json!({ "text": "x".repeat(401) })).unwrap_err().contains("one line"));
    assert!(from_tool(&serde_json::json!({ "text": "x", "quote": "y".repeat(301) })).unwrap_err().contains("over 300"));
    assert!(known_kind("open") && !known_kind("guess"));
    assert_eq!(as_json(&note(4, "fact", "x", true))["pinned"], true);
}
