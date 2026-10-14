//! **Weak-model lifting** (v3 Q5, switch `v3.effort`): a small model asked to be a careful engineer fails at the
//! things its size cannot do - keeping the shape of a long file, holding a plan, noticing that its check failed.
//! SDC lifts it instead of replacing it: the model is given room to think (its own `reasoning_effort`, or the
//! provider's thinking mode) and one short instruction to lean on the tools - verify each change, and say plainly
//! what is still unsure instead of guessing.
//!
//! Only a weak model is lifted: a strong one would spend money and time on thinking it does not need. What counts
//! as weak is said plainly - a small name (`mini`, `flash`, `7b`), or a price low enough that it is one.

/// Names that give away a small model.
const SMALL: &[&str] = &["mini", "nano", "tiny", "small", "lite", "light", "flash", "haiku", "instant", "fast", "3b", "4b", "7b", "8b", "9b", "13b", "14b", "1.5b", "2b", "phi", "gemma", "qwen2.5-coder:7b"];

/// A model costing less than this per million output tokens is treated as a small one.
const CHEAP: f64 = 1.0;

/// Is this model small enough to be worth lifting?
pub fn is_weak(model: &str, price: Option<(f64, f64)>) -> bool {
    let bare = model.rsplit('/').next().unwrap_or(model).to_ascii_lowercase();

    if SMALL.iter().any(|name| bare.contains(name)) {
        return true;
    }

    price.is_some_and(|(_, output)| output > 0.0 && output < CHEAP)
}

/// The thinking the model is given: `(thinking on, effort)`.
pub fn thinking_for(model: &str) -> (bool, Option<String>) {
    let bare = model.rsplit('/').next().unwrap_or(model).to_ascii_lowercase();

    /* A model that takes an effort word gets `high`; one with a thinking mode gets it switched on. */
    if bare.starts_with("deepseek-v4") || bare.contains("gpt-5") || bare.contains("o4") || bare.contains("grok-4") || bare.contains("qwen3") {
        return (true, Some("high".to_string()));
    }

    (true, None)
}

/// What a lifted model is told, once per turn.
pub fn instruction() -> &'static str {
    "[SDC: this task is on a small model, and SDC has lifted it for you. Work one step at a time and use the tools for every fact: read the real lines before changing them, run the check after changing them, and if something is still unsure, say so plainly rather than filling the gap with a guess.]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_models_are_lifted_and_strong_ones_are_left_alone() {
        assert!(is_weak("gpt-5-mini", Some((0.25, 2.0))));
        assert!(is_weak("qwen/qwen3-coder-7b", None));
        assert!(is_weak("some-model", Some((0.1, 0.5))), "cheap enough to be small");
        assert!(!is_weak("claude-sonnet-5-5", Some((3.0, 15.0))));
        assert!(!is_weak("deepseek-chat", Some((0.27, 1.1))));

        assert_eq!(thinking_for("deepseek-v4-pro"), (true, Some("high".to_string())));
        assert_eq!(thinking_for("qwen3-coder-plus"), (true, Some("high".to_string())));
        assert_eq!(thinking_for("llama3:8b"), (true, None), "a model with only a thinking mode");
        assert!(instruction().contains("small model") && instruction().contains("run the check"));
    }
}
