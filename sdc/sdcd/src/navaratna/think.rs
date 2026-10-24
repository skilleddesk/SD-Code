//! **N7-3 - the thinking budget** (switch `nr.think_budget`, off).
//!
//! Thinking tokens cost time and window and are only worth it where reasoning decides the result. If the local model supports a thinking mode
//! (read from its metadata or the probe, never assumed), it thinks for **planning, test-writing and error-fix** steps and does not for
//! boilerplate and formatting steps. The thinking the model may do counts toward the answer's reserve, so it can never push the prompt out of
//! the window.

use serde_json::Value;

use crate::sme::profile::Step as ProfileStep;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plan,
    TestWriting,
    ErrorFix,
    Boilerplate,
    Formatting,
    /// Reading and the rest: the model's own default stands.
    Other,
}

/// The kind of step from what the step profile says it is doing, and whether it is writing tests.
pub fn kind_of(step: ProfileStep, writing_tests: bool, boilerplate: bool) -> Kind {
    match step {
        ProfileStep::Plan => Kind::Plan,
        ProfileStep::Debug => Kind::ErrorFix,
        ProfileStep::Format => Kind::Formatting,
        ProfileStep::Patch if writing_tests => Kind::TestWriting,
        ProfileStep::Patch if boilerplate => Kind::Boilerplate,
        ProfileStep::Read | ProfileStep::Review | ProfileStep::Patch => Kind::Other,
    }
}

/// Does a model support a thinking mode? From `ollama show`'s `capabilities` list or the stored probe - never from the model's name alone.
pub fn supports_thinking(capabilities: &Value, probed_thinking: Option<bool>) -> bool {
    capabilities.as_array().is_some_and(|list| list.iter().any(|item| item.as_str() == Some("thinking"))) || probed_thinking == Some(true)
}

/// `Some(true)` think, `Some(false)` do not, `None` leave the model's default (also for a model that cannot think at all).
pub fn decide(kind: Kind, capable: bool) -> Option<bool> {
    if !capable {
        return None;
    }

    match kind {
        Kind::Plan | Kind::TestWriting | Kind::ErrorFix => Some(true),
        Kind::Boilerplate | Kind::Formatting => Some(false),
        Kind::Other => None,
    }
}

/// How many tokens thinking may take in a step that thinks: a quarter of the answer's reserve, at least 512.
pub fn thinking_allowance(max_output: u64) -> u64 {
    (max_output / 4).max(512)
}

/// The reserve a request must hold back: the answer, plus the thinking when it is on. This is what the context guard counts.
pub fn reserve(max_output: u64, thinking: Option<bool>) -> u64 {
    if thinking == Some(true) {
        max_output + thinking_allowance(max_output)
    } else {
        max_output
    }
}

/// Puts the decision into a native Ollama request body. Answers what was decided.
pub fn apply(body: &mut Value, kind: Kind, capable: bool) -> Option<bool> {
    let decision = decide(kind, capable)?;

    body["think"] = serde_json::json!(decision);
    Some(decision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn boilerplate_runs_without_thinking_and_fix_steps_run_with_it_on_a_capable_model() {
        for (kind, expected) in [(Kind::Boilerplate, Some(false)), (Kind::Formatting, Some(false)), (Kind::ErrorFix, Some(true)), (Kind::Plan, Some(true)), (Kind::TestWriting, Some(true)), (Kind::Other, None)] {
            assert_eq!(decide(kind, true), expected, "{kind:?}");
            assert_eq!(decide(kind, false), None, "{kind:?}: a model that cannot think is left alone");
        }
    }

    #[test]
    fn thinking_is_detected_from_metadata_and_never_assumed() {
        assert!(supports_thinking(&json!(["completion", "tools", "thinking"]), None));
        assert!(!supports_thinking(&json!(["completion", "tools"]), None));
        assert!(supports_thinking(&json!([]), Some(true)));
        assert!(!supports_thinking(&json!(null), None));
        assert!(!supports_thinking(&json!(null), Some(false)));
    }

    #[test]
    fn the_step_profile_maps_to_a_kind() {
        assert_eq!(kind_of(ProfileStep::Debug, false, false), Kind::ErrorFix);
        assert_eq!(kind_of(ProfileStep::Plan, false, false), Kind::Plan);
        assert_eq!(kind_of(ProfileStep::Patch, true, false), Kind::TestWriting);
        assert_eq!(kind_of(ProfileStep::Patch, false, true), Kind::Boilerplate);
        assert_eq!(kind_of(ProfileStep::Format, false, false), Kind::Formatting);
        assert_eq!(kind_of(ProfileStep::Read, false, false), Kind::Other);
    }

    #[test]
    fn thinking_counts_toward_the_reserve_and_cannot_push_the_prompt_out_of_the_window() {
        assert_eq!(reserve(2_000, Some(false)), 2_000);
        assert_eq!(reserve(2_000, None), 2_000);
        assert_eq!(reserve(2_000, Some(true)), 2_512);
        assert_eq!(reserve(8_000, Some(true)), 10_000);
        assert_eq!(reserve(100, Some(true)), 612);

        /* a prompt that fits with the plain reserve is refused when thinking is on and the margin is thin */
        let body = json!({ "messages": [{ "role": "user", "content": "word ".repeat(2_500) }], "tools": [] });
        let estimate = crate::guard::context::estimate_request(&body);
        let window = estimate + 2_100; /* 2,000 fits, 2,512 does not */

        assert!(crate::guard::context::check(&body, window, reserve(2_000, Some(false))).is_ok());
        assert!(crate::guard::context::check(&body, window, reserve(2_000, Some(true))).is_err());
    }

    #[test]
    fn apply_writes_the_think_field_only_when_it_decided() {
        let mut body = json!({ "model": "m" });

        assert_eq!(apply(&mut body, Kind::Formatting, true), Some(false));
        assert_eq!(body["think"], json!(false));

        let mut untouched = json!({ "model": "m" });

        assert_eq!(apply(&mut untouched, Kind::Plan, false), None);
        assert!(untouched.get("think").is_none());
    }
}
