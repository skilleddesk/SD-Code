//! **Silent Relay's settings (M1-1)**: the `[relay]` part of `rotation.toml`.
//!
//! ```toml
//! [relay]
//! auto = false                      # switching it on in Settings -> Agents is the person's consent
//! local_attempts = 2                # tries on the local model before a failed step is relayed
//! local_ttft_s = 45                 # a local model silent this long is slow (the old 600 s limit stays for Local-only)
//! max_cloud_steps_per_turn = 6
//! max_usd_per_turn = 0.25
//! show_chip = true                  # the small cloud chip on the LiveBar
//! preroute = false                  # send a plainly too-hard step straight to the chain (unmeasured)
//! learn = false                     # propose what the chain solved to the knowledge core (unmeasured)
//! ```
//!
//! Like the rest of the file this is a **preference**, not permission: `.sdc/policy.toml` with `privacy = "local-only"` wins over
//! `auto = true`, and the model-selection contract (`guard::modelgate`) is what lets a call to a chain row through at all.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RelayPolicy {
    #[serde(default)]
    pub auto: bool,
    #[serde(default = "two")]
    pub local_attempts: u32,
    #[serde(default = "forty_five")]
    pub local_ttft_s: u32,
    #[serde(default = "six")]
    pub max_cloud_steps_per_turn: u32,
    #[serde(default = "quarter")]
    pub max_usd_per_turn: f64,
    #[serde(default = "yes")]
    pub show_chip: bool,
    #[serde(default)]
    pub preroute: bool,
    #[serde(default)]
    pub learn: bool,
}

fn two() -> u32 {
    2
}

fn forty_five() -> u32 {
    45
}

fn six() -> u32 {
    6
}

fn quarter() -> f64 {
    0.25
}

fn yes() -> bool {
    true
}

impl Default for RelayPolicy {
    fn default() -> Self {
        Self { auto: false, local_attempts: 2, local_ttft_s: 45, max_cloud_steps_per_turn: 6, max_usd_per_turn: 0.25, show_chip: true, preroute: false, learn: false }
    }
}

impl RelayPolicy {
    /// Is the relay on? The person's switch in Settings -> Agents (`auto`), or the `relay.auto` v3 switch (the bench and a test turn it on without a file).
    pub fn enabled(&self) -> bool {
        self.auto || crate::v3::on("relay.auto")
    }

    /// Ranges a person can sensibly choose; the same rules for the Agents tab and a hand-edited file.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=5).contains(&self.local_attempts) {
            return Err("Relay: the local attempts must be between 1 and 5.".into());
        }

        if !(5..=600).contains(&self.local_ttft_s) {
            return Err("Relay: the local time-to-first-token limit must be between 5 and 600 seconds.".into());
        }

        if !(1..=50).contains(&self.max_cloud_steps_per_turn) {
            return Err("Relay: at most 1 to 50 cloud steps a turn.".into());
        }

        if !self.max_usd_per_turn.is_finite() || !(0.0..=100.0).contains(&self.max_usd_per_turn) {
            return Err("Relay: the dollar limit per turn must be between 0 and 100.".into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_plans_and_the_relay_is_off() {
        let policy = RelayPolicy::default();

        assert!(!policy.auto && !policy.preroute && !policy.learn && policy.show_chip);
        assert_eq!((policy.local_attempts, policy.local_ttft_s, policy.max_cloud_steps_per_turn), (2, 45, 6));
        assert_eq!(policy.max_usd_per_turn, 0.25);
        assert!(!policy.enabled(), "nothing is switched on until the person says so");
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn a_file_without_a_relay_part_reads_as_the_defaults_and_a_partial_one_fills_the_rest() {
        let none: crate::data::rotation::Rotation = toml::from_str("[[chain]]\nmodel = \"deepseek:deepseek-chat\"\n").unwrap();

        assert_eq!(none.relay, RelayPolicy::default());

        let some: crate::data::rotation::Rotation = toml::from_str("[relay]\nauto = true\nmax_usd_per_turn = 1.5\n").unwrap();

        assert!(some.relay.auto && some.relay.enabled());
        assert_eq!(some.relay.max_usd_per_turn, 1.5);
        assert_eq!(some.relay.local_attempts, 2);
    }

    #[test]
    fn values_out_of_range_are_refused_with_a_sentence() {
        for bad in [
            RelayPolicy { local_attempts: 0, ..Default::default() },
            RelayPolicy { local_attempts: 9, ..Default::default() },
            RelayPolicy { local_ttft_s: 1, ..Default::default() },
            RelayPolicy { max_cloud_steps_per_turn: 0, ..Default::default() },
            RelayPolicy { max_usd_per_turn: -1.0, ..Default::default() },
            RelayPolicy { max_usd_per_turn: f64::NAN, ..Default::default() },
        ] {
            assert!(bad.validate().unwrap_err().starts_with("Relay:"), "{bad:?}");
        }
    }

    #[test]
    fn the_v3_switch_turns_the_relay_on_without_a_file() {
        assert!(crate::v3::forcing(&["relay.auto"], || RelayPolicy::default().enabled()));
    }

    #[test]
    fn an_unknown_key_is_reported_not_ignored() {
        assert!(toml::from_str::<crate::data::rotation::Rotation>("[relay]\nautoo = true\n").is_err());
    }
}
