//! **`sdcd bench`** (v3 W1): the measurements every later v3 claim stands on.
//!
//! ```text
//! sdcd bench list        [--tier smoke|medium|full]
//! sdcd bench run         --models deepseek:deepseek-chat,anthropic-api:claude-sonnet-5-5 [--tier smoke] [--runs N]
//!                        [--label NAME] [--binary bench/bin/v1/sdcd] [--ablate FEATURE] [--max-usd 2] [--tasks a,b]
//!                        [--enable FEATURE,...] [--keep-learning] [--data-dir DIR] [--keep] [--dry-run]
//! sdcd bench competitors --models claude-code:claude-sonnet-5-5,codex:gpt-5.1-codex [--tier smoke] [...same]
//! sdcd bench lang        --models deepseek:deepseek-chat [--label NAME]
//! sdcd bench search      --root DIR [--repeats 10] [--label NAME] [--cold]
//! sdcd bench gen-tree    --out DIR [--files 50000]
//! sdcd bench speed       BASELINE CANDIDATE [--tolerance 0.05]  # the Speed Gate: single-step work <= 5% slower, zero-model = 0 calls
//! sdcd bench private                                      # where your own (git-ignored) tasks go, and an example
//! sdcd bench validate    [--tier all]                     # every task fails untouched and passes its reference
//! sdcd bench resume      [--kills 100] [--label NAME]     # A-19: kill -9 mid-turn, restart, check the work
//! sdcd bench profiles    --baseline L --candidate feature=L2[@own-baseline] [...]  # Q2: model-profiles.json from Q3 verdicts
//! sdcd bench fleet       [--label NAME]                   # A-16 on synthetic 5/10/20-server fleets
//! sdcd bench drift       --labels A,B,C [--target 0.8]   # Q4: a model's pass rate slipping away from where it was
//! sdcd bench baseline    LABEL [--as NAME] [--note TEXT] [--force]   # P0-1: freeze a finished result set
//! sdcd bench report      LABEL [--baseline LABEL]         # writes bench/results/LABEL/report-bn.md and summary.json
//!
//! Rules (P0-11, `bench::rules`): `--seed N` (default 20261017) and `--temperature T` go on every request, three runs
//! per task at least outside smoke (`--allow-few-runs` to accept none), and the noise of every result is printed.
//! sdcd bench compare     BASELINE CANDIDATE [--report-bn]  # the Q3 rule per model
//! ```
//!
//! Results go to `bench/results/<label>/`: `meta.json`, `runs.jsonl` (appended run by run, so a stopped bench
//! keeps what it measured), the raw `ledger.jsonl`, and the report. Every run spends real money; `--max-usd`
//! stops scheduling new runs once the measured spend reaches it, and `--dry-run` prints the plan only.

pub mod competitors;
pub mod lang;
pub mod ledger;
pub mod report;
pub mod rules;
pub mod sprt;
pub mod resume;
pub mod runner;
pub mod search;
pub mod stats;
pub mod task;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use runner::{Model, RunRecord};
use task::Task;

/// A v3 feature switched off for an ablation run: `SDC_ABLATE=feature[,feature]` in the bench daemon.
pub const ABLATE_ENV: &str = "SDC_ABLATE";

/// Is a v3 feature switched off by the bench (`--ablate`)? Features read this beside their own setting.
pub fn ablated(feature: &str) -> bool {
    std::env::var(ABLATE_ENV).is_ok_and(|list| list.split(',').any(|name| name.trim() == feature))
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|arg| arg == name).and_then(|index| args.get(index + 1)).cloned()
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

/// The words that are not flags or flag values.
fn positional(args: &[String]) -> Vec<String> {
    let switches = ["--keep", "--dry-run", "--report-bn", "--cold", "--keep-learning", "--force"];
    let mut words = Vec::new();
    let mut skip = false;

    for arg in args {
        if skip {
            skip = false;
        } else if arg.starts_with("--") {
            skip = !switches.contains(&arg.as_str());
        } else {
            words.push(arg.clone());
        }
    }

    words
}

/// The `bench/` folder: `--bench-dir`, or the nearest `bench/` (or `sdc/bench/`) with tasks, from here up.
fn bench_dir(args: &[String]) -> Result<PathBuf, String> {
    if let Some(dir) = flag(args, "--bench-dir") {
        return Ok(PathBuf::from(dir));
    }

    let here = std::env::current_dir().map_err(|error| error.to_string())?;

    for dir in here.ancestors() {
        for candidate in [dir.join("bench"), dir.join("sdc").join("bench")] {
            if candidate.join("tasks").is_dir() {
                return Ok(candidate);
            }
        }
    }

    Err("no bench/tasks folder here or above; pass --bench-dir".into())
}

fn results_dir(bench: &Path, label: &str) -> PathBuf {
    let path = Path::new(label);

    if path.join("runs.jsonl").exists() {
        return path.to_path_buf();
    }

    let results = bench.join("results").join(label);

    /* P0-1: a frozen baseline is named by its label like any other result set, so `--baseline 2026-10-17-deepseek`
       finds `bench/baselines/…` without anyone pasting a path into a script. */
    if !results.join("runs.jsonl").exists() && bench.join("baselines").join(label).join("runs.jsonl").exists() {
        return bench.join("baselines").join(label);
    }

    results
}

fn tier_defaults(tier: &str) -> (usize, f64) {
    match tier {
        "full" => (5, 250.0),
        "medium" => (3, 40.0),
        _ => (1, 2.0),
    }
}

fn git_commit(dir: &Path) -> String {
    task::git(dir, &["rev-parse", "--short", "HEAD"]).map(|out| out.trim().to_string()).unwrap_or_else(|_| "unknown".into())
}

/// `sdcd bench …` - the process's exit code.
pub fn main(args: &[String]) -> i32 {
    let Some(command) = args.first().cloned() else {
        eprintln!("usage: sdcd bench list|run|competitors|lang|search|gen-tree|report|compare|baseline (see `sdcd bench help`)");
        return 3;
    };
    let rest = &args[1..];
    let outcome = match command.as_str() {
        "list" => list(rest),
        "run" => run(rest, "sdc"),
        "competitors" => run(rest, "competitors"),
        "lang" => lang_cmd(rest),
        "search" => search_cmd(rest),
        "gen-tree" => gen_tree(rest),
        "report" => report_cmd(rest),
        "validate" => validate(rest),
        "private" => private_cmd(rest),
        "speed" => speed_cmd(rest),
        "resume" => resume_cmd(rest),
        "profiles" => profiles_cmd(rest),
        "fleet" => fleet_cmd(rest),
        "drift" => drift_cmd(rest),
        "baseline" => baseline_cmd(rest),
        "compare" => compare_cmd(rest),
        _ => {
            println!("{}", include_str!("mod.rs").lines().skip(2).take(19).map(|line| line.trim_start_matches("//! ")).collect::<Vec<_>>().join("\n"));
            Ok(())
        }
    };

    match outcome {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("sdcd bench: {error}");
            1
        }
    }
}

fn selected(args: &[String], bench: &Path) -> Result<(Vec<Task>, String), String> {
    let tier = flag(args, "--tier").unwrap_or_else(|| "smoke".into());

    if !["smoke", "medium", "full", "repeat", "fleet", "hard", "small", "private", "all"].contains(&tier.as_str()) {
        return Err(format!("unknown tier `{tier}`: smoke, medium, full, repeat, fleet, hard, small, private or all"));
    }

    let mut all = task::load_all(&bench.join("tasks"))?;

    /* P0-11: the person's own tasks, from `tasks/private/` (never committed), join the hard tier. */
    all.extend(task::load_private(bench)?);
    all.sort_by(|a, b| a.id.cmp(&b.id));
    /* The repeat tier (I4) runs each original before its repeat: ids sort `edit-…` before `repeat-…`. */
    let originals: Vec<String> = all.iter().filter(|task| tier == "repeat" && task.in_tier("repeat")).filter_map(|task| task.repeat_of.clone()).collect();
    let mut tasks: Vec<Task> = all.into_iter().filter(|task| tier == "all" || task.in_tier(&tier) || originals.contains(&task.id)).collect();

    if let Some(only) = flag(args, "--tasks") {
        let wanted: Vec<&str> = only.split(',').map(str::trim).collect();

        tasks.retain(|task| wanted.contains(&task.id.as_str()));
    }

    Ok((tasks, tier))
}

/// `sdcd bench speed BASELINE CANDIDATE [--tolerance 0.05]`: the Speed Gate (P0-12). Single-step work may not be more
/// than the tolerance slower than the baseline (median and p90, per model), and a task marked `zero_model` must have
/// made no model call. Exit code 1 when it fails, so a release can be stopped by it.
fn speed_cmd(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let names: Vec<&String> = args.iter().filter(|arg| !arg.starts_with("--")).collect();
    let (Some(base), Some(candidate)) = (names.first(), names.get(1)) else {
        return Err("usage: sdcd bench speed BASELINE CANDIDATE [--tolerance 0.05]".into());
    };
    let tolerance: f64 = flag(args, "--tolerance").and_then(|value| value.parse().ok()).unwrap_or(crate::trace::TOLERANCE);
    let zero_model: Vec<String> = task::load_all(&bench.join("tasks")).unwrap_or_default().into_iter().filter(|task| task.zero_model).map(|task| task.id).collect();
    let load = |label: &str| -> Result<Vec<crate::trace::Sample>, String> {
        Ok(report::load(&results_dir(&bench, label))?
            .runs
            .into_iter()
            .filter(|run| run.skipped.is_none())
            .map(|run| crate::trace::Sample { zero_model: zero_model.contains(&run.task), task: run.task, model: run.model, wall_s: run.wall_ms as f64 / 1_000.0, steps: run.steps, requests: run.requests })
            .collect())
    };
    let verdict = crate::trace::gate(&load(base)?, &load(candidate)?, tolerance);

    println!("speed gate {base} -> {candidate} (tolerance {:.0}%)", tolerance * 100.0);

    for line in &verdict.lines {
        println!("  {line}");
    }

    println!("{}", if verdict.pass { "SPEED GATE: PASS" } else { "SPEED GATE: FAIL" });

    if verdict.pass {
        Ok(())
    } else {
        Err("the speed gate failed".into())
    }
}

/// `sdcd bench private`: where the person's own tasks go, and what one looks like. Nothing is written.
fn private_cmd(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let dir = bench.join("tasks").join("private");

    println!("Private tasks live in {} - git-ignored, so they are never committed or published.", dir.display());
    println!("Each one is a normal task file, plus `hidden/<dir>/` and `reference/<id>/` beside it, and may name a real project folder:\n");
    println!("  # {}/fix-login.toml", dir.display());
    println!("  id = \"mine-01\"\n  kind = \"bugfix\"\n  fixture = \"/Users/me/code/shop\"   # an absolute folder: copied, never changed\n  prompt = '''\n  The login form accepts an empty password. Fix it.\n  '''\n  [verify]\n  hidden = \"mine-01\"\n  commands = [\"npm test --silent\"]\n");
    println!("They join the `hard` tier automatically: `sdcd bench run --tier hard --models ...`. Found now: {}", task::load_private(&bench)?.len());

    Ok(())
}

/// `sdcd bench validate`: every task must FAIL on its untouched fixture (so it measures something) and PASS
/// with its reference solution (`tasks/reference/<id>/`: files laid over the fixture, `ANSWER.txt` the answer).
/// No model is called.
fn validate(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let mut args = args.to_vec();

    if flag(&args, "--tier").is_none() {
        args.extend(["--tier".to_string(), "all".to_string()]);
    }

    let (tasks, _) = selected(&args, &bench)?;
    let mut wrong = Vec::new();

    for task in &tasks {
        if let Some(missing) = runner::missing_requirement(task) {
            println!("  {:<14} skipped: {missing}", task.id);
            continue;
        }

        let hidden = task::side_root(&bench, task, "hidden");
        let pristine = runner::prepare(task, &bench, "validate", "pristine", 0)?;
        let untouched = task::verify(task, "", &pristine, &hidden);

        runner::clean(&pristine);

        let reference = task::side_root(&bench, task, "reference").join(&task.id);
        let solved_dir = runner::prepare(task, &bench, "validate", "reference", 0)?;
        let answer = std::fs::read_to_string(reference.join("ANSWER.txt")).unwrap_or_default();

        if reference.is_dir() {
            task::copy_tree(&reference, &solved_dir)?;
            let _ = std::fs::remove_file(solved_dir.join("ANSWER.txt"));
        }

        let solved = task::verify(task, &answer, &solved_dir, &hidden);

        runner::clean(&solved_dir);

        let fine = !untouched.pass && solved.pass;

        println!("  {:<14} untouched: {:<4}  reference: {:<4} {}", task.id, if untouched.pass { "PASS" } else { "fail" }, if solved.pass { "PASS" } else { "fail" }, if fine { "ok" } else { "WRONG" });

        if !fine {
            if untouched.pass {
                println!("      the untouched fixture already passes - the task measures nothing");
            }

            for failure in &solved.failures {
                println!("      reference: {}", failure.chars().take(400).collect::<String>());
            }

            wrong.push(task.id.clone());
        }
    }

    if wrong.is_empty() {
        println!("{} task(s) valid", tasks.len());
        Ok(())
    } else {
        Err(format!("{} task(s) invalid: {}", wrong.len(), wrong.join(", ")))
    }
}

/// `sdcd bench resume`: A-19's local part, with a scripted provider (no key, no cost).
fn resume_cmd(args: &[String]) -> Result<(), String> {
    let kills = flag(args, "--kills").and_then(|value| value.parse().ok()).unwrap_or(100);
    let binary = match flag(args, "--binary") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe().map_err(|error| error.to_string())?,
    };
    let scratch = std::env::temp_dir().join("sdc-bench").join(format!("resume-{}", std::process::id()));
    let trials = resume::trials(&binary, kills, &scratch)?;
    let summary = resume::summarise(&trials);

    if std::env::var_os("SDC_BENCH_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&scratch);
    }

    println!(
        "{} kills: {} before the turn began, {} resumed; correct {}/{} ({:.1}%), damaged files {}, duplicated commands {}",
        summary.kills,
        summary.not_started,
        summary.resumed,
        summary.correct,
        summary.kills - summary.not_started,
        summary.correct_rate * 100.0,
        summary.damaged,
        summary.duplicated_commands
    );

    /* P0-4: the runner turn type, on the same journal. Its transport is a VPS, which a bench on a laptop cannot
       provide, so the trial drives the records, the writer and the replay the daemon really uses and leaves the
       ssh hop out - and says so in the line it prints. */
    /* The plan's VPS half: 20 kill points, five times each - the same 100 kills the local half runs. */
    let runner = resume::runner_trials(&scratch.join("runner"), (kills / 5).max(1), 5, 7);
    let runner_correct = runner.iter().filter(|trial| trial.correct()).count();

    println!(
        "runner (VPS) turns: {} kills, correct {}/{} ({:.1}%), damaged tails {} - records and replay only, no ssh hop on this machine",
        runner.len(),
        runner_correct,
        runner.len(),
        if runner.is_empty() { 0.0 } else { runner_correct as f64 / runner.len() as f64 * 100.0 },
        runner.iter().filter(|trial| trial.damaged_tail).count()
    );

    if let Some(label) = flag(args, "--label") {
        let bench = bench_dir(args)?;
        let out = results_dir(&bench, &label);

        std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
        std::fs::write(out.join("resume.json"), serde_json::to_string_pretty(&json!({ "summary": summary, "trials": trials })).unwrap_or_default()).map_err(|error| error.to_string())?;
        std::fs::write(
            out.join("resume-runner.json"),
            serde_json::to_string_pretty(&json!({
                "kills": runner.len(),
                "correct": runner_correct,
                "damaged_tails": runner.iter().filter(|trial| trial.damaged_tail).count(),
                "transport": "absent - the records, writer and replay the daemon uses, driven without an ssh hop",
                "trials": runner,
            }))
            .unwrap_or_default(),
        )
        .map_err(|error| error.to_string())?;
        write_meta_if_missing(&out, &label, "resume", &bench)?;
        println!("written {}", out.join("resume.json").display());
    }

    Ok(())
}

/// `sdcd bench profiles` (Q2): for each `--candidate feature=label` compared with `--baseline`, the models on which
/// the technique passed the Q3 rule get it in `protocol/model-profiles.json` (or `--out`).
fn profiles_cmd(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let baseline_label = flag(args, "--baseline").ok_or("--baseline LABEL is required")?;
    let baseline = report::load(&results_dir(&bench, &baseline_label))?;
    let candidates: Vec<(String, String)> = args
        .windows(2)
        .filter(|pair| pair[0] == "--candidate")
        .filter_map(|pair| pair[1].split_once('=').map(|(feature, label)| (feature.trim().to_string(), label.trim().to_string())))
        .collect();

    if candidates.is_empty() {
        return Err("give at least one --candidate feature=label (a run made with --enable feature)".into());
    }

    let mut profiles = crate::v3::profiles::Profiles { generated: chrono::Local::now().format("%Y-%m-%d").to_string(), ..Default::default() };

    for (feature, label) in &candidates {
        if !crate::v3::FEATURES.iter().any(|spec| spec.name == feature && spec.token_technique) {
            return Err(format!("`{feature}` is not a v3 token technique"));
        }

        /* `feature=label@baseline`: a technique measured on another tier brings its own baseline (the learning
           techniques only show on the repeat tier), so one file can hold every verdict. */
        let (label, baseline_label, own) = match label.split_once('@') {
            Some((label, own)) => (label, own, Some(report::load(&results_dir(&bench, own))?)),
            None => (label.as_str(), baseline_label.as_str(), None),
        };
        let baseline_runs = own.as_ref().map(|own| &own.runs).unwrap_or(&baseline.runs);
        let results = report::load(&results_dir(&bench, label))?;

        for comparison in report::compare(baseline_runs, &results.runs) {
            let verdict = &comparison.verdict;
            let evidence = format!(
                "{feature}: {label} vs {baseline_label} - {} pairs, pass {:+.1} pt, McNemar p {:.3}, billable saving {} → {}",
                comparison.pairs,
                verdict.pass_delta_points,
                verdict.p_value,
                comparison.billable_saving.map(|(median, (low, high))| format!("{:.1}% [{:.1}-{:.1}]", median * 100.0, low * 100.0, high * 100.0)).unwrap_or_else(|| "-".into()),
                if verdict.accepted { "accepted".to_string() } else { format!("rejected ({})", verdict.reasons.join("; ")) }
            );
            let profile = profiles.models.entry(comparison.model.clone()).or_default();

            println!("{:<40} [{}] {evidence}", comparison.model, match &comparison.sprt {
                crate::bench::sprt::Decision::Accept { log_likelihood } => format!("SPRT: better ({log_likelihood:+.2})"),
                crate::bench::sprt::Decision::Reject { log_likelihood } => format!("SPRT: not better ({log_likelihood:+.2})"),
                crate::bench::sprt::Decision::KeepSampling { log_likelihood } => format!("SPRT: take more pairs ({log_likelihood:+.2})"),
            });

            if verdict.accepted {
                profile.techniques.push(feature.clone());
            }

            profile.evidence.push(evidence);
        }
    }

    let out = flag(args, "--out").map(PathBuf::from).unwrap_or_else(|| bench.join("..").join("protocol").join("model-profiles.json"));

    std::fs::write(&out, serde_json::to_string_pretty(&profiles).unwrap_or_default() + "\n").map_err(|error| format!("{}: {error}", out.display()))?;
    println!("written {}", out.display());

    Ok(())
}

/// `sdcd bench fleet` (A-16, synthetic): token reduction of the clustered report over every output whole, and
/// whether every server that really differs is named in the report (hidden diffs must be 0).
fn fleet_cmd(args: &[String]) -> Result<(), String> {
    let mut rows = Vec::new();

    for (servers, different) in [(5, 1), (10, 2), (20, 3)] {
        let answers = crate::v3::fleet::synthetic(servers, different);
        let report = crate::v3::fleet::render("uptime && docker ps", &answers);
        let plain = crate::context::tokens_of(&crate::v3::fleet::render_plain("uptime && docker ps", &answers));
        let clustered = crate::context::tokens_of(&report);
        let groups = crate::v3::fleet::cluster(&answers);
        let named_different: usize = groups.iter().skip(1).map(|group| group.hosts.len()).sum();
        let hidden = different.saturating_sub(named_different);
        let reduction = 1.0 - clustered as f64 / plain as f64;

        println!("{servers:>3} servers, {different} really different: {plain} → {clustered} tokens ({:.1}% less), groups {}, hidden diffs {hidden}", reduction * 100.0, groups.len());
        rows.push(json!({ "servers": servers, "different": different, "plain_tokens": plain, "clustered_tokens": clustered, "reduction": reduction, "groups": groups.len(), "hidden_diffs": hidden }));
    }

    if let Some(label) = flag(args, "--label") {
        let bench = bench_dir(args)?;
        let out = results_dir(&bench, &label);

        std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
        std::fs::write(out.join("fleet.json"), serde_json::to_string_pretty(&json!({ "synthetic": true, "rows": rows })).unwrap_or_default()).map_err(|error| error.to_string())?;
        write_meta_if_missing(&out, &label, "fleet", &bench)?;
        println!("written {}", out.join("fleet.json").display());
    }

    Ok(())
}

/// `sdcd bench drift` (Q4): several labelled result sets in order - a model whose pass rate has slipped away from
/// the rate it was chosen for raises the alarm, and its profile wants another look.
fn drift_cmd(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let labels = flag(args, "--labels").ok_or("--labels A,B,C is required (the order is the order in time)")?;
    let target = flag(args, "--target").and_then(|value| value.parse::<f64>().ok()).unwrap_or(0.8);
    let slack = flag(args, "--slack").and_then(|value| value.parse::<f64>().ok()).unwrap_or(0.1);
    let alarm = flag(args, "--alarm").and_then(|value| value.parse::<f64>().ok()).unwrap_or(0.5);
    let mut by_model: std::collections::BTreeMap<String, Vec<f64>> = std::collections::BTreeMap::new();

    for label in labels.split(',').map(str::trim).filter(|label| !label.is_empty()) {
        let results = report::load(&results_dir(&bench, label))?;

        for summary in report::summarise(&results.runs) {
            by_model.entry(summary.model).or_default().push(summary.pass_rate);
        }
    }

    for (model, rates) in by_model {
        let reading: Vec<String> = rates.iter().map(|rate| format!("{:.2}", rate)).collect();

        match crate::bench::sprt::drift(&rates, target, slack, alarm) {
            Some(index) => println!("{model:<40} drifted at run {} of [{}]", index + 1, reading.join(", ")),
            None => println!("{model:<40} steady [{}]", reading.join(", ")),
        }
    }

    Ok(())
}

/// `sdcd bench baseline LABEL [--as NAME] [--note TEXT] [--force]` (P0-1): freezes a finished result set as the
/// thing a later change is measured against - `bench/baselines/NAME/` keeps `meta.json`, `runs.jsonl` and the
/// report exactly as they were measured, plus `baseline.json` with the commit and the sha256 of `runs.jsonl`.
///
/// A baseline is a claim about what the code did on a day, and every future comparison reads it, so it is written
/// once: a second freeze of the same name stops instead of quietly re-measuring what "before" means. `--as` names
/// the frozen folder - the plan's `bench/baselines/<date>/` - while the result set keeps the label it was run with.
fn baseline_cmd(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let label = positional(args).into_iter().next().ok_or("a label is required: sdcd bench baseline LABEL")?;
    let from = results_dir(&bench, &label);
    let results = report::load(&from)?;

    if results.runs.is_empty() {
        return Err(format!("{} holds no runs", from.display()));
    }

    let name = flag(args, "--as").unwrap_or_else(|| label.clone());
    let to = bench.join("baselines").join(&name);

    if to.join("baseline.json").exists() && !has(args, "--force") {
        return Err(format!("{} is already a baseline: pass --force to replace it, or freeze a new name", to.display()));
    }

    std::fs::create_dir_all(&to).map_err(|error| error.to_string())?;

    let runs = std::fs::read(from.join("runs.jsonl")).map_err(|error| error.to_string())?;

    for file in ["meta.json", "runs.jsonl", "ledger.jsonl", "report-bn.md", "seed.json"] {
        let source = from.join(file);

        if source.is_file() {
            std::fs::copy(&source, to.join(file)).map_err(|error| format!("{file}: {error}"))?;
        }
    }

    let frozen = json!({
        "baseline": name,
        "results": label,
        "frozen_at": chrono::Local::now().format("%Y-%m-%d").to_string(),
        "measured_on": results.meta["date"],
        "git_commit": results.meta["git_commit"],
        "sdcd_version": results.meta["sdcd_version"],
        "os": results.meta["os"],
        "models": results.meta["models"],
        "runs": results.runs.len(),
        /* The hash is of the file that was copied, so a baseline edited after the fact is visible as such. */
        "sha256": hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&runs)),
        "note": flag(args, "--note"),
        "from": from.display().to_string(),
    });

    std::fs::write(to.join("baseline.json"), format!("{}\n", serde_json::to_string_pretty(&frozen).unwrap_or_default())).map_err(|error| error.to_string())?;

    println!("baseline {name}: {} run(s) · {} · {}", frozen["runs"], frozen["git_commit"], to.display());

    Ok(())
}

fn list(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let (tasks, tier) = selected(args, &bench)?;

    println!("{} task(s) in tier {tier}:", tasks.len());

    for task in &tasks {
        println!("  {:<18} {:<13} {:<4} {:<14} {}", task.id, task.kind, task.language, task.fixture, task.prompt.lines().next().unwrap_or_default().chars().take(70).collect::<String>());
    }

    Ok(())
}

fn run(args: &[String], mode: &str) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let (tasks, tier) = selected(args, &bench)?;
    let (default_runs, default_usd) = tier_defaults(&tier);
    let runs: usize = flag(args, "--runs").and_then(|value| value.parse().ok()).unwrap_or(default_runs);
    let max_usd: f64 = flag(args, "--max-usd").and_then(|value| value.parse().ok()).unwrap_or(default_usd);
    /* P0-11: the rules. Three runs at least outside smoke, a fixed seed, and a stated temperature when one is given. */
    rules::check_runs(&tier, runs, has(args, "--allow-few-runs"))?;

    let seed: u64 = flag(args, "--seed").and_then(|value| value.parse().ok()).unwrap_or(rules::DEFAULT_SEED);
    let temperature: Option<f64> = flag(args, "--temperature").and_then(|value| value.parse().ok());
    let models: Vec<Model> = flag(args, "--models").ok_or("--models is required (provider:model, comma-separated)")?.split(',').map(Model::parse).collect::<Result<_, _>>()?;
    let ablate = flag(args, "--ablate");

    if mode == "competitors" {
        if let Some(model) = models.iter().find(|model| !competitors::AGENTS.contains(&model.provider.as_str())) {
            return Err(format!("`{}`: competitors are written claude-code:<model> or codex:<model>", model.label()));
        }
    }

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let default_label = format!("{date}-{tier}-{}{}", if mode == "competitors" { "competitors" } else { "sdc" }, ablate.as_deref().map(|feature| format!("-no-{feature}")).unwrap_or_default());
    let label = flag(args, "--label").unwrap_or(default_label);
    let total = tasks.len() * models.len() * runs;

    println!("bench {label}: {} task(s) × {} model(s) × {runs} run(s) = {total} run(s), budget ${max_usd:.2}", tasks.len(), models.len());

    if has(args, "--dry-run") {
        for model in &models {
            let price = if mode == "competitors" { crate::agent::price_of("", &model.model) } else { runner::price(model) };

            println!("  {:<40} price {}", model.label(), price.map(|(input, output)| format!("${input}/${output} per 1M")).unwrap_or_else(|| "unknown - USD will read 0".into()));
        }

        for task in &tasks {
            println!("  {:<18} {}", task.id, runner::missing_requirement(task).unwrap_or_else(|| "ready".into()));
        }

        return Ok(());
    }

    /* The daemon the bench starts inherits these, and `rules::apply` puts them on every request it makes. */
    std::env::set_var(rules::SEED_ENV, seed.to_string());

    match temperature {
        Some(value) => std::env::set_var(rules::TEMPERATURE_ENV, value.to_string()),
        None => std::env::remove_var(rules::TEMPERATURE_ENV),
    }

    let out = results_dir(&bench, &label);

    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;

    let binary = match flag(args, "--binary") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe().map_err(|error| error.to_string())?,
    };
    let binary_version = std::process::Command::new(&binary).arg("--version").output().map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string()).unwrap_or_default();
    let mut versions = serde_json::Map::new();

    if mode == "competitors" {
        for agent in competitors::AGENTS {
            if models.iter().any(|model| model.provider == agent) {
                versions.insert(agent.to_string(), json!(competitors::version(agent).unwrap_or_else(|| "not installed".into())));
            }
        }
    }

    let meta = json!({
        "label": label,
        "date": date,
        "mode": mode,
        "tier": tier,
        "runs": runs,
        "seed": seed,
        "temperature": temperature,
        "models": models.iter().map(Model::label).collect::<Vec<_>>(),
        "tasks": tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>(),
        "ablate": ablate,
        "enable": flag(args, "--enable"),
        "v3": crate::v3::status(),
        "max_usd": max_usd,
        "binary": binary.display().to_string(),
        "sdcd_version": binary_version,
        "git_commit": git_commit(&bench),
        "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        "competitor_versions": versions,
    });

    std::fs::write(out.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap_or_default()).map_err(|error| error.to_string())?;

    let scratch = std::env::temp_dir().join("sdc-bench").join(format!("{label}-daemon"));
    let mut daemon = if mode == "sdc" {
        Some(runner::Daemon::start_with(&binary, &scratch, ablate.as_deref(), flag(args, "--enable").as_deref(), flag(args, "--data-dir").as_deref().map(Path::new))?)
    } else {
        None
    };
    let mut client = match &daemon {
        Some(daemon) => Some(runner::Client::connect(daemon.port)?),
        None => None,
    };
    let mut spent = 0.0;
    let mut runs_file = std::fs::OpenOptions::new().create(true).append(true).open(out.join("runs.jsonl")).map_err(|error| error.to_string())?;
    let mut done = 0;

    /* Run index outermost: a budget that runs out leaves every task measured the same number of times. */
    for index in 0..runs {
        for task in &tasks {
            for model in &models {
                done += 1;

                let mut record = RunRecord {
                    label: label.clone(),
                    agent: if mode == "sdc" { "sdc".into() } else { model.provider.clone() },
                    ablate: ablate.clone(),
                    task: task.id.clone(),
                    kind: task.kind.clone(),
                    language: task.language.clone(),
                    repeat_of: task.repeat_of.clone(),
                    model: model.label(),
                    run: index,
                    ..Default::default()
                };

                if spent >= max_usd {
                    record.skipped = Some(format!("budget ${max_usd:.2} reached"));
                } else if let Some(missing) = runner::missing_requirement(task) {
                    record.skipped = Some(missing);
                } else {
                    let keep = has(args, "--keep");

                    match (client.as_mut(), &daemon) {
                        (Some(client), Some(daemon)) => run_sdc(client, daemon, task, model, &bench, &label, index, keep, has(args, "--keep-learning"), &mut record),
                        _ => run_competitor(task, model, &bench, &label, index, keep, &mut record),
                    }

                    spent += record.usd;
                }

                println!(
                    "[{done}/{total}] {:<18} {:<36} run {index}: {} · {} in ({} cached) / {} out · ${:.4} · {:.0}s",
                    task.id,
                    model.label(),
                    match (&record.skipped, record.pass) {
                        (Some(why), _) => format!("skipped ({why})"),
                        (None, true) => "PASS".into(),
                        (None, false) => format!("FAIL {}", record.error.clone().unwrap_or_else(|| record.failures.join("; ")).chars().take(120).collect::<String>()),
                    },
                    record.input_tokens,
                    record.cached_tokens,
                    record.output_tokens,
                    record.usd,
                    record.wall_ms as f64 / 1_000.0
                );

                writeln!(runs_file, "{}", serde_json::to_string(&record).unwrap_or_default()).map_err(|error| error.to_string())?;
            }
        }
    }

    if let Some(daemon) = daemon.as_mut() {
        let _ = std::fs::copy(&daemon.ledger_file, out.join("ledger.jsonl"));
        daemon.stop();
    }

    let results = report::load(&out)?;

    std::fs::write(out.join("report-bn.md"), report::markdown_bn(&results, None)).map_err(|error| error.to_string())?;
    std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&rules::machine_summary(&label, &results.meta, &results.runs)).unwrap_or_default()).map_err(|error| error.to_string())?;
    println!("spent ${spent:.4}; results in {}", out.display());

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_sdc(client: &mut runner::Client, daemon: &runner::Daemon, task: &Task, model: &Model, bench: &Path, label: &str, index: usize, keep: bool, learn: bool, record: &mut RunRecord) {
    let work = match runner::prepare(task, bench, label, &model.slug(), index) {
        Ok(work) => work,
        Err(error) => {
            record.error = Some(format!("fixture: {error}"));
            return;
        }
    };
    let before = ledger::read(&daemon.ledger_file).len();
    /* `--keep-learning`: what earlier runs on this fixture left in `.sdc/` (recipes, lessons, memory, pages read)
       is there when this one starts, as it would be for a person who keeps working in one project. */
    let learned = std::env::temp_dir().join("sdc-bench").join(label).join("learned").join(&task.fixture);

    if learn && learned.is_dir() {
        let _ = task::copy_tree(&learned, &work.join(".sdc"));
    }

    match runner::run_turn(client, task, model, &work) {
        Ok(turn) => {
            /* The ledger of this run: the records after the ones already there, of this turn's id. */
            let records: Vec<ledger::Record> = ledger::read(&daemon.ledger_file).into_iter().skip(before).collect();

            runner::fold_ledger(&records, record);

            if !record.ledger {
                record.input_tokens = turn.reported_input;
                record.cached_tokens = turn.reported_cached.min(turn.reported_input);
                record.output_tokens = turn.reported_output;
                record.tool_calls = turn.tool_cards.clone();
            }

            let price = runner::price(model);

            record.billable_equiv = runner::billable(record.input_tokens, record.cached_tokens, record.output_tokens, price);
            record.usd = runner::usd(record.input_tokens, record.cached_tokens, record.output_tokens, price);
            record.wall_ms = turn.wall_ms;
            record.ttft_ms = turn.ttft_ms;
            record.answer = turn.answer.chars().take(4_000).collect();
            record.error = turn.error.clone();

            let outcome = task::verify(task, &turn.answer, &work, &task::side_root(bench, task, "hidden"));

            record.pass = outcome.pass && !turn.timed_out;
            record.failures = outcome.failures;
        }
        Err(error) => record.error = Some(format!("{error} (daemon log: {})", daemon.log().display())),
    }

    if learn && work.join(".sdc").is_dir() {
        let _ = std::fs::remove_dir_all(&learned);
        let _ = task::copy_tree(&work.join(".sdc"), &learned);
    }

    if !keep {
        runner::clean(&work);
    }
}

fn run_competitor(task: &Task, model: &Model, bench: &Path, label: &str, index: usize, keep: bool, record: &mut RunRecord) {
    let work = match runner::prepare(task, bench, label, &model.slug(), index) {
        Ok(work) => work,
        Err(error) => {
            record.error = Some(format!("fixture: {error}"));
            return;
        }
    };
    let (report, wall_ms) = competitors::run(&model.provider, &model.model, &task.prompt, &work, Duration::from_secs(task.timeout_secs));
    let price = crate::agent::price_of("", &model.model);

    record.input_tokens = report.input_tokens;
    record.cached_tokens = report.cached_tokens;
    record.output_tokens = report.output_tokens;
    record.steps = report.steps;
    record.tool_calls = report.tool_calls.clone();
    record.billable_equiv = runner::billable(report.input_tokens, report.cached_tokens, report.output_tokens, price);
    /* Claude Code says what it cost; the catalogue price is the fallback, cached input at a tenth. */
    /* The competitor's own dollar figure is priced by its own table, so it is only right for its own models. A
       Claude Code run on another provider's model (ANTHROPIC_BASE_URL) reports Claude prices for it - for those
       the cost is worked out from the tokens at the catalogue price, as for SDC's runs. */
    let own_models = ["claude", "haiku", "sonnet", "opus"].iter().any(|name| model.model.to_lowercase().contains(name));
    let from_tokens = runner::usd(report.input_tokens, report.cached_tokens, report.output_tokens, price);

    record.usd = match report.reported_usd {
        Some(reported) if own_models || price.is_none() => reported,
        _ => from_tokens,
    };
    record.wall_ms = wall_ms;
    record.answer = report.answer.chars().take(4_000).collect();
    record.error = report.error.clone();

    let outcome = task::verify(task, &report.answer, &work, &task::side_root(bench, task, "hidden"));

    record.pass = outcome.pass && report.error.is_none();
    record.failures = outcome.failures;

    if !keep {
        runner::clean(&work);
    }
}

fn lang_cmd(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let corpus = lang::Corpus::parse(&std::fs::read_to_string(bench.join("lang").join("parallel.toml")).map_err(|error| error.to_string())?)?;
    let models: Vec<Model> = flag(args, "--models").ok_or("--models is required")?.split(',').map(Model::parse).collect::<Result<_, _>>()?;
    let label = flag(args, "--label").unwrap_or_else(|| format!("{}-lang", chrono::Local::now().format("%Y-%m-%d")));
    let out = results_dir(&bench, &label);
    let mut all = serde_json::Map::new();

    for model in &models {
        let probe = |text: &str| crate::agent::probe_input_tokens(&model.provider, &model.model, text);
        let costs = lang::measure(&corpus, &probe)?;

        for cost in &costs {
            println!("{:<36} {:<9} {:>6} tokens  {:.2}× English  (SDC estimate {:.2}× the real count)", model.label(), cost.language, cost.tokens, cost.ratio, cost.estimate_error);
        }

        all.insert(model.label(), serde_json::to_value(costs).unwrap_or(Value::Null));
    }

    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    std::fs::write(out.join("lang.json"), serde_json::to_string_pretty(&all).unwrap_or_default()).map_err(|error| error.to_string())?;
    write_meta_if_missing(&out, &label, "lang", &bench)?;
    println!("written {}", out.join("lang.json").display());

    Ok(())
}

fn write_meta_if_missing(out: &Path, label: &str, mode: &str, bench: &Path) -> Result<(), String> {
    if !out.join("runs.jsonl").exists() {
        std::fs::write(out.join("runs.jsonl"), "").map_err(|error| error.to_string())?;
    }

    if out.join("meta.json").exists() {
        return Ok(());
    }

    let meta = json!({
        "label": label,
        "date": chrono::Local::now().format("%Y-%m-%d").to_string(),
        "mode": mode,
        "sdcd_version": crate::VERSION,
        "git_commit": git_commit(bench),
        "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    });

    std::fs::write(out.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap_or_default()).map_err(|error| error.to_string())
}

fn search_cmd(args: &[String]) -> Result<(), String> {
    let root = PathBuf::from(flag(args, "--root").ok_or("--root DIR is required (sdcd bench gen-tree makes a 50k-file one)")?);
    let repeats = flag(args, "--repeats").and_then(|value| value.parse().ok()).unwrap_or(10);
    let timings = search::measure(&root, repeats)?;
    let files = walk_count(&root);

    for timing in &timings {
        println!("{:<8} {:<34} {:>5} hits  first {:>8.1} ms  p50 {:>8.1} ms  p95 {:>8.1} ms", timing.kind, timing.query, timing.results, timing.first_ms, timing.p50_ms, timing.p95_ms);
    }

    if let Some(label) = flag(args, "--label") {
        let bench = bench_dir(args)?;
        let out = results_dir(&bench, &label);

        std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
        std::fs::write(out.join("search.json"), serde_json::to_string_pretty(&json!({ "root": root.display().to_string(), "files": files, "cold": has(args, "--cold"), "repeats": repeats, "timings": timings })).unwrap_or_default())
            .map_err(|error| error.to_string())?;
        write_meta_if_missing(&out, &label, "search", &bench)?;
        println!("written {}", out.join("search.json").display());
    }

    Ok(())
}

fn walk_count(root: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(kind) if kind.is_file() => count += 1,
                _ => {}
            }
        }
    }

    count
}

fn gen_tree(args: &[String]) -> Result<(), String> {
    let out = PathBuf::from(flag(args, "--out").ok_or("--out DIR is required")?);
    let files = flag(args, "--files").and_then(|value| value.parse().ok()).unwrap_or(50_000);

    if out.exists() && std::fs::read_dir(&out).map(|mut entries| entries.next().is_some()).unwrap_or(false) {
        return Err(format!("{} is not empty; choose a new folder", out.display()));
    }

    search::generate_tree(&out, files)?;
    println!("{files} files written under {}", out.display());

    Ok(())
}

fn report_cmd(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let label = positional(args).into_iter().next().ok_or("usage: sdcd bench report LABEL [--baseline LABEL]")?;
    let out = results_dir(&bench, &label);
    let results = report::load(&out)?;
    let baseline = match flag(args, "--baseline") {
        Some(base) => Some(report::load(&results_dir(&bench, &base))?),
        None => None,
    };
    let text = report::markdown_bn(&results, baseline.as_ref());

    std::fs::write(out.join("report-bn.md"), &text).map_err(|error| error.to_string())?;
    std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&rules::machine_summary(&results.label, &results.meta, &results.runs)).unwrap_or_default()).map_err(|error| error.to_string())?;
    println!("{text}");

    Ok(())
}

fn compare_cmd(args: &[String]) -> Result<(), String> {
    let bench = bench_dir(args)?;
    let words = positional(args);
    let [base, candidate] = words.as_slice() else {
        return Err("usage: sdcd bench compare BASELINE CANDIDATE [--report-bn]".into());
    };
    let baseline = report::load(&results_dir(&bench, base))?;
    let results = report::load(&results_dir(&bench, candidate))?;

    for comparison in report::compare(&baseline.runs, &results.runs) {
        println!(
            "{:<40} pairs {:>3}  pass {:+.1} pt  p={:.3}  billable saving {}  → {}",
            comparison.model,
            comparison.pairs,
            comparison.verdict.pass_delta_points,
            comparison.verdict.p_value,
            comparison.billable_saving.map(|(median, (low, high))| format!("{:.1}% [{:.1}–{:.1}]", median * 100.0, low * 100.0, high * 100.0)).unwrap_or_else(|| "—".into()),
            if comparison.verdict.accepted { "ACCEPT".to_string() } else { format!("REJECT ({})", comparison.verdict.reasons.join("; ")) }
        );
    }

    if has(args, "--report-bn") {
        let out = results_dir(&bench, candidate);
        let text = report::markdown_bn(&results, Some(&baseline));

        std::fs::write(out.join(format!("compare-{}-bn.md", baseline.label)), &text).map_err(|error| error.to_string())?;
        println!("written {}", out.join(format!("compare-{}-bn.md", baseline.label)).display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn positional_words_skip_flags_and_their_values() {
        assert_eq!(positional(&strings(&["base", "--report-bn", "new", "--bench-dir", "x"])), ["base", "new"]);
    }

    /// A bench tree with one finished result set: what `sdcd bench run` leaves behind, minus the money.
    fn a_bench_tree(label: &str, runs: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let results = dir.path().join("results").join(label);

        std::fs::create_dir_all(dir.path().join("tasks")).unwrap();
        std::fs::create_dir_all(&results).unwrap();

        let lines: Vec<String> = (0..runs)
            .map(|run| serde_json::to_string(&RunRecord { label: label.to_string(), model: "deepseek:deepseek-chat".into(), run, pass: true, ..RunRecord::default() }).unwrap())
            .collect();

        std::fs::write(results.join("runs.jsonl"), format!("{}\n", lines.join("\n"))).unwrap();
        std::fs::write(results.join("meta.json"), r#"{"date":"2026-10-17","git_commit":"abc1234","sdcd_version":"sdcd 2.0.4 (SDCP 0.1)","os":"macos aarch64","models":["deepseek:deepseek-chat"]}"#).unwrap();

        dir
    }

    #[test]
    fn a_frozen_baseline_keeps_its_runs_and_is_found_by_its_label() {
        let dir = a_bench_tree("2026-10-17-deepseek", 3);
        let bench = dir.path().to_str().unwrap().to_string();

        baseline_cmd(&strings(&["2026-10-17-deepseek", "--bench-dir", &bench, "--as", "2026-10-17", "--note", "before P0-3"])).unwrap();

        let frozen = dir.path().join("baselines").join("2026-10-17");
        let measured = dir.path().join("results").join("2026-10-17-deepseek");

        assert_eq!(std::fs::read_to_string(frozen.join("runs.jsonl")).unwrap(), std::fs::read_to_string(measured.join("runs.jsonl")).unwrap());

        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(frozen.join("baseline.json")).unwrap()).unwrap();

        assert_eq!([manifest["baseline"].as_str(), manifest["results"].as_str()], [Some("2026-10-17"), Some("2026-10-17-deepseek")]);
        assert_eq!(manifest["runs"].as_u64(), Some(3));
        assert_eq!(manifest["git_commit"].as_str().map(str::len), Some(7));
        assert_eq!(manifest["note"], "before P0-3");
        assert_eq!(manifest["sha256"].as_str().unwrap().len(), 64);

        /* the frozen name alone has to find it: that is what `--baseline <name>` is given */
        std::fs::remove_dir_all(&measured).unwrap();
        assert_eq!(results_dir(dir.path(), "2026-10-17"), frozen);
    }

    #[test]
    fn a_baseline_is_frozen_once_and_force_replaces_it() {
        let dir = a_bench_tree("2026-10-17-deepseek", 1);
        let bench = dir.path().to_str().unwrap().to_string();
        let args = strings(&["2026-10-17-deepseek", "--bench-dir", &bench]);

        baseline_cmd(&args).unwrap();
        assert!(baseline_cmd(&args).unwrap_err().contains("already a baseline"));

        baseline_cmd(&strings(&["2026-10-17-deepseek", "--bench-dir", &bench, "--force"])).unwrap();
        assert!(dir.path().join("baselines/2026-10-17-deepseek/baseline.json").is_file());
    }

    #[test]
    fn an_ablation_names_features_in_a_list() {
        std::env::set_var(ABLATE_ENV, "ledger-dedup, repo-map");
        assert!(ablated("repo-map") && ablated("ledger-dedup") && !ablated("cache"));
        std::env::remove_var(ABLATE_ENV);
    }

    #[test]
    fn the_shipped_tasks_load_and_fill_the_plans_tiers() {
        let bench = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench");
        let everything = task::load_all(&bench.join("tasks")).unwrap();
        /* P0-11: the small-model bench is a tier of its own, so it is not among the plan's 40 that baselines measure. */
        let (small, tasks): (Vec<Task>, Vec<Task>) = everything.into_iter().partition(|task| task.tiers.iter().any(|tier| tier == "small"));
        let count = |kind: &str| tasks.iter().filter(|task| task.kind == kind).count();

        assert_eq!(small.len(), 8, "the seven situations of the small-model bench, Bangla and Banglish both");
        assert!(small.iter().all(|task| !task.in_tier("full")), "and none of them is in the full tier");
        assert!(tasks.iter().find(|task| task.id == "ml-bn").unwrap().in_tier("hard"), "the one task the baseline flipped is in the hard tier");

        /* The plan's 40: 8 find, 6 explain, 8 small edits, 6 bug fixes, 4 features, 4 DevOps, 4 multilingual. */
        assert_eq!([count("find"), count("explain"), count("edit"), count("bugfix"), count("feature"), count("devops"), count("multilingual")], [8, 6, 8, 6, 4, 4, 4]);
        assert_eq!(tasks.iter().filter(|task| task.in_tier("smoke")).count(), 8);
        assert_eq!(tasks.iter().filter(|task| task.in_tier("medium")).count(), 20);

        for task in tasks.iter().chain(&small) {
            assert!(bench.join("fixtures").join(&task.fixture).is_dir(), "{}: no fixture {}", task.id, task.fixture);

            if let Some(hidden) = &task.verify.hidden {
                assert!(bench.join("tasks").join("hidden").join(hidden).is_dir(), "{}: no hidden {hidden}", task.id);
            }

            if let Some(original) = &task.repeat_of {
                assert!(tasks.iter().any(|other| &other.id == original), "{}: repeats unknown {original}", task.id);
            }
        }
    }
}
