use super::*;

#[test]
fn a_window_decides_the_class_at_the_boundaries() {
    assert_eq!(Class::of(4_096), Class::Small);
    assert_eq!(Class::of(8_192), Class::Small);
    assert_eq!(Class::of(16_384), Class::Medium);
    assert_eq!(Class::of(32_768), Class::Medium);
    assert_eq!(Class::of(65_536), Class::Large);
    assert_eq!(Class::of(131_072), Class::Large);
    assert_eq!([Class::Small.name(), Class::Medium.name(), Class::Large.name()], ["S", "M", "L"]);
}

#[test]
fn every_class_fits_its_own_window_with_the_answer_and_the_thinking_held_back() {
    for window in [4_096, 8_192, 16_384, 32_768, 65_536, 131_072] {
        let budget = Budget::for_window(window);

        assert!(budget.input() + budget.output + budget.thinking <= window as usize, "{} does not fit {window}: {budget:?}", budget.sentence());
        assert!(budget.cap(Section::System) > 0 && budget.cap(Section::Card) > 0 && budget.cap(Section::Code) > 0);
    }

    /* the classes really differ: S is not given M's room */
    assert!(Budget::for_window(8_192).cap(Section::Code) < Budget::for_window(32_768).cap(Section::Code));
    assert!(Budget::for_window(32_768).cap(Section::Code) < Budget::for_window(131_072).cap(Section::Code));
}

#[test]
fn a_step_that_fits_and_one_that_does_not_are_told_apart_and_the_right_section_goes_first() {
    let budget = Budget::for_window(8_192);
    let window = 8_192usize;
    let system = (Section::System, budget.cap(Section::System));

    /* every section at its own cap is exactly what the budget is for */
    assert!(budget.fits(&[(Section::Card, budget.cap(Section::Card)), (Section::Code, budget.cap(Section::Code))]));
    assert_eq!(budget.drop_first(&[system, (Section::Code, budget.cap(Section::Code))]), None);

    /* one section far over: it goes, and the order is the priority order - the tool result first, then the schemas, then memory,
       then the card, then the code, and never the rules */
    assert_eq!(budget.drop_first(&[system, (Section::Code, window)]), Some(Section::Code));
    assert_eq!(budget.drop_first(&[system, (Section::ToolOutput, 200), (Section::Code, window)]), Some(Section::ToolOutput), "the tool result goes first, even when the code is the one that is too long");
    assert_eq!(budget.drop_first(&[system, (Section::Memory, 200), (Section::Tools, window)]), Some(Section::Tools), "the schemas go before the memory");
    assert_eq!(budget.drop_first(&[system, (Section::Memory, 200), (Section::Card, window)]), Some(Section::Memory), "memory goes before the card");
    assert_eq!(budget.drop_first(&[system, (Section::Card, window)]), Some(Section::Card));

    /* only the rules are left and they are over: no section is cut, the answer is a smaller step */
    assert_eq!(budget.drop_first(&[(Section::System, window)]), None);
    assert!(budget.fits(&[]), "an empty step always fits");
}

#[test]
fn trimming_cuts_at_a_line_bounds_says_so_and_never_splits_a_character() {
    let budget = Budget::for_window(8_192);
    let text = (1..=600).map(|line| format!("line {line} of a long file\n")).collect::<String>();
    let (trimmed, cut) = budget.trim(&text, Section::Code);

    assert!(cut);
    assert!(trimmed.len() <= budget.chars(Section::Code), "{} > {}", trimmed.len(), budget.chars(Section::Code));
    assert!(trimmed.ends_with("(trimmed to fit the model's window)"), "{}", &trimmed[trimmed.len().saturating_sub(60)..]);
    assert!(trimmed.lines().count() > 10);
    assert!(trimmed.starts_with("line 1 of a long file"));

    /* a line that ends exactly on the cap: the trim lands on a line boundary, not mid-word */
    assert!(trimmed.trim_end_matches("\n(trimmed to fit the model's window)").ends_with("long file"));

    /* a short text is untouched */
    assert_eq!(budget.trim("short\n", Section::Code), ("short\n".to_string(), false));

    /* a multi-byte text never panics and stays valid */
    let unicode = "বাংলা লেখা একটি লাইনে\n".repeat(400);
    let (trimmed, cut) = budget.trim(&unicode, Section::Memory);

    assert!(cut && trimmed.chars().count() > 1);
}

#[test]
fn the_tool_schemas_are_kept_while_their_cost_fits_under_the_cap() {
    let budget = Budget::for_window(8_192);
    let costs: Vec<usize> = vec![120, 130, 140, 900, 90];

    /* the first three fit (390 of 800), the fourth would overflow - and the fifth does not jump the queue */
    assert_eq!(budget.take_within(Section::Tools, costs.iter()), 3);

    /* a first schema larger than the whole cap is still kept: a step with no tools is not a step */
    assert_eq!(budget.take_within(Section::Tools, [5_000].iter()), 1);
    assert_eq!(budget.take_within(Section::Tools, [].iter()), 0);
}

#[test]
fn the_chars_of_a_section_are_its_tokens_times_four() {
    let budget = Budget::for_window(32_768);

    assert_eq!(budget.chars(Section::ToolOutput), budget.cap(Section::ToolOutput) * 4);
    assert!(budget.sentence().contains("class M of 32768 tokens"), "{}", budget.sentence());
}
