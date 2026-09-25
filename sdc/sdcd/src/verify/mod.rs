//! **Verify** - the machine's checks first, then a second AI reads the change (docs/ROADMAP-v4.md, Phase 3).
//!
//! The Verify tab had four fixed rows and a button that toasted; nothing was ever checked. A run is now
//! two stages, in this order and for a reason:
//!
//!   1. **checks** - what the project itself says "working" means: its own `typecheck`/`lint`/`test`/
//!      `build` scripts, `cargo check`/`cargo test`, `go vet`/`go test`, `pytest`. They are found by
//!      reading the folder's manifests, run in the folder (on the host, when the chat is on one), and
//!      each row says the command, how long it took and the tail of its output.
//!   2. **review** - a *different* engine than the one that wrote the change reads its diff (since the
//!      turn's checkpoint, new files included) and answers with a verdict and issues pinned to
//!      `file:line`. Different, because a model reviewing its own work shares its own blind spots; a
//!      diff, because the reviewer should read what changed, not the whole repository.
//!
//! A failing check skips the review by default: what a compiler can say costs nothing, and a model
//! asked to review code that does not build spends tokens telling you it does not build.
//!
//! The whole run travels as one `VerifyUpdated` snapshot per change, so a window that joins late draws
//! the same thing as one that watched it start.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::agent::workspace::Workspace;
use crate::engines::{EngineEvent, Prompt, Recorder};
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;
use crate::DaemonState;

/// How long one check may run. A test suite that takes longer is a suite to run by hand.
const CHECK_TIMEOUT: Duration = Duration::from_secs(300);

/// The most diff a reviewer is handed. Past this the review would be of a summary nobody wrote.
const DIFF_CAP: usize = 80_000;

/// Output lines a check row keeps.
const TAIL_LINES: usize = 12;

/// One check to run: the row's name and the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub command: String,
}

/// Who reviews: an engine, its model and the provider the model came from.
#[derive(Debug, Clone)]
pub struct Reviewer {
    pub engine: String,
    pub model: String,
    pub provider: Option<String>,
}

pub struct Request {
    pub verify_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    /// The turn's first checkpoint's shadow commit: the review reads everything since it.
    pub since: Option<String>,
    /// What the turn was asked to do, for the reviewer.
    pub task: String,
    pub root: String,
    pub remote: Option<crate::ssh::Ssh>,
    pub reviewer: Option<Reviewer>,
    pub review_failing: bool,
}

/// The checks a folder's manifests promise, in the order a person would run them: fast and loud first.
///
/// `files` is the folder's top level; `read` returns a manifest's text. Pure, so the rules are tested
/// without running anything.
pub fn plan(files: &[String], read: &dyn Fn(&str) -> Option<String>, posix: bool) -> Vec<Check> {
    let has = |name: &str| files.iter().any(|file| file == name);
    let mut checks = Vec::new();

    if has("package.json") {
        let manifest: Value = read("package.json")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        let scripts = manifest["scripts"].as_object().cloned().unwrap_or_default();
        let runner = if has("pnpm-lock.yaml") {
            "pnpm run"
        } else if has("yarn.lock") {
            "yarn"
        } else if has("bun.lockb") || has("bun.lock") {
            "bun run"
        } else {
            "npm run"
        };

        for name in ["typecheck", "type-check", "lint", "test", "build"] {
            let Some(script) = scripts.get(name).and_then(Value::as_str) else {
                continue;
            };

            /* A script that never exits is not a check: `vite`, `next dev`, anything watching. */
            let lowered = script.to_lowercase();

            if lowered.contains("--watch") || lowered.contains(" dev") || lowered.trim() == "vite" {
                continue;
            }

            /* The npm placeholder `echo "Error: no test specified" && exit 1` fails by design. */
            if lowered.contains("no test specified") {
                continue;
            }

            let row = if name == "type-check" { "typecheck" } else { name };

            if checks.iter().any(|check: &Check| check.name == row) {
                continue;
            }

            checks.push(Check { name: row.to_string(), command: format!("{runner} {name}") });
        }
    }

    if has("Cargo.toml") {
        checks.push(Check { name: "cargo check".into(), command: "cargo check --all-targets".into() });
        checks.push(Check { name: "cargo test".into(), command: "cargo test".into() });
    }

    if has("go.mod") {
        checks.push(Check { name: "go vet".into(), command: "go vet ./...".into() });
        checks.push(Check { name: "go test".into(), command: "go test ./...".into() });
    }

    let python = has("pyproject.toml") || has("setup.py") || has("requirements.txt");
    let pytest = has("tests") || has("pytest.ini") || read("pyproject.toml").map(|text| text.contains("pytest")).unwrap_or(false);

    if python && pytest {
        let interpreter = if posix { "python3" } else { "python" };

        checks.push(Check { name: "pytest".into(), command: format!("{interpreter} -m pytest -q") });
    }

    checks
}

/// A command with `CI=true`, which is what makes test runners run once instead of watching, and stops
/// colour codes and prompts - spelled for the shell it runs in.
fn non_interactive(command: &str, posix: bool) -> String {
    if posix {
        format!("CI=true {command}")
    } else {
        format!("set CI=true&& {command}")
    }
}

/// The reviewer's instructions: what to look for, and the one JSON shape to answer in.
pub fn review_prompt(task: &str, diff: &str) -> String {
    format!(
        "You are reviewing a code change that another AI made. You did not write it; find what is wrong with it.\n\
         \n\
         The task it was given:\n{task}\n\
         \n\
         The change, as a unified diff (new files included):\n```diff\n{diff}\n```\n\
         \n\
         Look for: bugs and wrong logic, a task that was only partly done, broken edge cases, security problems \
         (injection, secrets, unsafe input), and code that will fail at runtime. Do not report style or naming.\n\
         \n\
         Answer with ONLY one JSON object, no prose around it:\n\
         {{\"verdict\": \"pass\" or \"issues\", \"summary\": \"one or two sentences\", \"issues\": [{{\"file\": \"path\", \"line\": 12, \"severity\": \"high\" or \"medium\" or \"low\", \"message\": \"what is wrong and why\", \"fix\": \"how to fix it\"}}]}}\n\
         Use an empty issues list when the change is correct."
    )
}

/// The reviewer's answer, read as the JSON it was asked for - or said to be unreadable, with its words.
pub fn parse_review(text: &str) -> Value {
    let parsed = text
        .find('{')
        .zip(text.rfind('}'))
        .filter(|(start, end)| start < end)
        .and_then(|(start, end)| serde_json::from_str::<Value>(&text[start..=end]).ok());

    let Some(parsed) = parsed.filter(|value| value.get("verdict").is_some()) else {
        return json!({
            "verdict": "unreadable",
            "summary": format!(
                "The reviewer did not answer in the requested format. Its words: {}",
                text.trim().chars().take(600).collect::<String>()
            ),
            "issues": [],
        });
    };

    let issues: Vec<Value> = parsed["issues"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|issue| issue["message"].as_str().is_some_and(|message| !message.trim().is_empty()))
        .take(30)
        .map(|issue| {
            json!({
                "file": issue["file"].as_str().unwrap_or_default(),
                "line": issue["line"].as_u64(),
                "severity": match issue["severity"].as_str().unwrap_or("medium") {
                    "high" => "high",
                    "low" => "low",
                    _ => "medium",
                },
                "message": issue["message"],
                "fix": issue["fix"].as_str().unwrap_or_default(),
            })
        })
        .collect();
    /* The issues decide, not the word: a reviewer that says "issues" and lists none has found nothing
       a person could act on. */
    let verdict = if issues.is_empty() { "pass" } else { "issues" };

    json!({
        "verdict": verdict,
        "summary": parsed["summary"].as_str().unwrap_or_default(),
        "issues": issues,
    })
}

/// The run as one snapshot - what `VerifyUpdated` carries and what the Verify tab draws.
struct Run {
    request_id: String,
    session_id: String,
    turn_id: Option<String>,
    state: &'static str,
    pass: Option<bool>,
    checks: Vec<Value>,
    review: Option<Value>,
    note: String,
}

impl Run {
    fn push(&self, out: &Arc<dyn Notifier>) {
        out.push(
            event::verify_updated(json!({
                "verifyId": self.request_id,
                "sessionId": self.session_id,
                "turnId": self.turn_id,
                "state": self.state,
                "pass": self.pass,
                "checks": self.checks,
                "review": self.review,
                "note": self.note,
            })),
            Some(self.session_id.clone()),
            self.turn_id.clone(),
        );
    }
}

fn tail(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();

    lines.iter().rev().take(TAIL_LINES).rev().map(|line| line.chars().take(300).collect()).collect()
}

/// The whole run. Blocking stages run on the blocking pool; the reviewer is an engine turn.
pub async fn run(state: Arc<DaemonState>, out: Arc<dyn Notifier>, request: Request) {
    let posix = request.remote.is_some() || !cfg!(windows);
    let workspace = Arc::new(Workspace::new(&request.root, request.remote.clone()));
    let mut run = Run {
        request_id: request.verify_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        state: "running",
        pass: None,
        checks: Vec::new(),
        review: request.reviewer.as_ref().map(|reviewer| {
            json!({ "engine": reviewer.engine, "model": reviewer.model, "status": "pending" })
        }),
        note: String::new(),
    };

    /* Stage 1: which checks this folder has. */
    let planned = {
        let workspace = workspace.clone();

        tokio::task::spawn_blocking(move || {
            let files: Vec<String> = workspace
                .list(".")
                .map(|(entries, _)| entries.into_iter().map(|entry| entry.split(" (").next().unwrap_or_default().trim_end_matches('/').to_string()).collect())
                .unwrap_or_default();

            plan(&files, &|name| workspace.read(name).ok().map(|(text, _)| text), posix)
        })
        .await
        .unwrap_or_default()
    };

    run.checks = planned
        .iter()
        .map(|check| json!({ "name": check.name, "command": check.command, "status": "pending", "ms": Value::Null, "tail": [] }))
        .collect();

    if planned.is_empty() {
        run.note = "No checks found: the folder has no package.json scripts, Cargo.toml, go.mod or pytest setup to run.".into();
    }

    run.push(&out);

    let mut failed = false;

    for (index, check) in planned.iter().enumerate() {
        if crate::engines::cancel::requested(&request.verify_id) {
            break;
        }

        run.checks[index]["status"] = json!("running");
        run.push(&out);

        let line = non_interactive(&check.command, posix);
        let result = {
            let workspace = workspace.clone();

            tokio::task::spawn_blocking(move || workspace.run(&line, CHECK_TIMEOUT)).await
        };

        match result {
            Ok(Ok(report)) => {
                let passed = report.ok;

                failed = failed || !passed;
                run.checks[index]["status"] = json!(if passed { "pass" } else { "fail" });
                run.checks[index]["ms"] = json!(report.duration_ms);
                run.checks[index]["tail"] = json!(tail(&format!("{}\n{}", report.stdout, report.stderr)));

                if report.timed_out {
                    run.checks[index]["tail"] = json!([format!("stopped after {}s", CHECK_TIMEOUT.as_secs())]);
                }
            }
            Ok(Err(error)) => {
                failed = true;
                run.checks[index]["status"] = json!("fail");
                run.checks[index]["tail"] = json!([error.message]);
            }
            Err(_) => {
                failed = true;
                run.checks[index]["status"] = json!("fail");
            }
        }

        run.push(&out);
    }

    /* Stage 2: the review - by a different engine, on the diff. */
    let mut review_failed = false;

    if let Some(reviewer) = request.reviewer.clone() {
        if failed && !request.review_failing {
            run.review = Some(json!({
                "engine": reviewer.engine,
                "model": reviewer.model,
                "status": "skipped",
                "summary": "Skipped: a check failed. Fix it first, or run the review anyway.",
            }));
        } else {
            run.review = Some(json!({ "engine": reviewer.engine, "model": reviewer.model, "status": "running" }));
            run.push(&out);

            let review = review(&state, &workspace, &request, &reviewer).await;

            review_failed = review["status"] == "failed" || review["verdict"] == "issues";
            run.review = Some(review);
        }
    }

    run.state = "done";
    run.pass = Some(!failed && !review_failed);
    run.push(&out);
    crate::engines::cancel::clear(&request.verify_id);
}

async fn review(state: &Arc<DaemonState>, workspace: &Arc<Workspace>, request: &Request, reviewer: &Reviewer) -> Value {
    let base = json!({ "engine": reviewer.engine, "model": reviewer.model });
    let failed = |summary: String| {
        let mut value = base.clone();

        value["status"] = json!("failed");
        value["summary"] = json!(summary);
        value
    };

    let diff = {
        let workspace = workspace.clone();
        let since = request.since.clone();

        tokio::task::spawn_blocking(move || match since {
            Some(sha) => workspace.changes_since(&sha),
            None => workspace.diff(),
        })
        .await
    };
    let diff = match diff {
        Ok(Ok(diff)) => diff,
        Ok(Err(error)) => return failed(format!("Could not read the change: {}", error.message)),
        Err(_) => return failed("Could not read the change.".into()),
    };

    if diff.trim().is_empty() {
        let mut value = base.clone();

        value["status"] = json!("done");
        value["verdict"] = json!("pass");
        value["summary"] = json!("Nothing to review: the folder has no changes since the checkpoint.");
        value["issues"] = json!([]);

        return value;
    }

    let diff: String = if diff.len() > DIFF_CAP {
        format!("{}\n[… the diff is longer; the rest was cut …]", diff.chars().take(DIFF_CAP).collect::<String>())
    } else {
        diff
    };

    let Some(engine) = state.engines.get(&reviewer.engine) else {
        return failed(format!("`{}` is not an engine this daemon has.", reviewer.engine));
    };

    let prompt = Prompt {
        session_id: request.session_id.clone(),
        turn_id: format!("{}-review", request.verify_id),
        text: review_prompt(&request.task, &diff),
        model: reviewer.model.clone(),
        provider: reviewer.provider.clone(),
        history: Vec::new(),
        project_root: Some(request.root.clone()),
        remote: request.remote.clone(),
    };
    let recorder = Recorder::new();

    engine.start(prompt, &recorder.sink()).await;

    let events = recorder.events();
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::Delta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();

    if text.trim().is_empty() {
        let reason = events
            .iter()
            .find_map(|event| match event {
                EngineEvent::Failed(reason) => Some(reason.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "the reviewer answered with nothing".into());

        return failed(format!("The reviewer could not run: {reason}"));
    }

    let mut value = parse_review(&text);

    value["engine"] = json!(reviewer.engine);
    value["model"] = json!(reviewer.model);
    value["status"] = json!("done");

    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_node_project_runs_its_own_scripts_with_its_own_package_manager() {
        let manifest = r#"{"scripts":{"dev":"vite","build":"vite build","test":"vitest","lint":"eslint .","typecheck":"tsc --noEmit"}}"#;
        let checks = plan(&files(&["package.json", "pnpm-lock.yaml"]), &|_| Some(manifest.to_string()), true);

        assert_eq!(
            checks.iter().map(|check| check.command.as_str()).collect::<Vec<_>>(),
            ["pnpm run typecheck", "pnpm run lint", "pnpm run test", "pnpm run build"]
        );
    }

    #[test]
    fn scripts_that_never_exit_or_always_fail_are_not_checks() {
        let manifest = r#"{"scripts":{"build":"vite","test":"echo \"Error: no test specified\" && exit 1","lint":"eslint . --watch"}}"#;

        assert!(plan(&files(&["package.json"]), &|_| Some(manifest.to_string()), true).is_empty());
    }

    #[test]
    fn rust_go_and_python_projects_get_their_toolchains_checks() {
        let rust = plan(&files(&["Cargo.toml"]), &|_| None, true);
        let go = plan(&files(&["go.mod"]), &|_| None, true);
        let python = plan(&files(&["pyproject.toml", "tests"]), &|_| Some(String::new()), false);

        assert_eq!(rust[1].command, "cargo test");
        assert_eq!(go[0].command, "go vet ./...");
        assert_eq!(python[0].command, "python -m pytest -q");
        assert!(plan(&files(&["requirements.txt"]), &|_| None, true).is_empty(), "no tests, no pytest");
        assert!(plan(&files(&["README.md"]), &|_| None, true).is_empty());
    }

    #[test]
    fn the_ci_flag_is_spelled_for_the_shell_it_runs_in() {
        assert_eq!(non_interactive("npm test", true), "CI=true npm test");
        assert_eq!(non_interactive("npm test", false), "set CI=true&& npm test");
    }

    #[test]
    fn a_review_is_read_from_the_json_even_with_words_around_it() {
        let review = parse_review(
            "Here is my review:\n{\"verdict\":\"issues\",\"summary\":\"One bug.\",\"issues\":[{\"file\":\"src/pay.js\",\"line\":87,\"severity\":\"high\",\"message\":\"Rounds before the cap.\",\"fix\":\"Cap first.\"},{\"file\":\"x\",\"message\":\"  \"}]}\nThanks.",
        );

        assert_eq!(review["verdict"], "issues");
        assert_eq!(review["issues"].as_array().unwrap().len(), 1, "an empty message is dropped");
        assert_eq!(review["issues"][0]["line"], 87);
        assert_eq!(review["issues"][0]["severity"], "high");
    }

    #[test]
    fn a_review_with_no_issues_passes_and_an_unreadable_one_says_so() {
        assert_eq!(parse_review(r#"{"verdict":"issues","summary":"","issues":[]}"#)["verdict"], "pass");
        assert_eq!(parse_review("Looks fine to me!")["verdict"], "unreadable");
        assert!(parse_review("Looks fine to me!")["summary"].as_str().unwrap().contains("Looks fine"));
    }

    #[test]
    fn the_review_prompt_carries_the_task_and_the_diff() {
        let prompt = review_prompt("fix the 500", "+let x = 1;");

        assert!(prompt.contains("fix the 500"));
        assert!(prompt.contains("+let x = 1;"));
        assert!(prompt.contains("\"verdict\""));
    }

    /// The diff a review reads includes files that did not exist at the checkpoint.
    #[test]
    fn changes_since_a_checkpoint_include_new_files() {
        let root = std::env::temp_dir().join(format!("sdc-verify-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();

        let sha = match crate::git::checkpoint(&root, "test checkpoint") {
            Ok(sha) => sha,
            /* No git on this machine: nothing to measure, and the doctor says so elsewhere. */
            Err(_) => return,
        };

        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        std::fs::write(root.join("new.txt"), "brand new\n").unwrap();

        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let diff = workspace.changes_since(&sha).unwrap();

        assert!(diff.contains("+two"), "{diff}");
        assert!(diff.contains("new.txt") && diff.contains("+brand new"), "{diff}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
