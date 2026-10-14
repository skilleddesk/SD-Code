//! The competitors on the same tasks (v3 W1): Claude Code (`claude -p --output-format json`) and the Codex CLI
//! (`codex exec --json`), each in its own fresh copy of the fixture, verified exactly like SDC's runs.
//!
//! Both are run with their permission prompts off, because nobody is there to answer - which is why they only
//! ever run inside the throwaway copy `runner::prepare` made. Their versions are recorded with the results.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// What a competitor's run reported.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub answer: String,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    pub steps: u64,
    pub tool_calls: BTreeMap<String, u64>,
    pub reported_usd: Option<f64>,
    pub error: Option<String>,
}

pub const AGENTS: [&str; 2] = ["claude-code", "codex"];

fn program(agent: &str) -> &'static str {
    if agent == "codex" { "codex" } else { "claude" }
}

/// `claude --version` / `codex --version`, for the record.
pub fn version(agent: &str) -> Option<String> {
    let output = Command::new(program(agent)).arg("--version").output().ok()?;

    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The command line for one run.
pub fn command(agent: &str, model: &str, prompt: &str, work: &Path) -> Command {
    let mut command = Command::new(program(agent));

    if agent == "codex" {
        command.args(["exec", "--json", "--full-auto", "--skip-git-repo-check", "-m", model, "-C"]).arg(work).arg(prompt);
    } else {
        command.args(["-p", "--output-format", "json", "--dangerously-skip-permissions", "--model", model]).arg(prompt);
    }

    command.current_dir(work);
    command
}

/// Runs a competitor with a time cap; answers its report and the wall time.
pub fn run(agent: &str, model: &str, prompt: &str, work: &Path, timeout: Duration) -> (Report, u64) {
    let started = Instant::now();
    let mut child = match command(agent, model, prompt, work).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(error) => return (Report { error: Some(format!("{} could not start: {error}", program(agent))), ..Default::default() }, 0),
    };
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let mut timed_out = false;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => break,
        }
    }

    let text = reader.join().unwrap_or_default();
    let mut report = if agent == "codex" { parse_codex(&text) } else { parse_claude(&text) };

    if timed_out {
        report.error.get_or_insert_with(|| format!("stopped at the {} s time cap", timeout.as_secs()));
    }

    (report, started.elapsed().as_millis() as u64)
}

/// Claude Code's `--output-format json`: one result object. Its `input_tokens` excludes the cache, so the
/// whole input is `input + cache_creation + cache_read`, and `cache_read` is what the cache served.
pub fn parse_claude(text: &str) -> Report {
    let value: Value = text
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok().filter(|value| value["type"] == "result"))
        .or_else(|| serde_json::from_str(text.trim()).ok())
        .unwrap_or(Value::Null);

    if value.is_null() {
        return Report { error: Some(format!("no result object in Claude Code's output: {}", text.chars().take(300).collect::<String>())), ..Default::default() };
    }

    let usage = &value["usage"];
    let number = |field: &str| usage[field].as_u64().unwrap_or(0);

    Report {
        answer: value["result"].as_str().unwrap_or_default().to_string(),
        input_tokens: number("input_tokens") + number("cache_creation_input_tokens") + number("cache_read_input_tokens"),
        cached_tokens: number("cache_read_input_tokens"),
        output_tokens: number("output_tokens"),
        steps: value["num_turns"].as_u64().unwrap_or(0),
        tool_calls: BTreeMap::new(),
        reported_usd: value["total_cost_usd"].as_f64(),
        error: (value["is_error"] == true).then(|| value["subtype"].as_str().unwrap_or("error").to_string()),
    }
}

/// The Codex CLI's `exec --json`: one event per line. Usage comes on `turn.completed` (OpenAI's input
/// includes the cached part), the answer is the last `agent_message`, and every other completed item is a tool.
pub fn parse_codex(text: &str) -> Report {
    let mut report = Report::default();

    for value in text.lines().filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok()) {
        match value["type"].as_str().unwrap_or_default() {
            "turn.completed" => {
                let usage = &value["usage"];

                report.input_tokens += usage["input_tokens"].as_u64().unwrap_or(0);
                report.cached_tokens += usage["cached_input_tokens"].as_u64().unwrap_or(0);
                report.output_tokens += usage["output_tokens"].as_u64().unwrap_or(0);
            }
            "item.completed" => {
                let item = &value["item"];
                let kind = item["type"].as_str().or(item["item_type"].as_str()).unwrap_or("item");

                if kind == "agent_message" || kind == "assistant_message" {
                    report.answer = item["text"].as_str().unwrap_or_default().to_string();
                    report.steps += 1;
                } else if kind != "reasoning" {
                    *report.tool_calls.entry(kind.to_string()).or_default() += 1;
                }
            }
            "turn.failed" | "error" => {
                report.error = Some(value["error"]["message"].as_str().or(value["message"].as_str()).unwrap_or("codex reported an error").to_string());
            }
            _ => {}
        }
    }

    if report.input_tokens == 0 && report.answer.is_empty() && report.error.is_none() {
        report.error = Some(format!("no events in Codex's output: {}", text.chars().take(300).collect::<String>()));
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_codes_result_counts_the_cache_inside_the_input() {
        let text = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":6,"result":"Fixed it.","total_cost_usd":0.0421,"usage":{"input_tokens":120,"cache_creation_input_tokens":3000,"cache_read_input_tokens":21000,"output_tokens":800}}"#;
        let report = parse_claude(text);

        assert_eq!((report.input_tokens, report.cached_tokens, report.output_tokens, report.steps), (24_120, 21_000, 800, 6));
        assert_eq!(report.answer, "Fixed it.");
        assert_eq!(report.reported_usd, Some(0.0421));
        assert!(report.error.is_none());
        assert!(parse_claude("not json").error.is_some());
    }

    #[test]
    fn codex_events_add_up_to_one_report() {
        let text = [
            r#"{"type":"thread.started","thread_id":"t"}"#,
            r#"{"type":"item.completed","item":{"id":"1","type":"command_execution","command":"cargo test"}}"#,
            r#"{"type":"item.completed","item":{"id":"2","type":"file_change"}}"#,
            r#"{"type":"item.completed","item":{"id":"3","type":"reasoning","text":"…"}}"#,
            r#"{"type":"item.completed","item":{"id":"4","type":"agent_message","text":"Done: the test passes."}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":5000,"cached_input_tokens":4000,"output_tokens":300}}"#,
        ]
        .join("\n");
        let report = parse_codex(&text);

        assert_eq!((report.input_tokens, report.cached_tokens, report.output_tokens), (5_000, 4_000, 300));
        assert_eq!(report.answer, "Done: the test passes.");
        assert_eq!(report.tool_calls.get("command_execution"), Some(&1));
        assert!(!report.tool_calls.contains_key("reasoning"));
        assert!(parse_codex("").error.is_some());
    }
}
