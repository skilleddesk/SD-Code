//! **Draft–verify cascade** (v3 I3, switch `v3.cascade`): a cheap model drafts, a strong one takes over - but only
//! when the draft is actually stuck, not on every turn. "Stuck" is something SDC can see for itself: the same step
//! failed a second time, or a turn has collected three failures. Then the strong model (`cascade.strong` =
//! `provider:model`) is handed the same task with a capsule of what the draft did, what failed and what the error
//! said, and finishes it. Everything before that stays on the cheap model, which is where the saving is.
//!
//! The capsule is [`crate::v3::failover::capsule`]: the same provider-neutral state, for the same reason - the strong
//! model may not even be able to read the cheap one's conversation.

use crate::v3::lessons::Step;

pub const SETTING: &str = "cascade.strong";

pub fn is_setting(key: &str) -> bool {
    key == SETTING
}

fn slot() -> &'static std::sync::Mutex<Option<(String, String)>> {
    static SLOT: std::sync::OnceLock<std::sync::Mutex<Option<(String, String)>>> = std::sync::OnceLock::new();

    SLOT.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn configure(store: &crate::store::sqlite::Store) {
    let strong = store.setting(SETTING).ok().flatten().and_then(|text| parse(&text));

    if let Ok(mut slot) = slot().lock() {
        *slot = strong;
    }
}

/// The model that takes over when the draft is stuck.
pub fn strong() -> Option<(String, String)> {
    #[cfg(test)]
    if let Some(strong) = TEST_STRONG.with(|strong| strong.borrow().clone()) {
        return Some(strong);
    }

    slot().lock().ok().and_then(|slot| slot.clone())
}

#[cfg(test)]
thread_local! {
    /// A test's own strong model, so a stuck draft in another test does not reach it.
    static TEST_STRONG: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn set_strong_for_test(strong: Option<(String, String)>) {
    TEST_STRONG.with(|slot| *slot.borrow_mut() = strong);
}

/// `provider:model` - the model that takes over.
pub fn parse(text: &str) -> Option<(String, String)> {
    let (provider, model) = text.trim().split_once(':')?;

    (!provider.trim().is_empty() && !model.trim().is_empty()).then(|| (provider.trim().to_string(), model.trim().to_string()))
}

/// Is the draft stuck? Two tries at the same step, or three failures in one turn.
pub fn stuck(steps: &[Step]) -> bool {
    let failures: Vec<&Step> = steps.iter().filter(|step| !step.ok && !step.target.is_empty()).collect();

    failures.len() >= 3 || failures.iter().enumerate().any(|(index, step)| failures[..index].iter().any(|earlier| earlier.target == step.target))
}

/// What the strong model is told, on top of the capsule.
pub fn instruction(failed: &str) -> String {
    format!(
        "[SDC cascade: the small model was working on this task and got stuck - {failed}. You have the same task and the state it left behind. Do not start over: read what it changed, fix the part that failed, and finish the task.]"
    )
}

/// The task as the strong model receives it: the capsule of the draft's work, opened by the instruction.
pub fn handover(state: &crate::v3::failover::State, failed: &str) -> String {
    format!("{}\n\n{}", instruction(failed), crate::v3::failover::capsule(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(target: &str, ok: bool) -> Step {
        Step { tool: "run_command".into(), target: target.into(), ok, note: String::new() }
    }

    #[test]
    fn only_a_stuck_draft_hands_over() {
        assert!(!stuck(&[step("cargo test", false)]), "one failure is not stuck");
        assert!(!stuck(&[step("cargo test", false), step("read_file", true), step("edit_file", true)]));
        assert!(stuck(&[step("cargo test", false), step("cargo test", false)]), "the same step twice");
        assert!(stuck(&[step("a", false), step("b", false), step("c", false)]), "three failures");
        assert_eq!(parse("deepseek:deepseek-chat"), Some(("deepseek".to_string(), "deepseek-chat".to_string())));
        assert_eq!(parse("nonsense"), None);
    }

    #[test]
    fn the_strong_model_is_given_the_state_not_the_conversation() {
        let state = crate::v3::failover::State {
            goal: "make the orders tests pass",
            failed: "draft".into(),
            reason: "`cargo test` failed twice",
            plan: None,
            changed: vec!["src/orders.ts".into()],
            steps: &["run_command cargo test → error".to_string()],
            last_words: "I cannot see why it fails.",
        };
        let text = handover(&state, "`cargo test` failed twice");

        assert!(text.contains("got stuck") && text.contains("make the orders tests pass") && text.contains("src/orders.ts"));
        assert!(text.contains("Do not start over"));
    }
}
