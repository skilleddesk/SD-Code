use super::*;

fn sample(task: &str, model: &str, wall_s: f64, steps: u64) -> Sample {
    Sample { task: task.into(), model: model.into(), wall_s, steps, requests: steps, zero_model: false }
}

fn set(model: &str, walls: &[f64]) -> Vec<Sample> {
    walls.iter().enumerate().map(|(index, wall)| sample(&format!("t{index}"), model, *wall, 1)).collect()
}

fn the_store() -> Arc<Store> {
    Arc::new(Store::in_memory().unwrap())
}

#[test]
fn a_step_is_recorded_with_everything_the_plan_lists_and_read_back() {
    let store = the_store();

    insert(
        &store,
        &Step {
            task_id: "turn-1".into(), step_id: "turn-1.s1".into(), session_id: "s1".into(), step: 1, model_key: "ollama:qwen3.5:9b".into(), mode: "local-only".into(), purpose: "main".into(),
            tokens_in: 1200, tokens_out: 80, thinking_tokens: 30, ttft_ms: 400, tok_per_s: 25.0, wall_ms: 3600, verify: Some("PASS".into()), cost_usd: Some(0.0), escalation_reason: None,
        },
        "2026-10-17T00:00:00Z",
    )
    .unwrap();

    let rows = recent(&store, Some("s1"), 10);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["modelKey"], "ollama:qwen3.5:9b");
    assert_eq!((rows[0]["tokensIn"].clone(), rows[0]["thinkingTokens"].clone(), rows[0]["ttftMs"].clone(), rows[0]["wallMs"].clone()), (json!(1200), json!(30), json!(400), json!(3600)));
    assert_eq!(rows[0]["verify"], "PASS");
    assert!(recent(&store, Some("another"), 10).is_empty(), "rows are per session");
}

#[test]
fn the_verify_verdict_lands_on_the_sessions_newest_step() {
    let store = the_store();

    for step in 1..=3u32 {
        insert(&store, &Step { task_id: "t".into(), step_id: format!("t.s{step}"), session_id: "s".into(), step, model_key: "m".into(), mode: "pinned".into(), purpose: "main".into(), wall_ms: 10, ..Step::default() }, "2026").unwrap();
    }

    set_verify(&store, "s", "FAIL").unwrap();

    let rows = recent(&store, Some("s"), 10);

    assert_eq!(rows.iter().map(|row| row["verify"].clone()).collect::<Vec<_>>(), vec![Value::Null, Value::Null, json!("FAIL")]);
    assert_eq!(rows[0]["step"], 1, "oldest first");
}

#[test]
fn the_rate_is_output_over_the_time_after_the_first_token() {
    assert!((Step::rate(50, 1000, 2000) - 50.0).abs() < 1e-9);
    assert!(Step::rate(10, 5000, 1000).is_finite(), "a first token later than the end does not divide by zero");
}

/// The plan: trace overhead under 1 ms per step. What a step pays is the send into the writer's channel.
#[test]
fn recording_a_step_costs_the_step_under_a_millisecond() {
    let _ = test_store();

    let started = std::time::Instant::now();

    for step in 0..2_000 {
        record(Step { task_id: "bench".into(), step_id: format!("bench.s{step}"), model_key: "m".into(), mode: "pinned".into(), purpose: "main".into(), wall_ms: 5, ..Step::default() });
    }

    let each = started.elapsed() / 2_000;

    flush();

    assert!(each < std::time::Duration::from_millis(1), "recording took {each:?} per step");
}

#[test]
fn single_step_timings_are_a_median_and_a_p90_per_model() {
    let mut runs = set("m", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);

    runs.push(sample("long", "m", 600.0, 20));

    let timing = timings(&runs);

    assert_eq!(timing.len(), 1);
    assert_eq!(timing[0].runs, 10, "a 20-step task is not single-step work");
    assert_eq!(timing[0].median_s, 6.0);
    assert_eq!(timing[0].p90_s, 9.0);
}

#[test]
fn the_gate_passes_within_five_percent_and_fails_beyond_it() {
    let baseline = set("m", &[2.0; 20]);

    assert!(gate(&baseline, &set("m", &[2.09; 20]), TOLERANCE).pass, "4.5% slower is within the tolerance");
    assert!(!gate(&baseline, &set("m", &[2.11; 20]), TOLERANCE).pass, "5.5% slower is not");
    assert!(gate(&baseline, &set("m", &[1.0; 20]), TOLERANCE).pass, "faster is fine");
}

/// The plan's test: a gate that "fails on an injected 10% slowdown".
#[test]
fn the_gate_fails_on_an_injected_ten_percent_slowdown() {
    let baseline = set("m", &[1.0, 1.2, 1.4, 1.6, 1.8, 2.0, 2.2, 2.4, 2.6, 2.8]);
    let slowed: Vec<Sample> = baseline.iter().map(|sample| Sample { wall_s: sample.wall_s * 1.10, ..sample.clone() }).collect();
    let verdict = gate(&baseline, &slowed, TOLERANCE);

    assert!(!verdict.pass);
    assert!(verdict.lines.iter().any(|line| line.contains("TOO SLOW") && line.contains("+10.0%")), "{:?}", verdict.lines);
}

#[test]
fn a_slow_tail_alone_fails_the_p90() {
    let baseline = set("m", &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0]);
    let tail = set("m", &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 3.0, 3.0]);

    assert!(!gate(&baseline, &tail, TOLERANCE).pass, "the median is the same, the p90 is not");
}

#[test]
fn a_question_the_graph_answers_must_make_no_model_call() {
    let baseline = set("m", &[1.0; 5]);
    let mut candidate = set("m", &[1.0; 5]);

    candidate.push(Sample { task: "where-is-x".into(), model: "m".into(), wall_s: 0.01, steps: 0, requests: 0, zero_model: true });

    assert!(gate(&baseline, &candidate, TOLERANCE).pass);

    candidate.push(Sample { task: "who-calls-y".into(), model: "m".into(), wall_s: 3.0, steps: 1, requests: 1, zero_model: true });

    let verdict = gate(&baseline, &candidate, TOLERANCE);

    assert!(!verdict.pass && verdict.lines.iter().any(|line| line.contains("who-calls-y")), "{:?}", verdict.lines);
}

#[test]
fn a_model_with_no_baseline_is_said_not_silently_passed() {
    let verdict = gate(&set("old", &[1.0; 5]), &set("new", &[1.0; 5]), TOLERANCE);

    assert!(verdict.lines.iter().any(|line| line.contains("no baseline")));
}
