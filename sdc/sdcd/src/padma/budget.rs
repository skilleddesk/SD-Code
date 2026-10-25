//! **Context Budget Engine (P0-5; switches `padma.obs_cap`, `padma.overflow_rule`)**: the window is never exceeded, and an overflow is never
//! the end of a turn.
//!
//! * `obs_cap`: one tool output may take 10% of the window. What does not fit goes to the *vault* as `‹obs:N›` and the model fetches a part
//!   of it with `obs_read(n, range)`.
//! * Observation masking: the newest `k` (3) tool outputs stay whole; older ones become one line `[tool args → ‹obs:N›, N lines]`. The
//!   model's own words stay. Masking starts when the conversation passes 40% of the window, so a short turn is never rewritten (the
//!   provider's prompt cache only serves an unchanged start).
//! * Overflow Rule, in order: `k` 3→1; the Context Pack is dropped (the older part of the first message, then all of it, the goal stays);
//!   the answer's reserve 4096→1536 for a tool-call step; the unit is split (every old output masked, the tool-output cap halved); and
//!   when nothing fits the unit is `blocked(too_large)` and the turn goes on with the next one.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::agent::dialect::Dialect;

/// Tokens of the window one tool output may take, as characters.
pub fn obs_cap_chars(window: u64) -> usize {
    (window / 10).max(500) as usize * 4
}

/// The answer's reserve for a tool-call step after the third rung of the Overflow Rule.
pub const LOW_RESERVE: u64 = 1_536;

const VAULT_LIMIT: usize = 400;

struct Shelf {
    next: u32,
    items: BTreeMap<u32, String>,
}

fn vault() -> &'static Mutex<HashMap<String, Shelf>> {
    static VAULT: std::sync::OnceLock<Mutex<HashMap<String, Shelf>>> = std::sync::OnceLock::new();

    VAULT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Keeps `text` for the session; answers its number.
pub fn put(session: &str, text: &str) -> u32 {
    let mut vault = vault().lock().unwrap_or_else(|poison| poison.into_inner());
    let shelf = vault.entry(session.to_string()).or_insert(Shelf { next: 1, items: BTreeMap::new() });
    let n = shelf.next;

    shelf.next += 1;
    shelf.items.insert(n, text.to_string());

    while shelf.items.len() > VAULT_LIMIT {
        let oldest = *shelf.items.keys().next().unwrap_or(&0);

        shelf.items.remove(&oldest);
    }

    n
}

pub fn get(session: &str, n: u32) -> Option<String> {
    vault().lock().unwrap_or_else(|poison| poison.into_inner()).get(session).and_then(|shelf| shelf.items.get(&n).cloned())
}

/// The handle the model sees.
pub fn handle(n: u32) -> String {
    format!("‹obs:{n}›")
}

/// `obs_read(n, range)`: `range` is `"from-to"` lines (1-based), `"from"` for a block from there, or empty for the first block.
pub fn read(session: &str, n: u32, range: &str, limit_chars: usize) -> Result<String, String> {
    let Some(text) = get(session, n) else {
        return Err(format!("Nothing is kept under {}. Use the number exactly as it was printed.", handle(n)));
    };
    let lines: Vec<&str> = text.lines().collect();
    let (from, to) = parse_range(range, lines.len());
    let mut out = String::new();
    let mut last = from.saturating_sub(1);

    for (index, line) in lines.iter().enumerate().skip(from.saturating_sub(1)).take(to.saturating_sub(from.saturating_sub(1))) {
        if out.len() + line.len() + 1 > limit_chars && !out.is_empty() {
            break;
        }

        out.push_str(line);
        out.push('\n');
        last = index + 1;
    }

    let more = if last < lines.len() { format!("\n[lines {from}-{last} of {}; read on with range \"{}-\"]", lines.len(), last + 1) } else { format!("\n[lines {from}-{last} of {}; that is the end]", lines.len()) };

    Ok(format!("{}{more}", out.trim_end()))
}

fn parse_range(range: &str, total: usize) -> (usize, usize) {
    let range = range.trim();
    let (a, b) = range.split_once('-').map_or((range, ""), |(a, b)| (a.trim(), b.trim()));
    let from = a.parse::<usize>().unwrap_or(1).max(1);
    let to = b.parse::<usize>().unwrap_or(from + 119).max(from);

    (from.min(total.max(1)), to.min(total.max(1)).min(from + 399))
}

/// Cuts a tool output to `limit` characters and vaults the whole of it: the note says how to get the rest.
pub fn vault_cut(session: &str, text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }

    let n = put(session, text);
    let mut cut = limit.saturating_sub(220).max(200).min(text.len());

    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }

    if let Some(at) = text[..cut].rfind('\n') {
        cut = at;
    }

    let shown = text[..cut].lines().count();

    format!("{}\n\n[{} of {} lines shown; the full output is {} - read a part with obs_read(n: {n}, range: \"{}-{}\")]", text[..cut].trim_end(), shown, text.lines().count(), handle(n), shown + 1, shown + 120)
}

const MASKED_MARK: &str = " → ‹obs:";

fn is_masked(text: &str) -> bool {
    text.starts_with('[') && text.contains(MASKED_MARK)
}

fn one_line(text: &str, limit: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");

    if flat.chars().count() <= limit {
        return flat;
    }

    format!("{}…", flat.chars().take(limit).collect::<String>())
}

/// Each tool result's place, oldest first, with the call that made it: `(message index, block index, tool name, arguments)`.
fn results(dialect: Dialect, messages: &[Value]) -> Vec<(usize, Option<usize>, String, String)> {
    let mut calls: HashMap<String, (String, String)> = HashMap::new();
    let mut places = Vec::new();

    for (index, message) in messages.iter().enumerate() {
        match dialect {
            Dialect::OpenAi => {
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    calls.insert(call["id"].as_str().unwrap_or_default().to_string(), (call["function"]["name"].as_str().unwrap_or("tool").to_string(), call["function"]["arguments"].as_str().unwrap_or_default().to_string()));
                }

                if message["role"] == "tool" {
                    let (name, args) = calls.get(message["tool_call_id"].as_str().unwrap_or_default()).cloned().unwrap_or(("tool".to_string(), String::new()));

                    places.push((index, None, name, args));
                }
            }
            Dialect::Anthropic => {
                for (block, item) in message["content"].as_array().into_iter().flatten().enumerate() {
                    if item["type"] == "tool_use" {
                        calls.insert(item["id"].as_str().unwrap_or_default().to_string(), (item["name"].as_str().unwrap_or("tool").to_string(), item["input"].to_string()));
                    }

                    if item["type"] == "tool_result" {
                        let (name, args) = calls.get(item["tool_use_id"].as_str().unwrap_or_default()).cloned().unwrap_or(("tool".to_string(), String::new()));

                        places.push((index, Some(block), name, args));
                    }
                }
            }
        }
    }

    places
}

fn args_line(args: &str) -> String {
    match serde_json::from_str::<Value>(args) {
        Ok(Value::Object(map)) => one_line(&map.values().map(|value| value.as_str().map_or(value.to_string(), str::to_string)).collect::<Vec<_>>().join(" "), 60),
        _ => one_line(args, 60),
    }
}

/// Observation masking: all but the newest `keep` tool outputs of more than a few lines become a one-line handle. Answers how many were masked.
pub fn mask_old(dialect: Dialect, messages: &mut [Value], keep: usize, session: &str) -> usize {
    let places = results(dialect, messages);
    let until = places.len().saturating_sub(keep);
    let mut masked = 0;

    for (index, block, name, args) in places.into_iter().take(until) {
        let slot = match block {
            Some(block) => &mut messages[index]["content"][block]["content"],
            None => &mut messages[index]["content"],
        };
        let Some(text) = slot.as_str().map(str::to_string) else { continue };

        if is_masked(&text) || crate::context::tokens_of(&text) <= 60 {
            continue;
        }

        let n = put(session, &text);

        *slot = json!(format!("[{name} {}{MASKED_MARK}{n}›, {} lines; obs_read(n: {n}) brings back a part]", args_line(&args), text.lines().count()));
        masked += 1;
    }

    masked
}

/// The Context Pack of the first message (everything before `[The person's message]`) at drop level `level`: 1 keeps the first half of it,
/// 2 drops it. The person's message is never touched. Answers whether anything was cut.
pub fn drop_pack(messages: &mut [Value], level: u8) -> bool {
    const MARK: &str = "[The person's message]";

    let Some(first) = messages.iter_mut().find(|message| message["role"] == "user") else { return false };
    let Some(text) = first["content"].as_str().map(str::to_string) else { return false };
    let Some(at) = text.rfind(MARK) else { return false };
    let (pack, goal) = text.split_at(at);

    if pack.trim().is_empty() {
        return false;
    }

    let mut keep = if level >= 2 { 0 } else { pack.len() / 2 };

    while keep > 0 && !pack.is_char_boundary(keep) {
        keep -= 1;
    }

    first["content"] = json!(format!("{}{}{goal}", &pack[..keep], if keep == 0 { "[The project notes were left out to fit the model's window.]\n\n" } else { "\n[… notes cut to fit the model's window …]\n\n" }));

    true
}

/// The Overflow Rule's steps, in the order they are taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    KeepOne,
    DropPack,
    LowReserve,
    SplitUnit,
    BlockUnit,
}

impl Move {
    const ORDER: [Move; 5] = [Move::KeepOne, Move::DropPack, Move::LowReserve, Move::SplitUnit, Move::BlockUnit];

    pub fn name(self) -> &'static str {
        match self {
            Move::KeepOne => "masking keeps one output",
            Move::DropPack => "project notes dropped",
            Move::LowReserve => "answer reserve 4096 to 1536",
            Move::SplitUnit => "the unit is split",
            Move::BlockUnit => "blocked (too large)",
        }
    }
}

/// How far the Overflow Rule has gone for the step in hand.
#[derive(Debug, Default)]
pub struct Overflow {
    at: usize,
}

impl Overflow {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Move {
        let step = Move::ORDER[self.at.min(Move::ORDER.len() - 1)];

        self.at += 1;

        step
    }

    pub fn reset(&mut self) {
        self.at = 0;
    }

    pub fn blocked_before(&self) -> bool {
        self.at > Move::ORDER.len()
    }
}

thread_local! {
    static LOW_RESERVE_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Switches the lowered reserve on or off for this thread's next model calls.
pub fn set_low_reserve(on: bool) {
    LOW_RESERVE_ON.with(|flag| flag.set(on));
}

pub fn low_reserve_on() -> bool {
    LOW_RESERVE_ON.with(|flag| flag.get())
}

/// The reserve a step is checked with: `reserve`, or at most `LOW_RESERVE` once the Overflow Rule asked for it.
pub fn reserve(reserve: u64) -> u64 {
    if LOW_RESERVE_ON.with(|flag| flag.get()) {
        reserve.min(LOW_RESERVE)
    } else {
        reserve
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_a_tenth_of_the_window() {
        assert_eq!(obs_cap_chars(16_384), 1_638 * 4);
    }

    #[test]
    fn a_long_output_is_cut_and_the_rest_can_be_read_back_by_range() {
        let text: String = (1..=300).map(|n| format!("line {n}\n")).collect();
        let shown = vault_cut("s-budget-1", &text, 600);

        assert!(shown.len() < 900 && shown.contains("‹obs:"), "{shown}");

        let n: u32 = shown.split("‹obs:").nth(1).unwrap().split('›').next().unwrap().parse().unwrap();
        let part = read("s-budget-1", n, "200-205", 5_000).unwrap();

        assert!(part.starts_with("line 200\nline 201"), "{part}");
        assert!(part.contains("line 205") && !part.contains("line 206\n"));
        assert!(read("s-budget-1", 9_999, "", 100).is_err());
    }

    #[test]
    fn a_short_output_is_left_whole() {
        assert_eq!(vault_cut("s-budget-2", "small", 600), "small");
    }

    fn chat() -> Vec<Value> {
        let mut messages = vec![json!({ "role": "user", "content": "notes\n\n[The person's message]\nread them all" })];

        for n in 0..6 {
            messages.push(json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": format!("c{n}"), "type": "function", "function": { "name": "read_file", "arguments": format!("{{\"path\":\"f{n}.txt\"}}") } }] }));
            messages.push(json!({ "role": "tool", "tool_call_id": format!("c{n}"), "content": format!("line of file {n}\n").repeat(100) }));
        }

        messages
    }

    #[test]
    fn masking_keeps_the_newest_k_whole_and_the_rest_become_a_handle() {
        let mut messages = chat();

        assert_eq!(mask_old(Dialect::OpenAi, &mut messages, 3, "s-budget-3"), 3);

        let tools: Vec<String> = messages.iter().filter(|m| m["role"] == "tool").map(|m| m["content"].as_str().unwrap().to_string()).collect();

        assert!(tools[0].starts_with("[read_file f0.txt → ‹obs:"), "{}", tools[0]);
        assert!(tools[2].contains("‹obs:") && tools[3].starts_with("line of file 3"));
        assert_eq!(mask_old(Dialect::OpenAi, &mut messages, 3, "s-budget-3"), 0, "a handle is not masked twice");
        assert_eq!(mask_old(Dialect::OpenAi, &mut messages, 1, "s-budget-3"), 2);
    }

    #[test]
    fn the_pack_goes_in_two_levels_and_the_goal_stays() {
        let mut messages = vec![json!({ "role": "user", "content": format!("{}\n[The person's message]\nfix it", "note ".repeat(200)) })];

        assert!(drop_pack(&mut messages, 1));
        assert!(messages[0]["content"].as_str().unwrap().ends_with("[The person's message]\nfix it"));
        assert!(drop_pack(&mut messages, 2));
        assert!(!messages[0]["content"].as_str().unwrap().contains("note note"));
        assert!(messages[0]["content"].as_str().unwrap().ends_with("fix it"));
    }

    #[test]
    fn the_overflow_rule_goes_in_order_and_ends_blocked() {
        let mut rule = Overflow::default();
        let moves: Vec<Move> = (0..7).map(|_| rule.next()).collect();

        assert_eq!(&moves[..5], &Move::ORDER);
        assert_eq!(moves[6], Move::BlockUnit);

        rule.reset();

        assert_eq!(rule.next(), Move::KeepOne);
    }

    #[test]
    fn the_lowered_reserve_only_applies_when_asked() {
        set_low_reserve(false);
        assert_eq!(reserve(4_096), 4_096);
        set_low_reserve(true);
        assert_eq!(reserve(4_096), LOW_RESERVE);
        set_low_reserve(false);
    }
}
