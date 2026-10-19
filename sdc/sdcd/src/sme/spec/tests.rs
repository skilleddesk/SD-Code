use super::*;
use crate::understand::Reading;

fn brief_of(request: &str) -> Option<Spec> {
    let reading = Reading::of(request);

    of(request, &reading, &HashMap::new())
}

#[test]
fn a_bengali_request_becomes_a_short_english_spec_with_the_persons_paths_untouched() {
    let request = "billing/pay.py ফাইলে charge ফাংশনটা ঠিক করো, যেন currency ধরে নেয়. টেস্ট ভাঙা যাবে না. cargo test pass hote hobe.";
    let spec = brief_of(request).expect("a Bengali request gets a spec");

    assert!(!spec.goal.is_empty());
    assert!(spec.files.contains(&"billing/pay.py".to_string()), "{:?}", spec.files);
    assert!(spec.constraints.iter().any(|line| line.contains("টেস্ট") || line.to_ascii_lowercase().contains("test")), "{:?}", spec.constraints);
    assert!(spec.done_when.iter().any(|line| line.to_ascii_lowercase().contains("cargo test")), "{:?}", spec.done_when);

    let brief = spec.brief();

    assert!(brief.starts_with("## The request, as a technical spec"), "{brief}");
    assert!(!brief.contains("bilbing"));
    assert!(brief.contains("billing/pay.py"), "the path is kept exactly: {brief}");
    assert!(brief.lines().count() <= 8, "a spec is short: {brief}");
}

#[test]
fn a_banglish_request_works_the_same_way_and_its_commands_are_kept() {
    let request = "src/ui/header.rs e flicker ta thik koro, cargo build cholbe na jodi error thake. test pass korte hobe.";
    let spec = brief_of(request).expect("a Banglish request gets a spec");

    assert!(spec.files.contains(&"src/ui/header.rs".to_string()), "{:?}", spec.files);
    assert!(spec.brief().contains("src/ui/header.rs"));
    assert!(spec.language.contains("বাংলা"));

    /* the confirmation is in the person's language, with the English beside it */
    let confirm = spec.confirm();

    assert!(confirm.contains("এই বোঝাটা ঠিক?") && confirm.contains("Is this right?"), "{confirm}");
}

#[test]
fn an_english_request_gets_no_spec_because_the_agent_already_works_in_english() {
    assert_eq!(brief_of("fix the flicker in src/ui/header.rs and keep the tests green"), None);
    assert_eq!(brief_of(""), None);
    assert_eq!(of("পাঠাও", &Reading::of("পাঠাও"), &HashMap::new()).map(|spec| spec.goal.is_empty()), Some(false), "a short Bengali line still gets a goal");
}

#[test]
fn the_technical_tokens_are_paths_files_backticked_names_and_commands_and_nothing_else() {
    let found = technical("billing/pay.py আর src/ui/header.rs, তারপর `charge` আর cargo build আর 12/2");

    assert!(found.contains(&"billing/pay.py".to_string()), "{found:?}");
    assert!(found.contains(&"src/ui/header.rs".to_string()));
    assert!(found.iter().any(|token| token.starts_with("cargo")), "{found:?}");
    assert!(!found.iter().any(|token| token == "আর"), "ordinary words are not technical");
}

#[test]
fn the_same_request_always_gives_the_same_spec_and_a_short_one_is_trimmed_to_its_sentences() {
    let request = "টেস্ট ভাঙা যাবে না. charge ঠিক করো.";

    let first = brief_of(request).unwrap();
    let second = brief_of(request).unwrap();

    assert_eq!(first.brief(), second.brief(), "the brief is deterministic: it goes into the stable prefix");
    assert!(!first.constraints.is_empty(), "{:?}", first.constraints);
}
