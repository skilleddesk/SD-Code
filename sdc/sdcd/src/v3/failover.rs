//! **State capsule failover** (v3 I9): when a provider fails for good mid-task (its retries spent, a refused key,
//! an outage), the turn continues on the next model of `v3.failoverOrder` (`provider:model, provider:model`),
//! handed a provider-neutral capsule of the work instead of the old provider's conversation - which another
//! provider could not read anyway (different message shapes, thinking blocks).
//!
//! The capsule is built from what SDC itself knows, without a model call: the goal, the plan's state, the files
//! changed so far (a checkpoint was taken before the first), the last steps and their outcome, and what the model
//! last said. It stays under ~1,500 tokens.

use std::sync::{Mutex, OnceLock};

/// The models to continue on, in order (`v3.failoverOrder`).
fn order_slot() -> &'static Mutex<Vec<(String, String)>> {
    static ORDER: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();

    ORDER.get_or_init(|| Mutex::new(Vec::new()))
}

pub const SETTING: &str = "v3.failoverOrder";

/// `deepseek:deepseek-chat, anthropic-api:claude-sonnet-5-5` → pairs.
pub fn parse_order(text: &str) -> Vec<(String, String)> {
    text.split(',').filter_map(|item| item.trim().split_once(':')).map(|(provider, model)| (provider.trim().to_string(), model.trim().to_string())).filter(|(provider, model)| !provider.is_empty() && !model.is_empty()).collect()
}

pub fn configure(store: &crate::store::sqlite::Store) {
    let order = parse_order(&store.setting(SETTING).ok().flatten().unwrap_or_default());

    if let Ok(mut slot) = order_slot().lock() {
        *slot = order;
    }
}

#[cfg(test)]
thread_local! {
    /// A test's own failover order, seen only by its thread (other tests that expect a failure are unaffected).
    static TEST_ORDER: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Sets this thread's order (tests).
#[cfg(test)]
pub fn set_order_for_test(order: Vec<(String, String)>) {
    TEST_ORDER.with(|slot| *slot.borrow_mut() = order);
}

/// The next model after the ones already tried this turn.
pub fn next(tried: &[(String, String)]) -> Option<(String, String)> {
    #[cfg(test)]
    let order: Vec<(String, String)> = TEST_ORDER.with(|slot| slot.borrow().clone());
    #[cfg(not(test))]
    let order: Vec<(String, String)> = order_slot().lock().ok()?.clone();

    order.into_iter().find(|candidate| !tried.contains(candidate))
}

/// Per turn, the models it has run on.
type Tried = std::collections::HashMap<String, Vec<(String, String)>>;

fn tried_map() -> &'static Mutex<Tried> {
    static TRIED: OnceLock<Mutex<Tried>> = OnceLock::new();

    TRIED.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// The models a turn has already run on (the failed ones and the one running).
pub fn tried(turn_id: &str) -> Vec<(String, String)> {
    tried_map().lock().map(|map| map.get(turn_id).cloned().unwrap_or_default()).unwrap_or_default()
}

pub fn set_tried(turn_id: &str, tried: &[(String, String)]) {
    if let Ok(mut map) = tried_map().lock() {
        map.insert(turn_id.to_string(), tried.to_vec());
    }
}

pub fn forget(turn_id: &str) {
    if let Ok(mut map) = tried_map().lock() {
        map.remove(turn_id);
    }
}

/// What the capsule is built from.
#[derive(Debug, Default)]
pub struct State<'a> {
    pub goal: &'a str,
    pub failed: String,
    pub reason: &'a str,
    pub plan: Option<String>,
    pub changed: Vec<String>,
    pub steps: &'a [String],
    pub last_words: &'a str,
}

fn clip(text: &str, chars: usize) -> String {
    let text = text.trim();

    if text.chars().count() > chars {
        format!("{}…", text.chars().take(chars).collect::<String>())
    } else {
        text.to_string()
    }
}

/// The capsule: plain text any model reads, at most ~6,000 characters (~1,500 tokens).
pub fn capsule(state: &State) -> String {
    let mut out = format!(
        "[SDC failover: the model working on this task ({}) failed ({}). You continue the same task from the state below - do not start over, and check the files before changing them again.]\n\nGoal (the person's words):\n{}\n",
        state.failed,
        clip(state.reason, 200),
        clip(state.goal, 2_000)
    );

    if let Some(plan) = &state.plan {
        out.push_str(&format!("\nPlan steps still open:\n{}\n", clip(plan, 800)));
    }

    if !state.changed.is_empty() {
        let mut changed = state.changed.clone();

        changed.sort();
        out.push_str(&format!("\nFiles changed so far (a checkpoint holds the state before the first change): {}\n", clip(&changed.join(", "), 800)));
    }

    if !state.steps.is_empty() {
        let recent: Vec<&String> = state.steps.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();

        out.push_str(&format!("\nLast steps ({} in all):\n{}\n", state.steps.len(), clip(&recent.iter().map(|step| format!("- {step}")).collect::<Vec<_>>().join("\n"), 1_400)));
    }

    if !state.last_words.trim().is_empty() {
        out.push_str(&format!("\nWhat the previous model last said:\n{}\n", clip(state.last_words, 600)));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_is_read_and_tried_models_are_skipped() {
        let order = parse_order(" deepseek:deepseek-chat , anthropic-api:claude-sonnet-5-5,broken, :x ");

        assert_eq!(order, [("deepseek".to_string(), "deepseek-chat".to_string()), ("anthropic-api".to_string(), "claude-sonnet-5-5".to_string())]);
    }

    #[test]
    fn the_capsule_carries_the_state_and_stays_small() {
        let steps: Vec<String> = (0..40).map(|n| format!("edit_file src/file_{n}.rs → ok")).collect();
        let text = capsule(&State {
            goal: &"Add pagination to the orders list. ".repeat(200),
            failed: "deepseek:deepseek-chat".into(),
            reason: "Service Unavailable (503) · asked 9 times",
            plan: Some("- write the tests\n- update the docs".into()),
            changed: vec!["src/orders.ts".into(), "src/api/routes.ts".into()],
            steps: &steps,
            last_words: "I have added the page parameter; next the tests.",
        });

        assert!(text.contains("deepseek:deepseek-chat") && text.contains("503"));
        assert!(text.contains("src/api/routes.ts, src/orders.ts"), "files, sorted");
        assert!(text.contains("- write the tests") && text.contains("edit_file src/file_39.rs") && !text.contains("file_27.rs"), "the newest twelve steps");
        assert!(text.contains("next the tests"));
        assert!(text.chars().count() <= 6_000, "{} chars", text.chars().count());
    }
}
