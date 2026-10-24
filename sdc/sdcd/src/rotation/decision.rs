//! **When does a step go to the chain? (M1-2)** - the signals the agent loop already has, joined into one decision.
//!
//! Nothing here is new evidence. The loop already knows when a printed tool call could not be repaired (`agent::rescue`), when a check
//! failed (`runtime::step`, `verify`), when the turn goes round in circles (`sme::confidence`), when a prompt will not fit (`guard::context`),
//! when the model is silent, when it refuses and when a quote it wrote is not in the file (`sme::quote`). [`StepSignals`] collects them
//! for one step and [`decide`] turns them into [`Decision`] - a pure function, so the rule is a table a test can read.
//!
//! Two rules from the plan are in the code, not in a comment:
//!
//! * **A low confidence alone never relays** (principle 4: the compiler is the judge, not the model's doubt). It counts only beside
//!   another signal.
//! * **The limits come before the relay.** A step that wants the chain when the turn has used its cloud steps or its dollars asks the person once
//!   ([`Decision::AskUser`]) instead of spending.

use super::policy::RelayPolicy;

/// Why a step is relayed. The names are what the ledger, the receipt and the Trace say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelayReason {
    /// A tool call the model printed (or malformed) twice, after rescue.
    InvalidToolCall,
    /// The step's check failed `local_attempts` times.
    VerifyFail,
    /// The same calls over and over (A-B-A-B).
    Loop,
    /// The prompt does not fit the model's window even after folding.
    ContextOverflow,
    /// The local model took longer than `local_ttft_s` to say anything.
    Slow,
    /// The model said it cannot, or answered nothing.
    Refusal,
    /// A quote the model wrote is not in the file it names, twice.
    QuoteFail,
}

impl RelayReason {
    pub fn name(self) -> &'static str {
        match self {
            RelayReason::InvalidToolCall => "invalid_tool_call",
            RelayReason::VerifyFail => "verify_fail",
            RelayReason::Loop => "loop",
            RelayReason::ContextOverflow => "context_overflow",
            RelayReason::Slow => "slow",
            RelayReason::Refusal => "refusal",
            RelayReason::QuoteFail => "quote_fail",
        }
    }

    /// A sentence for the receipt, in English (the app translates the receipt's frame, not this).
    pub fn sentence(self) -> &'static str {
        match self {
            RelayReason::InvalidToolCall => "the local model's tool call stayed invalid",
            RelayReason::VerifyFail => "a check kept failing",
            RelayReason::Loop => "the steps went round in a circle",
            RelayReason::ContextOverflow => "the task did not fit the local model's window",
            RelayReason::Slow => "the local model was too slow",
            RelayReason::Refusal => "the local model could not do it",
            RelayReason::QuoteFail => "a quoted line was not in the file",
        }
    }
}

/// What one step of the local model showed. All of it is read from the loop's own state; building it changes nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepSignals {
    /// Printed or malformed tool calls in a row that rescue could not repair.
    pub invalid_tool_calls: u32,
    /// Failed checks (a command that exited non-zero after an edit, or an edit the syntax gate refused) in a row.
    pub verify_failures: u32,
    /// `sme::confidence` found a repeating pattern (a cycle, or the same error three times).
    pub looping: bool,
    /// `sme::confidence` is doubtful - a warning, not a reason.
    pub low_confidence: bool,
    /// The prompt would not fit even after the loop folded old results.
    pub context_overflow: bool,
    /// Milliseconds to the first token of the step's reply.
    pub ttft_ms: Option<u64>,
    /// The reply was a refusal, an "I cannot", or empty with no tool call.
    pub gave_up: bool,
    /// Quotes the verifier refused this step or the one before.
    pub quote_failures: u32,
}

/// What the turn has used of the relay so far.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Used {
    pub cloud_steps: u32,
    pub usd: f64,
    /// The person was already asked about going past the limits this turn (once is enough).
    pub asked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Carry on with the local model.
    StayLocal,
    /// Send this step to the chain.
    Relay(RelayReason),
    /// The step wants the chain but the turn's limits are used: ask once whether to go on.
    AskUser(RelayReason),
}

/// The reason a step needs the chain, or `None` while the local model should keep trying. Most urgent first.
pub fn reason_for(signals: &StepSignals, policy: &RelayPolicy) -> Option<RelayReason> {
    if signals.gave_up {
        return Some(RelayReason::Refusal);
    }

    if signals.invalid_tool_calls >= 2 {
        return Some(RelayReason::InvalidToolCall);
    }

    if signals.looping {
        return Some(RelayReason::Loop);
    }

    if signals.context_overflow {
        return Some(RelayReason::ContextOverflow);
    }

    if signals.quote_failures >= 2 {
        return Some(RelayReason::QuoteFail);
    }

    if signals.verify_failures >= policy.local_attempts {
        return Some(RelayReason::VerifyFail);
    }

    if signals.ttft_ms.is_some_and(|ms| ms > u64::from(policy.local_ttft_s) * 1000) {
        return Some(RelayReason::Slow);
    }

    /* Doubt alone is not a reason. Beside one failure it is enough: the failure names the reason. */
    if signals.low_confidence {
        if signals.verify_failures >= 1 {
            return Some(RelayReason::VerifyFail);
        }

        if signals.invalid_tool_calls >= 1 {
            return Some(RelayReason::InvalidToolCall);
        }

        if signals.quote_failures >= 1 {
            return Some(RelayReason::QuoteFail);
        }
    }

    None
}

/// The decision for one step. `StayLocal` whenever the relay is off.
pub fn decide(signals: &StepSignals, policy: &RelayPolicy, used: &Used) -> Decision {
    if !policy.enabled() {
        return Decision::StayLocal;
    }

    let Some(reason) = reason_for(signals, policy) else {
        return Decision::StayLocal;
    };

    let over_steps = used.cloud_steps >= policy.max_cloud_steps_per_turn;
    let over_usd = used.usd >= policy.max_usd_per_turn;

    if over_steps || over_usd {
        return if used.asked { Decision::StayLocal } else { Decision::AskUser(reason) };
    }

    Decision::Relay(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> RelayPolicy {
        RelayPolicy { auto: true, ..Default::default() }
    }

    fn with(change: impl FnOnce(&mut StepSignals)) -> StepSignals {
        let mut signals = StepSignals::default();

        change(&mut signals);

        signals
    }

    /// One row per reason, and the boundary on each side of it.
    #[test]
    fn every_reason_has_its_threshold() {
        let policy = on();
        let rows: Vec<(StepSignals, Option<RelayReason>)> = vec![
            (with(|s| s.invalid_tool_calls = 1), None),
            (with(|s| s.invalid_tool_calls = 2), Some(RelayReason::InvalidToolCall)),
            (with(|s| s.verify_failures = 1), None),
            (with(|s| s.verify_failures = 2), Some(RelayReason::VerifyFail)),
            (with(|s| s.looping = true), Some(RelayReason::Loop)),
            (with(|s| s.context_overflow = true), Some(RelayReason::ContextOverflow)),
            (with(|s| s.ttft_ms = Some(45_000)), None),
            (with(|s| s.ttft_ms = Some(45_001)), Some(RelayReason::Slow)),
            (with(|s| s.gave_up = true), Some(RelayReason::Refusal)),
            (with(|s| s.quote_failures = 1), None),
            (with(|s| s.quote_failures = 2), Some(RelayReason::QuoteFail)),
            (StepSignals::default(), None),
        ];

        for (signals, expected) in rows {
            assert_eq!(reason_for(&signals, &policy), expected, "{signals:?}");
        }
    }

    #[test]
    fn the_thresholds_follow_the_policy() {
        let policy = RelayPolicy { auto: true, local_attempts: 3, local_ttft_s: 10, ..Default::default() };

        assert_eq!(reason_for(&with(|s| s.verify_failures = 2), &policy), None);
        assert_eq!(reason_for(&with(|s| s.verify_failures = 3), &policy), Some(RelayReason::VerifyFail));
        assert_eq!(reason_for(&with(|s| s.ttft_ms = Some(10_500)), &policy), Some(RelayReason::Slow));
    }

    #[test]
    fn doubt_alone_never_relays_but_doubt_beside_a_failure_does() {
        let policy = on();

        assert_eq!(decide(&with(|s| s.low_confidence = true), &policy, &Used::default()), Decision::StayLocal);
        assert_eq!(decide(&with(|s| { s.low_confidence = true; s.verify_failures = 1 }), &policy, &Used::default()), Decision::Relay(RelayReason::VerifyFail));
        assert_eq!(decide(&with(|s| { s.low_confidence = true; s.invalid_tool_calls = 1 }), &policy, &Used::default()), Decision::Relay(RelayReason::InvalidToolCall));
        assert_eq!(decide(&with(|s| { s.low_confidence = true; s.quote_failures = 1 }), &policy, &Used::default()), Decision::Relay(RelayReason::QuoteFail));
    }

    #[test]
    fn with_the_relay_off_nothing_is_ever_relayed() {
        let off = RelayPolicy::default();

        assert_eq!(decide(&with(|s| s.gave_up = true), &off, &Used::default()), Decision::StayLocal);
    }

    #[test]
    fn the_limits_ask_once_and_then_stay_local() {
        let policy = RelayPolicy { auto: true, max_cloud_steps_per_turn: 2, max_usd_per_turn: 0.5, ..Default::default() };
        let stuck = with(|s| s.looping = true);

        assert_eq!(decide(&stuck, &policy, &Used { cloud_steps: 1, usd: 0.1, asked: false }), Decision::Relay(RelayReason::Loop));
        assert_eq!(decide(&stuck, &policy, &Used { cloud_steps: 2, usd: 0.1, asked: false }), Decision::AskUser(RelayReason::Loop));
        assert_eq!(decide(&stuck, &policy, &Used { cloud_steps: 0, usd: 0.5, asked: false }), Decision::AskUser(RelayReason::Loop), "the dollars count too");
        assert_eq!(decide(&stuck, &policy, &Used { cloud_steps: 2, usd: 0.1, asked: true }), Decision::StayLocal, "asked once; no second question");
    }

    #[test]
    fn the_most_urgent_reason_wins() {
        let many = with(|s| { s.gave_up = true; s.looping = true; s.verify_failures = 5; s.ttft_ms = Some(99_000) });

        assert_eq!(reason_for(&many, &on()), Some(RelayReason::Refusal));
    }

    /// The decision is code on the hot path of every step: it must cost nothing next to a model call.
    #[test]
    fn deciding_takes_well_under_five_milliseconds() {
        let policy = on();
        let signals = with(|s| { s.verify_failures = 1; s.low_confidence = true; s.ttft_ms = Some(2_000) });
        let started = std::time::Instant::now();

        for _ in 0..10_000 {
            std::hint::black_box(decide(std::hint::black_box(&signals), &policy, &Used::default()));
        }

        let each = started.elapsed() / 10_000;

        assert!(each < std::time::Duration::from_millis(5), "{each:?} per decision");
    }
}
