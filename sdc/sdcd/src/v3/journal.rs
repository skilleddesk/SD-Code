//! **The job journal** (v3 R1) and **resume** (R2).
//!
//! An agent turn writes ahead, into `job_events`, everything needed to continue it:
//!
//! * `start` - the turn's first messages, model, provider, folder, autonomy;
//! * `model` - each reply as it arrived (the assistant message and its tool calls);
//! * `intent` - a tool call, written *before* it runs (with an idempotency key);
//! * `result` - what the call answered, written after it ran;
//! * `messages` - anything else the loop added to the conversation (SDC's notes, steered words);
//! * `end` - the turn finished, failed or was stopped on purpose: nothing to resume.
//!
//! A turn with a `start` and no `end` was lost with the daemon (a crash, a power cut, a kill). On the next start
//! (`unfinished`) it is continued as a new turn in the same chat: the conversation is rebuilt from the journal,
//! a read that never answered is run again, and a call that changes things and never answered (an edit, a
//! command) is **not** repeated blindly - the model is told its outcome is unknown and to check before doing it
//! again. Writes are synchronous on SQLite in WAL mode, so a step the journal says happened, happened.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use rusqlite::params;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agent::dialect::ToolUse;
use crate::store::sqlite::Store;

/// The record shape this build writes (P0-4). A row written before `0201-journal-v2` reads back as 1, because
/// that is the shape it really has; every row from now on is 2 and carries a `step_id`. A row from a *newer*
/// build than this one is stepped over rather than guessed at, so a downgrade loses a turn instead of
/// misreading it.
pub const SCHEMA: i64 = 2;

/// Where a turn's tools ran (P0-4). R4's rule is unchanged - a turn that ran on a runner is reported, never
/// continued here - but the journal now records which it was, so a crashed VPS turn can be found and its chat
/// can be told what happened instead of the record being thrown away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Local,
    /// The tools ran over ssh on a VPS (R4: the far machine holds the files, the daemon holds the model).
    Vps,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Local => "local",
            Origin::Vps => "vps",
        }
    }

    fn parse(text: &str) -> Origin {
        match text {
            "vps" => Origin::Vps,
            _ => Origin::Local,
        }
    }
}

/// The tools that only look: an unanswered one is simply run again on resume.
pub const READ_ONLY: [&str; 6] = ["read_file", "grep", "glob", "list_dir", "search", "web_search"];

/// Resumes in a row for one task before SDC stops trying (a turn that crashes the daemon every time must not
/// crash it for ever).
pub const MAX_RESUMES: u64 = 3;

/// What a resumed call that changed things, and never answered, is answered with.
pub const UNKNOWN_OUTCOME: &str = "[SDC resume: SDC stopped while this call was running, so its outcome is unknown. Check the current state first (read the file, list the processes, look at the output) and only then decide whether to do it again.]";

fn installed() -> &'static OnceLock<Arc<Store>> {
    static STORE: OnceLock<Arc<Store>> = OnceLock::new();

    &STORE
}

/// The daemon's store, once (at bootstrap). Tests use [`Writer::with_store`] instead.
pub fn install(store: Arc<Store>) {
    let _ = installed().set(store);
}

/// The daemon's store, for v3 code that runs where no handler is at hand (the fleet tool's host list).
pub fn store() -> Option<Arc<Store>> {
    installed().get().cloned()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn sha(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// What a resumed turn starts from.
#[derive(Debug, Clone, PartialEq)]
pub struct Resume {
    pub from_turn: String,
    pub messages: Vec<Value>,
    /// The last reply's calls that never answered, with the answers already known for the others.
    pub calls: Vec<ToolUse>,
    pub answered: HashMap<String, (String, bool)>,
    pub depth: u64,
    /// The chat's secret placeholders at the newest step (W9), so `‹secret:N›` keeps its meaning.
    pub vault: Vec<(usize, String)>,
    /// The machine the interrupted turn ran on, when it was not this one (R4). A turn with this set is reported, not continued.
    pub remote: Option<String>,
    /// Which machine ran this turn's tools (P0-4). `remote.is_some()` and `origin == Vps` agree; the field is
    /// what a query can ask for without reading a payload.
    pub origin: Origin,
    /// The step the turn was inside when it was interrupted (P0-4, reserved for the Step Runtime).
    pub step_id: String,
    /// Records that did not read back - the damaged tail of a turn that was killed mid-write (P0-4). Counted so
    /// the answer can say "one record was damaged" instead of quietly putting the turn together without it.
    pub damaged: usize,
    /// Records written by a newer build than this one (P0-4). Stepped over: reading an unknown shape would put a
    /// wrong turn back together.
    pub skipped_newer: usize,
}

fn pending() -> &'static Mutex<HashMap<String, Resume>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Resume>>> = OnceLock::new();

    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Hands a rebuilt conversation to the next agent turn of a chat.
pub fn offer(session_id: &str, resume: Resume) {
    pending().lock().unwrap_or_else(|poison| poison.into_inner()).insert(session_id.to_string(), resume);
}

/// The conversation a chat's new turn continues from, if a resume was offered for it.
pub fn take(session_id: &str) -> Option<Resume> {
    pending().lock().unwrap_or_else(|poison| poison.into_inner()).remove(session_id)
}

/// The host a chat's lost turn ran on, when that turn's tools ran on another machine (R4).
///
/// Such a turn is not continued here: this daemon's journal knows what the turn asked for, but not what became of the work
/// on the other machine. The turn is spent - the journal is taken - so it is reported once, and the next turn starts clean.
pub fn lost_remote(session_id: &str) -> Option<String> {
    let mut offered = pending().lock().unwrap_or_else(|poison| poison.into_inner());

    /* A turn that ran here is left where it is: this is only about turns whose tools ran somewhere else. */
    offered.get(session_id)?.remote.as_ref()?;

    offered.remove(session_id)?.remote
}

/// One turn's journal. Writing never fails a turn: a journal that cannot be written is a lost resume, not a
/// lost turn.
pub struct Writer {
    store: Arc<Store>,
    turn_id: String,
    session_id: String,
    /// The step currently running, reserved for the Step Runtime (P0-12/P0-14). Empty until a step
    /// introduces itself with `Writer::step`.
    step_id: String,
    origin: Origin,
    ended: bool,
}

impl Writer {
    /// The journal for a turn, when the switch is on and the daemon installed its store.
    pub fn open(turn_id: &str, session_id: &str, start: Value) -> Option<Self> {
        Self::open_on(turn_id, session_id, start, Origin::Local)
    }

    /// The journal for a turn that runs its tools somewhere else (P0-4). Same records, same resume; `origin`
    /// is what tells a later reader that this turn's tools ran on the runner and not here.
    pub fn open_on(turn_id: &str, session_id: &str, start: Value, origin: Origin) -> Option<Self> {
        if !crate::v3::on("journal") {
            return None;
        }

        Self::in_place(installed().get()?.clone(), turn_id, session_id, start, origin)
    }

    pub fn with_store(store: Arc<Store>, turn_id: &str, session_id: &str, start: Value) -> Option<Self> {
        Self::in_place(store, turn_id, session_id, start, Origin::Local)
    }

    /// A turn whose tools run over ssh on a VPS (R4). Same journal, same resume: only `origin` differs, and the
    /// resume uses it to say "this one ran on the runner" instead of pretending it ran here.
    pub fn on_runner(store: Arc<Store>, turn_id: &str, session_id: &str, start: Value) -> Option<Self> {
        Self::in_place(store, turn_id, session_id, start, Origin::Vps)
    }

    fn in_place(store: Arc<Store>, turn_id: &str, session_id: &str, start: Value, origin: Origin) -> Option<Self> {
        /* P0-4: the journal's promise is that a record which was written survives a crash, so the connection is
           put in FULL synchronous mode once per turn: WAL's default `NORMAL` leaves the last commits in the
           operating system's cache, which a `kill -9` keeps but a power cut does not - and a power cut is the
           failure the journal exists for. */
        if store.path() != ":memory:" && !store.durable() {
            eprintln!("sdcd: the journal could not be put in durable mode; a power cut may lose the last record");
        }

        let writer = Self { store, turn_id: turn_id.to_string(), session_id: session_id.to_string(), step_id: String::new(), origin, ended: false };

        writer.write(0, "start", None, None, None, None, &start).then_some(writer)
    }

    /// The Step Runtime introduces the step a record belongs to (P0-4 reserves the field; P0-12/P0-14 fill it).
    /// An id and not the `step` number, because a resumed turn has to say which step it was *inside*: two runs of
    /// the same step number are different steps, and only an id tells them apart.
    pub fn step(&mut self, step_id: &str) {
        self.step_id = step_id.to_string();
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    #[allow(clippy::too_many_arguments)]
    fn write(&self, step: usize, kind: &str, call_id: Option<&str>, tool: Option<&str>, idem: Option<&str>, hash: Option<&str>, payload: &Value) -> bool {
        let Ok(connection) = self.store.connection.lock() else {
            return false;
        };

        connection
            .execute(
                "INSERT INTO job_events (turn_id, session_id, step, step_id, origin, schema, kind, call_id, tool, idem, hash, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![self.turn_id, self.session_id, step as i64, self.step_id, self.origin.as_str(), SCHEMA, kind, call_id, tool, idem, hash, payload.to_string(), now()],
            )
            .is_ok()
    }

    pub fn model(&self, step: usize, message: &Value, calls: &[ToolUse]) {
        let calls: Vec<Value> = calls.iter().map(|call| json!({ "id": call.id, "name": call.name, "input": call.input })).collect();

        self.write(step, "model", None, None, None, None, &json!({ "message": message, "calls": calls }));
    }

    /// The chat's secret placeholders (W9), after a step that may have added some.
    pub fn vault(&self, step: usize, pairs: &[(usize, String)]) {
        if !pairs.is_empty() {
            self.write(step, "vault", None, None, None, None, &serde_json::to_value(pairs).unwrap_or(Value::Null));
        }
    }

    /// Before a call runs. The idempotency key names the call, so a resume can tell it apart from a repeat.
    pub fn intent(&self, step: usize, call: &ToolUse) {
        let idem = sha(&format!("{}\u{1f}{}\u{1f}{}", self.turn_id, call.id, call.input));

        self.write(step, "intent", Some(&call.id), Some(&call.name), Some(&idem), None, &json!({ "input": call.input }));
    }

    /// After a call ran: its answer (kept whole - the resume sends it back), and the answer's hash.
    pub fn result(&self, step: usize, call_id: &str, tool: &str, content: &str, is_error: bool) {
        self.write(step, "result", Some(call_id), Some(tool), None, Some(&sha(content)), &json!({ "content": content, "isError": is_error }));
    }

    /// Messages the loop added that are not a reply or a call's answer.
    pub fn messages(&self, step: usize, messages: &[Value]) {
        if !messages.is_empty() {
            self.write(step, "messages", None, None, None, None, &Value::Array(messages.to_vec()));
        }
    }

    /// The turn ended on purpose (done, failed, stopped): nothing to resume.
    pub fn end(&mut self, outcome: &str) {
        if !self.ended {
            self.ended = true;
            self.write(0, "end", None, None, None, None, &json!({ "outcome": outcome }));
        }
    }
}

impl Drop for Writer {
    /// Every way out of the loop that runs code - an answer, a failure, a stop, a panic - is an end. Only a
    /// process that dies mid-turn leaves the journal open, which is exactly the case to resume.
    fn drop(&mut self) {
        self.end("returned");
    }
}

/// A turn that was lost with the daemon, ready to be continued.
#[derive(Debug, Clone, PartialEq)]
pub struct Unfinished {
    pub turn_id: String,
    pub session_id: String,
    pub start: Value,
    pub resume: Resume,
}

/// The turns with a `start` and no `end`, rebuilt.
pub fn unfinished(store: &Store) -> Vec<Unfinished> {
    let turns: Vec<(String, String)> = {
        let Ok(connection) = store.connection.lock() else {
            return Vec::new();
        };
        let Ok(mut statement) = connection.prepare(
            "SELECT turn_id, session_id FROM job_events WHERE kind = 'start' AND turn_id NOT IN (SELECT turn_id FROM job_events WHERE kind = 'end') ORDER BY id",
        ) else {
            return Vec::new();
        };

        statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    };

    turns.into_iter().filter_map(|(turn_id, session_id)| rebuild(store, &turn_id).map(|(start, resume)| Unfinished { turn_id, session_id, start, resume })).collect()
}

/// Marks a lost turn as handled (resumed, given up, or no longer resumable).
pub fn close(store: &Store, turn_id: &str, session_id: &str, outcome: &str) {
    if let Ok(connection) = store.connection.lock() {
        /* P0-4: the closing record carries the shape, the step and the origin the turn was written with. Writing
           the defaults here would relabel a runner turn as a local one in the very record that ends it. */
        let (step_id, origin) = connection
            .query_row(
                "SELECT step_id, origin FROM job_events WHERE turn_id = ?1 ORDER BY id DESC LIMIT 1",
                params![turn_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap_or_else(|_| (String::new(), Origin::Local.as_str().to_string()));

        let _ = connection.execute(
            "INSERT INTO job_events (turn_id, session_id, step, step_id, origin, schema, kind, payload, created_at) VALUES (?1, ?2, 0, ?3, ?4, ?5, 'end', ?6, ?7)",
            params![turn_id, session_id, step_id, origin, SCHEMA, json!({ "outcome": outcome }).to_string(), now()],
        );
    }
}

/// One `job_events` row as `rebuild` reads it: `kind, call_id, tool, payload, schema, step_id, origin`.
type EventRow = (String, Option<String>, Option<String>, String, i64, String, String);

/// The conversation of a journaled turn, as the loop had it at the moment it stopped.
pub fn rebuild(store: &Store, turn_id: &str) -> Option<(Value, Resume)> {
    let rows: Vec<EventRow> = {
        let connection = store.connection.lock().ok()?;
        let mut statement = connection
            .prepare("SELECT kind, call_id, tool, payload, schema, step_id, origin FROM job_events WHERE turn_id = ?1 ORDER BY id")
            .ok()?;

        let rows: Vec<EventRow> = statement
            .query_map(params![turn_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)))
            .ok()?
            .flatten()
            .collect();

        rows
    };
    let mut start = Value::Null;
    let mut messages: Vec<Value> = Vec::new();
    let mut calls: Vec<ToolUse> = Vec::new();
    let mut answered: HashMap<String, (String, bool)> = HashMap::new();
    let mut vault: Vec<(usize, String)> = Vec::new();
    let mut origin = Origin::Local;
    let mut step_id = String::new();
    let mut damaged = 0usize;
    let mut skipped_newer = 0usize;

    for (kind, call_id, _tool, payload, schema, row_step, row_origin) in rows {
        /* P0-4: the step and the origin are columns of the journal, not part of the versioned payload, so they
           are read even from a record this build cannot interpret: a resume should still be able to say which
           step the turn was inside, whichever build wrote the record. */
        origin = Origin::parse(&row_origin);
        if !row_step.is_empty() {
            step_id = row_step;
        }

        /* A row from a build newer than this one is stepped over, not guessed at: reading a shape this build does
           not know would put a wrong turn back together, which is worse than losing it. */
        if schema > SCHEMA {
            skipped_newer += 1;
            continue;
        }

        let payload: Value = match serde_json::from_str(&payload) {
            Ok(value) => value,
            Err(_) => {
                /* P0-4, the damaged tail: the last record of a turn that was killed mid-write. It is counted and
                   skipped, never used: a record that does not read back is not evidence of anything. */
                damaged += 1;
                continue;
            }
        };

        match kind.as_str() {
            "start" => {
                messages = payload["messages"].as_array().cloned().unwrap_or_default();
                start = payload;
            }
            "model" => {
                messages.push(payload["message"].clone());
                calls = payload["calls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|call| ToolUse { id: call["id"].as_str().unwrap_or_default().to_string(), name: call["name"].as_str().unwrap_or_default().to_string(), input: call["input"].clone() })
                    .collect();
                answered.clear();
            }
            "vault" => vault = serde_json::from_value(payload).unwrap_or_default(),
            "result" => {
                if let Some(id) = call_id {
                    answered.insert(id, (payload["content"].as_str().unwrap_or_default().to_string(), payload["isError"] == true));
                }
            }
            /* The step's answers are already in the conversation once the loop added them: they arrive here as
               part of `messages`, so the open calls are those of the newest reply only. */
            "messages" => {
                let carries_answers = payload.as_array().is_some_and(|added| added.iter().any(|message| message["role"] == "tool" || message["content"].as_array().is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_result"))));

                if carries_answers {
                    calls.clear();
                    answered.clear();
                }

                messages.extend(payload.as_array().cloned().unwrap_or_default());
            }
            _ => {}
        }
    }

    /* P0-4: the `start` record is the one a kill is most likely to leave half-written, and losing it used to
       erase the whole turn. When other records survived, the resume is handed over anyway and `start_lost` in the
       start value says that the prompt was the part that went: a turn that is reported as damaged is worth more
       than a turn that vanishes, and the alternative is a resumed chat that pretends nothing was ever asked. */
    let start_lost = start.is_null() && damaged > 0;

    if start.is_null() && !start_lost {
        return None;
    }

    if start_lost {
        start = json!({ "prompt": "", "start_lost": true });
    }

    let depth = start["resumeDepth"].as_u64().unwrap_or(0);
    /* R4: a turn that ran on a runner says which machine. Such a turn is reported, never continued here. */
    let remote = start["remote"].as_str().map(str::to_string);

    Some((start, Resume { from_turn: turn_id.to_string(), messages, calls, answered, depth, vault, remote, origin, step_id, damaged, skipped_newer }))
}

/// What an interrupted write found on disk.
pub const WRITE_DONE: &str = "[SDC resume: this change had already been made before SDC stopped - the file holds it.]";

/// How an interrupted call is settled on resume.
#[derive(Debug, Clone, PartialEq)]
enum Settled {
    Answered(String, bool),
    Rerun,
}

/// A file write is reconciled against the folder (R2): applied already, or safe to run again.
fn reconcile_write(call: &ToolUse, root: Option<&std::path::Path>) -> Option<Settled> {
    let root = root?;
    let path = root.join(call.input["path"].as_str()?.trim_start_matches("./"));
    let now = std::fs::read_to_string(&path).ok();

    match call.name.as_str() {
        /* Writing the whole file is idempotent: done if it already holds the content, otherwise write it. */
        "write_file" => Some(if now.as_deref() == call.input["content"].as_str() { Settled::Answered(WRITE_DONE.into(), false) } else { Settled::Rerun }),
        /* A replacement is done when the new text is there and the old is gone, and still to do when the old is
           there; anything else (the file moved on) is for the model to look at. */
        "edit_file" => {
            let (now, old, new) = (now?, call.input["old_text"].as_str()?, call.input["new_text"].as_str()?);

            if !old.is_empty() && now.contains(old) {
                Some(Settled::Rerun)
            } else if now.contains(new) {
                Some(Settled::Answered(WRITE_DONE.into(), false))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Which of a resumed reply's calls run again (reads, and writes that did not land), and the answer for each of
/// the others: its journaled answer, "already made", or - for a command - "outcome unknown".
pub fn settle(resume: &Resume, root: Option<&std::path::Path>) -> (Vec<ToolUse>, Vec<(String, String, bool)>) {
    let mut rerun = Vec::new();
    let mut known = Vec::new();

    for call in &resume.calls {
        let settled = if let Some((content, is_error)) = resume.answered.get(&call.id) {
            Settled::Answered(content.clone(), *is_error)
        } else if READ_ONLY.contains(&call.name.as_str()) {
            Settled::Rerun
        } else {
            reconcile_write(call, root).unwrap_or_else(|| Settled::Answered(UNKNOWN_OUTCOME.to_string(), true))
        };

        match settled {
            Settled::Answered(content, is_error) => known.push((call.id.clone(), content, is_error)),
            Settled::Rerun => rerun.push(call.clone()),
        }
    }

    (rerun, known)
}

/// Ids of a set of calls, for tests and logs.
pub fn ids(calls: &[ToolUse]) -> HashSet<String> {
    calls.iter().map(|call| call.id.clone()).collect()
}

/// The words a resumed turn shows in the chat.
pub const RESUME_PROMPT: &str = "Continue the task that was interrupted when SDC stopped.";

/// On daemon start (R2): every turn lost with the last daemon is continued as a new turn in its chat - or closed
/// with the reason it is not (resume switched off, folder gone, too many resumes in a row). Answers a sentence per
/// turn for the start-up log.
pub fn resume_lost_turns(state: &std::sync::Arc<crate::DaemonState>, out: std::sync::Arc<dyn crate::sdcp::notifications::Notifier>) -> Vec<String> {
    let mut said = Vec::new();

    for lost in unfinished(&state.store) {
        let root = lost.start["root"].as_str().unwrap_or_default().to_string();
        let reason = if !crate::v3::on("resume") {
            Some("resume is switched off (v3.resume)")
        } else if lost.resume.depth + 1 >= MAX_RESUMES {
            Some("it was resumed too many times in a row")
        } else if root.is_empty() || !std::path::Path::new(&root).is_dir() {
            Some("its folder is gone")
        } else {
            None
        };

        if let Some(reason) = reason {
            close(&state.store, &lost.turn_id, &lost.session_id, &format!("not resumed: {reason}"));
            said.push(format!("turn {} not resumed: {reason}", lost.turn_id));
            continue;
        }

        offer(&lost.session_id, lost.resume.clone());

        let line = json!({
            "v": crate::SDCP_VERSION,
            "id": format!("resume-{}", lost.turn_id),
            "method": "engine.start",
            "params": {
                "sessionId": lost.session_id,
                "prompt": RESUME_PROMPT,
                "engine": "native_api",
                "agent": true,
                "autonomy": lost.start["autonomy"].as_str().unwrap_or("ask"),
                "model": lost.start["model"],
                "provider": lost.start["provider"],
                "effort": lost.start["effort"],
            }
        })
        .to_string();
        let started = crate::sdcp::envelope::Envelope::parse(&line).map(|envelope| state.handler().handle(&envelope, out.clone()));

        match started.ok().and_then(|response| response.result).and_then(|result| result["turnId"].as_str().map(str::to_string)) {
            Some(turn) => {
                close(&state.store, &lost.turn_id, &lost.session_id, &format!("resumed as {turn}"));
                said.push(format!("turn {} resumed as {turn}", lost.turn_id));
            }
            None => {
                take(&lost.session_id);
                close(&state.store, &lost.turn_id, &lost.session_id, "resume could not start");
                said.push(format!("turn {} could not be resumed", lost.turn_id));
            }
        }
    }

    said
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, name: &str) -> ToolUse {
        ToolUse { id: id.into(), name: name.into(), input: json!({ "path": "a.txt" }) }
    }

    #[test]
    fn a_lost_turn_is_rebuilt_up_to_the_step_it_stopped_in() {
        let store = Arc::new(Store::in_memory().unwrap());
        let first = json!({ "role": "user", "content": "do it" });
        let writer = Writer::with_store(store.clone(), "t1", "s1", json!({ "messages": [first], "model": "m" })).unwrap();
        let reply_one = json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c1" }] });

        writer.model(1, &reply_one, &[call("c1", "read_file")]);
        writer.intent(1, &call("c1", "read_file"));
        writer.result(1, "c1", "read_file", "hello", false);
        writer.messages(2, &[json!({ "role": "tool", "tool_call_id": "c1", "content": "hello" })]);

        let reply_two = json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c2" }, { "id": "c3" }, { "id": "c4" }] });

        writer.model(2, &reply_two, &[call("c2", "write_file"), call("c3", "read_file"), call("c4", "run_command")]);
        writer.intent(2, &call("c2", "write_file"));
        writer.result(2, "c2", "write_file", "written", false);
        writer.intent(2, &call("c3", "read_file"));
        /* The daemon dies here: c3 never answered, c4 never started. */
        std::mem::forget(writer);

        let lost = unfinished(&store);

        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].resume.messages.len(), 4, "first message, reply, its answer, second reply");

        let (rerun, known) = settle(&lost[0].resume, None);

        assert_eq!(ids(&rerun), ids(&[call("c3", "read_file")]), "the read runs again");
        assert_eq!(known.len(), 2);
        assert_eq!(known[0], ("c2".to_string(), "written".to_string(), false), "an answered call keeps its answer");
        assert_eq!(known[1].1, UNKNOWN_OUTCOME, "a command that never answered is not repeated blindly");

        close(&store, "t1", "s1", "resumed");
        assert!(unfinished(&store).is_empty());
    }

    #[test]
    fn an_interrupted_write_is_reconciled_with_the_folder() {
        let root = std::env::temp_dir().join(format!("sdc-journal-writes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("done.txt"), "alpha\n").unwrap();
        std::fs::write(root.join("edit.txt"), "let x = 1;\n").unwrap();
        std::fs::write(root.join("edited.txt"), "let x = 2;\n").unwrap();

        let write = |id: &str, path: &str| ToolUse { id: id.into(), name: "write_file".into(), input: json!({ "path": path, "content": "alpha\n" }) };
        let edit = |id: &str, path: &str| ToolUse { id: id.into(), name: "edit_file".into(), input: json!({ "path": path, "old_text": "x = 1", "new_text": "x = 2" }) };
        let resume = Resume {
            from_turn: "t".into(),
            messages: vec![],
            calls: vec![write("w1", "done.txt"), write("w2", "missing.txt"), edit("e1", "edit.txt"), edit("e2", "edited.txt"), call("r1", "run_command")],
            answered: HashMap::new(),
            depth: 0,
            vault: vec![],
            remote: None,
            origin: Origin::Local,
            step_id: String::new(),
            damaged: 0,
            skipped_newer: 0,
        };
        let (rerun, known) = settle(&resume, Some(&root));

        assert_eq!(ids(&rerun), ids(&[write("w2", ""), edit("e1", "")]), "a write that did not land runs again");
        assert_eq!(known.iter().map(|(id, content, _)| (id.as_str(), content.as_str())).collect::<Vec<_>>(), [("w1", WRITE_DONE), ("e2", WRITE_DONE), ("r1", UNKNOWN_OUTCOME)]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_turn_that_returns_is_closed_and_never_resumed() {
        let store = Arc::new(Store::in_memory().unwrap());

        {
            let writer = Writer::with_store(store.clone(), "t2", "s1", json!({ "messages": [] })).unwrap();

            writer.model(1, &json!({ "role": "assistant", "content": "done" }), &[]);
        }

        assert!(unfinished(&store).is_empty());
    }

    /// The three fields P0-4 added, with the values a fresh turn has, so a test only names what it is about.
    fn resume(from_turn: &str, remote: Option<&str>, depth: u64) -> Resume {
        Resume {
            from_turn: from_turn.into(),
            messages: vec![],
            calls: vec![],
            answered: HashMap::new(),
            depth,
            vault: vec![],
            remote: remote.map(str::to_string),
            origin: match remote {
                Some(_) => Origin::Vps,
                None => Origin::Local,
            },
            step_id: String::new(),
            damaged: 0,
            skipped_newer: 0,
        }
    }

    #[test]
    fn an_offered_resume_is_taken_once() {
        let resume = resume("t", None, 1);

        offer("chat-x", resume.clone());
        assert_eq!(take("chat-x"), Some(resume));
        assert_eq!(take("chat-x"), None);
    }

    #[test]
    fn a_lost_turn_that_ran_elsewhere_is_reported_and_not_continued() {
        offer("chat-r", resume("t", Some("vps.example"), 0));

        /* The machine is named once... */
        assert_eq!(lost_remote("chat-r"), Some("vps.example".into()));
        /* ...and the journal is spent with it: nothing is left for a local resume to pick up. */
        assert_eq!(lost_remote("chat-r"), None);
        assert_eq!(take("chat-r"), None);
    }

    #[test]
    fn a_local_lost_turn_is_left_for_a_resume() {
        offer("chat-l", resume("t", None, 0));

        assert_eq!(lost_remote("chat-l"), None, "a local turn is continued, not merely reported");
        assert!(take("chat-l").is_some(), "and its journal is still there to continue from");
    }

    /* ---- P0-4: versioned records, the reserved step id, the runner's origin, and the damaged tail ---- */

    #[test]
    fn a_record_carries_the_schema_the_step_and_the_origin() {
        let store = Arc::new(Store::in_memory().unwrap());
        let mut writer = Writer::with_store(store.clone(), "t-1", "s-1", json!({ "prompt": "hi" })).unwrap();

        writer.step("step-7");
        writer.model(4, &json!({ "role": "assistant", "content": "ok" }), &[]);

        let connection = store.connection.lock().unwrap();
        let rows: Vec<(i64, String, String)> = connection
            .prepare("SELECT schema, step_id, origin FROM job_events WHERE turn_id = 't-1' ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .flatten()
            .collect();

        assert_eq!(rows.len(), 2, "the start record and the model record");
        /* The first record is written before a step introduces itself, which is why the field is allowed to be empty. */
        assert_eq!(rows[0], (SCHEMA, String::new(), "local".into()));
        assert_eq!(rows[1], (SCHEMA, "step-7".into(), "local".into()));
    }

    #[test]
    fn a_runner_turn_says_so_in_every_record_and_in_the_resume() {
        let store = Arc::new(Store::in_memory().unwrap());
        let mut writer = Writer::on_runner(store.clone(), "t-vps", "s-1", json!({ "prompt": "hi", "remote": "vps.example" })).unwrap();

        writer.step("step-2");
        writer.model(2, &json!({ "role": "assistant", "content": "ran make" }), &[call("c1", "Run")]);
        std::mem::forget(writer); /* a crash: no `end` record, exactly what a kill leaves behind */

        let (_, resumed) = rebuild(&store, "t-vps").unwrap();

        assert_eq!(resumed.origin, Origin::Vps, "the resume has to know the work happened elsewhere");
        assert_eq!(resumed.step_id, "step-2", "and which step it was inside");
        assert_eq!(resumed.damaged, 0);
        assert_eq!(resumed.calls.len(), 1);
    }

    #[test]
    fn a_damaged_tail_record_is_counted_and_skipped() {
        let store = Arc::new(Store::in_memory().unwrap());
        let mut writer = Writer::with_store(store.clone(), "t-2", "s-1", json!({ "prompt": "hi" })).unwrap();

        writer.step("step-1");
        writer.model(1, &json!({ "role": "user", "content": "first" }), &[]);
        writer.model(2, &json!({ "role": "assistant", "content": "second" }), &[]);

        /* What a kill during the last write leaves: the tail record's payload is not JSON any more. */
        store
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE job_events SET payload = '{\"role\": \"assis' WHERE turn_id = 't-2' AND id = (SELECT MAX(id) FROM job_events WHERE turn_id = 't-2')", [])
            .unwrap();

        let (_, resumed) = rebuild(&store, "t-2").unwrap();

        assert_eq!(resumed.damaged, 1, "the damaged record is counted, not silently dropped");
        assert_eq!(resumed.messages.len(), 1, "and the records before it are still good");
        assert_eq!(resumed.step_id, "step-1");
    }

    #[test]
    fn a_record_from_a_newer_build_is_stepped_over() {
        let store = Arc::new(Store::in_memory().unwrap());
        let writer = Writer::with_store(store.clone(), "t-3", "s-1", json!({ "prompt": "hi" })).unwrap();

        writer.model(1, &json!({ "role": "user", "content": "known" }), &[]);
        std::mem::forget(writer);

        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO job_events (turn_id, session_id, step, step_id, origin, schema, kind, payload, created_at) VALUES ('t-3', 's-1', 2, 'step-9', 'local', ?1, 'message', ?2, '2026-10-17T00:00:00Z')",
                params![SCHEMA + 1, json!({ "role": "assistant", "content": "from the future" }).to_string()],
            )
            .unwrap();

        let (_, resumed) = rebuild(&store, "t-3").unwrap();

        assert_eq!(resumed.skipped_newer, 1);
        assert_eq!(resumed.messages.len(), 1, "the shape this build does not know is not guessed at");
        assert_eq!(resumed.step_id, "step-9", "but its step id is still read: that field has not moved");
    }

    #[test]
    fn a_closing_record_keeps_the_origin_of_the_turn_it_closes() {
        let store = Arc::new(Store::in_memory().unwrap());
        let writer = Writer::on_runner(store.clone(), "t-4", "s-1", json!({ "prompt": "hi" })).unwrap();

        std::mem::forget(writer);
        close(&store, "t-4", "s-1", "done");

        let (_, resumed) = rebuild(&store, "t-4").unwrap();
        assert_eq!(resumed.origin, Origin::Vps, "the record that ends a runner turn is a runner record too");

        let origin: String = store
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT origin FROM job_events WHERE turn_id = 't-4' AND kind = 'end'", [], |row| row.get(0))
            .unwrap();

        assert_eq!(origin, "vps");
    }
}
