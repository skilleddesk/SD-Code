//! Messages sent into a turn that is still running (0.12.5).
//!
//! The report: *"claude code a build howyar moddai ami jamon onno comamnd dile o seta pore adjust kore
//! nisse amader sdc taw ai rokom hote hobe"*. Until now a message typed while a turn ran waited in a queue
//! and became the next turn, after the agent had finished doing what it was about to be told not to.
//!
//! An agent turn that can take one opens an inbox here; `engine.steer` drops the person's words in it, and
//! the agent loop reads it between steps - after a tool call, or when it was about to finish - and hands the
//! words to the model as the person's next message. A turn with no inbox (a CLI turn) answers `false`, and
//! the window queues the message as before.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn inboxes() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static INBOXES: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();

    INBOXES.get_or_init(Default::default)
}

/// The turn takes messages from now on. Dropping the guard closes the inbox.
pub fn open(turn_id: &str) -> Inbox {
    if let Ok(mut map) = inboxes().lock() {
        map.insert(turn_id.to_string(), Vec::new());
    }

    Inbox { turn_id: turn_id.to_string() }
}

/// Drops a message into a running turn. `false` when the turn has no inbox (not running, or not an agent).
pub fn push(turn_id: &str, text: &str) -> bool {
    let text = text.trim();

    if text.is_empty() {
        return false;
    }

    match inboxes().lock() {
        Ok(mut map) => match map.get_mut(turn_id) {
            Some(queue) => {
                queue.push(text.to_string());
                true
            }
            None => false,
        },
        Err(_) => false,
    }
}

/// An open inbox, owned by the agent loop.
pub struct Inbox {
    turn_id: String,
}

impl Inbox {
    /// Everything that arrived since the last look, oldest first.
    pub fn take(&self) -> Vec<String> {
        inboxes()
            .lock()
            .ok()
            .and_then(|mut map| map.get_mut(&self.turn_id).map(std::mem::take))
            .unwrap_or_default()
    }
}

impl Drop for Inbox {
    fn drop(&mut self) {
        if let Ok(mut map) = inboxes().lock() {
            map.remove(&self.turn_id);
        }
    }
}

/// The words as the model reads them: said to be the person's, sent while it worked.
pub fn as_message(texts: &[String]) -> String {
    format!(
        "[The person sent this while you were working - take it into account from now on, adjusting or changing course as it asks:]\n{}",
        texts.join("\n\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_reaches_an_open_turn_only() {
        assert!(!push("turn-steer-1", "stop"));

        let inbox = open("turn-steer-1");

        assert!(push("turn-steer-1", "use tabs, not spaces"));
        assert!(push("turn-steer-1", "and name it utils.ts"));
        assert!(!push("turn-steer-1", "   "));
        assert_eq!(inbox.take(), vec!["use tabs, not spaces".to_string(), "and name it utils.ts".to_string()]);
        assert!(inbox.take().is_empty());

        drop(inbox);

        assert!(!push("turn-steer-1", "too late"));
    }
}
