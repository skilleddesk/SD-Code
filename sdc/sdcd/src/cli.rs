//! **Headless `sdcd`** (1.0 in the plan): the same turns, checks and kill switch from a terminal or a CI
//! job, through the daemon that is already running - so a headless turn gets the Trust Kernel exactly
//! like a turn typed in the window: the checkpoint, the policy, the cost governor, the ledger.
//!
//! ```text
//! sdcd run --root ./shop --engine native_api --model deepseek-chat --provider deepseek --agent \
//!          --autonomy pro "fix the contact form"
//! sdcd kill                 # the kill switch
//! sdcd audit                # verify the ledger's chain
//! sdcd verify --session n12 # run Verify on a chat's newest turn
//! ```
//!
//! Exit codes: `0` the turn passed, `1` it failed or was stopped, `2` the daemon could not be reached,
//! `3` the command line was wrong. A permission the turn asks for is **declined** - nobody is there to
//! read it - and the decline is printed, so a CI log shows exactly what the turn wanted to do.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{json, Value};

struct Client {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    next: u64,
    /// Notifications that arrived while waiting for a response, kept for the stream reader.
    backlog: Vec<Value>,
}

impl Client {
    fn connect(port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|error| {
            format!("No SDC daemon is answering on 127.0.0.1:{port} ({error}). Open SDC, or start `sdcd` in another terminal.")
        })?;
        let reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);

        Ok(Self { writer: stream, reader, next: 1, backlog: Vec::new() })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = format!("cli-{}", self.next);

        self.next += 1;

        let line = json!({ "v": crate::SDCP_VERSION, "id": id, "method": method, "params": params }).to_string();

        self.writer.write_all(format!("{line}\n").as_bytes()).map_err(|error| error.to_string())?;

        loop {
            let message = self.read()?;

            if message["id"] == id.as_str() {
                return match message.get("error").filter(|error| !error.is_null()) {
                    Some(error) => Err(error["message"].as_str().unwrap_or("the daemon refused").to_string()),
                    None => Ok(message["result"].clone()),
                };
            }

            self.backlog.push(message);
        }
    }

    fn read(&mut self) -> Result<Value, String> {
        let mut line = String::new();

        loop {
            line.clear();

            if self.reader.read_line(&mut line).map_err(|error| error.to_string())? == 0 {
                return Err("the daemon closed the connection".into());
            }

            if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                return Ok(value);
            }
        }
    }

    fn notification(&mut self) -> Result<Value, String> {
        if !self.backlog.is_empty() {
            return Ok(self.backlog.remove(0));
        }

        self.read()
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|arg| arg == name).and_then(|index| args.get(index + 1)).cloned()
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

/// The words that are not flags or flag values: the prompt.
fn prompt_of(args: &[String]) -> String {
    let valued = ["--root", "--engine", "--model", "--provider", "--session", "--autonomy", "--host", "--port", "--intent", "--max-steps", "--effort"];
    let mut words = Vec::new();
    let mut skip = false;

    for arg in args {
        if skip {
            skip = false;
            continue;
        }

        if valued.contains(&arg.as_str()) {
            skip = true;
            continue;
        }

        if arg.starts_with("--") {
            continue;
        }

        words.push(arg.clone());
    }

    words.join(" ")
}

/// `sdcd <command> …` - returns the process's exit code.
pub fn main(command: &str, args: &[String], port: u16) -> i32 {
    let mut client = match Client::connect(port) {
        Ok(client) => client,
        Err(sentence) => {
            eprintln!("{sentence}");

            return 2;
        }
    };

    let _ = client.writer.set_read_timeout(Some(Duration::from_secs(3600)));

    match command {
        "kill" => match client.call("kill.all", json!({})) {
            Ok(result) => {
                println!("Stopped {} running task(s); {} checkpoint(s) keep the state they stopped in.", result["stopped"].as_array().map(Vec::len).unwrap_or(0), result["checkpoints"].as_array().map(Vec::len).unwrap_or(0));
                0
            }
            Err(error) => {
                eprintln!("{error}");
                1
            }
        },
        "audit" => match client.call("audit.verify", json!({})) {
            Ok(result) if result["intact"] == true => {
                println!("The audit ledger is intact: {} entries, head {}.", result["entries"], result["head"].as_str().unwrap_or(""));
                0
            }
            Ok(result) => {
                println!("The audit ledger is BROKEN at row {}: {}", result["brokenAt"], result["reason"].as_str().unwrap_or(""));
                1
            }
            Err(error) => {
                eprintln!("{error}");
                1
            }
        },
        "verify" => verify(&mut client, args),
        "run" => run(&mut client, args),
        _ => {
            eprintln!("unknown command `{command}`: run, verify, kill, audit");
            3
        }
    }
}

fn run(client: &mut Client, args: &[String]) -> i32 {
    let prompt = prompt_of(args);

    if prompt.trim().is_empty() {
        eprintln!("usage: sdcd run [--root DIR] [--host ID] [--engine E --model M --provider P] [--agent] [--autonomy ask|pro|auto] [--effort low|medium|high|max] [--json] \"the request\"");

        return 3;
    }

    let json_mode = has(args, "--json");
    let host = flag(args, "--host").unwrap_or_else(|| "local".into());
    let session = match flag(args, "--session") {
        Some(session) => session,
        None => {
            let project = match flag(args, "--root") {
                Some(root) => {
                    let root = std::fs::canonicalize(&root).map(|path| path.display().to_string().trim_start_matches(r"\\?\").to_string()).unwrap_or(root);

                    match client.call("project.add", json!({ "root": root, "hostId": host })) {
                        Ok(result) => result["projectId"].as_str().or(result["project"]["id"].as_str()).map(str::to_string),
                        Err(error) => {
                            eprintln!("{error}");

                            return 1;
                        }
                    }
                }
                None => None,
            };

            match client.call("session.open", json!({ "hostId": host, "title": prompt.chars().take(60).collect::<String>(), "projectId": project })) {
                Ok(result) => result["sessionId"].as_str().unwrap_or_default().to_string(),
                Err(error) => {
                    eprintln!("{error}");

                    return 1;
                }
            }
        }
    };
    let mut params = json!({
        "sessionId": session,
        "prompt": prompt,
        "engine": flag(args, "--engine").unwrap_or_else(|| "claude_code".into()),
        "agent": has(args, "--agent"),
        "autonomy": flag(args, "--autonomy").unwrap_or_else(|| "pro".into()),
    });

    for (key, name) in [("model", "--model"), ("provider", "--provider"), ("intentId", "--intent"), ("effort", "--effort")] {
        if let Some(value) = flag(args, name) {
            params[key] = json!(value);
        }
    }

    if let Some(steps) = flag(args, "--max-steps").and_then(|steps| steps.parse::<i64>().ok()) {
        params["maxSteps"] = json!(steps);
    }

    let turn = match client.call("engine.start", params) {
        Ok(result) => result["turnId"].as_str().unwrap_or_default().to_string(),
        Err(error) => {
            eprintln!("{error}");

            return 1;
        }
    };

    eprintln!("sdcd: turn {turn} in chat {session}");

    let mut passed = true;

    loop {
        let message = match client.notification() {
            Ok(message) => message,
            Err(error) => {
                eprintln!("sdcd: {error}");

                return 1;
            }
        };
        let event = &message["event"];
        let mine = message["turnId"] == turn.as_str() || event["turnId"] == turn.as_str();

        if !mine {
            continue;
        }

        if json_mode {
            println!("{event}");
        }

        match event["type"].as_str().unwrap_or_default() {
            "TurnDelta" if !json_mode => {
                print!("{}", event["delta"].as_str().unwrap_or_default());
                let _ = std::io::stdout().flush();
            }
            "ToolCallStarted" if !json_mode => eprintln!("\n▸ {} {}", event["name"].as_str().unwrap_or_default(), event["target"].as_str().unwrap_or_default()),
            "PermissionRequested" => {
                eprintln!("\n⚠ declined (headless): {} {} - {}", event["title"].as_str().unwrap_or_default(), event["target"].as_str().unwrap_or_default(), event["risk"].as_str().unwrap_or_default());

                let _ = client.call("permission.resolve", json!({ "permissionId": event["permissionId"], "decision": "deny" }));
            }
            "ErrorRaised" => {
                eprintln!("\n✖ {}: {}", event["title"].as_str().unwrap_or_default(), event["explanation"].as_str().unwrap_or_default());
                passed = false;
            }
            "PolicyViolation" | "BudgetStop" => {
                eprintln!("\n✖ {}", event["sentence"].as_str().unwrap_or_default());
                passed = false;
            }
            "CostUpdated" if !json_mode => eprintln!(
                "\nsdcd: ${:.4} ({}) · {} in / {} out",
                event["costUsd"].as_f64().unwrap_or(0.0),
                event["costSource"].as_str().unwrap_or(""),
                event["inputTokens"],
                event["outputTokens"]
            ),
            "TrustScored" => {
                if !json_mode {
                    eprintln!("sdcd: trust score {} ({})", event["score"], event["level"].as_str().unwrap_or(""));
                }

                /* The score is the last thing a turn says. */
                return if passed { 0 } else { 1 };
            }
            "TurnCompleted" if event["pass"] == false => passed = false,
            _ => {}
        }
    }
}

fn verify(client: &mut Client, args: &[String]) -> i32 {
    let Some(session) = flag(args, "--session") else {
        eprintln!("usage: sdcd verify --session <chat id>");

        return 3;
    };
    let verify_id = match client.call("verify.run", json!({ "sessionId": session })) {
        Ok(result) => result["verifyId"].as_str().unwrap_or_default().to_string(),
        Err(error) => {
            eprintln!("{error}");

            return 1;
        }
    };

    loop {
        let Ok(message) = client.notification() else {
            return 1;
        };
        let event = &message["event"];

        if event["type"] == "VerifyUpdated" && event["verifyId"] == verify_id.as_str() && event["state"] == "done" {
            for check in event["checks"].as_array().cloned().unwrap_or_default() {
                println!("{:>5}  {}", check["status"].as_str().unwrap_or(""), check["command"].as_str().unwrap_or(""));
            }

            println!("verdict: {}", event["verdict"].as_str().unwrap_or(if event["pass"] == true { "PASS" } else { "FAIL" }));

            return if event["pass"] == true { 0 } else { 1 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_is_every_word_that_is_not_a_flag() {
        let args: Vec<String> = ["--root", "./shop", "--agent", "fix", "the", "form", "--engine", "codex"].iter().map(|arg| arg.to_string()).collect();

        assert_eq!(prompt_of(&args), "fix the form");
        assert_eq!(flag(&args, "--engine").as_deref(), Some("codex"));
        assert!(has(&args, "--agent"));
    }

    #[test]
    fn no_daemon_is_exit_code_two_with_a_sentence() {
        assert_eq!(main("kill", &[], 1), 2);
    }
}
