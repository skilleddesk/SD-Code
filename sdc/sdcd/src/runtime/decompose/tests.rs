use super::*;

fn known() -> Vec<(String, String)> {
    vec![
        ("charge".to_string(), "billing/pay.py".to_string()),
        ("refund".to_string(), "billing/pay.py".to_string()),
        ("draw_header".to_string(), "ui/header.rs".to_string()),
        ("save".to_string(), "store/save.rs".to_string()),
    ]
}

#[test]
fn a_small_request_is_one_step_and_the_breakdown_says_why() {
    let small = breakdown("fix the flicker in ui/header.rs", &known());

    assert!(small.one_step_only);
    assert_eq!(small.steps.len(), 1);
    assert_eq!(small.steps[0].files, vec!["ui/header.rs"]);
    assert!(small.reason.contains("one step"), "{}", small.reason);

    /* two files and two symbols still fit */
    let fitting = breakdown("change charge and refund in billing/pay.py", &known());

    assert!(fitting.one_step_only, "{}", fitting.reason);
}

#[test]
fn a_request_naming_more_work_is_cut_by_its_own_structure() {
    let request = "1. fix charge in billing/pay.py 2. then make refund raise the same error 3. finally save must keep the old file";
    let planned = breakdown(request, &known());

    assert!(!planned.one_step_only);
    assert_eq!(planned.steps.len(), 3, "{:?}", planned.steps);
    assert!(planned.steps[0].goal.contains("charge"), "{:?}", planned.steps[0]);
    assert!(planned.steps[1].goal.contains("refund"));
    assert!(planned.steps[2].goal.contains("save"));
    assert!(planned.reason.contains("3 steps"), "{}", planned.reason);

    /* the symbols each step names come from the graph, so the file of the symbol is known before the step starts */
    assert!(planned.steps[0].symbols.iter().any(|name| name == "charge"), "{:?}", planned.steps[0].symbols);
    assert!(planned.steps[2].symbols.iter().any(|name| name == "save"));
}

#[test]
fn joins_and_semicolons_are_boundaries_but_a_list_of_nouns_is_not() {
    let joined = breakdown("fix charge in billing/pay.py and then fix save in store/save.rs and then fix draw_header in ui/header.rs", &known());

    assert!(joined.steps.len() >= 3, "{:?}", joined.steps);

    /* "charge and refund" is one thought: it is not cut in two */
    let one_thought = breakdown("make charge and refund agree on the error text", &known());

    assert!(one_thought.one_step_only || one_thought.steps.len() == 1, "{:?}", one_thought.steps);

    let semicolons = breakdown("ui/header.rs e flicker thik koro; store/save.rs e path thik koro; billing/pay.py e error thik koro", &known());

    assert_eq!(semicolons.steps.len(), 3, "{:?}", semicolons.steps);
}

#[test]
fn a_step_naming_three_files_is_cut_again_by_file_so_no_step_is_too_big() {
    let request = "make a.rs, b.rs and c.rs use the same error type";
    let files = files_in(request);

    assert_eq!(files.len(), 3);

    /* the three files make the request planned work, and each piece is bounded by the file rule */
    let mut too_big = 0;

    for step in pieces(request) {
        if files_in(&step).len() > FILES_PER_STEP {
            too_big += 1;
        }
    }

    assert_eq!(too_big, 0, "no piece names more than {FILES_PER_STEP} files");
}

#[test]
fn the_next_steps_files_are_known_before_the_next_step_starts() {
    let planned = breakdown("fix charge in billing/pay.py and then fix save in store/save.rs and then fix draw_header in ui/header.rs", &known());

    assert!(next_files(&planned, 0).iter().any(|file| file.contains("store/save.rs")), "{:?}", next_files(&planned, 0));
    assert!(next_files(&planned, planned.steps.len() - 1).is_empty(), "there is no step after the last one");
    assert!(next_files(&planned, 99).is_empty());
}

#[test]
fn a_plan_whose_step_names_too_many_files_is_reported_with_the_step_and_the_files() {
    let steps = vec!["fix the flicker".to_string(), "update a.rs, b.rs and c.rs".to_string(), "ship it".to_string()];
    let large = too_large(&steps);

    assert_eq!(large.len(), 1);
    assert_eq!(large[0].0, 1);
    assert_eq!(large[0].1.len(), 3);
    assert!(too_large(&["fix ui/header.rs".to_string()]).is_empty());
}

#[test]
fn files_are_paths_and_names_with_an_extension_and_nothing_else() {
    assert_eq!(files_in("billing/pay.py আর store/save.rs, তারপর 12/2 আর 3.5"), vec!["billing/pay.py".to_string(), "store/save.rs".to_string(), "3.5".to_string()]);
    assert!(files_in("make it nicer").is_empty());
}

#[test]
fn a_request_that_is_several_jobs_becomes_a_numbered_note_and_a_small_one_gets_none() {
    let several = breakdown("fix charge in billing/pay.py and then fix save in store/save.rs and then fix draw_header in ui/header.rs", &known());
    let text = note(&several).expect("three files are three jobs");

    assert!(text.starts_with("This request is several jobs."), "{text}");
    assert!(text.contains("1. ") && text.contains("2. ") && text.contains("(files: "), "{text}");
    assert!(text.contains("billing/pay.py") && text.contains("ui/header.rs"), "{text}");

    assert_eq!(note(&breakdown("rename total to subtotal in billing/pay.py", &known())), None);
}
