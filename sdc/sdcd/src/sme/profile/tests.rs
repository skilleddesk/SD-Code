use super::*;

fn tools(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn a_step_is_read_from_the_tools_the_last_reply_called() {
    assert_eq!(Step::of_tools(&[], false), Step::Plan);
    assert_eq!(Step::of_tools(&tools(&["read_file", "grep"]), false), Step::Read);
    assert_eq!(Step::of_tools(&tools(&["apply_patch"]), false), Step::Patch);
    assert_eq!(Step::of_tools(&tools(&["edit_file"]), false), Step::Patch);
    assert_eq!(Step::of_tools(&tools(&["write_file"]), false), Step::Format, "a whole-file write is mechanical");
    assert_eq!(Step::of_tools(&tools(&["git_diff"]), false), Step::Review);
    assert_eq!(Step::of_tools(&tools(&["whatever"]), false), Step::Plan);
    assert_eq!(Step::of_tools(&tools(&["read_file"]), true), Step::Debug, "a step after a failure is a debug step");
    assert_eq!([Step::Plan.name(), Step::Format.name(), Step::Debug.name()], ["plan", "format", "debug"]);
}

#[test]
fn qwen_gets_its_documented_numbers_and_another_family_is_left_alone() {
    let patch = Profile::for_step(Step::Patch, "qwen", true);

    assert_eq!((patch.temperature, patch.top_p, patch.top_k), (Some(0.6), Some(0.95), Some(20)));
    assert_eq!(patch.thinking, Some(true));

    let format = Profile::for_step(Step::Format, "qwen", true);

    assert_eq!(format.thinking, Some(false), "a mechanical step does not pay for thinking");
    assert_eq!(format.temperature, Some(0.7));

    let review = Profile::for_step(Step::Review, "qwen", false);

    assert_eq!((review.temperature, review.top_p), (Some(0.7), Some(0.8)), "the page's general-chat setting");
    assert_eq!(review.thinking, Some(false), "a turn with thinking off stays off");

    for step in [Step::Plan, Step::Read, Step::Patch, Step::Format, Step::Debug, Step::Review] {
        assert_eq!(Profile::for_step(step, "llama", true), Profile::neutral(step), "a family that documents nothing is not guessed at");
    }

    assert!(Profile::for_step(Step::Plan, "qwen", true).thinking == Some(true));
}

#[test]
fn the_family_is_read_from_the_name_the_server_reports() {
    assert_eq!(family_of("qwen3:8b"), "qwen");
    assert_eq!(family_of("Qwen/Qwen3-8B"), "qwen");
    assert_eq!(family_of("qwen2.5-coder:7b"), "qwen");
    assert_eq!(family_of("llama3.2:3b"), "llama");
    assert_eq!(family_of("gemma3:4b"), "gemma");
    assert_eq!(family_of("deepseek-r1:7b"), "deepseek");
    assert_eq!(family_of("something-new"), "unknown");
}

#[test]
fn applying_a_profile_touches_only_a_local_body_and_only_the_knobs_it_names() {
    let mut local = serde_json::json!({ "model": "qwen3:8b", "stream": true, "messages": [], "options": { "num_ctx": 32768 } });
    let before = local.clone();

    apply(&mut local, &Profile::for_step(Step::Patch, "qwen", true));

    assert_eq!(local["options"]["num_ctx"], 32768, "the context size is not the step's to change");
    assert_eq!(local["options"]["temperature"], 0.6);
    assert_eq!(local["options"]["top_k"], 20);
    assert_eq!(local["think"], true);

    apply(&mut local, &Profile::for_step(Step::Format, "qwen", true));
    assert_eq!(local["think"], false);

    /* a neutral profile changes nothing */
    let mut other = before.clone();

    apply(&mut other, &Profile::neutral(Step::Patch));
    assert_eq!(other, before);

    /* a cloud body has no `options`: it is left exactly as it was */
    let mut cloud = serde_json::json!({ "model": "gpt-5", "messages": [] });
    let original = cloud.clone();

    apply(&mut cloud, &Profile::for_step(Step::Patch, "qwen", true));
    assert_eq!(cloud, original);
}

#[test]
fn the_request_reports_the_step_that_was_set_and_forgets_it_afterwards() {
    set_current_step(Some(Step::Patch));

    let profile = for_request("qwen3:8b", true).unwrap();

    assert_eq!((profile.step, profile.temperature), (Step::Patch, Some(0.6)));
    assert_eq!(for_request("llama3:8b", true), None, "no documented numbers, no profile");

    set_current_step(None);

    assert_eq!(for_request("qwen3:8b", true), None, "no step set: nothing to apply");
}
