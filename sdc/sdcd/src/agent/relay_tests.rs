//! **Silent Relay on a driven turn (M1-11)**, without a local model: the local side is a scripted Ollama endpoint (a list of replies, one per
//! step: a tool call, an answer, silence), the chain side is scripted HTTP providers (a model that does the step, one that fails, one that must
//! never be reached). Each test drives a whole turn through `drive` and reads what happened from the events, the files and the audit log.

use serde_json::{json, Value};

use super::end_to_end::{call, chunk, forbidden_provider, local_target, ndjson, prompt_in, provider, raw_provider};
use super::*;
use crate::data::rotation::{Row, Rotation};
use crate::engines::Recorder;
use crate::rotation::policy::RelayPolicy;

fn done() -> Value {
    json!({ "message": { "role": "assistant", "content": "" }, "done": true, "done_reason": "stop", "prompt_eval_count": 900, "eval_count": 12 })
}

/// A local reply that calls one tool.
fn local_calls(name: &str, arguments: Value) -> String {
    ndjson(&[json!({ "message": { "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": name, "arguments": arguments } }] }, "done": false }), done()])
}

/// A local reply that says something and stops.
fn local_says(text: &str) -> String {
    ndjson(&[json!({ "message": { "role": "assistant", "content": text }, "done": false }), done()])
}

/// A local reply with nothing in it: the model has nothing to say - it could not do the step.
fn local_silent() -> String {
    ndjson(&[done()])
}

fn row(model: &str) -> Row {
    Row { model: model.into(), enabled: true, ..Default::default() }
}

fn rotation(models: &[&str], auto: bool) -> Rotation {
    Rotation { chain: models.iter().map(|model| row(model)).collect(), relay: RelayPolicy { auto, ..Default::default() }, ..Default::default() }
}

fn folder(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sdc-relay-{name}-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("notes.txt"), "the notes\n").unwrap();

    root
}

fn run_local(root: &std::path::Path, turn: &str, local_url: &str, rotation: Rotation) -> Recorder {
    let recorder = Recorder::new();
    let prompt = prompt_in(root, turn, "make the greeting file", "qwen3.5:9b");

    crate::data::rotation::forcing(rotation, || {
        drive(Backend::Ollama, &local_target(local_url, 8_192), &Workspace::new(root.to_str().unwrap(), None), Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());
    });

    recorder
}

fn summary_of(recorder: &Recorder) -> Option<Value> {
    recorder.events().into_iter().find_map(|event| match event {
        EngineEvent::RelaySummary(fields) => Some(fields),
        _ => None,
    })
}

fn steps_of(recorder: &Recorder) -> Vec<Value> {
    recorder
        .events()
        .into_iter()
        .filter_map(|event| match event {
            EngineEvent::RelayStep(fields) => Some(fields),
            _ => None,
        })
        .collect()
}

fn fake(name: &str, url: String, model: &str) {
    FAKE_TARGETS.with(|targets| targets.borrow_mut().insert(name.to_string(), super::end_to_end::fake_target(url, model)));
}

fn clear_fakes() {
    FAKE_TARGETS.with(|targets| targets.borrow_mut().clear());
}

/// The cloud model does the step: it writes the file and says so.
fn cloud_that_writes() -> (String, std::thread::JoinHandle<Vec<Value>>) {
    provider(vec![call("c1", "write_file", json!({ "path": "greeting.txt", "content": "hello from the chain\n" })), chunk(json!({ "content": "Wrote greeting.txt." })) + "data: [DONE]\n\n"])
}

#[test]
fn a_turn_the_local_model_can_do_alone_never_touches_the_chain() {
    let root = folder("alone");
    let (cloud_url, untouched) = forbidden_provider();
    let (local_url, local) = raw_provider(vec![local_calls("read_file", json!({ "path": "notes.txt" })), local_says("The notes say: the notes.")]);

    fake("cloud", cloud_url, "cloud-model");

    let recorder = run_local(&root, "turn-relay-alone", &local_url, rotation(&["cloud:cloud-model"], true));
    let summary = summary_of(&recorder).expect("a Silent Relay turn always ends with its split");

    assert!(matches!(recorder.events().last(), Some(EngineEvent::RelaySummary(_)) | Some(EngineEvent::Done { .. })));
    assert_eq!(local.join().unwrap().len(), 2, "the local model did both steps");
    assert!(untouched(), "the chain was never so much as connected to");
    assert_eq!((summary["stepsLocal"].as_u64(), summary["stepsCloud"].as_u64(), summary["attempts"].as_u64()), (Some(2), Some(0), Some(0)));
    assert!(steps_of(&recorder).is_empty());
    assert_eq!(crate::rotation::ledger::receipt_lines(&summary, false), ["This work: 2 step(s) - every step on the local model"]);

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_step_the_local_model_cannot_do_goes_to_the_chain_and_comes_back() {
    let root = folder("relay");
    let (cloud_url, cloud) = cloud_that_writes();
    let (local_url, local) = raw_provider(vec![local_calls("read_file", json!({ "path": "notes.txt" })), local_silent(), local_says("Carrying on: the file is there.")]);

    fake("cloud", cloud_url, "cloud-model");

    let recorder = run_local(&root, "turn-relay-one", &local_url, rotation(&["cloud:cloud-model"], true));
    let local_asked = local.join().unwrap();
    let cloud_asked = cloud.join().unwrap();

    assert_eq!(std::fs::read_to_string(root.join("greeting.txt")).unwrap(), "hello from the chain\n", "the chain model's edit is on disk");
    assert_eq!(cloud_asked.len(), 2, "the chain model took two calls: the tool call, then its words");
    assert_eq!(local_asked.len(), 3, "the local model: read, silence, then it carried on");

    let handover = local_asked[2]["messages"].as_array().unwrap().iter().filter_map(|message| message["content"].as_str()).find(|text| text.contains("[SDC handover")).expect("the local model is told what was done");

    assert!(handover.contains("cloud:cloud-model") && handover.contains("greeting.txt"), "{handover}");
    assert!(cloud_asked[0]["messages"].as_array().unwrap().iter().filter_map(|message| message["content"].as_str()).any(|text| text.contains("taking over ONE step")), "the capsule is what the chain model is given");

    let steps = steps_of(&recorder);

    assert_eq!(steps.len(), 1);
    assert_eq!((steps[0]["reason"].as_str(), steps[0]["toModel"].as_str(), steps[0]["outcome"].as_str()), (Some("refusal"), Some("cloud:cloud-model"), Some("done-unverified")), "no checks in this folder, and it says so");
    assert_eq!(steps[0]["fromModel"], "ollama:qwen3.5:9b");

    let summary = summary_of(&recorder).unwrap();

    assert_eq!((summary["stepsLocal"].as_u64(), summary["stepsCloud"].as_u64(), summary["stepsTotal"].as_u64()), (Some(2), Some(1), Some(3)));
    assert_eq!(summary["byModel"][0]["model"], "cloud:cloud-model");
    assert_eq!(summary["unverified"], 1);
    assert!(matches!(recorder.events().iter().rev().find(|event| matches!(event, EngineEvent::Done { .. })), Some(EngineEvent::Done { .. })), "the turn ended normally on the local model");

    /* What left the computer is on the audit log, as a fingerprint, before it was sent. */
    let audit = std::fs::read_to_string(root.join(".sdc/audit/sent.jsonl")).unwrap();

    let relays: Vec<&str> = audit.lines().filter(|line| line.contains("\"step\":\"relay\"")).collect();

    assert_eq!(relays.len(), 1, "one send to the chain, one line: {audit}");
    assert!(relays[0].contains("\"left_the_machine\":true") && !audit.contains("taking over"), "a fingerprint, not a copy of the text");

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn when_the_first_chain_model_fails_the_second_does_the_step() {
    let root = folder("second");
    let rejected = serde_json::json!({ "error": { "message": "invalid api key" } }).to_string();
    let (first_url, first) = raw_provider(vec![format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{rejected}", rejected.len())]);
    let (second_url, second) = cloud_that_writes();
    let (local_url, _local) = raw_provider(vec![local_silent(), local_says("Carrying on.")]);

    fake("first", first_url, "first-model");
    fake("second", second_url, "second-model");

    let recorder = run_local(&root, "turn-relay-second", &local_url, rotation(&["first:first-model", "second:second-model"], true));

    assert_eq!(first.join().unwrap().len(), 1);
    assert_eq!(second.join().unwrap().len(), 2);
    assert!(root.join("greeting.txt").exists());

    let steps = steps_of(&recorder);

    assert_eq!(steps.iter().map(|step| (step["toModel"].as_str().unwrap().to_string(), step["outcome"].as_str().unwrap().to_string())).collect::<Vec<_>>(), [("first:first-model".to_string(), "failed".to_string()), ("second:second-model".to_string(), "done-unverified".to_string())]);

    let summary = summary_of(&recorder).unwrap();

    assert_eq!((summary["attempts"].as_u64(), summary["stepsCloud"].as_u64()), (Some(2), Some(1)));

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn when_every_chain_model_fails_the_turn_says_so_and_the_failed_attempt_leaves_nothing_behind() {
    let root = folder("allfail");
    let rejected = serde_json::json!({ "error": { "message": "invalid api key" } }).to_string();
    let refusal = || format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{rejected}", rejected.len());
    let (url, cloud) = raw_provider(vec![refusal()]);
    let (local_url, local) = raw_provider(vec![local_silent(), local_says("I could not do that step, and the chain could not either.")]);

    fake("cloud", url, "cloud-model");

    let recorder = run_local(&root, "turn-relay-allfail", &local_url, rotation(&["cloud:cloud-model"], true));

    assert_eq!(cloud.join().unwrap().len(), 1);

    let local_asked = local.join().unwrap();
    let told = local_asked[1]["messages"].as_array().unwrap().iter().filter_map(|message| message["content"].as_str()).find(|text| text.contains("could not be done")).expect("the local model is told the chain failed");

    assert!(told.contains("invalid api key") || told.contains("did not finish"), "{told}");
    assert!(!root.join("greeting.txt").exists());

    let summary = summary_of(&recorder).unwrap();

    assert_eq!((summary["stepsCloud"].as_u64(), summary["failed"].as_u64()), (Some(0), Some(1)), "no cloud step is claimed");
    assert!(matches!(recorder.events().iter().rev().find(|event| matches!(event, EngineEvent::Done { .. } | EngineEvent::Failed(_))), Some(EngineEvent::Done { .. })), "and the turn still ends");

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_local_only_project_never_relays_whatever_is_switched_on() {
    let root = folder("localonly");

    std::fs::create_dir_all(root.join(".sdc")).unwrap();
    std::fs::write(root.join(".sdc/policy.toml"), "privacy = \"local-only\"\n").unwrap();

    let (url, untouched) = forbidden_provider();
    let (local_url, _local) = raw_provider(vec![local_silent()]);

    fake("cloud", url, "cloud-model");

    let policy = crate::trust::policy::Policy::load_local(&root);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-relay-localonly", "do it", "qwen3.5:9b");

    crate::data::rotation::forcing(rotation(&["cloud:cloud-model"], true), || {
        drive(Backend::Ollama, &local_target(&local_url, 8_192), &Workspace::new(root.to_str().unwrap(), None), Options { autonomy: Autonomy::Auto, max_steps: 4, auto_check: false, research: None }, None, &policy, &prompt, &recorder.sink());
    });

    assert!(untouched(), "nothing left the computer");
    assert!(summary_of(&recorder).is_none() && steps_of(&recorder).is_empty(), "no relay mode, no receipt");

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_the_consent_the_local_rotation_stays_manual() {
    let root = folder("noconsent");
    let (url, untouched) = forbidden_provider();
    let (local_url, _local) = raw_provider(vec![local_silent()]);

    fake("cloud", url, "cloud-model");

    let recorder = run_local(&root, "turn-relay-noconsent", &local_url, rotation(&["cloud:cloud-model"], false));

    assert!(untouched(), "relay.auto is off: the chain is only ever called on the person's click");
    assert!(summary_of(&recorder).is_none());

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_model_picked_by_hand_is_never_relayed() {
    let root = folder("pinned");
    let (url, untouched) = forbidden_provider();
    let (api_url, _api) = provider(vec![chunk(json!({ "content": "" })) + "data: [DONE]\n\n"]);

    fake("cloud", url, "cloud-model");

    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-relay-pinned", "do it", "gpt-pinned");
    let target = super::end_to_end::fake_target(api_url, "gpt-pinned");

    crate::data::rotation::forcing(rotation(&["cloud:cloud-model"], true), || {
        drive(Backend::Api, &target, &Workspace::new(root.to_str().unwrap(), None), Options { autonomy: Autonomy::Auto, max_steps: 4, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());
    });

    assert!(untouched(), "a pinned turn calls its own model and nobody else");
    assert!(summary_of(&recorder).is_none() && steps_of(&recorder).is_empty());

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_ledger_switch_only_decides_whether_the_split_is_written_down() {
    let root = folder("ledger-off");
    let (cloud_url, untouched) = forbidden_provider();
    let (local_url, _local) = raw_provider(vec![local_says("Nothing to do.")]);

    fake("cloud", cloud_url, "cloud-model");

    let recorder = crate::v3::ablating(&["relay.ledger"], || run_local(&root, "turn-relay-ledger-off", &local_url, rotation(&["cloud:cloud-model"], true)));

    assert!(untouched());
    assert!(summary_of(&recorder).is_none(), "relay.ledger off: no summary event");
    assert!(matches!(recorder.events().iter().rev().find(|event| matches!(event, EngineEvent::Done { .. })), Some(EngineEvent::Done { .. })), "and the turn is the same");

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

/// A chain provider that is slow to answer: it waits `delay` before it sends anything.
fn slow_provider(delay: std::time::Duration, replies: Vec<String>) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{BufRead, Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for reply in replies {
            let (socket, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
            let mut length = 0usize;

            loop {
                let mut line = String::new();

                reader.read_line(&mut line).unwrap();

                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }

                if line == "\r\n" {
                    break;
                }
            }

            let mut body = vec![0u8; length];

            reader.read_exact(&mut body).unwrap();
            std::thread::sleep(delay);

            let mut socket = socket;

            let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{reply}");
        }
    });

    (url, server)
}

#[test]
fn a_chain_model_that_takes_too_long_is_given_up_on_and_its_edits_are_put_back() {
    let root = folder("watchdog");
    let (slow_url, slow) = slow_provider(std::time::Duration::from_millis(2_500), vec![call("c1", "write_file", json!({ "path": "late.txt", "content": "too late\n" })), chunk(json!({ "content": "done" })) + "data: [DONE]\n\n"]);
    let (fast_url, fast) = cloud_that_writes();
    let (local_url, _local) = raw_provider(vec![local_silent(), local_says("Carrying on.")]);

    fake("slow", slow_url, "slow-model");
    fake("fast", fast_url, "fast-model");

    let mut chain = rotation(&["slow:slow-model", "fast:fast-model"], true);

    chain.relay.call_timeout_s = 1;

    let recorder = run_local(&root, "turn-relay-watchdog", &local_url, chain);
    let steps = steps_of(&recorder);

    assert_eq!(steps.iter().map(|step| step["outcome"].as_str().unwrap()).collect::<Vec<_>>(), ["failed", "done-unverified"], "the slow one is given up on, the next one does it");
    assert!(!root.join("late.txt").exists(), "what the slow model wrote after its time was put back");
    assert!(root.join("greeting.txt").exists());

    fast.join().unwrap();
    drop(slow);
    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn past_its_allowance_the_turn_asks_once_and_a_no_keeps_the_work_local() {
    let root = folder("allowance");
    let (cloud_url, cloud) = cloud_that_writes();
    let (local_url, _local) = raw_provider(vec![local_silent(), local_silent(), local_says("Staying local, as you asked.")]);

    fake("cloud", cloud_url, "cloud-model");

    let mut chain = rotation(&["cloud:cloud-model"], true);

    chain.relay.max_cloud_steps_per_turn = 1;

    let recorder = Recorder::new();
    let answering = recorder.clone();
    let answerer = std::thread::spawn(move || {
        for _ in 0..200 {
            if let Some(question) = answering.events().into_iter().find_map(|event| match event {
                EngineEvent::Question { question_id, options, .. } => Some((question_id, options)),
                _ => None,
            }) {
                assert_eq!(question.1, ["Yes, for this turn", "No, stop here"], "the choices the plan names");
                crate::agent::gate::resolve(&question.0, "answer:No, stop here");

                return true;
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        false
    });
    let prompt = prompt_in(&root, "turn-relay-allowance", "make the greeting file", "qwen3.5:9b");

    crate::data::rotation::forcing(chain, || {
        drive(Backend::Ollama, &local_target(&local_url, 8_192), &Workspace::new(root.to_str().unwrap(), None), Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None }, None, &Default::default(), &prompt, &recorder.sink());
    });

    assert!(answerer.join().unwrap(), "the person was asked");
    assert_eq!(cloud.join().unwrap().len(), 2, "the chain was used exactly once");
    assert_eq!(steps_of(&recorder).len(), 1, "and the refusal after the allowance did not send a second step");

    clear_fakes();
    let _ = std::fs::remove_dir_all(&root);
}
