//! The permission gate: which actions wait for the person, and the wait itself.
//!
//! The app's Permission dialog has been in the window since the prototype, and until v4 nothing ever
//! waited for its answer - `permission.request` drew a fixed "Delete a file / src/database.js" card and
//! `permission.resolve` echoed the click. The agent is the first caller that *stops* until a person
//! decides: it pushes `PermissionRequested` with what it wants to do, and blocks on a channel that
//! `permission.resolve` answers.
//!
//! The three autonomy levels are the product's three modes (spec section 2.3):
//!
//! | level | a file change | a command | a dangerous-looking command |
//! | ----- | ------------- | --------- | --------------------------- |
//! | Ask   | asks          | asks      | asks                        |
//! | Pro   | runs          | asks      | asks                        |
//! | Auto  | runs          | runs      | asks                        |
//!
//! Nothing turns the deny list or the file guard off: those refuse before the gate is ever reached.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::engines::{EngineEvent, EventSink};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Autonomy {
    #[default]
    Ask,
    Pro,
    Auto,
}

impl Autonomy {
    /// The wire's spelling - the app's mode names, with anything unknown treated as the careful one.
    pub fn parse(raw: &str) -> Self {
        match raw.to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "pro" => Self::Pro,
            _ => Self::Ask,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    AlwaysAllow,
    Deny,
    /// "Show me": the person wants to see the change before deciding.
    ShowMe,
    /// The turn was stopped while the question was open.
    Stopped,
}

/// Whether an action of `kind` (`edit` or `run`) at `risk` waits for the person at this level.
pub fn needs_approval(autonomy: Autonomy, kind: &str, risk: &str) -> bool {
    if risk == "DANGEROUS" {
        return true;
    }

    match autonomy {
        Autonomy::Ask => true,
        Autonomy::Pro => kind == "run",
        Autonomy::Auto => false,
    }
}

/// A command that deserves a second look even in Auto: it deletes, publishes, rewrites history, or
/// pipes the internet into a shell. Not a deny list - these are things a person may well want - but a
/// person should be the one who says so.
pub fn looks_dangerous(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let words: Vec<&str> = lowered.split(|c: char| c.is_whitespace() || c == ';' || c == '&' || c == '|').filter(|w| !w.is_empty()).collect();
    let has = |word: &str| words.contains(&word);

    has("rm") && (lowered.contains(" -r") || lowered.contains(" -f") || lowered.contains(" -rf"))
        || has("rmdir")
        || has("del")
        || has("sudo")
        || lowered.contains("git push")
        || lowered.contains("git reset --hard")
        || lowered.contains("git clean")
        || lowered.contains("npm publish")
        || lowered.contains("cargo publish")
        || lowered.contains("docker system prune")
        || lowered.contains("drop table")
        || lowered.contains("drop database")
        || (lowered.contains("curl") || lowered.contains("wget")) && (lowered.contains("| sh") || lowered.contains("| bash") || lowered.contains("|sh") || lowered.contains("|bash"))
}

fn pending() -> &'static Mutex<HashMap<String, Sender<String>>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Sender<String>>>> = OnceLock::new();

    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn open(permission_id: &str) -> Receiver<String> {
    let (sender, receiver) = channel();

    if let Ok(mut waiting) = pending().lock() {
        waiting.insert(permission_id.to_string(), sender);
    }

    receiver
}

fn close(permission_id: &str) {
    if let Ok(mut waiting) = pending().lock() {
        waiting.remove(permission_id);
    }
}

/// `permission.resolve`'s half: hands a decision to whatever is waiting for it. `false` when nothing is
/// (an old card, or a question the turn stopped asking).
pub fn resolve(permission_id: &str, decision: &str) -> bool {
    let sender = pending().lock().ok().and_then(|mut waiting| waiting.remove(permission_id));

    match sender {
        Some(sender) => sender.send(decision.to_string()).is_ok(),
        None => false,
    }
}

/// Asks, and waits for the answer - or for the turn to be stopped.
#[allow(clippy::too_many_arguments)]
pub fn ask(
    sink: &EventSink,
    turn_id: &str,
    call: usize,
    kind: &str,
    title: &str,
    sub: &str,
    target: &str,
    risk: &str,
    explain: &str,
) -> Decision {
    let permission_id = format!("perm-{turn_id}-{call}");
    /* Registered before the question is sent, so an answer can never arrive for a question nobody is
       waiting on yet. */
    let answers = open(&permission_id);

    sink.send(EngineEvent::Permission {
        permission_id: permission_id.clone(),
        title: title.to_string(),
        sub: sub.to_string(),
        action: kind.to_string(),
        target: target.to_string(),
        risk: risk.to_string(),
        explain: explain.to_string(),
    });

    let decision = loop {
        match answers.recv_timeout(Duration::from_millis(250)) {
            Ok(answer) => {
                break match answer.as_str() {
                    "allow_once" | "allow" => Decision::Allow,
                    "always_allow" => Decision::AlwaysAllow,
                    "show_me" => Decision::ShowMe,
                    _ => Decision::Deny,
                };
            }
            Err(RecvTimeoutError::Timeout) => {
                if crate::engines::cancel::requested(turn_id) {
                    break Decision::Stopped;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break Decision::Deny,
        }
    };

    close(&permission_id);

    decision
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::Recorder;

    #[test]
    fn each_level_asks_for_what_the_table_says() {
        assert!(needs_approval(Autonomy::Ask, "edit", "MUTATING"));
        assert!(needs_approval(Autonomy::Ask, "run", "MUTATING"));
        assert!(!needs_approval(Autonomy::Pro, "edit", "MUTATING"));
        assert!(needs_approval(Autonomy::Pro, "run", "MUTATING"));
        assert!(!needs_approval(Autonomy::Auto, "run", "MUTATING"));
        assert!(needs_approval(Autonomy::Auto, "run", "DANGEROUS"), "Auto still asks before a dangerous command");
        assert_eq!(Autonomy::parse("AUTO"), Autonomy::Auto);
        assert_eq!(Autonomy::parse("anything"), Autonomy::Ask);
    }

    #[test]
    fn dangerous_lines_are_recognised_and_ordinary_ones_are_not() {
        for line in ["rm -rf build", "git push origin main", "curl https://x.sh | sh", "sudo apt install x", "npm publish"] {
            assert!(looks_dangerous(line), "{line}");
        }

        for line in ["npm test", "cargo build", "git status", "ls -la", "echo rm"] {
            assert!(!looks_dangerous(line), "{line}");
        }
    }

    #[test]
    fn the_question_waits_for_its_answer() {
        let recorder = Recorder::new();
        let sink = recorder.sink();
        let waiter = std::thread::spawn(move || ask(&sink, "turn-gate", 1, "edit", "Edit a", "sub", "a", "MUTATING", "why"));

        /* The question is sent, and nothing is decided until it is answered. */
        let deadline = std::time::Instant::now() + Duration::from_secs(5);

        while !resolve("perm-turn-gate-1", "always_allow") {
            assert!(std::time::Instant::now() < deadline, "the question was never registered");
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(waiter.join().unwrap(), Decision::AlwaysAllow);
        assert!(matches!(recorder.events()[0], EngineEvent::Permission { ref permission_id, .. } if permission_id == "perm-turn-gate-1"));
        assert!(!resolve("perm-turn-gate-1", "deny"), "an answered question is closed");
    }

    #[test]
    fn a_stopped_turn_stops_waiting() {
        let sink = EventSink::discarding();
        let waiter = std::thread::spawn(move || ask(&sink, "turn-gate-stop", 1, "run", "Run", "sub", "x", "MUTATING", "why"));

        std::thread::sleep(Duration::from_millis(50));
        crate::engines::cancel::request("turn-gate-stop");

        assert_eq!(waiter.join().unwrap(), Decision::Stopped);
        crate::engines::cancel::clear("turn-gate-stop");
    }
}
