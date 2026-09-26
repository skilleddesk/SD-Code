//! **SDC Agent** - the daemon's own agent loop (docs/ROADMAP-v4.md, Phase 2).
//!
//! Until v4, "give it a prompt and the project gets built" worked exactly as far as the CLI behind the
//! chat could take it: `claude`, `codex` and `gemini` are agents, and an API key or a local model was a
//! question-and-answer chat. This module is the loop those CLIs have inside them, run by the daemon:
//!
//! ```text
//!   ask the model ──► it answers with text, and maybe tool calls
//!        ▲                     │
//!        │          for each call: card ► (checkpoint) ► gate ► run ► result
//!        └───── results go back as the next message, until it stops calling tools
//! ```
//!
//! Why here and not in the window: the eight tools sit on the daemon paths that already exist and
//! already work **on a host** (`fs.*`, `shell.run`, `git.diff` over `ssh`), so an agent whose chat is
//! on a VPS works inside that VPS with no second implementation. The checkpoint before the first
//! change (P5), the deny list, the file guard and the permission dialog are the daemon's, so the agent
//! obeys them without re-implementing them.
//!
//! Bounds, because an unbounded loop is a bill: `max_steps` model calls per turn (default 25), a
//! Stop that drops the connection mid-answer, and a token count on the turn's footer.

pub mod dialect;
pub mod gate;
pub mod tools;
pub mod workspace;

use std::collections::HashSet;

use async_trait::async_trait;
use serde_json::Value;

use crate::engines::{EngineEvent, EngineStatus, EventSink, Prompt, Role};
use dialect::{Dialect, Reply};
use gate::Autonomy;
use tools::ToolContext;
use workspace::Workspace;

/// The default number of model calls one turn may make, and the ceiling a caller may raise it to.
pub const DEFAULT_STEPS: usize = 25;
pub const MAX_STEPS: usize = 80;

/// Where the model for this turn lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// A provider behind an API key (`native_api`'s endpoints).
    Api,
    /// The local Ollama daemon, through its OpenAI-compatible `/v1` endpoint.
    Ollama,
}

pub struct SdcAgent {
    backend: Backend,
    autonomy: Autonomy,
    max_steps: usize,
    checkpoint: Option<tools::Checkpointer>,
}

impl SdcAgent {
    pub fn new(backend: Backend, autonomy: Autonomy, max_steps: usize) -> Self {
        Self { backend, autonomy, max_steps: max_steps.clamp(1, MAX_STEPS), checkpoint: None }
    }

    /// The daemon's checkpoint for this turn, taken by the agent itself before its first change.
    pub fn with_checkpoint(mut self, checkpoint: tools::Checkpointer) -> Self {
        self.checkpoint = Some(checkpoint);
        self
    }
}

#[async_trait]
impl crate::engines::Engine for SdcAgent {
    fn id(&self) -> &'static str {
        "sdc_agent"
    }

    async fn start(&self, prompt: Prompt, sink: &EventSink) {
        let sink = sink.clone();
        let (backend, autonomy, max_steps) = (self.backend, self.autonomy, self.max_steps);
        let checkpoint = self.checkpoint.clone();

        /* Every step blocks - a streaming HTTP read, a file read over ssh, a command - so the whole loop
           runs on the blocking pool, and its events travel through the sink while it does. */
        let _ = tokio::task::spawn_blocking(move || run(backend, autonomy, max_steps, checkpoint, &prompt, &sink)).await;
    }

    async fn cancel(&self, turn_id: &str) -> bool {
        /* The loop checks the cancel mark between every read and every step; setting it (which
           `engine.cancel` has already done) is the whole cancellation. */
        crate::engines::cancel::request(turn_id);

        true
    }

    fn status(&self, _turn_id: &str) -> EngineStatus {
        EngineStatus::Idle
    }
}

/// The resolved endpoint of a turn: where to POST, with which headers, in which dialect.
struct Target {
    url: String,
    headers: Vec<(String, String)>,
    dialect: Dialect,
    model: String,
    /// `$in / $out` per million tokens, when the catalogue knows the model.
    price: Option<(f64, f64)>,
    thinking: bool,
}

fn target(backend: Backend, prompt: &Prompt) -> Result<Target, String> {
    match backend {
        Backend::Ollama => {
            let model = if prompt.model.trim().is_empty() {
                "llama3.2:3b".to_string()
            } else {
                crate::engines::native_api::api_model(&prompt.model).to_string()
            };

            Ok(Target {
                url: "http://127.0.0.1:11434/v1/chat/completions".to_string(),
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                dialect: Dialect::OpenAi,
                model,
                price: None,
                thinking: false,
            })
        }
        Backend::Api => {
            let endpoint = crate::engines::native_api::endpoint_for(&prompt.model, prompt.provider.as_deref());
            let key = crate::auth::keychain::get(&endpoint.key_ref).unwrap_or_default();

            if key.is_empty() {
                return Err(format!(
                    "No API key for {}. Connect it in the Provider Hub; the key is stored in the OS keychain.",
                    endpoint.provider
                ));
            }

            let model = crate::engines::native_api::api_model(&prompt.model).to_string();
            let mut headers = vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("accept".to_string(), "text/event-stream".to_string()),
            ];
            let dialect = if endpoint.dialect == "anthropic" {
                headers.push(("x-api-key".to_string(), key));
                headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
                Dialect::Anthropic
            } else {
                headers.push(("authorization".to_string(), format!("Bearer {key}")));
                Dialect::OpenAi
            };

            Ok(Target {
                url: endpoint.url,
                headers,
                dialect,
                thinking: dialect == Dialect::Anthropic && crate::engines::native_api::adaptive_thinking(&model),
                price: price_of(&endpoint.provider, &model),
                model,
            })
        }
    }
}

/// The catalogue's `"$4 / $20"` for a model, as numbers.
fn price_of(provider: &str, model: &str) -> Option<(f64, f64)> {
    let blocks = crate::providers::models::blocked();
    let row = blocks
        .iter()
        .filter(|block| block.id == provider || provider.is_empty())
        .flat_map(|block| block.models.iter())
        .find(|row| row["id"].as_str() == Some(model))?;

    parse_price(row["cost"].as_str()?)
}

fn parse_price(cost: &str) -> Option<(f64, f64)> {
    let mut numbers = cost.split('/').map(|part| part.trim().trim_start_matches('$').trim().parse::<f64>().ok());

    Some((numbers.next()??, numbers.next()??))
}

/// The turn's footer: tokens in and out, steps, and the price when the catalogue knows it.
fn meta(steps: usize, input: u64, output: u64, price: Option<(f64, f64)>) -> String {
    let tokens = |count: u64| {
        if count >= 1000 { format!("{:.1}k", count as f64 / 1000.0) } else { count.to_string() }
    };
    let mut meta = format!(
        "{} step{} · {} in · {} out",
        steps,
        if steps == 1 { "" } else { "s" },
        tokens(input),
        tokens(output)
    );

    if let Some((per_in, per_out)) = price {
        let cost = input as f64 / 1_000_000.0 * per_in + output as f64 / 1_000_000.0 * per_out;

        /* Two decimals said "≈$0.00" for a real, small bill; below a cent the digits that matter are shown. */
        if cost >= 0.01 {
            meta.push_str(&format!(" · ≈${cost:.2}"));
        } else {
            meta.push_str(&format!(" · ≈${cost:.4}"));
        }
    }

    meta
}

fn system_prompt(workspace: &Workspace) -> String {
    format!(
        "You are SDC Agent, a coding agent working inside a person's project through the tools you are given.\n\
         \n\
         Project folder: {root}\n\
         Machine: {place}\n\
         Shell for run_command: {shell}\n\
         \n\
         How to work:\n\
         - Understand before changing: list the folder and read the files that matter first.\n\
         - For anything with more than two steps, call update_plan first. The person watches that checklist: call update_plan again each time a step starts or finishes, and mark every step done before your final answer.\n\
         - Change files with edit_file (exact text replacement); use write_file for new files or full rewrites.\n\
         - Verify your work: build it, run the tests or run the program with run_command, read the output, and fix what fails.\n\
         - Never start a command that does not exit on its own (a dev server, a watcher). Say how to start it instead.\n\
         - Stay inside the project folder. Secrets (.env, keys) are hidden from you on purpose; do not try to read them.\n\
         - If the person declines an action, do not try it another way; explain what you wanted to do.\n\
         - When you are done, stop calling tools and answer with a short summary: what you changed, how you verified it, and anything the person must do themselves.\n\
         - If the task is unclear or needs a decision only the person can make, ask in your answer instead of guessing.",
        root = workspace.root(),
        place = workspace.place(),
        shell = workspace.shell(),
    )
}

/// A sink that separates the text of one step from the text of the step before it.
///
/// The answer is every step's text in a row; without a break, "Reading the file." and "Fixed it." of
/// two steps run together into one sentence.
fn step_sink(sink: &EventSink, separate: bool) -> EventSink {
    let inner = sink.clone();
    let pending = std::sync::Mutex::new(separate);

    EventSink::new(move |event| {
        if let EngineEvent::Delta(text) = &event {
            if let Ok(mut pending) = pending.lock() {
                if *pending && !text.trim().is_empty() {
                    *pending = false;
                    inner.send(EngineEvent::Delta("\n\n".to_string()));
                }
            }
        }

        inner.send(event);
    })
}

/// The loop.
pub fn run(
    backend: Backend,
    autonomy: Autonomy,
    max_steps: usize,
    checkpoint: Option<tools::Checkpointer>,
    prompt: &Prompt,
    sink: &EventSink,
) {
    let Some(root) = prompt.project_root.as_deref().filter(|root| !root.trim().is_empty()) else {
        sink.send(EngineEvent::Failed(
            "Agent mode works inside a folder, and this chat has none. Open a folder for it (Files → Open folder), or switch to Chat."
                .to_string(),
        ));

        return;
    };

    let workspace = Workspace::new(root, prompt.remote.clone());
    let target = match target(backend, prompt) {
        Ok(target) => target,
        Err(reason) => {
            sink.send(EngineEvent::Failed(reason));

            return;
        }
    };

    drive(backend, &target, &workspace, autonomy, max_steps, checkpoint, prompt, sink);
}

/// The loop over a resolved endpoint - separate from `run` so a test can point it at a loopback
/// server and watch a whole turn: files written, commands run, the answer, the footer.
#[allow(clippy::too_many_arguments)]
fn drive(
    backend: Backend,
    target: &Target,
    workspace: &Workspace,
    autonomy: Autonomy,
    max_steps: usize,
    checkpoint: Option<tools::Checkpointer>,
    prompt: &Prompt,
    sink: &EventSink,
) {
    let turn_id = prompt.turn_id.clone();
    let stopped = || crate::engines::cancel::requested(&turn_id);
    let system = system_prompt(workspace);
    let specs = tools::specs();
    let mut messages: Vec<Value> = prompt
        .history
        .iter()
        .map(|message| match message.role {
            Role::User => dialect::user_message(&message.text),
            Role::Assistant => dialect::assistant_message(&message.text),
        })
        .collect();

    messages.push(dialect::user_message(&prompt.text));

    let mut context = ToolContext {
        workspace,
        sink,
        turn_id: &turn_id,
        autonomy,
        always: HashSet::new(),
        calls: 0,
        checkpoint,
        checkpointed: false,
    };
    let (mut input_tokens, mut output_tokens) = (0u64, 0u64);
    let mut said_something = false;

    for step in 1..=max_steps {
        if stopped() {
            return;
        }

        let body = dialect::body(target.dialect, &target.model, &system, &messages, &specs, target.thinking).to_string();
        let lines = match crate::engines::native_api::open_stream(&target.url, &target.headers, &body) {
            Ok(lines) => lines,
            Err(reason) => {
                sink.send(EngineEvent::Failed(unreachable(backend, &reason)));

                return;
            }
        };
        let reply: Reply = match dialect::read_reply(target.dialect, lines, &step_sink(sink, said_something), &stopped) {
            Ok(reply) => reply,
            Err(reason) if reason == "stopped" => return,
            Err(reason) => {
                sink.send(EngineEvent::Failed(reason));

                return;
            }
        };

        input_tokens += reply.input_tokens;
        output_tokens += reply.output_tokens;
        said_something = said_something || !reply.text.trim().is_empty();

        if reply.stop == "refusal" {
            sink.send(EngineEvent::Failed(
                "The model declined to continue this request (a safety refusal). Rephrase the task, or try another model.".to_string(),
            ));

            return;
        }

        messages.push(reply.message.clone());

        if reply.tool_uses.is_empty() {
            let summary = if reply.stop == "max_tokens" || reply.stop == "length" {
                "Cut off: the answer reached the model's length limit"
            } else {
                "Done"
            };

            sink.send(EngineEvent::Done {
                summary: summary.to_string(),
                meta: meta(step, input_tokens, output_tokens, target.price),
                pass: None,
            });

            return;
        }

        let mut results = Vec::new();

        for call in &reply.tool_uses {
            if stopped() {
                return;
            }

            let outcome = tools::execute(&mut context, call);

            results.push((call.id.clone(), outcome.content, outcome.is_error));
        }

        messages.extend(dialect::tool_results(target.dialect, &results));
    }

    /* The step budget ran out with work still going on. That is said, with the way to continue, rather
       than dressed up as a finished turn. */
    sink.send(EngineEvent::Delta(format!(
        "\n\nI stopped after {max_steps} steps, the limit for one turn. Send \"continue\" to let me keep going from here."
    )));
    sink.send(EngineEvent::Done {
        summary: "Paused at the step limit".to_string(),
        meta: meta(max_steps, input_tokens, output_tokens, target.price),
        pass: None,
    });
}

/// The sentence for a request that never reached a model.
fn unreachable(backend: Backend, reason: &str) -> String {
    match backend {
        Backend::Ollama if reason.contains("refused") || reason.contains("11434") => format!(
            "Ollama is not answering on this machine ({reason}). Start it with `ollama serve`, then send the prompt again."
        ),
        _ => reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalogue_price_is_read_as_two_numbers() {
        assert_eq!(parse_price("$4 / $20"), Some((4.0, 20.0)));
        assert_eq!(parse_price("$0.14 / $0.28"), Some((0.14, 0.28)));
        assert_eq!(parse_price("free"), None);
        assert_eq!(parse_price("subscription"), None);
        assert_eq!(price_of("anthropic-api", "claude-opus-5-5"), Some((4.0, 20.0)));
    }

    #[test]
    fn the_footer_says_what_the_turn_used_and_what_it_cost() {
        assert_eq!(meta(1, 900, 40, None), "1 step · 900 in · 40 out");
        assert_eq!(meta(7, 12_400, 3_100, Some((4.0, 20.0))), "7 steps · 12.4k in · 3.1k out · ≈$0.11");
        assert_eq!(meta(8, 17_500, 886, Some((0.14, 0.28))), "8 steps · 17.5k in · 886 out · ≈$0.0027");
    }

    #[test]
    fn a_chat_without_a_folder_is_told_what_to_do() {
        let recorder = crate::engines::Recorder::new();
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-nofolder".into(),
            text: "build it".into(),
            model: "llama3.2:3b".into(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
                    autonomy: Default::default(),
        };

        run(Backend::Ollama, Autonomy::Ask, 5, None, &prompt, &recorder.sink());

        assert!(matches!(&recorder.events()[..], [EngineEvent::Failed(reason)] if reason.contains("Open a folder")));
    }

    #[test]
    fn steps_are_separated_only_when_the_new_step_says_something() {
        let recorder = crate::engines::Recorder::new();
        let sink = step_sink(&recorder.sink(), true);

        sink.send(EngineEvent::Delta(" ".into()));
        sink.send(EngineEvent::Delta("Fixed it.".into()));
        sink.send(EngineEvent::Delta(" Done.".into()));

        assert_eq!(
            recorder.events(),
            vec![
                EngineEvent::Delta(" ".into()),
                EngineEvent::Delta("\n\n".into()),
                EngineEvent::Delta("Fixed it.".into()),
                EngineEvent::Delta(" Done.".into()),
            ]
        );
    }
}

/// A whole turn against a scripted provider on loopback: the agent must write a real file, run a real
/// command in the folder, feed both results back, and finish with the model's summary.
#[cfg(test)]
mod end_to_end {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// One SSE reply per request, in order. Each request's body is kept so the test can read what the
    /// agent sent back.
    fn provider(replies: Vec<String>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut bodies = Vec::new();

            for reply in replies {
                let (socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
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
                bodies.push(serde_json::from_slice(&body).unwrap());

                let mut socket = socket;

                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{reply}").unwrap();
            }

            bodies
        });

        (url, server)
    }

    fn chunk(delta: Value) -> String {
        format!("data: {}\n\n", serde_json::json!({ "choices": [{ "delta": delta }] }))
    }

    fn call(id: &str, name: &str, arguments: Value) -> String {
        chunk(serde_json::json!({ "tool_calls": [{ "index": 0, "id": id, "function": { "name": name, "arguments": arguments.to_string() } }] }))
            + "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n"
            + "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20}}\n\ndata: [DONE]\n\n"
    }

    #[test]
    fn the_agent_writes_runs_reads_the_results_and_finishes() {
        let root = std::env::temp_dir().join(format!("sdc-agent-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let listing = if cfg!(windows) { "type hello.txt" } else { "cat hello.txt" };
        let (url, server) = provider(vec![
            call("c1", "write_file", serde_json::json!({ "path": "hello.txt", "content": "made by the agent\n" })),
            call("c2", "run_command", serde_json::json!({ "command": listing })),
            chunk(serde_json::json!({ "content": "Created hello.txt and checked it." })) + "data: [DONE]\n\n",
        ]);
        let target = Target {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-e2e".into(),
            text: "make hello.txt".into(),
            model: "local-test".into(),
            provider: None,
            history: vec![crate::engines::Message::user("earlier"), crate::engines::Message::assistant("ok")],
            project_root: Some(root.to_str().unwrap().to_string()),
            remote: None,
                    autonomy: Default::default(),
        };
        let recorder = crate::engines::Recorder::new();

        /* The checkpoint must see the folder as it was before the write - the race this test pins. */
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, bool)>::new()));
        let probe = seen.clone();
        let file = root.join("hello.txt");
        let checkpoint: tools::Checkpointer = std::sync::Arc::new(move |title: &str| {
            probe.lock().unwrap().push((title.to_string(), file.exists()));
        });

        drive(Backend::Api, &target, &workspace, Autonomy::Auto, 10, Some(checkpoint), &prompt, &recorder.sink());

        assert_eq!(
            *seen.lock().unwrap(),
            vec![("Before Create hello.txt".to_string(), false)],
            "one checkpoint, taken before the file existed"
        );

        let bodies = server.join().unwrap();
        let events = recorder.events();

        /* The file is real, and the command that read it back saw what the agent wrote. */
        assert_eq!(std::fs::read_to_string(root.join("hello.txt")).unwrap(), "made by the agent\n");
        assert!(bodies[2]["messages"].as_array().unwrap().iter().any(|message| {
            message["role"] == "tool" && message["content"].as_str().unwrap_or_default().contains("made by the agent")
        }));

        /* The conversation carried the history with its roles, the system prompt first. */
        assert_eq!(bodies[0]["messages"][0]["role"], "system");
        assert_eq!(bodies[0]["messages"][2]["role"], "assistant");
        assert_eq!(bodies[0]["tools"].as_array().unwrap().len(), 8);

        /* The window saw two cards (Create, Run), the answer, and a footer with what the turn used. */
        let cards: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                EngineEvent::ToolStarted { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(cards, ["Create", "Run"]);
        assert!(events.contains(&EngineEvent::Delta("Created hello.txt and checked it.".into())));
        assert!(matches!(events.last(), Some(EngineEvent::Done { meta, .. }) if meta.starts_with("3 steps · 200 in · 40 out")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_step_limit_pauses_the_turn_and_says_how_to_continue() {
        let root = std::env::temp_dir().join(format!("sdc-agent-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let (url, server) = provider(vec![
            call("c1", "list_dir", serde_json::json!({ "path": "." })),
            call("c2", "list_dir", serde_json::json!({ "path": "." })),
        ]);
        let target = Target {
            url,
            headers: Vec::new(),
            dialect: Dialect::OpenAi,
            model: "local-test".to_string(),
            price: None,
            thinking: false,
        };
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let prompt = Prompt {
            session_id: "s1".into(),
            turn_id: "turn-agent-limit".into(),
            text: "look around".into(),
            model: "local-test".into(),
            provider: None,
            history: Vec::new(),
            project_root: Some(root.to_str().unwrap().to_string()),
            remote: None,
                    autonomy: Default::default(),
        };
        let recorder = crate::engines::Recorder::new();

        drive(Backend::Api, &target, &workspace, Autonomy::Ask, 2, None, &prompt, &recorder.sink());
        server.join().unwrap();

        assert!(matches!(recorder.events().last(), Some(EngineEvent::Done { summary, .. }) if summary == "Paused at the step limit"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
