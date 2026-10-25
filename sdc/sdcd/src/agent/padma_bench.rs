//! **PADMA P4-2: the PADMA bench** - `sdcd bench padma [--live] [--tasks a,b] [--out FILE]`.
//!
//! Twelve short prompts (English, Bengali, Banglish), each run through the real turn (`turn_body`: ledger, recipes, loop, guards) twice - every
//! `padma.*` switch off, then every one on - against a scripted model. The scripted model is the same in both runs: it announces an action and
//! stops, goes silent, invents a finding, writes a broken file and gets a patch wrong, exactly where the 25 Oct incident did. So the table
//! measures the code around the model, not the model. The live run (`--live`, the real Ollama and the model the person has chosen) is the
//! owner's (P4-3); it uses the same tasks and checks, but the "stopped without a tool" and "empty reply" columns need the scripted model and show `-`.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};

use super::*;
use crate::engines::{EngineEvent, Recorder};

/// How the scripted model behaves when it is asked.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tag {
    Plain,
    Announce,
    Silent,
}

struct Reply {
    wire: String,
    tag: Tag,
}

fn done_line() -> Value {
    json!({ "message": { "role": "assistant", "content": "" }, "done": true, "done_reason": "stop", "prompt_eval_count": 900, "eval_count": 12 })
}

fn wire(lines: &[Value]) -> String {
    let body = lines.iter().map(|line| line.to_string()).collect::<Vec<_>>().join("\n") + "\n";

    format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n{body}")
}

fn say(text: &str, tag: Tag) -> Reply {
    Reply { wire: wire(&[json!({ "message": { "role": "assistant", "content": text }, "done": false }), done_line()]), tag }
}

fn silence() -> Reply {
    say("", Tag::Silent)
}

fn call(name: &str, arguments: Value) -> Reply {
    Reply { wire: wire(&[json!({ "message": { "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": name, "arguments": arguments } }] }, "done": false }), done_line()]), tag: Tag::Plain }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Review,
    Site,
    Page,
    Fix,
}

/// One bench task: a short prompt, what the project holds, and the recipe the prompt must reach.
struct Task {
    id: &'static str,
    prompt: &'static str,
    kind: Kind,
    ui: bool,
    recipe: &'static str,
}

const TASKS: &[Task] = &[
    Task { id: "review-en", prompt: "review the whole project and report errors", kind: Kind::Review, ui: false, recipe: "R-REVIEW-CODE" },
    Task { id: "review-bn", prompt: "পুরো প্রজেক্ট দেখো, error রিপোর্ট করো", kind: Kind::Review, ui: false, recipe: "R-REVIEW-CODE" },
    Task { id: "review-banglish", prompt: "puro project review koro, bug khujo", kind: Kind::Review, ui: false, recipe: "R-REVIEW-CODE" },
    Task { id: "ui-errors-en", prompt: "review the whole project and find the UI errors", kind: Kind::Review, ui: true, recipe: "R-REVIEW-CODE" },
    Task { id: "ui-errors-bn", prompt: "পুরো প্রজেক্টের UI error গুলো খুঁজে রিপোর্ট করো", kind: Kind::Review, ui: true, recipe: "R-REVIEW-CODE" },
    Task { id: "site-en", prompt: "review my website for problems", kind: Kind::Site, ui: false, recipe: "R-REVIEW-SITE" },
    Task { id: "site-bn", prompt: "আমার ওয়েবসাইট রিভিউ করো", kind: Kind::Site, ui: false, recipe: "R-REVIEW-SITE" },
    Task { id: "page-en", prompt: "make a landing page for a bakery", kind: Kind::Page, ui: false, recipe: "R-BUILD-PAGE" },
    Task { id: "page-bn", prompt: "একটা বেকারির ল্যান্ডিং পেজ বানাও", kind: Kind::Page, ui: false, recipe: "R-BUILD-PAGE" },
    Task { id: "page-banglish", prompt: "ekta gym er jonno landing page banao", kind: Kind::Page, ui: false, recipe: "R-BUILD-PAGE" },
    Task { id: "fix-en", prompt: "fix the failing tests", kind: Kind::Fix, ui: false, recipe: "R-FIX" },
    Task { id: "fix-banglish", prompt: "failing test gulo thik koro", kind: Kind::Fix, ui: false, recipe: "R-FIX" },
];

const GOOD_PAGE: &str = "<html><head><title>Bakery</title><link rel=\"stylesheet\" href=\"styles.css\"></head><body><h1>Fresh bread</h1><img src=\"https://example.com/loaf.png\" alt=\"a loaf\"></body></html>";
const GOOD_CSS: &str = "body { color: brown; }";
const SITE_PAGE: &str = "<html><head></head><body>\n<a href=\"gone.html\">Gone</a>\n</body></html>";
const BROKEN_MATH: &str = "exports.add = (a, b) => {\n  return a - b;\n};\n";
const FIXED_MATH: &str = "exports.add = (a, b) => {\n  return a + b;\n};\n";

/// The files that hold a real problem, as `(path, line)`; the evidence is the text of that line.
fn truth(task: &Task) -> Vec<(&'static str, usize)> {
    match (task.kind, task.ui) {
        (Kind::Review, false) => vec![("src/auth.py", 2), ("src/store.py", 2)],
        (Kind::Review, true) => vec![("src/App.jsx", 3), ("src/Form.jsx", 2)],
        _ => Vec::new(),
    }
}

fn files_of(task: &Task) -> Vec<(String, String)> {
    let filler: String = (0..420).map(|n| format!("value_{n} = {n}\n")).collect();
    let js_filler: String = (0..420).map(|n| format!("const value_{n} = {n};\n")).collect();

    match (task.kind, task.ui) {
        (Kind::Review, false) => vec![
            ("src/auth.py".into(), format!("def login(user, password):\n    if password == \"admin123\":\n        return True\n{filler}")),
            ("src/store.py".into(), format!("def find(db, name):\n    return db.execute(\"SELECT * FROM users WHERE name = '\" + name + \"'\")\n{filler}")),
            ("src/util.py".into(), format!("def util():\n    return 1\n{filler}")),
            ("src/misc.py".into(), format!("def misc():\n    return 1\n{filler}")),
        ],
        (Kind::Review, true) => vec![
            ("src/App.jsx".into(), format!("export function App() {{\n  return (\n    <img src=\"logo.png\" />\n  );\n}}\n{js_filler}")),
            ("src/Form.jsx".into(), format!("export function Form({{ submit }}) {{\n  return <button onClick={{submit()}}>Send</button>;\n}}\n{js_filler}")),
            ("src/Nav.jsx".into(), format!("export function Nav() {{\n  return <nav />;\n}}\n{js_filler}")),
            ("src/Foot.jsx".into(), format!("export function Foot() {{\n  return <footer />;\n}}\n{js_filler}")),
        ],
        (Kind::Site, _) => vec![("index.html".into(), SITE_PAGE.into())],
        (Kind::Page, _) => vec![("README.md".into(), "# bakery\n".into())],
        (Kind::Fix, _) => vec![
            ("package.json".into(), "{ \"name\": \"fixture\", \"version\": \"1.0.0\", \"scripts\": { \"test\": \"node test.js\" } }\n".into()),
            ("src/math.js".into(), BROKEN_MATH.into()),
            ("test.js".into(), "const assert = require('assert');\nconst { add } = require('./src/math');\nassert.strictEqual(add(1, 2), 3);\nconsole.log('ok');\n".into()),
        ],
    }
}

/// The scripted model. It answers by what the request says (a recipe's step has a fixed opening) and, for the plain tool loop, by how many loop requests it has had.
struct Sim {
    task: &'static Task,
    root: PathBuf,
    state: Mutex<SimState>,
}

#[derive(Default)]
struct SimState {
    requests: u64,
    loops: usize,
    quiet_done: bool,
    last: Option<Tag>,
}

impl Sim {
    fn serve(&self, body: &Value) -> String {
        let mut state = self.state.lock().unwrap();
        let text = last_user(body);
        let reply = self.recipe_reply(&text, &mut state).unwrap_or_else(|| self.loop_reply(&mut state));

        state.requests += 1;
        state.last = Some(reply.tag);

        reply.wire
    }

    fn recipe_reply(&self, text: &str, state: &mut SimState) -> Option<Reply> {
        if text.contains("Review this source file") {
            let path = text.lines().find_map(|line| line.strip_prefix("File: ")).map(|rest| rest.split_whitespace().next().unwrap_or("").to_string()).unwrap_or_default();

            /* laziness: one file's first answer is nothing at all */
            if path == self.quiet_file() && !state.quiet_done {
                state.quiet_done = true;

                return Some(silence());
            }

            let mut lines: Vec<String> = Vec::new();

            for (file, line) in truth(self.task) {
                let evidence = std::fs::read_to_string(self.root.join(file)).ok().and_then(|source| source.lines().nth(line - 1).map(|found| found.trim().to_string())).unwrap_or_default();

                if file == path && !evidence.is_empty() && text.contains(&evidence) {
                    lines.push(format!("{file}:{line} · high · security · a real defect on this line · {evidence}"));
                }
            }

            /* invention: a finding whose evidence is nowhere in the file */
            if path == self.quiet_file() && state.quiet_done {
                lines.push(format!("{path}:1 · high · logic · an invented problem · launch_missiles(user)"));
            }

            return Some(say(&if lines.is_empty() { "NO FINDINGS".to_string() } else { lines.join("\n") }, Tag::Plain));
        }

        if text.contains("These are the verified findings") || text.contains("These are the measured findings") {
            return Some(say("The project has real problems; fix the highest-severity first.", Tag::Plain));
        }

        if text.contains("Plan a small static web page") {
            return Some(say("FILE index.html | the page\nFILE styles.css | the look\nDEFAULT: warm colours\nDEFAULT: English text", Tag::Plain));
        }

        if text.contains("Write the complete file index.html") {
            return Some(say(GOOD_PAGE, Tag::Plain));
        }

        if text.contains("Write the complete file styles.css") {
            /* a broken file at first, right once it is told why */
            return Some(say(if text.contains("failed a check") { GOOD_CSS } else { "body { color: brown; " }, Tag::Plain));
        }

        if text.contains("A test run of this project fails") {
            let patch = if text.contains("still fails (the change was undone)") { "a + b" } else { "a * b" };

            return Some(say(&format!("FILE: src/math.js\n<<<<<<< SEARCH\n  return a - b;\n=======\n  return {patch};\n>>>>>>> REPLACE"), Tag::Plain));
        }

        None
    }

    fn quiet_file(&self) -> &'static str {
        if self.task.ui { "src/Nav.jsx" } else { "src/util.py" }
    }

    fn loop_reply(&self, state: &mut SimState) -> Reply {
        let step = state.loops;

        state.loops += 1;

        let read = |path: &str| call("read_file", json!({ "path": path }));
        let announce = |text: &str| say(text, Tag::Announce);
        let (a, b, c, d) = if self.task.ui { ("src/App.jsx", "src/Form.jsx", "src/Nav.jsx", "src/Foot.jsx") } else { ("src/auth.py", "src/store.py", "src/util.py", "src/misc.py") };
        let script: Vec<Reply> = match self.task.kind {
            Kind::Review => {
                let refs = truth(self.task).iter().map(|(file, line)| format!("{file}:{line} · high · security · a real defect on this line · see the line")).collect::<Vec<_>>().join("\n");

                vec![read(a), read(b), announce("Now I'll read the rest of the files:"), read(c), read(d), silence(), say(&format!("{refs}\n{c}:1 · high · logic · an invented problem · launch_missiles(user)"), Tag::Plain)]
            }
            Kind::Site => vec![read("index.html"), announce("Now I'll check the links:"), read("index.html"), silence(), say("The page has no title and links to gone.html, which does not exist.", Tag::Plain)],
            Kind::Page => vec![
                announce("Sure, I'll create the page files now:"),
                call("write_file", json!({ "path": "landing-page/index.html", "content": GOOD_PAGE })),
                call("write_file", json!({ "path": "landing-page/styles.css", "content": GOOD_CSS })),
                say("The landing page is ready in landing-page/.", Tag::Plain),
            ],
            Kind::Fix => vec![read("src/math.js"), announce("Now I'll change the function:"), call("write_file", json!({ "path": "src/math.js", "content": FIXED_MATH })), silence(), say("I fixed add().", Tag::Plain)],
        };

        script.into_iter().nth(step).unwrap_or_else(|| say("[script ended]", Tag::Plain))
    }
}

fn last_user(body: &Value) -> String {
    body["messages"].as_array().into_iter().flatten().rev().find(|message| message["role"] == "user").and_then(|message| message["content"].as_str()).unwrap_or_default().to_string()
}

/// A local HTTP server in front of the scripted model; it stops when dropped.
struct Server {
    url: String,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn start(sim: Arc<Sim>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free local port");
        let url = format!("http://{}/v1/chat/completions", listener.local_addr().expect("an address"));
        let stop = Arc::new(AtomicBool::new(false));
        let halt = stop.clone();

        listener.set_nonblocking(true).ok();

        std::thread::spawn(move || {
            while !halt.load(Ordering::Relaxed) {
                let Ok((socket, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));

                    continue;
                };

                socket.set_nonblocking(false).ok();
                socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();

                let Ok(clone) = socket.try_clone() else { continue };
                let mut reader = BufReader::new(clone);
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

                let reply = sim.serve(&serde_json::from_slice(&body).unwrap_or(Value::Null));
                let mut socket = socket;

                let _ = write!(socket, "{reply}");
            }
        });

        Server { url, stop }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What one run of one task measured.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub task: &'static str,
    pub on: bool,
    pub skipped: bool,
    pub completed: bool,
    pub recipe: String,
    pub stopped_without_tool: Option<u32>,
    pub empty_reply_stops: Option<u32>,
    pub overflow: bool,
    pub unmatched: u32,
    pub local_requests: u64,
    pub cloud_requests: u64,
    pub millis: u128,
    pub note: String,
}

fn refs_in(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();

    for word in text.split(|c: char| c.is_whitespace() || c == '`' || c == '(' || c == ')' || c == ',') {
        let Some((path, line)) = word.rsplit_once(':') else { continue };

        if line.chars().all(|c| c.is_ascii_digit()) && !line.is_empty() && [".py", ".jsx", ".js", ".html"].iter().any(|ext| path.ends_with(ext)) && !found.contains(&word.to_string()) {
            found.push(word.trim_end_matches('.').to_string());
        }
    }

    found
}

fn node_available() -> bool {
    std::process::Command::new("node").arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|status| status.success())
}

fn npm_available() -> bool {
    let program = if cfg!(windows) { "npm.cmd" } else { "npm" };

    std::process::Command::new(program).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|status| status.success())
}

fn verify(task: &Task, root: &Path, said: &str) -> bool {
    match task.kind {
        Kind::Review => truth(task).iter().all(|(file, line)| said.contains(&format!("{file}:{line}"))),
        Kind::Site => {
            let lower = said.to_lowercase();

            lower.contains("title") && lower.contains("gone.html")
        }
        Kind::Page => {
            let css = std::fs::read_to_string(root.join("landing-page/styles.css")).unwrap_or_default();
            let html = std::fs::read_to_string(root.join("landing-page/index.html")).unwrap_or_default();

            html.contains("<title>") && !css.is_empty() && css.matches('{').count() == css.matches('}').count()
        }
        Kind::Fix => std::process::Command::new("node").arg("test.js").current_dir(root).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|status| status.success()),
    }
}

fn arm_switches(on: bool) -> Vec<(&'static str, bool)> {
    crate::v3::FEATURES.iter().filter(|feature| feature.name.starts_with("padma.")).map(|feature| (feature.name, on)).collect()
}

/// Runs one task, with every `padma.*` switch `on` or off, against the scripted model (or the real one with `live`).
fn run_task(task: &'static Task, on: bool, live: bool) -> Outcome {
    let mut outcome = Outcome { task: task.id, on, ..Default::default() };

    if task.kind == Kind::Fix && !(node_available() && npm_available()) {
        outcome.skipped = true;
        outcome.note = "node/npm missing".into();

        return outcome;
    }

    let root = std::env::temp_dir().join(format!("sdc-padma-bench-{}-{}-{}", std::process::id(), task.id, if on { "on" } else { "off" }));

    let _ = std::fs::remove_dir_all(&root);

    for (path, content) in files_of(task) {
        let full = root.join(&path);

        std::fs::create_dir_all(full.parent().expect("a parent")).ok();
        std::fs::write(full, content).ok();
    }

    let sim = Arc::new(Sim { task, root: root.clone(), state: Mutex::new(SimState::default()) });
    let server = (!live).then(|| Server::start(sim.clone()));
    let turn_id = format!("turn-padma-bench-{}-{}", task.id, if on { "on" } else { "off" });
    let prompt = Prompt {
        session_id: "padma-bench".into(),
        turn_id: turn_id.clone(),
        text: task.prompt.into(),
        model: "qwen3.5:9b".into(),
        provider: None,
        history: Vec::new(),
        project_root: Some(root.to_string_lossy().to_string()),
        remote: None,
        autonomy: Default::default(),
        resume: None,
        images: Vec::new(),
        effort: None,
    };
    let target = match &server {
        Some(server) => Target {
            url: server.url.clone(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "qwen3.5:9b".to_string(),
            price: None,
            thinking: false,
            effort: None,
            local_context: Some(8_192),
            local_window: false,
        },
        None => match super::target(Backend::Ollama, &prompt) {
            Ok(target) => target,
            Err(reason) => {
                outcome.note = format!("no live model: {reason}");

                let _ = std::fs::remove_dir_all(&root);

                return outcome;
            }
        },
    };
    let recorder = Recorder::new();
    let options = Options { autonomy: Autonomy::Auto, max_steps: 12, auto_check: false, research: None };
    let started = Instant::now();

    crate::v3::with_switches(&arm_switches(on), || {
        turn_body(Backend::Ollama, &target, root.to_str().expect("a utf-8 path"), &Workspace::new(root.to_str().expect("a utf-8 path"), None), options, None, &Default::default(), &prompt, None, &recorder.sink());
    });

    outcome.millis = started.elapsed().as_millis();

    let events = recorder.events();
    let said: String = events.iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect();
    let failed = events.iter().find_map(|event| if let EngineEvent::Failed(reason) = event { Some(reason.clone()) } else { None });
    let summary = events.iter().find_map(|event| if let EngineEvent::Done { summary, .. } = event { Some(summary.clone()) } else { None });
    let progress = events.iter().rev().find_map(|event| if let EngineEvent::TaskProgress { local, cloud, .. } = event { Some((*local, *cloud)) } else { None });

    outcome.overflow = failed.as_deref().is_some_and(|reason| reason.contains("ContextOverflow")) || said.contains("ContextOverflow");
    outcome.cloud_requests = progress.map_or(0, |(_, cloud)| cloud);

    if let Some(server_state) = server.as_ref().map(|_| sim.state.lock().unwrap()) {
        outcome.local_requests = server_state.requests.saturating_sub(outcome.cloud_requests);
        outcome.stopped_without_tool = Some(u32::from(server_state.last == Some(Tag::Announce)));
        outcome.empty_reply_stops = Some(u32::from(server_state.last == Some(Tag::Silent)));
    }

    let wanted: HashSet<String> = truth(task).iter().map(|(file, line)| format!("{file}:{line}")).collect();

    outcome.unmatched = if task.kind == Kind::Review { refs_in(&said).iter().filter(|found| !wanted.contains(*found)).count() as u32 } else { 0 };
    let ledgered = crate::padma::ledger::Ledger::open(&root).ok().and_then(|ledger| ledger.get(&crate::padma::task::id_of(&turn_id)));
    let blocked: Vec<String> = ledgered.iter().flat_map(|task| task.steps.iter()).filter(|step| step.status == crate::padma::ledger::Status::Blocked).map(|step| format!("{}: {}", step.unit, step.handover_note)).collect();

    outcome.recipe = ledgered.map(|task| task.contract.recipe).unwrap_or_default();
    outcome.completed = failed.is_none() && summary.as_deref() == Some("Done") && verify(task, &root, &said);
    outcome.note = failed.or_else(|| (!blocked.is_empty()).then(|| blocked.join("; "))).or_else(|| summary.filter(|text| text != "Done")).unwrap_or_else(|| if outcome.completed { String::new() } else { format!("checked but not met; the answer began: {}", said.chars().take(200).collect::<String>().replace('\n', " / ")) });

    let _ = std::fs::remove_dir_all(&root);

    outcome
}

fn rate(outcomes: &[&Outcome]) -> String {
    let ran: Vec<&&Outcome> = outcomes.iter().filter(|outcome| !outcome.skipped).collect();

    format!("{}/{}", ran.iter().filter(|outcome| outcome.completed).count(), ran.len())
}

fn cell(value: Option<u32>) -> String {
    value.map_or("-".to_string(), |value| value.to_string())
}

/// The off-against-on table, in markdown.
pub fn render(outcomes: &[Outcome], live: bool) -> String {
    let mut text = String::new();

    text.push_str(&format!("# PADMA bench ({})\n\n", if live { "LIVE model" } else { "scripted model - measures the code around the model, not the model" }));
    text.push_str("| task | recipe (on) | done off | done on | stops w/o tool off/on | empty-reply stops off/on | overflow off/on | unmatched off/on | local share on | ms off/on |\n|---|---|---|---|---|---|---|---|---|---|\n");

    for task in TASKS {
        let (Some(off), Some(on)) = (outcomes.iter().find(|o| o.task == task.id && !o.on), outcomes.iter().find(|o| o.task == task.id && o.on)) else { continue };

        if off.skipped || on.skipped {
            text.push_str(&format!("| {} | - | skipped ({}) | | | | | | | |\n", task.id, off.note));

            continue;
        }

        let total = (on.local_requests + on.cloud_requests).max(1);

        text.push_str(&format!(
            "| {} | {} | {} | {} | {}/{} | {}/{} | {}/{} | {}/{} | {}% | {}/{} |\n",
            task.id,
            if on.recipe.is_empty() { "-".to_string() } else if on.recipe == task.recipe { on.recipe.clone() } else { format!("{} (expected {})", on.recipe, task.recipe) },
            if off.completed { "yes" } else { "NO" },
            if on.completed { "yes" } else { "NO" },
            cell(off.stopped_without_tool),
            cell(on.stopped_without_tool),
            cell(off.empty_reply_stops),
            cell(on.empty_reply_stops),
            u8::from(off.overflow),
            u8::from(on.overflow),
            off.unmatched,
            on.unmatched,
            on.local_requests * 100 / total,
            off.millis,
            on.millis
        ));
    }

    let (off, on): (Vec<&Outcome>, Vec<&Outcome>) = (outcomes.iter().filter(|o| !o.on).collect(), outcomes.iter().filter(|o| o.on).collect());
    let sum = |set: &[&Outcome], pick: fn(&Outcome) -> u32| set.iter().map(|outcome| pick(outcome)).sum::<u32>();

    text.push_str(&format!(
        "\n**Completion rate:** off {} - on {}\n**Stopped without a tool:** off {} - on {}\n**Empty-reply stops:** off {} - on {}\n**Overflow deaths:** off {} - on {}\n**Unmatched findings:** off {} - on {}\n",
        rate(&off),
        rate(&on),
        sum(&off, |o| o.stopped_without_tool.unwrap_or(0)),
        sum(&on, |o| o.stopped_without_tool.unwrap_or(0)),
        sum(&off, |o| o.empty_reply_stops.unwrap_or(0)),
        sum(&on, |o| o.empty_reply_stops.unwrap_or(0)),
        sum(&off, |o| u32::from(o.overflow)),
        sum(&on, |o| u32::from(o.overflow)),
        sum(&off, |o| o.unmatched),
        sum(&on, |o| o.unmatched),
    ));

    if !live {
        text.push_str("\nLimits: the scripted model's slips (an announcement, silence, an invented finding, a broken file, a wrong patch) are the author's assumptions, the same in both columns; no cloud chain is configured, so the local share is 100% by construction; the numbers say what the code does with those slips, not how often a real 9B makes them. The live run is the owner's (P4-3).\n");
    }

    text
}

/// Every task, off and on.
pub fn run_all(only: &[String], live: bool) -> Vec<Outcome> {
    let mut outcomes = Vec::new();

    for task in TASKS.iter().filter(|task| only.is_empty() || only.iter().any(|id| id == task.id)) {
        outcomes.push(run_task(task, false, live));
        outcomes.push(run_task(task, true, live));
    }

    outcomes
}

/// `sdcd bench padma [--live] [--tasks a,b] [--out FILE]`
pub fn command(args: &[String]) -> Result<(), String> {
    let flag = |name: &str| args.iter().position(|arg| arg == name).and_then(|index| args.get(index + 1)).cloned();
    let live = args.iter().any(|arg| arg == "--live");
    let only: Vec<String> = flag("--tasks").map(|list| list.split(',').map(|id| id.trim().to_string()).collect()).unwrap_or_default();
    let scratch = std::env::temp_dir().join(format!("sdc-padma-bench-data-{}", std::process::id()));

    /* the bench never touches the person's own settings, history or audit */
    if !live {
        std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
        std::env::set_var("SDC_DATA_DIR", &scratch);
    }

    let table = render(&run_all(&only, live), live);

    println!("{table}");

    if let Some(out) = flag("--out") {
        std::fs::write(&out, &table).map_err(|error| format!("could not write {out}: {error}"))?;
    }

    let _ = std::fs::remove_dir_all(&scratch);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_at_least_ten_tasks_in_three_languages_and_every_prompt_is_short() {
        assert!(TASKS.len() >= 10);
        assert!(TASKS.iter().all(|task| task.prompt.lines().count() <= 2 && task.prompt.chars().count() < 80));
        assert!(TASKS.iter().any(|task| task.prompt.chars().any(|c| ('\u{0980}'..='\u{09FF}').contains(&c))), "a Bengali prompt");
        assert!(TASKS.iter().any(|task| task.id.ends_with("banglish")), "a Banglish prompt");
    }

    /// The whole bench on the scripted model: the same twelve prompts, every switch off against every switch on.
    #[test]
    fn with_the_switches_on_every_short_prompt_reaches_its_recipe_and_finishes_and_with_them_off_the_scripted_slips_end_the_turn() {
        let outcomes = run_all(&[], false);
        let ran = |on: bool| outcomes.iter().filter(|outcome| outcome.on == on && !outcome.skipped).collect::<Vec<_>>();

        for task in TASKS.iter().filter(|task| task.kind != Kind::Fix || (node_available() && npm_available())) {
            let on = outcomes.iter().find(|outcome| outcome.task == task.id && outcome.on).expect("an on-run");

            assert_eq!(on.recipe, task.recipe, "{}: the prompt must reach its recipe ({})", task.id, on.note);
            assert!(on.completed, "{}: {}", task.id, on.note);
            assert_eq!((on.stopped_without_tool, on.empty_reply_stops, on.overflow, on.unmatched, on.cloud_requests), (Some(0), Some(0), false, 0, 0), "{}", task.id);
        }

        assert!(!ran(true).is_empty() && ran(true).len() == ran(false).len());
        assert!(ran(false).iter().all(|outcome| !outcome.completed), "the announcement ends the plain turn: {:?}", ran(false).iter().map(|o| (o.task, o.note.clone())).collect::<Vec<_>>());
        assert!(ran(false).iter().all(|outcome| outcome.stopped_without_tool == Some(1)));

        let table = render(&outcomes, false);

        assert!(table.contains("Completion rate") && table.contains("review-banglish") && table.contains("scripted model"), "{table}");
    }

    #[test]
    fn found_references_are_read_from_a_report() {
        assert_eq!(refs_in("src/auth.py:2 · high\nand `src/store.py:2`, then src/a.rs:9"), vec!["src/auth.py:2", "src/store.py:2"]);
    }
}
