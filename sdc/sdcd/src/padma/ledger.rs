//! **Task Contract and Task Ledger (P1-1, switch `padma.ledger`)**: every task is written down and survives failures, restarts and new
//! messages. The contract says what was asked and when it is done; the ledger says, per step, where the work stands. Both live in one
//! SQLite file under the project (`.sdc/padma/ledger.db`), written on every transition inside a transaction (WAL, `synchronous = FULL`), so
//! a kill at any moment leaves the last committed transition and nothing half-written.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// What was asked, and when it counts as finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskContract {
    pub id: String,
    pub session_id: String,
    /// The goal in English (the person's own words until the inner-English step rewrites it).
    pub goal_en: String,
    /// Where the person's original words are kept.
    pub original_ref: String,
    pub recipe: String,
    pub scope: String,
    pub done_when: Vec<String>,
    pub deliverable: String,
    pub language_out: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Todo,
    Running,
    Done,
    Blocked,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Running => "running",
            Status::Done => "done",
            Status::Blocked => "blocked",
        }
    }

    fn of(word: &str) -> Status {
        match word {
            "running" => Status::Running,
            "done" => Status::Done,
            "blocked" => Status::Blocked,
            _ => Status::Todo,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub step_id: String,
    pub unit: String,
    pub status: Status,
    pub attempts: u32,
    pub evidence_refs: Vec<String>,
    pub handover_note: String,
}

/// Where a task stands as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Done,
    Blocked,
    Paused,
    Cancelled,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Done => "done",
            State::Blocked => "blocked",
            State::Paused => "paused",
            State::Cancelled => "cancelled",
        }
    }

    fn of(word: &str) -> State {
        match word {
            "done" => State::Done,
            "blocked" => State::Blocked,
            "paused" => State::Paused,
            "cancelled" => State::Cancelled,
            _ => State::Open,
        }
    }

    /// The task can still be continued.
    pub fn unfinished(self) -> bool {
        matches!(self, State::Open | State::Blocked | State::Paused)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Criterion {
    pub text: String,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub contract: TaskContract,
    pub state: State,
    pub last_error: Option<String>,
    pub criteria: Vec<Criterion>,
    pub steps: Vec<Step>,
}

/// done / blocked / remaining step counts, as the final message lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub done: usize,
    pub blocked: usize,
    pub remaining: usize,
}

impl Task {
    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();

        for step in &self.steps {
            match step.status {
                Status::Done => counts.done += 1,
                Status::Blocked => counts.blocked += 1,
                Status::Todo | Status::Running => counts.remaining += 1,
            }
        }

        counts
    }

    /// The task as the protocol shows it.
    pub fn json(&self) -> Value {
        let counts = self.counts();

        json!({
            "taskId": self.contract.id,
            "sessionId": self.contract.session_id,
            "goal": self.contract.goal_en,
            "originalRef": self.contract.original_ref,
            "recipe": self.contract.recipe,
            "scope": self.contract.scope,
            "deliverable": self.contract.deliverable,
            "languageOut": self.contract.language_out,
            "createdAt": self.contract.created_at,
            "state": self.state.word(),
            "lastError": self.last_error,
            "doneWhen": self.criteria.iter().map(|criterion| json!({ "text": criterion.text, "evidence": criterion.evidence })).collect::<Vec<_>>(),
            "steps": self.steps.iter().map(|step| json!({
                "stepId": step.step_id,
                "unit": step.unit,
                "status": step.status.word(),
                "attempts": step.attempts,
                "evidenceRefs": step.evidence_refs,
                "handoverNote": step.handover_note,
            })).collect::<Vec<_>>(),
            "counts": { "done": counts.done, "blocked": counts.blocked, "remaining": counts.remaining },
        })
    }
}

pub struct Ledger {
    db: Connection,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn text_list(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value).unwrap_or_default()
}

impl Ledger {
    /// Where a project's ledger lives.
    pub fn path_for(root: &Path) -> PathBuf {
        root.join(".sdc").join("padma").join("ledger.db")
    }

    pub fn open(root: &Path) -> Result<Ledger, String> {
        let path = Ledger::path_for(root);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }

        Ledger::at(&path)
    }

    pub fn at(path: &Path) -> Result<Ledger, String> {
        let db = Connection::open(path).map_err(|error| error.to_string())?;

        db.busy_timeout(std::time::Duration::from_secs(5)).map_err(|error| error.to_string())?;
        db.query_row("PRAGMA journal_mode = WAL", [], |row| row.get::<_, String>(0)).map_err(|error| error.to_string())?;
        db.execute_batch(
            "PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS tasks (
                 id TEXT PRIMARY KEY, session_id TEXT NOT NULL, goal_en TEXT NOT NULL, original_ref TEXT NOT NULL, recipe TEXT NOT NULL,
                 scope TEXT NOT NULL, deliverable TEXT NOT NULL, language_out TEXT NOT NULL, created_at TEXT NOT NULL,
                 state TEXT NOT NULL DEFAULT 'open', last_error TEXT, seq INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS criteria (task_id TEXT NOT NULL, idx INTEGER NOT NULL, text TEXT NOT NULL, evidence TEXT, PRIMARY KEY (task_id, idx));
             CREATE TABLE IF NOT EXISTS steps (
                 task_id TEXT NOT NULL, ord INTEGER NOT NULL, step_id TEXT NOT NULL, unit TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'todo',
                 attempts INTEGER NOT NULL DEFAULT 0, evidence_refs TEXT NOT NULL DEFAULT '[]', handover_note TEXT NOT NULL DEFAULT '',
                 PRIMARY KEY (task_id, step_id)
             );
             CREATE TABLE IF NOT EXISTS journal (seq INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, task_id TEXT NOT NULL, step_id TEXT, what TEXT NOT NULL, note TEXT NOT NULL DEFAULT '');",
        )
        .map_err(|error| error.to_string())?;

        Ok(Ledger { db })
    }

    fn journal(tx: &rusqlite::Transaction, task: &str, step: Option<&str>, what: &str, note: &str) -> rusqlite::Result<()> {
        tx.execute("INSERT INTO journal (ts, task_id, step_id, what, note) VALUES (?1, ?2, ?3, ?4, ?5)", params![now(), task, step, what, note])?;

        Ok(())
    }

    /// Writes a new contract with its first steps (unit names) and its done-when list.
    pub fn create(&mut self, contract: &TaskContract, units: &[&str]) -> Result<(), String> {
        let tx = self.db.transaction().map_err(|error| error.to_string())?;
        let next: i64 = tx.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM tasks", [], |row| row.get(0)).map_err(|error| error.to_string())?;

        tx.execute(
            "INSERT INTO tasks (id, session_id, goal_en, original_ref, recipe, scope, deliverable, language_out, created_at, seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![contract.id, contract.session_id, contract.goal_en, contract.original_ref, contract.recipe, contract.scope, contract.deliverable, contract.language_out, contract.created_at, next],
        )
        .map_err(|error| error.to_string())?;

        for (index, text) in contract.done_when.iter().enumerate() {
            tx.execute("INSERT INTO criteria (task_id, idx, text) VALUES (?1, ?2, ?3)", params![contract.id, index as i64, text]).map_err(|error| error.to_string())?;
        }

        for (order, unit) in units.iter().enumerate() {
            tx.execute("INSERT INTO steps (task_id, ord, step_id, unit) VALUES (?1, ?2, ?3, ?4)", params![contract.id, order as i64, format!("s{}", order + 1), unit]).map_err(|error| error.to_string())?;
        }

        Ledger::journal(&tx, &contract.id, None, "created", &contract.goal_en).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    /// Moves one step to a status. `running` counts an attempt. Evidence is added to the step (never replaced); the note replaces the handover.
    pub fn set_step(&mut self, task: &str, step: &str, status: Status, evidence: Option<&str>, note: Option<&str>) -> Result<(), String> {
        let tx = self.db.transaction().map_err(|error| error.to_string())?;
        let (refs, attempts): (String, i64) = tx
            .query_row("SELECT evidence_refs, attempts FROM steps WHERE task_id = ?1 AND step_id = ?2", params![task, step], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| format!("no step {step} in task {task}: {error}"))?;
        let mut refs = text_list(&refs);

        if let Some(evidence) = evidence {
            if !refs.iter().any(|seen| seen == evidence) {
                refs.push(evidence.to_string());
            }
        }

        let attempts = attempts + i64::from(status == Status::Running);

        tx.execute(
            "UPDATE steps SET status = ?3, attempts = ?4, evidence_refs = ?5, handover_note = COALESCE(?6, handover_note) WHERE task_id = ?1 AND step_id = ?2",
            params![task, step, status.word(), attempts, json!(refs).to_string(), note],
        )
        .map_err(|error| error.to_string())?;
        Ledger::journal(&tx, task, Some(step), status.word(), note.unwrap_or_default()).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    /// Adds a step to a running task (a recipe finds the units as it goes).
    pub fn add_step(&mut self, task: &str, unit: &str) -> Result<String, String> {
        let tx = self.db.transaction().map_err(|error| error.to_string())?;
        let next: i64 = tx.query_row("SELECT COALESCE(MAX(ord), -1) + 1 FROM steps WHERE task_id = ?1", params![task], |row| row.get(0)).map_err(|error| error.to_string())?;
        let id = format!("s{}", next + 1);

        tx.execute("INSERT INTO steps (task_id, ord, step_id, unit) VALUES (?1, ?2, ?3, ?4)", params![task, next, id, unit]).map_err(|error| error.to_string())?;
        Ledger::journal(&tx, task, Some(&id), "added", unit).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;

        Ok(id)
    }

    /// Records the evidence that one done-when line is met.
    pub fn prove(&mut self, task: &str, index: usize, evidence: &str) -> Result<(), String> {
        let tx = self.db.transaction().map_err(|error| error.to_string())?;

        tx.execute("UPDATE criteria SET evidence = ?3 WHERE task_id = ?1 AND idx = ?2", params![task, index as i64, evidence]).map_err(|error| error.to_string())?;
        Ledger::journal(&tx, task, None, "proved", &format!("{index}: {evidence}")).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    /// Where the task as a whole stands; `error` is the last failure (the words a pasted error is matched against).
    pub fn set_state(&mut self, task: &str, state: State, error: Option<&str>) -> Result<(), String> {
        let tx = self.db.transaction().map_err(|error| error.to_string())?;

        tx.execute("UPDATE tasks SET state = ?2, last_error = COALESCE(?3, last_error) WHERE id = ?1", params![task, state.word(), error]).map_err(|error| error.to_string())?;
        Ledger::journal(&tx, task, None, state.word(), error.unwrap_or_default()).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn get(&self, task: &str) -> Option<Task> {
        let (contract, state, last_error) = self
            .db
            .query_row(
                "SELECT id, session_id, goal_en, original_ref, recipe, scope, deliverable, language_out, created_at, state, last_error FROM tasks WHERE id = ?1",
                params![task],
                |row| {
                    Ok((
                        TaskContract {
                            id: row.get(0)?,
                            session_id: row.get(1)?,
                            goal_en: row.get(2)?,
                            original_ref: row.get(3)?,
                            recipe: row.get(4)?,
                            scope: row.get(5)?,
                            deliverable: row.get(6)?,
                            language_out: row.get(7)?,
                            created_at: row.get(8)?,
                            done_when: Vec::new(),
                        },
                        State::of(&row.get::<_, String>(9)?),
                        row.get::<_, Option<String>>(10)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()?;
        let criteria: Vec<Criterion> = self
            .db
            .prepare("SELECT text, evidence FROM criteria WHERE task_id = ?1 ORDER BY idx")
            .and_then(|mut statement| statement.query_map(params![task], |row| Ok(Criterion { text: row.get(0)?, evidence: row.get(1)? }))?.collect())
            .unwrap_or_default();
        let steps: Vec<Step> = self
            .db
            .prepare("SELECT step_id, unit, status, attempts, evidence_refs, handover_note FROM steps WHERE task_id = ?1 ORDER BY ord")
            .and_then(|mut statement| {
                statement
                    .query_map(params![task], |row| {
                        Ok(Step {
                            step_id: row.get(0)?,
                            unit: row.get(1)?,
                            status: Status::of(&row.get::<_, String>(2)?),
                            attempts: row.get::<_, i64>(3)? as u32,
                            evidence_refs: text_list(&row.get::<_, String>(4)?),
                            handover_note: row.get(5)?,
                        })
                    })?
                    .collect()
            })
            .unwrap_or_default();
        let mut contract = contract;

        contract.done_when = criteria.iter().map(|criterion| criterion.text.clone()).collect();

        Some(Task { contract, state, last_error, criteria, steps })
    }

    /// The tasks of one session (or all), newest first.
    pub fn list(&self, session: Option<&str>, limit: usize) -> Vec<Task> {
        let ids: Vec<String> = self
            .db
            .prepare("SELECT id FROM tasks WHERE (?1 IS NULL OR session_id = ?1) ORDER BY seq DESC LIMIT ?2")
            .and_then(|mut statement| statement.query_map(params![session, limit as i64], |row| row.get(0))?.collect())
            .unwrap_or_default();

        ids.iter().filter_map(|id| self.get(id)).collect()
    }

    /// The newest task of a session that can still be continued.
    pub fn active(&self, session: &str) -> Option<Task> {
        self.list(Some(session), 8).into_iter().find(|task| task.state.unfinished())
    }

    /// The journal of a task: what happened, in order.
    pub fn journal_of(&self, task: &str) -> Vec<(String, Option<String>, String)> {
        self.db
            .prepare("SELECT ts, step_id, what FROM journal WHERE task_id = ?1 ORDER BY seq")
            .and_then(|mut statement| statement.query_map(params![task], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.collect())
            .unwrap_or_default()
    }
}

/// A fresh contract for a request.
pub fn contract_for(id: &str, session_id: &str, request: &str, original_ref: &str) -> TaskContract {
    TaskContract {
        id: id.to_string(),
        session_id: session_id.to_string(),
        goal_en: request.chars().take(600).collect(),
        original_ref: original_ref.to_string(),
        recipe: "R-ANSWER".to_string(),
        scope: "small".to_string(),
        done_when: vec!["the request is answered, or its work is done and shown".to_string()],
        deliverable: "an answer in the chat, or the changed files".to_string(),
        language_out: "same as the request".to_string(),
        created_at: now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-ledger-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        root
    }

    #[test]
    fn a_contract_and_its_steps_are_written_and_read_back() {
        let root = folder("basic");
        let mut ledger = Ledger::open(&root).unwrap();
        let contract = contract_for("task-1", "s1", "review the whole repo", "turn:t1");

        ledger.create(&contract, &["file a", "file b", "file c"]).unwrap();
        ledger.set_step("task-1", "s1", Status::Running, None, None).unwrap();
        ledger.set_step("task-1", "s1", Status::Done, Some("call-1"), Some("reviewed")).unwrap();
        ledger.set_step("task-1", "s2", Status::Blocked, None, Some("cannot read it")).unwrap();

        let task = ledger.get("task-1").unwrap();

        assert_eq!(task.counts(), Counts { done: 1, blocked: 1, remaining: 1 });
        assert_eq!(task.steps[0].attempts, 1);
        assert_eq!(task.steps[0].evidence_refs, vec!["call-1"]);
        assert_eq!(task.steps[1].handover_note, "cannot read it");
        assert_eq!(task.json()["counts"]["remaining"], 1);

        ledger.prove("task-1", 0, "call-1").unwrap();

        assert_eq!(ledger.get("task-1").unwrap().criteria[0].evidence.as_deref(), Some("call-1"));
        assert!(ledger.journal_of("task-1").len() >= 5);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The crash test: a process that dies mid-run leaves the ledger as its last committed transition; a new process sees the same states.
    #[test]
    fn killed_mid_run_the_same_step_states_come_back() {
        let root = folder("crash");

        {
            let mut ledger = Ledger::open(&root).unwrap();

            ledger.create(&contract_for("task-1", "s1", "fix all the files", "turn:t1"), &["a", "b", "c", "d"]).unwrap();
            ledger.set_step("task-1", "s1", Status::Done, Some("e1"), None).unwrap();
            ledger.set_step("task-1", "s2", Status::Done, Some("e2"), None).unwrap();
            ledger.set_step("task-1", "s3", Status::Running, None, Some("editing c")).unwrap();

            /* the process dies here: the connection is dropped without any "finish" */
            std::mem::forget(ledger);
        }

        let again = Ledger::open(&root).unwrap();
        let task = again.get("task-1").unwrap();
        let states: Vec<&str> = task.steps.iter().map(|step| step.status.word()).collect();

        assert_eq!(states, vec!["done", "done", "running", "todo"]);
        assert_eq!(task.steps[2].handover_note, "editing c");
        assert_eq!(task.state, State::Open);
        assert_eq!(again.active("s1").unwrap().contract.id, "task-1", "an unfinished task is found again");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_an_unfinished_task_is_active_and_the_newest_wins() {
        let root = folder("active");
        let mut ledger = Ledger::open(&root).unwrap();

        ledger.create(&contract_for("task-1", "s1", "one", "turn:1"), &["x"]).unwrap();
        ledger.create(&contract_for("task-2", "s1", "two", "turn:2"), &["x"]).unwrap();
        ledger.create(&contract_for("task-3", "s2", "other chat", "turn:3"), &["x"]).unwrap();
        ledger.set_state("task-2", State::Done, None).unwrap();

        assert_eq!(ledger.active("s1").unwrap().contract.id, "task-1");
        assert!(ledger.active("s9").is_none());

        ledger.set_state("task-1", State::Blocked, Some("ContextOverflow: too big")).unwrap();

        assert_eq!(ledger.active("s1").unwrap().last_error.as_deref(), Some("ContextOverflow: too big"));
        assert_eq!(ledger.list(Some("s1"), 10).len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }
}
