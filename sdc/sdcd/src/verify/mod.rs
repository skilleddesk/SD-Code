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
//!
//! 0.12 (the Trust Kernel's Verify engine) adds, between the two:
//!
//!   * **scans** - the change's added lines through the secret scanner and the SAST rules
//!     (`trust::scan`), always, and the project's dependency audit (`npm audit`, `composer audit`,
//!     `pip-audit`, `cargo audit`) when its tool is there;
//!   * **a verdict with four words** - `PASS` (checks ran and passed, scans clean, the review found
//!     nothing), `FAIL`, `NO_CHECKS` (the folder has nothing to run and nothing changed), `UNPROVEN`
//!     (nothing contradicts the change, but no test proved it either);
//!   * **the acceptance criteria** of a confirmed Intent Contract, which the reviewer judges one by one;
//!   * a **security focus** for the role pipeline's SecReview step.

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
    /// The confirmed Intent Contract's conditions: the reviewer says for each whether the change meets it.
    pub acceptance: Vec<String>,
    /// `security` makes the review a security review (the role pipeline's SecReview step).
    pub focus: Option<String>,
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
pub fn non_interactive(command: &str, posix: bool) -> String {
    if posix {
        format!("CI=true {command}")
    } else {
        format!("set CI=true&& {command}")
    }
}

/// The reviewer's instructions with the Trust Kernel's additions: the acceptance criteria to judge one by
/// one, and - for a security review - what to look at first.
pub fn review_prompt_with(task: &str, diff: &str, acceptance: &[String], focus: Option<&str>) -> String {
    let mut prompt = review_prompt(task, diff);

    if focus == Some("security") {
        prompt.push_str(
            "\n\nThis is a SECURITY review. Look first for: injection (SQL, shell, template), cross-site scripting, broken \
             authentication or authorization, secrets in code, unsafe deserialisation, path traversal, SSRF, missing input \
             validation, and dangerous defaults. Mark anything exploitable as severity high.",
        );
    }

    if !acceptance.is_empty() {
        let list: Vec<String> = acceptance.iter().enumerate().map(|(index, text)| format!("{}. {text}", index + 1)).collect();

        prompt.push_str(&format!(
            "\n\nThe person agreed that the task is done when all of these hold:\n{}\n\
             Add to your JSON a \"criteria\" list with one entry per condition, in order: {{\"met\": true or false, \"why\": \"one sentence\"}}.",
            list.join("\n")
        ));
    }

    prompt
}

/// The review prompt with the checks SDC ran and their results, stated as facts. The diff shows only what
/// changed; files that were already there (a test script, a config) are not in it, and a reviewer must not
/// judge a condition unmet for want of seeing them when a check has already proved it.
pub fn with_facts(prompt: String, facts: &[String]) -> String {
    if facts.is_empty() {
        return prompt;
    }

    format!(
        "{prompt}\n\nFacts, measured by SDC just now in the project folder - do not contradict them; the project has files that are not in the diff. A condition about the project's tests, build, lint or types is MET when the matching check below PASSED:\n{}",
        facts.iter().map(|fact| format!("- {fact}")).collect::<Vec<_>>().join("\n")
    )
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

    let criteria: Vec<Value> = parsed["criteria"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|criterion| json!({ "met": criterion["met"].as_bool(), "why": criterion["why"].as_str().unwrap_or_default() }))
        .collect();
    /* An unmet condition the person agreed to is an issue, whatever else the reviewer found. */
    let verdict = if criteria.iter().any(|criterion| criterion["met"] == false) { "issues" } else { verdict };

    json!({
        "verdict": verdict,
        "summary": parsed["summary"].as_str().unwrap_or_default(),
        "issues": issues,
        "criteria": criteria,
    })
}

/// A condition the person agreed to that a check SDC ran has **measured** is decided by the measurement,
/// not by a reviewer's reading of the diff (P3). A condition about the tests, the build, the lint or the
/// types whose matching check passed is met; the reason says it was measured. Found live: a reviewer that
/// saw only `math.js` in the diff said "npm test cannot pass" twice, right after it had passed.
pub fn measured_criteria(review: &mut Value, acceptance: &[String], checks: &[Value]) {
    let Some(criteria) = review["criteria"].as_array_mut() else {
        return;
    };
    let passed = |names: &[&str]| -> Option<String> {
        checks
            .iter()
            .find(|check| check["status"] == "pass" && names.iter().any(|name| check["name"].as_str().is_some_and(|check_name| check_name.contains(name))))
            .and_then(|check| check["command"].as_str().map(str::to_string))
    };

    for (index, criterion) in criteria.iter_mut().enumerate() {
        if criterion["met"] != false {
            continue;
        }

        let Some(text) = acceptance.get(index).map(|text| text.to_lowercase()) else {
            continue;
        };
        let measured = if text.contains("test") {
            passed(&["test", "pytest"])
        } else if text.contains("build") || text.contains("compile") {
            passed(&["build", "cargo check"])
        } else if text.contains("lint") {
            passed(&["lint", "vet"])
        } else if text.contains("type") {
            passed(&["typecheck"])
        } else {
            None
        };

        if let Some(command) = measured {
            criterion["met"] = json!(true);
            criterion["why"] = json!(format!("Measured by SDC: `{command}` passed. (The reviewer judged it from the diff alone.)"));
            criterion["measured"] = json!(true);
        }
    }

    let unmet = criteria.iter().any(|criterion| criterion["met"] == false);
    let high = review["issues"].as_array().is_some_and(|issues| issues.iter().any(|issue| issue["severity"] == "high"));

    if !unmet && !high && review["verdict"] == "issues" {
        /* What is left are observations, not blockers: the review passes, and the issues stay listed. */
        review["verdict"] = json!("pass");
    }
}

/// The dependency audit a folder's manifests allow: `(name, command)`, or `None`.
pub fn dependency_audit(files: &[String], posix: bool) -> Option<(&'static str, String)> {
    let has = |name: &str| files.iter().any(|file| file == name);
    let quiet = if posix { " 2>/dev/null" } else { "" };

    if has("package-lock.json") {
        Some(("npm audit", format!("npm audit --json --omit=dev{quiet}")))
    } else if has("pnpm-lock.yaml") {
        Some(("pnpm audit", format!("pnpm audit --json --prod{quiet}")))
    } else if has("composer.lock") {
        Some(("composer audit", format!("composer audit --format=json --no-interaction{quiet}")))
    } else if has("requirements.txt") {
        Some(("pip-audit", format!("pip-audit -r requirements.txt -f json{quiet}")))
    } else if has("Cargo.lock") {
        Some(("cargo audit", format!("cargo audit --json{quiet}")))
    } else {
        None
    }
}

/// A dependency audit's JSON as counts by severity - npm's and pnpm's `metadata.vulnerabilities`,
/// composer's `advisories`, pip-audit's `dependencies[].vulns`, cargo-audit's `vulnerabilities.list`.
pub fn read_audit(text: &str) -> Option<Value> {
    let start = text.find('{').or_else(|| text.find('['))?;
    let value: Value = serde_json::from_str(text[start..].trim()).ok()?;
    let mut counts = json!({ "critical": 0, "high": 0, "moderate": 0, "low": 0 });

    if let Some(vulnerabilities) = value.pointer("/metadata/vulnerabilities").and_then(Value::as_object) {
        for key in ["critical", "high", "moderate", "low"] {
            counts[key] = json!(vulnerabilities.get(key).and_then(Value::as_u64).unwrap_or(0));
        }
    } else if let Some(advisories) = value.get("advisories").and_then(Value::as_object) {
        let total: usize = advisories.values().map(|list| list.as_array().map(Vec::len).unwrap_or(0)).sum();

        counts["high"] = json!(total);
    } else if let Some(list) = value.pointer("/vulnerabilities/list").and_then(Value::as_array) {
        counts["high"] = json!(list.len());
    } else {
        let dependencies = value.get("dependencies").and_then(Value::as_array)?;
        let total: usize = dependencies.iter().map(|dependency| dependency["vulns"].as_array().map(Vec::len).unwrap_or(0)).sum();

        counts["high"] = json!(total);
    }

    Some(counts)
}

/// The run's word: `PASS`, `FAIL`, `NO_CHECKS` or `UNPROVEN` (P4 - "nothing failed" is not "proved").
pub fn verdict(checks: &[Value], scans: &Value, review: Option<&Value>, changed: bool) -> &'static str {
    let ran: Vec<&Value> = checks.iter().filter(|check| check["status"] == "pass" || check["status"] == "fail").collect();
    let failed_check = ran.iter().any(|check| check["status"] == "fail");
    let secrets = scans["secrets"].as_array().map(Vec::len).unwrap_or(0);
    let sast_high = scans["sast"].as_array().map(|findings| findings.iter().filter(|finding| finding["severity"] == "high").count()).unwrap_or(0);
    let deps_bad = scans["dependencies"]["counts"]["critical"].as_u64().unwrap_or(0) + scans["dependencies"]["counts"]["high"].as_u64().unwrap_or(0) > 0;
    let review_issues = review.is_some_and(|review| {
        review["verdict"] == "issues" && review["issues"].as_array().map(|issues| issues.iter().any(|issue| issue["severity"] == "high")).unwrap_or(false)
            || review["criteria"].as_array().map(|criteria| criteria.iter().any(|criterion| criterion["met"] == false)).unwrap_or(false)
    });

    if failed_check || secrets > 0 || sast_high > 0 || deps_bad || review_issues {
        return "FAIL";
    }

    if ran.is_empty() && !changed && review.is_none() {
        return "NO_CHECKS";
    }

    let reviewed_clean = review.is_some_and(|review| review["verdict"] == "pass");

    if ran.is_empty() && !reviewed_clean {
        return "UNPROVEN";
    }

    "PASS"
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
    scans: Value,
    verdict: Option<&'static str>,
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
                "scans": self.scans,
                "verdict": self.verdict,
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
        scans: json!({ "state": "pending" }),
        verdict: None,
    };

    crate::trust::kill::begin(&request.verify_id, "verify", Some(&request.session_id), "Verify");

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

    /* Stage 1b (0.12): the change's added lines through the secret scanner and the SAST rules, and the
       project's dependency audit when its tool is there. Rules, not opinions: every finding names its rule. */
    let diff_text = {
        let workspace = workspace.clone();
        let since = request.since.clone();

        tokio::task::spawn_blocking(move || match since {
            Some(sha) => workspace.changes_since(&sha),
            None => workspace.diff(),
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
    };
    let changed = !diff_text.trim().is_empty();
    let (secret_rules, sast_rules) = crate::trust::scan::rule_counts();

    run.scans = json!({
        "state": "running",
        "secrets": crate::trust::scan::secrets(&diff_text),
        "sast": crate::trust::scan::sast(&diff_text),
        "rules": { "secrets": secret_rules, "sast": sast_rules },
        "dependencies": Value::Null,
    });
    run.push(&out);

    let audit = {
        let workspace = workspace.clone();

        tokio::task::spawn_blocking(move || {
            let files: Vec<String> = workspace
                .list(".")
                .map(|(entries, _)| entries.into_iter().map(|entry| entry.split(" (").next().unwrap_or_default().trim_end_matches('/').to_string()).collect())
                .unwrap_or_default();
            let (name, command) = dependency_audit(&files, posix)?;
            let report = workspace.run(&command, Duration::from_secs(180)).ok()?;

            Some(match read_audit(&report.stdout) {
                Some(counts) => json!({ "tool": name, "counts": counts, "status": "done" }),
                None => json!({ "tool": name, "status": "unavailable", "detail": tail(&format!("{}\n{}", report.stdout, report.stderr)).join(" ") }),
            })
        })
        .await
        .ok()
        .flatten()
    };

    run.scans["dependencies"] = audit.unwrap_or_else(|| json!({ "status": "none", "detail": "No lock file with an audit tool on this machine: dependencies were not checked." }));
    run.scans["state"] = json!("done");
    run.push(&out);

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

            /* What SDC itself ran is a fact the reviewer is given and may not contradict (found live in 0.12: a
               reviewer that saw only the diff said "npm test cannot pass" right after npm test had passed). */
            let facts: Vec<String> = run
                .checks
                .iter()
                .filter(|check| check["status"] == "pass" || check["status"] == "fail")
                .map(|check| {
                    format!(
                        "the project's own {} check (`{}`, the same as running its {} script by any other spelling) {}",
                        check["name"].as_str().unwrap_or_default(),
                        check["command"].as_str().unwrap_or_default(),
                        check["name"].as_str().unwrap_or_default(),
                        if check["status"] == "pass" { "PASSED" } else { "FAILED" }
                    )
                })
                .collect();
            let mut review = review(&state, &workspace, &request, &reviewer, &facts).await;

            measured_criteria(&mut review, &request.acceptance, &run.checks);

            review_failed = review["status"] == "failed" || review["verdict"] == "issues";
            run.review = Some(review);
        }
    }

    let word = verdict(&run.checks, &run.scans, run.review.as_ref().filter(|review| review["status"] == "done"), changed);

    run.state = "done";
    run.verdict = Some(word);
    /* `pass` means proven (P4): PASS and nothing else - an UNPROVEN run is not a passing one. */
    let _ = review_failed;
    run.pass = Some(!failed && word == "PASS");
    run.push(&out);
    crate::engines::cancel::clear(&request.verify_id);
    crate::trust::kill::end(&request.verify_id);

    /* The turn this run verified is scored again: a proven turn loses the "not verified" cap. */
    if let Some(turn) = request.turn_id.as_deref() {
        let policy = crate::trust::policy::Policy::load(Some(&request.root), request.remote.as_ref());

        crate::sdcp::methods::score_turn(&state, &*out, &request.session_id, turn, 0, &policy, Some(&request.root));
    }
}

async fn review(state: &Arc<DaemonState>, workspace: &Arc<Workspace>, request: &Request, reviewer: &Reviewer, facts: &[String]) -> Value {
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
        /* VR-6: nothing changed is not "the change is correct" - there is no change. The review is skipped,
           and said to be. */
        let mut value = base.clone();

        value["status"] = json!("skipped");
        value["verdict"] = Value::Null;
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
        text: with_facts(review_prompt_with(&request.task, &diff, &request.acceptance, request.focus.as_deref()), facts),
        model: reviewer.model.clone(),
        provider: reviewer.provider.clone(),
        history: Vec::new(),
        project_root: Some(request.root.clone()),
        remote: request.remote.clone(),
        /* A review only reads; the careful level is the right one for it. */
        autonomy: crate::agent::gate::Autonomy::Ask,
        resume: None,
        images: Vec::new(),
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

    #[test]
    fn the_verdict_has_four_words_and_never_calls_unproven_work_a_pass() {
        let pass = json!({ "status": "pass" });
        let fail = json!({ "status": "fail" });
        let clean = json!({ "secrets": [], "sast": [], "dependencies": { "counts": { "critical": 0, "high": 0 } } });
        let leaked = json!({ "secrets": [{ "rule": "aws-access-key" }], "sast": [] });
        let reviewed = json!({ "verdict": "pass", "issues": [], "criteria": [] });

        assert_eq!(verdict(std::slice::from_ref(&pass), &clean, None, true), "PASS");
        assert_eq!(verdict(&[fail], &clean, None, true), "FAIL");
        assert_eq!(verdict(&[pass], &leaked, None, true), "FAIL", "a secret fails the run");
        assert_eq!(verdict(&[], &clean, None, true), "UNPROVEN", "nothing ran, nothing proved");
        assert_eq!(verdict(&[], &clean, Some(&reviewed), true), "PASS", "a clean second-AI review proves a change with no tests");
        assert_eq!(verdict(&[], &clean, None, false), "NO_CHECKS");

        let unmet = json!({ "verdict": "pass", "issues": [], "criteria": [{ "met": false }] });

        assert_eq!(verdict(&[json!({ "status": "pass" })], &clean, Some(&unmet), true), "FAIL", "an unmet agreed condition fails");
    }

    #[test]
    fn a_measured_check_decides_a_condition_the_reviewer_could_not_see() {
        let mut review = json!({
            "verdict": "issues",
            "issues": [{ "severity": "medium", "message": "no test in the diff" }],
            "criteria": [{ "met": true, "why": "ok" }, { "met": false, "why": "npm test cannot pass" }, { "met": false, "why": "no email" }],
        });
        let checks = vec![json!({ "name": "test", "command": "npm run test", "status": "pass" })];

        measured_criteria(&mut review, &["add works".into(), "npm test passes".into(), "an email is sent".into()], &checks);

        assert_eq!(review["criteria"][1]["met"], true);
        assert!(review["criteria"][1]["why"].as_str().unwrap().contains("Measured by SDC"));
        assert_eq!(review["criteria"][2]["met"], false, "a condition no check measures stays the reviewer's call");
        assert_eq!(review["verdict"], "issues", "one condition is still unmet");

        review["criteria"][2]["met"] = json!(true);
        measured_criteria(&mut review, &[], &checks);

        assert_eq!(review["verdict"], "pass", "medium observations do not block");
    }

    #[test]
    fn dependency_audits_are_read_from_each_tools_json() {
        let npm = r#"{"metadata":{"vulnerabilities":{"info":0,"low":1,"moderate":2,"high":1,"critical":0}}}"#;
        let pip = r#"{"dependencies":[{"name":"flask","vulns":[{"id":"PYSEC-1"}]},{"name":"x","vulns":[]}]}"#;

        assert_eq!(read_audit(npm).unwrap()["high"], 1);
        assert_eq!(read_audit(pip).unwrap()["high"], 1);
        assert!(read_audit("not json").is_none());
        assert_eq!(dependency_audit(&files(&["package-lock.json"]), true).unwrap().0, "npm audit");
        assert!(dependency_audit(&files(&["README.md"]), true).is_none());
    }

    #[test]
    fn the_reviewer_judges_each_agreed_condition() {
        let prompt = review_prompt_with("fix the form", "+x", &["The form sends an email".into()], Some("security"));

        assert!(prompt.contains("1. The form sends an email"));
        assert!(prompt.contains("SECURITY review"));
        assert!(with_facts(prompt.clone(), &["`npm test` PASSED".into()]).ends_with("- `npm test` PASSED"));
        assert_eq!(with_facts(prompt.clone(), &[]), prompt);

        let review = parse_review(r#"{"verdict":"pass","summary":"ok","issues":[],"criteria":[{"met":false,"why":"no email is sent"}]}"#);

        assert_eq!(review["verdict"], "issues", "an unmet condition turns a pass into issues");
        assert_eq!(review["criteria"][0]["why"], "no email is sent");
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
