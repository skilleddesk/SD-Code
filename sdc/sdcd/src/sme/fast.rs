//! **Fast path (P1-15, switch `sme.fast_path`)**: most requests are small - a question about one file, a one-line change - and the
//! machinery built for large tasks (the spec restatement with its confirmation, the decomposer's plan) only slows them down.
//! A request is on the fast path when it is short, names at most one file, and does not ask for several things. The fast path
//! only *skips preparation*: every safety gate (secret guard, ModelGate, syntax gate, transaction) is unchanged.

use crate::runtime::decompose::files_in;

/// Longest request, in characters, that can be a fast one.
pub const MAX_CHARS: usize = 160;

/// Words that join a second job on to the first - in English, Bengali and Banglish.
const JOINERS: &[&str] = &["and then", "then", "also", "after that", "as well", "additionally", "tarpor", "tarpore", "ar ", "ebong", "এবং", "তারপর", "এরপর", "আর ", "আবার"];

/// True when the request needs no preparation beyond the goal itself.
pub fn is_fast(request: &str) -> bool {
    let text = request.trim();
    let lower = text.to_lowercase();

    if text.is_empty() || text.chars().count() > MAX_CHARS || text.lines().count() > 2 {
        return false;
    }

    if files_in(text).len() > 1 {
        return false;
    }

    /* several questions or instructions in one message are several jobs */
    if text.matches('?').count() > 1 || text.matches('\u{0964}').count() > 1 || text.matches(". ").count() > 1 {
        return false;
    }

    !JOINERS.iter().any(|joiner| lower.contains(joiner))
}

#[cfg(test)]
mod tests;
