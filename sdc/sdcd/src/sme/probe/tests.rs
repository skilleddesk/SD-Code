use super::*;

/// A model with known limits, so the probe's arithmetic is checked against a truth rather than a hope.
struct Scripted {
    key: ModelKey,
    /// Beyond this many prompt tokens the front of the prompt is lost (what a too-small window does).
    window: u64,
    tools: Tools,
    json_ok: bool,
    /// The most numbered lines it ever finishes.
    line_cap: u64,
    /// How many characters one token covers: 4.5 is a coarse tokenizer; Qwen3.5 splits this filler at about 2.3.
    chars_per_token: f64,
    asked: Vec<Ask>,
}

#[derive(Clone, Copy, PartialEq)]
enum Tools {
    Structured,
    PrintedAsText,
    Silent,
}

fn key(digest: &str) -> ModelKey {
    ModelKey { provider: "ollama".into(), model: "test:9b".into(), digest: digest.into(), quantization: "Q4_K_M".into(), runtime: "0.12.3".into() }
}

fn scripted(window: u64) -> Scripted {
    Scripted { key: key("sha256:aaaa"), window, tools: Tools::Structured, json_ok: true, line_cap: 4_096, chars_per_token: 4.5, asked: Vec::new() }
}

impl Runner for Scripted {
    fn identity(&self) -> Result<ModelKey, String> {
        Ok(self.key.clone())
    }

    fn ask(&mut self, ask: &Ask) -> Result<Reply, String> {
        self.asked.push(ask.clone());

        let tokens = (ask.prompt.len() as f64 / self.chars_per_token) as u64;
        /* A runtime keeps no more than the window it was asked for, whatever the model could hold: Ollama cuts the front of a prompt that overflows num_ctx. */
        let window = self.window.min(ask.num_ctx);
        let mut reply = Reply { prompt_tokens: tokens, first_token_ms: 100, total_ms: 1_100, output_tokens: 50, had_logprobs: true, ..Reply::default() };

        if let Some((name, _)) = &ask.tool {
            let numbers: Vec<i64> = ask.prompt.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect();

            match self.tools {
                Tools::Structured => reply.tool_calls.push((name.clone(), json!({ "a": numbers[0], "b": numbers[1] }))),
                Tools::PrintedAsText => reply.text = format!("<tool_call>{{\"name\": \"{name}\", \"arguments\": {{\"a\": {}, \"b\": {}}}}}</tool_call>", numbers[0], numbers[1]),
                Tools::Silent => reply.text = "I would add them.".into(),
            }

            return Ok(reply);
        }

        if ask.schema.is_some() {
            reply.text = if self.json_ok { r#"Sure: {"name":"Ada","age":36,"tags":["a","b"]}"#.into() } else { "name: Ada, age: 36".into() };

            return Ok(reply);
        }

        if let Some(start) = ask.prompt.find("The access code is ") {
            /* Position of the needle as a share of the prompt; the front is lost past the window. */
            let share = start as f64 / ask.prompt.len() as f64;
            let kept_from = if tokens > window { 1.0 - window as f64 / tokens as f64 } else { 0.0 };
            let code: String = ask.prompt[start + 19..].split('.').next().unwrap_or("").to_string();

            reply.text = if share >= kept_from { code } else { "I do not see a code.".into() };

            return Ok(reply);
        }

        if let Some(lines) = ask.prompt.strip_prefix("Write the numbers from 1 to ").and_then(|rest| rest.split(',').next()).and_then(|n| n.trim().parse::<u64>().ok()) {
            reply.text = (1..=lines.min(self.line_cap)).map(|n| format!("{n}. word")).collect::<Vec<_>>().join("\n");

            return Ok(reply);
        }

        reply.text = "one, two, three".into();

        Ok(reply)
    }

    fn memory(&mut self) -> Option<(u64, f64)> {
        Some((6_600_000_000, 1.0))
    }
}

fn small_plan() -> Plan {
    Plan { seed: 7, contexts: vec![1_024, 2_048, 4_096, 8_192], tool_trials: 6, schema_trials: 4, outputs: vec![50, 100, 200, 400] }
}

#[test]
fn it_finds_the_window_a_model_really_has() {
    let mut model = scripted(2_600);
    let profile = run(&mut model, &small_plan(), "2026-10-17").unwrap();

    assert_eq!(profile.effective_context, 2_048, "holds 2k, loses the front at 4k");
    assert_eq!(profile.status, Status::Probed);
    assert_eq!(profile.class, Class::S);
}

#[test]
fn a_model_that_holds_everything_is_measured_at_the_top_candidate() {
    let mut model = scripted(1_000_000);
    let profile = run(&mut model, &small_plan(), "t").unwrap();

    assert_eq!(profile.effective_context, 8_192);
}

#[test]
fn a_finer_tokenizer_does_not_push_the_prompt_past_num_ctx_and_measure_every_model_as_zero() {
    let mut model = scripted(1_000_000);

    model.chars_per_token = 2.3;

    let profile = run(&mut model, &small_plan(), "t").unwrap();

    assert_eq!(profile.effective_context, 8_192, "a model that holds everything is measured at the top size, not at 0");
}

#[test]
fn every_depth_is_tested_and_a_needle_missed_at_one_depth_fails_the_size() {
    let mut model = scripted(1_000_000);

    run(&mut model, &small_plan(), "t").unwrap();

    let needle_prompts: Vec<&Ask> = model.asked.iter().filter(|ask| ask.prompt.contains("The access code is")).collect();

    assert_eq!(needle_prompts.len(), 4 * 4, "four sizes x depths 25/50/75/100%");

    let at = |ask: &&Ask| ask.prompt.find("The access code is").unwrap() as f64 / ask.prompt.len() as f64;
    let first_size: Vec<f64> = needle_prompts.iter().take(4).map(at).collect();

    assert!(first_size.windows(2).all(|pair| pair[0] < pair[1]), "needles move deeper: {first_size:?}");
}

#[test]
fn tool_calls_are_counted_as_structured_printed_or_missing() {
    for (tools, structured, printed) in [(Tools::Structured, 1.0, 0), (Tools::PrintedAsText, 0.0, 6), (Tools::Silent, 0.0, 0)] {
        let mut model = scripted(1_000_000);

        model.tools = tools;

        let profile = run(&mut model, &small_plan(), "t").unwrap();

        assert_eq!(profile.tool_call_reliability, structured);
        assert_eq!(profile.tool_calls_printed_as_text, printed);
    }
}

#[test]
fn json_adherence_and_output_length_are_measured() {
    let mut model = scripted(1_000_000);

    model.json_ok = false;
    model.line_cap = 150;

    let profile = run(&mut model, &small_plan(), "t").unwrap();

    assert_eq!(profile.json_adherence, 0.0);
    assert_eq!(profile.max_reliable_output, 100, "finishes 100 lines, stops at 150 so 200 is cut");
}

#[test]
fn speed_memory_logprobs_come_from_the_runtime() {
    let mut model = scripted(1_000_000);
    let profile = run(&mut model, &small_plan(), "t").unwrap();

    assert_eq!(profile.first_token_ms, 100);
    assert!((profile.tokens_per_second - 50.0).abs() < 0.001, "50 tokens in 1.0 s of generating");
    assert_eq!(profile.vram_bytes, Some(6_600_000_000));
    assert_eq!(profile.gpu_share, Some(1.0));
    assert!(profile.logprobs);
}

#[test]
fn the_probe_is_deterministic_for_a_seed() {
    let (mut a, mut b) = (scripted(5_000), scripted(5_000));
    let first = run(&mut a, &small_plan(), "t").unwrap();
    let second = run(&mut b, &small_plan(), "t").unwrap();

    assert_eq!(first, second);
    /* The first question calibrates the tokenizer and carries a per-run nonce so a cached prompt cannot undercount; the rest are fixed. */
    assert_eq!(a.asked.iter().skip(1).map(|ask| ask.prompt.clone()).collect::<Vec<_>>(), b.asked.iter().skip(1).map(|ask| ask.prompt.clone()).collect::<Vec<_>>(), "same questions, byte for byte");

    let other = Plan { seed: 8, ..small_plan() };
    let mut c = scripted(5_000);

    run(&mut c, &other, "t").unwrap();

    assert_ne!(a.asked[1].prompt, c.asked[1].prompt, "another seed asks different questions");
}

#[test]
fn the_probe_sends_nothing_of_the_persons() {
    std::env::set_var("SDC_PROBE_CANARY", "super-secret-canary-value");

    let mut model = scripted(1_000_000);

    run(&mut model, &small_plan(), "t").unwrap();

    for ask in &model.asked {
        assert!(!ask.prompt.contains("super-secret-canary-value"), "the environment never reaches a prompt");
        assert!(!ask.prompt.to_lowercase().contains("/users/") && !ask.prompt.contains("sk-"), "no path or key shapes");
    }

    std::env::remove_var("SDC_PROBE_CANARY");
}

#[test]
fn changing_any_part_of_the_identity_makes_the_stored_profile_provisional() {
    let _guard = DATA_DIR_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());

    let mut model = scripted(1_000_000);
    let plan = Plan { contexts: vec![2_048, 16_384], ..small_plan() };
    let profile = run(&mut model, &plan, "2026-10-17").unwrap();

    save(&profile).unwrap();

    assert_eq!(lookup(&profile.key).unwrap().status, Status::Probed);
    assert_eq!(label(&lookup(&profile.key).unwrap()), "M");

    for changed in [
        ModelKey { digest: "sha256:bbbb".into(), ..profile.key.clone() },
        ModelKey { quantization: "Q8_0".into(), ..profile.key.clone() },
        ModelKey { runtime: "0.13.0".into(), ..profile.key.clone() },
    ] {
        let found = lookup(&changed).expect("the earlier measurement is still offered");

        assert_eq!(found.status, Status::Provisional, "{:?}", changed.id());
        assert!(label(&found).ends_with('?'));
    }

    assert!(lookup(&ModelKey { model: "another:7b".into(), ..profile.key.clone() }).is_none(), "another model has no profile");

    std::env::remove_var("SDC_DATA_DIR");
}

#[test]
fn classes_follow_the_measurements() {
    assert_eq!(classify(8_000, 1.0, 32_768), Class::S);
    assert_eq!(classify(32_768, 0.5, 32_768), Class::S, "a model that cannot call tools is small whatever its window");
    assert_eq!(classify(16_384, 0.9, 32_768), Class::M);
    assert_eq!(classify(65_536, 1.0, 65_536), Class::L);
}

#[test]
fn printed_tool_calls_and_schema_answers_are_recognised() {
    assert!(printed_tool_call("<tool_call>{\"name\":\"add\"}</tool_call>"));
    assert!(printed_tool_call("{\"name\": \"add\", \"arguments\": {\"a\":1}}"));
    assert!(!printed_tool_call("the sum is 5"));
    assert!(follows_person_schema(r#"ok {"name":"A","age":3,"tags":[]}"#));
    assert!(!follows_person_schema(r#"{"name":"A","age":"3","tags":[]}"#));
    assert!(!follows_person_schema("no json"));
}

#[test]
fn a_model_is_due_for_measuring_until_its_current_identity_is_probed() {
    let mut model = scripted(1_000_000);
    let probed = run(&mut model, &small_plan(), "2026-10-17").unwrap();
    let provisional = Profile { status: Status::Provisional, ..probed.clone() };

    assert!(due(None), "never measured");
    assert!(due(Some(&provisional)), "measured on an earlier file or runtime");
    assert!(!due(Some(&probed)), "measured on this one");
}

#[test]
fn the_background_probe_is_the_quick_plan_and_is_on_by_default() {
    let quick = quick_plan();

    assert_eq!(quick.contexts, vec![2_048, 4_096, 8_192]);
    assert!(quick.tool_trials < Plan::default().tool_trials && quick.schema_trials < Plan::default().schema_trials);
    assert_eq!(quick.seed, Plan::default().seed);
    assert!(crate::v3::FEATURES.iter().any(|feature| feature.name == "models.probe" && feature.default_on));
}

/// The limiter behind `after_use`: the set is process-wide and tests run in parallel, so each name here is its own.
#[test]
fn a_model_is_asked_about_once_per_run_and_each_model_has_its_own_turn() {
    assert!(first_ask("limiter-test-a:1b"), "the first turn on a model asks");
    assert!(!first_ask("limiter-test-a:1b"), "the second does not, even if the first probe failed");
    assert!(first_ask("limiter-test-b:1b"), "another model is not held back by the first");
    assert!(!first_ask("limiter-test-b:1b"));
}
