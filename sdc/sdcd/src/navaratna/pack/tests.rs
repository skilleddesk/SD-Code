use super::*;
use crate::navaratna::spec::{parse, Unit};

fn big_file(functions: usize) -> String {
    (0..functions).map(|n| format!("pub fn helper_{n}(value: u64) -> u64 {{\n    let step = value + {n};\n    let again = step * 2;\n    let more = again + step;\n    more - {n}\n}}\n\n")).collect()
}

fn units() -> Vec<Unit> {
    parse("Add a function double_total(items) that returns twice the total.\n\nIf the list is empty, return 0. Otherwise, return 2 * the sum.")
}

fn input<'a>(units: &'a [Unit], files: Vec<FileCtx>) -> StepInput<'a> {
    StepInput {
        rules: "Rules: edit with replace_symbol. Do not guess.",
        goal: "Add double_total",
        requirement_ids: vec!["R-001".into(), "R-002".into()],
        done_when: "tests/total.rs passes",
        units: units.iter().collect(),
        files,
        contracts: vec!["pub fn total(items: &[Item]) -> u64".into()],
        glossary: vec![("cart".into(), "the list of items".into())],
        handover: vec!["step 1 added Item".into(), "step 2 added total()".into()],
        example: None,
        budget_tokens: 4_000,
    }
}

fn shop(functions: usize) -> FileCtx {
    let mut text = big_file(functions);

    text.push_str("pub fn total(items: &[u64]) -> u64 {\n    items.iter().sum()\n}\n");

    FileCtx { path: "src/shop.rs".into(), text, targets: vec!["total".into()] }
}

#[test]
fn the_pack_has_the_goal_on_top_the_units_word_for_word_and_the_goal_again_on_the_last_line() {
    let units = units();
    let pack = build(&input(&units, vec![shop(40)]), &|_| 1.0).unwrap();

    assert!(pack.text.starts_with("Rules: edit with replace_symbol."));
    assert!(pack.text.find("## Goal").unwrap() < pack.text.find("## Spec").unwrap());
    assert!(pack.text.find("## Spec").unwrap() < pack.text.find("## Code").unwrap());
    assert!(pack.text.contains(&format!("[{}] {}", units[0].id, units[0].text)), "units are word for word");
    assert!(pack.text.contains("Requirements: R-001, R-002") && pack.text.contains("Done when: tests/total.rs passes"));

    let last = pack.text.lines().last().unwrap();

    assert_eq!(last, "Remember - goal: Add double_total | done when: tests/total.rs passes");
}

#[test]
fn the_skeleton_pack_drops_tokens_and_always_keeps_the_target_body() {
    let units = units();
    let whole = crate::v3::ablating(&["nr.skeleton_pack"], || build(&input(&units, vec![shop(60)]), &|_| 1.0)).unwrap();
    let skeleton = crate::v3::forcing(&["nr.skeleton_pack"], || build(&input(&units, vec![shop(60)]), &|_| 1.0)).unwrap();

    assert!(skeleton.tokens < whole.tokens / 2, "{} vs {}", skeleton.tokens, whole.tokens);
    assert!(skeleton.text.contains("items.iter().sum()"), "the target body is there");
    assert!(skeleton.text.contains("# body of total (lines"));
    assert!(skeleton.text.contains("pub fn helper_3(value: u64) -> u64"), "signatures of the others are there");
    assert!(!skeleton.text.contains("let again = step * 2"), "their bodies are not");
}

#[test]
fn the_target_body_is_present_in_every_fixture() {
    let units = units();

    crate::v3::forcing(&["nr.skeleton_pack"], || {
        for functions in [0, 1, 5, 40, 200] {
            let pack = build(&input(&units, vec![shop(functions)]), &|_| 1.0).unwrap();

            assert!(pack.text.contains("items.iter().sum()"), "{functions} functions");
        }
    });
}

#[test]
fn a_small_file_is_included_whole_and_a_look_alike_ranked_low_is_left_out_of_the_skeleton() {
    let units = units();
    let small = FileCtx { path: "src/tiny.rs".into(), text: "pub fn a() {}\n".into(), targets: vec![] };

    crate::v3::forcing(&["nr.skeleton_pack"], || {
        let pack = build(&input(&units, vec![small]), &|_| 1.0).unwrap();

        assert!(pack.text.contains("--- src/tiny.rs ---\npub fn a() {}"));

        let ranked = build(&input(&units, vec![shop(30)]), &|name: &str| if name == "total" || name == "helper_1" { 1.0 } else { 0.0 }).unwrap();

        assert!(ranked.text.contains("helper_1(") && !ranked.text.contains("helper_2("), "{}", ranked.text);
        assert!(ranked.text.contains("items.iter().sum()"), "a target is never ranked out");
    });
}

#[test]
fn the_size_of_a_step_does_not_depend_on_how_many_steps_came_before() {
    let units = units();
    let mut few = input(&units, vec![shop(10)]);
    let mut many = input(&units, vec![shop(10)]);

    few.handover = (0..2).map(|n| format!("note {n}")).collect();
    many.handover = (0..500).map(|n| format!("note {n}")).collect();

    let (a, b) = (build(&few, &|_| 1.0).unwrap(), build(&many, &|_| 1.0).unwrap());

    assert!(b.tokens.abs_diff(a.tokens) < 20, "{} vs {}", a.tokens, b.tokens);
    assert!(b.text.contains("note 499") && b.text.contains("note 497") && !b.text.contains("note 496"), "only the last three");
    assert!(!b.text.contains("note 0\n"));
}

#[test]
fn nothing_of_an_earlier_conversation_can_be_in_a_pack() {
    /* the input has no field for a chat turn: what is in the pack is only what is listed above */
    let units = units();
    let pack = build(&input(&units, vec![]), &|_| 1.0).unwrap();

    for foreign in ["assistant:", "user:", "tool_calls", "\"role\""] {
        assert!(!pack.text.contains(foreign), "{foreign}");
    }
}

#[test]
fn over_budget_the_example_goes_first_then_the_notes_then_the_bodies_and_never_a_unit() {
    let units = units();
    let mut over = input(&units, vec![shop(20)]);

    over.example = Some("x ".repeat(400));
    over.budget_tokens = 10_000;

    let roomy = crate::v3::forcing(&["nr.skeleton_pack"], || build(&over, &|_| 1.0)).unwrap();

    assert!(roomy.dropped.is_empty() && roomy.text.contains("A similar problem"));

    over.budget_tokens = roomy.tokens - 10;

    let pack = crate::v3::forcing(&["nr.skeleton_pack"], || build(&over, &|_| 1.0)).unwrap();

    assert_eq!(pack.dropped.first(), Some(&"example"));
    assert!(!pack.text.contains("A similar problem"));

    over.budget_tokens = 180;
    over.example = None;

    let split = crate::v3::forcing(&["nr.skeleton_pack"], || build(&over, &|_| 1.0));

    assert!(split.is_err(), "{split:?}");
    assert!(split.unwrap_err().to_string().starts_with("NeedsSplit"));
}

#[test]
fn a_language_without_a_grammar_is_shown_by_its_head_and_labelled() {
    let units = units();
    let zig = FileCtx { path: "src/main.zig".into(), text: "const x = 1;\n".repeat(200), targets: vec![] };

    crate::v3::forcing(&["nr.skeleton_pack"], || {
        let pack = build(&input(&units, vec![zig]), &|_| 1.0).unwrap();

        assert!(pack.text.contains("(no grammar: first 40 lines)"));
    });
}
