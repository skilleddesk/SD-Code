//! **Privacy split** (v3 I6, cloud part; switch `v3.privacy`): files a project marks `secret_paths` in
//! `.sdc/policy.toml` never reach any model - a read of one is refused, grep and search lines from one are dropped,
//! and a command that names one has its output withheld - and every file whose content did go to a model is
//! written to `.sdc/audit/sent.jsonl` with the provider, the model and the time ("what went where").
//!
//! The local summariser that would send a *summary* of a secret file instead is deferred with local models (D-1).

use std::io::Write;

use serde_json::json;

use crate::agent::dialect::{ToolResult, ToolUse};
use crate::trust::policy::Policy;

pub const AUDIT_FILE: &str = ".sdc/audit/sent.jsonl";

fn refusal(path: &str, pattern: &str) -> String {
    format!("[SDC privacy: {path} is marked secret in .sdc/policy.toml (`{pattern}`). Its content is never sent to a model. Work around it, or ask the person.]")
}

/// The path a grep/search line names (`path:line: text`).
fn line_path(line: &str) -> &str {
    line.split(':').next().unwrap_or_default().trim()
}

/// Withholds secret content from a step's answers. Answers the files whose content was sent (for the audit).
pub fn filter(policy: &Policy, root: &str, calls: &[ToolUse], results: &mut [ToolResult]) -> Vec<String> {
    let mut sent = Vec::new();

    for (call, result) in calls.iter().zip(results.iter_mut()) {
        if result.2 {
            continue;
        }

        match call.name.as_str() {
            "read_file" | "view_image" => {
                let path = call.input["path"].as_str().unwrap_or_default();

                match policy.secret(path, Some(root)) {
                    Some(pattern) => {
                        result.1 = refusal(path, &pattern);
                        result.3 = None;
                    }
                    None => sent.push(path.trim_start_matches("./").to_string()),
                }
            }
            "grep" | "search" => {
                let before = result.1.lines().count();
                let kept: Vec<String> = result.1.lines().filter(|line| policy.secret(line_path(line), Some(root)).is_none()).map(str::to_string).collect();

                sent.extend(kept.iter().map(|line| line_path(line).to_string()).filter(|path| !path.is_empty() && !path.starts_with('[')));

                if kept.len() < before {
                    result.1 = format!("{}\n[SDC privacy: {} line(s) from files marked secret were left out.]", kept.join("\n"), before - kept.len());
                }
            }
            "run_command" | "start_process" => {
                let command = call.input["command"].as_str().unwrap_or_default();

                if let Some((path, pattern)) = command.split_whitespace().find_map(|word| {
                    let word = word.trim_matches(|c: char| c == '"' || c == '\'' || c == ';' || c == '|' || c == '>' || c == '<');

                    policy.secret(word, Some(root)).map(|pattern| (word.to_string(), pattern))
                }) {
                    result.1 = format!("[SDC privacy: this command names {path}, which is marked secret (`{pattern}`); its output is not sent to a model. It ran; check the outcome another way or ask the person.]");
                }
            }
            _ => {}
        }
    }

    sent.sort();
    sent.dedup();
    sent
}

/// Appends what went where to the project's audit file (local projects).
pub fn audit(root: &str, provider: &str, model: &str, files: &[String]) {
    if files.is_empty() {
        return;
    }

    let path = std::path::Path::new(root).join(AUDIT_FILE);

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let line = json!({ "at": chrono::Utc::now().to_rfc3339(), "provider": provider, "model": model, "files": files }).to_string();

    if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(out, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, input: serde_json::Value) -> ToolUse {
        ToolUse { id: name.into(), name: name.into(), input }
    }

    #[test]
    fn secret_files_are_withheld_from_every_tool_answer() {
        let policy = Policy::parse("secret_paths = [\"config/prod/**\", \"*.sql\"]\n", "test");
        let calls = vec![
            call("read_file", json!({ "path": "config/prod/db.yml" })),
            call("read_file", json!({ "path": "src/app.rs" })),
            call("grep", json!({ "pattern": "password" })),
            call("run_command", json!({ "command": "cat dump.sql | head" })),
        ];
        let mut results: Vec<ToolResult> = vec![
            ("1".into(), "password: hunter2".into(), false, None),
            ("2".into(), "fn main() {}".into(), false, None),
            ("3".into(), "config/prod/db.yml:3: password: hunter2\nsrc/app.rs:9: let password = ask();".into(), false, None),
            ("4".into(), "INSERT INTO users VALUES ('hunter2')".into(), false, None),
        ];
        let sent = filter(&policy, "/p", &calls, &mut results);

        assert!(results.iter().all(|result| !result.1.contains("hunter2")), "{results:?}");
        assert!(results[2].1.contains("src/app.rs:9") && results[2].1.contains("1 line(s)"));
        assert_eq!(sent, ["src/app.rs"]);
    }
}
