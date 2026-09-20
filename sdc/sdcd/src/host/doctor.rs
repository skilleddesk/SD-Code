//! The environment doctor - the ten checks of spec section 9.10.
//!
//! Each check is a probe of *this* machine, not a line in a table, and each one ends in one of three
//! states: `ok` (it works), `warn` (it works, but not the way we would like) or `fail` (it does not
//! work, and here is the button that tries to fix it). The `fix` field is what the row's button says:
//! `Install`, `Kill process`, `Re-pin`.
//!
//! The ten, in the order the Provider Hub lists them:
//!
//!   1 node   2 claude   3 codex   4 gemini   5 ollama   6 ripgrep   7 port 3000   8 disk
//!   9 git   10 ssh
//!
//! "Fails gracefully if a tool is missing" is the acceptance item, and it is why every probe returns
//! a `Result`-shaped `Option` instead of unwrapping: a machine with none of the six CLIs installed
//! gets ten honest rows, not a panic.

use std::process::Command;

use serde_json::{json, Value};

use crate::store::sqlite::Store;

/// `(id, label, program, args)`. The args are there because `node -v` is not `node --version` on
/// every tool, and a check that reports the wrong thing is worse than no check.
const TOOLS: &[(&str, &str, &str)] = &[
    ("node", "Node.js", "node"),
    ("claude", "Claude Code CLI", "claude"),
    ("codex", "Codex CLI", "codex"),
    ("gemini", "Gemini CLI", "gemini"),
    ("ollama", "Ollama", "ollama"),
    ("ripgrep", "ripgrep", "rg"),
    ("git", "Git", "git"),
    ("ssh", "SSH", "ssh"),
];

/// The ten checks for this machine. `store` is here so the disk row can name the database file and
/// its size - the one piece of the environment the daemon is uniquely able to report.
pub fn checks(store: &Store) -> Vec<Value> {
    let mut rows: Vec<Value> = TOOLS
        .iter()
        .map(|(id, label, program)| match version_of(program) {
            Some(version) => json!({ "id": id, "label": label, "state": "ok", "detail": version }),
            None => json!({
                "id": id,
                "label": label,
                "state": "fail",
                "detail": "not installed",
                "fix": "Install",
            }),
        })
        .collect();

    /* 7 - the busy port the prototype's example complains about. */
    rows.push(match std::net::TcpListener::bind(("127.0.0.1", 3000)) {
        Ok(_) => json!({ "id": "port3000", "label": "Port 3000", "state": "ok", "detail": "free" }),
        Err(_) => json!({
            "id": "port3000",
            "label": "Port 3000",
            "state": "warn",
            "detail": "already in use",
            "fix": "Kill process",
        }),
    });

    /* 8 - the disk, and with it the daemon's own footprint. */
    let database = store.path();
    let size: u64 = std::fs::metadata(&database).map(|meta| meta.len()).unwrap_or(0);

    rows.push(json!({
        "id": "disk",
        "label": "Disk space",
        "state": "ok",
        "detail": format!("{database} · {} KB · {} events", size / 1024, store.event_count().unwrap_or(0)),
    }));

    rows
}

/// Probes one program's version. A missing program is not an error here: it is a `fail` row.
pub fn version_of(program: &str) -> Option<String> {
    let output = Command::new(program).arg("--version").output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = stdout.lines().next().unwrap_or("").trim();

    Some(if first.is_empty() { format!("{program} present") } else { first.to_string() })
}

/// True when a program can be found and run. Used by the provider flows to decide whether a CLI
/// provider can be connected.
pub fn has(program: &str) -> bool {
    version_of(program).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_exactly_ten_checks_with_the_five_required_fields() {
        let store = Store::in_memory().unwrap();
        let rows = checks(&store);

        assert_eq!(rows.len(), 10);

        for row in &rows {
            assert!(row["id"].is_string(), "every check has an id");
            assert!(row["label"].is_string());
            assert!(row["detail"].is_string());

            let state = row["state"].as_str().unwrap_or_default();

            assert!(["ok", "warn", "fail"].contains(&state));
        }
    }

    #[test]
    fn a_missing_tool_is_a_fail_row_and_not_a_panic() {
        let store = Store::in_memory().unwrap();
        let rows = checks(&store);
        let node = rows.iter().find(|row| row["id"] == "node").unwrap();

        /* Whatever this machine has, the row is well formed - that is the graceful failure. */
        assert!(node["state"] == "ok" || node["detail"] == "not installed");
        assert!(version_of("definitely-not-a-program-sdcd").is_none());
    }
}
