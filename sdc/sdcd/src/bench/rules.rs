//! **Bench rules (P0-11)**: what makes a bench result worth believing.
//!
//! A pass rate from one run is a coin flip dressed as a number: at a temperature above zero the same task passes
//! on one run and fails on the next, and "A beat B" can be nothing but that. So, in code and not in a README:
//!
//! 1. **a fixed seed** and a stated temperature, sent with every request (`SDC_BENCH_SEED`, `SDC_BENCH_TEMPERATURE`
//!    reach the daemon the bench starts, and [`apply`] puts them on the request body);
//! 2. **at least three runs per task** outside the smoke tier (`--allow-few-runs` says out loud that the result has
//!    no noise estimate);
//! 3. **paired runs** (same task, same run index, two builds) for McNemar - that is `report::compare`, and its
//!    McNemar helper (`stats::mcnemar_p`) is tested against the binomial table;
//! 4. **noise next to every result**: [`noise`] says how many tasks flipped between runs and how far apart the
//!    runs' pass rates were, and it goes into the report and into `summary.json`;
//! 5. **a budget** (`--max-usd`), which the bench already enforced.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::runner::RunRecord;

pub const DEFAULT_SEED: u64 = 20_261_017;
pub const MIN_RUNS: usize = 3;
pub const SEED_ENV: &str = "SDC_BENCH_SEED";
pub const TEMPERATURE_ENV: &str = "SDC_BENCH_TEMPERATURE";

/// The runs the plan asks for: three or more, except in the smoke tier (a quick "does it still work").
pub fn check_runs(tier: &str, runs: usize, allow_few: bool) -> Result<(), String> {
    if tier == "smoke" || allow_few || runs >= MIN_RUNS {
        return Ok(());
    }

    Err(format!("{runs} run(s) per task is not enough to tell a difference from noise: the {tier} tier needs at least {MIN_RUNS} (--runs {MIN_RUNS}), or --allow-few-runs to accept a result with no noise estimate"))
}

/// Puts the bench's seed and temperature on a request body. `native` is Ollama's own `/api/chat` shape, which keeps
/// them under `options`; `thinking` leaves the temperature alone (an Anthropic request with thinking on must use 1).
pub fn apply(body: &mut Value, native: bool, thinking: bool) {
    let seed = std::env::var(SEED_ENV).ok().and_then(|text| text.trim().parse::<u64>().ok());
    let temperature = std::env::var(TEMPERATURE_ENV).ok().and_then(|text| text.trim().parse::<f64>().ok());

    if seed.is_none() && temperature.is_none() {
        return;
    }

    let target = if native { &mut body["options"] } else { &mut *body };

    if let Some(seed) = seed {
        target["seed"] = json!(seed);
    }

    if let Some(temperature) = temperature.filter(|_| !thinking) {
        target["temperature"] = json!(temperature);
    }
}

/// How much of a result is chance.
#[derive(Debug, Clone, PartialEq)]
pub struct Noise {
    pub tasks: usize,
    /// Tasks with at least two runs whose runs did not all agree.
    pub flaky_tasks: usize,
    /// Each run index's pass rate over the tasks, ascending by index.
    pub run_pass_rates: Vec<f64>,
    /// The widest gap between two runs' pass rates, in points of 0..=1.
    pub spread: f64,
}

pub fn noise(runs: &[RunRecord]) -> Noise {
    let mut by_task: BTreeMap<&str, Vec<bool>> = BTreeMap::new();
    let mut by_run: BTreeMap<usize, (usize, usize)> = BTreeMap::new();

    for run in runs.iter().filter(|run| run.skipped.is_none()) {
        by_task.entry(run.task.as_str()).or_default().push(run.pass);

        let entry = by_run.entry(run.run).or_default();

        entry.0 += usize::from(run.pass);
        entry.1 += 1;
    }

    let run_pass_rates: Vec<f64> = by_run.values().map(|(passed, total)| *passed as f64 / (*total).max(1) as f64).collect();
    let spread = run_pass_rates.iter().cloned().fold(f64::MIN, f64::max) - run_pass_rates.iter().cloned().fold(f64::MAX, f64::min);

    Noise {
        tasks: by_task.len(),
        flaky_tasks: by_task.values().filter(|outcomes| outcomes.len() > 1 && outcomes.iter().any(|pass| *pass) && outcomes.iter().any(|pass| !*pass)).count(),
        spread: if run_pass_rates.len() > 1 { spread } else { 0.0 },
        run_pass_rates,
    }
}

/// `summary.json`: the numbers a machine can read, with the rules the result was produced under.
pub fn machine_summary(label: &str, meta: &Value, runs: &[RunRecord]) -> Value {
    let mut groups: BTreeMap<(String, String), Vec<RunRecord>> = BTreeMap::new();

    for run in runs {
        groups.entry((run.agent.clone(), run.model.clone())).or_default().push(run.clone());
    }

    let results: Vec<Value> = groups
        .into_iter()
        .map(|((agent, model), group)| {
            let done: Vec<&RunRecord> = group.iter().filter(|run| run.skipped.is_none()).collect();
            let passed = done.iter().filter(|run| run.pass).count();
            let noise = noise(&group);

            json!({
                "agent": agent,
                "model": model,
                "runs": done.len(),
                "passed": passed,
                "pass_rate": if done.is_empty() { 0.0 } else { passed as f64 / done.len() as f64 },
                "noise": {
                    "tasks": noise.tasks,
                    "flaky_tasks": noise.flaky_tasks,
                    "run_pass_rates": noise.run_pass_rates,
                    "spread": noise.spread,
                },
                "usd": done.iter().map(|run| run.usd).sum::<f64>(),
            })
        })
        .collect();

    json!({
        "schema": 1,
        "label": label,
        "tier": meta["tier"],
        "runs_per_task": meta["runs"],
        "rules": {
            "seed": std::env::var(SEED_ENV).ok().and_then(|text| text.parse::<u64>().ok()).unwrap_or(DEFAULT_SEED),
            "temperature": std::env::var(TEMPERATURE_ENV).ok().and_then(|text| text.parse::<f64>().ok()),
            "min_runs": MIN_RUNS,
        },
        "results": results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(task: &str, run: usize, pass: bool) -> RunRecord {
        RunRecord { label: "l".into(), agent: "sdc".into(), task: task.into(), kind: "edit".into(), language: "en".into(), model: "m".into(), run, pass, ..RunRecord::default() }
    }

    #[test]
    fn fewer_than_three_runs_is_refused_outside_smoke_unless_said_out_loud() {
        assert!(check_runs("medium", 3, false).is_ok());
        assert!(check_runs("full", 5, false).is_ok());
        assert!(check_runs("smoke", 1, false).is_ok());
        assert!(check_runs("medium", 1, true).is_ok());

        let error = check_runs("medium", 2, false).unwrap_err();

        assert!(error.contains("at least 3") && error.contains("--allow-few-runs"), "{error}");
    }

    #[test]
    fn the_seed_and_temperature_reach_the_request_in_the_shape_each_endpoint_takes() {
        let _lock = crate::sme::probe::DATA_DIR_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());

        std::env::set_var(SEED_ENV, "7");
        std::env::set_var(TEMPERATURE_ENV, "0.6");

        let mut openai = json!({ "model": "m", "messages": [] });
        let mut native = json!({ "model": "m", "options": { "num_ctx": 4096 } });
        let mut thinking = json!({ "model": "m" });

        apply(&mut openai, false, false);
        apply(&mut native, true, false);
        apply(&mut thinking, false, true);

        assert_eq!((openai["seed"].clone(), openai["temperature"].clone()), (json!(7), json!(0.6)));
        assert_eq!((native["options"]["seed"].clone(), native["options"]["temperature"].clone(), native["options"]["num_ctx"].clone()), (json!(7), json!(0.6), json!(4096)));
        assert!(thinking["temperature"].is_null() && thinking["seed"] == 7, "thinking keeps its own temperature");

        std::env::remove_var(SEED_ENV);
        std::env::remove_var(TEMPERATURE_ENV);

        let mut plain = json!({ "model": "m" });

        apply(&mut plain, false, false);

        assert_eq!(plain, json!({ "model": "m" }), "no bench, no change to any request");
    }

    #[test]
    fn noise_counts_tasks_that_flipped_and_how_far_the_runs_are_apart() {
        let runs = vec![
            record("a", 0, true), record("a", 1, true), record("a", 2, true),
            record("b", 0, true), record("b", 1, false), record("b", 2, true),
            record("c", 0, false), record("c", 1, false), record("c", 2, false),
            record("d", 0, true), record("d", 1, true), record("d", 2, false),
        ];
        let noise = noise(&runs);

        assert_eq!((noise.tasks, noise.flaky_tasks), (4, 2), "b and d flipped; a and c are steady");
        assert_eq!(noise.run_pass_rates, vec![0.75, 0.5, 0.5]);
        assert!((noise.spread - 0.25).abs() < 1e-9);
    }

    #[test]
    fn one_run_has_no_noise_estimate_and_says_zero_spread() {
        let noise = noise(&[record("a", 0, true), record("b", 0, false)]);

        assert_eq!((noise.flaky_tasks, noise.spread), (0, 0.0));
    }

    #[test]
    fn the_machine_summary_carries_the_rules_and_the_noise() {
        let runs = vec![record("a", 0, true), record("a", 1, false), record("a", 2, true)];
        let summary = machine_summary("lbl", &json!({ "tier": "medium", "runs": 3 }), &runs);

        assert_eq!(summary["schema"], 1);
        assert_eq!(summary["rules"]["min_runs"], 3);
        assert_eq!(summary["results"][0]["noise"]["flaky_tasks"], 1);
        assert!((summary["results"][0]["pass_rate"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-9);
    }
}
