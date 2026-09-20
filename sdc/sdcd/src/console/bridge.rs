//! The console bridge - the preview's errors become the next turn (spec sections 15.4, 15.5).
//!
//! A preview pane reports a failure the way a browser does: a message, then `at LoginForm.tsx:42:11`.
//! `parse_console_line` is the only place that shape is understood, and it is a pure function so the
//! fixtures can hold it - a console parser that only works inside a live WebView would be untestable.
//!
//! `ConsoleBridge` is the session's side of the contract: the bytes come from the app's preview (the
//! only thing that can see its own console), and the daemon records which session is listening, which
//! is what makes "Fix with agent" land in the right chat.

use std::collections::HashSet;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;

/// The sessions whose preview console is being listened to.
#[derive(Default)]
pub struct ConsoleBridge {
    attached: Mutex<HashSet<String>>,
}

impl ConsoleBridge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach(&self, session_id: &str, url: &str) -> Result<Value, ErrorObject> {
        self.attached
            .lock()
            .map_err(|_| ErrorObject::internal("console registry poisoned"))?
            .insert(session_id.to_string());

        Ok(json!({ "attached": true, "sessionId": session_id, "url": url }))
    }

    pub fn detach(&self, session_id: &str) -> Result<Value, ErrorObject> {
        self.attached
            .lock()
            .map_err(|_| ErrorObject::internal("console registry poisoned"))?
            .remove(session_id);

        Ok(json!({ "detached": true, "sessionId": session_id }))
    }

    pub fn is_attached(&self, session_id: &str) -> bool {
        self.attached.lock().map(|set| set.contains(session_id)).unwrap_or(false)
    }

    pub fn attached_count(&self) -> usize {
        self.attached.lock().map(|set| set.len()).unwrap_or(0)
    }
}

/// One parsed console line, in the shape `ConsoleError` carries.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleLine {
    pub level: String,
    pub message: String,
    pub source: String,
    pub file: String,
    pub line: i64,
}

impl ConsoleLine {
    pub fn to_event_payload(&self, session_id: &str) -> Value {
        json!({
            "sessionId": session_id,
            "level": self.level,
            "message": self.message,
            "source": self.source,
            "file": self.file,
            "line": self.line,
        })
    }
}

/// Parses one line of a preview console.
///
/// The two shapes a WebView actually produces:
///
/// * `Uncaught ReferenceError: handleSubmit is not defined at LoginForm.tsx:42:11`
/// * `Warning: Each child in a list should have a unique "key" prop. at UserList.tsx:18:5`
///
/// The level comes from the text and the location from the trailing `at` clause - the same
/// information the app's Console tab shows as `at LoginForm.tsx:42:11`.
pub fn parse_console_line(raw: &str) -> Option<ConsoleLine> {
    let text = raw.trim();

    if text.is_empty() {
        return None;
    }

    let level = if text.contains("Uncaught")
        || text.starts_with("Error")
        || text.contains("TypeError")
        || text.contains("ReferenceError")
    {
        "error"
    } else if text.starts_with("Warning") || text.contains("Warning:") {
        "warn"
    } else {
        "info"
    };

    let location = text
        .split_whitespace()
        .rev()
        .find(|token| token.starts_with("at") || token.contains(".tsx:") || token.contains(".ts:") || token.contains(".js:"))
        .unwrap_or("");
    let (file, line) = split_location(location.trim_start_matches("at").trim());

    Some(ConsoleLine {
        level: level.to_string(),
        message: text.split(" at ").next().unwrap_or(text).to_string(),
        source: if location.is_empty() { String::new() } else { format!("at {location}") },
        file,
        line,
    })
}

/// `LoginForm.tsx:42:11` → `("LoginForm.tsx", 42)`. A bare file name keeps line 0.
fn split_location(location: &str) -> (String, i64) {
    let mut parts = location.rsplitn(3, ':');
    let first = parts.next().unwrap_or_default().to_string();
    let second = parts.next().map(str::to_string);
    let third = parts.next().map(str::to_string);

    match (first.parse::<i64>(), second, third) {
        /* file:line */
        (Ok(line), Some(file), None) => (file, line),
        /* file:line:col */
        (Ok(_column), Some(line), Some(file)) => (file, line.parse().unwrap_or(0)),
        _ => (location.to_string(), 0),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_two_shapes_a_webview_produces() {
        let error = parse_console_line(
            "Uncaught ReferenceError: handleSubmit is not defined at LoginForm.tsx:42:11",
        )
        .unwrap();

        assert_eq!(error.level, "error");
        assert_eq!(error.file, "LoginForm.tsx");
        assert_eq!(error.line, 42);
        assert!(error.source.contains("LoginForm.tsx:42:11"));

        let warning = parse_console_line(
            r#"Warning: Each child in a list should have a unique "key" prop. at UserList.tsx:18:5"#,
        )
        .unwrap();

        assert_eq!(warning.level, "warn");
        assert_eq!(warning.file, "UserList.tsx");
        assert_eq!(warning.line, 18);
    }

    #[test]
    fn a_bare_file_and_line_is_split_too() {
        assert_eq!(split_location("a.ts:7"), ("a.ts".to_string(), 7));
        assert_eq!(split_location("a.ts"), ("a.ts".to_string(), 0));
    }

    #[test]
    fn a_line_with_no_location_is_still_a_line() {
        let info = parse_console_line("Preview reloaded").unwrap();

        assert_eq!(info.level, "info");
        assert_eq!(info.file, "");
        assert_eq!(info.line, 0);
    }

    #[test]
    fn an_empty_line_is_nothing() {
        assert!(parse_console_line("   ").is_none());
    }

    #[test]
    fn attach_and_detach_track_one_session() {
        let bridge = ConsoleBridge::new();

        assert!(!bridge.is_attached("s1"));
        bridge.attach("s1", "http://localhost:3000").unwrap();
        assert!(bridge.is_attached("s1"));
        assert_eq!(bridge.attached_count(), 1);

        bridge.detach("s1").unwrap();
        assert!(!bridge.is_attached("s1"));
    }

    #[test]
    fn the_event_payload_matches_the_console_error_catalogue_entry() {
        let payload = parse_console_line("Error: boom at a.ts:1:1").unwrap().to_event_payload("s1");

        assert_eq!(payload["sessionId"], json!("s1"));
        assert_eq!(payload["level"], json!("error"));
        assert_eq!(payload["file"], json!("a.ts"));
    }
}

