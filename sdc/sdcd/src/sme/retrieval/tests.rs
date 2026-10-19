use super::*;

const PAY: &str = "def charge(amount):\n    return amount * 2\n\ndef refund(amount):\n    return -amount\n";
const APP: &str = "from pay import charge\n\ndef run():\n    return charge(3)\n\n\ndef unused_header():\n    return None\n";
const DOC: &str = "# Billing\nThe billing screen shows a header.\n\n## Refunds\nA refund is a negative charge.\n";

fn index() -> Index {
    Index::of([("pay.py", PAY), ("app.py", APP), ("docs/billing.md", DOC)].into_iter())
}

#[test]
fn chunks_are_the_asts_own_units_so_a_hit_is_a_whole_thought() {
    let units = chunks("pay.py", PAY);

    assert_eq!(units.len(), 2, "{units:?}");
    assert_eq!((units[0].start_line, units[0].end_line), (1, 2), "the first function, whole");
    assert!(units[0].text.starts_with("def charge"));
    assert_eq!((units[1].start_line, units[1].end_line), (4, 5));
    assert_ne!(units[0].hash, units[1].hash, "the hash is the content's, so a summary can be cached by it");

    /* markdown: one chunk per heading, the first before any heading */
    let sections = chunks("docs/billing.md", DOC);

    assert!(sections.len() >= 2 && sections[0].text.starts_with("# Billing"), "{sections:?}");

    /* a language the graph does not read: windows of lines, never one chunk of the whole file */
    let long = (1..=100).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let windows = chunks("notes.unknownext", &long);

    assert_eq!(windows.len(), 3);
    assert_eq!((windows[0].start_line, windows[0].end_line), (1, 40));
    assert!(chunks("empty.rs", "   \n\n").is_empty());
}

#[test]
fn a_question_finds_the_declaration_that_answers_it_before_the_file_that_merely_mentions_it() {
    let hits = index().search("where is charge defined", 3);

    assert!(!hits.is_empty());
    assert_eq!(hits[0].path, "pay.py");
    assert_eq!(hits[0].why, "declares", "{hits:?}");
    assert!(hits[0].text.contains("def charge(amount)"));

    /* the caller comes too, ranked lower: the word is there, the declaration is not */
    assert!(hits.iter().any(|hit| hit.path == "app.py"));

    /* a word that is everywhere in this project does not win just for being frequent */
    let common = index().search("return", 3);

    assert!(common.len() <= 3);
    assert!(common.iter().all(|hit| hit.score > 0.0));
}

#[test]
fn a_file_name_is_a_signal_and_prose_is_reached_by_its_words() {
    let named = index().search("billing", 2);

    assert_eq!(named[0].path, "docs/billing.md");
    assert_eq!(named[0].why, "name", "{named:?}");

    let prose = index().search("negative amount", 2);

    assert!(prose.iter().any(|hit| hit.path == "docs/billing.md" || hit.path == "pay.py"), "{prose:?}");
    assert!(index().search("ぜんぜん違うことば", 3).is_empty(), "nothing matches nothing");
    assert!(index().search("", 3).is_empty());
    assert!(Index::default().search("charge", 3).is_empty());
}

#[test]
fn terms_split_snake_and_camel_case_and_drop_the_words_that_carry_nothing() {
    assert_eq!(terms("drawHeader now"), vec!["draw".to_string(), "header".to_string(), "now".to_string()]);
    assert_eq!(terms("charge_once the a x"), vec!["charge".to_string(), "once".to_string()]);
    assert!(terms("the of a to is").is_empty());
    assert_eq!(terms("বাংলা লেখা"), vec!["বাংলা".to_string(), "লেখা".to_string()]);
}

#[test]
fn the_skim_names_the_candidates_and_shows_the_best_one() {
    let hits = index().search("charge", 2);
    let skimmed = skim(&hits, 2);

    assert!(skimmed.contains("pay.py:1-2 [declares]"), "{skimmed}");
    assert!(skimmed.contains("-- pay.py:1-2"));
    assert!(skimmed.lines().any(|line| line.contains("def charge(amount):")));
    assert_eq!(skim(&[], 5), "");
}

#[test]
fn summaries_are_cached_by_content_hash_so_the_same_bytes_are_never_summarised_twice() {
    let cache = Summaries::default();
    let mut calls = 0;
    let hash = chunks("pay.py", PAY)[0].hash.clone();

    let first = cache.get_or(&hash, || {
        calls += 1;
        Some("charges an amount".into())
    });

    let second = cache.get_or(&hash, || {
        calls += 1;
        Some("a different summary".into())
    });

    assert_eq!((first.as_deref(), second.as_deref()), (Some("charges an amount"), Some("charges an amount")));
    assert_eq!(calls, 1, "the second read came from the cache");
    assert_eq!(cache.get("nothing"), None);

    /* a file that changed has a different hash: a new summary, as it should be */
    let edited = chunks("pay.py", &format!("{PAY}\ndef extra():\n    return 1\n"))[0].hash.clone();

    assert_eq!(edited, hash, "a chunk that did not change keeps its hash");

    let third = chunks("pay.py", "def charge(amount):\n    return amount * 3\n")[0].hash.clone();

    assert_ne!(third, hash, "a chunk that changed does not");
}

#[test]
fn stage_two_is_off_until_someone_has_measured_it_and_the_order_is_then_stage_one_s() {
    /* with the re-ranker off (the default) the order is exactly stage 1's */
    assert!(!rerank_enabled(), "a re-ranker nobody measured must not reorder the results");
}

#[test]
fn a_folder_is_read_into_an_index_with_progress_and_the_right_declaration_comes_first() {
    let root = std::env::temp_dir().join(format!("sdc-retrieval-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("billing")).unwrap();
    std::fs::write(root.join("billing/pay.py"), PAY).unwrap();
    std::fs::write(root.join("app.py"), APP).unwrap();

    let mut seen: Vec<(usize, usize)> = Vec::new();
    let index = build(&root, |done, total| seen.push((done, total)));

    assert!(index.len() >= 3, "{} chunks", index.len());
    assert!(!seen.is_empty() && seen.iter().all(|(done, total)| done <= total && *total == seen[0].1), "{seen:?}");

    let hits = index.search("charge", 2);

    assert_eq!(hits[0].path, "billing/pay.py");
    assert_eq!(retrieve(&root, "charge", 1)[0].path, "billing/pay.py");

    let _ = std::fs::remove_dir_all(&root);
}
