//! **The cloud step executor (M1-4)**: one relayed step, walked down the person's chain until a model has done it *and* the project's
//! own checks agree.
//!
//! The executor owns the order of events and nothing else; the model call, the check and the undo are closures the agent loop hands in
//! (`agent::relay`), so this file is a rule a test can drive with a script:
//!
//! 1. take the chain's next model, and ask the turn's gate (`Purpose::Chain`) - a model the gate refuses is skipped, never called;
//! 2. the model works (a bounded loop of its own, with the step's tools); every edit it makes goes through the Step Transaction;
//! 3. the project's checks run. **Green is the only success** - the model's own "done" is not evidence. A project with no checks
//!    has nothing to be green about; that is recorded as such (`Verified::NoChecks`), never as a pass;
//! 4. anything else - the call failed, the model gave up, a check is red - and the step's edits are undone, and the next model gets
//!    the same step. When the chain is used up the step fails honestly (`Relayed::ok == false`) and the loop says so.
//!
//! Every attempt is a [`StepRecord`] with its reason, tokens, cost and time: the ledger, the receipt and the Trace are built from them.

use crate::data::rotation::{is_local, Rotation};
use crate::guard::modelgate::{Gate, Purpose};

use super::decision::RelayReason;

/// One model of the chain that can be relayed to.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainModel {
    pub provider: String,
    pub model: String,
    /// Dollars a step may cost on this row; `0` is "no limit set" for a cloud row.
    pub max_cost_per_step: f64,
}

impl ChainModel {
    pub fn label(&self) -> String {
        format!("{}:{}", self.provider, self.model)
    }
}

/// The models a step may be relayed to, in the person's order: enabled rows that are not local. A project that is `local-only` has none.
pub fn chain_of(rotation: &Rotation, local_only: bool) -> Vec<ChainModel> {
    if local_only {
        return Vec::new();
    }

    rotation
        .usable()
        .into_iter()
        .filter(|row| !is_local(&row.model))
        .filter_map(|row| row.model.split_once(':').map(|(provider, model)| ChainModel { provider: provider.to_string(), model: model.to_string(), max_cost_per_step: row.max_cost_per_step }))
        .collect()
}

/// What one call to a chain model came to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attempt {
    /// The model finished its bounded loop with an answer (not that the work is good - the checks say that).
    pub finished: bool,
    pub answer: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: Option<f64>,
    /// Why it did not finish: the provider's words.
    pub failure: String,
    /// Paths the model changed, for the undo and the handover note.
    pub edited: Vec<String>,
}

/// What the project's own checks said after the model's work.
#[derive(Debug, Clone, PartialEq)]
pub enum Verified {
    /// These checks ran and passed.
    Green(Vec<String>),
    /// The folder has no checks to run: nothing is proven, and the record says so.
    NoChecks,
    /// A check failed; the tail of what it said.
    Red(String),
}

/// One attempt, as the ledger keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRecord {
    pub model: String,
    /// 0 for the chain's first model, 1 for the second, ...
    pub position: usize,
    pub reason: RelayReason,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: Option<f64>,
    pub duration_ms: u64,
    /// `done`, `done-unverified`, `failed`, `red`, `refused`.
    pub outcome: &'static str,
}

/// The result of relaying one step.
#[derive(Debug, Clone, PartialEq)]
pub struct Relayed {
    pub ok: bool,
    /// The model that did it (or the last one asked).
    pub model: String,
    pub answer: String,
    pub records: Vec<StepRecord>,
    pub verified: Verified,
    pub edited: Vec<String>,
    /// Why it failed, in words for the person, when it did.
    pub failure: String,
}

impl Relayed {
    pub fn cost_usd(&self) -> f64 {
        self.records.iter().filter_map(|record| record.cost_usd).sum()
    }
}

/// Relays one step. `call` runs the model, `verify` runs the checks, `undo` puts back the paths a failed attempt changed.
pub fn run(
    chain: &[ChainModel],
    reason: RelayReason,
    gate: &Gate,
    call: &mut dyn FnMut(&ChainModel, usize) -> Attempt,
    verify: &mut dyn FnMut() -> Verified,
    undo: &mut dyn FnMut(&[String]),
) -> Relayed {
    let mut records = Vec::new();
    let mut failure = String::new();
    let mut last = String::new();

    for (position, model) in chain.iter().enumerate() {
        last = model.label();

        /* The model-selection contract: a chain model is called as the chain, or not at all. */
        if let Err(violation) = gate.permit(Purpose::Chain, Some(&model.provider), &model.model) {
            failure = format!("{} was refused by the model-selection contract ({violation})", model.label());
            records.push(StepRecord { model: model.label(), position, reason, tokens_in: 0, tokens_out: 0, cost_usd: None, duration_ms: 0, outcome: "refused" });

            continue;
        }

        let started = std::time::Instant::now();
        let attempt = call(model, position);
        let duration_ms = started.elapsed().as_millis() as u64;
        let record = |outcome: &'static str| StepRecord { model: model.label(), position, reason, tokens_in: attempt.tokens_in, tokens_out: attempt.tokens_out, cost_usd: attempt.cost_usd, duration_ms, outcome };

        if !attempt.finished {
            undo(&attempt.edited);
            failure = format!("{} did not finish the step: {}", model.label(), if attempt.failure.is_empty() { "no answer" } else { attempt.failure.as_str() });
            records.push(record("failed"));

            continue;
        }

        match verify() {
            Verified::Red(tail) => {
                undo(&attempt.edited);
                failure = format!("{} finished, but the project's checks failed: {}", model.label(), tail.lines().last().unwrap_or("see the check's output"));
                records.push(record("red"));
            }
            verified => {
                records.push(record(if matches!(verified, Verified::Green(_)) { "done" } else { "done-unverified" }));

                return Relayed { ok: true, model: model.label(), answer: attempt.answer, records, verified, edited: attempt.edited, failure: String::new() };
            }
        }
    }

    if chain.is_empty() {
        failure = "the Rotation Chain has no cloud model that this project may use".to_string();
    }

    Relayed { ok: false, model: last, answer: String::new(), records, verified: Verified::NoChecks, edited: Vec::new(), failure }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Vec<ChainModel> {
        vec![
            ChainModel { provider: "deepseek".into(), model: "deepseek-chat".into(), max_cost_per_step: 0.0 },
            ChainModel { provider: "anthropic-api".into(), model: "claude-sonnet-5-5".into(), max_cost_per_step: 0.0 },
        ]
    }

    fn gate() -> Gate {
        Gate::new(Some("ollama"), "qwen3.5:9b", true, false).with_chain(&[(Some("deepseek"), "deepseek-chat"), (Some("anthropic-api"), "claude-sonnet-5-5")]).with_relay(true)
    }

    fn good(edited: &[&str]) -> Attempt {
        Attempt { finished: true, answer: "fixed".into(), tokens_in: 4_000, tokens_out: 300, cost_usd: Some(0.002), edited: edited.iter().map(|path| path.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn the_first_model_that_finishes_with_green_checks_wins() {
        let mut calls = Vec::new();
        let relayed = run(&chain(), RelayReason::Loop, &gate(), &mut |model, _| { calls.push(model.label()); good(&["src/a.rs"]) }, &mut || Verified::Green(vec!["cargo test".into()]), &mut |_| panic!("nothing to undo"));

        assert!(relayed.ok);
        assert_eq!(relayed.model, "deepseek:deepseek-chat");
        assert_eq!(calls, ["deepseek:deepseek-chat"], "the second model is never asked");
        assert_eq!(relayed.records.len(), 1);
        assert_eq!(relayed.records[0].outcome, "done");
        assert_eq!(relayed.verified, Verified::Green(vec!["cargo test".into()]));
        assert_eq!(relayed.edited, ["src/a.rs"]);
    }

    #[test]
    fn a_failed_first_model_is_undone_and_the_second_gets_the_same_step() {
        let mut undone: Vec<Vec<String>> = Vec::new();
        let relayed = run(
            &chain(),
            RelayReason::VerifyFail,
            &gate(),
            &mut |model, _| if model.provider == "deepseek" { Attempt { finished: false, failure: "HTTP 500".into(), edited: vec!["src/half.rs".into()], ..Default::default() } } else { good(&["src/b.rs"]) },
            &mut || Verified::Green(vec![]),
            &mut |paths| undone.push(paths.to_vec()),
        );

        assert!(relayed.ok && relayed.model == "anthropic-api:claude-sonnet-5-5");
        assert_eq!(undone, [vec!["src/half.rs".to_string()]], "what the failed attempt changed was put back");
        assert_eq!(relayed.records.iter().map(|record| (record.position, record.outcome)).collect::<Vec<_>>(), [(0, "failed"), (1, "done")]);
    }

    #[test]
    fn red_checks_are_not_a_success_even_if_the_model_says_done() {
        let mut undone = 0;
        let relayed = run(&chain(), RelayReason::Loop, &gate(), &mut |_, _| good(&["src/a.rs"]), &mut || Verified::Red("test result: FAILED. 1 failed".into()), &mut |_| undone += 1);

        assert!(!relayed.ok, "no model made the checks green");
        assert_eq!(undone, 2, "both attempts were undone");
        assert!(relayed.failure.contains("checks failed") && relayed.failure.contains("FAILED"));
        assert!(relayed.records.iter().all(|record| record.outcome == "red"));
    }

    #[test]
    fn no_checks_is_recorded_as_unverified_not_as_a_pass() {
        let relayed = run(&chain(), RelayReason::Loop, &gate(), &mut |_, _| good(&[]), &mut || Verified::NoChecks, &mut |_| {});

        assert!(relayed.ok);
        assert_eq!(relayed.records[0].outcome, "done-unverified");
        assert_eq!(relayed.verified, Verified::NoChecks);
    }

    #[test]
    fn a_model_the_gate_refuses_is_never_called() {
        let strict = Gate::new(Some("ollama"), "qwen3.5:9b", true, false).with_chain(&[(Some("anthropic-api"), "claude-sonnet-5-5")]).with_relay(true);
        let mut asked = Vec::new();
        let relayed = run(&chain(), RelayReason::Loop, &strict, &mut |model, _| { asked.push(model.label()); good(&[]) }, &mut || Verified::Green(vec![]), &mut |_| {});

        assert_eq!(asked, ["anthropic-api:claude-sonnet-5-5"]);
        assert_eq!(relayed.records[0].outcome, "refused");
        assert!(relayed.ok);
    }

    #[test]
    fn an_empty_chain_fails_honestly() {
        let relayed = run(&[], RelayReason::Loop, &gate(), &mut |_, _| unreachable!(), &mut || Verified::NoChecks, &mut |_| {});

        assert!(!relayed.ok && relayed.failure.contains("no cloud model"));
    }

    #[test]
    fn the_chain_is_the_enabled_cloud_rows_in_order_and_nothing_for_a_local_only_project() {
        let rotation: Rotation = toml::from_str(
            "[[chain]]\nmodel = \"ollama:qwen3.5:9b\"\n[[chain]]\nmodel = \"deepseek:deepseek-chat\"\nmax_cost_per_step = 0.05\n[[chain]]\nmodel = \"openai-api:gpt-5\"\nenabled = false\n[[chain]]\nmodel = \"anthropic-api:claude-sonnet-5-5\"\n",
        )
        .unwrap();
        let rows = chain_of(&rotation, false);

        assert_eq!(rows.iter().map(ChainModel::label).collect::<Vec<_>>(), ["deepseek:deepseek-chat", "anthropic-api:claude-sonnet-5-5"]);
        assert_eq!(rows[0].max_cost_per_step, 0.05);
        assert!(chain_of(&rotation, true).is_empty());
    }

    #[test]
    fn the_cost_of_a_relay_is_the_sum_of_its_attempts() {
        let relayed = run(&chain(), RelayReason::Loop, &gate(), &mut |_, _| good(&[]), &mut || Verified::Red("x".into()), &mut |_| {});

        assert!((relayed.cost_usd() - 0.004).abs() < 1e-9);
    }
}
