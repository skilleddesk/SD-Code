//! The step executor on driven turns: a long request to a local model in Auto mode becomes fresh single-message prompts, each answered with edits that
//! land in the folder; every other turn keeps the chat loop.

use serde_json::{json, Value};

use super::end_to_end::{call, chunk, forbidden_provider, local_target, ndjson, prompt_in, provider, raw_provider};
use crate::data::rotation::{Row, Rotation};
use crate::rotation::policy::RelayPolicy;
use super::*;
use crate::engines::{EngineEvent, Recorder};

fn folder(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sdc-steps-{name}-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/shop.py"), "def total(items):\n    return 0\n").unwrap();
    std::fs::write(root.join("src/cart.py"), "def count(items):\n    return 0\n").unwrap();

    root
}

fn says(text: &str) -> String {
    let done = json!({ "message": { "role": "assistant", "content": "" }, "done": true, "done_reason": "stop", "prompt_eval_count": 900, "eval_count": 12 });

    ndjson(&[json!({ "message": { "role": "assistant", "content": text }, "done": false }), done])
}

fn edit(path: &str, symbol: &str, code: &str) -> String {
    says(&format!("FILE: {path}\nSYMBOL: {symbol}\n```python\n{code}\n```"))
}

/// A request over the size that starts the step executor: two requirements and a lot of background that asks for nothing.
fn long_request() -> String {
    long_request_of(36)
}

fn long_request_of(background: usize) -> String {
    let mut text = String::from("Add a function total(items) in src/shop.py that returns the sum of the prices.\n\nAdd a function count(items) in src/cart.py that returns how many items there are.\n\n");

    for n in 0..background {
        text.push_str(&format!("The shop number {n} has a blue door and a green roof.\n\n"));
    }

    text
}

/// One turn, with `on` switched on and `off` switched off.
///
/// `off` exists because of 3.2: `nr.fresh_step` and the rest of the NAVARATNA switches are on by default now, so a
/// test that wants the plain chat loop has to **say** so. It used to get it by leaving `on` empty, which quietly
/// became "run the step executor" and hung the suite against a provider scripted for one reply.
fn drive_turn_with(name: &str, autonomy: Autonomy, text: &str, replies: Vec<String>, on: &[&str], off: &[&str]) -> (std::path::PathBuf, Vec<Value>, Recorder) {
    let root = folder(name);
    let (url, server) = raw_provider(replies);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, &format!("turn-steps-{name}"), text, "qwen3.5:9b");
    let options = Options { autonomy, max_steps: 8, auto_check: false, research: None };

    crate::v3::ablating(off, || crate::v3::forcing(on, || drive(Backend::Ollama, &local_target(&url, 16_384), &Workspace::new(root.to_str().unwrap(), None), options, None, &Default::default(), &prompt, &recorder.sink())));

    let bodies = server.join().unwrap();

    (root, bodies, recorder)
}

fn drive_turn(name: &str, autonomy: Autonomy, text: &str, replies: Vec<String>, on: &[&str]) -> (std::path::PathBuf, Vec<Value>, Recorder) {
    drive_turn_with(name, autonomy, text, replies, on, &[])
}

fn text_of(recorder: &Recorder) -> String {
    recorder.events().iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect()
}

#[test]
fn a_long_request_in_auto_mode_runs_as_fresh_steps_whose_edits_land_in_the_folder() {
    let (root, bodies, recorder) = drive_turn(
        "happy",
        Autonomy::Auto,
        &long_request(),
        vec![says("S-999: R"), says("nonsense"), edit("src/shop.py", "total", "def total(items):\n    return sum(items)"), edit("src/cart.py", "count", "def count(items):\n    return len(items)")],
        &["nr.fresh_step"],
    );

    assert_eq!(bodies.len(), 4, "two labelling rounds and one call per step");

    for body in &bodies {
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(2), "a system line and ONE user message - never a growing chat: {body}");
        assert!(body["tools"].as_array().is_none_or(Vec::is_empty), "a step is answered with text, not tool calls");
    }

    let second_step = bodies[3]["messages"][1]["content"].as_str().unwrap();

    assert!(second_step.contains("step-1 (R-001) done: src/shop.py: total"), "the next step gets a handover note: {second_step}");
    assert!(!second_step.contains("return sum(items)\n```"), "not the earlier answer");
    assert!(second_step.contains("src/cart.py") && second_step.ends_with("done when: Add a function count(items) in src/cart.py that returns how many items there are."), "{second_step}");

    assert_eq!(std::fs::read_to_string(root.join("src/shop.py")).unwrap(), "def total(items):\n    return sum(items)\n");
    assert_eq!(std::fs::read_to_string(root.join("src/cart.py")).unwrap(), "def count(items):\n    return len(items)\n");

    let said = text_of(&recorder);

    assert!(said.contains("NAVARATNA") && said.contains("2 of 2 steps done, 2 of them by the local model") && said.contains("not proven"), "{said}");
    assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::Done { summary, pass: Some(true), .. } if summary == "Done")));
    assert!(root.join(".sdc/vault").exists(), "the whole request is kept in the vault");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_step_the_model_cannot_do_is_blocked_and_said_and_the_folder_is_left_as_it_was() {
    let junk = says("I would simply return the sum.");
    let (root, _, recorder) = drive_turn("blocked", Autonomy::Auto, &long_request(), vec![says("x"), says("x"), junk.clone(), junk.clone(), junk.clone(), junk.clone(), junk.clone(), junk], &["nr.fresh_step"]);
    let said = text_of(&recorder);

    assert!(said.contains("BLOCKED") && said.contains("holds no edit"), "{said}");
    assert_eq!(std::fs::read_to_string(root.join("src/shop.py")).unwrap(), "def total(items):\n    return 0\n");
    assert!(recorder.events().iter().any(|event| matches!(event, EngineEvent::Done { summary, pass: Some(false), .. } if summary == "2 step(s) blocked")));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_auto_mode_a_long_request_and_the_switch_start_it_every_other_turn_keeps_the_chat_loop() {
    let long = long_request();

    /* Ask mode: the person would be asked before each edit, which the executor does not do - so the loop runs, with its tools */
    let (root, bodies, _) = drive_turn("ask", Autonomy::Ask, &long, vec![says("Understood.")], &["nr.fresh_step"]);

    assert!(!bodies[0]["tools"].as_array().unwrap().is_empty(), "the chat loop offers tools");
    let _ = std::fs::remove_dir_all(&root);

    /* the switch off - said out loud since 3.2, when it became the default */
    let (root, bodies, _) = drive_turn_with("off", Autonomy::Auto, &long, vec![says("Understood.")], &[], &["nr.fresh_step"]);

    assert!(!bodies[0]["tools"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&root);

    /* a short request */
    let (root, bodies, _) = drive_turn("short", Autonomy::Auto, "Add a function total.", vec![says("Understood.")], &["nr.fresh_step"]);

    assert!(!bodies[0]["tools"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_protected_file_and_a_secret_are_refused_by_the_executor_as_by_the_chat_tools() {
    let project_policy = crate::trust::policy::Policy { protected_paths: vec!["src/shop.py".to_string()], ..Default::default() };

    let root = folder("policy");
    let (url, server) = raw_provider(vec![says("x"), says("x"), edit("src/shop.py", "total", "def total(items):\n    return sum(items)"), edit("src/shop.py", "total", "def total(items):\n    return sum(items)"), edit("src/shop.py", "total", "def total(items):\n    return sum(items)"), edit("src/cart.py", "count", "def count(items):\n    key = \"sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\n    return len(items)"), edit("src/cart.py", "count", "def count(items):\n    return len(items)")]);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, "turn-steps-policy", &long_request(), "qwen3.5:9b");
    let options = Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None };

    crate::v3::forcing(&["nr.fresh_step"], || drive(Backend::Ollama, &local_target(&url, 16_384), &Workspace::new(root.to_str().unwrap(), None), options, None, &project_policy, &prompt, &recorder.sink()));
    drop(server);

    let said = text_of(&recorder);

    assert!(said.contains("protected by this project's policy"), "{said}");
    assert_eq!(std::fs::read_to_string(root.join("src/shop.py")).unwrap(), "def total(items):\n    return 0\n", "a protected file stays as it was");
    assert!(!std::fs::read_to_string(root.join("src/cart.py")).unwrap().contains("sk-ant"), "a secret never reaches a file");

    let _ = std::fs::remove_dir_all(&root);
}

fn fake(name: &str, url: String, model: &str) {
    FAKE_TARGETS.with(|targets| targets.borrow_mut().insert(name.to_string(), super::end_to_end::fake_target(url, model)));
}

fn chain(role: &str, auto: bool) -> Rotation {
    Rotation { chain: vec![Row { model: "cloud:cloud-model".into(), enabled: true, role: role.into(), ..Default::default() }], relay: RelayPolicy { auto, ..Default::default() }, ..Default::default() }
}

fn drive_with_chain(name: &str, rotation: Rotation, replies: Vec<String>, on: &[&str]) -> (std::path::PathBuf, std::thread::JoinHandle<Vec<Value>>, Recorder) {
    drive_with_chain_of(name, rotation, replies, on, &long_request())
}

fn drive_with_chain_of(name: &str, rotation: Rotation, replies: Vec<String>, on: &[&str], text: &str) -> (std::path::PathBuf, std::thread::JoinHandle<Vec<Value>>, Recorder) {
    let root = folder(name);
    let (url, server) = raw_provider(replies);
    let recorder = Recorder::new();
    let prompt = prompt_in(&root, &format!("turn-steps-{name}"), text, "qwen3.5:9b");
    let options = Options { autonomy: Autonomy::Auto, max_steps: 8, auto_check: false, research: None };

    crate::data::rotation::forcing(rotation, || crate::v3::forcing(on, || drive(Backend::Ollama, &local_target(&url, 16_384), &Workspace::new(root.to_str().unwrap(), None), options, None, &Default::default(), &prompt, &recorder.sink())));

    (root, server, recorder)
}

/// The Blueprint a cloud architect would answer for `long_request()`: units 1 and 2 are the requirements, every other unit is background.
fn blueprint_json() -> String {
    let background: Vec<String> = (3..=46).map(|n| format!("\"S-{n:03}\"")).collect();

    format!(
        "{{\"folders\": [\"src/\"], \"models\": [], \"contracts\": [{{\"file\": \"src/shop.py\", \"signature\": \"def total(items):\", \"requirements\": [\"R-001\"]}}], \"requirements\": [{{\"id\": \"R-001\", \"units\": [\"S-001\"]}}, {{\"id\": \"R-002\", \"units\": [\"S-002\"]}}], \"background\": [{}]}}",
        background.join(", ")
    )
}

#[test]
fn with_the_relay_consent_one_architect_call_gives_the_skeleton_and_the_steps_use_its_contracts() {
    let (cloud_url, cloud) = provider(vec![chunk(json!({ "content": blueprint_json() })) + "data: [DONE]\n\n"]);

    fake("cloud", cloud_url, "cloud-model");

    /* 46 units: past the size at which one architect call is worth it; the labelling takes two batches a round */
    let (root, server, recorder) = drive_with_chain_of(
        "blueprint",
        chain("architect", true),
        vec![says("x"), says("x"), says("x"), says("x"), edit("src/shop.py", "total", "def total(items):\n    return sum(items)"), edit("src/cart.py", "count", "def count(items):\n    return len(items)")],
        &["nr.fresh_step", "nr.blueprint"],
        &long_request_of(44),
    );
    let local = server.join().unwrap();
    let asked = cloud.join().unwrap();

    assert_eq!(asked.len(), 1, "exactly one cloud call");
    assert!(asked[0].to_string().contains("EMPTY signatures") && asked[0].to_string().contains("S-001"), "{}", asked[0]);
    assert!(local[4]["messages"][1]["content"].as_str().unwrap().contains("src/shop.py: def total(items):"), "the step is given the architect's contract: {}", local[4]);

    let said = text_of(&recorder);

    assert!(said.contains("the architect (cloud:cloud-model) is asked") && said.contains("2 of 2 steps done, 2 of them by the local model"), "{said}");
    assert!(std::fs::read_to_string(root.join(".sdc/audit/sent.jsonl")).unwrap_or_default().contains("blueprint"), "what left the machine is in the audit");

    FAKE_TARGETS.with(|targets| targets.borrow_mut().clear());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_the_consent_no_cloud_call_is_made_and_the_plan_is_the_local_one() {
    let (cloud_url, untouched) = forbidden_provider();

    fake("cloud", cloud_url, "cloud-model");

    let (root, server, recorder) = drive_with_chain("noconsent", chain("architect", false), vec![says("x"), says("x"), edit("src/shop.py", "total", "def total(items):\n    return sum(items)"), edit("src/cart.py", "count", "def count(items):\n    return len(items)")], &["nr.fresh_step", "nr.blueprint"]);

    server.join().unwrap();

    assert!(untouched(), "the chain was never connected to");
    assert!(text_of(&recorder).contains("2 of 2 steps done"));

    FAKE_TARGETS.with(|targets| targets.borrow_mut().clear());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_step_the_local_model_cannot_do_goes_to_the_chain_which_does_it_and_the_rest_carries_on_locally() {
    let (cloud_url, cloud) = provider(vec![call("c1", "write_file", json!({ "path": "src/shop.py", "content": "def total(items):\n    return sum(items)\n" })), chunk(json!({ "content": "Wrote src/shop.py." })) + "data: [DONE]\n\n"]);

    fake("cloud", cloud_url, "cloud-model");

    let junk = says("I would simply return the sum.");
    let (root, server, recorder) = drive_with_chain("relay", chain("", true), vec![says("x"), says("x"), junk.clone(), junk.clone(), junk, edit("src/cart.py", "count", "def count(items):\n    return len(items)")], &["nr.fresh_step"]);

    server.join().unwrap();
    cloud.join().unwrap();

    let said = text_of(&recorder);

    assert!(said.contains("2 of 2 steps done, 1 of them by the local model"), "{said}");
    assert!(said.contains("relay"), "{said}");
    assert_eq!(std::fs::read_to_string(root.join("src/shop.py")).unwrap(), "def total(items):\n    return sum(items)\n", "the chain's work is in the folder");
    assert_eq!(std::fs::read_to_string(root.join("src/cart.py")).unwrap(), "def count(items):\n    return len(items)\n");

    FAKE_TARGETS.with(|targets| targets.borrow_mut().clear());
    let _ = std::fs::remove_dir_all(&root);
}
