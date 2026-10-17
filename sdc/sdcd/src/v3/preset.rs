//! **Profiles over the switches (Phase 4.4)**: Safe, Balanced and Economical are three named ways to set the ~40 switches of `v3`, so a person
//! who does not want to read forty descriptions can pick a posture, and one who does can still see and flip every switch under Advanced.
//!
//! * **Safe** - everything that protects stays on, every automatic helper that could surprise is off (the router, the cascade, the bandit,
//!   automations, fleet), and Max quality is on, which turns every token technique off.
//! * **Balanced** - what SDC ships: every switch back at its default.
//! * **Economical** - Balanced, plus the saving helpers that are default-on are made explicitly on again (router, cascade, learning, budget).
//!
//! The Q3 rule is not a preset's to bend: **no preset ever switches a token technique on.** A technique is on for a model only when its own
//! bench result says it lowered nothing by more than a point (`protocol/model-profiles.json`); Economical leaves them to that file, and Safe
//! turns them all off. A test pins this.

use serde_json::{json, Value};

use super::{Feature, FEATURES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Safe,
    Balanced,
    Economical,
}

impl Preset {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "safe" => Some(Self::Safe),
            "balanced" => Some(Self::Balanced),
            "economical" => Some(Self::Economical),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Balanced => "balanced",
            Self::Economical => "economical",
        }
    }
}

/// Switches that protect: Safe makes sure they are on.
const PROTECTIVE: &[&str] = &[
    "journal", "resume", "offline", "rate_limit", "redact", "untrusted", "privacy", "cas", "terms", "knowledge_core", "local.runtime_guard", "models.contract", "exec.sandbox", "trace.steps", "receipt", "budget",
];
/// Automatic helpers that act without being asked: Safe turns them off.
const SURPRISING: &[&str] = &["router", "cascade", "bandit", "automations", "fleet", "runner", "parallel"];
/// The default-on helpers that save money: Economical makes sure they are on.
const SAVING: &[&str] = &["router", "cascade", "learn", "budget", "knowledge_core"];

/// What a preset says about one switch: `Some(true/false)` writes it, `None` clears the person's setting (the default applies).
pub fn plan(preset: Preset) -> Vec<(&'static str, Option<bool>)> {
    FEATURES
        .iter()
        .map(|feature: &Feature| {
            let says = match preset {
                Preset::Safe if feature.token_technique => None, /* Max quality turns it off; the switch itself is left alone */
                Preset::Safe if PROTECTIVE.contains(&feature.name) => Some(true),
                Preset::Safe if SURPRISING.contains(&feature.name) => Some(false),
                Preset::Economical if SAVING.contains(&feature.name) => Some(true),
                _ => None,
            };

            (feature.name, says)
        })
        .collect()
}

/// Whether Max quality (every token technique off) belongs to the preset.
pub fn max_quality(preset: Preset) -> bool {
    preset == Preset::Safe
}

/// The setting writes a preset means: `(key, value)`, an empty value clearing the setting.
pub fn settings(preset: Preset) -> Vec<(String, String)> {
    let mut writes: Vec<(String, String)> = plan(preset)
        .into_iter()
        .map(|(name, says)| (super::key(name), says.map_or(String::new(), |on| if on { "on" } else { "off" }.to_string())))
        .collect();

    writes.push(("v3.maxQuality".to_string(), if max_quality(preset) { "on" } else { "" }.to_string()));
    writes
}

/// Which preset the stored settings amount to, or `custom` when the person has set switches by hand.
pub fn current(read: &dyn Fn(&str) -> Option<String>) -> &'static str {
    for preset in [Preset::Safe, Preset::Balanced, Preset::Economical] {
        if settings(preset).iter().all(|(key, value)| read(key).unwrap_or_default().trim().eq_ignore_ascii_case(value)) {
            return preset.name();
        }
    }

    "custom"
}

/// Every switch for the Advanced list: its state, its description, what kind it is, and - when it does nothing yet - why.
pub fn list() -> Value {
    json!(super::status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_preset_ever_switches_a_token_technique_on() {
        for preset in [Preset::Safe, Preset::Balanced, Preset::Economical] {
            for (name, says) in plan(preset) {
                let technique = FEATURES.iter().find(|feature| feature.name == name).unwrap().token_technique;

                assert!(!(technique && says == Some(true)), "{preset:?} would switch on {name}: the Q3 rule says only the bench may");
            }
        }
    }

    #[test]
    fn safe_protects_turns_helpers_off_and_asks_for_max_quality() {
        let safe = plan(Preset::Safe);
        let says = |name: &str| safe.iter().find(|(feature, _)| *feature == name).unwrap().1;

        assert_eq!(says("redact"), Some(true));
        assert_eq!(says("models.contract"), Some(true));
        assert_eq!(says("router"), Some(false));
        assert_eq!(says("cascade"), Some(false));
        assert_eq!(says("bandit"), Some(false));
        assert!(max_quality(Preset::Safe) && !max_quality(Preset::Balanced) && !max_quality(Preset::Economical));
    }

    #[test]
    fn balanced_clears_every_setting_so_the_defaults_apply() {
        assert!(plan(Preset::Balanced).iter().all(|(_, says)| says.is_none()));
        assert!(settings(Preset::Balanced).iter().all(|(_, value)| value.is_empty()));
    }

    #[test]
    fn economical_turns_the_saving_helpers_back_on_and_touches_nothing_else() {
        let economical = plan(Preset::Economical);

        for (name, says) in &economical {
            assert_eq!(*says, SAVING.contains(name).then_some(true), "{name}");
        }
    }

    #[test]
    fn the_current_preset_is_recognised_from_the_stored_settings_and_a_hand_change_is_custom() {
        let stored = |preset: Preset| {
            let map: std::collections::HashMap<String, String> = settings(preset).into_iter().collect();

            move |key: &str| map.get(key).cloned().filter(|value| !value.is_empty())
        };

        for preset in [Preset::Safe, Preset::Balanced, Preset::Economical] {
            assert_eq!(current(&stored(preset)), preset.name());
        }

        let tweaked = |key: &str| if key == "v3.fleet" { Some("on".to_string()) } else { None };

        assert_eq!(current(&tweaked), "custom");
        assert_eq!(Preset::parse(" Safe "), Some(Preset::Safe));
        assert_eq!(Preset::parse("wild"), None);
    }
}
