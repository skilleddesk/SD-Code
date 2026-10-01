//! **MCP in the SDC Agent** (1.x in the plan): a project's own Model Context Protocol servers become the
//! agent's tools, next to its eight.
//!
//! A project lists its servers in `.sdc/mcp.json` - the same shape the MCP clients use:
//!
//! ```json
//! { "mcpServers": { "db": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-postgres", "postgres://localhost/shop"] } } }
//! ```
//!
//! Each server is started for the turn over stdio (newline-delimited JSON-RPC 2.0), asked for its tools,
//! and stopped when the turn ends. Its tools reach the model as `mcp__<server>__<tool>`, and every call
//! goes through the same gate as a command: an MCP tool can do anything its server can, so it is a `run`
//! for the permission rules, with the card, the checkpoint and the ledger row that come with one.
//!
//! A server is a program on the machine the project is on. For a folder on a VPS (0.13) that machine is the
//! VPS: the server is started there through `ssh` - the same connection the agent's commands use - and its
//! stdio travels back over it, so the protocol is unchanged and the server sees the host's files, database
//! and network, which is what a project's server is for.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use super::dialect::ToolSpec;

pub const CONFIG: &str = ".sdc/mcp.json";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Tool names and descriptions live for the whole process (a `ToolSpec` holds `&'static str`); each
/// distinct string is kept once, so the set is bounded by the tools that exist, not by the turns.
fn intern(text: String) -> &'static str {
    static STRINGS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

    let mut strings = STRINGS.get_or_init(|| Mutex::new(HashSet::new())).lock().unwrap_or_else(|poison| poison.into_inner());

    if let Some(existing) = strings.get(text.as_str()) {
        return existing;
    }

    let leaked: &'static str = Box::leak(text.into_boxed_str());

    strings.insert(leaked);
    leaked
}

struct Server {
    name: String,
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next: u64,
}

/// A server's arguments, as strings.
fn args_of(spec: &Value) -> Vec<String> {
    spec["args"].as_array().map(|args| args.iter().filter_map(|arg| arg.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The line a host runs for a server: into the folder, its environment, the program and its arguments,
/// every piece quoted for `sh`.
pub fn remote_line(spec: &Value, root: &str) -> Result<String, String> {
    let command = spec["command"].as_str().ok_or("no `command`")?;
    let folder = crate::ssh::ops::remote_expr(root).map_err(|error| error.message)?;
    let env: Vec<String> = spec["env"]
        .as_object()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|(key, _)| key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .filter_map(|(key, value)| value.as_str().map(|value| format!("{key}={}", crate::ssh::sh_quote(value))))
        .collect();
    let words: Vec<String> = std::iter::once(command.to_string()).chain(args_of(spec)).map(|word| crate::ssh::sh_quote(&word)).collect();

    Ok(format!("cd {folder} && exec env {} {}", env.join(" "), words.join(" ")).replace("env  ", "env "))
}

impl Server {
    fn start(name: &str, spec: &Value, root: &std::path::Path) -> Result<Self, String> {
        let command = spec["command"].as_str().ok_or_else(|| format!("{name}: no `command`"))?;
        let mut process = match crate::host::program::command(command) {
            Some(process) => process,
            None => Command::new(command),
        };

        process.args(args_of(spec)).current_dir(root);

        for (key, value) in spec["env"].as_object().cloned().unwrap_or_default() {
            if let Some(value) = value.as_str() {
                process.env(key, value);
            }
        }

        Self::launch(name, process)
    }

    /// A server on a host, over `ssh` (0.13).
    fn start_remote(name: &str, spec: &Value, root: &str, ssh: &crate::ssh::Ssh) -> Result<Self, String> {
        let line = remote_line(spec, root).map_err(|error| format!("{name}: {error}"))?;
        let (program, ssh_args) = ssh.launcher(None).map_err(|error| format!("{name}: {}", error.message))?;
        let launcher = crate::host::program::command_for(std::path::Path::new(&program));
        let mut process = Command::new(launcher.get_program());

        process.args(launcher.get_args()).args(ssh_args).arg(line);

        Self::launch(name, process)
    }

    fn launch(name: &str, mut process: Command) -> Result<Self, String> {
        process.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            process.creation_flags(0x0800_0000);
        }

        let mut child = process.spawn().map_err(|error| format!("{name}: `{:?}` did not start ({error})", process.get_program()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (sender, lines) = channel();

        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        let mut server = Self { name: name.to_string(), child, stdin, lines, next: 1 };

        server.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "sdc", "version": crate::VERSION } }),
            START_TIMEOUT,
        )?;
        server.notify("notifications/initialized", json!({}))?;

        Ok(server)
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        self.stdin.write_all(format!("{message}\n").as_bytes()).and_then(|_| self.stdin.flush()).map_err(|error| format!("{}: {error}", self.name))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next;

        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;

        let deadline = std::time::Instant::now() + timeout;

        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let line = self.lines.recv_timeout(left).map_err(|_| format!("{} did not answer `{method}` in {}s", self.name, timeout.as_secs()))?;
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };

            if message["id"] != id {
                continue;
            }

            if let Some(error) = message.get("error") {
                return Err(format!("{}: {}", self.name, error["message"].as_str().unwrap_or("error")));
            }

            return Ok(message["result"].clone());
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The turn's MCP servers and the tools they offer.
pub struct McpTools {
    servers: Vec<Server>,
    /// `public name → (server index, the server's own tool name)`.
    routes: HashMap<String, (usize, String)>,
    specs: Vec<ToolSpec>,
}

/// `mcp__<server>__<tool>`, reduced to the characters every provider accepts in a tool name.
pub fn public_name(server: &str, tool: &str) -> String {
    let clean = |text: &str| text.chars().map(|character| if character.is_ascii_alphanumeric() || character == '_' || character == '-' { character } else { '_' }).collect::<String>();

    format!("mcp__{}__{}", clean(server), clean(tool)).chars().take(64).collect()
}

impl McpTools {
    /// Starts the servers a folder's `.sdc/mcp.json` names. `None` when it names none; the warnings say
    /// which servers could not start, so the turn can tell the person.
    pub fn start(root: &std::path::Path) -> (Option<Self>, Vec<String>) {
        let Ok(text) = std::fs::read_to_string(root.join(CONFIG)) else {
            return (None, Vec::new());
        };

        Self::from_config(&text, &|name, spec| Server::start(name, spec, root))
    }

    /// The servers of a folder on a host (0.13): its `.sdc/mcp.json` read there, each server started there.
    pub fn start_remote(root: &str, ssh: &crate::ssh::Ssh) -> (Option<Self>, Vec<String>) {
        let path = format!("{}/{CONFIG}", root.trim_end_matches('/'));
        let Some(text) = crate::ssh::ops::read(ssh, &path, 64 * 1024).ok().and_then(|value| value["text"].as_str().map(str::to_string)) else {
            return (None, Vec::new());
        };

        Self::from_config(&text, &|name, spec| Server::start_remote(name, spec, root, ssh))
    }

    fn from_config(text: &str, start: &dyn Fn(&str, &Value) -> Result<Server, String>) -> (Option<Self>, Vec<String>) {
        let config: Value = match serde_json::from_str(text) {
            Ok(config) => config,
            Err(error) => return (None, vec![format!("{CONFIG} is not valid JSON: {error}")]),
        };
        let servers = config.get("mcpServers").or_else(|| config.get("servers")).and_then(Value::as_object).cloned().unwrap_or_default();
        let mut tools = Self { servers: Vec::new(), routes: HashMap::new(), specs: Vec::new() };
        let mut warnings = Vec::new();

        for (name, spec) in servers {
            let mut server = match start(&name, &spec) {
                Ok(server) => server,
                Err(warning) => {
                    warnings.push(warning);
                    continue;
                }
            };
            let listed = match server.request("tools/list", json!({}), START_TIMEOUT) {
                Ok(listed) => listed,
                Err(warning) => {
                    warnings.push(warning);
                    continue;
                }
            };
            let index = tools.servers.len();

            for tool in listed["tools"].as_array().cloned().unwrap_or_default() {
                let Some(tool_name) = tool["name"].as_str() else {
                    continue;
                };
                let public = public_name(&name, tool_name);

                tools.specs.push(ToolSpec {
                    name: intern(public.clone()),
                    description: intern(format!("[MCP server `{name}`] {}", tool["description"].as_str().unwrap_or(tool_name))),
                    schema: tool.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                });
                tools.routes.insert(public, (index, tool_name.to_string()));
            }

            tools.servers.push(server);
        }

        if tools.specs.is_empty() {
            return (None, warnings);
        }

        (Some(tools), warnings)
    }

    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    pub fn handles(&self, name: &str) -> bool {
        self.routes.contains_key(name)
    }

    /// `server.tool`, for the tool card.
    pub fn label(&self, name: &str) -> String {
        self.routes
            .get(name)
            .map(|(index, tool)| format!("{}.{tool}", self.servers[*index].name))
            .unwrap_or_else(|| name.to_string())
    }

    /// Calls one tool; the text of its answer, or the server's error.
    pub fn call(&mut self, name: &str, arguments: &Value) -> Result<String, String> {
        let (index, tool) = self.routes.get(name).cloned().ok_or_else(|| format!("no MCP tool `{name}`"))?;
        let result = self.servers[index].request("tools/call", json!({ "name": tool, "arguments": arguments }), CALL_TIMEOUT)?;
        let text: Vec<String> = result["content"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|part| match part["type"].as_str() {
                Some("text") => part["text"].as_str().unwrap_or_default().to_string(),
                Some(other) => format!("[{other} content]"),
                None => part.to_string(),
            })
            .collect();
        let joined = text.join("\n");

        if result["isError"] == true {
            Err(joined)
        } else {
            Ok(joined)
        }
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;

    #[test]
    fn a_host_server_line_goes_into_the_folder_and_quotes_every_word() {
        let spec = json!({ "command": "npx", "args": ["-y", "@modelcontextprotocol/server-postgres", "postgres://u:p@localhost/shop db"], "env": { "PGSSL": "no" } });
        let line = remote_line(&spec, "/var/www/shop").unwrap();

        assert!(line.starts_with("cd "), "{line}");
        assert!(line.contains("exec env PGSSL='no' 'npx' '-y'"), "{line}");
        assert!(line.contains("'postgres://u:p@localhost/shop db'"), "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_safe_for_every_provider() {
        assert_eq!(public_name("my db", "run.query"), "mcp__my_db__run_query");
        assert!(public_name(&"x".repeat(80), "y").len() <= 64);
    }

    #[test]
    fn a_folder_without_a_config_has_no_mcp_tools() {
        let (tools, warnings) = McpTools::start(&std::env::temp_dir().join("sdc-no-mcp-here"));

        assert!(tools.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_broken_config_is_a_warning_not_a_crash() {
        let root = std::env::temp_dir().join(format!("sdc-mcp-broken-{}", std::process::id()));
        let _ = std::fs::create_dir_all(root.join(".sdc"));
        std::fs::write(root.join(CONFIG), "{ not json").unwrap();

        let (tools, warnings) = McpTools::start(&root);

        assert!(tools.is_none());
        assert!(warnings[0].contains("not valid JSON"));

        std::fs::write(root.join(CONFIG), r#"{"mcpServers":{"ghost":{"command":"sdc-no-such-program-xyz"}}}"#).unwrap();

        let (tools, warnings) = McpTools::start(&root);

        assert!(tools.is_none());
        assert!(warnings[0].contains("did not start"), "{warnings:?}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
