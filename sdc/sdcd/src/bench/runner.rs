//! The bench runner (v3 W1): every task × model × run in a fresh copy of its fixture, headless, through a daemon
//! of its own (a scratch database, the ledger on), with step and time caps, then verified.
//!
//! The daemon is the binary under test (`--binary`, default: this one), so `bench/bin/v1/sdcd` can be measured
//! with the same runner. A binary without the ledger still reports its totals (`CostUpdated`); only the
//! per-part breakdown and the cached count are then missing, and the record says so (`ledger: false`).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ledger;
use super::task::{self, Task};

/// A model under test: `provider:model`, e.g. `deepseek:deepseek-chat`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub provider: String,
    pub model: String,
}

impl Model {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.split_once(':') {
            Some((provider, model)) if !provider.trim().is_empty() && !model.trim().is_empty() => Ok(Self { provider: provider.trim().into(), model: model.trim().into() }),
            _ => Err(format!("`{text}`: a model is written provider:model, e.g. deepseek:deepseek-chat")),
        }
    }

    pub fn label(&self) -> String {
        format!("{}:{}", self.provider, self.model)
    }

    pub fn slug(&self) -> String {
        self.label().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect()
    }
}

/// One run's result: a line of `runs.jsonl`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub label: String,
    /// `sdc` for SDC, `claude-code` / `codex` for the competitors.
    pub agent: String,
    pub ablate: Option<String>,
    pub task: String,
    pub kind: String,
    pub language: String,
    pub repeat_of: Option<String>,
    pub model: String,
    pub run: usize,
    pub pass: bool,
    pub failures: Vec<String>,
    /// Why the run did not happen (missing tool, budget reached); a skipped run has no numbers.
    pub skipped: Option<String>,
    pub error: Option<String>,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    pub billable_equiv: f64,
    pub usd: f64,
    pub wall_ms: u64,
    pub ttft_ms: Option<u64>,
    pub steps: u64,
    pub requests: u64,
    pub tool_calls: BTreeMap<String, u64>,
    /// The input split into `ledger::PARTS`, summed over the run's requests.
    pub parts: [u64; 6],
    /// Whether the numbers came from the ledger (true) or only from the daemon's totals.
    pub ledger: bool,
    /// File-read results sent to the model (SDC's count), and of them re-reads of a path read before (A-10).
    #[serde(default)]
    pub read_tokens: u64,
    #[serde(default)]
    pub repeat_read_tokens: u64,
    pub answer: String,
}

/// `uncached_in + 0.1 * cached_in + out * (price_out / price_in)` - the plan's billable-equivalent tokens.
pub fn billable(input: u64, cached: u64, output: u64, price: Option<(f64, f64)>) -> f64 {
    let ratio = match price {
        Some((input_price, output_price)) if input_price > 0.0 => output_price / input_price,
        _ => 4.0,
    };

    input.saturating_sub(cached) as f64 + 0.1 * cached as f64 + output as f64 * ratio
}

/// Dollars for a run at catalogue prices (per million tokens); cached input at a tenth of the input price.
pub fn usd(input: u64, cached: u64, output: u64, price: Option<(f64, f64)>) -> f64 {
    let Some((input_price, output_price)) = price else {
        return 0.0;
    };

    (input.saturating_sub(cached) as f64 + 0.1 * cached as f64) * input_price / 1e6 + output as f64 * output_price / 1e6
}

/// Sums a turn's ledger records into a run.
pub fn fold_ledger(records: &[ledger::Record], record: &mut RunRecord) {
    for read in records.iter().flat_map(|entry| &entry.reads) {
        record.read_tokens += read.tokens;

        if read.repeat {
            record.repeat_read_tokens += read.tokens;
        }
    }

    for entry in records.iter().filter(|entry| !entry.role.ends_with("-reads")) {
        record.input_tokens += entry.input_tokens;
        record.cached_tokens += entry.cached_tokens;
        record.output_tokens += entry.output_tokens;
        record.requests += 1;

        if entry.role == "main" {
            record.steps += 1;
        }

        for (sum, part) in record.parts.iter_mut().zip(entry.parts) {
            *sum += part;
        }

        for tool in &entry.tools_called {
            *record.tool_calls.entry(tool.clone()).or_default() += 1;
        }
    }

    record.ledger = !records.is_empty();
}

/// A fresh copy of a task's fixture, committed so a read-only task can be checked with `git status`.
pub fn prepare(task: &Task, bench: &Path, label: &str, slug: &str, run: usize) -> Result<PathBuf, String> {
    let work = std::env::temp_dir().join("sdc-bench").join(label).join(format!("{}-{slug}-{run}", task.id));
    let _ = std::fs::remove_dir_all(&work);

    task::copy_tree(&bench.join("fixtures").join(&task.fixture), &work)?;

    let work = std::fs::canonicalize(&work).map_err(|error| error.to_string())?;

    task::git(&work, &["init", "-q"])?;
    task::git(&work, &["add", "-A"])?;
    task::git(&work, &["-c", "user.email=bench@sdc.local", "-c", "user.name=sdc-bench", "commit", "-qm", "fixture", "--no-gpg-sign"])?;

    Ok(work)
}

/// Removes a run's folder and the shadow repository SDC kept for it.
pub fn clean(work: &Path) {
    if let Ok(shadow) = crate::git::shadow_path(work) {
        let _ = std::fs::remove_dir_all(shadow);
    }

    let _ = std::fs::remove_dir_all(work);
}

/// Is every tool a task needs on this machine?
pub fn missing_requirement(task: &Task) -> Option<String> {
    task.requires.iter().find(|tool| Command::new(tool.as_str()).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|status| !status.success()).unwrap_or(true)).map(|tool| format!("needs `{tool}`, which this machine lacks"))
}

/// The bench's own daemon.
pub struct Daemon {
    child: Child,
    pub port: u16,
    pub ledger_file: PathBuf,
    scratch: PathBuf,
}

impl Daemon {
    pub fn start(binary: &Path, scratch: &Path, ablate: Option<&str>, data_dir: Option<&Path>) -> Result<Self, String> {
        std::fs::create_dir_all(scratch).map_err(|error| error.to_string())?;

        let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|listener| listener.local_addr()).map_err(|error| error.to_string())?.port();
        let ledger_file = scratch.join("ledger.jsonl");
        let mut command = Command::new(binary);

        command
            .args(["--port", &port.to_string(), "--database"])
            .arg(scratch.join("bench.db"))
            .env(ledger::ENV, &ledger_file)
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(scratch.join("daemon.log")).map(Stdio::from).unwrap_or_else(|_| Stdio::null()));

        if let Some(feature) = ablate {
            command.env(super::ABLATE_ENV, feature);
        }

        if let Some(dir) = data_dir {
            command.env("SDC_DATA_DIR", dir);
        }

        let child = command.spawn().map_err(|error| format!("{}: {error}", binary.display()))?;
        let mut daemon = Self { child, port, ledger_file, scratch: scratch.to_path_buf() };
        let started = Instant::now();

        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if started.elapsed() > Duration::from_secs(30) {
                daemon.stop();

                return Err(format!("the bench daemon did not answer on port {port} within 30 s (log: {})", scratch.join("daemon.log").display()));
            }

            if let Ok(Some(status)) = daemon.child.try_wait() {
                return Err(format!("the bench daemon exited at start ({status}); log: {}", scratch.join("daemon.log").display()));
            }

            std::thread::sleep(Duration::from_millis(100));
        }

        Ok(daemon)
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn log(&self) -> PathBuf {
        self.scratch.join("daemon.log")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}

/// SDCP over loopback, with a reader thread so waiting can time out without losing half a line.
pub struct Client {
    writer: TcpStream,
    lines: Receiver<Value>,
    next: u64,
    backlog: Vec<Value>,
}

impl Client {
    pub fn connect(port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|error| error.to_string())?;
        let reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
        let (sender, lines) = channel();

        std::thread::spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else { break };

                if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            }
        });

        Ok(Self { writer: stream, lines, next: 1, backlog: Vec::new() })
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = format!("bench-{}", self.next);

        self.next += 1;

        let line = json!({ "v": crate::SDCP_VERSION, "id": id, "method": method, "params": params }).to_string();

        self.writer.write_all(format!("{line}\n").as_bytes()).map_err(|error| error.to_string())?;

        let deadline = Instant::now() + Duration::from_secs(120);

        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self.lines.recv_timeout(left).map_err(|_| format!("`{method}` got no answer"))?;

            if message["id"] == id.as_str() {
                return match message.get("error").filter(|error| !error.is_null()) {
                    Some(error) => Err(error["message"].as_str().unwrap_or("the daemon refused").to_string()),
                    None => Ok(message["result"].clone()),
                };
            }

            self.backlog.push(message);
        }
    }

    /// The next notification, or `None` when nothing came within `wait`.
    pub fn notification(&mut self, wait: Duration) -> Result<Option<Value>, String> {
        if !self.backlog.is_empty() {
            return Ok(Some(self.backlog.remove(0)));
        }

        match self.lines.recv_timeout(wait) {
            Ok(value) => Ok(Some(value)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err("the bench daemon closed the connection".into()),
        }
    }
}

/// What a turn produced, from its events.
#[derive(Debug, Default)]
pub struct TurnResult {
    pub answer: String,
    pub ttft_ms: Option<u64>,
    pub wall_ms: u64,
    pub reported_input: u64,
    pub reported_output: u64,
    pub tool_cards: BTreeMap<String, u64>,
    pub error: Option<String>,
    pub timed_out: bool,
}

/// Folds one event of the run's turn into its result; answers whether the turn has said its last word.
pub fn absorb(event: &Value, started: Instant, result: &mut TurnResult) -> bool {
    let first_token = |result: &mut TurnResult| {
        if result.ttft_ms.is_none() {
            result.ttft_ms = Some(started.elapsed().as_millis() as u64);
        }
    };

    match event["type"].as_str().unwrap_or_default() {
        "TurnDelta" => {
            first_token(result);
            result.answer.push_str(event["delta"].as_str().unwrap_or_default());
        }
        "ThinkingDelta" | "ToolCallDrafting" => first_token(result),
        "ToolCallStarted" => {
            first_token(result);
            *result.tool_cards.entry(event["tool"].as_str().unwrap_or("tool").to_string()).or_default() += 1;
        }
        "CostUpdated" => {
            result.reported_input = event["inputTokens"].as_u64().unwrap_or(result.reported_input);
            result.reported_output = event["outputTokens"].as_u64().unwrap_or(result.reported_output);
        }
        "ErrorRaised" => {
            result.error = Some(format!("{}: {}", event["title"].as_str().unwrap_or_default(), event["explanation"].as_str().unwrap_or_default()));
        }
        "PolicyViolation" | "BudgetStop" => result.error = Some(event["sentence"].as_str().unwrap_or("stopped by policy").to_string()),
        "TrustScored" => return true,
        _ => {}
    }

    false
}

const NOBODY: &str = "Nobody can answer during this benchmark run. Proceed with your best judgement and finish the task.";

/// Runs one task as one headless agent turn and waits for it to end (or for its time cap).
pub fn run_turn(client: &mut Client, task: &Task, model: &Model, work: &Path) -> Result<TurnResult, String> {
    let root = work.display().to_string();
    let added = client.call("project.add", json!({ "root": root, "hostId": "local" }))?;
    let project = added["projectId"].as_str().or(added["project"]["id"].as_str()).map(str::to_string);
    let session = client.call("session.open", json!({ "hostId": "local", "title": format!("bench {}", task.id), "projectId": project }))?;
    let session = session["sessionId"].as_str().unwrap_or_default().to_string();
    let started = Instant::now();
    let turn = client.call(
        "engine.start",
        json!({
            "sessionId": session,
            "prompt": task.prompt,
            "engine": "native_api",
            "agent": true,
            "autonomy": "auto",
            "model": model.model,
            "provider": model.provider,
            "maxSteps": task.max_steps,
        }),
    )?;
    let turn = turn["turnId"].as_str().unwrap_or_default().to_string();
    let deadline = started + Duration::from_secs(task.timeout_secs);
    let mut result = TurnResult::default();
    let mut completed_at: Option<Instant> = None;
    let mut cancelled_at: Option<Instant> = None;

    loop {
        let now = Instant::now();

        if now > deadline && cancelled_at.is_none() {
            result.timed_out = true;
            cancelled_at = Some(now);
            let _ = client.call("engine.cancel", json!({ "turnId": turn }));
        }

        /* A turn that completed but never scored, or a cancel that never landed, ends the wait anyway. */
        if completed_at.is_some_and(|at| at.elapsed() > Duration::from_secs(20)) || cancelled_at.is_some_and(|at| at.elapsed() > Duration::from_secs(60)) {
            break;
        }

        let Some(message) = client.notification(Duration::from_millis(500))? else {
            continue;
        };
        let event = &message["event"];

        if message["turnId"] != turn.as_str() && event["turnId"] != turn.as_str() {
            continue;
        }

        match event["type"].as_str().unwrap_or_default() {
            "PermissionRequested" => {
                let _ = client.call("permission.resolve", json!({ "permissionId": event["permissionId"], "decision": "deny" }));
            }
            "QuestionAsked" => {
                let _ = client.call("question.answer", json!({ "questionId": event["questionId"], "turnId": turn, "sessionId": session, "answer": NOBODY }));
            }
            "TurnCompleted" => {
                completed_at.get_or_insert_with(Instant::now);
            }
            _ => {}
        }

        if absorb(event, started, &mut result) {
            break;
        }
    }

    result.wall_ms = started.elapsed().as_millis() as u64;

    if result.timed_out {
        result.error.get_or_insert_with(|| format!("stopped at the {} s time cap", task.timeout_secs));
    }

    Ok(result)
}

/// The prices the catalogue lists for a model (per million tokens).
pub fn price(model: &Model) -> Option<(f64, f64)> {
    crate::agent::price_of(&model.provider, crate::engines::native_api::api_model(&model.model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_are_written_provider_colon_model() {
        assert_eq!(Model::parse("deepseek:deepseek-chat").unwrap(), Model { provider: "deepseek".into(), model: "deepseek-chat".into() });
        assert_eq!(Model::parse("openrouter:qwen/qwen3-coder").unwrap().model, "qwen/qwen3-coder");
        assert!(Model::parse("deepseek-chat").is_err());
        assert_eq!(Model::parse("a:b/c").unwrap().slug(), "a_b_c");
    }

    #[test]
    fn billable_equivalent_and_dollars_follow_the_plans_formula() {
        /* 10k in of which 8k cached, 1k out, $1 / $4: 2,000 + 800 + 4,000. */
        assert!((billable(10_000, 8_000, 1_000, Some((1.0, 4.0))) - 6_800.0).abs() < 1e-9);
        assert!((usd(10_000, 8_000, 1_000, Some((1.0, 4.0))) - (2_800.0 / 1e6 + 4_000.0 / 1e6)).abs() < 1e-12);
        assert_eq!(usd(1, 0, 1, None), 0.0);
    }

    #[test]
    fn a_turns_ledger_folds_into_one_run() {
        let records = vec![
            ledger::Record { role: "main".into(), input_tokens: 100, cached_tokens: 0, output_tokens: 5, tools_called: vec!["read_file".into(), "grep".into()], parts: [10, 20, 0, 30, 0, 40], ..Default::default() },
            ledger::Record { role: "sub".into(), input_tokens: 50, cached_tokens: 10, output_tokens: 5, tools_called: vec!["read_file".into()], parts: [5, 5, 40, 0, 0, 0], ..Default::default() },
            ledger::Record {
                role: "main-reads".into(),
                reads: vec![
                    ledger::FileRead { path: "a.rs".into(), tokens: 300, repeat: false, deduped: false },
                    ledger::FileRead { path: "a.rs".into(), tokens: 280, repeat: true, deduped: false },
                ],
                ..Default::default()
            },
        ];
        let mut run = RunRecord::default();

        fold_ledger(&records, &mut run);

        assert_eq!((run.input_tokens, run.cached_tokens, run.output_tokens, run.steps, run.requests), (150, 10, 10, 1, 2));
        assert_eq!(run.parts, [15, 25, 40, 30, 0, 40]);
        assert_eq!(run.parts.iter().sum::<u64>(), run.input_tokens, "the ledger total is the provider's");
        assert_eq!(run.tool_calls["read_file"], 2);
        assert_eq!((run.read_tokens, run.repeat_read_tokens), (580, 280));
        assert!(run.ledger);
    }

    #[test]
    fn events_make_the_answer_the_first_token_time_and_the_end() {
        let started = Instant::now();
        let mut result = TurnResult::default();

        assert!(!absorb(&json!({ "type": "ToolCallStarted", "tool": "read" }), started, &mut result));
        assert!(!absorb(&json!({ "type": "TurnDelta", "delta": "Hello " }), started, &mut result));
        assert!(!absorb(&json!({ "type": "TurnDelta", "delta": "world" }), started, &mut result));
        assert!(!absorb(&json!({ "type": "CostUpdated", "inputTokens": 900, "outputTokens": 30 }), started, &mut result));
        assert!(absorb(&json!({ "type": "TrustScored", "score": 90 }), started, &mut result));

        assert_eq!(result.answer, "Hello world");
        assert!(result.ttft_ms.is_some());
        assert_eq!((result.reported_input, result.reported_output), (900, 30));
        assert_eq!(result.tool_cards["read"], 1);
    }
}
