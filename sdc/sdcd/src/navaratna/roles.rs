//! **N8-3 - role-based rotation and quota failover** (switch `nr.role_rotation`, off).
//!
//! A chain row may carry a **role** - `architect` (designs once), `fixer` (repairs a step the local model could not), or `any` (the default).
//! A job asks for a role and gets the first healthy row that has it, else the first healthy row of any role. The user's order is never
//! changed: rows are only skipped. A row that answers HTTP 429, runs out of quota, has no key or times out is put to **cool down** for a while
//! and the next row is asked, so a quota on row 1 never stops a turn when row 2 is healthy. The turn's step and dollar limits stay in force
//! (they belong to the relay policy and are checked by the caller).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::rotation::executor::ChainModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Architect,
    Fixer,
    Any,
}

impl Role {
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "architect" => Role::Architect,
            "fixer" => Role::Fixer,
            _ => Role::Any,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Architect => "architect",
            Role::Fixer => "fixer",
            Role::Any => "any",
        }
    }
}

/// Is `text` a role the file may hold (empty means `any`)?
pub fn valid_role(text: &str) -> bool {
    matches!(text.trim().to_ascii_lowercase().as_str(), "" | "any" | "architect" | "fixer")
}

/// Why a row stopped being usable, and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    RateLimited,
    Quota,
    MissingKey,
    Timeout,
}

impl Cause {
    pub fn cooldown(self) -> Duration {
        match self {
            Cause::RateLimited => Duration::from_secs(60),
            Cause::Timeout => Duration::from_secs(120),
            Cause::Quota => Duration::from_secs(3_600),
            Cause::MissingKey => Duration::from_secs(24 * 3_600),
        }
    }
}

/// What a failure says about the row: only the four causes cool a row down; any other failure does not.
pub fn classify(failure: &str) -> Option<Cause> {
    let text = failure.to_ascii_lowercase();

    if text.contains("429") || text.contains("rate limit") || text.contains("rate-limit") || text.contains("too many requests") {
        Some(Cause::RateLimited)
    } else if text.contains("quota") || text.contains("insufficient") || text.contains("billing") || text.contains("credit") || text.contains("exceeded your current") {
        Some(Cause::Quota)
    } else if text.contains("api key") || text.contains("no key") || text.contains("missing key") || text.contains("not signed in") || text.contains("401") || text.contains("unauthorized") {
        Some(Cause::MissingKey)
    } else if text.contains("timed out") || text.contains("timeout") || text.contains("time limit") {
        Some(Cause::Timeout)
    } else {
        None
    }
}

/// Rows that are cooling down.
#[derive(Debug, Default)]
pub struct Cooling {
    until: HashMap<String, Instant>,
}

/// The key a row cools under.
///
/// In a release build it is the row's own label, because a provider's rate limit is a fact about that provider and
/// belongs to the whole process - that is the point of [`table`].
///
/// In a **test** build the thread is part of it. The table is one per process and the test binary runs its tests at
/// the same time, so a row that one test deliberately fails (`when_every_chain_model_fails...`) used to be skipped in
/// another test running beside it: its chain looked empty, the turn fell back to a scripted local provider that had
/// already finished, and the run hung for two minutes on a network that was never coming. That is a flake, not a
/// finding, and it only appeared once `nr.role_rotation` was on by default (3.2).
fn key(label: &str) -> String {
    #[cfg(test)]
    {
        format!("{:?}\u{1}{label}", std::thread::current().id())
    }

    #[cfg(not(test))]
    {
        label.to_string()
    }
}

impl Cooling {
    pub fn cool(&mut self, label: &str, cause: Cause, now: Instant) {
        self.until.insert(key(label), now + cause.cooldown());
    }

    pub fn is_cooling(&self, label: &str, now: Instant) -> bool {
        self.until.get(&key(label)).is_some_and(|until| *until > now)
    }

    pub fn clear(&mut self) {
        self.until.clear();
    }
}

/// The process's one table of cooling rows.
pub fn table() -> &'static Mutex<Cooling> {
    static TABLE: OnceLock<Mutex<Cooling>> = OnceLock::new();

    TABLE.get_or_init(Default::default)
}

/// The rows to ask for a job of `want`, in the user's order: healthy rows only; among them the ones with the role if there is any, else all.
/// With the switch off the chain is returned as it is.
pub fn candidates(chain: &[ChainModel], want: Role, cooling: &Cooling, now: Instant) -> Vec<ChainModel> {
    if !crate::v3::on("nr.role_rotation") {
        return chain.to_vec();
    }

    let healthy: Vec<&ChainModel> = chain.iter().filter(|row| !cooling.is_cooling(&row.label(), now)).collect();
    let matching: Vec<&ChainModel> = healthy.iter().copied().filter(|row| want == Role::Any || Role::parse(&row.role) == want).collect();
    let chosen = if matching.is_empty() { healthy } else { matching };

    chosen.into_iter().cloned().collect()
}

/// Notes a failed call of a row: if its words say quota, rate limit, a missing key or a timeout, the row cools down.
pub fn note_failure(label: &str, failure: &str) -> Option<Cause> {
    if !crate::v3::on("nr.role_rotation") {
        return None;
    }

    let cause = classify(failure)?;

    if let Ok(mut table) = table().lock() {
        table.cool(label, cause, Instant::now());
    }

    Some(cause)
}

/// The candidates for a job against the process's table.
pub fn candidates_now(chain: &[ChainModel], want: Role) -> Vec<ChainModel> {
    match table().lock() {
        Ok(table) => candidates(chain, want, &table, Instant::now()),
        Err(_) => chain.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(model: &str, role: &str) -> ChainModel {
        ChainModel { provider: "p".into(), model: model.into(), max_cost_per_step: 0.0, role: role.into() }
    }

    fn chain() -> Vec<ChainModel> {
        vec![row("cheap", ""), row("architect-1", "architect"), row("fixer-1", "fixer"), row("fixer-2", "fixer")]
    }

    fn names(rows: &[ChainModel]) -> Vec<&str> {
        rows.iter().map(|row| row.model.as_str()).collect()
    }

    #[test]
    fn a_job_gets_the_rows_with_its_role_in_the_users_order_else_every_healthy_row() {
        crate::v3::forcing(&["nr.role_rotation"], || {
            let cooling = Cooling::default();
            let now = Instant::now();

            assert_eq!(names(&candidates(&chain(), Role::Architect, &cooling, now)), ["architect-1"]);
            assert_eq!(names(&candidates(&chain(), Role::Fixer, &cooling, now)), ["fixer-1", "fixer-2"]);
            assert_eq!(names(&candidates(&chain(), Role::Any, &cooling, now)), ["cheap", "architect-1", "fixer-1", "fixer-2"], "the order is the user's");

            let no_architect = vec![row("a", ""), row("b", "fixer")];

            assert_eq!(names(&candidates(&no_architect, Role::Architect, &cooling, now)), ["a", "b"], "nobody has the role: every healthy row, in the user's order");
            assert_eq!(names(&candidates(&[row("b", "fixer")], Role::Architect, &cooling, now)), ["b"], "else any row at all");
        });
    }

    #[test]
    fn a_429_on_row_one_never_stops_the_turn_when_row_two_is_healthy() {
        crate::v3::forcing(&["nr.role_rotation"], || {
            let mut cooling = Cooling::default();
            let now = Instant::now();

            cooling.cool("p:fixer-1", classify("HTTP 429 Too Many Requests").unwrap(), now);

            assert_eq!(names(&candidates(&chain(), Role::Fixer, &cooling, now)), ["fixer-2"]);

            cooling.cool("p:fixer-2", Cause::Quota, now);

            assert_eq!(names(&candidates(&chain(), Role::Fixer, &cooling, now)), ["cheap", "architect-1"], "no healthy fixer: any healthy row");
            assert!(candidates(&[row("only", "")], Role::Any, &{
                let mut c = Cooling::default();
                c.cool("p:only", Cause::MissingKey, now);
                c
            }, now).is_empty(), "nothing healthy: the caller says so honestly");
        });
    }

    #[test]
    fn a_row_comes_back_when_its_cooldown_is_over() {
        let mut cooling = Cooling::default();
        let now = Instant::now();

        cooling.cool("p:x", Cause::RateLimited, now);

        assert!(cooling.is_cooling("p:x", now + Duration::from_secs(59)));
        assert!(!cooling.is_cooling("p:x", now + Duration::from_secs(61)));
        assert!(!cooling.is_cooling("p:y", now));
    }

    #[test]
    fn the_four_causes_are_told_apart_and_an_ordinary_failure_cools_nothing() {
        assert_eq!(classify("HTTP 429"), Some(Cause::RateLimited));
        assert_eq!(classify("You exceeded your current quota"), Some(Cause::Quota));
        assert_eq!(classify("insufficient credit"), Some(Cause::Quota));
        assert_eq!(classify("no API key for this provider"), Some(Cause::MissingKey));
        assert_eq!(classify("the call timed out after 180 s"), Some(Cause::Timeout));
        assert_eq!(classify("the model refused"), None);
        assert_eq!(classify("the project's checks failed"), None);
    }

    #[test]
    fn the_process_table_cools_a_row_on_a_failure_and_is_off_when_the_switch_is() {
        /* 3.2: on by default, so the off case is said out loud. */
        crate::v3::ablating(&["nr.role_rotation"], || {
            assert_eq!(note_failure("p:zzz-off", "HTTP 429"), None, "switched off, nothing cools");
        });

        crate::v3::forcing(&["nr.role_rotation"], || {
            assert_eq!(note_failure("p:zzz-on", "HTTP 429"), Some(Cause::RateLimited));
            assert!(!names(&candidates_now(&[row("zzz-on", ""), row("next", "")], Role::Fixer)).contains(&"zzz-on"));
            assert_eq!(note_failure("p:zzz-other", "the checks failed"), None);
        });

        if let Ok(mut table) = table().lock() {
            table.clear();
        }
    }

    #[test]
    fn with_the_switch_off_the_chain_is_returned_whole_and_in_order() {
        crate::v3::ablating(&["nr.role_rotation"], || {
            let mut cooling = Cooling::default();

            cooling.cool("p:cheap", Cause::Quota, Instant::now());

            assert_eq!(names(&candidates(&chain(), Role::Architect, &cooling, Instant::now())), ["cheap", "architect-1", "fixer-1", "fixer-2"]);
        });

        assert!(valid_role("") && valid_role("Fixer") && !valid_role("boss"));
    }
}
