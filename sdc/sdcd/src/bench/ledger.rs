//! **The context ledger** (v3 W1): what each model request was made of, recorded beside the request rather
//! than inside it - the request itself is byte-for-byte what it was without the ledger.
//!
//! Every step of the agent loop (and of a `task` sub-agent) appends one [`Record`]: the provider's own usage
//! (input, cached, output), the model time, the tools the reply called, and the input split into six parts -
//! system prompt, tool schemas, context SDC added (folder map, memory, notes), history, tool results and the
//! person's words. SDC can only *count* the parts (`context::tokens_of`); the provider only reports the total.
//! The parts are therefore apportioned to the provider's total by their counted share (largest remainder),
//! so the ledger always sums to exactly what the provider billed - the shares are an estimate, the total is not.
//!
//! The ledger is off unless `SDC_BENCH_LEDGER` names a file (the bench daemon sets it) or a test captures a
//! turn ([`capture`]). Off, nothing is computed.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::dialect::ToolSpec;

/// The environment variable that turns the ledger on: a JSONL file every record is appended to.
pub const ENV: &str = "SDC_BENCH_LEDGER";

/// The six parts of a request's input, in the order the plan reports them.
pub const PARTS: [&str; 6] = ["system", "tools", "context", "history", "tool_results", "user"];

/// The marker SDC's prompt wrapper puts before the person's own words: what comes before it is SDC's.
const PERSON_MARK: &str = "[The person's message]";

/// One model request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub turn_id: String,
    /// `main` for the turn's own loop, `sub` for a `task` sub-agent.
    pub role: String,
    pub step: usize,
    pub model: String,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    pub model_ms: u64,
    /// The tool calls the reply made, by tool name.
    pub tools_called: Vec<String>,
    /// SDC's count of each part (`PARTS` order).
    pub counted: [u64; 6],
    /// The provider's input tokens, apportioned to the parts (`PARTS` order); sums to `input_tokens`.
    pub parts: [u64; 6],
    /// For a `reads` record (no request of its own): the file reads one step handed back (A-10).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<FileRead>,
}

/// One `read_file` result as it went back to the model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileRead {
    pub path: String,
    /// SDC's count of what was sent (after an unchanged re-read became one line).
    pub tokens: u64,
    /// The path was read earlier in the same turn.
    pub repeat: bool,
    /// The re-read was unchanged and went back as one line.
    pub deduped: bool,
}

/// Writes the reads one step handed back, as a `reads` record (no tokens of its own).
pub fn record_reads(turn_id: &str, role: &str, step: usize, reads: Vec<FileRead>) {
    if reads.is_empty() {
        return;
    }

    write(Record { turn_id: turn_id.to_string(), role: format!("{role}-reads"), step, reads, ..Default::default() });
}

/// What a request was made of, before the provider answered.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Shape {
    pub counted: [u64; 6],
}

fn captures() -> &'static Mutex<HashMap<String, Vec<Record>>> {
    static CAPTURES: OnceLock<Mutex<HashMap<String, Vec<Record>>>> = OnceLock::new();

    CAPTURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file() -> Option<String> {
    std::env::var(ENV).ok().filter(|path| !path.trim().is_empty())
}

/// Starts keeping a turn's records in memory (tests); [`take`] hands them back.
pub fn capture(turn_id: &str) {
    captures().lock().unwrap_or_else(|poison| poison.into_inner()).insert(turn_id.to_string(), Vec::new());
}

/// The records kept for a captured turn, and the capture ends.
pub fn take(turn_id: &str) -> Vec<Record> {
    captures().lock().unwrap_or_else(|poison| poison.into_inner()).remove(turn_id).unwrap_or_default()
}

/// Is anything listening for this turn? When not, the loop skips the counting entirely.
pub fn enabled(turn_id: &str) -> bool {
    file().is_some() || captures().lock().unwrap_or_else(|poison| poison.into_inner()).contains_key(turn_id)
}

/// Counts the parts of one request. `history_len` is how many messages at the start are earlier turns, and
/// `context_chars` how many characters at the start of the first new user message SDC put there (the folder map).
pub fn shape(system: &str, specs: &[ToolSpec], messages: &[Value], history_len: usize, context_chars: usize) -> Shape {
    let count = crate::context::tokens_of;
    let mut counted = [0u64; 6];

    counted[0] = count(system);
    counted[1] = specs.iter().map(|spec| count(spec.name) + count(spec.description) + count(&spec.schema.to_string())).sum();

    for (index, message) in messages.iter().enumerate() {
        let role = message["role"].as_str().unwrap_or_default();

        if index < history_len {
            counted[3] += value_tokens(message);
            continue;
        }

        match role {
            "assistant" => counted[3] += value_tokens(message),
            "tool" => counted[4] += value_tokens(&message["content"]),
            "user" if index == history_len => {
                let (context, user) = split_first_message(message, context_chars);

                counted[2] += context;
                counted[5] += user;
            }
            /* After the first message a user message is a step's tool results (Anthropic) and SDC's notes. */
            "user" => match message["content"].as_array() {
                Some(blocks) => {
                    for block in blocks {
                        if block["type"] == "tool_result" {
                            counted[4] += value_tokens(&block["content"]);
                        } else {
                            counted[2] += value_tokens(block);
                        }
                    }
                }
                None => counted[2] += value_tokens(&message["content"]),
            },
            _ => counted[2] += value_tokens(message),
        }
    }

    Shape { counted }
}

/// The first new user message: the folder map (`context_chars`) and SDC's wrapper before the person's mark are
/// context, the rest is the person's.
fn split_first_message(message: &Value, context_chars: usize) -> (u64, u64) {
    /* An empty piece is nothing (`tokens_of` answers at least 1). */
    let count = |text: &str| if text.is_empty() { 0 } else { crate::context::tokens_of(text) };
    let (text, extra) = match &message["content"] {
        Value::String(text) => (text.clone(), 0),
        Value::Array(blocks) => {
            let text = blocks.iter().filter(|block| block["type"] == "text").filter_map(|block| block["text"].as_str()).collect::<Vec<_>>().join("\n");
            let images = blocks.iter().filter(|block| block["type"] != "text").map(value_tokens).sum();

            (text, images)
        }
        _ => (String::new(), 0),
    };
    let map_end = floor_boundary(&text, context_chars);
    let (map, rest) = text.split_at(map_end);
    let (wrapper, words) = match rest.rfind(PERSON_MARK) {
        Some(at) => rest.split_at(at),
        None => ("", rest),
    };

    (count(map) + count(wrapper), count(words) + extra)
}

fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());

    while !text.is_char_boundary(at) {
        at -= 1;
    }

    at
}

fn value_tokens(value: &Value) -> u64 {
    crate::agent::dialect::tokens(std::slice::from_ref(value))
}

/// Splits `total` over the parts by their counted share; the result sums to `total` exactly (largest remainder).
pub fn apportion(total: u64, counted: &[u64; 6]) -> [u64; 6] {
    let sum: u64 = counted.iter().sum();
    let mut parts = [0u64; 6];

    if total == 0 {
        return parts;
    }

    if sum == 0 {
        parts[5] = total;
        return parts;
    }

    let mut remainders: Vec<(usize, u128)> = Vec::with_capacity(6);
    let mut given = 0u64;

    for (index, &count) in counted.iter().enumerate() {
        let exact = total as u128 * count as u128;

        parts[index] = (exact / sum as u128) as u64;
        given += parts[index];
        remainders.push((index, exact % sum as u128));
    }

    remainders.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    for (index, _) in remainders.into_iter().take((total - given) as usize) {
        parts[index] += 1;
    }

    parts
}

/// Writes one request's record, once the provider has answered.
#[allow(clippy::too_many_arguments)]
pub fn record(turn_id: &str, role: &str, step: usize, model: &str, shape: Shape, reply: &crate::agent::dialect::Reply, model_ms: u64) {
    let record = Record {
        turn_id: turn_id.to_string(),
        role: role.to_string(),
        step,
        model: model.to_string(),
        input_tokens: reply.input_tokens,
        cached_tokens: reply.cached_tokens,
        output_tokens: reply.output_tokens,
        model_ms,
        tools_called: reply.tool_uses.iter().map(|call| call.name.clone()).collect(),
        counted: shape.counted,
        parts: apportion(reply.input_tokens, &shape.counted),
        reads: Vec::new(),
    };

    write(record);
}

fn write(record: Record) {
    let turn_id = record.turn_id.clone();

    if let Some(kept) = captures().lock().unwrap_or_else(|poison| poison.into_inner()).get_mut(&turn_id) {
        kept.push(record.clone());
    }

    if let Some(path) = file() {
        /* One line per write, appended: two turns writing at once interleave whole lines, never halves. */
        if let Ok(line) = serde_json::to_string(&record) {
            if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = out.write_all(format!("{line}\n").as_bytes());
            }
        }
    }
}

/// Reads a ledger file back (the bench runner).
pub fn read(path: &std::path::Path) -> Vec<Record> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apportioned_parts_always_sum_to_the_providers_total() {
        for (total, counted) in [
            (1_000, [10, 20, 30, 40, 50, 60]),
            (7, [1, 1, 1, 1, 1, 1]),
            (123_457, [3_000, 900, 0, 12_000, 40_000, 77]),
            (5, [0, 0, 0, 0, 0, 0]),
            (0, [9, 9, 9, 9, 9, 9]),
        ] {
            let parts = apportion(total, &counted);

            assert_eq!(parts.iter().sum::<u64>(), total, "{counted:?}");
        }

        assert_eq!(apportion(100, &[1, 0, 0, 0, 0, 1]), [50, 0, 0, 0, 0, 50]);
    }

    #[test]
    fn a_request_is_split_into_its_six_parts() {
        let spec = ToolSpec { name: "read_file", description: "Reads a file.", schema: json!({ "type": "object" }) };
        let messages = vec![
            json!({ "role": "user", "content": "an earlier question" }),
            json!({ "role": "assistant", "content": "an earlier answer" }),
            json!({ "role": "user", "content": "FOLDER MAP\n\nSDC notes [The person's message] fix the bug" }),
            json!({ "role": "assistant", "content": null, "tool_calls": [{ "id": "c1", "function": { "name": "read_file", "arguments": "{\"path\":\"a.rs\"}" } }] }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "fn main() {}\n".repeat(20) }),
            json!({ "role": "user", "content": "[SDC: run the checks]" }),
        ];
        let shape = shape("You are SDC.", &[spec], &messages, 2, "FOLDER MAP\n\n".len());

        assert!(shape.counted.iter().all(|&count| count > 0), "every part is present: {:?}", shape.counted);
        assert!(shape.counted[4] > shape.counted[5], "the tool result outweighs the person's four words");

        /* The person's words alone, and SDC's wrapper and map as context. */
        let (context, user) = split_first_message(&messages[2], "FOLDER MAP\n\n".len());

        assert_eq!(user, crate::context::tokens_of("[The person's message] fix the bug"));
        assert_eq!(context, crate::context::tokens_of("FOLDER MAP\n\n") + crate::context::tokens_of("SDC notes "));
    }

    #[test]
    fn anthropic_tool_results_and_notes_are_told_apart() {
        let messages = vec![
            json!({ "role": "user", "content": "do it" }),
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "a long result ".repeat(30), "is_error": false },
                { "type": "text", "text": "[SDC: note]" },
            ] }),
        ];
        let shape = shape("", &[], &messages, 0, 0);

        assert!(shape.counted[4] > 0 && shape.counted[2] > 0);
        assert_eq!(shape.counted[2], crate::context::tokens_of("[SDC: note]") + crate::context::tokens_of("text"));
    }

    #[test]
    fn nothing_is_kept_for_a_turn_nobody_captures() {
        assert!(!captures().lock().unwrap().contains_key("nobody"));

        capture("ledger-unit");
        assert!(enabled("ledger-unit"));

        let reply = crate::agent::dialect::Reply { input_tokens: 90, output_tokens: 10, ..Default::default() };

        record("ledger-unit", "main", 1, "m", Shape { counted: [1, 1, 1, 0, 0, 1] }, &reply, 5);
        record("someone-else", "main", 1, "m", Shape::default(), &reply, 5);

        let kept = take("ledger-unit");

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].parts.iter().sum::<u64>(), 90);
        assert!(take("someone-else").is_empty());
    }
}
