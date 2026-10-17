//! **Local Runtime Guard (P0-8, switch `local.runtime_guard`)**: a prompt is never allowed to be cut silently.
//!
//! Ollama answers `200 OK` to a prompt longer than `num_ctx` and drops its *start* - the system prompt, the tools and
//! the goal - without a word, and its OpenAI-compatible `/v1` road cannot even set `num_ctx` (the agent has used the
//! native `/api/chat` with an explicit `num_ctx` since 0.16.1; `guard::context` is what makes that provable).
//! Three rules, all in code:
//!
//! 1. **Count before sending.** [`estimate_request`] over-counts on purpose: when it is wrong it is wrong in the
//!    direction of refusing, never of sending too much. The caller that gets [`ContextOverflow`] splits the work.
//! 2. **Never send an overflow.** [`check`] says no when `prompt + max output > window`, for a local model *and*
//!    for every cloud model, using the model's documented window.
//! 3. **Compare after.** [`judge`] sets what the runtime says it read (`prompt_eval_count`) against what SDC
//!    expected. Where the runtime reuses a cached prefix the count is smaller than the prompt, so a *small* count
//!    proves nothing; what proves a problem is a count at or past the window, or past our own over-estimate.

use serde_json::Value;

/// The prompt did not fit. `estimated` is an over-count, so `estimated > budget` can refuse a prompt that would
/// have fitted; that is the price of never silently truncating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextOverflow {
    pub estimated: u64,
    pub max_output: u64,
    pub window: u64,
}

impl std::fmt::Display for ContextOverflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ContextOverflow: this step is about {} tokens with {} reserved for the answer, and the model's window is {}; SDC splits the work instead of sending it",
            self.estimated, self.max_output, self.window
        )
    }
}

/// What came back, against what was expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judgement {
    /// The runtime's count is within what was expected.
    Consistent,
    /// The runtime read more than SDC counted: the estimate was too low, so truncation cannot be ruled out.
    Underestimated,
    /// The runtime's count reached the window: it filled up, and the start of the prompt may be gone.
    FilledWindow,
}

/// An upper-biased token count of one text: a token is at most three ASCII bytes, and a non-ASCII character
/// (Bengali, CJK, an emoji) is counted as two tokens. Real tokenizers do better; that is the margin.
pub fn estimate_text(text: &str) -> u64 {
    let (mut ascii, mut other) = (0u64, 0u64);

    for c in text.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }

    ascii.div_ceil(3) + other * 2
}

/// The tokens of a whole request body (OpenAI or Anthropic shaped, or Ollama's native): every message, every
/// tool schema, the system prompt, plus a per-message overhead for the chat template's own markers.
pub fn estimate_request(body: &Value) -> u64 {
    let mut total = 0u64;

    if let Some(system) = body["system"].as_str() {
        total += estimate_text(system) + 8;
    }

    for message in body["messages"].as_array().into_iter().flatten() {
        total += 8 + estimate_text(&message.to_string());
    }

    for tool in body["tools"].as_array().into_iter().flatten() {
        total += estimate_text(&tool.to_string());
    }

    total + 16
}

/// `Ok(estimated)` when the prompt and the answer's reserve fit the window; otherwise [`ContextOverflow`].
pub fn check(body: &Value, window: u64, max_output: u64) -> Result<u64, ContextOverflow> {
    let estimated = estimate_request(body);

    if estimated.saturating_add(max_output) > window {
        return Err(ContextOverflow { estimated, max_output, window });
    }

    Ok(estimated)
}

/// Sets the runtime's `prompt_eval_count` against the over-estimate and the window. `actual == 0` means the
/// runtime reported nothing, which is not evidence either way.
pub fn judge(estimated: u64, actual: u64, window: u64) -> Judgement {
    if actual == 0 {
        return Judgement::Consistent;
    }

    if actual >= window {
        return Judgement::FilledWindow;
    }

    if actual > estimated {
        return Judgement::Underestimated;
    }

    Judgement::Consistent
}

/// The window a cloud model's own catalogue row (or its family) documents, `None` when nothing does - a model
/// SDC knows nothing about is not given an invented limit, so the guard never refuses what it cannot judge.
pub fn documented_window(model: &str) -> Option<u64> {
    let api_model = crate::engines::native_api::api_model(model);

    crate::providers::models::blocked()
        .iter()
        .flat_map(|block| block.models.iter())
        .find(|row| row["id"].as_str().is_some_and(|id| id == model || id == api_model))
        .and_then(|row| row["ctx"].as_u64())
        .filter(|ctx| *ctx > 0)
        .or_else(|| crate::context::family_window(api_model))
}

/// The most the answer may take for a request: the request's own cap when it names one, else a reserve of a
/// quarter of the window, never less than 1,024.
pub fn max_output(body: &Value, window: u64) -> u64 {
    body["max_tokens"].as_u64().or_else(|| body["options"]["num_predict"].as_u64().filter(|n| *n > 0)).unwrap_or_else(|| (window / 4).max(1_024))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body_of(chars: usize) -> Value {
        json!({ "system": "be brief", "messages": [{ "role": "user", "content": "x".repeat(chars) }], "tools": [] })
    }

    #[test]
    fn the_estimate_never_undercounts_a_real_tokenizer() {
        /* English prose is about 4 bytes a token and code about 3; the estimate takes 3. Bengali is about 1 a
           character; the estimate takes 2. */
        assert!(estimate_text(&"hello world ".repeat(100)) >= 1200 / 4);
        assert!(estimate_text("বাংলা ভাষা") >= 8);
        assert!(estimate_text("") == 0);
    }

    #[test]
    fn a_twice_too_big_prompt_is_never_sent() {
        let window = 4_096;
        let big = body_of(window as usize * 6);

        match check(&big, window, 512) {
            Err(overflow) => {
                assert!(overflow.estimated > window);
                assert!(overflow.to_string().starts_with("ContextOverflow"));
            }
            Ok(_) => panic!("an overflow was allowed through"),
        }
    }

    #[test]
    fn a_prompt_that_fits_is_allowed_with_its_reserve() {
        let small = body_of(2_000);

        assert!(check(&small, 8_192, 1_024).is_ok());
        /* The same prompt with an answer reserve that does not fit is refused. */
        assert!(check(&small, 1_000, 900).is_err());
    }

    #[test]
    fn the_answers_reserve_counts_against_the_window() {
        let body = body_of(900);
        let estimated = estimate_request(&body);

        assert!(check(&body, estimated + 100, 100).is_ok());
        assert!(check(&body, estimated + 100, 101).is_err());
    }

    #[test]
    fn a_count_that_fills_the_window_or_passes_the_estimate_is_invalid() {
        assert_eq!(judge(3_000, 2_900, 4_096), Judgement::Consistent);
        assert_eq!(judge(3_000, 500, 4_096), Judgement::Consistent, "a cached prefix reads back less - proves nothing");
        assert_eq!(judge(3_000, 3_500, 4_096), Judgement::Underestimated);
        assert_eq!(judge(3_000, 4_096, 4_096), Judgement::FilledWindow);
        assert_eq!(judge(3_000, 0, 4_096), Judgement::Consistent, "no count is not evidence");
    }

    /// The silent-truncation scenario in numbers: a sentinel (the goal) at the start of a prompt twice the window
    /// is refused before it can be dropped, and a prompt just under the window keeps its sentinel in what is sent.
    #[test]
    fn a_sentinel_at_the_start_is_never_in_a_prompt_that_could_be_cut() {
        let window = 4_096u64;
        let sentinel = "GOAL: answer PINEAPPLE";
        let make = |filler: usize| json!({ "messages": [{ "role": "user", "content": format!("{sentinel}\n{}", "filler ".repeat(filler)) }] });

        assert!(check(&make(window as usize * 2), window, 256).is_err(), "2x window: refused, so the sentinel is never silently dropped");

        let near = make(900);

        assert!(check(&near, window, 256).is_ok());
        assert!(near.to_string().contains("PINEAPPLE"), "what is sent still starts with the goal");
    }

    #[test]
    fn the_reserve_is_the_requests_own_cap_or_a_quarter_window() {
        assert_eq!(max_output(&json!({ "max_tokens": 777 }), 8_000), 777);
        assert_eq!(max_output(&json!({ "options": { "num_predict": 300 } }), 8_000), 300);
        assert_eq!(max_output(&json!({}), 8_000), 2_000);
        assert_eq!(max_output(&json!({}), 1_000), 1_024);
    }
}
