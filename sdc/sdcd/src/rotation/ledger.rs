//! **The Relay Ledger and the receipt (M1-7, switch `relay.ledger`)**: what went to the chain, counted from the steps themselves.
//!
//! Every attempt on a chain model is a [`StepRecord`](super::executor::StepRecord); a turn's ledger is nothing but those records plus the
//! count of steps the local model did. The receipt, the Cost Center, the Proof Pack and `sdcd relay report` all read the same numbers,
//! so they cannot disagree.
//!
//! **Cost labels (principle P4).** `measured` - the provider reported the cost itself; `priced` - the provider reported the tokens and the
//! price comes from the catalogue; `estimate` - SDC counted the tokens itself. An estimate is never shown as measured.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::decision::RelayReason;
use super::executor::{Relayed, StepRecord};

/// How a number was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Label {
    /// Counted by SDC (no provider usage). The weakest.
    Estimate,
    /// Provider tokens x the catalogue's price.
    Priced,
    /// The provider's own cost.
    Measured,
}

impl Label {
    pub fn name(self) -> &'static str {
        match self {
            Label::Estimate => "estimate",
            Label::Priced => "priced",
            Label::Measured => "measured",
        }
    }

    /// The label of a sum is its weakest part: one estimate makes the total an estimate.
    pub fn weakest(self, other: Label) -> Label {
        self.min(other)
    }
}

/// What one model did in a turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelUse {
    pub steps: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}

/// A turn's ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnLedger {
    /// Steps the local model finished without help.
    pub steps_local: u32,
    /// Steps a chain model finished (an attempt that failed is not a step, but it is in `attempts`).
    pub steps_cloud: u32,
    /// Every call made on a chain model, successful or not.
    pub attempts: u32,
    pub by_model: BTreeMap<String, ModelUse>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub label: Label,
    pub reasons: Vec<(RelayReason, u32)>,
    /// Relayed steps that no check could confirm (the folder has none).
    pub unverified: u32,
    /// Steps nobody could do.
    pub failed: u32,
    /// For each relay: the time SDC itself spent around the model (capsule, safety checkpoint, checks, undo), in milliseconds.
    pub overhead_ms: Vec<u64>,
}

impl Default for TurnLedger {
    fn default() -> Self {
        Self { steps_local: 0, steps_cloud: 0, attempts: 0, by_model: BTreeMap::new(), tokens_in: 0, tokens_out: 0, cost_usd: 0.0, label: Label::Measured, reasons: Vec::new(), unverified: 0, failed: 0, overhead_ms: Vec::new() }
    }
}

/// How an attempt's money was learned.
pub fn label_of(record: &StepRecord, tokens_reported: bool, cost_reported: bool) -> Label {
    if record.cost_usd.is_none() && record.tokens_in + record.tokens_out == 0 {
        return Label::Estimate;
    }

    match (cost_reported, tokens_reported) {
        (true, _) => Label::Measured,
        (false, true) => Label::Priced,
        (false, false) => Label::Estimate,
    }
}

impl TurnLedger {
    /// The local model finished a step by itself.
    pub fn local_step(&mut self) {
        self.steps_local += 1;
    }

    /// One relayed step, with how its money was learned.
    pub fn relayed(&mut self, relayed: &Relayed, label: Label) {
        for record in &relayed.records {
            if record.outcome == "refused" {
                continue;
            }

            self.attempts += 1;

            let used = self.by_model.entry(record.model.clone()).or_default();

            used.tokens_in += record.tokens_in;
            used.tokens_out += record.tokens_out;
            used.cost_usd += record.cost_usd.unwrap_or(0.0);
            self.tokens_in += record.tokens_in;
            self.tokens_out += record.tokens_out;
            self.cost_usd += record.cost_usd.unwrap_or(0.0);
        }

        self.label = if self.attempts == relayed.records.iter().filter(|record| record.outcome != "refused").count() as u32 { label } else { self.label.weakest(label) };

        if relayed.ok {
            self.steps_cloud += 1;
            self.by_model.entry(relayed.model.clone()).or_default().steps += 1;

            if relayed.records.last().is_some_and(|record| record.outcome == "done-unverified") {
                self.unverified += 1;
            }

            if let Some(first) = relayed.records.first() {
                match self.reasons.iter_mut().find(|(reason, _)| *reason == first.reason) {
                    Some((_, count)) => *count += 1,
                    None => self.reasons.push((first.reason, 1)),
                }
            }
        } else {
            self.failed += 1;
        }
    }

    pub fn steps_total(&self) -> u32 {
        self.steps_local + self.steps_cloud
    }

    /// The whole turn stayed on the local model.
    pub fn all_local(&self) -> bool {
        self.attempts == 0
    }

    /// The `RelaySummary` event's fields.
    pub fn summary_json(&self, session_id: &str, turn_id: &str) -> Value {
        json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "stepsTotal": self.steps_total(),
            "stepsLocal": self.steps_local,
            "stepsCloud": self.steps_cloud,
            "attempts": self.attempts,
            "failed": self.failed,
            "unverified": self.unverified,
            "byModel": self.by_model.iter().map(|(model, used)| json!({ "model": model, "steps": used.steps, "tokensIn": used.tokens_in, "tokensOut": used.tokens_out, "costUsd": used.cost_usd })).collect::<Vec<_>>(),
            "tokensIn": self.tokens_in,
            "tokensOut": self.tokens_out,
            "costUsd": self.cost_usd,
            "costLabel": self.label.name(),
            "reasons": self.reasons.iter().map(|(reason, count)| json!({ "reason": reason.name(), "count": count })).collect::<Vec<_>>(),
            "overheadMs": self.overhead_ms,
        })
    }
}

/// The p50 and p90 of some numbers (nearest rank); `None` for no numbers.
pub fn percentiles(values: &[u64]) -> Option<(u64, u64)> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();

    sorted.sort_unstable();

    let at = |p: usize| sorted[((sorted.len() * p).div_ceil(100)).clamp(1, sorted.len()) - 1];

    Some((at(50), at(90)))
}

/// The `RelayStep` event's fields for one attempt.
pub fn step_json(session_id: &str, turn_id: &str, step_id: &str, from_model: &str, record: &StepRecord, label: Label, overhead_ms: u64) -> Value {
    json!({
        "sessionId": session_id,
        "turnId": turn_id,
        "stepId": step_id,
        "fromModel": from_model,
        "toModel": record.model,
        "reason": record.reason.name(),
        "attempt": record.position + 1,
        "tokensOut": record.tokens_out,
        "tokensIn": record.tokens_in,
        "costUsd": record.cost_usd,
        "costLabel": label.name(),
        "durationMs": record.duration_ms,
        "overheadMs": overhead_ms,
        "outcome": record.outcome,
    })
}

/// Sums `RelaySummary` payloads (the event log's, one per turn) into the report `sdcd relay report` and `relay.report` print.
pub fn report(summaries: &[Value]) -> Value {
    let mut ledger = TurnLedger::default();
    let mut label = Label::Measured;
    let mut turns_with_cloud = 0u32;
    let mut daily: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut overheads: Vec<u64> = Vec::new();

    for summary in summaries {
        let number = |key: &str| summary[key].as_u64().unwrap_or(0);

        ledger.steps_local += number("stepsLocal") as u32;
        ledger.steps_cloud += number("stepsCloud") as u32;
        ledger.attempts += number("attempts") as u32;
        ledger.failed += number("failed") as u32;
        ledger.unverified += number("unverified") as u32;
        ledger.tokens_in += number("tokensIn");
        ledger.tokens_out += number("tokensOut");
        ledger.cost_usd += summary["costUsd"].as_f64().unwrap_or(0.0);

        if number("attempts") > 0 {
            turns_with_cloud += 1;
            label = label.weakest(match summary["costLabel"].as_str() {
                Some("measured") => Label::Measured,
                Some("priced") => Label::Priced,
                _ => Label::Estimate,
            });
        }

        for row in summary["byModel"].as_array().into_iter().flatten() {
            let used = ledger.by_model.entry(row["model"].as_str().unwrap_or_default().to_string()).or_default();

            used.steps += row["steps"].as_u64().unwrap_or(0) as u32;
            used.tokens_in += row["tokensIn"].as_u64().unwrap_or(0);
            used.tokens_out += row["tokensOut"].as_u64().unwrap_or(0);
            used.cost_usd += row["costUsd"].as_f64().unwrap_or(0.0);
        }

        for row in summary["reasons"].as_array().into_iter().flatten() {
            let name = row["reason"].as_str().unwrap_or_default();
            let count = row["count"].as_u64().unwrap_or(0) as u32;

            if let Some(reason) = [RelayReason::InvalidToolCall, RelayReason::VerifyFail, RelayReason::Loop, RelayReason::ContextOverflow, RelayReason::Slow, RelayReason::Refusal, RelayReason::QuoteFail, RelayReason::Preroute].into_iter().find(|reason| reason.name() == name) {
                match ledger.reasons.iter_mut().find(|(known, _)| *known == reason) {
                    Some((_, total)) => *total += count,
                    None => ledger.reasons.push((reason, count)),
                }
            }
        }

        overheads.extend(summary["overheadMs"].as_array().into_iter().flatten().filter_map(Value::as_u64));

        if let Some(day) = summary["day"].as_str() {
            let entry = daily.entry(day.to_string()).or_default();

            entry.0 += number("stepsLocal") as u32;
            entry.1 += number("stepsCloud") as u32;
        }
    }

    let total = ledger.steps_total();
    let cloud_share = if total == 0 { 0.0 } else { f64::from(ledger.steps_cloud) * 100.0 / f64::from(total) };

    json!({
        "turns": summaries.len(),
        "turnsWithCloud": turns_with_cloud,
        "stepsTotal": total,
        "stepsLocal": ledger.steps_local,
        "stepsCloud": ledger.steps_cloud,
        "cloudPercent": (cloud_share * 10.0).round() / 10.0,
        "attempts": ledger.attempts,
        "failed": ledger.failed,
        "unverified": ledger.unverified,
        "tokensIn": ledger.tokens_in,
        "tokensOut": ledger.tokens_out,
        "costUsd": ledger.cost_usd,
        "costLabel": if turns_with_cloud == 0 { "none" } else { label.name() },
        "byModel": ledger.by_model.iter().map(|(model, used)| json!({ "model": model, "steps": used.steps, "tokensIn": used.tokens_in, "tokensOut": used.tokens_out, "costUsd": used.cost_usd })).collect::<Vec<_>>(),
        "reasons": ledger.reasons.iter().map(|(reason, count)| json!({ "reason": reason.name(), "count": count })).collect::<Vec<_>>(),
        "overhead": match percentiles(&overheads) { Some((p50, p90)) => json!({ "samples": overheads.len(), "p50Ms": p50, "p90Ms": p90 }), None => Value::Null },
        "trend": daily.iter().map(|(day, (local, cloud))| json!({ "day": day, "stepsLocal": local, "stepsCloud": cloud, "cloudPercent": if local + cloud == 0 { 0.0 } else { (f64::from(*cloud) * 1000.0 / f64::from(local + cloud)).round() / 10.0 } })).collect::<Vec<_>>(),
    })
}

/// The receipt's lines in English, for the CLI and as the frame the app translates. Empty when the turn had no steps at all.
pub fn receipt_lines(summary: &Value, bengali: bool) -> Vec<String> {
    let number = |key: &str| summary[key].as_u64().unwrap_or(0);
    let (total, local, cloud) = (number("stepsTotal"), number("stepsLocal"), number("stepsCloud"));

    if total == 0 {
        return Vec::new();
    }

    let percent = |part: u64| part * 100 / total;
    let short = |tokens: u64| if tokens >= 1_000 { format!("{:.1}k", tokens as f64 / 1_000.0) } else { tokens.to_string() };
    let digits = |text: String| if bengali { text.chars().map(|c| match c { '0' => '০', '1' => '১', '2' => '২', '3' => '৩', '4' => '৪', '5' => '৫', '6' => '৬', '7' => '৭', '8' => '৮', '9' => '৯', other => other }).collect() } else { text };

    if number("attempts") == 0 {
        return vec![if bengali { digits(format!("এই কাজ: {total}টি ধাপ — সব ধাপ লোকালে")) } else { format!("This work: {total} step(s) - every step on the local model") }];
    }

    let models: Vec<String> = summary["byModel"].as_array().into_iter().flatten().filter_map(|row| row["model"].as_str()).map(str::to_string).collect();
    let reasons: Vec<String> = summary["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            let name = row["reason"].as_str().unwrap_or_default();
            let count = row["count"].as_u64().unwrap_or(0);

            format!("{count} {}", name.replace('_', " "))
        })
        .collect();

    if bengali {
        vec![
            digits(format!("এই কাজ: {total}টি ধাপ — লোকালে {local}টি ({}%), ক্লাউডে {cloud}টি ({})", percent(local), models.join(", "))),
            digits(format!("ক্লাউডে গেছে: {} token পাঠানো, {} এসেছে", short(number("tokensIn")), short(number("tokensOut")))),
            digits(format!("কারণ: {} · বিস্তারিত: Trace", reasons.join(", "))),
        ]
    } else {
        vec![
            format!("This work: {total} steps - {local} on the local model ({}%), {cloud} on the cloud ({})", percent(local), models.join(", ")),
            format!("Sent to the cloud: {} tokens out, {} back", short(number("tokensIn")), short(number("tokensOut"))),
            format!("Why: {} · details: Trace", reasons.join(", ")),
        ]
    }
}

/// `sdcd relay report` as text: the share that went to the chain, where, why, what it cost and how the share moved day by day.
pub fn report_text(report: &Value, bengali: bool) -> String {
    let number = |key: &str| report[key].as_u64().unwrap_or(0);
    let digits = |text: String| -> String { if bengali { text.chars().map(|c| if c.is_ascii_digit() { char::from_u32('০' as u32 + c.to_digit(10).unwrap_or(0)).unwrap_or(c) } else { c }).collect() } else { text } };
    let (total, local, cloud) = (number("stepsTotal"), number("stepsLocal"), number("stepsCloud"));
    let mut lines = Vec::new();

    if total == 0 {
        return if bengali { "এখনো কোনো ধাপ রেকর্ড হয়নি।".to_string() } else { "No steps recorded yet.".to_string() };
    }

    lines.push(digits(if bengali {
        format!("মোট {total}টি ধাপ ({}টি turn): লোকালে {local}টি, ক্লাউডে {cloud}টি ({}%)", number("turns"), report["cloudPercent"])
    } else {
        format!("{total} steps over {} turn(s): {local} on the local model, {cloud} on the chain ({}%)", number("turns"), report["cloudPercent"])
    }));

    if number("attempts") == 0 {
        lines.push(if bengali { "কোনো ধাপ ক্লাউডে যায়নি।".to_string() } else { "Nothing went to the chain.".to_string() });
    } else {
        lines.push(digits(if bengali {
            format!("ক্লাউডে {}টি চেষ্টা ({}টি ব্যর্থ ধাপ) · {} token পাঠানো, {} এসেছে · খরচ ${:.4} ({})", number("attempts"), number("failed"), number("tokensIn"), number("tokensOut"), report["costUsd"].as_f64().unwrap_or(0.0), report["costLabel"].as_str().unwrap_or("estimate"))
        } else {
            format!("{} attempt(s) on the chain ({} step(s) nobody could do) · {} tokens sent, {} back · cost ${:.4} ({})", number("attempts"), number("failed"), number("tokensIn"), number("tokensOut"), report["costUsd"].as_f64().unwrap_or(0.0), report["costLabel"].as_str().unwrap_or("estimate"))
        }));

        for row in report["byModel"].as_array().into_iter().flatten() {
            lines.push(digits(format!("  {} - {} step(s), ${:.4}", row["model"].as_str().unwrap_or_default(), row["steps"].as_u64().unwrap_or(0), row["costUsd"].as_f64().unwrap_or(0.0))));
        }

        let reasons: Vec<String> = report["reasons"].as_array().into_iter().flatten().map(|row| format!("{} {}", row["count"].as_u64().unwrap_or(0), row["reason"].as_str().unwrap_or_default().replace('_', " "))).collect();

        lines.push(digits(format!("{} {}", if bengali { "কারণ:" } else { "Why:" }, reasons.join(", "))));
    }

    if number("unverified") > 0 {
        lines.push(digits(if bengali { format!("{}টি ধাপ যাচাই করা যায়নি (প্রজেক্টে কোনো check নেই)।", number("unverified")) } else { format!("{} relayed step(s) could not be verified (the folder has no checks).", number("unverified")) }));
    }

    if let Some(overhead) = report["overhead"].as_object() {
        lines.push(digits(if bengali {
            format!("রিলের নিজস্ব সময় (মডেলের সময় বাদে): p50 {} ms, p90 {} ms ({}টি রিলে)", overhead["p50Ms"], overhead["p90Ms"], overhead["samples"])
        } else {
            format!("Relay overhead around the model: p50 {} ms, p90 {} ms ({} relays)", overhead["p50Ms"], overhead["p90Ms"], overhead["samples"])
        }));
    }

    let trend = report["trend"].as_array().cloned().unwrap_or_default();

    if trend.len() > 1 {
        lines.push(if bengali { "দিনে দিনে ক্লাউড-শতাংশ:".to_string() } else { "Chain share by day:".to_string() });

        for row in trend {
            lines.push(digits(format!("  {} - {}%", row["day"].as_str().unwrap_or_default(), row["cloudPercent"])));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::executor::Verified;

    fn record(model: &str, outcome: &'static str, tokens: (u64, u64), cost: Option<f64>) -> StepRecord {
        StepRecord { model: model.into(), position: 0, reason: RelayReason::VerifyFail, tokens_in: tokens.0, tokens_out: tokens.1, cost_usd: cost, duration_ms: 900, outcome }
    }

    fn relayed(records: Vec<StepRecord>, ok: bool) -> Relayed {
        Relayed { ok, model: records.last().map(|record| record.model.clone()).unwrap_or_default(), answer: String::new(), records, verified: Verified::Green(vec![]), edited: vec![], failure: String::new() }
    }

    #[test]
    fn the_totals_are_the_sum_of_the_records_and_nothing_else() {
        let mut ledger = TurnLedger::default();

        for _ in 0..10 {
            ledger.local_step();
        }

        ledger.relayed(&relayed(vec![record("deepseek:deepseek-chat", "failed", (3_000, 100), Some(0.001)), record("anthropic-api:claude-sonnet-5-5", "done", (5_400, 1_100), Some(0.002))], true), Label::Priced);

        assert_eq!((ledger.steps_local, ledger.steps_cloud, ledger.steps_total()), (10, 1, 11));
        assert_eq!(ledger.attempts, 2, "a failed attempt is counted as an attempt, not as a step");
        assert_eq!((ledger.tokens_in, ledger.tokens_out), (8_400, 1_200));
        assert!((ledger.cost_usd - 0.003).abs() < 1e-12);
        assert_eq!(ledger.by_model["anthropic-api:claude-sonnet-5-5"].steps, 1);
        assert_eq!(ledger.by_model["deepseek:deepseek-chat"].steps, 0, "the model that failed finished no step");
        assert_eq!(ledger.label, Label::Priced);
        assert!(!ledger.all_local());
    }

    #[test]
    fn an_estimate_is_never_called_measured() {
        let mut ledger = TurnLedger::default();

        ledger.relayed(&relayed(vec![record("a:b", "done", (1_000, 50), Some(0.001))], true), Label::Measured);
        assert_eq!(ledger.label, Label::Measured);

        ledger.relayed(&relayed(vec![record("a:b", "done", (1_000, 50), Some(0.001))], true), Label::Estimate);
        assert_eq!(ledger.label, Label::Estimate, "one estimate in the sum makes the whole an estimate");

        assert_eq!(label_of(&record("a:b", "done", (0, 0), None), false, false), Label::Estimate);
        assert_eq!(label_of(&record("a:b", "done", (10, 5), Some(0.1)), true, true), Label::Measured);
        assert_eq!(label_of(&record("a:b", "done", (10, 5), Some(0.1)), true, false), Label::Priced);
    }

    #[test]
    fn a_turn_that_never_left_the_machine_says_so() {
        let mut ledger = TurnLedger::default();

        ledger.local_step();
        ledger.local_step();

        let lines = receipt_lines(&ledger.summary_json("s", "t"), true);

        assert_eq!(lines, ["এই কাজ: ২টি ধাপ — সব ধাপ লোকালে"]);
        assert!(ledger.all_local());
        assert_eq!(receipt_lines(&ledger.summary_json("s", "t"), false), ["This work: 2 step(s) - every step on the local model"]);
        assert!(receipt_lines(&TurnLedger::default().summary_json("s", "t"), false).is_empty(), "a turn with no steps has no receipt");
    }

    #[test]
    fn the_receipt_matches_the_plans_example_in_bengali() {
        let mut ledger = TurnLedger::default();

        for _ in 0..10 {
            ledger.local_step();
        }

        ledger.relayed(&relayed(vec![record("deepseek:deepseek-chat", "done", (4_200, 550), Some(0.0016))], true), Label::Priced);
        ledger.relayed(&relayed(vec![StepRecord { reason: RelayReason::Loop, ..record("deepseek:deepseek-chat", "done", (4_200, 550), Some(0.0015)) }], true), Label::Priced);

        let lines = receipt_lines(&ledger.summary_json("s", "t"), true);

        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("এই কাজ: ১২টি ধাপ — লোকালে ১০টি (৮৩%), ক্লাউডে ২টি (deepseek:deepseek-chat)"), "{}", lines[0]);
        assert!(lines[1].contains("৮.৪k token পাঠানো") && lines[1].contains("১.১k এসেছে") && !lines[1].contains('$'), "{}", lines[1]);
        assert!(lines[2].contains("১ verify fail") && lines[2].contains("১ loop"), "{}", lines[2]);
    }

    #[test]
    fn a_failed_relay_is_counted_and_no_cloud_step_is_claimed() {
        let mut ledger = TurnLedger::default();

        ledger.relayed(&relayed(vec![record("a:b", "red", (1_000, 100), Some(0.001))], false), Label::Priced);

        assert_eq!((ledger.steps_cloud, ledger.failed, ledger.attempts), (0, 1, 1));
    }

    #[test]
    fn the_report_adds_summaries_up_and_shows_the_share_and_the_trend() {
        let mut one = TurnLedger::default();

        for _ in 0..9 {
            one.local_step();
        }

        one.relayed(&relayed(vec![record("deepseek:deepseek-chat", "done", (4_000, 500), Some(0.002))], true), Label::Priced);

        let mut a = one.summary_json("s", "t1");
        let mut b = TurnLedger::default();

        for _ in 0..5 {
            b.local_step();
        }

        let mut b = b.summary_json("s", "t2");

        a["day"] = json!("2026-10-01");
        b["day"] = json!("2026-10-02");

        let report = report(&[a, b]);

        assert_eq!((report["turns"].as_u64(), report["turnsWithCloud"].as_u64()), (Some(2), Some(1)));
        assert_eq!((report["stepsLocal"].as_u64(), report["stepsCloud"].as_u64(), report["stepsTotal"].as_u64()), (Some(14), Some(1), Some(15)));
        assert_eq!(report["cloudPercent"], 6.7);
        assert_eq!(report["costLabel"], "priced");
        assert_eq!(report["trend"][0]["cloudPercent"], 10.0);
        assert_eq!(report["trend"][1]["cloudPercent"], 0.0);
        assert_eq!(report["byModel"][0]["model"], "deepseek:deepseek-chat");
        assert_eq!(super::report(&[])["costLabel"], "none");
    }

    #[test]
    fn a_step_event_names_where_it_went_and_why() {
        let event = step_json("s1", "t1", "t1.r1", "ollama:qwen3.5:9b", &record("deepseek:deepseek-chat", "done", (4_000, 500), Some(0.002)), Label::Priced, 120);

        assert_eq!(event["toModel"], "deepseek:deepseek-chat");
        assert_eq!(event["reason"], "verify_fail");
        assert_eq!(event["costLabel"], "priced");
        assert_eq!(event["attempt"], 1);
        assert_eq!(event["overheadMs"], 120);
    }

    #[test]
    fn the_report_reads_as_text_in_both_languages() {
        let mut ledger = TurnLedger::default();

        for _ in 0..9 {
            ledger.local_step();
        }

        ledger.relayed(&relayed(vec![record("deepseek:deepseek-chat", "done-unverified", (4_000, 500), Some(0.002))], true), Label::Priced);

        let mut summary = ledger.summary_json("s", "t1");

        summary["day"] = json!("2026-10-01");

        let data = report(&[summary]);
        let english = report_text(&data, false);
        let bangla = report_text(&data, true);

        assert!(english.contains("10 steps over 1 turn(s): 9 on the local model, 1 on the chain (10.0%)"), "{english}");
        assert!(english.contains("deepseek:deepseek-chat - 1 step(s)") && english.contains("(priced)") && english.contains("could not be verified"));
        assert!(bangla.contains("মোট ১০টি ধাপ (১টি turn): লোকালে ৯টি, ক্লাউডে ১টি (১০.০%)"), "{bangla}");
        assert_eq!(report_text(&report(&[]), false), "No steps recorded yet.");
        assert_eq!(report_text(&report(&[]), true), "এখনো কোনো ধাপ রেকর্ড হয়নি।");
    }

    #[test]
    fn the_overhead_is_reported_as_p50_and_p90() {
        assert_eq!(percentiles(&[]), None);
        assert_eq!(percentiles(&[100]), Some((100, 100)));
        assert_eq!(percentiles(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 100]), Some((50, 90)));

        let mut ledger = TurnLedger::default();

        ledger.local_step();
        ledger.overhead_ms = vec![200, 400, 900];

        let data = report(&[ledger.summary_json("s", "t")]);

        assert_eq!(data["overhead"], json!({ "samples": 3, "p50Ms": 400, "p90Ms": 900 }));
        assert!(report_text(&data, false).contains("p50 400 ms, p90 900 ms (3 relays)"));
    }
}
