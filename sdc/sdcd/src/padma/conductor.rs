//! **The Conductor (P1-4, switch `padma.conductor`, off until the bench says)**: one driver for a task made of units. The states are
//! classify → contract → plan → [budget → model → gate → ledger]* → finish → report. The first two are done when the Conductor is handed a task
//! (the turn's contract is written by `task::begin`); the rest happen here, in order, for every unit.
//!
//! What it adds over a loop that runs until the model stops:
//! - a **per-unit budget** (at most [`UNIT_CALLS`] tool calls): a unit that spends it without proof is marked blocked and the rest go on;
//! - a **per-turn wall-clock budget**: when it is spent, the task is checkpointed - done units stay done, the rest stay `todo` - and the
//!   person is told to say "continue"; the next turn picks up at the first unit that is not done;
//! - `task.progress` after every unit, in the person's words ("file 37/120");
//! - a closing line that always lists done / blocked / remaining.
//!
//! The Conductor takes the work as a closure, so a recipe step (P3-1), a sub-agent or a test can be a unit. It names no model.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::engines::{EngineEvent, EventSink};

use super::ledger::{Ledger, State, Status};
use super::task::progress;

/// The most tool calls one unit may use.
pub const UNIT_CALLS: usize = 8;

/// How long one turn may run before it is checkpointed.
pub const TURN_MINUTES: u64 = 30;

/// The states a task goes through, in order. `trace` of a run lists the ones it passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Classify,
    Contract,
    Plan,
    Budget,
    Model,
    Gate,
    Ledger,
    Finish,
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub unit_calls: usize,
    pub turn: Duration,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { unit_calls: UNIT_CALLS, turn: Duration::from_secs(TURN_MINUTES * 60) }
    }
}

thread_local! {
    static OVERRIDE: RefCell<Option<Budget>> = const { RefCell::new(None) };
}

/// Runs `body` with another budget (a test, a bench or a recipe that needs a longer or shorter turn).
pub fn with_budget<R>(budget: Budget, body: impl FnOnce() -> R) -> R {
    struct Restore(Option<Budget>);

    impl Drop for Restore {
        fn drop(&mut self) {
            let before = self.0.take();

            OVERRIDE.with(|slot| *slot.borrow_mut() = before);
        }
    }

    let before = OVERRIDE.with(|slot| slot.borrow_mut().replace(budget));
    let _restore = Restore(before);

    body()
}

/// The budget in force on this thread.
pub fn budget() -> Budget {
    OVERRIDE.with(|slot| *slot.borrow()).unwrap_or_default()
}

/// What a unit of work hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitRun {
    /// How many tool calls the unit made.
    pub calls: usize,
    /// `Ok(evidence)` when the unit is proved done; `Err(reason)` when it could not be.
    pub result: Result<String, String>,
}

/// What a run came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub done: usize,
    pub blocked: usize,
    pub remaining: usize,
    /// The turn budget ended before every unit was tried; the task is checkpointed.
    pub paused: bool,
    pub trace: Vec<Phase>,
}

/// The turn's clock: when it started and what it may spend. One clock is shared by every step of a recipe.
#[derive(Clone)]
pub struct Clock {
    started: Instant,
    pub budget: Budget,
    stop: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl Clock {
    pub fn start(budget: Budget) -> Clock {
        Clock { started: Instant::now(), budget, stop: None }
    }

    /// The person can also end the turn early (Stop): the clock then reads as spent and the task is checkpointed like a turn that ran out of time.
    pub fn stoppable(mut self, stop: std::sync::Arc<dyn Fn() -> bool + Send + Sync>) -> Clock {
        self.stop = Some(stop);

        self
    }

    pub fn spent(&self) -> bool {
        self.started.elapsed() >= self.budget.turn || self.stop.as_ref().is_some_and(|stop| stop())
    }
}

/// Runs the named units of `task_id` that are not yet done, one at a time, through budget → model → gate → ledger. A unit the ledger does not have is
/// planned (added) first; a unit that is already done or blocked is left as it is (a resumed task keeps what it finished). `noun` names a unit in the
/// progress label ("file" → "file 37/120"). Returns the states passed and whether the clock ran out before every unit was tried.
pub fn run_units(ledger: &mut Ledger, task_id: &str, noun: &str, units: &[&str], sink: &EventSink, clock: &Clock, mut work: impl FnMut(&str, usize) -> UnitRun) -> (Vec<Phase>, bool) {
    let mut trace = vec![Phase::Plan];
    let known = ledger.get(task_id).map(|task| task.steps).unwrap_or_default();

    for unit in units {
        if !known.iter().any(|step| step.unit == *unit) {
            let _ = ledger.add_step(task_id, unit);
        }
    }

    let steps = ledger.get(task_id).map(|task| task.steps).unwrap_or_default();
    let total = units.len();
    let budget = clock.budget;

    for (index, unit) in units.iter().enumerate() {
        let Some(step) = steps.iter().find(|step| step.unit == *unit) else {
            continue;
        };

        if matches!(step.status, Status::Done | Status::Blocked) {
            continue;
        }

        trace.push(Phase::Budget);

        if clock.spent() {
            return (trace, true);
        }

        trace.push(Phase::Model);

        let _ = ledger.set_step(task_id, &step.step_id, Status::Running, None, None);
        let run = work(&step.unit, budget.unit_calls);

        trace.push(Phase::Gate);

        let (status, evidence, note) = match &run.result {
            Ok(evidence) if run.calls <= budget.unit_calls && !evidence.trim().is_empty() => (Status::Done, Some(evidence.as_str()), None),
            Ok(_) if run.calls > budget.unit_calls => (Status::Blocked, None, Some(format!("budget: the unit used {} tool calls, the limit is {}", run.calls, budget.unit_calls))),
            Ok(_) => (Status::Blocked, None, Some("no evidence that the unit is done".to_string())),
            Err(reason) => (Status::Blocked, None, Some(reason.clone())),
        };

        trace.push(Phase::Ledger);

        let _ = ledger.set_step(task_id, &step.step_id, status, evidence, note.as_deref());

        progress(ledger, task_id, sink, &format!("{noun} {}/{total}", index + 1));
    }

    (trace, false)
}

/// Closes the run: the task's state, the last progress event, and the closing lines (the "continue" message if paused, always done / blocked / remaining).
pub fn finish(ledger: &mut Ledger, task_id: &str, sink: &EventSink, bengali: bool, clock: &Clock, paused: bool, mut trace: Vec<Phase>) -> Outcome {
    trace.push(Phase::Finish);

    let counts = ledger.get(task_id).map(|task| task.counts()).unwrap_or_default();

    if paused {
        let _ = ledger.set_state(task_id, State::Paused, None);
        progress(ledger, task_id, sink, "paused");
    } else if counts.blocked > 0 {
        let _ = ledger.set_state(task_id, State::Blocked, Some("some units are blocked"));
        progress(ledger, task_id, sink, "blocked");
    } else {
        let _ = ledger.set_state(task_id, State::Done, None);
        progress(ledger, task_id, sink, "done");
    }

    trace.push(Phase::Report);

    if paused {
        sink.send(EngineEvent::Delta(super::english::pause_line(bengali, clock.budget.turn.as_secs() / 60)));
    }

    sink.send(EngineEvent::Delta(super::english::status_line(bengali, counts.done, counts.blocked, counts.remaining)));

    Outcome { done: counts.done, blocked: counts.blocked, remaining: counts.remaining, paused, trace }
}

/// The whole path for a task of one list of units: classify and contract are done (the caller wrote the contract), then plan, the units, finish, report.
#[allow(clippy::too_many_arguments)]
pub fn conduct(ledger: &mut Ledger, task_id: &str, noun: &str, units: &[&str], sink: &EventSink, bengali: bool, budget: Budget, work: impl FnMut(&str, usize) -> UnitRun) -> Outcome {
    let clock = Clock::start(budget);
    let (steps, paused) = run_units(ledger, task_id, noun, units, sink, &clock, work);
    let mut trace = vec![Phase::Classify, Phase::Contract];

    trace.extend(steps);

    finish(ledger, task_id, sink, bengali, &clock, paused, trace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::Recorder;
    use crate::padma::ledger::contract_for;

    fn ledger(name: &str) -> (Ledger, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("sdc-conductor-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut ledger = Ledger::open(&root).unwrap();

        ledger.create(&contract_for("t1", "s", "review every file", "turn:t1"), &[]).unwrap();

        (ledger, root)
    }

    fn labels(recorder: &Recorder) -> Vec<String> {
        recorder.events().iter().filter_map(|event| if let EngineEvent::TaskProgress { label, .. } = event { Some(label.clone()) } else { None }).collect()
    }

    fn said(recorder: &Recorder) -> String {
        recorder.events().iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect()
    }

    #[test]
    fn every_unit_goes_through_the_states_and_reports_its_place() {
        let (mut ledger, root) = ledger("all");
        let recorder = Recorder::new();
        let names: Vec<String> = (1..=120).map(|n| format!("src/f{n}.rs")).collect();
        let units: Vec<&str> = names.iter().map(String::as_str).collect();
        let outcome = conduct(&mut ledger, "t1", "file", &units, &recorder.sink(), false, Budget::default(), |unit, _| UnitRun { calls: 2, result: Ok(format!("reviewed {unit}")) });

        assert_eq!((outcome.done, outcome.blocked, outcome.remaining, outcome.paused), (120, 0, 0, false));
        assert_eq!(&outcome.trace[..3], &[Phase::Classify, Phase::Contract, Phase::Plan]);
        assert_eq!(&outcome.trace[3..8], &[Phase::Budget, Phase::Model, Phase::Gate, Phase::Ledger, Phase::Budget]);
        assert_eq!(&outcome.trace[outcome.trace.len() - 2..], &[Phase::Finish, Phase::Report]);
        assert!(labels(&recorder).contains(&"file 37/120".to_string()), "{:?}", labels(&recorder).len());
        assert_eq!(ledger.get("t1").unwrap().state, State::Done);
        assert!(said(&recorder).contains("120 done"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_unit_over_its_call_budget_or_without_proof_is_blocked_and_the_rest_go_on() {
        let (mut ledger, root) = ledger("blocked");
        let recorder = Recorder::new();
        let outcome = conduct(&mut ledger, "t1", "file", &["a", "b", "c", "d"], &recorder.sink(), false, Budget::default(), |unit, limit| match unit {
            "a" => UnitRun { calls: limit + 1, result: Ok("looked".into()) },
            "b" => UnitRun { calls: 1, result: Ok("  ".into()) },
            "c" => UnitRun { calls: 1, result: Err("could not read it".into()) },
            _ => UnitRun { calls: 1, result: Ok("reviewed d".into()) },
        });
        let task = ledger.get("t1").unwrap();

        assert_eq!((outcome.done, outcome.blocked), (1, 3));
        assert!(task.steps[0].handover_note.contains("budget") && task.steps[1].handover_note.contains("no evidence") && task.steps[2].handover_note.contains("could not read"));
        assert_eq!(task.state, State::Blocked);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_spent_turn_budget_checkpoints_and_the_next_turn_does_not_redo_finished_units() {
        let (mut ledger, root) = ledger("pause");
        let recorder = Recorder::new();
        let units = ["a", "b", "c"];
        let mut seen = Vec::new();
        /* Wide enough that a slow CI runner's setup does not spend it before the first unit (1.0.9: the Windows
           release runner paused with 0 done at 300 ms), and one unit always spends it. */
        let short = Budget { unit_calls: UNIT_CALLS, turn: Duration::from_millis(2_000) };
        let first = conduct(&mut ledger, "t1", "file", &units, &recorder.sink(), false, short, |unit, _| {
            seen.push(unit.to_string());
            std::thread::sleep(Duration::from_millis(2_100));

            UnitRun { calls: 1, result: Ok(format!("reviewed {unit}")) }
        });

        assert!(first.paused && first.done == 1 && first.remaining == 2, "{first:?}");
        assert_eq!(ledger.get("t1").unwrap().state, State::Paused);
        assert!(said(&recorder).contains("continue") && said(&recorder).contains("1 done · 0 blocked · 2 remaining"), "{}", said(&recorder));

        let second = conduct(&mut ledger, "t1", "file", &units, &recorder.sink(), false, Budget::default(), |unit, _| {
            seen.push(unit.to_string());

            UnitRun { calls: 1, result: Ok(format!("reviewed {unit}")) }
        });

        assert!(!second.paused && second.done == 3 && second.remaining == 0, "{second:?}");
        assert_eq!(seen, vec!["a", "b", "c"], "a finished unit is not run again");
        assert_eq!(ledger.get("t1").unwrap().state, State::Done);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_closing_lines_are_in_the_persons_language() {
        let (mut ledger, root) = ledger("bn");
        let recorder = Recorder::new();
        let short = Budget { unit_calls: UNIT_CALLS, turn: Duration::ZERO };

        conduct(&mut ledger, "t1", "file", &["a"], &recorder.sink(), true, short, |_, _| unreachable!("the budget was spent before the first unit"));

        assert!(said(&recorder).contains("continue") && said(&recorder).contains("কাজ:"), "{}", said(&recorder));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_budget_set_for_a_scope_is_gone_after_it() {
        let short = Budget { unit_calls: 2, turn: Duration::from_secs(1) };

        assert_eq!(budget(), Budget::default());
        with_budget(short, || assert_eq!(budget(), short));
        assert_eq!(budget(), Budget::default());
    }
}
