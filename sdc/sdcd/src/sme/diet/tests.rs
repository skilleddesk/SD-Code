use super::*;
use serde_json::json;

#[test]
fn a_step_is_offered_at_most_six_tools_and_never_none() {
    let specs: Vec<String> = (1..=20).map(|n| format!("tool{n}")).collect();
    let kept = at_most(&specs, MAX_SCHEMAS);

    assert_eq!(kept, specs[..6].to_vec(), "the best six, in their ranked order");
    assert_eq!(at_most(&specs, 0), vec!["tool1".to_string()], "zero is read as one: a step with no tools is not a step");
    assert_eq!(at_most(&specs[..2], MAX_SCHEMAS).len(), 2);
    assert!(at_most::<String>(&[], MAX_SCHEMAS).is_empty());
}

#[test]
fn the_system_prompt_is_cut_at_a_line_boundary_to_the_class_and_marked_and_is_the_same_bytes_every_time() {
    let rules = (1..=400).map(|line| format!("- rule {line}: read the file before you change it\n")).collect::<String>();

    let small = slim(&rules, Class::Small);

    assert!(small.len() <= system_cap(Class::Small) + 130, "{} chars", small.len());
    assert!(small.ends_with("verify before you answer]"));
    assert!(small.starts_with("- rule 1: read the file"));
    assert!(small.lines().count() > 20);
    assert_eq!(small, slim(&rules, Class::Small), "the same input is the same bytes");
    assert_eq!(slim(&rules, Class::Small), slim(&rules, Class::Small), "and it does not drift between calls");

    /* a big class keeps more of the same prompt; a short prompt is untouched */
    assert!(slim(&rules, Class::Large).len() > small.len());
    assert_eq!(slim("short rules\n", Class::Small), "short rules\n");
    assert_eq!(slim("", Class::Small), "");

    /* a multi-byte prompt never panics and stays valid UTF-8 */
    let bengali = "- নিয়ম: ফাইল পড়ে তারপর লেখ\n".repeat(300);

    assert!(slim(&bengali, Class::Small).len() <= system_cap(Class::Small) + 130);
}

#[test]
fn the_goal_sits_above_and_below_the_evidence_and_sending_it_twice_changes_nothing() {
    let text = "The file is:\nfn a() {}".to_string();
    let goal = "make a() return 2";
    let once = bookend(&text, goal);

    assert!(once.starts_with("Goal: make a() return 2\n\n"), "{once}");
    assert!(once.ends_with("\n\nThe goal, again: make a() return 2"), "{once}");
    assert_eq!(bookend(&once, goal), once, "idempotent");
    assert_eq!(bookend(&text, "   "), text, "no goal, no bookend");
    assert_eq!(bookend("", "x"), "Goal: x\n\n\n\nThe goal, again: x");
}

#[test]
fn the_prefix_is_the_same_bytes_for_every_step_of_a_task_and_empty_parts_leave_no_holes() {
    let first = stable(&[("The rules", Some("be small")), ("The project", Some("a/ b/")), ("This task", None)]);
    let second = stable(&[("The rules", Some("be small")), ("The project", Some("a/ b/")), ("This task", None)]);

    assert_eq!(first, second);
    assert_eq!(prefix_key(&[("The rules", "be small"), ("The project", "a/ b/")]), prefix_key(&[("The rules", "be small"), ("The project", "a/ b/")]));
    assert!(!first.contains("\n\n\n"), "an empty part is left out, not left as a blank line");
    assert_eq!(first, "## The rules\nbe small\n\n## The project\na/ b/");

    /* the project map changing is a new prefix - and only then */
    assert_ne!(prefix_key(&[("The rules", "be small"), ("The project", "a/ c/")]), prefix_key(&[("The rules", "be small"), ("The project", "a/ b/")]));
}

#[test]
fn the_few_tokens_a_step_may_spend_on_its_notes_are_enforced_not_hoped_for() {
    let budget = Budget::for_window(4_096);
    let notes = "note about the file\n".repeat(400);
    let fitted = fit_notes(&notes, &budget);

    assert!(fitted.len() <= budget.chars(Section::Card));
    assert!(fitted.contains("(trimmed to fit the model's window)"));
    assert_eq!(fit_notes("short\n", &budget), "short\n");
}

#[test]
fn the_goal_can_be_repeated_under_the_last_words_of_a_conversation() {
    let mut messages = vec![json!({ "role": "user", "content": "here is the file" })];
    let added = append_to_last(&mut messages, "The goal, again: make it return 2");

    assert!(added);
    assert_eq!(messages[0]["content"], "here is the file\n\nThe goal, again: make it return 2");
    assert!(!append_to_last(&mut [], "x"));
    assert!(!append_to_last(&mut [json!({ "role": "assistant", "tool_calls": [] })], "x"), "a tool-call message has no plain text to extend");
}

#[test]
fn the_cap_keeps_the_tools_that_matter_not_the_first_registered() {
    let registered = ["read_file", "list_dir", "search", "git_diff", "write_file", "edit_file", "run_command", "update_plan", "grep", "glob", "web_fetch", "remember", "ranked_search", "mcp__x__tool"];
    let kept = at_most(&prioritise(&registered, |name| name), MAX_SCHEMAS);

    assert_eq!(kept, ["read_file", "grep", "edit_file", "write_file", "run_command", "ranked_search"]);
    assert_eq!(prioritise(&["zzz", "read_file"], |name| name), ["read_file", "zzz"], "a tool nobody ranked comes after the ranked ones");
    assert_eq!(prioritise(&["b", "a"], |name| name), ["b", "a"], "unranked tools keep the order they came in");
}
