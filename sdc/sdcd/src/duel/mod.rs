//! Duel mode: the same prompt on two engines (master spec section 16.6).
//!
//! Three decisions live here, and each one is a rule rather than a preference:
//!
//! * **two engines, one prompt.** `plan()` refuses a duel with fewer than two engines: a one-engine
//!   duel is a turn, and pretending otherwise would cost the user a wasted run.
//! * **`Keep` archives the loser.** Nothing is deleted, here or anywhere else in this daemon
//!   (principle P4: no silent loss). `keep(id, engine)` names the winner; `keep(id, None)` is
//!   `Keep neither`, which is a real answer and is why the app's pane footer has three buttons.
//! * **the panes carry their own numbers.** Time, cost and the pass/fail verdict travel per pane, so
//!   the app can show them side by side without a second query.

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;

/// One side of a duel, as `DuelStarted`'s `panes` carries it.
pub struct Pane {
    pub engine: String,
    pub model: String,
    pub time: String,
    pub cost: String,
    pub pass: bool,
    pub headline: String,
    pub files: Vec<String>,
}

impl Pane {
    pub fn to_value(&self) -> Value {
        json!({
            "engine": self.engine,
            "model": self.model,
            "time": self.time,
            "cost": self.cost,
            "pass": self.pass,
            "headline": self.headline,
            "files": self.files,
        })
    }
}

/// One engine's default model, so a duel can be started from a prompt alone.
pub fn model_for(engine: &str) -> &'static str {
    match engine {
        "codex" => "default",
        "gemini" => "gemini-2.5-pro",
        "native_api" => "anthropic/claude-sonnet-5",
        "ollama" => "llama3.2:3b",
        _ => "sonnet",
    }
}

/// Validates the engines of a duel and returns them in the order the panes render.
pub fn plan(engines: &[String]) -> Result<[String; 2], ErrorObject> {
    let unique: Vec<String> = engines
        .iter()
        .filter(|engine| !engine.trim().is_empty())
        .fold(Vec::new(), |mut acc, engine| {
            if !acc.contains(engine) {
                acc.push(engine.clone());
            }

            acc
        });

    if unique.len() < 2 {
        return Err(ErrorObject::bad_request(
            "a duel needs two different engines; one engine is just a turn",
        ));
    }

    Ok([unique[0].clone(), unique[1].clone()])
}

/// The two panes a duel starts with, before either engine has answered.
pub fn pending_panes(engines: &[String; 2]) -> Vec<Value> {
    engines
        .iter()
        .map(|engine| {
            Pane {
                engine: engine.clone(),
                model: model_for(engine).to_string(),
                time: "…".to_string(),
                cost: "…".to_string(),
                pass: false,
                headline: "running".to_string(),
                files: Vec::new(),
            }
            .to_value()
        })
        .collect()
}

/// Which engine was kept, or `None` for `Keep neither`. The pair is returned so the caller can
/// archive the other run and say which one won.
pub fn resolve(keep: Option<&str>, engines: &[String; 2]) -> (Option<String>, Vec<String>) {
    match keep {
        Some(winner) if engines.contains(&winner.to_string()) => {
            let archived = engines.iter().filter(|engine| *engine != winner).cloned().collect();

            (Some(winner.to_string()), archived)
        }
        /* `Keep neither`, or a name that is not in this duel: both runs are archived, and the caller
           still gets a `DuelResolved` event so the tab cannot be left mid-duel. */
        _ => (None, engines.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duel_needs_two_different_engines() {
        assert!(plan(&["claude_code".to_string()]).is_err());
        assert!(plan(&["claude_code".to_string(), "claude_code".to_string()]).is_err());

        let engines = plan(&["claude_code".to_string(), "codex".to_string(), "codex".to_string()]).unwrap();

        assert_eq!(engines, ["claude_code".to_string(), "codex".to_string()]);
    }

    #[test]
    fn keeping_a_winner_archives_the_other() {
        let engines = ["claude_code".to_string(), "codex".to_string()];
        let (kept, archived) = resolve(Some("codex"), &engines);

        assert_eq!(kept.as_deref(), Some("codex"));
        assert_eq!(archived, vec!["claude_code"]);
    }

    #[test]
    fn keep_neither_archives_both_and_still_resolves() {
        let engines = ["claude_code".to_string(), "codex".to_string()];
        let (kept, archived) = resolve(None, &engines);

        assert!(kept.is_none());
        assert_eq!(archived.len(), 2);
    }

    #[test]
    fn the_pending_panes_name_each_engines_own_model() {
        let panes = pending_panes(&["claude_code".to_string(), "ollama".to_string()]);

        assert_eq!(panes[0]["model"], json!("sonnet"));
        assert_eq!(panes[1]["model"], json!("llama3.2:3b"));
        assert_eq!(panes[1]["engine"], json!("ollama"));
    }
}
