//! `sdcd bench resume --kills 100` (v3 A-19, local part): a scripted turn is killed (`SIGKILL`, no warning) at a
//! random moment, the daemon is started again, and the bench checks that the turn finished from its journal -
//! every file right, nothing lost, and the one command that must not run twice ran at most once.
//!
//! The model is a scripted OpenAI-compatible provider inside this process ([`MockProvider`]): no key, no cost,
//! and the same five steps every time:
//!
//! 1. `write_file notes/a.txt "alpha"`   2. `run_command "sleep 0.3 && echo run >> ran.log"` (not idempotent)
//! 3. `write_file notes/b.txt "beta"`    4. `read_file notes/a.txt`                          5. the answer.
//!
//! The provider picks the next step from the number of tool answers in the conversation it is sent, so it
//! continues correctly from any conversation a resume rebuilds - which is what is being tested.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use super::runner::{Client, Daemon};

/// The scripted provider.
pub struct MockProvider {
    pub port: u16,
}

fn step_reply(step: usize) -> Vec<Value> {
    let call = |name: &str, arguments: Value| {
        vec![
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": format!("call-{step}"), "function": { "name": name, "arguments": arguments.to_string() } }] } }] }),
            json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }),
        ]
    };

    let mut chunks = match step {
        0 => call("write_file", json!({ "path": "notes/a.txt", "content": "alpha\n" })),
        1 => call("run_command", json!({ "command": "sleep 0.3 && echo run >> ran.log" })),
        2 => call("write_file", json!({ "path": "notes/b.txt", "content": "beta\n" })),
        3 => call("read_file", json!({ "path": "notes/a.txt" })),
        _ => vec![json!({ "choices": [{ "delta": { "content": "Done: a.txt, b.txt and the build log." } }] }), json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] })],
    };

    chunks.push(json!({ "choices": [], "usage": { "prompt_tokens": 500 + step * 100, "completion_tokens": 20 } }));
    chunks
}

impl MockProvider {
    /// Starts the provider on a free port; it serves until the process ends. `delay` is each reply's wait.
    pub fn start(delay: Duration) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();

        std::thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let _ = serve(socket, delay);
                });
            }
        });

        Ok(Self { port })
    }

    /// The `local-servers.json` that makes it a provider for a daemon whose data folder is `dir`.
    pub fn register(&self, dir: &Path) -> std::io::Result<String> {
        let id = format!("local-127-0-0-1-{}", self.port);
        let servers = json!([{ "id": id, "label": "bench mock", "base": format!("http://127.0.0.1:{}/v1", self.port), "models": [{ "id": "mock", "ctx": 64_000 }] }]);

        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("local-servers.json"), servers.to_string())?;

        Ok(id)
    }
}

fn serve(socket: std::net::TcpStream, delay: Duration) -> std::io::Result<()> {
    let mut reader = BufReader::new(socket.try_clone()?);
    let mut first = String::new();
    let mut length = 0usize;

    reader.read_line(&mut first)?;

    loop {
        let mut line = String::new();

        reader.read_line(&mut line)?;

        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }

        if line == "\r\n" || line.is_empty() {
            break;
        }
    }

    let mut body = vec![0u8; length];

    reader.read_exact(&mut body)?;

    let mut socket = socket;

    if first.starts_with("GET") {
        let models = json!({ "data": [{ "id": "mock", "object": "model" }] }).to_string();

        return write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{models}", models.len());
    }

    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let answers = request["messages"].as_array().map(|messages| messages.iter().filter(|message| message["role"] == "tool").count()).unwrap_or(0);

    std::thread::sleep(delay);
    write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n")?;

    for chunk in step_reply(answers) {
        write!(socket, "data: {chunk}\n\n")?;
    }

    write!(socket, "data: [DONE]\n\n")
}

/// One kill-and-restart trial.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Trial {
    pub index: usize,
    pub kill_after_ms: u64,
    /// The daemon was killed before the turn's journal began: nothing to resume.
    pub not_started: bool,
    pub resumed: bool,
    pub finished: bool,
    pub files_right: bool,
    pub command_runs: usize,
    pub seconds_to_finish: f64,
    pub note: String,
}

impl Trial {
    /// A trial counts as a correct resume when the work came out whole and the command did not run twice.
    pub fn correct(&self) -> bool {
        self.not_started || (self.finished && self.files_right && self.command_runs <= 1)
    }
}

fn open_turns(database: &Path) -> Option<(i64, i64)> {
    let connection = rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let started: i64 = connection.query_row("SELECT COUNT(*) FROM job_events WHERE kind = 'start'", [], |row| row.get(0)).ok()?;
    let open: i64 = connection
        .query_row("SELECT COUNT(*) FROM job_events WHERE kind = 'start' AND turn_id NOT IN (SELECT turn_id FROM job_events WHERE kind = 'end')", [], |row| row.get(0))
        .ok()?;

    Some((started, open))
}

fn start_turn(client: &mut Client, provider: &str, work: &Path) -> Result<String, String> {
    let added = client.call("project.add", json!({ "root": work.display().to_string(), "hostId": "local" }))?;
    let project = added["projectId"].as_str().or(added["project"]["id"].as_str()).map(str::to_string);
    let session = client.call("session.open", json!({ "hostId": "local", "title": "resume trial", "projectId": project }))?;
    let session = session["sessionId"].as_str().unwrap_or_default().to_string();

    client.call(
        "engine.start",
        json!({ "sessionId": session, "prompt": "Make notes a and b and run the build once.", "engine": "native_api", "agent": true, "autonomy": "auto", "model": "mock", "provider": provider }),
    )?;

    Ok(session)
}

fn check_files(work: &Path) -> (bool, usize) {
    let read = |name: &str| std::fs::read_to_string(work.join(name)).unwrap_or_default();
    let runs = read("ran.log").lines().filter(|line| line.trim() == "run").count();

    (read("notes/a.txt") == "alpha\n" && read("notes/b.txt") == "beta\n", runs)
}

/// Runs the trials. `binary` is the daemon under test.
pub fn trials(binary: &Path, kills: usize, scratch: &Path) -> Result<Vec<Trial>, String> {
    let provider = MockProvider::start(Duration::from_millis(120)).map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };

    /* How long the whole turn takes when nothing goes wrong: kills are spread over that. */
    let full = {
        let (work, data) = (scratch.join("calibrate-work"), scratch.join("calibrate-data"));
        let started = Instant::now();
        let trial = one(binary, &provider, &work, &data, None)?;

        if !trial.finished || !trial.files_right {
            return Err(format!("the uninterrupted turn did not finish correctly: {}", trial.note));
        }

        started.elapsed().as_millis() as u64
    };

    for index in 0..kills {
        let kill_after = 50 + next() % full.max(200);
        let (work, data) = (scratch.join(format!("work-{index}")), scratch.join(format!("data-{index}")));
        let mut trial = one(binary, &provider, &work, &data, Some(Duration::from_millis(kill_after)))?;

        trial.index = index;
        trial.kill_after_ms = kill_after;
        println!(
            "[{}/{kills}] kill at {kill_after:>5} ms: {}{}",
            index + 1,
            if trial.correct() { "ok" } else { "WRONG" },
            if trial.not_started { " (killed before the turn began)".to_string() } else { format!(" · finished {} · files {} · command ran {}× · {:.1}s", trial.finished, trial.files_right, trial.command_runs, trial.seconds_to_finish) }
        );
        let correct = trial.correct();

        results.push(trial);

        if correct || std::env::var_os("SDC_BENCH_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&work);
            let _ = std::fs::remove_dir_all(&data);
        } else {
            println!("      kept {} and {}", work.display(), data.display());
        }
    }

    Ok(results)
}

fn one(binary: &Path, provider: &MockProvider, work: &Path, data: &Path, kill_after: Option<Duration>) -> Result<Trial, String> {
    let _ = std::fs::remove_dir_all(work);
    let _ = std::fs::remove_dir_all(data);
    std::fs::create_dir_all(work).map_err(|error| error.to_string())?;

    let work: PathBuf = std::fs::canonicalize(work).map_err(|error| error.to_string())?;
    let provider_id = provider.register(data).map_err(|error| error.to_string())?;
    let mut trial = Trial { index: 0, kill_after_ms: 0, not_started: false, resumed: false, finished: false, files_right: false, command_runs: 0, seconds_to_finish: 0.0, note: String::new() };
    let database = data.join("bench.db");
    let mut daemon = Daemon::start(binary, data, None, Some(data))?;
    let mut client = Client::connect(daemon.port)?;

    start_turn(&mut client, &provider_id, &work)?;

    let started = Instant::now();

    if let Some(wait) = kill_after {
        std::thread::sleep(wait);
        drop(client);
        daemon.stop();

        match open_turns(&database) {
            Some((0, _)) | None => {
                trial.not_started = true;
                return Ok(trial);
            }
            Some((_, 0)) => trial.note = "the turn had finished before the kill".into(),
            Some(_) => trial.resumed = true,
        }

        daemon = Daemon::start(binary, data, None, Some(data))?;
    }

    /* Done when every journaled turn has ended and the files are there (or 60 s pass). */
    let deadline = Instant::now() + Duration::from_secs(60);

    loop {
        let (files, runs) = check_files(&work);
        let finished = matches!(open_turns(&database), Some((started, 0)) if started > 0);

        if (finished && files) || Instant::now() > deadline {
            trial.finished = finished;
            trial.files_right = files;
            trial.command_runs = runs;
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    trial.seconds_to_finish = started.elapsed().as_secs_f64();
    daemon.stop();

    if !trial.finished {
        trial.note = format!("did not finish; daemon log {}", daemon.log().display());
    }

    Ok(trial)
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub kills: usize,
    pub not_started: usize,
    pub resumed: usize,
    pub correct: usize,
    pub correct_rate: f64,
    pub damaged: usize,
    pub duplicated_commands: usize,
}

pub fn summarise(trials: &[Trial]) -> Summary {
    let counted: Vec<&Trial> = trials.iter().filter(|trial| !trial.not_started).collect();
    let correct = counted.iter().filter(|trial| trial.correct()).count();

    Summary {
        kills: trials.len(),
        not_started: trials.len() - counted.len(),
        resumed: counted.iter().filter(|trial| trial.resumed).count(),
        correct,
        correct_rate: if counted.is_empty() { 0.0 } else { correct as f64 / counted.len() as f64 },
        damaged: counted.iter().filter(|trial| trial.finished && !trial.files_right).count(),
        duplicated_commands: counted.iter().filter(|trial| trial.command_runs > 1).count(),
    }
}

/// One line of a runner turn: what the daemon writes while the turn's tools run over ssh (P0-4).
#[derive(Clone, Copy, Debug)]
enum Op {
    /// The Step Runtime introduces a step; the reserved `step_id` column carries it from here on.
    Step(usize),
    /// A reply from the model that asked for this many tool calls on the runner.
    Reply(usize, usize),
    /// A result that came back from the runner for call `index` of that step.
    Result(usize, usize),
    /// The tool-answer messages the daemon appends once the results are in.
    Answers,
}

/// A runner turn: four steps, the first asking for two calls and the second for one (the shape R4's turns have).
fn runner_script() -> Vec<Op> {
    vec![
        Op::Step(1),
        Op::Reply(1, 2),
        Op::Result(1, 0),
        Op::Result(1, 1),
        Op::Answers,
        Op::Step(2),
        Op::Reply(2, 1),
        Op::Result(2, 0),
        Op::Answers,
        Op::Step(3),
        Op::Reply(3, 0),
    ]
}

const ANSWERS_PER_STEP: usize = 2;

fn runner_call(step: usize, index: usize) -> String {
    format!("c{step}-{index}")
}

/// What a resume of the first `ops` records has to say, worked out from the script alone. Written out here a
/// second time on purpose: a resume is only correct against an expectation that was not read off the resume.
#[derive(Debug, PartialEq)]
struct Expected {
    step_id: String,
    messages: usize,
    calls: usize,
    answered: usize,
}

fn expected(ops: &[Op]) -> Expected {
    let mut step_id = String::new();
    let mut announced = String::new();
    let mut messages = 0;
    let mut calls = 0;
    let mut answered = 0;

    for op in ops {
        match op {
            /* A step that announced itself and then wrote nothing left no record: the journal can only know the
               step a record belongs to, which is why `step_id` is a column and not a separate record. */
            Op::Step(number) => announced = format!("step-{number}"),
            /* A reply is one message, and it is what a crash leaves as the outstanding calls. */
            Op::Reply(_, count) => {
                step_id = announced.clone();
                messages += 1;
                calls = *count;
                answered = 0;
            }
            Op::Result(..) => {
                step_id = announced.clone();
                answered += 1;
            }
            /* The tool answers carry the results, so the outstanding calls are spent with them. */
            Op::Answers => {
                step_id = announced.clone();
                messages += ANSWERS_PER_STEP;
                calls = 0;
                answered = 0;
            }
        }
    }

    Expected { step_id, messages, calls, answered }
}

/// One kill-and-resume of a runner turn.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RunnerTrial {
    pub index: usize,
    /// How many of the turn's records had been written when the writer was killed.
    pub kill_after: usize,
    /// Whether the tail record was left damaged, as a kill during the last write leaves it.
    pub damaged_tail: bool,
    pub origin_kept: bool,
    pub step_kept: bool,
    pub messages_kept: bool,
    pub calls_kept: bool,
    pub answered_kept: bool,
    pub damaged_counted: bool,
    pub note: String,
}

impl RunnerTrial {
    pub fn correct(&self) -> bool {
        self.origin_kept && self.step_kept && self.messages_kept && self.calls_kept && self.answered_kept && self.damaged_counted
    }
}

/// Kills a runner turn at `points` places, `repeats` times each, and rebuilds it every time.
///
/// This is the VPS half of P0-4. The records, the writer and the replay are the ones the daemon really uses;
/// what is missing against a live trial is only the transport, because a kill -9 trial over ssh needs a VPS to
/// run against and this machine has none. `sdcd bench resume` reports the two halves separately for that reason.
pub fn runner_trials(root: &std::path::Path, points: usize, repeats: usize, seed: u64) -> Vec<RunnerTrial> {
    use crate::v3::journal::{Origin, Writer};

    let script = runner_script();
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut trials = Vec::new();
    let mut index = 0;

    for point in 0..points {
        for repeat in 0..repeats {
            /* A fresh store every trial: a resume is what the journal says, and a journal that already has a
               turn in it would make the answer come from somewhere else. */
            let dir = root.join(format!("runner-{point}-{repeat}"));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::create_dir_all(&dir);

            let store = match crate::store::sqlite::Store::open(&dir.join("sdc.db")) {
                Ok(store) => std::sync::Arc::new(store),
                Err(error) => {
                    trials.push(RunnerTrial {
                        index,
                        kill_after: point,
                        damaged_tail: false,
                        origin_kept: false,
                        step_kept: false,
                        messages_kept: false,
                        calls_kept: false,
                        answered_kept: false,
                        damaged_counted: false,
                        note: format!("the store would not open: {error}"),
                    });
                    index += 1;
                    continue;
                }
            };

            let start = serde_json::json!({ "prompt": "fix the failing test", "remote": "vps.example" });
            let Some(mut writer) = Writer::on_runner(store.clone(), &format!("t-{point}-{repeat}"), "s-1", start) else {
                continue;
            };

            /* Walk the script to the kill point: everything up to here was written before the power went. */
            let kill_after = if point == 0 { 0 } else { (next() as usize) % script.len() + 1 };
            let mut steps_done = 0usize;

            for op in script.iter().take(kill_after) {
                match *op {
                    Op::Step(number) => {
                        writer.step(&format!("step-{number}"));
                        steps_done = number;
                    }
                    Op::Reply(step, count) => {
                        let calls: Vec<crate::agent::dialect::ToolUse> = (0..count)
                            .map(|slot| crate::agent::dialect::ToolUse { id: runner_call(step, slot), name: "Run".into(), input: serde_json::json!({ "shell": "make test" }) })
                            .collect();

                        writer.model(step, &serde_json::json!({ "role": "assistant", "content": format!("step {step}") }), &calls);
                    }
                    Op::Result(step, slot) => {
                        writer.result(step, &runner_call(step, slot), "Run", "ok", false);
                    }
                    Op::Answers => {
                        let answers: Vec<serde_json::Value> = (0..ANSWERS_PER_STEP)
                            .map(|slot| serde_json::json!({ "role": "tool", "content": format!("result {slot}") }))
                            .collect();

                        writer.messages(steps_done, &answers);
                    }
                }
            }

            /* The kill: no `end` record, exactly what `kill -9` leaves behind. */
            std::mem::forget(writer);

            /* One trial in three is left with the last write half done. */
            let damaged_tail = next() % 3 == 0 && kill_after > 0;

            if damaged_tail {
                let _ = store
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE job_events SET payload = substr(payload, 1, length(payload) / 2) WHERE id = (SELECT MAX(id) FROM job_events)",
                        [],
                    );
            }

            /* A damaged tail loses one *record*, and a `Step` op writes no record - only the record after it
               carries that step - so the walk back skips the steps that announced themselves and wrote nothing. */
            let surviving = if damaged_tail {
                let mut end = kill_after - 1;

                while end > 0 && matches!(script[end], Op::Step(_)) {
                    end -= 1;
                }

                &script[..end]
            } else {
                &script[..kill_after]
            };
            /* The tail record is damaged, not gone: its payload is unreadable but its *columns* still are, and
               `step_id` is a column. So the step it announced is the step the resume reports, while the messages,
               calls and answers come from the records that still read back. */
            let mut want = expected(&script[..kill_after]);

            if damaged_tail {
                let read_back = expected(surviving);

                want.messages = read_back.messages;
                want.calls = read_back.calls;
                want.answered = read_back.answered;
            }
            let rebuilt = crate::v3::journal::rebuild(&store, &format!("t-{point}-{repeat}"));

            let trial = match rebuilt {
                None => RunnerTrial {
                    index,
                    kill_after,
                    damaged_tail,
                    origin_kept: false,
                    step_kept: false,
                    messages_kept: false,
                    calls_kept: false,
                    answered_kept: false,
                    damaged_counted: false,
                    note: "the turn could not be rebuilt at all".into(),
                },
                Some((_, resumed)) => {
                    let origin_kept = resumed.origin == Origin::Vps;
                    let step_kept = resumed.step_id == want.step_id;
                    let messages_kept = resumed.messages.len() == want.messages;
                    let calls_kept = resumed.calls.len() == want.calls;
                    /* An answered call is one the turn may not run again; a missing one would run twice. */
                    let answered_kept = resumed.answered.len() == want.answered;
                    let damaged_counted = resumed.damaged == usize::from(damaged_tail);

                    RunnerTrial {
                        index,
                        kill_after,
                        damaged_tail,
                        origin_kept,
                        step_kept,
                        messages_kept,
                        calls_kept,
                        answered_kept,
                        damaged_counted,
                        note: if origin_kept && step_kept && messages_kept && calls_kept && answered_kept && damaged_counted {
                            String::new()
                        } else {
                            format!(
                                "want step {} / {} messages / {} calls / {} answered / {} damaged, got {} / {} / {} / {} / {}",
                                want.step_id,
                                want.messages,
                                want.calls,
                                want.answered,
                                usize::from(damaged_tail),
                                resumed.step_id,
                                resumed.messages.len(),
                                resumed.calls.len(),
                                resumed.answered.len(),
                                resumed.damaged,
                            )
                        },
                    }
                }
            };

            trials.push(trial);
            index += 1;

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    trials
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runner_script_is_the_shape_the_trial_expects() {
        let script = runner_script();

        assert_eq!(expected(&script[..0]), Expected { step_id: String::new(), messages: 0, calls: 0, answered: 0 });
        /* Through the first answers: one reply, two results, then the answers spend the calls. */
        assert_eq!(expected(&script[..5]), Expected { step_id: "step-1".into(), messages: 1 + ANSWERS_PER_STEP, calls: 0, answered: 0 });
        /* Killed right after the reply: the two calls are outstanding and none is answered. */
        assert_eq!(expected(&script[..2]), Expected { step_id: "step-1".into(), messages: 1, calls: 2, answered: 0 });
        /* Killed after one result: that one is answered, and it is the one that must not run again. */
        assert_eq!(expected(&script[..3]), Expected { step_id: "step-1".into(), messages: 1, calls: 2, answered: 1 });
    }

    #[test]
    fn a_runner_turn_resumes_correctly_at_every_kill_point() {
        let dir = tempfile::tempdir().unwrap();
        let trials = runner_trials(dir.path(), 20, 5, 7);

        assert_eq!(trials.len(), 100, "20 kill points, five times each");

        for trial in &trials {
            assert!(trial.correct(), "trial {} (killed after {} records{}): {}", trial.index, trial.kill_after, if trial.damaged_tail { ", tail damaged" } else { "" }, trial.note);
        }

        assert!(trials.iter().any(|trial| trial.damaged_tail), "the damaged-tail case has to be among them");
    }

    #[test]
    fn the_script_continues_from_any_number_of_answers() {
        assert!(step_reply(0)[0].to_string().contains("notes/a.txt"));
        assert!(step_reply(1)[0].to_string().contains("ran.log"));
        assert!(step_reply(4)[0].to_string().contains("Done"));
        assert!(step_reply(9)[0].to_string().contains("Done"), "past the script it answers");
    }

    #[test]
    fn a_trial_is_correct_only_with_whole_files_and_one_command() {
        let base = Trial { index: 0, kill_after_ms: 0, not_started: false, resumed: true, finished: true, files_right: true, command_runs: 1, seconds_to_finish: 1.0, note: String::new() };

        assert!(base.correct());
        assert!(!Trial { command_runs: 2, ..base.clone() }.correct());
        assert!(!Trial { files_right: false, ..base.clone() }.correct());
        assert!(Trial { not_started: true, finished: false, ..base.clone() }.correct());

        let summary = summarise(&[base.clone(), Trial { command_runs: 2, ..base }]);

        assert_eq!((summary.correct, summary.duplicated_commands), (1, 1));
    }
}
