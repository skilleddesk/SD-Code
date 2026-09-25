//! Which turns a person has stopped.
//!
//! `engine.cancel` used to push `TurnCompleted: Interrupted` and nothing else. The engine was never
//! told, so the Stop button marked the turn finished while the model went on answering (and billing)
//! behind it, and the deltas that kept arriving flipped the turn back to `running` in the window.
//!
//! One process-wide set, keyed by turn id (unique for the daemon's life): `engine.cancel` adds the
//! turn, the turn loop stops forwarding its events, and the HTTP adapters and the agent check it
//! between reads so the connection is dropped rather than drained. A CLI engine is also killed through
//! its own `Engine::cancel`, which is what stops a process.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn set() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Marks a turn as stopped.
pub fn request(turn_id: &str) {
    if let Ok(mut turns) = set().lock() {
        turns.insert(turn_id.to_string());
    }
}

/// Whether a turn has been stopped.
pub fn requested(turn_id: &str) -> bool {
    set().lock().map(|turns| turns.contains(turn_id)).unwrap_or(false)
}

/// Forgets a turn once it has ended, so the set does not grow for the daemon's whole life.
pub fn clear(turn_id: &str) {
    if let Ok(mut turns) = set().lock() {
        turns.remove(turn_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_turn_is_remembered_until_it_is_cleared() {
        assert!(!requested("turn-cancel-test"));

        request("turn-cancel-test");
        assert!(requested("turn-cancel-test"));
        assert!(!requested("turn-cancel-other"));

        clear("turn-cancel-test");
        assert!(!requested("turn-cancel-test"));
    }
}
