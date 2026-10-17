//! **The model-selection contract and the Pin Guard (P0-10, switch `models.contract`)**: the model the person chose
//! is the model that runs - and a call to any other model is a bug that fails a test.
//!
//! | what the person chose | mode | what may run |
//! |---|---|---|
//! | a cloud, API or CLI model | [`Mode::Pinned`] | only that model, for every call: the loop, sub-agents, review |
//! | a local model, and a Rotation Chain with an enabled row | [`Mode::LocalRotation`] | the local model, and chain rows - only as [`Purpose::Chain`] |
//! | a local model, an empty or disabled chain, or `privacy = "local-only"` | [`Mode::LocalOnly`] | only that local model |
//!
//! Every model call asks [`Gate::permit`] first. The automatic model-changers - the router (W6), the cascade (I3),
//! the failover (I9), the ladder, the planner-executor and the bandit - ask [`Gate::allows`]; under a pin the answer
//! is no, so they are not "off by a setting that could be set", they are unreachable. The only exceptions are the
//! ones the person writes down themselves (`/research`'s final-answer model, the Duel reviewer): they are named in
//! [`Gate::exceptions`] and a receipt lists them separately. With the switch off the gate permits everything, which
//! is the behaviour of every earlier version.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Pinned,
    LocalRotation,
    LocalOnly,
}

impl Mode {
    /// The words the badge shows (the UI translates them).
    pub fn name(self) -> &'static str {
        match self {
            Mode::Pinned => "pinned",
            Mode::LocalRotation => "local-rotation",
            Mode::LocalOnly => "local-only",
        }
    }
}

/// Why a call wants a model. A purpose is how the gate tells "the model the person chose" from "a model SDC wants".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    /// The turn's own model: the loop, a sub-agent exploring on it, a repair on it.
    Main,
    /// A sub-agent that explores with a provider's cheaper model.
    SubAgentFast,
    Router,
    Cascade,
    Failover,
    Ladder,
    PlannerExecutor,
    Bandit,
    /// A row of the Rotation Chain (Local Rotation only; P2).
    Chain,
    /// `/research`'s final-answer model - only when the person set one.
    ResearchFinal,
    /// The Duel reviewer - only when the person picked one.
    DuelReviewer,
    /// Verify's AI review - only the Reviewer model the person chose (`models.reviewer`, Phase 0.10).
    Reviewer,
}

impl Purpose {
    /// The purposes a person can switch on in Settings and so may name a different model for.
    pub fn is_user_exception(self) -> bool {
        matches!(self, Purpose::ResearchFinal | Purpose::DuelReviewer | Purpose::Reviewer)
    }
}

/// What a blocked call says. Carried as text through the agent, and the tests look for the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinViolationError {
    pub mode: &'static str,
    pub purpose: Purpose,
    pub wanted: String,
    pub pinned: String,
}

impl std::fmt::Display for PinViolationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PinViolationError: a {:?} call wanted {} but this turn is {} to {}", self.purpose, self.wanted, self.mode, self.pinned)
    }
}

/// A model as the gate compares it: the provider and the id the API is called with, case-folded.
fn same(a: &(String, String), b: &(String, String)) -> bool {
    a.1 == b.1 && (a.0 == b.0 || a.0.is_empty() || b.0.is_empty())
}

fn normal(provider: Option<&str>, model: &str) -> (String, String) {
    (provider.unwrap_or_default().to_ascii_lowercase(), crate::engines::native_api::api_model(model).to_ascii_lowercase())
}

#[derive(Debug, Clone)]
pub struct Gate {
    pub mode: Mode,
    pinned: (String, String),
    /// The exceptions the person wrote down, with the model each one names.
    pub exceptions: Vec<(Purpose, (String, String))>,
    /// The Rotation Chain's models in the person's order (`None`: not told - every `Purpose::Chain` call is let through, as before the
    /// chain was passed to the gate). `Some(list)` is enforced: a chain call to a model that is not in the list is refused.
    chain: Option<Vec<(String, String)>>,
    /// False when `models.contract` is off: then everything is permitted, as in every earlier version.
    enforcing: bool,
}

/// Decides the mode: a local provider is Local Rotation with an enabled chain row and Local-only without one, or when
/// the project says so; anything else is a Manual Pin.
pub fn mode_for(provider: Option<&str>, chain_has_rows: bool, project_local_only: bool) -> Mode {
    if provider == Some("ollama") {
        if chain_has_rows && !project_local_only {
            Mode::LocalRotation
        } else {
            Mode::LocalOnly
        }
    } else {
        Mode::Pinned
    }
}

impl Gate {
    pub fn new(provider: Option<&str>, model: &str, chain_has_rows: bool, project_local_only: bool) -> Self {
        Self { mode: mode_for(provider, chain_has_rows, project_local_only), pinned: normal(provider, model), exceptions: Vec::new(), chain: None, enforcing: crate::v3::on("models.contract") }
    }

    /// A gate that enforces whatever `models.contract` says, for a call made outside a turn (Verify's review): nothing is
    /// pinned, so only a named exception gets through.
    pub fn contract(provider: Option<&str>, model: &str, chain_has_rows: bool, project_local_only: bool) -> Self {
        Self::new(provider, model, chain_has_rows, project_local_only)
    }

    /// A gate that permits everything (no turn registered, or the switch off).
    pub fn open() -> Self {
        Self { mode: Mode::Pinned, pinned: (String::new(), String::new()), exceptions: Vec::new(), chain: None, enforcing: false }
    }

    pub fn with_exception(mut self, purpose: Purpose, provider: Option<&str>, model: &str) -> Self {
        if purpose.is_user_exception() {
            self.exceptions.push((purpose, normal(provider, model)));
        }

        self
    }

    /// Tells the gate which models the Rotation Chain names, in order (enabled rows only). From then on a `Purpose::Chain` call is
    /// permitted only to one of them - and the chain has an order: [`Gate::chain_model`].
    pub fn with_chain(mut self, rows: &[(Option<&str>, &str)]) -> Self {
        self.chain = Some(rows.iter().map(|(provider, model)| normal(*provider, model)).collect());

        self
    }

    /// The model at position `index` of the chain (0 is the first the person listed).
    pub fn chain_model(&self, index: usize) -> Option<(String, String)> {
        self.chain.as_ref()?.get(index).cloned()
    }

    pub fn chain_len(&self) -> usize {
        self.chain.as_ref().map_or(0, Vec::len)
    }

    pub fn pinned_model(&self) -> &str {
        &self.pinned.1
    }

    /// May an automatic model-changer run at all? Under a pin, never; in Local Rotation only the chain.
    pub fn allows(&self, purpose: Purpose) -> bool {
        if !self.enforcing {
            return true;
        }

        match purpose {
            Purpose::Main | Purpose::ResearchFinal | Purpose::DuelReviewer | Purpose::Reviewer => true,
            Purpose::Chain => self.mode == Mode::LocalRotation,
            Purpose::SubAgentFast | Purpose::Router | Purpose::Cascade | Purpose::Failover | Purpose::Ladder | Purpose::PlannerExecutor | Purpose::Bandit => false,
        }
    }

    /// May this call go to this model?
    pub fn permit(&self, purpose: Purpose, provider: Option<&str>, model: &str) -> Result<(), PinViolationError> {
        if !self.enforcing {
            return Ok(());
        }

        let wanted = normal(provider, model);
        let fine = same(&wanted, &self.pinned)
            || (purpose == Purpose::Chain && self.mode == Mode::LocalRotation && self.chain.as_ref().is_none_or(|list| list.iter().any(|row| same(row, &wanted))))
            || self.exceptions.iter().any(|(allowed, named)| *allowed == purpose && same(named, &wanted));

        if fine {
            Ok(())
        } else {
            Err(PinViolationError { mode: self.mode.name(), purpose, wanted: format!("{}:{}", wanted.0, wanted.1), pinned: format!("{}:{}", self.pinned.0, self.pinned.1) })
        }
    }
}

fn registry() -> &'static Mutex<HashMap<String, Gate>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Gate>>> = OnceLock::new();

    REGISTRY.get_or_init(Default::default)
}

/// Holds a turn's gate for as long as the turn runs. Empty (it releases nothing) when the turn already had one.
pub struct Registered(Option<String>);

impl Drop for Registered {
    fn drop(&mut self) {
        if let (Some(id), Ok(mut map)) = (self.0.take(), registry().lock()) {
            map.remove(&id);
        }
    }
}

/// Registers the gate of a turn; every call made for the turn finds it with [`of`]. A turn that already has a gate
/// keeps it: a hand-over inside the turn (a capsule to another model) must not be able to *replace* the pin with the
/// model it hands over to.
pub fn register(turn_id: &str, gate: Gate) -> Registered {
    match registry().lock() {
        Ok(mut map) if !map.contains_key(turn_id) => {
            map.insert(turn_id.to_string(), gate);

            Registered(Some(turn_id.to_string()))
        }
        _ => Registered(None),
    }
}

/// Names an exception the person wrote down on a turn's gate (a no-op for a turn without one, and for a purpose a
/// person cannot add an exception for).
pub fn allow_exception(turn_id: &str, purpose: Purpose, provider: Option<&str>, model: &str) {
    if let Ok(mut map) = registry().lock() {
        if let Some(gate) = map.get_mut(turn_id) {
            *gate = gate.clone().with_exception(purpose, provider, model);
        }
    }
}

/// The turn's gate, or an open one when the turn registered none (a call made outside a turn, a test).
pub fn of(turn_id: &str) -> Gate {
    registry().lock().ok().and_then(|map| map.get(turn_id).cloned()).unwrap_or_else(Gate::open)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(provider: &str, model: &str, chain: bool, project_local: bool) -> Gate {
        Gate { enforcing: true, ..Gate::new(Some(provider), model, chain, project_local) }
    }

    #[test]
    fn the_mode_follows_what_the_person_chose() {
        assert_eq!(mode_for(Some("anthropic-api"), true, false), Mode::Pinned, "a cloud model is pinned even with a chain on file");
        assert_eq!(mode_for(Some("deepseek"), false, false), Mode::Pinned);
        assert_eq!(mode_for(Some("ollama"), true, false), Mode::LocalRotation);
        assert_eq!(mode_for(Some("ollama"), false, false), Mode::LocalOnly, "no chain row, no rotation");
        assert_eq!(mode_for(Some("ollama"), true, true), Mode::LocalOnly, "the project says local-only");
        assert_eq!(mode_for(None, true, false), Mode::Pinned);
    }

    #[test]
    fn a_pinned_turn_permits_its_own_model_and_nothing_else() {
        let pinned = gate("deepseek", "deepseek-chat", true, false);

        assert!(pinned.permit(Purpose::Main, Some("deepseek"), "deepseek-chat").is_ok());
        assert!(pinned.permit(Purpose::Main, Some("deepseek"), "deepseek/deepseek-chat").is_ok(), "the registry's spelling is the same model");

        for (purpose, provider, model) in [
            (Purpose::Main, "deepseek", "deepseek-v4-pro"),
            (Purpose::SubAgentFast, "deepseek", "deepseek-flash"),
            (Purpose::Router, "openai-api", "gpt-5-nano"),
            (Purpose::Failover, "anthropic-api", "claude-sonnet-5-5"),
            (Purpose::Chain, "anthropic-api", "claude-sonnet-5-5"),
        ] {
            let error = pinned.permit(purpose, Some(provider), model).expect_err("a foreign call must be refused");

            assert!(error.to_string().starts_with("PinViolationError"), "{error}");
            assert_eq!(error.mode, "pinned");
        }
    }

    #[test]
    fn under_a_pin_every_automatic_model_changer_is_unreachable() {
        let pinned = gate("anthropic-api", "claude-sonnet-5-5", true, false);

        for purpose in [Purpose::SubAgentFast, Purpose::Router, Purpose::Cascade, Purpose::Failover, Purpose::Ladder, Purpose::PlannerExecutor, Purpose::Bandit, Purpose::Chain] {
            assert!(!pinned.allows(purpose), "{purpose:?} must be off under a pin");
        }

        assert!(pinned.allows(Purpose::Main));
    }

    #[test]
    fn local_only_never_leaves_the_machine_and_rotation_may_use_the_chain() {
        let only = gate("ollama", "qwen3.5:9b", false, false);

        assert!(only.permit(Purpose::Main, Some("ollama"), "qwen3.5:9b").is_ok());
        assert!(only.permit(Purpose::Chain, Some("deepseek"), "deepseek-chat").is_err());
        assert!(!only.allows(Purpose::Chain));

        let rotation = gate("ollama", "qwen3.5:9b", true, false);

        assert!(rotation.allows(Purpose::Chain));
        assert!(rotation.permit(Purpose::Chain, Some("deepseek"), "deepseek-chat").is_ok(), "a chain row may be called, as a chain row");
        assert!(rotation.permit(Purpose::Router, Some("deepseek"), "deepseek-chat").is_err(), "but not as a router's choice");
        assert!(rotation.permit(Purpose::Main, Some("deepseek"), "deepseek-chat").is_err());
    }

    #[test]
    fn only_the_exceptions_the_person_wrote_down_get_through_and_only_for_their_model() {
        let pinned = gate("deepseek", "deepseek-chat", false, false).with_exception(Purpose::ResearchFinal, Some("anthropic-api"), "claude-opus-5-5").with_exception(Purpose::Router, Some("openai-api"), "gpt-5");

        assert!(pinned.permit(Purpose::ResearchFinal, Some("anthropic-api"), "claude-opus-5-5").is_ok());
        assert!(pinned.permit(Purpose::ResearchFinal, Some("anthropic-api"), "claude-sonnet-5-5").is_err(), "the exception names one model");
        assert!(pinned.permit(Purpose::Main, Some("anthropic-api"), "claude-opus-5-5").is_err(), "and one purpose");
        assert_eq!(pinned.exceptions.len(), 1, "a router is not a thing a person can add as an exception");
    }

    #[test]
    fn switch_off_is_the_old_behaviour() {
        let open = crate::v3::ablating(&["models.contract"], || Gate::new(Some("deepseek"), "deepseek-chat", false, false));

        assert!(open.permit(Purpose::Router, Some("openai-api"), "gpt-5").is_ok());
        assert!(open.allows(Purpose::Router) && open.allows(Purpose::Failover) && open.allows(Purpose::Bandit));
        assert!(Gate::open().permit(Purpose::Cascade, None, "anything").is_ok());
    }

    #[test]
    fn a_turns_gate_is_found_by_its_id_and_released_after() {
        let held = register("turn-gate-test", gate("deepseek", "deepseek-chat", false, false));

        assert!(of("turn-gate-test").permit(Purpose::Main, Some("openai-api"), "gpt-5").is_err());
        assert!(of("turn-not-registered").permit(Purpose::Main, Some("openai-api"), "gpt-5").is_ok(), "no turn, no gate: an open one");

        /* A second registration for the same turn - a hand-over inside it - cannot replace the pin. */
        let _inner = register("turn-gate-test", gate("openai-api", "gpt-5", false, false));

        assert!(of("turn-gate-test").permit(Purpose::Main, Some("openai-api"), "gpt-5").is_err(), "the first pin stands");

        drop(held);

        assert!(of("turn-gate-test").permit(Purpose::Main, Some("openai-api"), "gpt-5").is_ok(), "released");
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    fn rotation_gate() -> Gate {
        Gate { enforcing: true, ..Gate::new(Some("ollama"), "qwen3.5:9b", true, false) }.with_chain(&[(Some("deepseek"), "deepseek-chat"), (Some("anthropic-api"), "claude-sonnet-5-5")])
    }

    /// Phase 3.6: with a chain on the gate, a call to a model that is not in the chain is refused - whatever its purpose.
    #[test]
    fn only_the_models_the_person_listed_are_reachable_and_in_their_order() {
        let gate = rotation_gate();

        assert!(gate.permit(Purpose::Chain, Some("deepseek"), "deepseek-chat").is_ok());
        assert!(gate.permit(Purpose::Chain, Some("anthropic-api"), "claude-sonnet-5-5").is_ok());
        assert!(gate.permit(Purpose::Chain, Some("openai-api"), "gpt-5").is_err(), "a model nobody listed");
        assert!(gate.permit(Purpose::Chain, Some("deepseek"), "deepseek-v4-pro").is_err(), "another model of a listed provider");
        assert!(gate.permit(Purpose::Router, Some("deepseek"), "deepseek-chat").is_err(), "a listed model, but not as a router's choice");
        assert!(gate.permit(Purpose::Main, Some("deepseek"), "deepseek-chat").is_err(), "the chain is not the main model");
        assert_eq!(gate.chain_model(0), Some(("deepseek".into(), "deepseek-chat".into())));
        assert_eq!(gate.chain_model(1).unwrap().1, "claude-sonnet-5-5");
        assert_eq!(gate.chain_model(2), None, "after the last entry there is no one else to ask");
    }

    #[test]
    fn a_disabled_entry_is_simply_not_on_the_gate() {
        let gate = Gate { enforcing: true, ..Gate::new(Some("ollama"), "qwen3.5:9b", true, false) }.with_chain(&[(Some("anthropic-api"), "claude-sonnet-5-5")]);

        assert!(gate.permit(Purpose::Chain, Some("deepseek"), "deepseek-chat").is_err());
        assert_eq!(gate.chain_len(), 1);
    }

    #[test]
    fn a_pinned_cloud_model_never_reaches_a_chain_even_if_a_chain_is_on_file() {
        let pinned = Gate { enforcing: true, ..Gate::new(Some("deepseek"), "deepseek-chat", true, false) }.with_chain(&[(Some("anthropic-api"), "claude-sonnet-5-5")]);

        assert_eq!(pinned.mode, Mode::Pinned);
        assert!(pinned.permit(Purpose::Chain, Some("anthropic-api"), "claude-sonnet-5-5").is_err());
        assert!(!pinned.allows(Purpose::Chain));
    }
}
