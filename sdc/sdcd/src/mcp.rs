//! **SDC as an MCP server (Phase 4.9)**: `sdcd mcp` speaks the Model Context Protocol on stdin/stdout, so Claude Code, Cursor, Codex or any
//! MCP client can use SDC's safety layer from inside their own agent loop:
//!
//! | tool | what it does | model calls | changes anything? |
//! |---|---|---|---|
//! | `sdc_secret_scan` | where in a folder a password, key or token sits (never the value) | none | no |
//! | `sdc_policy_check` | what the folder's `.sdc/policy.toml` says about a read, a write or a command: allow, ask or deny | none | no |
//! | `sdc_verify` | the folder's own checks (tests, types, lint, build), in SDC's sandbox | none | no (checks may write caches) |
//! | `sdc_checkpoint` / `sdc_checkpoints` | a Time Machine checkpoint of the folder / the list | none | adds a checkpoint |
//! | `sdc_restore` | puts the folder back to a checkpoint - **needs `confirm: true`**, and takes a safety checkpoint first so it can be undone | none | yes |
//! | `sdc_code_graph` / `sdc_ranked_search` | where a name is declared and used / the code that answers a question - from the Code Graph and BM25, files the policy keeps from models left out | none | no |
//! | `sdc_memory_read` | the project's memory (stated and observed facts only) and the rules in CLAUDE.md / AGENTS.md, tagged `[file]` | none | no |
//! | `sdc_slopguard_check` | which packages an install command names do not exist in their registry | none (a registry lookup) | no |
//! | `sdc_trace_recent` / `sdc_relay_report` | SDC's own Step Trace rows / the Silent Relay report (need a running SDC) | none | no |
//! | `sdc_vps_run` | a command on one of SDC's connected servers, through its signed-in connection - read-only commands run; anything else needs `confirm: true`; destructive patterns are refused | none | only with confirm |
//!
//! Nothing here sends anything to a model. The tools that need a running SDC (only `sdc_vps_run`) say so when it is not. Protocol: JSON-RPC 2.0,
//! one message per line, version `2024-11-05`; the server answers `initialize`, `ping`, `tools/list` and `tools/call` and ignores notifications.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const PROTOCOL: &str = "2024-11-05";

fn tools() -> Value {
    let path = json!({ "type": "string", "description": "Absolute path of the project folder" });

    json!([
        { "name": "sdc_secret_scan", "description": "Find passwords, API keys and tokens already sitting in a project folder. Returns file, line and kind - never the value. Use before committing or deploying.", "inputSchema": { "type": "object", "properties": { "path": path }, "required": ["path"] } },
        { "name": "sdc_policy_check", "description": "Ask the project's .sdc/policy.toml whether an action is allowed: returns allow, ask (a person must approve) or deny, and why. Check before writing a file, reading a possibly secret file, or running a command.", "inputSchema": { "type": "object", "properties": { "path": path, "action": { "type": "string", "enum": ["read", "write", "run"] }, "target": { "type": "string", "description": "A file path (read, write) or a command line (run)" } }, "required": ["path", "action", "target"] } },
        { "name": "sdc_verify", "description": "Run the project's own checks (typecheck, lint, tests, build as its manifests define them) in SDC's sandbox and report pass or fail for each with the tail of the output.", "inputSchema": { "type": "object", "properties": { "path": path }, "required": ["path"] } },
        { "name": "sdc_checkpoint", "description": "Save a Time Machine checkpoint of the folder before a risky change. Returns its id.", "inputSchema": { "type": "object", "properties": { "path": path, "title": { "type": "string" } }, "required": ["path"] } },
        { "name": "sdc_checkpoints", "description": "List the folder's Time Machine checkpoints, newest first.", "inputSchema": { "type": "object", "properties": { "path": path }, "required": ["path"] } },
        { "name": "sdc_restore", "description": "Put the folder back exactly as a checkpoint recorded it. Destructive for changes made since, so it requires confirm=true; a safety checkpoint of the current state is taken first, so it can itself be undone.", "inputSchema": { "type": "object", "properties": { "path": path, "id": { "type": "string", "description": "A checkpoint id from sdc_checkpoints" }, "confirm": { "type": "boolean" } }, "required": ["path", "id"] } },
        { "name": "sdc_code_graph", "description": "Ask the project's Code Graph (no model): where a name is declared and who calls it (`name`), or the declarations and tests of one file (`file`, relative to the folder). Files the policy lists in secret_paths are never shown.", "inputSchema": { "type": "object", "properties": { "path": path, "name": { "type": "string" }, "file": { "type": "string" } }, "required": ["path"] } },
        { "name": "sdc_ranked_search", "description": "Find the code that answers a question, ranked from names, declarations and BM25, with no model: the best chunks as path:lines and a skim of the best one. Files the policy lists in secret_paths are never shown.", "inputSchema": { "type": "object", "properties": { "path": path, "query": { "type": "string" }, "k": { "type": "integer", "description": "How many chunks (1-20, default 6)" } }, "required": ["path", "query"] } },
        { "name": "sdc_memory_read", "description": "What the project remembers: the facts in .sdc/memory.md, lessons and recipes that a person stated or a tool observed (a model's guesses, marked [inferred], are left out), and the rules in CLAUDE.md and AGENTS.md tagged [file].", "inputSchema": { "type": "object", "properties": { "path": path }, "required": ["path"] } },
        { "name": "sdc_slopguard_check", "description": "Before running an install command (npm install, pip install, cargo add ...): look every package it names up in its registry and say which do not exist (a name a model made up). Needs the network; an unreachable registry is reported as unchecked, never as missing.", "inputSchema": { "type": "object", "properties": { "command": { "type": "string" } }, "required": ["command"] } },
        { "name": "sdc_trace_recent", "description": "The newest Step Trace rows of SDC's own model calls (model, tokens, time, cost, verify), oldest first. Needs SDC running.", "inputSchema": { "type": "object", "properties": { "limit": { "type": "integer", "description": "1-500, default 20" }, "sessionId": { "type": "string" } } } },
        { "name": "sdc_relay_report", "description": "How much of the work the local model did and how much went to the Rotation Chain (Silent Relay), with tokens, cost labels and overhead. Needs SDC running.", "inputSchema": { "type": "object", "properties": { "since": { "type": "string", "description": "A period like 7d or 24h, or an RFC 3339 time" }, "sessionId": { "type": "string" } } } },
        { "name": "sdc_vps_run", "description": "Run a command on one of the servers connected in SDC (needs SDC running). Read-only commands (uptime, df, free, systemctl status, docker ps, journalctl --no-pager ...) run at once; anything else needs confirm=true; destructive patterns (rm -rf /, mkfs, dd to a device, shutdown, fork bombs) are always refused.", "inputSchema": { "type": "object", "properties": { "host": { "type": "string", "description": "A host id or name from SDC" }, "command": { "type": "string" }, "confirm": { "type": "boolean" } }, "required": ["host", "command"] } }
    ])
}

/// A folder the tools may work on: it exists, is a directory, and is not the filesystem root or the home folder itself.
pub fn folder(arguments: &Value) -> Result<PathBuf, String> {
    let given = arguments["path"].as_str().filter(|path| !path.trim().is_empty()).ok_or("`path` is required")?;
    let real = Path::new(given).canonicalize().map_err(|_| format!("{given} is not a folder on this computer"))?;

    if !real.is_dir() {
        return Err(format!("{given} is not a folder"));
    }

    if real.parent().is_none() || dirs::home_dir().is_some_and(|home| home.canonicalize().ok().as_deref() == Some(real.as_path())) {
        return Err("refusing to work on the filesystem root or the whole home folder: give a project folder".into());
    }

    Ok(real)
}

/// Patterns no confirmation unlocks.
const DESTRUCTIVE: &[&str] = &["rm -rf /", "rm -rf /*", "rm -fr /", "mkfs", "dd if=", "dd of=/dev", "> /dev/sd", "> /dev/nvme", "shutdown", "poweroff", "reboot", "halt", ":(){", "chmod -r 777 /", "chown -r", "--no-preserve-root", "drop database", "drop table", "truncate table"];

/// Whether a command is refused outright.
pub fn destructive(command: &str) -> Option<&'static str> {
    let lowered = command.to_lowercase();

    DESTRUCTIVE.iter().copied().find(|pattern| lowered.contains(pattern))
}

fn secret_scan(arguments: &Value) -> Result<String, String> {
    let root = folder(arguments)?;
    let sweep = crate::trust::sweep::run(&root);

    Ok(serde_json::to_string_pretty(&crate::trust::sweep::to_json(&sweep)).unwrap_or_default())
}

fn policy_check(arguments: &Value) -> Result<String, String> {
    let root = folder(arguments)?;
    let policy = crate::trust::policy::Policy::load_local(&root);
    let base = root.to_string_lossy().to_string();
    let target = arguments["target"].as_str().ok_or("`target` is required")?;
    let (verdict, why) = match arguments["action"].as_str() {
        Some("read") => match policy.secret(target, Some(&base)) {
            Some(pattern) => ("deny", format!("{target} matches secret_paths `{pattern}`: its content must never be sent to a model")),
            None if crate::trust::secretguard::sensitive_name(target).is_some() => ("ask", format!("the file name of {target} says it holds a secret; a person should approve a read")),
            None => ("allow", "no policy rule covers this read".to_string()),
        },
        Some("write") => match policy.protected(target, Some(&base)) {
            Some(pattern) => ("ask", format!("{target} matches protected_paths `{pattern}`: a person must approve a change")),
            None => ("allow", "no policy rule covers this write".to_string()),
        },
        Some("run") => match (policy.denies(target), policy.always_asks(target)) {
            (Some(rule), _) => ("deny", format!("the policy denies commands matching `{rule}`")),
            (None, Some(rule)) => ("ask", format!("commands matching `{rule}` always wait for a person")),
            _ => ("allow", "no policy rule covers this command".to_string()),
        },
        _ => return Err("`action` must be read, write or run".into()),
    };

    Ok(json!({ "verdict": verdict, "why": why, "policy": policy.source, "policyError": policy.error }).to_string())
}

fn verify(arguments: &Value) -> Result<String, String> {
    let root = folder(arguments)?;
    let files: Vec<String> = std::fs::read_dir(&root).map_err(|error| error.to_string())?.flatten().map(|entry| entry.file_name().to_string_lossy().to_string()).collect();
    let checks = crate::verify::plan(&files, &|name| std::fs::read_to_string(root.join(name)).ok(), !cfg!(windows));

    if checks.is_empty() {
        return Ok(json!({ "pass": null, "note": "No checks were found: this folder has no package.json scripts, Cargo.toml, go.mod or Python tests SDC knows how to run." }).to_string());
    }

    let mut results = Vec::new();
    let mut pass = true;

    for check in &checks {
        let (line, note) = crate::verify::sandboxed_for_mcp(&check.command, &root.to_string_lossy());
        let outcome = run_line(&line, &root, std::time::Duration::from_secs(600));

        pass &= outcome.0;
        results.push(json!({ "name": check.name, "command": check.command, "pass": outcome.0, "tail": outcome.1, "note": note }));
    }

    Ok(json!({ "pass": pass, "checks": results }).to_string())
}

/// Runs a shell line in `root` with a deadline; answers success and the last lines of the output.
fn run_line(line: &str, root: &Path, limit: std::time::Duration) -> (bool, Vec<String>) {
    let mut command = if cfg!(windows) { let mut command = std::process::Command::new("cmd"); command.args(["/C", line]); command } else { let mut command = std::process::Command::new("sh"); command.args(["-c", line]); command };

    command.current_dir(root).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());

    let Ok(mut child) = command.spawn() else {
        return (false, vec!["the check could not be started".into()]);
    };
    let started = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();

                return (false, vec![format!("stopped after {} seconds", limit.as_secs())]);
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(error) => return (false, vec![error.to_string()]),
        }
    }

    match child.wait_with_output() {
        Ok(output) => {
            let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();

            (output.status.success(), lines.iter().rev().take(12).rev().map(|line| line.chars().take(240).collect()).collect())
        }
        Err(error) => (false, vec![error.to_string()]),
    }
}

fn checkpoint(arguments: &Value) -> Result<String, String> {
    let root = folder(arguments)?;
    let title = arguments["title"].as_str().filter(|title| !title.trim().is_empty()).unwrap_or("MCP checkpoint");
    let sha = crate::git::checkpoint(&root, &format!("mcp: {title}")).map_err(|error| error.message)?;

    Ok(json!({ "id": sha, "title": title }).to_string())
}

fn checkpoints(arguments: &Value) -> Result<String, String> {
    let root = folder(arguments)?;
    let shadow = crate::git::Shadow::of(&root).map_err(|error| error.message)?;
    let text = shadow.run(&["log", "--format=%H\t%cI\t%s", "-n", "30"]).map_err(|error| error.message)?;
    let rows: Vec<Value> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');

            Some(json!({ "id": parts.next()?, "at": parts.next()?, "title": parts.next()? }))
        })
        .collect();

    Ok(json!({ "checkpoints": rows }).to_string())
}

fn restore(arguments: &Value) -> Result<String, String> {
    let root = folder(arguments)?;
    let id = arguments["id"].as_str().unwrap_or_default();

    if id.len() < 7 || id.len() > 40 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("`id` must be a checkpoint id from sdc_checkpoints".into());
    }

    if arguments["confirm"] != true {
        return Err("This puts the folder back to the checkpoint and removes later changes. Ask the person, then call again with confirm=true.".into());
    }

    let safety = crate::git::checkpoint(&root, "mcp: before restore").map_err(|error| error.message)?;

    crate::git::restore(&root, id).map_err(|error| error.message)?;

    Ok(json!({ "restoredTo": id, "undoWith": safety, "note": "the state before this restore was saved as the checkpoint in `undoWith`" }).to_string())
}

fn vps_run(arguments: &Value, daemon: &mut dyn FnMut(&str, Value) -> Result<Value, String>) -> Result<String, String> {
    let host = arguments["host"].as_str().filter(|host| !host.trim().is_empty()).ok_or("`host` is required")?;
    let command = arguments["command"].as_str().filter(|command| !command.trim().is_empty()).ok_or("`command` is required")?;

    if let Some(pattern) = destructive(command) {
        return Err(format!("refused: the command contains `{pattern}`, which SDC never runs on a server from an AI tool, even with confirmation"));
    }

    if !crate::v3::fleet::read_only(command) && arguments["confirm"] != true {
        return Err("This command may change the server. Ask the person, then call again with confirm=true. (Read-only commands such as uptime, df, free, systemctl status and docker ps run without it.)".into());
    }

    let hosts = daemon("session.list", json!({}))?;
    let known = hosts["hosts"].as_array().cloned().unwrap_or_default();
    let found = known.iter().find(|candidate| candidate["hostId"] == host || candidate["name"] == host);
    let host_id = found.and_then(|candidate| candidate["hostId"].as_str()).ok_or_else(|| format!("no connected host called `{host}` in SDC"))?;
    let answer = daemon("shell.run", json!({ "hostId": host_id, "line": command, "timeoutMs": 60_000 }))?;

    Ok(answer.to_string())
}

/// Hides what the folder's policy keeps from models: a `secret_paths` match, or a file name that says it holds a secret.
fn withheld(policy: &crate::trust::policy::Policy, root: &Path, relative: &str) -> bool {
    policy.secret(relative, Some(&root.to_string_lossy())).is_some() || crate::trust::secretguard::sensitive_name(relative).is_some()
}

fn code_graph(arguments: &Value) -> Result<String, String> {
    let root = folder(arguments)?;
    let policy = crate::trust::policy::Policy::load_local(&root);
    let (index, _) = crate::graph::index::refreshed(&root);
    let name = arguments["name"].as_str().map(str::trim).filter(|name| !name.is_empty());
    let file = arguments["file"].as_str().map(str::trim).filter(|file| !file.is_empty());
    let def_json = |path: &str, def: &crate::graph::parse::Def| json!({ "file": path, "kind": def.kind, "name": def.qualified, "startLine": def.start_line, "endLine": def.end_line });

    match (name, file) {
        (Some(name), _) => {
            let declared: Vec<Value> = index.where_is(name).iter().filter(|(path, _)| !withheld(&policy, &root, path)).map(|(path, def)| def_json(path, def)).collect();
            let callers: Vec<Value> = index.callers(name).iter().filter(|(path, _, _)| !withheld(&policy, &root, path)).take(200).map(|(path, line, call)| json!({ "file": path, "line": line, "call": call })).collect();

            Ok(json!({ "name": name, "declared": declared, "usedAt": callers }).to_string())
        }
        (None, Some(file)) => {
            if withheld(&policy, &root, file) {
                return Err(format!("{file} is kept from models by the project's policy"));
            }

            let symbols: Vec<Value> = index.symbols_of(file).iter().map(|def| def_json(file, def)).collect();
            let tests: Vec<String> = index.tests_for(file).into_iter().filter(|path| !withheld(&policy, &root, path)).collect();

            Ok(json!({ "file": file, "symbols": symbols, "tests": tests }).to_string())
        }
        (None, None) => Err("give `name` (a function, type or method) or `file`".into()),
    }
}

fn ranked_search(arguments: &Value) -> Result<String, String> {
    let root = folder(arguments)?;
    let query = arguments["query"].as_str().map(str::trim).filter(|query| !query.is_empty()).ok_or("`query` is required")?;
    let k = arguments["k"].as_u64().unwrap_or(6).clamp(1, 20) as usize;
    let policy = crate::trust::policy::Policy::load_local(&root);
    let hits: Vec<_> = crate::sme::retrieval::retrieve(&root, query, k + 10).into_iter().filter(|hit| !withheld(&policy, &root, &hit.path)).take(k).collect();

    if hits.is_empty() {
        return Ok(format!("Nothing in this project matches `{query}`."));
    }

    Ok(crate::sme::retrieval::skim(&hits, 24))
}

/// `.sdc/memory.md`, lessons and recipes minus what a model only guessed, then CLAUDE.md and AGENTS.md as `[file]` rules.
fn memory_read(arguments: &Value) -> Result<String, String> {
    use crate::v3::memory_v2::{entries, parse, Origin};

    let root = folder(arguments)?;
    let mut lines: Vec<String> = Vec::new();

    for entry in entries(&root) {
        let Ok(text) = std::fs::read_to_string(root.join(&entry.path)) else {
            continue;
        };

        for fact in parse(&text).into_iter().filter(|fact| fact.origin != Origin::Inferred) {
            lines.push(format!("{} {} ({})", fact.origin.tag(), fact.text, entry.path));
        }
    }

    for name in ["CLAUDE.md", "AGENTS.md"] {
        let Ok(text) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };

        for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).take(200) {
            lines.push(format!("[file] {} ({name})", line.trim_start_matches(['-', '*', ' '])));
        }
    }

    Ok(if lines.is_empty() { "This project remembers nothing yet.".to_string() } else { lines.join("\n") })
}

fn slopguard_check(arguments: &Value) -> Result<String, String> {
    let command = arguments["command"].as_str().map(str::trim).filter(|command| !command.is_empty()).ok_or("`command` is required")?;
    let verdict = crate::trust::slopguard::check(command);
    let names = |packages: &[crate::trust::slopguard::Package]| packages.iter().map(|package| package.name.clone()).collect::<Vec<_>>();

    Ok(json!({ "refused": verdict.refusal().is_some(), "missing": names(&verdict.missing), "unchecked": names(&verdict.unchecked), "message": verdict.refusal().or_else(|| verdict.note()) }).to_string())
}

fn trace_recent(arguments: &Value, daemon: &mut dyn FnMut(&str, Value) -> Result<Value, String>) -> Result<String, String> {
    let limit = arguments["limit"].as_i64().unwrap_or(20).clamp(1, 500);
    let mut params = json!({ "limit": limit });

    if let Some(session) = arguments["sessionId"].as_str() {
        params["sessionId"] = json!(session);
    }

    Ok(daemon("trace.list", params)?.to_string())
}

fn relay_report(arguments: &Value, daemon: &mut dyn FnMut(&str, Value) -> Result<Value, String>) -> Result<String, String> {
    let mut params = json!({});

    for key in ["since", "sessionId"] {
        if let Some(value) = arguments[key].as_str() {
            params[key] = json!(value);
        }
    }

    Ok(daemon("relay.report", params)?.to_string())
}

/// Calls one tool. `daemon` reaches a running SDC for the tools that need it.
pub fn call(name: &str, arguments: &Value, daemon: &mut dyn FnMut(&str, Value) -> Result<Value, String>) -> Result<String, String> {
    match name {
        "sdc_secret_scan" => secret_scan(arguments),
        "sdc_policy_check" => policy_check(arguments),
        "sdc_verify" => verify(arguments),
        "sdc_checkpoint" => checkpoint(arguments),
        "sdc_checkpoints" => checkpoints(arguments),
        "sdc_restore" => restore(arguments),
        "sdc_vps_run" => vps_run(arguments, daemon),
        "sdc_code_graph" => code_graph(arguments),
        "sdc_ranked_search" => ranked_search(arguments),
        "sdc_memory_read" => memory_read(arguments),
        "sdc_slopguard_check" => slopguard_check(arguments),
        "sdc_trace_recent" => trace_recent(arguments, daemon),
        "sdc_relay_report" => relay_report(arguments, daemon),
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// One JSON-RPC message in, the reply out (`None` for a notification).
pub fn handle(message: &Value, daemon: &mut dyn FnMut(&str, Value) -> Result<Value, String>) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message["method"].as_str().unwrap_or_default();
    let reply = |result: Value| id.clone().map(|id| json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    let failure = |code: i64, text: String| id.clone().map(|id| json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": text } }));

    match method {
        "initialize" => reply(json!({
            "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "sdc", "version": crate::VERSION },
            "instructions": "SDC's safety layer: scan for secrets, check a policy, verify a project, checkpoint and restore with the Time Machine, search the code graph and memory, check install commands for made-up packages, read SDC's trace and relay report, and run safe commands on connected servers. Nothing here calls an AI model.",
        })),
        "ping" => reply(json!({})),
        "tools/list" => reply(json!({ "tools": tools() })),
        "tools/call" => {
            let name = message["params"]["name"].as_str().unwrap_or_default();
            let arguments = message["params"]["arguments"].clone();

            match call(name, &arguments, daemon) {
                Ok(text) => reply(json!({ "content": [{ "type": "text", "text": text }], "isError": false })),
                Err(text) => reply(json!({ "content": [{ "type": "text", "text": text }], "isError": true })),
            }
        }
        m if m.starts_with("notifications/") => None,
        "" => None,
        other => failure(-32601, format!("method not found: {other}")),
    }
}

/// `sdcd mcp [--port N]`: reads requests from stdin until it closes.
pub fn main(args: &[String]) -> i32 {
    let port = args.iter().position(|arg| arg == "--port").and_then(|at| args.get(at + 1)).and_then(|value| value.parse().ok()).unwrap_or(crate::paths::DEFAULT_PORT);
    let mut client: Option<crate::cli::Client> = None;
    let mut daemon = move |method: &str, params: Value| -> Result<Value, String> {
        if client.is_none() {
            client = Some(crate::cli::Client::connect(port)?);
        }

        client.as_mut().expect("connected").call(method, params)
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };

        if line.trim().is_empty() {
            continue;
        }

        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&message, &mut daemon),
            Err(error) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {error}") } })),
        };

        if let Some(reply) = reply {
            if writeln!(stdout, "{reply}").and_then(|_| stdout.flush()).is_err() {
                break;
            }
        }
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_daemon(_: &str, _: Value) -> Result<Value, String> {
        Err("SDC is not running".into())
    }

    fn folder_with(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-mcp-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn text_of(reply: &Value) -> String {
        reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn it_introduces_itself_lists_thirteen_tools_and_ignores_notifications() {
        let mut daemon = no_daemon;
        let init = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }), &mut daemon).unwrap();

        assert_eq!(init["result"]["serverInfo"]["name"], "sdc");
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert!(handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), &mut daemon).is_none());

        let listed = handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }), &mut daemon).unwrap();
        let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().filter_map(|tool| tool["name"].as_str()).collect();

        assert_eq!(names, ["sdc_secret_scan", "sdc_policy_check", "sdc_verify", "sdc_checkpoint", "sdc_checkpoints", "sdc_restore", "sdc_code_graph", "sdc_ranked_search", "sdc_memory_read", "sdc_slopguard_check", "sdc_trace_recent", "sdc_relay_report", "sdc_vps_run"]);
        assert!(listed["result"]["tools"].as_array().unwrap().iter().all(|tool| tool["inputSchema"]["type"] == "object"));
        assert_eq!(handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" }), &mut daemon).unwrap()["error"]["code"], -32601);
        assert_eq!(handle(&json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" }), &mut daemon).unwrap()["result"], json!({}));
    }

    #[test]
    fn the_secret_scan_reports_where_and_never_the_value() {
        let root = folder_with("scan");

        std::fs::write(root.join("config.txt"), "api_key = \"FAKEvalue9Zq4Xw7Lm2Pt8\"\n").unwrap();

        let mut daemon = no_daemon;
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "sdc_secret_scan", "arguments": { "path": root.to_string_lossy() } } }), &mut daemon).unwrap();
        let text = text_of(&reply);

        assert_eq!(reply["result"]["isError"], false);
        assert!(text.contains("config.txt") && !text.contains("FAKEvalue"), "{text}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_policy_check_says_allow_ask_or_deny_and_why() {
        let root = folder_with("policy");

        std::fs::create_dir_all(root.join(".sdc")).unwrap();
        std::fs::write(root.join(".sdc/policy.toml"), "secret_paths = [\"vault/**\"]\nprotected_paths = [\"deploy/**\"]\ndeny_commands = [\"drop database\"]\nalways_ask = [\"git push\"]\n").unwrap();

        let ask = |action: &str, target: &str| -> Value {
            let mut daemon = no_daemon;
            let reply = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "sdc_policy_check", "arguments": { "path": root.to_string_lossy(), "action": action, "target": target } } }), &mut daemon).unwrap();

            serde_json::from_str(&text_of(&reply)).unwrap()
        };

        assert_eq!(ask("read", "vault/keys.txt")["verdict"], "deny");
        assert_eq!(ask("read", "src/app.rs")["verdict"], "allow");
        assert_eq!(ask("read", "pass.txt")["verdict"], "ask", "a file whose name says secret asks first");
        assert_eq!(ask("write", "deploy/run.sh")["verdict"], "ask");
        assert_eq!(ask("write", "src/app.rs")["verdict"], "allow");
        assert_eq!(ask("run", "mysql -e 'DROP DATABASE shop'")["verdict"], "deny");
        assert_eq!(ask("run", "git push origin main")["verdict"], "ask");
        assert_eq!(ask("run", "ls")["verdict"], "allow");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_restore_needs_a_yes_takes_a_safety_checkpoint_and_can_be_undone() {
        if !crate::host::tools::has_working_git() {
            eprintln!("skipped: no git");

            return;
        }

        let root = folder_with("restore");

        std::fs::write(root.join("a.txt"), "one").unwrap();

        let mut daemon = no_daemon;
        let made = call("sdc_checkpoint", &json!({ "path": root.to_string_lossy(), "title": "start" }), &mut daemon).unwrap();
        let id = serde_json::from_str::<Value>(&made).unwrap()["id"].as_str().unwrap().to_string();

        std::fs::write(root.join("a.txt"), "two").unwrap();
        std::fs::write(root.join("b.txt"), "new").unwrap();

        let refused = call("sdc_restore", &json!({ "path": root.to_string_lossy(), "id": id }), &mut daemon).unwrap_err();

        assert!(refused.contains("confirm=true"));
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two", "nothing changed without the yes");

        let restored = call("sdc_restore", &json!({ "path": root.to_string_lossy(), "id": id, "confirm": true }), &mut daemon).unwrap();
        let undo = serde_json::from_str::<Value>(&restored).unwrap()["undoWith"].as_str().unwrap().to_string();

        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one");
        assert!(!root.join("b.txt").exists());

        call("sdc_restore", &json!({ "path": root.to_string_lossy(), "id": undo, "confirm": true }), &mut daemon).unwrap();

        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two", "the restore itself was undone");

        let listed = call("sdc_checkpoints", &json!({ "path": root.to_string_lossy() }), &mut daemon).unwrap();

        assert!(listed.contains("start") && listed.contains("before restore"));
        assert!(call("sdc_restore", &json!({ "path": root.to_string_lossy(), "id": "not-hex!", "confirm": true }), &mut daemon).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_server_command_is_read_only_or_confirmed_and_destructive_ones_are_never_run() {
        let mut calls: Vec<(String, Value)> = Vec::new();
        let mut daemon = |method: &str, params: Value| -> Result<Value, String> {
            calls.push((method.to_string(), params));

            Ok(if method == "session.list" { json!({ "hosts": [{ "hostId": "h-1", "name": "web-1" }] }) } else { json!({ "exitCode": 0, "stdout": "ok" }) })
        };

        assert!(call("sdc_vps_run", &json!({ "host": "web-1", "command": "uptime" }), &mut daemon).is_ok());
        assert!(call("sdc_vps_run", &json!({ "host": "web-1", "command": "systemctl restart nginx" }), &mut daemon).unwrap_err().contains("confirm=true"));
        assert!(call("sdc_vps_run", &json!({ "host": "web-1", "command": "systemctl restart nginx", "confirm": true }), &mut daemon).is_ok());

        for bad in ["rm -rf /", "sudo mkfs.ext4 /dev/sda1", "dd if=/dev/zero of=/dev/sda", "shutdown -h now", "mysql -e 'DROP DATABASE shop'"] {
            assert!(call("sdc_vps_run", &json!({ "host": "web-1", "command": bad, "confirm": true }), &mut daemon).unwrap_err().contains("refused"), "{bad}");
        }

        assert!(call("sdc_vps_run", &json!({ "host": "nobody", "command": "uptime" }), &mut daemon).unwrap_err().contains("no connected host"), "a host SDC does not know is not guessed");

        assert_eq!(calls.iter().filter(|(method, params)| method == "shell.run" && params["hostId"] == "h-1").count(), 2, "the name was resolved to the id; refused commands never reached the server");
        assert!(call("sdc_vps_run", &json!({ "host": "web-1", "command": "uptime" }), &mut no_daemon).unwrap_err().contains("not running"));
    }

    #[test]
    fn the_filesystem_root_and_the_home_folder_are_not_projects() {
        assert!(folder(&json!({ "path": "/" })).is_err());
        assert!(folder(&json!({ "path": dirs::home_dir().unwrap().to_string_lossy() })).is_err());
        assert!(folder(&json!({ "path": "/no/such/folder/anywhere" })).is_err());
        assert!(folder(&json!({})).is_err());
    }

    /* the folder name is the test's own: two tests that shared one name would delete each other's files when they ran at the same time */
    fn project(name: &str) -> PathBuf {
        let root = folder_with(name);

        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".sdc")).unwrap();
        std::fs::write(root.join("src/ledger.rs"), "pub fn compute_total(items: &[u64]) -> u64 {\n    items.iter().sum()\n}\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {\n    let total = compute_total(&[1, 2]);\n    println!(\"{total}\");\n}\n").unwrap();
        std::fs::write(root.join("src/vault.rs"), "pub fn compute_total_secret() -> u64 { 42 }\n").unwrap();
        std::fs::write(root.join(".sdc/policy.toml"), "secret_paths = [\"src/vault.rs\"]\n").unwrap();
        root
    }

    #[test]
    fn the_code_graph_answers_where_and_who_and_hides_what_the_policy_keeps() {
        let root = project("read-tools-code-graph");
        let mut daemon = no_daemon;
        let named = call("sdc_code_graph", &json!({ "path": root.to_string_lossy(), "name": "compute_total" }), &mut daemon).unwrap();

        assert!(named.contains("src/ledger.rs") && named.contains("src/main.rs"), "{named}");

        let secret = call("sdc_code_graph", &json!({ "path": root.to_string_lossy(), "name": "compute_total_secret" }), &mut daemon).unwrap();

        assert!(!secret.contains("vault.rs"), "a secret_paths file is not shown: {secret}");
        assert!(call("sdc_code_graph", &json!({ "path": root.to_string_lossy(), "file": "src/vault.rs" }), &mut daemon).is_err());
        assert!(call("sdc_code_graph", &json!({ "path": root.to_string_lossy() }), &mut daemon).is_err(), "a name or a file is needed");
    }

    #[test]
    fn the_ranked_search_finds_code_and_never_returns_a_withheld_file() {
        let root = project("read-tools-ranked-search");
        let mut daemon = no_daemon;
        let found = call("sdc_ranked_search", &json!({ "path": root.to_string_lossy(), "query": "compute total of items" }), &mut daemon).unwrap();

        assert!(found.contains("src/ledger.rs"), "{found}");

        let secret = call("sdc_ranked_search", &json!({ "path": root.to_string_lossy(), "query": "compute_total_secret" }), &mut daemon).unwrap();

        assert!(!secret.contains("vault.rs"), "{secret}");
        assert!(call("sdc_ranked_search", &json!({ "path": root.to_string_lossy(), "query": " " }), &mut daemon).is_err());
    }

    #[test]
    fn memory_read_leaves_out_guesses_and_tags_the_rules_files() {
        let root = folder_with("memory-read");
        let mut daemon = no_daemon;

        std::fs::create_dir_all(root.join(".sdc")).unwrap();
        std::fs::write(root.join(".sdc/memory.md"), "- [stated] Deploys go through make ship.\n- [inferred] The team may prefer tabs.\n- [observed] Tests run with pnpm test.\n").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "# Rules\n- Never edit generated files.\n").unwrap();

        let text = call("sdc_memory_read", &json!({ "path": root.to_string_lossy() }), &mut daemon).unwrap();

        assert!(text.contains("[stated] Deploys go through make ship."), "{text}");
        assert!(text.contains("[observed] Tests run with pnpm test."), "{text}");
        assert!(!text.contains("prefer tabs"), "an [inferred] guess is not handed on: {text}");
        assert!(text.contains("[file] Never edit generated files."), "{text}");
        assert!(!text.contains("# Rules"), "headings are not rules");
        assert_eq!(call("sdc_memory_read", &json!({ "path": folder_with("memory-empty").to_string_lossy() }), &mut daemon).unwrap(), "This project remembers nothing yet.");
    }

    #[test]
    fn trace_and_relay_tools_ask_the_running_daemon_and_say_so_when_it_is_not() {
        let mut asked: Vec<(String, Value)> = Vec::new();
        let mut daemon = |method: &str, params: Value| -> Result<Value, String> {
            asked.push((method.to_string(), params));

            Ok(json!({ "ok": true }))
        };

        call("sdc_trace_recent", &json!({ "limit": 9999, "sessionId": "s1" }), &mut daemon).unwrap();
        call("sdc_relay_report", &json!({ "since": "7d" }), &mut daemon).unwrap();

        assert_eq!(asked[0].0, "trace.list");
        assert_eq!(asked[0].1["limit"], 500, "the limit is clamped");
        assert_eq!(asked[0].1["sessionId"], "s1");
        assert_eq!(asked[1], ("relay.report".to_string(), json!({ "since": "7d" })));
        assert!(call("sdc_trace_recent", &json!({}), &mut no_daemon).unwrap_err().contains("not running"));
        assert!(call("sdc_relay_report", &json!({}), &mut no_daemon).unwrap_err().contains("not running"));
    }

    #[test]
    fn the_slopguard_tool_needs_a_command_and_reports_the_shape() {
        let mut daemon = no_daemon;

        assert!(call("sdc_slopguard_check", &json!({}), &mut daemon).is_err());

        let plain = call("sdc_slopguard_check", &json!({ "command": "ls -la" }), &mut daemon).unwrap();

        assert!(plain.contains("\"refused\":false") && plain.contains("\"missing\":[]"), "{plain}");
    }
}
