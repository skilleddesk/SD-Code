//! The kill switch's register of everything that is running (Trust Kernel, part 7).
//!
//! A turn, a Verify run, a deploy, a playbook - each registers here when it starts and leaves when it
//! ends, so one keystroke (`Ctrl+Shift+.`) can stop all of them without anyone having to remember which
//! kinds of work exist. Stopping is the same cancel mark each of them already checks
//! (`engines::cancel`), so nothing here reaches into another module's internals.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct Active {
    pub id: String,
    /// `turn`, `verify`, `deploy`, `playbook`, `guardian`.
    pub kind: &'static str,
    pub session_id: Option<String>,
    pub label: String,
    pub started: String,
}

fn register() -> &'static Mutex<HashMap<String, Active>> {
    static ACTIVE: OnceLock<Mutex<HashMap<String, Active>>> = OnceLock::new();

    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Work started.
pub fn begin(id: &str, kind: &'static str, session_id: Option<&str>, label: &str) {
    if let Ok(mut active) = register().lock() {
        active.insert(
            id.to_string(),
            Active {
                id: id.to_string(),
                kind,
                session_id: session_id.map(str::to_string),
                label: label.chars().take(120).collect(),
                started: chrono::Utc::now().to_rfc3339(),
            },
        );
    }
}

/// Work ended, however it ended.
pub fn end(id: &str) {
    if let Ok(mut active) = register().lock() {
        active.remove(id);
    }
}

/// Everything running now.
pub fn active() -> Vec<Active> {
    register().lock().map(|active| active.values().cloned().collect()).unwrap_or_default()
}

/// Marks everything running as cancelled and empties the register. The caller stops the engines and
/// the processes; this is the part every kind of work shares.
pub fn cancel_all() -> Vec<Active> {
    let stopped: Vec<Active> = register().lock().map(|mut active| active.drain().map(|(_, work)| work).collect()).unwrap_or_default();

    for work in &stopped {
        crate::engines::cancel::request(&work.id);
    }

    stopped
}

pub fn to_json(work: &Active) -> Value {
    json!({ "id": work.id, "kind": work.kind, "sessionId": work.session_id, "label": work.label, "started": work.started })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_registered_is_cancelled_once() {
        begin("kill-test-turn", "turn", Some("s1"), "Fix the form");
        begin("kill-test-verify", "verify", Some("s1"), "Verify");

        assert!(active().iter().any(|work| work.id == "kill-test-turn"));

        let stopped = cancel_all();

        assert!(stopped.iter().any(|work| work.id == "kill-test-verify"));
        assert!(crate::engines::cancel::requested("kill-test-turn"));
        assert!(!active().iter().any(|work| work.id.starts_with("kill-test")));

        crate::engines::cancel::clear("kill-test-turn");
        crate::engines::cancel::clear("kill-test-verify");
    }
}
