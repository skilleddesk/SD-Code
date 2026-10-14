//! **The router** (v3 W6, switch `v3.router`): SDC runs every turn on the model the person chose, whether the turn
//! is "what does this error mean?" or a fifty-file refactor. The router sends a plainly light request to the fast
//! model the person named (`router.fast` = `provider:model`) and leaves everything else on the main model, and it
//! estimates the turn's cost from the tokens the task looks like it needs, so the person is not surprised at the end.
//!
//! The decision is made from the person's own words, before anything is sent: no model is asked, nothing is spent
//! deciding. A request that names files to change, or asks for work rather than an answer, stays on the main model -
//! and the reader of the answer is told which model answered.

use std::sync::{Mutex, OnceLock};

/// The words that mean work rather than a question: those turns stay on the main model whatever else they say.
const HEAVY: &[&str] = &[
    "refactor", "implement", "fix", "rewrite", "migrate", "add", "build", "create", "write", "change", "update", "remove", "delete", "rename", "test", "debug", "optimi", "review",
    "audit", "design", "architect", "deploy", "publish", "release", "koro", "banao", "thik", "sompurno", "complete",
];

/// The words that mean a question.
const LIGHT: &[&str] = &["what", "why", "which", "where", "explain", "summar", "meaning", "difference", "how does", "tell me", "kothay", "keno", "kemon", "ki ", "bolo"];

pub const SETTING: &str = "router.fast";

fn slot() -> &'static Mutex<Option<(String, String)>> {
    static SLOT: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();

    SLOT.get_or_init(|| Mutex::new(None))
}

pub fn is_setting(key: &str) -> bool {
    key == SETTING
}

/// `deepseek:deepseek-chat` → a pair; anything else means the router is off.
pub fn parse(text: &str) -> Option<(String, String)> {
    let (provider, model) = text.trim().split_once(':')?;

    (!provider.trim().is_empty() && !model.trim().is_empty()).then(|| (provider.trim().to_string(), model.trim().to_string()))
}

pub fn configure(store: &crate::store::sqlite::Store) {
    let fast = store.setting(SETTING).ok().flatten().and_then(|text| parse(&text));

    if let Ok(mut slot) = slot().lock() {
        *slot = fast;
    }
}

/// The fast model in force.
pub fn fast() -> Option<(String, String)> {
    #[cfg(test)]
    if let Some(fast) = TEST_FAST.with(|fast| fast.borrow().clone()) {
        return Some(fast);
    }

    slot().lock().ok().and_then(|slot| slot.clone())
}

#[cfg(test)]
thread_local! {
    /// A test's own fast model, seen only by its thread - other tests' light requests stay on their own model.
    static TEST_FAST: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn set_fast_for_test(fast: Option<(String, String)>) {
    TEST_FAST.with(|slot| *slot.borrow_mut() = fast);
}

/// Is the person's request light enough to hand to the fast model? A question with no file named and no work asked
/// for: short, no path, none of the words that mean work.
pub fn is_light(request: &str) -> bool {
    let lower = request.to_lowercase();

    if request.split_whitespace().count() > 40 || lower.contains('/') || lower.contains(".rs") || lower.contains(".ts") || lower.contains(".py") || lower.contains("\\") {
        return false;
    }

    if HEAVY.iter().any(|word| lower.contains(word)) {
        return false;
    }

    LIGHT.iter().any(|word| lower.contains(word))
}

/// The turn's model: the fast one for a light request when the person named one, the main model otherwise.
pub fn route<T>(main: T, request: &str, resolve: &dyn Fn(&str, &str) -> Option<T>) -> T {
    let Some((provider, model)) = fast() else {
        return main;
    };

    if !is_light(request) {
        return main;
    }

    match resolve(&provider, &model) {
        Some(fast) => fast,
        None => main,
    }
}

/// What a turn is likely to cost, in dollars: the price of the model (per million in and out) times the tokens a
/// task of this kind usually takes (a question: one step; work: a dozen).
pub fn estimate(request: &str, price: Option<(f64, f64)>, context_tokens: u64) -> Option<f64> {
    let (input_price, output_price) = price?;
    let steps = if is_light(request) { 2.0 } else { 12.0 };
    let input = (context_tokens.max(2_000) as f64 * steps * 0.7).min(400_000.0);
    let output = 400.0 * steps;

    Some((input * input_price + output * output_price) / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_goes_to_the_fast_model_and_work_does_not() {
        assert!(is_light("what does this error mean?"));
        assert!(is_light("kemon kore kaj kore ei function ta? explain"));
        assert!(!is_light("fix the orders list in src/orders.ts"), "work stays on the main model");
        assert!(!is_light("what is wrong with src/orders.ts?"), "a named file means reading code");
        assert!(!is_light("explain and then refactor the pricing module"));

        set_fast_for_test(Some(("fast-provider".into(), "fast-model".into())));

        let resolve = |provider: &str, model: &str| Some(format!("{provider}:{model}"));
        let picked = route("main".to_string(), "explain the difference between these two", &resolve);

        assert_eq!(picked, "fast-provider:fast-model");
        assert_eq!(route("main".to_string(), "fix the build", &resolve), "main");
        assert_eq!(route("main".to_string(), "explain this", &|_: &str, _: &str| None), "main", "an unresolvable fast model falls back");

        set_fast_for_test(None);
        assert_eq!(route("main".to_string(), "explain this", &resolve), "main", "no fast model named");
    }

    #[test]
    fn the_estimate_grows_with_the_work_and_the_price() {
        let question = estimate("what does this mean?", Some((1.0, 4.0)), 10_000).unwrap();
        let work = estimate("refactor the orders module", Some((1.0, 4.0)), 10_000).unwrap();

        assert!(question > 0.0 && work > question * 3.0, "{question} vs {work}");
        assert!(estimate("what does this mean?", None, 10_000).is_none(), "no price, no estimate");
    }
}
