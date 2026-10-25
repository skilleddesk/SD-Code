//! **N9-3 - the live bench** (`sdcd bench navaratna --live`).
//!
//! The scripted bench measures the code around a fake model. This one runs the real step executor (`navaratna::steps::run`) on a real local model
//! (Ollama) in a fresh copy of a small demo project, once with the default switches and once more for each switch asked for, switched on. Each
//! run is judged by the demo's own tests. The numbers are for the model that was used on the machine that ran it - nothing else.
//!
//! `sdcd bench navaratna --live [--model qwen3.5:9b] [--ollama http://127.0.0.1:11434] [--window 16384] [--switches a,b] [--out FILE]`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::scratch;
use crate::navaratna::steps::{self as nr, plan::PlanStep, Check, Config, StepResult};

const FILES: &[(&str, &str)] = &[
    ("README.md", include_str!("demo/README.md")),
    ("textkit/__init__.py", ""),
    ("textkit/markdown.py", include_str!("demo/textkit/markdown.py")),
    ("textkit/slug.py", include_str!("demo/textkit/slug.py")),
    ("textkit/stats.py", include_str!("demo/textkit/stats.py")),
    ("textkit/tokenize.py", include_str!("demo/textkit/tokenize.py")),
    ("textkit/wrap.py", include_str!("demo/textkit/wrap.py")),
    ("tests/__init__.py", ""),
    ("tests/test_textkit.py", include_str!("demo/tests/test_textkit.py")),
    ("pyproject.toml", "[project]\nname = \"textkit\"\nversion = \"0.1\"\n\n[tool.pytest.ini_options]\ntestpaths = [\"tests\"]\n"),
];

const PROMPT: &str = include_str!("demo/prompt.txt");

/// Switches that act inside the step executor and are worth a live comparison.
const DEFAULT_SWITCHES: &[&str] = &["nr.test_first", "nr.best_of_n", "nr.error_slice", "nr.think_budget", "nr.contract_lock", "nr.out_chunk"];

struct Options {
    model: String,
    base: String,
    window: u64,
    switches: Vec<String>,
    out: Option<String>,
}

fn options(args: &[String]) -> Options {
    let value = |flag: &str| args.iter().position(|arg| arg == flag).and_then(|at| args.get(at + 1)).cloned();

    Options {
        model: value("--model").unwrap_or_else(|| "qwen3.5:9b".into()),
        base: value("--ollama").unwrap_or_else(|| "http://127.0.0.1:11434".into()),
        window: value("--window").and_then(|text| text.parse().ok()).unwrap_or(16_384),
        switches: value("--switches").map(|list| list.split(',').map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).collect()).unwrap_or_else(|| DEFAULT_SWITCHES.iter().map(|name| name.to_string()).collect()),
        out: value("--out"),
    }
}

struct Ollama<'a> {
    options: &'a Options,
    calls: usize,
    output_tokens: u64,
}

impl nr::Engine for Ollama<'_> {
    fn complete(&mut self, request: &nr::Request) -> Result<String, String> {
        let mut body = crate::agent::ollama_native::body(
            json!({ "model": self.options.model, "messages": [
                { "role": "system", "content": "You are a careful programming assistant working on a project folder. Follow the requested answer format exactly and answer with nothing else." },
                { "role": "user", "content": request.prompt }
            ] }),
            self.options.window,
        );

        body["stream"] = json!(false);
        body["think"] = json!(request.think.unwrap_or(false));

        if let Some(temperature) = request.temperature {
            body["options"]["temperature"] = json!(temperature);
        }

        let text = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(900))
            .build()
            .post(&format!("{}/api/chat", self.options.base.trim_end_matches('/')))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
            .map_err(|error| format!("Ollama at {} did not answer: {error}", self.options.base))?
            .into_string()
            .map_err(|error| format!("Ollama's answer could not be read: {error}"))?;
        let reply: Value = serde_json::from_str(&text).map_err(|error| format!("Ollama's answer was not JSON: {error}"))?;

        self.calls += 1;
        self.output_tokens += reply["eval_count"].as_u64().unwrap_or(0);

        let answer = reply["message"]["content"].as_str().unwrap_or_default().to_string();

        /* `SDC_STEP_TRACE=file`: every prompt and answer is appended to the file (the same switch the agent's step executor has). */
        if let Some(path) = std::env::var_os("SDC_STEP_TRACE") {
            use std::io::Write;

            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "===== PROMPT ({:?}) =====\n{}\n===== ANSWER =====\n{answer}\n", request.purpose, request.prompt);
            }
        }

        Ok(answer)
    }

    fn name(&self) -> String {
        format!("ollama:{}", self.options.model)
    }
}

struct Demo {
    root: PathBuf,
    plan: Vec<crate::verify::Check>,
}

impl Demo {
    fn new() -> Self {
        let root = scratch("live");

        for (path, text) in FILES {
            let target = root.join(path);

            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let _ = std::fs::write(target, text);
        }

        let names: Vec<String> = FILES.iter().map(|(path, _)| path.to_string()).filter(|path| !path.contains('/')).collect();
        let plan = crate::verify::plan(&names, &|name| std::fs::read_to_string(root.join(name)).ok(), !cfg!(windows));

        Self { root, plan }
    }

    fn run(&self, command: &str) -> Check {
        let line = crate::verify::non_interactive(command, !cfg!(windows));
        let (shell, args) = crate::pty::shell_for_line(&line);

        /* the same timed runner the agent uses: a test the model wrote may loop for ever */
        match crate::pty::PtyManager::new().run_once(&shell, &args, Some(&self.root.to_string_lossy()), Duration::from_secs(180)) {
            Ok(answer) => Check {
                ran: true,
                passed: answer["ok"].as_bool().unwrap_or(false),
                log: format!("{}\n{}", answer["stdout"].as_str().unwrap_or_default(), answer["stderr"].as_str().unwrap_or_default()),
            },
            Err(error) => Check { ran: true, passed: false, log: error.message },
        }
    }
}

impl nr::Project for Demo {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(path)).ok()
    }

    fn files(&self) -> Vec<String> {
        crate::search::files(&self.root, 3_000)
    }

    fn apply(&mut self, step: &str, writes: &[(String, String)]) -> Result<(), String> {
        let edits: Vec<(String, Option<String>)> = writes.iter().map(|(path, text)| (path.clone(), Some(text.clone()))).collect();

        crate::runtime::step::apply(&self.root, step, &edits).map(|_| ()).map_err(|refusal| refusal.0)
    }

    fn put(&mut self, writes: &[(String, Option<String>)]) {
        for (path, text) in writes {
            let target = self.root.join(path);

            match text {
                Some(text) => {
                    if let Some(parent) = target.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }

                    let _ = std::fs::write(target, text);
                }
                None => {
                    let _ = std::fs::remove_file(target);
                }
            }
        }
    }

    fn checks(&mut self) -> Check {
        let commands: Vec<String> = self.plan.iter().take(3).map(|check| check.command.clone()).collect();

        if commands.is_empty() {
            return Check::default();
        }

        for command in &commands {
            let result = self.run(command);

            if !result.passed {
                return result;
            }
        }

        Check { ran: true, passed: true, log: commands.join("\n") }
    }

    fn run_tests(&mut self, _writes: &[(String, String)], _ids: &[String]) -> Option<BTreeMap<String, bool>> {
        None
    }

    fn escalate(&mut self, _step: &PlanStep, _error: &str, _touched: &[String]) -> Option<Check> {
        None
    }

    fn progress(&mut self, line: &str) {
        eprintln!("    {line}");
    }
}

#[derive(Debug, Clone)]
struct Outcome {
    label: String,
    done: usize,
    steps: usize,
    attempts: u32,
    calls: usize,
    output_tokens: u64,
    seconds: u64,
    suite_passes: bool,
    note: String,
}

fn one_run(label: &str, switches: &[(&str, bool)], options: &Options) -> Outcome {
    let started = Instant::now();
    let demo = Demo::new();
    let mut engine = Ollama { options, calls: 0, output_tokens: 0 };
    let root = demo.root.clone();
    let mut demo = demo;

    let (report, note) = crate::v3::with_switches(switches, || {
        let rank = |goal: &str| -> Vec<String> { crate::sme::retrieval::retrieve(&root, goal, 6).into_iter().map(|hit| hit.path).collect() };

        match nr::plan::build(PROMPT, &mut engine, &rank) {
            Ok(plan) => {
                let (report, _) = nr::run(&plan, &mut engine, &mut demo, &Config { budget_tokens: nr::budget_for(options.window, None), ..Config::default() });

                (Some(report), String::new())
            }
            Err(blocked) => (None, format!("not planned: {blocked}")),
        }
    });

    let suite = nr::Project::checks(&mut demo);
    let (done, steps, attempts) = report.as_ref().map_or((0, 0, 0), |report| {
        let done = report.steps.iter().filter(|(_, result)| matches!(result, StepResult::Done { .. })).count();
        let attempts = report.steps.iter().map(|(_, result)| if let StepResult::Done { attempts, .. } = result { *attempts } else { 0 }).sum();

        (done, report.steps.len(), attempts)
    });

    let _ = std::fs::remove_dir_all(&demo.root);

    Outcome { label: label.to_string(), done, steps, attempts, calls: engine.calls, output_tokens: engine.output_tokens, seconds: started.elapsed().as_secs(), suite_passes: suite.ran && suite.passed, note }
}

fn verdict(base: &Outcome, row: &Outcome) -> &'static str {
    let score = |outcome: &Outcome| (outcome.done, outcome.suite_passes, std::cmp::Reverse(outcome.calls));

    match score(row).cmp(&score(base)) {
        std::cmp::Ordering::Greater => "helps",
        std::cmp::Ordering::Equal => "no change",
        std::cmp::Ordering::Less => "worse",
    }
}

fn render(options: &Options, rows: &[Outcome]) -> String {
    let mut out = format!("# NAVARATNA live bench\n\nLIVE run: the real step executor on `{}` (Ollama, num_ctx {}), a fresh copy of a four-step demo for every row, judged by the demo's own tests. Different model, machine or day can give different numbers.\n\n| run | steps done | attempts | model calls | output tokens | seconds | suite passes | vs baseline |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n", options.model, options.window);

    for row in rows {
        let against = if row.label == "baseline" { "-" } else { verdict(&rows[0], row) };

        out.push_str(&format!("| {} | {}/{} | {} | {} | {} | {} | {} | {} |\n", row.label, row.done, row.steps, row.attempts, row.calls, row.output_tokens, row.seconds, if row.suite_passes { "yes" } else { "no" }, against));
    }

    for row in rows.iter().filter(|row| !row.note.is_empty()) {
        out.push_str(&format!("\n{}: {}\n", row.label, row.note));
    }

    out
}

fn ollama_is_up(options: &Options) -> Result<(), String> {
    let url = format!("{}/api/tags", options.base.trim_end_matches('/'));
    let text = ureq::get(&url).timeout(Duration::from_secs(5)).call().map_err(|error| format!("Ollama is not answering at {}: {error}", options.base))?.into_string().map_err(|error| error.to_string())?;
    let tags: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let has = tags["models"].as_array().is_some_and(|models| models.iter().any(|model| model["name"].as_str() == Some(options.model.as_str())));

    if has { Ok(()) } else { Err(format!("Ollama is up but has no model `{}` (run `ollama pull {}`)", options.model, options.model)) }
}

pub fn command(args: &[String]) -> Result<(), String> {
    let options = options(args);

    ollama_is_up(&options)?;

    let mut rows = Vec::new();

    eprintln!("live bench: baseline");
    rows.push(one_run("baseline", &[], &options));

    for name in &options.switches {
        eprintln!("live bench: {name} on");
        rows.push(one_run(&format!("{name} on"), &[(name.as_str(), true)], &options));
    }

    let text = render(&options, &rows);

    println!("{text}");

    if let Some(path) = &options.out {
        std::fs::write(Path::new(path), &text).map_err(|error| format!("{path}: {error}"))?;
        println!("written to {path}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(label: &str, done: usize, calls: usize) -> Outcome {
        Outcome { label: label.into(), done, steps: 4, attempts: 4, calls, output_tokens: 100, seconds: 1, suite_passes: true, note: String::new() }
    }

    #[test]
    fn the_flags_are_read_and_the_defaults_are_the_9b_on_localhost() {
        let given = options(&["--live".into(), "--model".into(), "m:1b".into(), "--switches".into(), "nr.test_first, nr.best_of_n".into()]);

        assert_eq!((given.model.as_str(), given.window, given.switches.len()), ("m:1b", 16_384, 2));
        assert_eq!(options(&[]).base, "http://127.0.0.1:11434");
    }

    #[test]
    fn a_row_is_judged_by_steps_done_first_and_calls_after() {
        let base = outcome("baseline", 3, 10);

        assert_eq!(verdict(&base, &outcome("a", 4, 20)), "helps");
        assert_eq!(verdict(&base, &outcome("b", 3, 10)), "no change");
        assert_eq!(verdict(&base, &outcome("c", 2, 5)), "worse");
        assert_eq!(verdict(&base, &outcome("d", 3, 8)), "helps");
    }

    #[test]
    fn the_report_names_the_model_and_says_it_is_live() {
        let text = render(&options(&[]), &[outcome("baseline", 4, 9), outcome("nr.test_first on", 4, 7)]);

        assert!(text.contains("LIVE run") && text.contains("qwen3.5:9b") && text.contains("| nr.test_first on | 4/4 |") && text.contains("helps"));
    }

    #[test]
    fn the_demo_is_a_working_project_with_a_plan_of_checks() {
        let demo = Demo::new();

        assert!(demo.root.join("textkit/slug.py").exists() && !demo.plan.is_empty());

        let _ = std::fs::remove_dir_all(&demo.root);
    }

    #[test]
    fn without_ollama_the_command_says_so_and_changes_nothing() {
        let error = command(&["--live".into(), "--ollama".into(), "http://127.0.0.1:9".into()]).unwrap_err();

        assert!(error.contains("Ollama is not answering"), "{error}");
    }
}
