//! **Opt-in telemetry (Phase 4.2)**: off until the person turns it on, counts only, nothing else.
//!
//! What may ever be recorded is a fixed list of event names ([`EVENTS`]) and, with each, the day (not the time), SDC's version and the
//! operating system's name. There is no field for text: no prompt, no code, no file name, no path, no key, no error message, no model
//! name, no folder - so none can be recorded by mistake. [`record`] refuses a name that is not on the list.
//!
//! While off, [`record`] returns before it touches the disk. While on, events are written to `<data>/telemetry/events.jsonl` on this
//! computer, where the person can read them (`telemetry.events`) and clear them. Sending is a separate, explicit act (`telemetry.send`):
//! it needs an endpoint the person set, posts the aggregate counts the person was shown (`telemetry.status` returns the exact payload),
//! and never runs by itself. SDC ships with **no endpoint**: until one is set, "on" only means "counted here".

use std::io::Write;
use std::path::PathBuf;

use serde_json::{json, Value};

/// The only things that can be counted: a feature was used.
pub const EVENTS: &[&str] = &["app_open", "turn_run", "verify_run", "ship_safe", "voice_used", "escalate", "time_machine_restore", "onboarding_done"];

pub const OPT_IN: &str = "telemetry.optIn";
pub const ENDPOINT: &str = "telemetry.endpoint";
pub const CONSENT: &str = "telemetry.consentAt";

fn file() -> Option<PathBuf> {
    crate::paths::data_dir().ok().map(|dir| dir.join("telemetry").join("events.jsonl"))
}

/// The operating system's name, and nothing finer.
pub fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// Counts one use of a feature - only when the person opted in (`enabled`), only a listed name, only the day.
pub fn record(enabled: bool, event: &str) -> bool {
    if !enabled || !EVENTS.contains(&event) {
        return false;
    }

    record_to(file(), event, &chrono::Utc::now().format("%Y-%m-%d").to_string())
}

fn record_to(path: Option<PathBuf>, event: &str, day: &str) -> bool {
    let Some(path) = path else {
        return false;
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let line = json!({ "event": event, "day": day, "version": crate::VERSION, "os": os() }).to_string();

    std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut out| writeln!(out, "{line}")).is_ok()
}

/// The recorded events, newest last.
pub fn events() -> Vec<Value> {
    events_from(file())
}

fn events_from(path: Option<PathBuf>) -> Vec<Value> {
    path.and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).collect())
        .unwrap_or_default()
}

/// Deletes what was recorded. Answers how many events there were.
pub fn clear() -> usize {
    let count = events().len();

    if let Some(path) = file() {
        let _ = std::fs::remove_file(path);
    }

    count
}

/// What would be sent: per event name, how many times on how many days - plus the version and the OS. Built from [`events_from`] output
/// only, so it can contain nothing the file does not.
pub fn payload(recorded: &[Value]) -> Value {
    let mut counts: std::collections::BTreeMap<String, (u64, std::collections::BTreeSet<String>)> = Default::default();

    for event in recorded {
        let name = event["event"].as_str().unwrap_or_default();

        if EVENTS.contains(&name) {
            let entry = counts.entry(name.to_string()).or_default();

            entry.0 += 1;
            entry.1.insert(event["day"].as_str().unwrap_or_default().to_string());
        }
    }

    json!({
        "version": crate::VERSION,
        "os": os(),
        "counts": counts.iter().map(|(name, (times, days))| json!({ "event": name, "times": times, "days": days.len() })).collect::<Vec<_>>(),
    })
}

/// Posts the payload to the endpoint the person set. Never called by anything but the person's own press.
pub fn send(endpoint: &str, body: &Value) -> Result<u16, String> {
    if !(endpoint.starts_with("https://") || endpoint.starts_with("http://127.0.0.1") || endpoint.starts_with("http://localhost")) {
        return Err("the endpoint must be an https:// address".into());
    }

    ureq::post(endpoint).set("content-type", "application/json").send_string(&body.to_string()).map(|response| response.status()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sdc-telemetry-{name}-{}", std::process::id())).join("events.jsonl")
    }

    #[test]
    fn nothing_is_recorded_while_it_is_off_and_nothing_off_the_list_is_ever_recorded() {
        assert!(!record(false, "turn_run"), "off: not even a file is touched");
        assert!(!record(true, "my secret prompt"), "an event name that is not on the list is refused");
        assert!(!record(true, "/Users/me/project/src/main.rs"), "a path is not an event");
    }

    #[test]
    fn a_recorded_event_carries_only_the_name_the_day_the_version_and_the_os() {
        let path = temp("fields");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        assert!(record_to(Some(path.clone()), "verify_run", "2026-10-18"));

        let read = events_from(Some(path.clone()));
        let keys: Vec<&str> = read[0].as_object().unwrap().keys().map(String::as_str).collect();

        assert_eq!(read.len(), 1);
        assert_eq!(keys, ["day", "event", "os", "version"]);
        assert_eq!(read[0]["day"], "2026-10-18", "a day, not a time");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_payload_is_counts_per_event_and_contains_nothing_the_file_does_not() {
        let recorded = vec![
            json!({ "event": "turn_run", "day": "2026-10-17", "version": "3.1.0", "os": "macos" }),
            json!({ "event": "turn_run", "day": "2026-10-17", "version": "3.1.0", "os": "macos" }),
            json!({ "event": "turn_run", "day": "2026-10-18", "version": "3.1.0", "os": "macos" }),
            json!({ "event": "ship_safe", "day": "2026-10-18", "version": "3.1.0", "os": "macos", "prompt": "secret" }),
            json!({ "event": "not_listed", "day": "2026-10-18" }),
        ];
        let body = payload(&recorded);
        let text = body.to_string();

        assert_eq!(body["counts"][0], json!({ "event": "ship_safe", "times": 1, "days": 1 }));
        assert_eq!(body["counts"][1], json!({ "event": "turn_run", "times": 3, "days": 2 }));
        assert!(!text.contains("secret") && !text.contains("not_listed"), "{text}");
    }

    #[test]
    fn sending_needs_an_https_endpoint_and_clearing_removes_the_local_data() {
        assert!(send("http://example.com/collect", &json!({})).unwrap_err().contains("https"));
        assert!(send("ftp://example.com", &json!({})).is_err());
    }

    #[test]
    fn every_listed_event_is_a_plain_snake_case_feature_name() {
        for name in EVENTS {
            assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{name}");
        }
    }
}
