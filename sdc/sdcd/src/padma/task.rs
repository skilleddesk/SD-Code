//! **The task of a turn (P1-1, switch `padma.ledger`)**: a turn that works in a folder is written down before the model is asked - a
//! contract and one step - and the ledger follows the turn's events: every edit or command that succeeded is evidence for the step, the end of
//! the turn closes it (done, blocked, or paused with a handover note). The events pass through unchanged; this only watches them, and a ledger
//! that cannot be written never stops the work.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::engines::{EngineEvent, EventSink, Prompt};

use super::ledger::{contract_for, Ledger, State, Status};

#[derive(Default)]
struct Seen {
    started: HashMap<String, String>,
    evidence: Vec<String>,
    finished: bool,
}

struct Shared {
    ledger: Mutex<Ledger>,
    task_id: String,
    seen: Mutex<Seen>,
    sink: EventSink,
    /// The person's status lines are owed in Bengali (P2-4).
    bengali: bool,
}

/// Follows one turn. Cheap to clone; the clone inside the wrapped sink and the one the caller keeps are the same task.
#[derive(Clone)]
pub struct Tracker(Arc<Shared>);

thread_local! {
    static CONTRACTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

struct Leave;

impl Drop for Leave {
    fn drop(&mut self) {
        CONTRACTS.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Runs a turn that has a contract: the Finish Gate (P1-2) holds only such a turn, so a bare `drive` call - a probe, a sub-agent - is not held.
pub fn with_contract<R>(turn: impl FnOnce() -> R) -> R {
    CONTRACTS.with(|depth| depth.set(depth.get() + 1));

    let _leave = Leave;

    turn()
}

/// Whether the code running now is inside a turn that has a contract.
pub fn contract_active() -> bool {
    CONTRACTS.with(|depth| depth.get() > 0)
}

/// The person's own words of a prompt (the context pack in front of them is not the goal).
pub fn request_of(prompt: &Prompt) -> String {
    let words = prompt.text.rsplit("[The person's message]").next().unwrap_or(&prompt.text).trim();

    words.strip_prefix(super::english::NOTE_BN).unwrap_or(words).trim().to_string()
}

/// The id a turn's task has.
pub fn id_of(turn_id: &str) -> String {
    format!("task-{turn_id}")
}

pub(super) fn progress(ledger: &Ledger, task_id: &str, sink: &EventSink, label: &str) {
    if let Some(task) = ledger.get(task_id) {
        let counts = task.counts();

        sink.send(EngineEvent::TaskProgress {
            task_id: task_id.to_string(),
            done: counts.done as u64,
            blocked: counts.blocked as u64,
            total: task.steps.len() as u64,
            label: label.to_string(),
            recipe: task.contract.recipe.clone(),
            findings: Vec::new(),
            local: 0,
            cloud: 0,
        });
    }
}

/// What a message is to the session's unfinished task (P1-5, switch `padma.continue_bind`).
#[allow(clippy::large_enum_variant)]
pub enum Bound {
    /// A new request: a new contract.
    Fresh,
    /// "continue" or a pasted error: this turn goes on with the task, and its prompt says so.
    Resume { task_id: String, prompt: Prompt },
    /// A question about why it stopped: the answer comes from the ledger, no model is asked.
    Explain(String),
}

/// Reads the message against the session's unfinished task, before a contract is written.
pub fn bind(root: &str, prompt: &Prompt) -> Bound {
    use super::bind::{explain, resolve, ActiveTask, Binding};

    if !crate::v3::on("padma.continue_bind") {
        return Bound::Fresh;
    }

    let Ok(ledger) = Ledger::open(std::path::Path::new(root)) else {
        return Bound::Fresh;
    };
    let Some(task) = ledger.active(&prompt.session_id) else {
        return Bound::Fresh;
    };
    let active = ActiveTask { id: task.contract.id.clone(), goal: task.contract.goal_en.clone(), last_error: task.last_error.clone() };
    let message = request_of(prompt);
    let note = task.steps.iter().map(|step| step.handover_note.as_str()).find(|note| !note.is_empty()).unwrap_or("");

    match resolve(&message, Some(&active)) {
        Binding::New => Bound::Fresh,
        Binding::Explain(_) => Bound::Explain(explain(&active, note)),
        Binding::Resume(id) => {
            let stopped = match &task.last_error {
                Some(error) => format!(" It stopped with: {error}."),
                None if !note.is_empty() => format!(" It stopped: {note}."),
                None => String::new(),
            };
            let instruction = format!(
                "Continue the unfinished task: \"{}\".{stopped} The person said: \"{message}\". Pick up where it stopped; do not start over and do not redo what is already done.",
                task.contract.goal_en
            );
            let mut resumed = prompt.clone();

            resumed.text = match prompt.text.rfind("[The person's message]") {
                Some(at) => format!("{}[The person's message]\n{instruction}", &prompt.text[..at]),
                None => instruction,
            };

            Bound::Resume { task_id: id, prompt: resumed }
        }
    }
}

/// Writes the turn's contract and returns the tracker with the sink the turn must use. `None` when the ledger cannot be opened: the turn
/// then runs exactly as it did. With `resume`, the turn goes on with that task instead of starting a new one.
pub fn begin(root: &str, prompt: &Prompt, sink: &EventSink, resume: Option<&str>) -> Option<(Tracker, EventSink)> {
    let mut ledger = Ledger::open(std::path::Path::new(root)).ok()?;
    let resumed = resume.filter(|id| ledger.get(id).is_some());
    let task_id = match resumed {
        Some(id) => id.to_string(),
        None => id_of(&prompt.turn_id),
    };

    if resumed.is_some() {
        ledger.set_state(&task_id, State::Open, None).ok()?;
    } else {
        let request = request_of(prompt);
        let mut contract = contract_for(&task_id, &prompt.session_id, &request, &format!("turn:{}", prompt.turn_id));

        contract.done_when = super::gate::criteria(&request);
        contract.language_out = super::english::language_out(prompt).to_string();

        if crate::v3::on("padma.scope") {
            let scope = super::scope::classify(&request, &super::scope::RepoStats::of(std::path::Path::new(root)));

            contract.recipe = scope.recipe.id().to_string();
            contract.scope = if scope.large { "large" } else { "small" }.to_string();
        }

        ledger.create(&contract, &["do the request"]).ok()?;
    }

    ledger.set_step(&task_id, "s1", Status::Running, None, None).ok()?;
    progress(&ledger, &task_id, sink, if resumed.is_some() { "resumed" } else { "started" });

    let bengali = ledger.get(&task_id).is_some_and(|task| task.contract.language_out == "bn");
    let tracker = Tracker(Arc::new(Shared { ledger: Mutex::new(ledger), task_id, seen: Mutex::new(Seen::default()), sink: sink.clone(), bengali }));
    let watcher = tracker.clone();
    let inner = sink.clone();
    let wrapped = EventSink::new(move |event| {
        watcher.watch(&event);
        inner.send(event);
    });

    Some((tracker, wrapped))
}

impl Tracker {
    fn watch(&self, event: &EngineEvent) {
        let shared = &self.0;
        let Ok(mut seen) = shared.seen.lock() else {
            return;
        };

        match event {
            EngineEvent::ToolStarted { call_id, tool, .. } => {
                seen.started.insert(call_id.clone(), tool.clone());
            }
            EngineEvent::ToolCompleted { call_id, status, .. } if status == "done" => {
                if seen.started.get(call_id).is_some_and(|tool| tool == "edit" || tool == "run") {
                    seen.evidence.push(call_id.clone());

                    if let Ok(mut ledger) = shared.ledger.lock() {
                        let _ = ledger.set_step(&shared.task_id, "s1", Status::Running, Some(call_id), None);
                    }
                }
            }
            EngineEvent::Done { summary, .. } if !seen.finished => {
                seen.finished = true;
                self.close(summary, None, &seen.evidence, true);
            }
            EngineEvent::Failed(reason) if !seen.finished => {
                seen.finished = true;
                self.close("Blocked", Some(reason), &seen.evidence, true);
            }
            _ => {}
        }
    }

    fn close(&self, summary: &str, error: Option<&str>, evidence: &[String], say: bool) {
        let shared = &self.0;
        let Ok(mut ledger) = shared.ledger.lock() else {
            return;
        };
        let (status, state, note) = if summary.starts_with("Blocked") {
            (Status::Blocked, State::Blocked, summary.to_string())
        } else if summary.starts_with("Paused") || summary == "Interrupted" || summary == "Force killed" {
            (Status::Todo, State::Paused, format!("{summary}: say \"continue\" to go on"))
        } else {
            (Status::Done, State::Done, String::new())
        };
        let last = evidence.last().map(String::as_str);

        let _ = ledger.set_step(&shared.task_id, "s1", status, last, if note.is_empty() { None } else { Some(note.as_str()) });

        if status == Status::Done {
            let _ = ledger.prove(&shared.task_id, 0, last.unwrap_or("answered"));
        }

        let _ = ledger.set_state(&shared.task_id, state, error);

        progress(&ledger, &shared.task_id, &shared.sink, state.word());

        /* P1-2: a turn that did not finish says where the task stands - what is done, what is blocked, what is left. */
        if say && state != State::Done && crate::v3::on("padma.finish_gate") {
            if let Some(task) = ledger.get(&shared.task_id) {
                let counts = task.counts();

                shared.sink.send(EngineEvent::Delta(super::english::status_line(shared.bengali, counts.done, counts.blocked, counts.remaining)));
            }
        }
    }

    /// Works on the task's ledger (a recipe run, P3-2). Holds the ledger for the whole of `work`, so `work` must send events to the raw sink, never through
    /// the wrapped one.
    pub fn with_ledger<R>(&self, work: impl FnOnce(&mut Ledger, &str) -> R) -> Option<R> {
        let mut ledger = self.0.ledger.lock().ok()?;

        Some(work(&mut ledger, &self.0.task_id))
    }

    /// Whether the person is answered in Bengali.
    pub fn bengali(&self) -> bool {
        self.0.bengali
    }

    /// A recipe has set the task's rows and state itself; the turn's end must not rewrite them.
    pub fn settled(&self) {
        if let Ok(mut seen) = self.0.seen.lock() {
            seen.finished = true;
        }
    }

    /// The turn's code returned. A turn that ended with neither `Done` nor `Failed` is paused, not lost.
    pub fn finish(&self) {
        let (open, evidence) = match self.0.seen.lock() {
            Ok(mut seen) => (!std::mem::replace(&mut seen.finished, true), seen.evidence.clone()),
            Err(_) => (false, Vec::new()),
        };

        if open {
            self.close("Interrupted", None, &evidence, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::Recorder;

    fn prompt(root: &str) -> Prompt {
        Prompt {
            session_id: "s".into(),
            turn_id: "t1".into(),
            text: "context\n[The person's message]\nfix the bug in a.py".into(),
            model: "m".into(),
            provider: None,
            history: Vec::new(),
            project_root: Some(root.into()),
            remote: None,
            autonomy: Default::default(),
            resume: None,
            images: Vec::new(),
            effort: None,
        }
    }

    fn folder(name: &str) -> String {
        let root = std::env::temp_dir().join(format!("sdc-task-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        root.to_string_lossy().to_string()
    }

    fn run_turn(root: &str, events: Vec<EngineEvent>, finish: bool) -> Vec<EngineEvent> {
        let recorder = Recorder::new();
        let (tracker, sink) = begin(root, &prompt(root), &recorder.sink(), None).unwrap();

        for event in events {
            sink.send(event);
        }

        if finish {
            tracker.finish();
        }

        recorder.events()
    }

    fn tool(id: &str, kind: &str, status: &str) -> Vec<EngineEvent> {
        vec![
            EngineEvent::ToolStarted { call_id: id.into(), tool: kind.into(), name: "x".into(), target: "a.py".into() },
            EngineEvent::ToolCompleted { call_id: id.into(), status: status.into(), meta: String::new(), diff: None },
        ]
    }

    fn done(summary: &str) -> EngineEvent {
        EngineEvent::Done { summary: summary.into(), meta: String::new(), pass: None }
    }

    #[test]
    fn a_finished_turn_is_a_done_task_with_its_edit_as_evidence() {
        let root = folder("done");
        let mut events = tool("c1", "read", "done");

        events.extend(tool("c2", "edit", "done"));
        events.push(done("Fixed"));

        let seen = run_turn(&root, events, true);
        let ledger = Ledger::open(std::path::Path::new(&root)).unwrap();
        let task = ledger.get(&id_of("t1")).unwrap();

        assert_eq!(task.state, State::Done);
        assert_eq!(task.contract.goal_en, "fix the bug in a.py");
        assert_eq!(task.steps[0].evidence_refs, vec!["c2".to_string()], "only the edit is evidence, not the read");
        assert!(task.criteria[0].evidence.is_some());
        assert!(seen.iter().any(|event| matches!(event, EngineEvent::TaskProgress { done: 1, total: 1, .. })));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_or_blocked_turn_is_blocked_and_keeps_the_error() {
        let root = folder("failed");

        run_turn(&root, vec![EngineEvent::Failed("ContextOverflow: too long".into())], true);

        let ledger = Ledger::open(std::path::Path::new(&root)).unwrap();
        let task = ledger.get(&id_of("t1")).unwrap();

        assert_eq!(task.state, State::Blocked);
        assert_eq!(task.last_error.as_deref(), Some("ContextOverflow: too long"));
        assert_eq!(task.steps[0].status, Status::Blocked);
        assert!(ledger.active("s").is_some(), "a blocked task can be continued");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_turn_that_just_stops_is_paused_with_a_handover_note() {
        let root = folder("paused");

        run_turn(&root, tool("c1", "edit", "done"), true);

        let ledger = Ledger::open(std::path::Path::new(&root)).unwrap();
        let task = ledger.get(&id_of("t1")).unwrap();

        assert_eq!(task.state, State::Paused);
        assert!(task.steps[0].handover_note.contains("continue"));
        assert_eq!(task.steps[0].evidence_refs, vec!["c1".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    fn said(text: &str, turn: &str) -> Prompt {
        Prompt { turn_id: turn.into(), text: format!("context\n[The person's message]\n{text}"), ..prompt("") }
    }

    #[test]
    fn continue_after_a_block_resumes_the_same_task_and_a_finished_one_is_not_resumed() {
        let root = folder("bind");

        run_turn(&root, vec![EngineEvent::Failed("ContextOverflow: too long".into())], true);

        let Bound::Resume { task_id, prompt: resumed } = bind(&root, &said("ok koro", "t2")) else {
            panic!("ok koro must resume");
        };

        assert_eq!(task_id, id_of("t1"));
        assert!(resumed.text.contains("Continue the unfinished task") && resumed.text.contains("fix the bug in a.py") && resumed.text.contains("ContextOverflow"));
        assert!(resumed.text.starts_with("context\n"), "the context pack stays in front");

        let recorder = Recorder::new();
        let (tracker, sink) = begin(&root, &resumed, &recorder.sink(), Some(&task_id)).unwrap();

        sink.send(done("Fixed"));
        tracker.finish();

        let ledger = Ledger::open(std::path::Path::new(&root)).unwrap();

        assert_eq!(ledger.list(Some("s"), 10).len(), 1, "no second task was made");
        assert_eq!(ledger.get(&task_id).unwrap().state, State::Done);
        assert!(matches!(bind(&root, &said("continue", "t3")), Bound::Fresh), "a finished task is not resumed");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_new_request_next_to_an_unfinished_task_is_fresh_and_a_why_is_explained() {
        let root = folder("bind-new");

        run_turn(&root, vec![EngineEvent::Failed("ollama failed: reset".into())], true);

        assert!(matches!(bind(&root, &said("add a login page", "t2")), Bound::Fresh));
        assert!(matches!(bind(&root, &said("why did it stop?", "t2")), Bound::Explain(text) if text.contains("ollama failed")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_tool_is_not_evidence() {
        let root = folder("tool-failed");

        run_turn(&root, [tool("c1", "edit", "failed"), vec![done("Done")]].concat(), true);

        let ledger = Ledger::open(std::path::Path::new(&root)).unwrap();

        assert!(ledger.get(&id_of("t1")).unwrap().steps[0].evidence_refs.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }
}
