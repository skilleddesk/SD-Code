//! Reports from `bench/results/` (v3 W1): every number is computed here from the saved runs, never typed.
//!
//! `summary` is one result set per (agent × model); `compare` pairs two sets by (task, run) and applies the Q3
//! rule; `markdown_bn` writes the Bengali report the gates use.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::ledger::PARTS;
use super::runner::RunRecord;
use super::stats;

/// A saved result set: `meta.json` and `runs.jsonl`.
pub struct Results {
    pub label: String,
    pub meta: Value,
    pub runs: Vec<RunRecord>,
    pub extra: BTreeMap<String, Value>,
}

pub fn load(dir: &Path) -> Result<Results, String> {
    let meta: Value = std::fs::read_to_string(dir.join("meta.json")).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);
    let runs = std::fs::read_to_string(dir.join("runs.jsonl"))
        .map_err(|error| format!("{}: {error}", dir.join("runs.jsonl").display()))?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<RunRecord>(line).map_err(|error| format!("runs.jsonl: {error}")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut extra = BTreeMap::new();

    for name in ["search", "lang", "resume"] {
        if let Some(value) = std::fs::read_to_string(dir.join(format!("{name}.json"))).ok().and_then(|text| serde_json::from_str(&text).ok()) {
            extra.insert(name.to_string(), value);
        }
    }

    let label = dir.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();

    Ok(Results { label, meta, runs, extra })
}

/// A median and its bootstrap interval.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct Estimate {
    pub median: f64,
    pub low: f64,
    pub high: f64,
}

fn estimate(values: &[f64]) -> Estimate {
    let median = stats::median(values).unwrap_or(0.0);
    let (low, high) = stats::bootstrap_ci(values, stats::median).unwrap_or((median, median));

    Estimate { median, low, high }
}

/// One (agent × model) in one result set.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub agent: String,
    pub model: String,
    pub runs: usize,
    pub skipped: usize,
    pub passed: usize,
    pub pass_rate: f64,
    pub input: Estimate,
    pub output: Estimate,
    pub billable: Estimate,
    pub usd: Estimate,
    pub usd_total: f64,
    pub wall_s: Estimate,
    pub ttft_s: Option<Estimate>,
    /// Cached input over all input (A-08/A-09).
    pub cached_share: f64,
    /// Each ledger part's share of all input (A-02); empty without the ledger.
    pub part_shares: BTreeMap<String, f64>,
    /// The median, over tasks with two runs or more, of the input tokens' coefficient of variation (A-03).
    pub input_cv: Option<f64>,
    /// Re-reads of a path already read in the turn, over all file-read tokens (A-10).
    pub repeat_read_share: Option<f64>,
}

pub fn summarise(runs: &[RunRecord]) -> Vec<Summary> {
    let mut groups: BTreeMap<(String, String), Vec<&RunRecord>> = BTreeMap::new();

    for run in runs {
        groups.entry((run.agent.clone(), run.model.clone())).or_default().push(run);
    }

    groups
        .into_iter()
        .map(|((agent, model), group)| {
            let done: Vec<&RunRecord> = group.iter().copied().filter(|run| run.skipped.is_none()).collect();
            let column = |pick: fn(&RunRecord) -> f64| done.iter().map(|run| pick(run)).collect::<Vec<f64>>();
            let input_total: u64 = done.iter().map(|run| run.input_tokens).sum();
            let with_ledger: Vec<&&RunRecord> = done.iter().filter(|run| run.ledger).collect();
            let ledger_input: u64 = with_ledger.iter().map(|run| run.input_tokens).sum();
            let mut part_shares = BTreeMap::new();

            if ledger_input > 0 {
                for (index, part) in PARTS.iter().enumerate() {
                    part_shares.insert(part.to_string(), with_ledger.iter().map(|run| run.parts[index]).sum::<u64>() as f64 / ledger_input as f64);
                }
            }

            let mut per_task: BTreeMap<&str, Vec<f64>> = BTreeMap::new();

            for run in &done {
                per_task.entry(run.task.as_str()).or_default().push(run.input_tokens as f64);
            }

            let cvs: Vec<f64> = per_task.values().filter_map(|values| stats::cv(values)).collect();
            let read_total: u64 = done.iter().map(|run| run.read_tokens).sum();
            let ttfts: Vec<f64> = done.iter().filter_map(|run| run.ttft_ms).map(|ms| ms as f64 / 1_000.0).collect();
            let passed = done.iter().filter(|run| run.pass).count();

            Summary {
                agent,
                model,
                runs: done.len(),
                skipped: group.len() - done.len(),
                passed,
                pass_rate: if done.is_empty() { 0.0 } else { passed as f64 / done.len() as f64 },
                input: estimate(&column(|run| run.input_tokens as f64)),
                output: estimate(&column(|run| run.output_tokens as f64)),
                billable: estimate(&column(|run| run.billable_equiv)),
                usd: estimate(&column(|run| run.usd)),
                usd_total: done.iter().map(|run| run.usd).sum(),
                wall_s: estimate(&column(|run| run.wall_ms as f64 / 1_000.0)),
                ttft_s: (!ttfts.is_empty()).then(|| estimate(&ttfts)),
                cached_share: if input_total == 0 { 0.0 } else { done.iter().map(|run| run.cached_tokens).sum::<u64>() as f64 / input_total as f64 },
                part_shares,
                input_cv: stats::median(&cvs),
                repeat_read_share: (read_total > 0).then(|| done.iter().map(|run| run.repeat_read_tokens).sum::<u64>() as f64 / read_total as f64),
            }
        })
        .collect()
}

/// One model's comparison of two result sets.
#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub model: String,
    pub pairs: usize,
    pub verdict: stats::Verdict,
    pub billable_saving: Option<(f64, (f64, f64))>,
    pub input_saving: Option<(f64, (f64, f64))>,
    pub usd_saving: Option<(f64, (f64, f64))>,
}

/// Pairs `baseline` and `candidate` runs of the same agent, model, task and run index, and applies Q3 per model.
pub fn compare(baseline: &[RunRecord], candidate: &[RunRecord]) -> Vec<Comparison> {
    let key = |run: &RunRecord| (run.agent.clone(), run.model.clone(), run.task.clone(), run.run);
    let base: BTreeMap<_, &RunRecord> = baseline.iter().filter(|run| run.skipped.is_none()).map(|run| (key(run), run)).collect();
    let models: BTreeSet<String> = candidate.iter().map(|run| run.model.clone()).collect();

    models
        .into_iter()
        .filter_map(|model| {
            let pairs: Vec<(&RunRecord, &RunRecord)> = candidate
                .iter()
                .filter(|run| run.model == model && run.skipped.is_none())
                .filter_map(|run| base.get(&key(run)).map(|old| (*old, run)))
                .collect();

            if pairs.is_empty() {
                return None;
            }

            let q3_pairs: Vec<(bool, bool, f64, f64)> = pairs.iter().map(|(old, new)| (old.pass, new.pass, old.billable_equiv, new.billable_equiv)).collect();
            let saving = |pick: fn(&RunRecord) -> f64| stats::paired_saving(&pairs.iter().map(|(old, new)| (pick(old), pick(new))).collect::<Vec<_>>());

            Some(Comparison {
                model,
                pairs: pairs.len(),
                verdict: stats::q3(&q3_pairs),
                billable_saving: saving(|run| run.billable_equiv),
                input_saving: saving(|run| run.input_tokens as f64),
                usd_saving: saving(|run| run.usd),
            })
        })
        .collect()
}

fn thousands(value: f64) -> String {
    if value >= 10_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

fn ci(estimate: &Estimate, show: fn(f64) -> String) -> String {
    format!("{} [{}–{}]", show(estimate.median), show(estimate.low), show(estimate.high))
}

fn percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

fn saving_text(saving: &Option<(f64, (f64, f64))>) -> String {
    match saving {
        Some((median, (low, high))) => format!("{} [{}–{}]", percent(*median), percent(*low), percent(*high)),
        None => "—".into(),
    }
}

/// The Bengali report of one result set, and of its comparison with a baseline when one is given.
pub fn markdown_bn(results: &Results, baseline: Option<&Results>) -> String {
    let mut out = String::new();
    let meta = &results.meta;
    let summaries = summarise(&results.runs);

    out.push_str(&format!("# Bench রিপোর্ট — `{}`\n\n", results.label));
    out.push_str("> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।\n\n");
    out.push_str(&format!(
        "- তারিখ: {} · tier: {} · প্রতি task-এ run: {} · sdcd: {} · commit: `{}` · OS: {}\n",
        meta["date"].as_str().unwrap_or("?"),
        meta["tier"].as_str().unwrap_or("?"),
        meta["runs"],
        meta["sdcd_version"].as_str().unwrap_or("?"),
        meta["git_commit"].as_str().unwrap_or("?"),
        meta["os"].as_str().unwrap_or("?")
    ));

    if let Some(versions) = meta["competitor_versions"].as_object().filter(|versions| !versions.is_empty()) {
        out.push_str(&format!("- প্রতিযোগী version: {}\n", versions.iter().map(|(name, version)| format!("{name} `{}`", version.as_str().unwrap_or("?"))).collect::<Vec<_>>().join(", ")));
    }

    if let Some(ablate) = meta["ablate"].as_str() {
        out.push_str(&format!("- ablation: `{ablate}` বন্ধ\n"));
    }

    out.push_str("\nমান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।\n\n");
    out.push_str("## প্রতি model-এর ফল\n\n| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n");

    for summary in &summaries {
        out.push_str(&format!(
            "| {} | {} | {} ({}) | {} ({}/{}) | {} | {} | {} | {} | ${:.3} | {} | {} | {} |\n",
            summary.agent,
            summary.model,
            summary.runs,
            summary.skipped,
            percent(summary.pass_rate),
            summary.passed,
            summary.runs,
            ci(&summary.input, thousands),
            ci(&summary.output, thousands),
            ci(&summary.billable, thousands),
            ci(&summary.usd, |value| format!("${value:.4}")),
            summary.usd_total,
            ci(&summary.wall_s, |value| format!("{value:.0}")),
            summary.ttft_s.as_ref().map(|estimate| ci(estimate, |value| format!("{value:.1}"))).unwrap_or_else(|| "—".into()),
            percent(summary.cached_share),
        ));
    }

    let with_parts: Vec<&Summary> = summaries.iter().filter(|summary| !summary.part_shares.is_empty()).collect();

    if !with_parts.is_empty() {
        out.push_str("\n## Input কী দিয়ে তৈরি (ledger, A-02)\n\nপ্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।\n\n| agent | model | ");
        out.push_str(&PARTS.join(" | "));
        out.push_str(" | system+tools |\n|---|---|");
        out.push_str(&"---|".repeat(PARTS.len() + 1));
        out.push('\n');

        for summary in with_parts {
            let share = |part: &str| summary.part_shares.get(part).copied().unwrap_or(0.0);

            out.push_str(&format!("| {} | {} | ", summary.agent, summary.model));
            out.push_str(&PARTS.iter().map(|part| percent(share(part))).collect::<Vec<_>>().join(" | "));
            out.push_str(&format!(" | {} |\n", percent(share("system") + share("tools"))));
        }
    }

    out.push_str("\n## অনুমান যাচাইয়ের কাঁচা সংখ্যা\n\n| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |\n|---|---|---|---|---|\n");

    for summary in &summaries {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            summary.agent,
            summary.model,
            summary.input_cv.map(percent).unwrap_or_else(|| "— (একই task-এ ≥২ run লাগে)".into()),
            summary.repeat_read_share.map(percent).unwrap_or_else(|| "—".into()),
            percent(summary.cached_share)
        ));
    }

    if let Some(lang) = results.extra.get("lang").and_then(Value::as_object) {
        out.push_str("\n## ভাষা অনুযায়ী token খরচ (A-14, provider-এর নিজের গণনা)\n\n| model | ভাষা | tokens | English-এর তুলনায় | SDC-র অনুমান / আসল |\n|---|---|---|---|---|\n");

        for (model, costs) in lang {
            for cost in costs.as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "| {model} | {} | {} | {:.2}× | {:.2} |\n",
                    cost["language"].as_str().unwrap_or("?"),
                    cost["tokens"],
                    cost["ratio"].as_f64().unwrap_or(0.0),
                    cost["estimate_error"].as_f64().unwrap_or(0.0)
                ));
            }
        }
    }

    if let Some(search) = results.extra.get("search").and_then(Value::as_object) {
        out.push_str(&format!(
            "\n## Search-এর গতি (A-04/A-05)\n\nফোল্ডার: {} ফাইল · প্রথম call-এ OS cache {}।\n\n| ধরন | query | ফল | প্রথম (ms) | p50 (ms) | p95 (ms) |\n|---|---|---|---|---|---|\n",
            search.get("files").cloned().unwrap_or(Value::Null),
            if search.get("cold") == Some(&Value::Bool(true)) { "খালি করা হয়েছিল" } else { "খালি করা হয়নি (সত্যিকারের cold নয়)" }
        ));

        for timing in search.get("timings").and_then(Value::as_array).into_iter().flatten() {
            out.push_str(&format!(
                "| {} | `{}` | {} | {:.1} | {:.1} | {:.1} |\n",
                timing["kind"].as_str().unwrap_or("?"),
                timing["query"].as_str().unwrap_or("?"),
                timing["results"],
                timing["first_ms"].as_f64().unwrap_or(0.0),
                timing["p50_ms"].as_f64().unwrap_or(0.0),
                timing["p95_ms"].as_f64().unwrap_or(0.0)
            ));
        }
    }

    if let Some(resume) = results.extra.get("resume").map(|value| &value["summary"]) {
        out.push_str(&format!(
            "\n## হঠাৎ বন্ধ হওয়ার পর resume (A-19, local)\n\n| kill | turn শুরুর আগেই | resume হয়েছে | সঠিক | হার | ফাইল নষ্ট | command দুবার চলেছে |\n|---|---|---|---|---|---|---|\n| {} | {} | {} | {} | {:.1}% | {} | {} |\n",
            resume["kills"], resume["not_started"], resume["resumed"], resume["correct"], resume["correct_rate"].as_f64().unwrap_or(0.0) * 100.0, resume["damaged"], resume["duplicated_commands"]
        ));
    }

    if let Some(baseline) = baseline {
        let comparisons = compare(&baseline.runs, &results.runs);

        out.push_str(&format!("\n## তুলনা: `{}` (আগে) বনাম `{}` (এখন)\n\nQ3 নিয়ম: McNemar p ≥ 0.1, pass rate কমবে ≤ 1 point, আর billable সাশ্রয়ের 95% CI > 0।\n\n| model | জোড়া | pass পরিবর্তন | McNemar p | input সাশ্রয় | billable সাশ্রয় | USD সাশ্রয় | রায় |\n|---|---|---|---|---|---|---|---|\n", baseline.label, results.label));

        for comparison in &comparisons {
            out.push_str(&format!(
                "| {} | {} | {:+.1} pt | {:.3} | {} | {} | {} | {} |\n",
                comparison.model,
                comparison.pairs,
                comparison.verdict.pass_delta_points,
                comparison.verdict.p_value,
                saving_text(&comparison.input_saving),
                saving_text(&comparison.billable_saving),
                saving_text(&comparison.usd_saving),
                if comparison.verdict.accepted { "✅ গ্রহণযোগ্য".to_string() } else { format!("❌ {}", comparison.verdict.reasons.join("; ")) }
            ));
        }

        if comparisons.is_empty() {
            out.push_str("| — | ০ | মিল আছে এমন (task, model, run) জোড়া নেই | | | | | |\n");
        }
    }

    let failed: Vec<&RunRecord> = results.runs.iter().filter(|run| run.skipped.is_none() && !run.pass).collect();

    if !failed.is_empty() {
        out.push_str("\n## যেসব run fail করেছে\n\n| agent | model | task | run | কারণ |\n|---|---|---|---|---|\n");

        for run in failed {
            let reason = run.error.clone().into_iter().chain(run.failures.iter().cloned()).collect::<Vec<_>>().join("; ").replace('|', "\\|");

            out.push_str(&format!("| {} | {} | {} | {} | {} |\n", run.agent, run.model, run.task, run.run, reason.chars().take(240).collect::<String>()));
        }
    }

    let skipped: BTreeSet<String> = results.runs.iter().filter_map(|run| run.skipped.clone().map(|why| format!("{} — {why}", run.task))).collect();

    if !skipped.is_empty() {
        out.push_str("\n## বাদ পড়া run\n\n");

        for line in skipped {
            out.push_str(&format!("- {line}\n"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(model: &str, task: &str, index: usize, pass: bool, input: u64) -> RunRecord {
        RunRecord {
            label: "x".into(),
            agent: "sdc".into(),
            model: model.into(),
            task: task.into(),
            run: index,
            pass,
            input_tokens: input,
            cached_tokens: input / 2,
            output_tokens: 100,
            billable_equiv: input as f64,
            usd: input as f64 / 1e6,
            wall_ms: 10_000,
            ledger: true,
            parts: [input / 4, input / 4, 0, 0, input / 2, 0],
            ..Default::default()
        }
    }

    #[test]
    fn summaries_give_pass_rate_cache_share_and_ledger_shares() {
        let runs = vec![run("m", "t1", 0, true, 1_000), run("m", "t1", 1, false, 1_200), run("m", "t2", 0, true, 800)];
        let summary = &summarise(&runs)[0];

        assert_eq!((summary.runs, summary.passed), (3, 2));
        assert!((summary.cached_share - 0.5).abs() < 1e-9);
        assert!((summary.part_shares["tool_results"] - 0.5).abs() < 1e-9);
        assert!(summary.input_cv.unwrap() > 0.0, "t1 has two runs");
    }

    #[test]
    fn a_comparison_pairs_runs_and_applies_q3() {
        let baseline: Vec<RunRecord> = (0..12).map(|index| run("m", &format!("t{index}"), 0, true, 1_000)).collect();
        let candidate: Vec<RunRecord> = (0..12).map(|index| run("m", &format!("t{index}"), 0, true, 600)).collect();
        let comparisons = compare(&baseline, &candidate);

        assert_eq!(comparisons.len(), 1);
        assert_eq!(comparisons[0].pairs, 12);
        assert!(comparisons[0].verdict.accepted, "{:?}", comparisons[0].verdict.reasons);

        let report = markdown_bn(
            &Results { label: "new".into(), meta: serde_json::json!({ "date": "2026-10-14", "tier": "smoke", "runs": 1 }), runs: candidate, extra: BTreeMap::new() },
            Some(&Results { label: "old".into(), meta: Value::Null, runs: baseline, extra: BTreeMap::new() }),
        );

        assert!(report.contains("গ্রহণযোগ্য"));
        assert!(report.contains("40.0%"), "the saving is printed");
    }
}
