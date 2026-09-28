//! What Claude Code is writing *before* the tool runs (0.14.2).
//!
//! The report: *"live steamming omg lvl ar bad ... monai hosse nah je live steamming hosse"*. Measured
//! against a dev daemon: a turn that wrote a 150-line file sent a `Read` card at 6.0 s and then **nothing
//! for 31.7 s**, until the `Write` card arrived at 37.7 s already finished. The model was streaming the
//! file the whole time, as `input_json_delta` pieces, and the parser ignored them because a tool card is
//! opened from the finished `assistant` line (0.14.1: opening it earlier gave every card an empty target).
//!
//! This tracker reads those pieces without opening a card: it follows each `tool_use` block from its
//! `content_block_start` to its `content_block_stop` and says, a few times a second, which tool is being
//! prepared, what it targets as soon as that much of the JSON has arrived, how much has been written, and
//! the last lines of it. The card itself still opens from the `assistant` line, so the loop guard and the
//! checkpoint see exactly what they saw before.
//!
//! It also remembers each `Write`/`Edit`/`MultiEdit` input, so the card's end carries the diff - the
//! `Write` card used to finish as `done · 1 ln` with Claude's "File created successfully" as its only line.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::engines::EngineEvent;

/// How often a draft that is still growing is reported. Every piece would flood the event log.
const DRAFT_EVERY: Duration = Duration::from_millis(300);

/// The tail of the draft a report carries: enough lines to watch it being written, never the whole file.
const PREVIEW_LINES: usize = 10;
const PREVIEW_CHARS: usize = 900;

/// The rows a diff built from a tool's input may carry (the SDC Agent's own cap).
const DIFF_ROWS: usize = 160;

/// Tools whose drafts are not shown: the checklist is the plan card, and `ToolSearch` only loads a tool.
const QUIET: [&str; 2] = ["TodoWrite", "ToolSearch"];

struct Block {
    id: String,
    name: String,
    json: String,
    reported: Option<Instant>,
}

#[derive(Default)]
pub struct DraftTracker {
    blocks: HashMap<u64, Block>,
    /// Finished inputs of the file tools, by call id, until their result arrives.
    edits: HashMap<String, (String, Value)>,
}

impl DraftTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The draft reports one stream line adds, before the line's own events.
    pub fn observe(&mut self, line: &str) -> Vec<EngineEvent> {
        self.observe_at(line, Instant::now())
    }

    fn observe_at(&mut self, line: &str, now: Instant) -> Vec<EngineEvent> {
        let trimmed = line.trim();

        if !trimmed.starts_with('{') {
            return Vec::new();
        }

        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            return Vec::new();
        };

        match value.get("type").and_then(Value::as_str) {
            Some("stream_event") => self.stream_event(value.get("event").unwrap_or(&Value::Null), now),
            Some("assistant") => {
                self.remember_edits(&value);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn stream_event(&mut self, event: &Value, now: Instant) -> Vec<EngineEvent> {
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);

        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "content_block_start" => {
                let block = event.get("content_block").unwrap_or(&Value::Null);

                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    return Vec::new();
                }

                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();

                if QUIET.contains(&name.as_str()) {
                    return Vec::new();
                }

                let id = block.get("id").and_then(Value::as_str).unwrap_or("call").to_string();
                let started = Block { id, name, json: String::new(), reported: Some(now) };
                let report = draft_of(&started);

                self.blocks.insert(index, started);

                vec![report]
            }
            "content_block_delta" => {
                let Some(block) = self.blocks.get_mut(&index) else {
                    return Vec::new();
                };
                let Some(piece) = event.pointer("/delta/partial_json").and_then(Value::as_str) else {
                    return Vec::new();
                };

                block.json.push_str(piece);

                if block.reported.is_some_and(|at| now.duration_since(at) < DRAFT_EVERY) {
                    return Vec::new();
                }

                block.reported = Some(now);

                vec![draft_of(block)]
            }
            "content_block_stop" => match self.blocks.remove(&index) {
                /* The last word on a draft that grew since its last report, so it does not end mid-line. */
                Some(block) if !block.json.is_empty() => vec![draft_of(&block)],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn remember_edits(&mut self, value: &Value) {
        let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) else {
            return;
        };

        for block in blocks.iter().filter(|block| block["type"] == "tool_use") {
            let name = block["name"].as_str().unwrap_or_default();

            if matches!(name, "Write" | "Edit" | "MultiEdit") {
                if let Some(id) = block["id"].as_str() {
                    self.edits.insert(id.to_string(), (name.to_string(), block["input"].clone()));
                }
            }
        }
    }

    /// A finished file tool's card, with the diff its input describes and a meta that counts the lines.
    pub fn complete(&mut self, event: EngineEvent) -> EngineEvent {
        let EngineEvent::ToolCompleted { call_id, status, meta, diff: None } = event else {
            return event;
        };
        let Some((name, input)) = self.edits.remove(&call_id) else {
            return EngineEvent::ToolCompleted { call_id, status, meta, diff: None };
        };

        if status != "done" {
            return EngineEvent::ToolCompleted { call_id, status, meta, diff: None };
        }

        let rows = diff_of(&name, &input);
        let added = rows.iter().filter(|row| row["change"] == "add").count();
        let removed = rows.iter().filter(|row| row["change"] == "rem").count();
        let meta = if removed == 0 { format!("done · +{added}") } else { format!("done · +{added} −{removed}") };
        let shown: Vec<Value> = rows.into_iter().take(DIFF_ROWS).collect();

        EngineEvent::ToolCompleted { call_id, status, meta, diff: Some(Value::Array(shown)) }
    }
}

/// A report on a tool call still arriving, for an adapter that assembles calls itself (the SDC Agent's
/// dialects): the call's id, its tool's name, and its arguments so far.
pub fn report(call_id: &str, name: &str, json: &str) -> EngineEvent {
    draft_of(&Block { id: call_id.to_string(), name: name.to_string(), json: json.to_string(), reported: None })
}

/// The same few-a-second pace for an adapter that keeps its own state.
#[derive(Default)]
pub struct Pace(Option<Instant>);

impl Pace {
    /// True when a report is due, and counts it as sent.
    pub fn due(&mut self) -> bool {
        self.due_at(Instant::now())
    }

    fn due_at(&mut self, now: Instant) -> bool {
        if self.0.is_some_and(|at| now.duration_since(at) < DRAFT_EVERY) {
            return false;
        }

        self.0 = Some(now);
        true
    }
}

/// One report on a block: its tool, its target once known, its size, and its newest lines.
fn draft_of(block: &Block) -> EngineEvent {
    let target = ["file_path", "command", "pattern", "url", "query", "path", "notebook_path", "description"]
        .iter()
        .find_map(|key| field(&block.json, key).filter(|(value, closed)| *closed && !value.is_empty()).map(|(value, _)| value))
        .unwrap_or_default();
    let body = ["content", "new_string", "new_text", "patch", "command", "prompt", "new_source"]
        .iter()
        .find_map(|key| string_field(&block.json, key))
        .unwrap_or_default();

    EngineEvent::ToolDraft {
        call_id: block.id.clone(),
        name: block.name.clone(),
        target: target.lines().next().unwrap_or_default().chars().take(300).collect(),
        chars: body.chars().count() as u64,
        preview: tail(&body),
    }
}

/// The last lines of a text, capped in characters, cut at a line where it can be.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(PREVIEW_LINES);
    let joined = lines[start..].join("\n");
    let count = joined.chars().count();

    if count <= PREVIEW_CHARS {
        return joined;
    }

    joined.chars().skip(count - PREVIEW_CHARS).collect()
}

/// The value of a string field in a JSON object that may still be arriving: `"key": "…` read up to its
/// closing quote, or to where the text ends. Escapes are decoded; a half-sent escape at the end is dropped.
pub fn string_field(json: &str, key: &str) -> Option<String> {
    field(json, key).map(|(value, _)| value)
}

/// `string_field`, and whether its closing quote has arrived. A target is shown only once it is whole:
/// a path read mid-way flashed as `H` and then `H:\SDC` before the file's name.
fn field(json: &str, key: &str) -> Option<(String, bool)> {
    let needle = format!("\"{key}\"");
    let mut search = 0;

    while let Some(found) = json[search..].find(&needle) {
        let after = search + found + needle.len();
        let rest = json[after..].trim_start();

        search = after;

        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };

        return Some(decode(rest));
    }

    None
}

fn decode(rest: &str) -> (String, bool) {
    let mut out = String::new();
    let mut chars = rest.chars();

    while let Some(c) = chars.next() {
        match c {
            '"' => return (out, true),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => {}
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();

                    if hex.len() < 4 {
                        break;
                    }

                    if let Some(decoded) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(decoded);
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }

    (out, false)
}

/// The card's rows for a file tool's input: a new file is all additions; an edit is its old text
/// removed and its new text added. Line numbers count within the change, since the file is not read.
fn diff_of(name: &str, input: &Value) -> Vec<Value> {
    let row = |number: usize, text: &str, change: &str| json!({ "lineNumber": number.to_string(), "text": text, "change": change });
    let pair = |old: &str, new: &str| -> Vec<Value> {
        let mut rows: Vec<Value> = old.lines().enumerate().map(|(index, line)| row(index + 1, line, "rem")).collect();

        rows.extend(new.lines().enumerate().map(|(index, line)| row(index + 1, line, "add")));
        rows
    };

    match name {
        "Write" => input["content"]
            .as_str()
            .unwrap_or_default()
            .lines()
            .enumerate()
            .map(|(index, line)| row(index + 1, line, "add"))
            .collect(),
        "Edit" => pair(input["old_string"].as_str().unwrap_or_default(), input["new_string"].as_str().unwrap_or_default()),
        "MultiEdit" => input["edits"]
            .as_array()
            .map(|edits| {
                edits
                    .iter()
                    .flat_map(|edit| pair(edit["old_string"].as_str().unwrap_or_default(), edit["new_string"].as_str().unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(event: Value) -> String {
        json!({ "type": "stream_event", "event": event }).to_string()
    }

    /// The 31-second silence, replayed: a Write block streams its input and the tracker reports it -
    /// the tool, the file as soon as its path is complete, the size, and the newest lines.
    #[test]
    fn a_file_being_written_is_reported_while_it_streams() {
        let mut tracker = DraftTracker::new();
        let t0 = Instant::now();

        let opened = tracker.observe_at(
            &line(json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "toolu_w", "name": "Write", "input": {} } })),
            t0,
        );

        assert!(matches!(&opened[..], [EngineEvent::ToolDraft { name, chars: 0, .. }] if name == "Write"));

        let piece = |json: &str| line(json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": json } }));

        /* Too soon after the last report: gathered, not sent. */
        assert!(tracker.observe_at(&piece(r#"{"file_path": "/srv/app/no"#), t0 + Duration::from_millis(100)).is_empty());

        let report = tracker.observe_at(&piece(r##"tes.md", "content": "# Notes\nline two\nline th"##), t0 + Duration::from_millis(400));
        let [EngineEvent::ToolDraft { call_id, target, chars, preview, .. }] = &report[..] else {
            panic!("expected one draft, got {report:?}");
        };

        assert_eq!(call_id, "toolu_w");
        assert_eq!(target, "/srv/app/notes.md");
        assert_eq!(*chars, "# Notes\nline two\nline th".chars().count() as u64);
        assert!(preview.ends_with("line th"));

        /* The stop reports the whole of it once more, then the block is forgotten. */
        let last = tracker.observe_at(&line(json!({ "type": "content_block_stop", "index": 1 })), t0 + Duration::from_millis(450));

        assert_eq!(last.len(), 1);
        assert!(tracker.observe_at(&piece("x"), t0 + Duration::from_secs(2)).is_empty());
    }

    #[test]
    fn text_blocks_and_quiet_tools_are_not_drafts() {
        let mut tracker = DraftTracker::new();

        assert!(tracker
            .observe(&line(json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } })))
            .is_empty());
        assert!(tracker
            .observe(&line(json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "t", "name": "TodoWrite", "input": {} } })))
            .is_empty());
    }

    /// A target is named once whole; the body shows as it grows.
    #[test]
    fn a_half_sent_target_is_not_shown_but_its_body_is() {
        let EngineEvent::ToolDraft { target, preview, .. } = report("c1", "Bash", r#"{"command": "npm run te"#) else {
            unreachable!()
        };

        assert_eq!(target, "");
        assert_eq!(preview, "npm run te");

        let EngineEvent::ToolDraft { target, .. } = report("c1", "write_file", r#"{"path": "src/app.js", "content": "con"#) else {
            unreachable!()
        };

        assert_eq!(target, "src/app.js");
    }

    #[test]
    fn a_half_sent_string_is_read_up_to_where_it_ends() {
        assert_eq!(string_field(r#"{"command": "npm run te"#, "command").as_deref(), Some("npm run te"));
        assert_eq!(string_field(r#"{"content": "a\nb\"cé\"#, "content").as_deref(), Some("a\nb\"cé"));
        assert_eq!(string_field(r#"{"content": "x\u00"#, "content").as_deref(), Some("x"));
        assert_eq!(string_field(r#"{"file_path""#, "file_path"), None);
    }

    /// The limiter a dialect keeps for itself (0.14.2): the first call is always due, since nothing has
    /// been sent yet; the next one lands inside the window and is held back; and once the window passes,
    /// a report is due again.
    #[test]
    fn a_pace_is_due_once_then_not_again_until_the_window_passes() {
        let mut pace = Pace::default();
        let t0 = Instant::now();

        assert!(pace.due_at(t0), "nothing has been sent yet, so the first call is due");
        assert!(!pace.due_at(t0 + Duration::from_millis(100)), "too soon after the last report");
        assert!(pace.due_at(t0 + DRAFT_EVERY + Duration::from_millis(1)), "due again once the window passes");
    }

    /// The `Write` card that ended as `done · 1 ln`: it now ends with the file's lines as its diff.
    #[test]
    fn a_finished_write_carries_its_lines_as_the_diff() {
        let mut tracker = DraftTracker::new();

        tracker.observe(
            &json!({ "type": "assistant", "message": { "content": [
                { "type": "tool_use", "id": "toolu_w", "name": "Write", "input": { "file_path": "a.md", "content": "one\ntwo" } },
                { "type": "tool_use", "id": "toolu_e", "name": "Edit", "input": { "file_path": "b.js", "old_string": "let x = 1;", "new_string": "const x = 1;\nexport { x };" } }
            ] } })
            .to_string(),
        );

        let EngineEvent::ToolCompleted { meta, diff: Some(diff), .. } = tracker.complete(EngineEvent::ToolCompleted {
            call_id: "toolu_w".into(),
            status: "done".into(),
            meta: "done · 1 ln".into(),
            diff: None,
        }) else {
            panic!("the write should carry a diff");
        };

        assert_eq!(meta, "done · +2");
        assert_eq!(diff[1], json!({ "lineNumber": "2", "text": "two", "change": "add" }));

        let EngineEvent::ToolCompleted { meta, .. } = tracker.complete(EngineEvent::ToolCompleted {
            call_id: "toolu_e".into(),
            status: "done".into(),
            meta: String::new(),
            diff: None,
        }) else {
            unreachable!()
        };

        assert_eq!(meta, "done · +2 −1");
    }
}
