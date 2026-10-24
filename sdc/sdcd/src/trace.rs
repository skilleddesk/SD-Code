//! **Step Trace and Speed Gate (P0-12, switch `trace.steps`)**: where the time and the tokens of a turn went, kept as
//! rows, so a claim about speed or about which model fails which step is a query and not an impression.
//!
//! One row per model call ([`Step`]): the task (turn) and step, the model key and the mode it ran under, tokens in and
//! out, thinking tokens, time to first token, tokens per second, wall time, the verify result once there is one, the
//! cost, and why it escalated when it did. Rows are written by one background thread behind a channel, so recording
//! a step costs the step a channel send (microseconds) and never a disk sync: the plan asks for under a millisecond.
//!
//! The Speed Gate ([`gate`]) is the other half: single-step work must not get more than 5% slower than the last
//! release (median and p90), and a question the Code Graph or git can answer must make no model call at all.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::sqlite::Store;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub task_id: String,
    pub step_id: String,
    pub session_id: String,
    pub step: u32,
    pub model_key: String,
    pub mode: String,
    pub purpose: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub thinking_tokens: u64,
    pub ttft_ms: u64,
    pub tok_per_s: f64,
    pub wall_ms: u64,
    pub verify: Option<String>,
    pub cost_usd: Option<f64>,
    pub escalation_reason: Option<String>,
    /// Input tokens the provider's prompt cache served (0 when it said nothing, or nothing was cached): the rest of the input was a miss.
    pub cached_tokens: u64,
}

impl Step {
    /// Tokens per second of the *generating* part: output tokens over the time after the first token.
    pub fn rate(tokens_out: u64, ttft_ms: u64, wall_ms: u64) -> f64 {
        let generating = wall_ms.saturating_sub(ttft_ms).max(1);

        tokens_out as f64 * 1000.0 / generating as f64
    }
}

enum Message {
    Row(Box<Step>, String),
    Verify { session_id: String, verdict: String },
    Flush(Sender<()>),
}

struct Writer {
    sender: Mutex<Sender<Message>>,
}

fn writer() -> &'static OnceLock<Writer> {
    static WRITER: OnceLock<Writer> = OnceLock::new();

    &WRITER
}

/// Starts the writer thread on the daemon's store. Called once, beside the journal's own install.
pub fn install(store: Arc<Store>) {
    let (sender, receiver) = channel::<Message>();

    if writer().set(Writer { sender: Mutex::new(sender) }).is_err() {
        return;
    }

    std::thread::Builder::new()
        .name("sdc-step-trace".into())
        .spawn(move || {
            while let Ok(message) = receiver.recv() {
                match message {
                    Message::Row(step, at) => {
                        let _ = insert(&store, &step, &at);
                    }
                    Message::Verify { session_id, verdict } => {
                        let _ = set_verify(&store, &session_id, &verdict);
                    }
                    Message::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        })
        .ok();
}

fn send(message: Message) {
    if let Some(writer) = writer().get() {
        if let Ok(sender) = writer.sender.lock() {
            let _ = sender.send(message);
        }
    }
}

/// Records one model call. A no-op with the switch off or before the daemon installed the writer.
pub fn record(step: Step) {
    if !crate::v3::on("trace.steps") {
        return;
    }

    send(Message::Row(Box::new(step), chrono::Utc::now().to_rfc3339()));
}

/// The verify verdict of a session's newest traced step (`PASS`, `FAIL`, ...), once Verify has run.
pub fn record_verify(session_id: &str, verdict: &str) {
    if crate::v3::on("trace.steps") {
        send(Message::Verify { session_id: session_id.to_string(), verdict: verdict.to_string() });
    }
}

/// Waits until everything sent so far is written (tests, and a clean shutdown).
pub fn flush() {
    let (done, wait) = channel();

    send(Message::Flush(done));

    let _ = wait.recv_timeout(std::time::Duration::from_secs(5));
}

fn insert(store: &Store, step: &Step, at: &str) -> rusqlite::Result<usize> {
    let connection = store.connection.lock().unwrap_or_else(|poison| poison.into_inner());

    connection.execute(
        "INSERT INTO step_trace (task_id, step_id, session_id, step, model_key, mode, purpose, tokens_in, tokens_out, thinking_tokens, ttft_ms, tok_per_s, wall_ms, verify, cost_usd, escalation_reason, created_at, cached_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            step.task_id, step.step_id, step.session_id, step.step, step.model_key, step.mode, step.purpose, step.tokens_in as i64, step.tokens_out as i64, step.thinking_tokens as i64, step.ttft_ms as i64,
            step.tok_per_s, step.wall_ms as i64, step.verify, step.cost_usd, step.escalation_reason, at, step.cached_tokens as i64
        ],
    )
}

fn set_verify(store: &Store, session_id: &str, verdict: &str) -> rusqlite::Result<usize> {
    let connection = store.connection.lock().unwrap_or_else(|poison| poison.into_inner());

    connection.execute(
        "UPDATE step_trace SET verify = ?2 WHERE id = (SELECT id FROM step_trace WHERE session_id = ?1 ORDER BY id DESC LIMIT 1)",
        params![session_id, verdict],
    )
}

/// The newest `limit` steps, oldest first - of one session when given, else of everything.
pub fn recent(store: &Store, session_id: Option<&str>, limit: usize) -> Vec<Value> {
    let connection = store.connection.lock().unwrap_or_else(|poison| poison.into_inner());
    let columns = "task_id, step_id, session_id, step, model_key, mode, purpose, tokens_in, tokens_out, thinking_tokens, ttft_ms, tok_per_s, wall_ms, verify, cost_usd, escalation_reason, created_at, cached_tokens";
    let sql = format!("SELECT {columns} FROM step_trace WHERE (?1 = '' OR session_id = ?1) ORDER BY id DESC LIMIT ?2");
    let Ok(mut statement) = connection.prepare(&sql) else {
        return Vec::new();
    };
    let rows = statement.query_map(params![session_id.unwrap_or(""), limit as i64], |row| {
        Ok(json!({
            "taskId": row.get::<_, String>(0)?, "stepId": row.get::<_, String>(1)?, "sessionId": row.get::<_, String>(2)?, "step": row.get::<_, i64>(3)?,
            "modelKey": row.get::<_, String>(4)?, "mode": row.get::<_, String>(5)?, "purpose": row.get::<_, String>(6)?,
            "tokensIn": row.get::<_, i64>(7)?, "tokensOut": row.get::<_, i64>(8)?, "thinkingTokens": row.get::<_, i64>(9)?,
            "ttftMs": row.get::<_, i64>(10)?, "tokPerS": row.get::<_, f64>(11)?, "wallMs": row.get::<_, i64>(12)?,
            "verify": row.get::<_, Option<String>>(13)?, "costUsd": row.get::<_, Option<f64>>(14)?, "escalationReason": row.get::<_, Option<String>>(15)?,
            "at": row.get::<_, String>(16)?, "cachedTokens": row.get::<_, i64>(17)?,
        }))
    });
    let mut out: Vec<Value> = rows.map(|rows| rows.flatten().collect()).unwrap_or_default();

    out.reverse();

    out
}

// ---------------------------------------------------------------------------------------------------------------
// The receipt (P0-15)
// ---------------------------------------------------------------------------------------------------------------

/// A step ran on this machine when its model key names a local runtime.
fn is_local(model_key: &str) -> bool {
    model_key.starts_with("ollama:") || model_key.starts_with("local:") || model_key.starts_with("vllm:")
}

/// A model under one purpose, as a receipt row is keyed.
type ReceiptKey = (String, String, String);
/// What the steps of that row add up to: steps, tokens in, tokens out, wall ms, and cost (no price -> `None`).
type ReceiptSum = (u64, u64, u64, u64, Option<f64>);

/// The receipt of a turn, **computed from its Step Trace rows and nothing else**, so it cannot disagree with the trace:
/// one row per model (and purpose), what each cost in tokens, time and money, whether it was local or a Rotation Chain
/// step, and the totals. A cost is `null` for a model with no price (a subscription, a local model), never a made-up zero.
pub fn receipt(store: &Store, turn_id: &str) -> Value {
    let connection = store.connection.lock().unwrap_or_else(|poison| poison.into_inner());
    let mut rows: std::collections::BTreeMap<ReceiptKey, ReceiptSum> = std::collections::BTreeMap::new();

    if let Ok(mut statement) = connection.prepare("SELECT model_key, mode, purpose, tokens_in, tokens_out, wall_ms, cost_usd FROM step_trace WHERE task_id = ?1 ORDER BY id") {
        let found = statement.query_map(params![turn_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)? as u64, row.get::<_, i64>(4)? as u64, row.get::<_, i64>(5)? as u64, row.get::<_, Option<f64>>(6)?)));

        for (model, mode, purpose, tokens_in, tokens_out, wall_ms, cost) in found.into_iter().flatten().flatten() {
            let entry = rows.entry((model, mode, purpose)).or_default();

            entry.0 += 1;
            entry.1 += tokens_in;
            entry.2 += tokens_out;
            entry.3 += wall_ms;
            entry.4 = match (entry.4, cost) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
            };
        }
    }

    let (mut steps, mut tokens_in, mut tokens_out, mut wall_ms, mut local_steps, mut chain_steps) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut cost: Option<f64> = None;
    let mut out = Vec::new();

    for ((model, mode, purpose), (count, input, output, wall, row_cost)) in rows {
        let local = is_local(&model);
        let chain = purpose == "chain";

        steps += count;
        tokens_in += input;
        tokens_out += output;
        wall_ms += wall;
        local_steps += if local { count } else { 0 };
        chain_steps += if chain { count } else { 0 };
        cost = match (cost, row_cost) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
        };

        out.push(json!({ "modelKey": model, "mode": mode, "purpose": purpose, "local": local, "chain": chain, "steps": count, "tokensIn": input, "tokensOut": output, "wallMs": wall, "costUsd": row_cost }));
    }

    json!({
        "turnId": turn_id,
        "rows": out,
        "totals": { "steps": steps, "tokensIn": tokens_in, "tokensOut": tokens_out, "wallMs": wall_ms, "costUsd": cost, "localSteps": local_steps, "chainSteps": chain_steps },
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The Speed Gate
// ---------------------------------------------------------------------------------------------------------------

/// What the gate measures for one model: single-step work, median and p90 wall time in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    pub model: String,
    pub runs: usize,
    pub median_s: f64,
    pub p90_s: f64,
}

/// A run, reduced to what the gate reads.
#[derive(Debug, Clone)]
pub struct Sample {
    pub task: String,
    pub model: String,
    pub wall_s: f64,
    pub steps: u64,
    pub requests: u64,
    /// The task is a question the Code Graph or git answers: it must make no model call.
    pub zero_model: bool,
}

/// The longest a "single step" task is: at most this many agent steps.
pub const SINGLE_STEP: u64 = 2;
/// The plan's tolerance: more than 5% slower fails the release.
pub const TOLERANCE: f64 = 0.05;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }

    let rank = (p * (sorted.len() - 1) as f64).round() as usize;

    sorted[rank.min(sorted.len() - 1)]
}

/// Median and p90 of each model's single-step runs.
pub fn timings(samples: &[Sample]) -> Vec<Timing> {
    let mut models: Vec<&str> = samples.iter().map(|sample| sample.model.as_str()).collect();

    models.sort_unstable();
    models.dedup();

    models
        .into_iter()
        .filter_map(|model| {
            let mut walls: Vec<f64> = samples.iter().filter(|sample| sample.model == model && !sample.zero_model && sample.steps <= SINGLE_STEP).map(|sample| sample.wall_s).collect();

            if walls.is_empty() {
                return None;
            }

            walls.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            Some(Timing { model: model.to_string(), runs: walls.len(), median_s: percentile(&walls, 0.5), p90_s: percentile(&walls, 0.9) })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub pass: bool,
    /// One line per problem, and one per model checked, so a green gate still says what it measured.
    pub lines: Vec<String>,
}

/// The gate: for every model in both sets, the candidate's median and p90 may not exceed the baseline's by more than
/// `tolerance`; and every zero-model task must have made no model call.
pub fn gate(baseline: &[Sample], candidate: &[Sample], tolerance: f64) -> Verdict {
    let (before, after) = (timings(baseline), timings(candidate));
    let mut lines = Vec::new();
    let mut pass = true;

    for new in &after {
        let Some(old) = before.iter().find(|old| old.model == new.model) else {
            lines.push(format!("{}: no baseline to compare with ({} runs)", new.model, new.runs));
            continue;
        };
        let slower = |new: f64, old: f64| old > 0.0 && new > old * (1.0 + tolerance);
        let (median_bad, p90_bad) = (slower(new.median_s, old.median_s), slower(new.p90_s, old.p90_s));

        if median_bad || p90_bad {
            pass = false;
        }

        lines.push(format!(
            "{}: median {:.2}s -> {:.2}s ({:+.1}%), p90 {:.2}s -> {:.2}s ({:+.1}%) over {} runs - {}",
            new.model,
            old.median_s,
            new.median_s,
            (new.median_s / old.median_s.max(1e-9) - 1.0) * 100.0,
            old.p90_s,
            new.p90_s,
            (new.p90_s / old.p90_s.max(1e-9) - 1.0) * 100.0,
            new.runs,
            if median_bad || p90_bad { "TOO SLOW" } else { "ok" }
        ));
    }

    for sample in candidate.iter().filter(|sample| sample.zero_model && sample.requests > 0) {
        pass = false;
        lines.push(format!("{}: answerable without a model, but made {} model call(s)", sample.task, sample.requests));
    }

    if after.is_empty() {
        lines.push("no single-step runs to measure".to_string());
    }

    Verdict { pass, lines }
}

/// The one store every test shares for the writer thread (the writer is installed once per process).
#[cfg(test)]
pub fn test_store() -> Arc<Store> {
    static STORE: OnceLock<Arc<Store>> = OnceLock::new();

    STORE
        .get_or_init(|| {
            let store = Arc::new(Store::in_memory().unwrap());

            install(store.clone());

            store
        })
        .clone()
}

#[cfg(test)]
mod tests;
