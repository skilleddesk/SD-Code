//! **PADMA P0-1: the Never-Again suite.** The 25 Oct 2026 incident (`docs/padma/PADMA.md`, RC1-RC10) as tests that fail while the fault
//! exists and pass only when it is fixed. One test per guarantee. Each is listed in [`XFAIL`] with the card that fixes it: a listed
//! test must FAIL (the fault is still there) and a card removes its entries when it turns them green - a listed test that passes is an
//! error, so a marker can never outlive its fix.
//!
//! The model is a scripted fake of Ollama's `/api/chat` (no live model, R8). Unlike the shared test provider, it never waits for a
//! request that does not come, so a turn that ends early fails an assertion instead of hanging.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::end_to_end::{local_target, ndjson, prompt_in};
use super::*;
use crate::engines::{EngineEvent, Recorder};

/// Tests that are expected to fail until their card lands: `(test, card)`. P0-1 starts with all of them.
const XFAIL: &[(&str, &str)] = &[
];

fn guarded(name: &str, body: impl FnOnce()) {
    match XFAIL.iter().find(|(listed, _)| *listed == name) {
        None => body(),
        Some((_, card)) => {
            let outcome = catch_unwind(AssertUnwindSafe(body));

            assert!(outcome.is_err(), "{name} is marked expected-fail for PADMA {card} but it PASSES - remove it from XFAIL");
            eprintln!("XFAIL never_again::{name} - the fault is still there; fixed by PADMA {card}");
        }
    }
}

/// A scripted Ollama. Requests are answered in order; when the script runs out it answers `[script ended]`, so a loop that asks for
/// more than the script holds is visible in `bodies`.
struct Fake {
    url: String,
    bodies: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
}

impl Fake {
    fn start(script: Vec<String>) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
        let (bodies, stop) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)));
        let (seen, halt) = (bodies.clone(), stop.clone());

        listener.set_nonblocking(true).unwrap();

        std::thread::spawn(move || {
            let mut script = script.into_iter();

            while !halt.load(Ordering::Relaxed) {
                let Ok((socket, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));

                    continue;
                };

                socket.set_nonblocking(false).ok();
                socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();

                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut length = 0usize;

                loop {
                    let mut line = String::new();

                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }

                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }

                    if line == "\r\n" {
                        break;
                    }
                }

                let mut body = vec![0u8; length];

                if reader.read_exact(&mut body).is_err() {
                    continue;
                }

                seen.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or(Value::Null));

                let mut socket = socket;
                let reply = script.next().unwrap_or_else(|| says("[script ended]"));

                let _ = write!(socket, "{reply}");
            }
        });

        Fake { url, bodies, stop }
    }

    fn bodies(&self) -> Vec<Value> {
        self.bodies.lock().unwrap().clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn done_line() -> Value {
    json!({ "message": { "role": "assistant", "content": "" }, "done": true, "done_reason": "stop", "prompt_eval_count": 900, "eval_count": 12 })
}

fn says(text: &str) -> String {
    ndjson(&[json!({ "message": { "role": "assistant", "content": text }, "done": false }), done_line()])
}

fn silence() -> String {
    ndjson(&[json!({ "message": { "role": "assistant", "content": "" }, "done": false }), done_line()])
}

fn thinks(thinking: &str) -> String {
    ndjson(&[json!({ "message": { "role": "assistant", "content": "", "thinking": thinking }, "done": false }), done_line()])
}

fn tool(name: &str, arguments: Value) -> String {
    ndjson(&[json!({ "message": { "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": name, "arguments": arguments } }] }, "done": false }), done_line()])
}

struct Run {
    root: std::path::PathBuf,
    bodies: Vec<Value>,
    events: Vec<EngineEvent>,
}

impl Run {
    fn said(&self) -> String {
        self.events.iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect()
    }

    fn failed(&self) -> Option<String> {
        self.events.iter().find_map(|event| if let EngineEvent::Failed(reason) = event { Some(reason.clone()) } else { None })
    }

    fn done(&self) -> Option<String> {
        self.events.iter().find_map(|event| if let EngineEvent::Done { summary, .. } = event { Some(summary.clone()) } else { None })
    }

    fn tools_run(&self) -> usize {
        self.events.iter().filter(|event| matches!(event, EngineEvent::ToolCompleted { .. })).count()
    }

    /// What the tool said back in request number `request` (0-based), as the model saw it.
    fn tool_answers(&self, request: usize) -> Vec<String> {
        self.bodies[request]["messages"].as_array().into_iter().flatten().filter(|message| message["role"] == "tool").filter_map(|message| message["content"].as_str().map(str::to_string)).collect()
    }

    fn last_user_note(&self, request: usize) -> String {
        self.bodies[request]["messages"].as_array().into_iter().flatten().rev().find(|message| message["role"] == "user").and_then(|message| message["content"].as_str()).unwrap_or_default().to_string()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn folder(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sdc-never-again-{name}-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("hello.txt"), "hi from the folder\n").unwrap();
    std::fs::write(root.join("src/my_file.rs"), "fn main() {}\n").unwrap();

    root
}

fn run(name: &str, request: &str, script: Vec<String>, window: u64, max_steps: usize, on: &[&str]) -> Run {
    let root = folder(name);

    run_in(root, name, request, script, window, max_steps, on)
}

fn run_in(root: std::path::PathBuf, name: &str, request: &str, script: Vec<String>, window: u64, max_steps: usize, on: &[&str]) -> Run {
    let fake = Fake::start(script);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, &format!("turn-never-again-{name}"), request, "qwen3.5:9b");
    let options = Options { autonomy: Autonomy::Auto, max_steps, auto_check: false, research: None };

    crate::v3::forcing(on, || drive(Backend::Ollama, &local_target(&fake.url, window), &Workspace::new(root.to_str().unwrap(), None), options, None, &Default::default(), &prompt, &recorder.sink()));

    Run { root, bodies: fake.bodies(), events: recorder.events() }
}

/// G2 / RC1: "now I will do X" followed by silence is not an answer.
#[test]
fn announce_then_stop() {
    guarded("announce_then_stop", || {
        let got = run(
            "announce",
            "check the build",
            vec![says("Now I'll check the build.\n1. Run the build"), tool("read_file", json!({ "path": "hello.txt" })), says("The build file says hi. Nothing is wrong.")],
            16_384,
            12,
            &[],
        );

        assert_eq!(got.bodies.len(), 3, "the model must be asked again after it only announced: {:?}", got.done());
        assert!(got.last_user_note(1).contains("You described an action but did not call a tool"), "{}", got.last_user_note(1));
        assert_eq!(got.tools_run(), 1);
        assert!(got.said().contains("Nothing is wrong"));
        assert_eq!(got.done().as_deref(), Some("Done"));
    });
}

/// G2 / RC1: the 25 Oct reply that ended in a colon.
#[test]
fn colon_end_no_tool() {
    guarded("colon_end_no_tool", || {
        let got = run("colon", "check the build", vec![says("now checking build:"), tool("read_file", json!({ "path": "hello.txt" })), says("The build is fine.")], 16_384, 12, &[]);

        assert_eq!(got.tools_run(), 1, "a reply that ends in a colon is not the end of the work");
        assert!(got.said().contains("The build is fine."));
        assert_eq!(got.done().as_deref(), Some("Done"));
    });
}

/// G3 / RC2: Qwen3.5 printed its tool call as text. (The content form was already read by P1-6's `sme.toolcall_rescue`, so this one is green
/// from the start; the forms P1-6 missed - a call inside the thinking - are `thinking_only` and `padma::extract`'s own tests.)
#[test]
fn qwen35_text_toolcall() {
    guarded("qwen35_text_toolcall", || {
        let got = run("qwen", "what does hello.txt say?", vec![says("<function=read_file>\n<parameter=path>\nhello.txt\n</parameter>\n</function>"), says("It says hi.")], 16_384, 12, &[]);

        assert_eq!(got.bodies.len(), 2);
        assert!(got.tool_answers(1).iter().any(|answer| answer.contains("hi from the folder")), "the printed call must have been run: {:?}", got.tool_answers(1));
        assert!(got.said().contains("It says hi."));
    });
}

/// G3 / RC2: a reply with nothing but thinking - the call is inside the thinking.
#[test]
fn thinking_only() {
    guarded("thinking_only", || {
        let got = run(
            "thinking",
            "what does hello.txt say?",
            vec![thinks("I should read it. <tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"hello.txt\"}}</tool_call>"), says("It says hi.")],
            16_384,
            12,
            &[],
        );

        assert_eq!(got.bodies.len(), 2, "a thinking-only reply must not end the turn");
        assert!(got.tool_answers(1).iter().any(|answer| answer.contains("hi from the folder")));
        assert!(!got.bodies[1].to_string().contains("I should read it"), "the thinking is never sent back");
    });
}

/// G4 / RC8: an empty answer is not the end of the task.
#[test]
fn empty_answer() {
    guarded("empty_answer", || {
        let got = run("empty", "what does hello.txt say?", vec![silence(), silence(), silence(), says("The file says hi.")], 16_384, 12, &[]);

        assert!(got.failed().is_none(), "an empty reply ended the turn: {:?}", got.failed());
        assert!(got.said().contains("The file says hi."), "{}", got.said());
        assert_eq!(got.done().as_deref(), Some("Done"));
    });
}

/// G4: and an answer that stays empty forever ends blocked, in words, after at most six rungs.
#[test]
fn empty_forever_is_blocked_after_six_rungs() {
    guarded("empty_forever_is_blocked_after_six_rungs", || {
        let got = run("empty-forever", "what does hello.txt say?", (0..12).map(|_| silence()).collect(), 16_384, 12, &[]);

        assert!(got.bodies.len() <= 7, "one request plus at most six rungs, got {}", got.bodies.len());
        assert!(got.failed().is_none());
        assert!(got.done().is_some_and(|summary| summary.starts_with("Blocked")), "{:?}", got.done());
        assert!(got.said().to_lowercase().contains("blocked"), "{}", got.said());
    });
}

/// G5 / RC3+RC4: 12,397 + 4,096 reserve > 16,384 (109 over) ended six turns of work. Thirty big reads in a 16k window must go on.
#[test]
fn overflow_by_109() {
    guarded("overflow_by_109", || {
        let root = folder("overflow");

        for n in 0..30 {
            std::fs::write(root.join(format!("big{n}.txt")), format!("{}\n", format!("line {n} of the log, with some words in it\n").repeat(300))).unwrap();
        }

        let mut script: Vec<String> = (0..28).map(|n| tool("read_file", json!({ "path": format!("big{n}.txt") }))).collect();

        script.push(says("I read all of them."));

        let got = run_in(root, "overflow", "read every big file and tell me when you are done", script, 16_384, 40, &[]);

        assert!(got.failed().is_none(), "ContextOverflow ended the turn: {:?}", got.failed());
        assert!(got.bodies.len() >= 26, "the run must continue past step 25, got {} requests", got.bodies.len());
        assert!(got.said().contains("I read all of them."));

        for body in &got.bodies {
            assert!(body.to_string().len() / 4 + 4_096 <= 16_384 + 400, "a request that would not fit was sent ({} chars)", body.to_string().len());
        }
    });
}

/// G6 / RC5+RC6: a two-line prompt for a whole-project review is a big job and starts the step executor.
#[test]
fn short_prompt_full_review() {
    guarded("short_prompt_full_review", || {
        let request = "পুরো SDC প্রজেক্ট দেখো, error রিপোর্ট করো";
        let repo = crate::padma::scope::RepoStats { files: 900, lines: 250_000 };
        let scope = crate::padma::scope::classify(request, &repo);

        assert_eq!(scope.recipe, crate::padma::scope::Recipe::ReviewCode);
        assert!(scope.large);

        let options = Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None };

        assert!(
            crate::v3::forcing(&["padma.scope", "nr.fresh_step"], || steps::applies(Backend::Ollama, &options, false, false, request, &crate::v3::ModelCtx::local(16_384), &repo)),
            "the step executor must start for a short request that is a large job"
        );
    });
}

/// P2-2: with `padma.think_policy` on, a model known to think is told not to think for a tool-call step and to think for the repair after a failed
/// tool; with it off, no flag is sent at all.
#[test]
fn the_think_flag_follows_the_step_kind() {
    let model = "padma-think-fixture:1b";

    crate::sme::probe::save(&crate::sme::probe::Profile {
        key: crate::sme::probe::ModelKey { provider: "ollama".into(), model: model.into(), digest: "think-fixture".into(), quantization: "Q4_K_M".into(), runtime: "0".into() },
        status: crate::sme::probe::Status::Probed,
        class: crate::sme::probe::Class::S,
        effective_context: 16_384,
        max_reliable_output: 512,
        tool_call_reliability: 1.0,
        tool_calls_printed_as_text: 0,
        json_adherence: 1.0,
        tokens_per_second: 30.0,
        first_token_ms: 100,
        vram_bytes: None,
        gpu_share: None,
        logprobs: false,
        thinking: true,
        seed: 1,
        probed_at: "2026-01-01".into(),
    })
    .unwrap();

    let script = || vec![tool("read_file", json!({ "path": "hello.txt" })), tool("read_file", json!({ "path": "no-such-file.txt" })), says("The second file does not exist."), says("[script ended]")];
    let turn = |on: &[&str]| {
        let root = folder("think-flag");
        let fake = Fake::start(script());
        let recorder = Recorder::new();
        let prompt = prompt_in(&root, "turn-never-again-think", "read hello.txt and then no-such-file.txt", model);
        let options = Options { autonomy: Autonomy::Auto, max_steps: 6, auto_check: false, research: None };
        let target = Target { model: model.to_string(), ..local_target(&fake.url, 16_384) };

        crate::v3::forcing(on, || drive(Backend::Ollama, &target, &Workspace::new(root.to_str().unwrap(), None), options, None, &Default::default(), &prompt, &recorder.sink()));

        let bodies = fake.bodies();

        let _ = std::fs::remove_dir_all(&root);

        bodies
    };

    let on = turn(&["padma.think_policy"]);

    assert!(on.len() >= 3, "{on:?}");
    assert_eq!(on[0]["think"], json!(false), "the first step is a tool-call step");
    assert_eq!(on[1]["think"], json!(false), "so is the next one, after a read that worked");
    assert_eq!(on[2]["think"], json!(true), "the step after a failed tool is a repair");

    let off = turn(&[]);

    assert!(off.iter().all(|body| body.get("think").is_none()), "{off:?}");
}

/// P2-3: a step that names its tools is offered only those - the prompt is smaller - and a tool it did not name still runs if the model calls it.
#[test]
fn a_step_is_offered_only_its_tools_and_an_unlisted_call_still_runs() {
    let turn = |on: &[&str]| {
        let root = folder("tool-subset");
        let fake = Fake::start(vec![tool("list_dir", json!({ "path": "." })), says("Listed.")]);
        let recorder = Recorder::new();
        let prompt = prompt_in(&root, "turn-never-again-subset", "list the folder", "qwen3.5:9b");
        let options = Options { autonomy: Autonomy::Auto, max_steps: 4, auto_check: false, research: None };

        crate::v3::forcing(on, || {
            crate::padma::tools::with_tools(&["read_file", "grep"], || drive(Backend::Ollama, &local_target(&fake.url, 16_384), &Workspace::new(root.to_str().unwrap(), None), options, None, &Default::default(), &prompt, &recorder.sink()))
        });

        let got = Run { root, bodies: fake.bodies(), events: recorder.events() };
        let answer = got.tool_answers(1).join("\n");

        (got.bodies[0]["tools"].as_array().map_or(0, Vec::len), got.bodies[0].to_string().len(), answer)
    };
    let (full_tools, full_bytes, _) = turn(&[]);
    let (few_tools, few_bytes, answer) = turn(&["padma.tool_subset"]);

    assert_eq!(few_tools, 2, "only the declared tools");
    assert!(full_tools > few_tools, "{full_tools}");
    assert!(few_bytes < full_bytes, "the prompt is smaller: {few_bytes} < {full_bytes}");
    assert!(answer.contains("hello.txt") && !answer.to_lowercase().contains("unknown tool"), "the unlisted call ran: {answer}");
}

/// P2-4: with `padma.inner_english` on, a Bengali request is put into English once; the ledger's goal and the model's request are English, the
/// model is told to answer in Bengali, and a task that stops says where it stands in Bengali. A restatement that lost the file name is not trusted.
#[test]
fn a_bengali_request_is_worked_in_english_and_answered_in_bengali() {
    let request = "`billing.py` এর 3 টা টেস্ট ঠিক করো";
    let root = folder("inner-english");
    let fake = Fake::start(vec![says("Fix the 3 tests of `billing.py`"), silence(), silence(), silence(), silence(), silence(), silence(), silence(), silence(), silence()]);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-never-again-english", request, "qwen3.5:9b");
    let target = local_target(&fake.url, 16_384);
    let options = Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None };

    crate::v3::forcing(&["padma.inner_english"], || {
        let english = english_prompt(Backend::Ollama, &target, &prompt);

        assert_eq!(crate::padma::task::request_of(&english), "Fix the 3 tests of `billing.py`");
        assert_eq!(crate::padma::task::request_of(&prompt), request, "the original prompt is untouched");

        let (tracker, watched) = crate::padma::task::begin(root.to_str().unwrap(), &english, &recorder.sink(), None).expect("a ledger");

        crate::padma::task::with_contract(|| drive(Backend::Ollama, &target, &Workspace::new(root.to_str().unwrap(), None), options, None, &Default::default(), &english, &watched));
        tracker.finish();
    });

    let bodies = fake.bodies();
    let ledger = crate::padma::ledger::Ledger::open(&root).unwrap();
    let task = ledger.get(&crate::padma::task::id_of("turn-never-again-english")).expect("the task");

    assert_eq!(task.contract.goal_en, "Fix the 3 tests of `billing.py`");
    assert_eq!(task.contract.language_out, "bn");
    assert!(bodies[0].to_string().contains("You never answer it") && bodies[0].to_string().contains(request), "the one tiny translation call");
    assert!(bodies[1].to_string().contains("Fix the 3 tests of `billing.py`") && bodies[1].to_string().contains("Write your answer to them in Bengali"), "the work is asked in English with the Bengali-answer note");
    assert!(!bodies[1].to_string().contains("ঠিক করো"), "the Bengali request is not what the model works from");

    let said: String = recorder.events().iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect();

    assert!(said.contains("কাজ:") && !said.contains("Task: "), "the unfinished-task line is in Bengali: {said}");

    let lost = Fake::start(vec![says("Fix the tests")]);
    let untrusted = english_prompt(Backend::Ollama, &local_target(&lost.url, 16_384), &prompt);

    assert_eq!(untrusted.text, prompt.text, "an English line that lost the file name and the number is not used");

    let _ = std::fs::remove_dir_all(&root);
}

/// P1-4: with `padma.conductor` on, a turn whose wall-clock budget is spent is checkpointed with a "continue" message instead of running on; a
/// small question still takes one model request.
#[test]
fn the_conductor_checkpoints_a_turn_whose_time_is_spent_and_leaves_a_small_question_alone() {
    let turn = |budget: crate::padma::conductor::Budget| {
        let root = folder("conductor-time");
        let fake = Fake::start(vec![says("Four.")]);
        let recorder = Recorder::new();
        let prompt = prompt_in(&root, "turn-never-again-conductor", "what is 2 + 2?", "qwen3.5:9b");
        let options = Options { autonomy: Autonomy::Auto, max_steps: 6, auto_check: false, research: None };

        crate::v3::forcing(&["padma.conductor"], || {
            crate::padma::conductor::with_budget(budget, || {
                let (tracker, watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");

                crate::padma::task::with_contract(|| drive(Backend::Ollama, &local_target(&fake.url, 16_384), &Workspace::new(root.to_str().unwrap(), None), options, None, &Default::default(), &prompt, &watched));
                tracker.finish();
            })
        });

        let state = crate::padma::ledger::Ledger::open(&root).unwrap().get(&crate::padma::task::id_of("turn-never-again-conductor")).unwrap().state;
        let got = Run { root, bodies: fake.bodies(), events: recorder.events() };

        (got.bodies.len(), got.done(), got.said(), state)
    };
    let (asked, done, said, state) = turn(crate::padma::conductor::Budget { unit_calls: 8, turn: std::time::Duration::ZERO });

    assert_eq!(asked, 0, "no model call after the time is spent");
    assert_eq!(done.as_deref(), Some("Paused at the time limit"));
    assert!(said.contains("continue"), "{said}");
    assert_eq!(state, crate::padma::ledger::State::Paused);

    let (asked, done, said, state) = turn(crate::padma::conductor::Budget::default());

    assert_eq!(asked, 1, "a small question is one request");
    assert_eq!(done.as_deref(), Some("Done"));
    assert!(said.contains("Four.") && state == crate::padma::ledger::State::Done);
}

/// P1-4: the replayed incident - many units, each its own short model turn - completes every unit with the fake engine, and a unit that fails
/// does not stop the others.
#[test]
fn the_conductor_completes_every_unit_of_a_replayed_review() {
    let root = folder("conductor-units");
    let names: Vec<String> = (0..6).map(|n| format!("big{n}.txt")).collect();

    for name in &names {
        std::fs::write(root.join(name), format!("{}\n", "a line of the log\n".repeat(400))).unwrap();
    }

    let mut script = Vec::new();

    for name in &names {
        script.push(tool("read_file", json!({ "path": name })));
        script.push(says(&format!("{name}: no problem found.")));
    }

    let fake = Fake::start(script);
    let recorder = Recorder::new();
    let target = local_target(&fake.url, 16_384);
    let mut ledger = crate::padma::ledger::Ledger::open(&root).unwrap();

    ledger.create(&crate::padma::ledger::contract_for("task-units", "s1", "review every file", "turn:units"), &[]).unwrap();

    let units: Vec<&str> = names.iter().map(String::as_str).collect();
    let outcome = crate::padma::conductor::conduct(&mut ledger, "task-units", "file", &units, &recorder.sink(), false, crate::padma::conductor::Budget::default(), |unit, limit| {
        let sink = Recorder::new();
        let prompt = prompt_in(&root, &format!("turn-unit-{unit}"), &format!("review {unit}"), "qwen3.5:9b");
        let options = Options { autonomy: Autonomy::Auto, max_steps: limit, auto_check: false, research: None };

        drive(Backend::Ollama, &target, &Workspace::new(root.to_str().unwrap(), None), options, None, &Default::default(), &prompt, &sink.sink());

        let events = sink.events();
        let calls = events.iter().filter(|event| matches!(event, EngineEvent::ToolCompleted { .. })).count();
        let answer: String = events.iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect();

        crate::padma::conductor::UnitRun { calls, result: if answer.contains("no problem found") { Ok(answer) } else { Err("no answer".into()) } }
    });

    assert_eq!((outcome.done, outcome.blocked, outcome.remaining, outcome.paused), (6, 0, 0, false), "{outcome:?}");
    assert_eq!(fake.bodies().len(), 12, "two requests per unit");

    let labels: Vec<String> = recorder.events().iter().filter_map(|event| if let EngineEvent::TaskProgress { label, .. } = event { Some(label.clone()) } else { None }).collect();

    assert!(labels.contains(&"file 6/6".to_string()), "{labels:?}");

    let _ = std::fs::remove_dir_all(&root);
}

/// P3-2: a large review goes through its recipe: one model request per file, every one a ledger row, the report built only from findings whose evidence is in
/// the file, and the turn's end does not rewrite what the recipe settled.
#[test]
fn a_large_review_runs_as_a_recipe_and_reports_only_what_is_proved() {
    let root = folder("recipe-review");
    let filler: String = (0..420).map(|n| format!("value_{n} = {n}\n")).collect();

    std::fs::write(root.join("src/auth.py"), format!("def login(user, password):\n    if password == \"admin123\":\n        return True\n{filler}")).unwrap();

    for name in ["misc", "store", "util"] {
        std::fs::write(root.join(format!("src/{name}.py")), format!("def {name}():\n    return 1\n{filler}")).unwrap();
    }

    let fake = Fake::start(vec![
        says("src/auth.py:2 · high · security · the password is compared to a hard-coded value · if password == \"admin123\":\nsrc/auth.py:3 · low · logic · returns early · log_attempt(user)"),
        says("NO FINDINGS"),
        says("NO FINDINGS"),
        says("NO FINDINGS"),
        says("NO FINDINGS"),
        says("NO FINDINGS"),
        says("One serious problem: a hard-coded password in src/auth.py."),
    ]);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-never-again-recipe", "review the whole project and report errors", "qwen3.5:9b");
    let target = local_target(&fake.url, 16_384);
    let handled = crate::v3::forcing(&["padma.ledger", "padma.scope", "padma.recipes"], || {
        let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");
        let handled = recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &recorder.sink(), &tracker);

        tracker.finish();

        handled
    });
    let ledger = crate::padma::ledger::Ledger::open(&root).unwrap();
    let task = ledger.get(&crate::padma::task::id_of("turn-never-again-recipe")).expect("the task");
    let said: String = recorder.events().iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect();
    let bodies = fake.bodies();

    assert!(handled, "the contract named R-REVIEW-CODE and the project is large: {:?}", task.contract);
    assert_eq!(task.contract.recipe, "R-REVIEW-CODE");
    assert_eq!(bodies.len(), 7, "one request per file (six) and one for the summary");
    assert!(bodies[0].to_string().contains("File: src/auth.py") && bodies[0].to_string().contains("2|     if password"), "{}", bodies[0]);
    assert!(said.contains("1 verified findings (1 high") && said.contains("evidence: `if password == \"admin123\":`"), "{said}");
    assert!(!said.contains("log_attempt") && said.contains("Dropped: 1 whose evidence is not in the file"), "{said}");
    assert!(said.contains("One serious problem"), "{said}");
    assert_eq!(task.state, crate::padma::ledger::State::Done, "the end of the turn does not rewrite the recipe's state");
    assert_eq!(task.counts().blocked + task.counts().remaining, 0, "{:?}", task.steps);
    assert!(task.steps.iter().any(|step| step.unit == "review:src/auth.py") && task.steps.iter().filter(|step| step.unit.starts_with("review:")).count() == 6, "{:?}", task.steps);
    assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::Done { summary, .. } if summary == "Done")));

    let _ = std::fs::remove_dir_all(&root);
}

/// P3-4: "make a landing page" goes through R-BUILD-PAGE: a plan, one fresh request per file, a bad file asked again with the reason,
/// files written only into landing-page/, and the written page checked from the disk.
#[test]
fn a_page_is_planned_written_per_file_checked_and_kept_in_its_folder() {
    let root = folder("recipe-page");
    let fake = Fake::start(vec![
        says("FILE index.html | the page\nFILE styles.css | the look\nDEFAULT: warm bakery colours\nDEFAULT: English text"),
        says("```html\n<html><head><title>Bakery</title><link rel=\"stylesheet\" href=\"styles.css\"></head><body><h1>Fresh bread</h1><img src=\"https://example.com/loaf.png\" alt=\"a loaf\"></body></html>\n```"),
        says("body { color: brown; "),
        says("body { color: brown; }"),
    ]);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-never-again-page", "make a landing page for a bakery", "qwen3.5:9b");
    let target = local_target(&fake.url, 16_384);
    let handled = crate::v3::forcing(&["padma.ledger", "padma.scope", "padma.recipes"], || {
        let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");
        let handled = recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &recorder.sink(), &tracker);

        tracker.finish();

        handled
    });
    let ledger = crate::padma::ledger::Ledger::open(&root).unwrap();
    let task = ledger.get(&crate::padma::task::id_of("turn-never-again-page")).expect("the task");
    let said: String = recorder.events().iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect();
    let bodies = fake.bodies();

    assert!(handled, "the contract: {:?}", task.contract);
    assert_eq!(task.contract.recipe, "R-BUILD-PAGE");
    assert_eq!(bodies.len(), 4, "the plan, two files, and one second try for the broken css");
    assert!(bodies[1].to_string().contains("Write the complete file index.html") && !bodies[1].to_string().contains("FILE index.html | the page\nFILE styles.css | the look\nDEFAULT: warm bakery colours\nDEFAULT: English text\n"), "{}", bodies[1]);
    assert!(bodies[3].to_string().contains("failed a check") && bodies[3].to_string().contains("braces"), "{}", bodies[3]);
    assert!(root.join("landing-page/index.html").exists() && root.join("landing-page/styles.css").exists());
    assert_eq!(std::fs::read_to_string(root.join("landing-page/styles.css")).unwrap(), "body { color: brown; }");
    assert!(std::fs::read_to_string(root.join("landing-page/index.html")).unwrap().starts_with("<html>"), "the code fence is not written");
    assert!(said.contains("landing-page/index.html") && said.contains("warm bakery colours"), "{said}");
    assert_eq!(task.state, crate::padma::ledger::State::Done);
    assert_eq!(task.counts().blocked + task.counts().remaining, 0, "{:?}", task.steps);
    assert!(!root.join("index.html").exists(), "nothing is written outside the page folder");

    let _ = std::fs::remove_dir_all(&root);
}

/// P3-4: a file that is still wrong after its second try is Blocked with the reason, and is not written.
#[test]
fn a_page_file_that_stays_wrong_is_blocked_not_written() {
    let root = folder("recipe-page-bad");
    let fake = Fake::start(vec![says("FILE index.html | the page"), says("just some words"), says("still just words")]);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-never-again-page-bad", "build a landing page for a gym", "qwen3.5:9b");
    let target = local_target(&fake.url, 16_384);

    crate::v3::forcing(&["padma.ledger", "padma.scope", "padma.recipes"], || {
        let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");

        assert!(recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &recorder.sink(), &tracker));

        tracker.finish();
    });

    let ledger = crate::padma::ledger::Ledger::open(&root).unwrap();
    let task = ledger.get(&crate::padma::task::id_of("turn-never-again-page-bad")).expect("the task");
    let row = task.steps.iter().find(|step| step.unit == "page:index.html").expect("the page row");

    assert_eq!(row.status, crate::padma::ledger::Status::Blocked, "{:?}", task.steps);
    assert!(row.handover_note.contains("failed its check after 2 tries"), "{}", row.handover_note);
    assert!(!root.join("landing-page/index.html").exists());
    assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::Done { summary, .. } if summary.starts_with("Blocked"))));

    let _ = std::fs::remove_dir_all(&root);
}

fn node_project(name: &str) -> Option<std::path::PathBuf> {
    let program = if cfg!(windows) { "npm.cmd" } else { "npm" };

    if !std::process::Command::new(program).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|status| status.success()) {
        eprintln!("npm is not installed here: the R-FIX test with a real test run is skipped");

        return None;
    }

    let root = folder(name);

    std::fs::write(root.join("package.json"), "{ \"name\": \"fixture\", \"version\": \"1.0.0\", \"scripts\": { \"test\": \"node test.js\" } }\n").unwrap();
    std::fs::write(root.join("src/math.js"), "exports.add = (a, b) => {\n  return a - b;\n};\nexports.twice = (a) => {\n  return a * 3;\n};\n").unwrap();
    std::fs::write(root.join("test.js"), "const assert = require('assert');\nconst { add, twice } = require('./src/math');\nassert.strictEqual(add(1, 2), 3);\nassert.strictEqual(twice(4), 8);\nconsole.log('ok');\n").unwrap();

    Some(root)
}

fn run_fix(root: &std::path::Path, turn: &str, replies: Vec<String>) -> (Vec<serde_json::Value>, crate::padma::ledger::Task, Recorder) {
    let fake = Fake::start(replies);
    let recorder = Recorder::new();
    let prompt = prompt_in(root, turn, "fix the failing tests", "qwen3.5:9b");
    let target = local_target(&fake.url, 16_384);

    crate::v3::forcing(&["padma.ledger", "padma.scope", "padma.recipes"], || {
        let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");

        assert!(recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &recorder.sink(), &tracker), "the recipe took the turn");

        tracker.finish();
    });

    let task = crate::padma::ledger::Ledger::open(root).unwrap().get(&crate::padma::task::id_of(turn)).expect("the task");

    (fake.bodies(), task, recorder)
}

/// P3-5: "fix the failing tests" goes through R-FIX: the real failure is reproduced, a patch that does not make the whole run green is undone and
/// the model is asked again with the test output, and the kept patch is the one that passes.
#[test]
fn a_failing_test_is_fixed_by_a_patch_that_is_kept_only_if_every_test_passes() {
    let Some(root) = node_project("recipe-fix") else { return };
    let (bodies, task, recorder) = run_fix(
        &root,
        "turn-never-again-fix",
        vec![
            says("FILE: src/math.js\n<<<<<<< SEARCH\n  return a - b;\n=======\n  return a * b;\n>>>>>>> REPLACE"),
            says("FILE: src/math.js\n<<<<<<< SEARCH\n  return a - b;\n=======\n  return a + b;\n>>>>>>> REPLACE\n\nFILE: src/math.js\n<<<<<<< SEARCH\n  return a * 3;\n=======\n  return a * 2;\n>>>>>>> REPLACE"),
        ],
    );
    let said: String = recorder.events().iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect();

    assert_eq!(task.contract.recipe, "R-FIX");
    assert_eq!(bodies.len(), 2, "one patch, and one second try");
    assert!(bodies[0].to_string().contains("AssertionError") && bodies[0].to_string().contains("File: src/math.js"), "{}", bodies[0]);
    assert!(bodies[1].to_string().contains("still fails (the change was undone)"), "{}", bodies[1]);
    assert!(std::fs::read_to_string(root.join("src/math.js")).unwrap().contains("return a + b;"));
    assert!(std::fs::read_to_string(root.join("src/math.js")).unwrap().contains("return a * 2;"));
    assert!(said.contains("src/math.js") && said.contains("passes after it"), "{said}");
    assert_eq!(task.state, crate::padma::ledger::State::Done, "{:?}", task.steps);
    assert_eq!(task.counts().blocked + task.counts().remaining, 0, "{:?}", task.steps);

    let _ = std::fs::remove_dir_all(&root);
}

/// P3-5: when no patch makes the tests pass, the task is Blocked with the reason and the files are exactly as they were.
#[test]
fn a_fix_that_never_goes_green_is_blocked_and_leaves_the_files_alone() {
    let Some(root) = node_project("recipe-fix-blocked") else { return };
    let before = std::fs::read_to_string(root.join("src/math.js")).unwrap();
    let wrong = || says("FILE: src/math.js\n<<<<<<< SEARCH\n  return a - b;\n=======\n  return a * b;\n>>>>>>> REPLACE");
    let (bodies, task, recorder) = run_fix(&root, "turn-never-again-fix-blocked", vec![wrong(), wrong(), wrong()]);
    let row = task.steps.iter().find(|step| step.unit == "patch").expect("the patch row");

    assert_eq!(bodies.len(), 3, "the first try and two more");
    assert_eq!(row.status, crate::padma::ledger::Status::Blocked, "{:?}", task.steps);
    assert!(row.handover_note.contains("failed its check after 3 tries"), "{}", row.handover_note);
    assert_eq!(std::fs::read_to_string(root.join("src/math.js")).unwrap(), before);
    assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::Done { summary, .. } if summary.starts_with("Blocked"))));

    let _ = std::fs::remove_dir_all(&root);
}

/// P3-5: tests that already pass are not "fixed" - the answer says so and no model request is made.
#[test]
fn passing_tests_are_not_touched() {
    let Some(root) = node_project("recipe-fix-green") else { return };

    std::fs::write(root.join("test.js"), "console.log('ok');\n").unwrap();

    let (bodies, task, _recorder) = run_fix(&root, "turn-never-again-fix-green", vec![says("unused")]);
    let row = task.steps.iter().find(|step| step.unit == "reproduce").expect("the reproduce row");

    assert!(bodies.is_empty(), "no model request for a green run");
    assert_eq!(row.status, crate::padma::ledger::Status::Blocked);
    assert!(row.handover_note.contains("passes already"), "{}", row.handover_note);

    let _ = std::fs::remove_dir_all(&root);
}

/// P3-3: with `padma.site_probe` on, "review my website" goes through R-REVIEW-SITE: code probes the folder's pages, the report holds
/// only what was measured and names what could not be, and the model only writes the short summary. With the switch off nothing is routed.
#[test]
fn a_site_review_is_probed_by_code_and_only_when_its_switch_is_on() {
    let root = folder("recipe-site");

    std::fs::write(root.join("index.html"), "<html><head></head><body>\n<a href=\"gone.html\">Gone</a>\n</body></html>").unwrap();

    let fake = Fake::start(vec![says("Two problems: a missing title and a dead link.")]);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-never-again-site", "review my website for problems", "qwen3.5:9b");
    let target = local_target(&fake.url, 16_384);
    let handled = crate::v3::forcing(&["padma.ledger", "padma.scope", "padma.recipes", "padma.site_probe"], || {
        let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");
        let handled = recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &recorder.sink(), &tracker);

        tracker.finish();

        handled
    });
    let said: String = recorder.events().iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect();
    let task = crate::padma::ledger::Ledger::open(&root).unwrap().get(&crate::padma::task::id_of("turn-never-again-site")).expect("the task");

    assert!(handled && task.contract.recipe == "R-REVIEW-SITE", "{:?}", task.contract);
    assert_eq!(fake.bodies().len(), 1, "only the summary is a model request");
    assert!(said.contains("the page has no title") && said.contains("[broken-link]") && said.contains("gone.html"), "{said}");
    assert!(said.contains("skipped the live site") && said.contains("Two problems"), "{said}");

    let off = Recorder::new();
    let prompt = prompt_in(&root, "turn-never-again-site-off", "review my website for problems", "qwen3.5:9b");
    let routed = crate::v3::forcing(&["padma.ledger", "padma.scope", "padma.recipes"], || {
        let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &off.sink(), None).expect("a ledger");

        recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &off.sink(), &tracker)
    });

    assert!(!routed, "the switch is off: the normal loop answers");

    let _ = std::fs::remove_dir_all(&root);
}

fn notices(recorder: &Recorder) -> Vec<(String, String)> {
    recorder.events().into_iter().filter_map(|event| if let EngineEvent::Notice { kind, text } = event { Some((kind, text)) } else { None }).collect()
}

fn git_available() -> bool {
    std::process::Command::new("git").arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|status| status.success())
}

fn git_init(root: &std::path::Path) {
    assert!(std::process::Command::new("git").arg("init").arg("-q").current_dir(root).status().unwrap().success());
}

/// P4-1: a two-line request routed to a recipe uses the NAVARATNA engines when `padma.recipe_engines` is on - and says which, in one line. With it off the
/// same run has none of them: the step prompt is not rewritten and no receipt is shown.
#[test]
fn a_recipe_step_uses_the_engines_and_shows_a_receipt_only_when_the_switch_is_on() {
    let run_review = |name: &str, engines: bool| {
        let root = folder(name);
        let filler: String = (0..420).map(|n| format!("value_{n} = {n}\n")).collect();

        for file in ["auth", "misc", "store", "util"] {
            std::fs::write(root.join(format!("src/{file}.py")), format!("def {file}():\n    return 1\n{filler}")).unwrap();
        }

        let mut replies: Vec<String> = (0..6).map(|_| says("NO FINDINGS")).collect();

        replies.push(says("Nothing wrong."));

        let fake = Fake::start(replies);
        let recorder = Recorder::new();
        let turn = format!("turn-never-again-{name}");
        let prompt = prompt_in(&root, &turn, "review the whole project and report errors", "qwen3.5:9b");
        let target = local_target(&fake.url, 16_384);
        let mut on = vec!["padma.ledger", "padma.scope", "padma.recipes"];

        if engines {
            on.push("padma.recipe_engines");
        }

        let handled = crate::v3::forcing(&on, || {
            let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");
            let handled = recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &recorder.sink(), &tracker);

            tracker.finish();

            handled
        });
        let bodies = fake.bodies();

        let _ = std::fs::remove_dir_all(&root);

        (handled, bodies, notices(&recorder))
    };

    let (handled, bodies, shown) = run_review("engines-on", true);
    let first = bodies[0].to_string();

    assert!(handled && bodies.len() == 7, "{}", bodies.len());
    assert!(first.contains("## Goal") && first.contains("Done when:") && first.contains("Remember - goal:"), "the step prompt is the fresh one: {first}");

    let receipt = shown.iter().find(|(kind, _)| kind == "engines").map(|(_, text)| text.clone()).expect("a receipt line");

    assert!(receipt.starts_with("Engines used:") && receipt.contains("fresh_step x7"), "{receipt}");

    let (handled, bodies, shown) = run_review("engines-off", false);

    assert!(handled && bodies.len() == 7);
    assert!(!bodies[0].to_string().contains("## Goal"), "{}", bodies[0]);
    assert!(!shown.iter().any(|(kind, _)| kind == "engines"), "no receipt without the switch: {shown:?}");
}

/// P4-1 on R-FIX: a patch that edits a test is refused (the test is the anchor) and the model is asked again with the reason; a whole function replaced
/// by its name is accepted and kept only because every test passes; the written file is recorded in the registry; the step ran under a git shadow.
#[test]
fn a_fix_under_the_engines_keeps_the_tests_as_anchors_and_may_replace_a_function_by_name() {
    let program = if cfg!(windows) { "npm.cmd" } else { "npm" };

    if !git_available() || !std::process::Command::new(program).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|status| status.success()) {
        eprintln!("git or npm is missing here: the engines R-FIX test is skipped");

        return;
    }

    let root = folder("recipe-fix-engines");

    std::fs::write(root.join("package.json"), "{ \"name\": \"fixture\", \"version\": \"1.0.0\", \"scripts\": { \"test\": \"node test.js\" } }\n").unwrap();
    std::fs::write(root.join("src/math.js"), "function add(a, b) {\n  return a - b;\n}\nexports.add = add;\n").unwrap();
    std::fs::write(root.join("test.js"), "const assert = require('assert');\nconst { add } = require('./src/math');\nassert.strictEqual(add(1, 2), 3);\nconsole.log('ok');\n").unwrap();
    git_init(&root);

    let fake = Fake::start(vec![
        says("FILE: test.js\n<<<<<<< SEARCH\nassert.strictEqual(add(1, 2), 3);\n=======\nassert.strictEqual(add(1, 2), -1);\n>>>>>>> REPLACE"),
        says("FILE: src/math.js\nSYMBOL: add\n```js\nfunction add(a, b) {\n  return a + b;\n}\n```"),
    ]);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-never-again-fix-engines", "fix the failing tests", "qwen3.5:9b");
    let target = local_target(&fake.url, 16_384);

    crate::v3::forcing(&["padma.ledger", "padma.scope", "padma.recipes", "padma.recipe_engines"], || {
        let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");

        assert!(recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &recorder.sink(), &tracker));

        tracker.finish();
    });

    let bodies = fake.bodies();
    let task = crate::padma::ledger::Ledger::open(&root).unwrap().get(&crate::padma::task::id_of("turn-never-again-fix-engines")).expect("the task");
    let receipt = notices(&recorder).into_iter().find(|(kind, _)| kind == "engines").map(|(_, text)| text).expect("a receipt line");

    assert_eq!(bodies.len(), 2, "the refused test edit, then the symbol patch: {:?} {:?}", task.steps, notices(&recorder));
    assert!(bodies[0].to_string().contains("SYMBOL:") && bodies[0].to_string().contains("skeleton"), "the prompt offers the symbol form and shows skeletons: {}", bodies[0]);
    assert!(bodies[1].to_string().contains("test.js is a test") && bodies[1].to_string().contains("anchor"), "{}", bodies[1]);
    assert!(std::fs::read_to_string(root.join("test.js")).unwrap().contains("add(1, 2), 3"), "the test was not touched");
    assert!(std::fs::read_to_string(root.join("src/math.js")).unwrap().contains("return a + b;"));
    assert_eq!(task.state, crate::padma::ledger::State::Done, "{:?}", task.steps);

    for engine in ["fresh_step", "skeleton_pack", "test_first", "symbol_patch", "git_shadow", "registry"] {
        assert!(receipt.contains(engine), "{engine} is missing from: {receipt}");
    }

    assert!(std::fs::read_to_string(root.join(".sdc/registry.json")).unwrap_or_default().contains("src/math.js"), "the written file is in the registry");

    let _ = std::fs::remove_dir_all(&root);
}

fn run_page(root: &std::path::Path, turn: &str, local: Vec<String>, engines: bool, rotation: crate::data::rotation::Rotation) -> (Vec<serde_json::Value>, Recorder) {
    let fake = Fake::start(local);
    let recorder = Recorder::new();
    let prompt = prompt_in(root, turn, "build a landing page for a gym", "qwen3.5:9b");
    let target = local_target(&fake.url, 16_384);
    let mut on = vec!["padma.ledger", "padma.scope", "padma.recipes"];

    if engines {
        on.push("padma.recipe_engines");
    }

    crate::data::rotation::forcing(rotation, || {
        crate::v3::forcing(&on, || {
            let (tracker, _watched) = crate::padma::task::begin(root.to_str().unwrap(), &prompt, &recorder.sink(), None).expect("a ledger");

            assert!(recipe_turn(Backend::Ollama, &target, root.to_str().unwrap(), &prompt, &recorder.sink(), &tracker));

            tracker.finish();
        })
    });

    (fake.bodies(), recorder)
}

fn chain(consent: bool) -> crate::data::rotation::Rotation {
    crate::data::rotation::Rotation {
        chain: vec![crate::data::rotation::Row { model: "cloud:cloud-model".into(), enabled: true, ..Default::default() }],
        relay: crate::rotation::policy::RelayPolicy { auto: consent, ..Default::default() },
        ..Default::default()
    }
}

fn last_progress(recorder: &Recorder) -> (u64, u64) {
    recorder.events().iter().rev().find_map(|event| if let EngineEvent::TaskProgress { local, cloud, .. } = event { Some((*local, *cloud)) } else { None }).expect("a progress event")
}

/// P4-1 / M1-4: a unit the local model could not get past its check after every try goes once to the chain - cleaned, written to the audit before it is sent -
/// and what comes back is checked by the same check. Only with the engines on and the person's consent; otherwise it stays Blocked and the chain is never reached.
#[test]
fn a_unit_the_local_model_cannot_pass_goes_once_to_the_chain_and_only_when_allowed() {
    let good = "<html><head><title>Gym</title></head><body><h1>Lift</h1></body></html>";
    let plan = || says("FILE index.html | the page");
    let stuck = || vec![plan(), says("just some words"), says("still just words")];
    let fake_cloud = |url: String| FAKE_TARGETS.with(|targets| targets.borrow_mut().insert("cloud".to_string(), super::end_to_end::fake_target(url, "cloud-model")));

    /* allowed: the chain's answer passes the page check and is written */
    let root = folder("recipe-relay");
    let (cloud_url, cloud) = super::end_to_end::provider(vec![super::end_to_end::chunk(json!({ "content": good })) + "data: [DONE]\n\n"]);

    fake_cloud(cloud_url);

    let (bodies, recorder) = run_page(&root, "turn-never-again-relay", stuck(), true, chain(true));
    let asked = cloud.join().unwrap();
    let receipt = notices(&recorder).into_iter().find(|(kind, _)| kind == "engines").map(|(_, text)| text).expect("a receipt line");
    let audit = std::fs::read_to_string(root.join(".sdc/audit/sent.jsonl")).unwrap_or_default();

    assert_eq!(bodies.len(), 3, "the plan and two local tries");
    assert_eq!(asked.len(), 1, "one request to the chain");
    assert!(asked[0].to_string().contains("smaller local model tried 2 time(s)"), "{}", asked[0]);
    assert_eq!(std::fs::read_to_string(root.join("landing-page/index.html")).unwrap(), good);
    assert!(receipt.contains("relay x1"), "{receipt}");
    assert_eq!(audit.lines().filter(|line| line.contains("\"step\":\"recipe-relay\"") && line.contains("\"left_the_machine\":true")).count(), 1, "{audit}");
    assert_eq!(last_progress(&recorder), (3, 1), "three local requests and one cloud request are counted apart");

    /* no consent: the chain is never connected to, the unit is Blocked */
    let root_no = folder("recipe-relay-no-consent");
    let (cloud_url, untouched) = super::end_to_end::forbidden_provider();

    fake_cloud(cloud_url);

    let (_, recorder) = run_page(&root_no, "turn-never-again-relay-no", stuck(), true, chain(false));
    let task = crate::padma::ledger::Ledger::open(&root_no).unwrap().get(&crate::padma::task::id_of("turn-never-again-relay-no")).unwrap();

    assert!(untouched(), "without the person's consent nothing leaves the computer");
    assert!(task.steps.iter().any(|step| step.unit == "page:index.html" && step.status == crate::padma::ledger::Status::Blocked));
    assert_eq!(last_progress(&recorder).1, 0);

    /* engines off, consent given: same */
    let root_off = folder("recipe-relay-off");
    let (cloud_url, untouched) = super::end_to_end::forbidden_provider();

    fake_cloud(cloud_url);

    let (_, recorder) = run_page(&root_off, "turn-never-again-relay-off", stuck(), false, chain(true));

    assert!(untouched(), "the recipe relay belongs to the engines switch");
    assert!(notices(&recorder).iter().all(|(kind, _)| kind != "engines"));

    FAKE_TARGETS.with(|targets| targets.borrow_mut().clear());

    for dir in [root, root_no, root_off] {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// G7 / RC7: "ok koro" after a failure resumes the task; a pasted SDC error does too; a new request does not.
#[test]
fn resume_after_fail() {
    guarded("resume_after_fail", || {
        use crate::padma::bind::{resolve, ActiveTask, Binding};

        let active = ActiveTask { id: "task-1".into(), goal: "review the whole project and report errors".into(), last_error: Some("ContextOverflow: this step is about 12397 tokens".into()) };

        assert_eq!(resolve("ok koro", Some(&active)), Binding::Resume("task-1".into()));
        assert_eq!(resolve("continue", Some(&active)), Binding::Resume("task-1".into()));
        assert_eq!(resolve("ContextOverflow: this step is about 12397 tokens with 4096 reserved for the answer, and the model's window is 16384", Some(&active)), Binding::Resume("task-1".into()));
        assert_eq!(resolve("write me a haiku about rust", Some(&active)), Binding::New);
        assert_eq!(resolve("ok koro", None), Binding::New);
    });
}

/// G8 / RC8: a file error says what it really is, where, and what is near - on a path with a space.
#[test]
fn win_os_error_2_3_5() {
    guarded("win_os_error_2_3_5", || {
        let got = run(
            "os-error",
            "read the file",
            vec![tool("read_file", json!({ "path": "src/my file.rs" })), tool("read_file", json!({ "path": "no such dir/x.txt" })), says("Neither exists.")],
            16_384,
            12,
            &[],
        );
        let first = got.tool_answers(1).join("\n");
        let second = got.tool_answers(2).join("\n");

        assert!(first.to_lowercase().contains("does not exist"), "file-not-found must be said in words: {first}");
        assert!(first.contains(got.root.to_str().unwrap().trim_end_matches(['\\', '/'])) && first.contains("my file.rs"), "the resolved path: {first}");
        assert!(first.contains("my_file.rs"), "the nearest existing path: {first}");
        assert!(second.to_lowercase().contains("folder") && second.to_lowercase().contains("does not exist"), "path-not-found: {second}");
    });
}

/// G9 / RC9: a reply that claims work it has no tool evidence for does not get "high".
#[test]
fn no_evidence_claim() {
    guarded("no_evidence_claim", || {
        use crate::sdcp::events::StoredEvent;

        let entry = |seq: i64, event: Value| StoredEvent { seq, ts: String::new(), session_id: Some("s".into()), turn_id: Some("t".into()), event };
        let claim = vec![
            entry(1, json!({ "type": "TurnStarted" })),
            entry(2, json!({ "type": "TurnDelta", "delta": "I fixed the bug in src/main.rs and ran the tests - all of them pass." })),
            entry(3, json!({ "type": "TurnCompleted", "summary": "Done" })),
        ];
        let (_, level, _) = crate::trust::score::turn(&crate::trust::score::facts(&claim, "t", &Default::default(), None), 0);

        assert_eq!(level, "low", "a claim with no tool evidence is capped at low");

        let blocked = vec![entry(1, json!({ "type": "TurnStarted" })), entry(2, json!({ "type": "TurnCompleted", "summary": "Blocked: the model gave no answer" }))];
        let (_, level, _) = crate::trust::score::turn(&crate::trust::score::facts(&blocked, "t", &Default::default(), None), 0);

        assert_eq!(level, "blocked");
    });
}

/// G10 / RC10: SDC's own notice (the GPU share) is never part of the answer, and so never of the history.
#[test]
fn no_ui_notice_in_history() {
    guarded("no_ui_notice_in_history", || {
        crate::padma::notice::force_gpu_notice(Some("Only 86% of this model is on the GPU; the rest runs on the CPU and every answer will be slower."));

        let got = run("notice", "what does hello.txt say?", vec![says("It says hi.")], 16_384, 6, &["local.autotune"]);

        crate::padma::notice::force_gpu_notice(None);

        assert!(!got.said().contains("on the GPU"), "the notice is part of the answer text: {}", got.said());
        assert!(got.said().contains("It says hi."));
        assert!(got.events.iter().any(|event| matches!(event, EngineEvent::Notice { kind, text } if kind == "gpu_share" && text.contains("on the GPU"))), "the notice still reaches the person, as an event");
    });
}

/// P1-2 (G1): "done" with nothing changed is not done. A turn that has a contract and was asked to change something is sent back twice, then ends
/// blocked with the reason - never "Done".
#[test]
fn a_done_with_no_work_behind_it_is_sent_back_then_blocked() {
    let got = crate::padma::task::with_contract(|| {
        run("gate-blocked", "fix the typo in hello.txt", vec![says("Done. The typo is fixed."), says("Done, it is fixed."), says("Really done."), says("never asked")], 16_384, 12, &["padma.finish_gate"])
    });

    assert_eq!(got.bodies.len(), 3, "asked twice more, then stopped: {:?}", got.done());
    assert!(got.last_user_note(1).contains("nothing has been changed or run yet"), "{}", got.last_user_note(1));
    assert_eq!(got.done().as_deref(), Some("Blocked: no evidence of the work"));
    assert!(got.said().contains("Not finished: the request asked for a change"), "{}", got.said());
}

/// P1-2: the same request ends normally once the model really makes the change after the signal.
#[test]
fn a_done_after_a_real_edit_ends_the_turn() {
    let got = crate::padma::task::with_contract(|| {
        run(
            "gate-edit",
            "fix the typo in hello.txt",
            vec![says("Done."), tool("edit_file", json!({ "path": "hello.txt", "old_text": "hi from", "new_text": "hello from" })), says("Fixed: the file now says hello.")],
            16_384,
            12,
            &["padma.finish_gate"],
        )
    });

    assert_eq!(got.bodies.len(), 3, "{:?}", got.done());
    assert_eq!(got.done().as_deref(), Some("Done"));
    assert_eq!(std::fs::read_to_string(got.root.join("hello.txt")).unwrap(), "hello from the folder
");
}

/// P1-2: a question, and a turn without a contract, are one step as before.
#[test]
fn a_question_and_an_uncontracted_turn_are_never_held() {
    let question = crate::padma::task::with_contract(|| run("gate-question", "what does hello.txt say?", vec![says("It says hi.")], 16_384, 6, &["padma.finish_gate"]));

    assert_eq!(question.bodies.len(), 1);
    assert_eq!(question.done().as_deref(), Some("Done"));

    let bare = run("gate-bare", "fix the typo in hello.txt", vec![says("Done.")], 16_384, 6, &["padma.finish_gate"]);

    assert_eq!(bare.bodies.len(), 1, "a bare drive call has no contract to hold it");
}

/// P0-3 negative: a plain short answer that merely ends a sentence is not an announcement.
#[test]
fn a_short_plain_answer_is_not_reprompted() {
    let got = run("plain", "what is 2 + 2?", vec![says("2 + 2 = 4")], 16_384, 12, &[]);

    assert_eq!(got.bodies.len(), 1, "a genuine short answer must end the turn: {:?}", got.done());
    assert_eq!(got.done().as_deref(), Some("Done"));
}

/// P0-3: `FINAL:` ends the turn on purpose even if the text looks like an announcement.
#[test]
fn final_marker_ends_the_turn() {
    let got = run("final", "check the build", vec![says("FINAL: now I'll leave it as it is:")], 16_384, 12, &[]);

    assert_eq!(got.bodies.len(), 1, "{:?}", got.done());
}

/// P0-5: a request that cannot fit even after every rung of the Overflow Rule is blocked (too large) in words - it is not a `Failed` turn.
#[test]
fn a_request_that_never_fits_is_blocked_not_failed() {
    let goal = format!("summarise this: {}", "the quick brown fox ".repeat(2_000));
    let got = run("never-fits", &goal, (0..8).map(|_| says("ok")).collect(), 4_096, 24, &[]);

    assert!(got.failed().is_none(), "{:?}", got.failed());
    assert!(got.done().is_some_and(|summary| summary.starts_with("Blocked")), "{:?}", got.done());
    assert!(got.said().contains("too large"), "{}", got.said());
    assert!(got.bodies.is_empty(), "nothing that would not fit is ever sent");
}
