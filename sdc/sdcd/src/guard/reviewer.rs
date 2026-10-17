//! **Builder model and Reviewer model (Phase 0.10).**
//!
//! The Manual Pin contract says the model the person chose is the model that runs. Verify's AI review used to break it
//! quietly: the app picked "a different engine than the one that wrote the change" by itself, so a person who pinned
//! DeepSeek could get a review from Claude or GPT they never chose, on their key, with their diff.
//!
//! Now there are two explicit choices, and nothing else:
//!
//! * the **Builder** - the model of the chat (the pin), and
//! * the **Reviewer** - a second model the person names in Settings (`models.reviewer`, JSON `{engine, provider, model}`).
//!
//! No reviewer chosen means **review off** - and the Verify tab says so, instead of a model the person did not pick
//! reading their code. The checks and scans (which use no model) always run. The request's own `reviewer` is honoured
//! only when it is the chosen one; under `models.contract` off, every earlier version's behaviour is kept.

use serde_json::Value;

use crate::guard::modelgate::{Gate, Purpose};
use crate::verify::Reviewer;

/// The setting that holds the person's reviewer.
pub const SETTING: &str = "models.reviewer";

/// What Verify does about its AI review, and why - shown as it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    pub reviewer: Option<Reviewer>,
    /// `chosen` (the person's reviewer), `off` (none chosen), `unchecked` (contract off: whatever the app sent).
    pub mode: &'static str,
    pub note: String,
}

impl PartialEq for Reviewer {
    fn eq(&self, other: &Self) -> bool {
        self.engine == other.engine && self.model == other.model && self.provider == other.provider
    }
}

/// The reviewer the person chose, from the setting's text; `None` for an empty or unreadable value (review off).
pub fn parse(setting: &str) -> Option<Reviewer> {
    let value: Value = serde_json::from_str(setting.trim()).ok()?;
    let engine = value["engine"].as_str().filter(|engine| !engine.is_empty())?;

    Some(Reviewer {
        engine: engine.to_string(),
        model: value["model"].as_str().unwrap_or_default().to_string(),
        provider: value["provider"].as_str().filter(|id| !id.is_empty()).map(str::to_string),
    })
}

/// Decides the review for a Verify request: `requested` is what the app sent, `chosen` is the setting.
pub fn resolve(requested: Option<Reviewer>, chosen: Option<Reviewer>, enforcing: bool) -> Resolution {
    if !enforcing {
        let mode = if requested.is_some() { "unchecked" } else { "off" };

        return Resolution { reviewer: requested, mode, note: "The model contract is switched off; the review runs as the app asked.".into() };
    }

    let Some(chosen) = chosen else {
        return Resolution {
            reviewer: None,
            mode: "off",
            note: "AI review is off: you have not chosen a Reviewer model. Checks and scans still ran. Choose one in the Verify tab to turn the review on.".into(),
        };
    };

    /* The gate decides, not a comparison written here: the chosen reviewer is the one exception the person wrote down. */
    let gate = Gate::contract(None, "", false, false).with_exception(Purpose::Reviewer, chosen.provider.as_deref(), &chosen.model);

    match requested {
        Some(asked) if gate.permit(Purpose::Reviewer, asked.provider.as_deref(), &asked.model).is_err() => Resolution {
            reviewer: Some(chosen),
            mode: "chosen",
            note: format!("The app asked for {}, which is not the Reviewer you chose; your chosen Reviewer was used.", asked.model),
        },
        _ => Resolution { reviewer: Some(chosen), mode: "chosen", note: "Reviewed by the Reviewer model you chose.".into() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reviewer(engine: &str, provider: Option<&str>, model: &str) -> Reviewer {
        Reviewer { engine: engine.into(), model: model.into(), provider: provider.map(str::to_string) }
    }

    #[test]
    fn a_setting_is_read_and_a_blank_or_broken_one_means_no_reviewer() {
        assert_eq!(parse(r#"{"engine":"api","provider":"openai-api","model":"gpt-5"}"#), Some(reviewer("api", Some("openai-api"), "gpt-5")));
        assert_eq!(parse(""), None);
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"engine":"","model":"x"}"#), None);
    }

    #[test]
    fn with_no_reviewer_chosen_the_review_is_off_whatever_the_app_sends() {
        let sent = reviewer("claude_code", None, "sonnet");
        let outcome = resolve(Some(sent), None, true);

        assert_eq!(outcome.reviewer, None, "the app's pick is not a choice");
        assert_eq!(outcome.mode, "off");
        assert!(outcome.note.contains("off"));
    }

    #[test]
    fn the_chosen_reviewer_is_used_and_an_unchosen_one_is_never_swapped_in() {
        let chosen = reviewer("api", Some("openai-api"), "gpt-5");
        let same = resolve(Some(chosen.clone()), Some(chosen.clone()), true);

        assert_eq!(same.reviewer, Some(chosen.clone()));
        assert_eq!(same.mode, "chosen");

        let other = resolve(Some(reviewer("claude_code", None, "sonnet")), Some(chosen.clone()), true);

        assert_eq!(other.reviewer, Some(chosen), "the person's choice wins over the app's");
        assert!(other.note.contains("not the Reviewer you chose"));
    }

    #[test]
    fn contract_off_is_the_old_behaviour() {
        let sent = reviewer("claude_code", None, "sonnet");
        let outcome = resolve(Some(sent.clone()), None, false);

        assert_eq!(outcome.reviewer, Some(sent));
        assert_eq!(outcome.mode, "unchecked");
    }
}
