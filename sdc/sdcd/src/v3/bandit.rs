//! **Bandit router** (v3 L4, switch `v3.bandit`, off until it passes Q3): the router's rule - a question to the
//! fast model, work to the main one - is a guess about the person's project. The bandit replaces the guess with
//! what actually happened: each kind of request (a question, a change, a search) is played against the models the
//! person has, and the one that has done best for that kind is picked - best meaning *the turn finished* and *it
//! did not cost much*, which is the trade the person is making.
//!
//! Exploration is deliberate: a model with no record for a kind is tried first, and one whose record is close is
//! tried again now and then (UCB1). The record lives in `.sdc/bandit.json`, which the person can read, and the
//! whole thing is off until a paid comparison says it beats the plain rule.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FILE: &str = ".sdc/bandit.json";

/// How much of a model's record is needed before its average is worth trusting (the exploration weight).
const EXPLORE: f64 = 1.2;

/// What one arm (one model) has done for one kind of request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Arm {
    pub pulls: u64,
    /// The running total of rewards; the mean is `total / pulls`.
    pub total: f64,
}

/// The whole record: per kind of request, per model.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Record {
    #[serde(default)]
    pub kinds: HashMap<String, HashMap<String, Arm>>,
}

/// The kind of a request: enough to tell a question from a change from a search.
pub fn kind_of(request: &str) -> String {
    let lower = request.to_lowercase();

    if ["grep", "search", "find all", "where is", "khuje", "kothay"].iter().any(|word| lower.contains(word)) {
        return "search".to_string();
    }

    if crate::v3::router::is_light(request) {
        return "question".to_string();
    }

    if ["read", "explain", "review", "audit", "dekho", "dekhao"].iter().any(|word| lower.contains(word)) {
        return "read".to_string();
    }

    "change".to_string()
}

impl Record {
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> bool {
        if let Some(parent) = root.join(FILE).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        std::fs::write(root.join(FILE), serde_json::to_string_pretty(self).unwrap_or_default() + "\n").is_ok()
    }

    /// One arm's record for one kind.
    pub fn arm(&self, kind: &str, model: &str) -> Arm {
        self.kinds.get(kind).and_then(|arms| arms.get(model)).cloned().unwrap_or_default()
    }

    /// How many turns this kind has seen at all (the `N` of UCB1).
    fn seen(&self, kind: &str) -> u64 {
        self.kinds.get(kind).map(|arms| arms.values().map(|arm| arm.pulls).sum()).unwrap_or(0)
    }

    /// Which model to use for this kind: an untried one first, then the highest upper confidence bound.
    pub fn choose<'a>(&self, kind: &str, models: &[&'a str]) -> Option<&'a str> {
        if models.is_empty() {
            return None;
        }

        let seen = self.seen(kind);

        /* An arm with no pulls has an unbounded bound: it is tried first. */
        if let Some(fresh) = models.iter().find(|model| self.arm(kind, model).pulls == 0) {
            return Some(fresh);
        }

        let bound = |arm: &Arm| arm.total / arm.pulls as f64 + EXPLORE * ((seen as f64).ln() / arm.pulls as f64).sqrt();

        models.iter().copied().max_by(|left, right| bound(&self.arm(kind, left)).partial_cmp(&bound(&self.arm(kind, right))).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Records how a turn on this model went: the reward is what the bandit learns from.
    pub fn observe(&mut self, kind: &str, model: &str, reward: f64) {
        let arm = self.kinds.entry(kind.to_string()).or_default().entry(model.to_string()).or_default();

        arm.pulls += 1;
        arm.total += reward.clamp(-1.0, 1.0);
    }

    /// What each model has earned for a kind, best first - for the person to look at.
    pub fn ranking(&self, kind: &str) -> Vec<(String, Arm, f64)> {
        let mut ranked: Vec<(String, Arm, f64)> = self
            .kinds
            .get(kind)
            .map(|arms| arms.iter().map(|(model, arm)| (model.clone(), arm.clone(), arm.total / arm.pulls.max(1) as f64)).collect())
            .unwrap_or_default();

        ranked.sort_by(|left, right| right.2.partial_cmp(&left.2).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }
}

/// What a finished turn teaches the bandit: it got there (1) or it did not (0), minus what it cost.
///
/// `cost` and `budget` are in the same unit (the turn's estimate against the person's budget for it): a turn that
/// spent its whole budget pays a quarter of its reward for it, so a cheap model that finishes wins over an
/// expensive one that finishes just as well.
pub fn reward(finished: bool, cost: Option<f64>, budget: Option<f64>) -> f64 {
    let base = if finished { 1.0 } else { 0.0 };
    let price = match (cost, budget) {
        (Some(cost), Some(budget)) if budget > 0.0 => (cost / budget).clamp(0.0, 1.0) * 0.25,
        _ => 0.0,
    };

    base - price
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untried_model_is_tried_and_the_better_one_wins_afterwards() {
        let mut record = Record::default();
        let models = ["fast", "main"];

        assert_eq!(record.choose("question", &models), Some("fast"), "the first is untried");
        record.observe("question", "fast", 1.0);
        assert_eq!(record.choose("question", &models), Some("main"), "then the other");

        record.observe("question", "main", 0.1);

        /* From here the better one should lead, with the other still tried now and then. */
        let mut leader = 0;

        for _ in 0..20 {
            let chosen = record.choose("question", &models).unwrap();

            if chosen == "fast" {
                leader += 1;
            }

            record.observe("question", chosen, if chosen == "fast" { 1.0 } else { 0.1 });
        }

        assert!(leader >= 13, "the model that finishes leads: {leader} of 20");
        assert_eq!(record.choose("change", &["only"]), Some("only"), "a kind with no history starts fresh");
        assert_eq!(record.choose("question", &[]), None);
    }

    #[test]
    fn the_record_survives_a_restart_and_the_ranking_reads_plainly() {
        let root = std::env::temp_dir().join(format!("sdc-bandit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let mut record = Record::default();

        record.observe("change", "draft", 0.9);
        record.observe("change", "draft", 0.9);
        record.observe("change", "strong", 0.5);

        assert!(record.save(&root));

        let loaded = Record::load(&root);

        assert_eq!(loaded, record);
        assert_eq!(loaded.ranking("change").first().unwrap().0, "draft");
        assert_eq!(loaded.arm("change", "draft").pulls, 2);
        assert_eq!(Record::load(&root.join("nowhere")), Record::default(), "a project with no record starts empty");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_kind_of_a_request_is_told_from_its_words() {
        assert_eq!(kind_of("what does this error mean?"), "question");
        assert_eq!(kind_of("where is the charge function used"), "search");
        assert_eq!(kind_of("read the pricing module and review it"), "read");
        assert_eq!(kind_of("add pagination to the orders list"), "change");
    }

    #[test]
    fn finishing_cheaply_beats_finishing_dearly_and_failing_scores_nothing() {
        assert_eq!(reward(true, None, None), 1.0);
        assert_eq!(reward(false, None, None), 0.0);
        assert_eq!(reward(true, Some(0.02), Some(0.10)), 0.95, "a fifth of the budget costs a twentieth of the reward");
        assert_eq!(reward(true, Some(0.50), Some(0.10)), 0.75, "and over budget is capped");
        assert_eq!(reward(false, Some(0.05), Some(0.10)), -0.125, "a turn that did not get there scores nothing, less what it cost");
    }
}
