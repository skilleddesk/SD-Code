//! **The LSP bridge (P1-12, switch `verify.lsp`)**: after the agent changes a file, a language server says what is wrong with it - type
//! errors, missing names, wrong arities - which a syntax check cannot see, and each error goes back to the model together with the
//! declaration of the names it mentions (from the Code Graph), so the repair prompt carries the signature the model got wrong.
//!
//! * Servers are **started lazily and kept warm** per (project, language): the first diagnostics wait for the server to start, the next
//!   come in tens of milliseconds. A server that dies is started again once for the same request, so a crash costs time, not the answer.
//! * Which server: `rust-analyzer`, `pyright-langserver`, `typescript-language-server`, `gopls` - whichever is on the PATH. None
//!   installed is an ordinary answer (`None`), and the tree-sitter gate (P1-2) still judges syntax; SDC installs none of them by itself.
//! * The protocol is LSP over stdio (`Content-Length` framing). Only what diagnostics need is spoken: `initialize`, `initialized`,
//!   `textDocument/didOpen` and `didChange`, `publishDiagnostics`, `shutdown`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::graph::Lang;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub column: usize,
    /// `error`, `warning`, `info` or `hint`.
    pub severity: &'static str,
    pub message: String,
    pub source: String,
}

/// The server command for a language, when one is installed.
pub fn server_for(lang: Lang) -> Option<(String, Vec<String>)> {
    /* `SDC_LSP_COMMAND='["program","arg",...]'` names the server for every language: a server SDC does not know, or a test's. */
    if let Some(custom) = std::env::var("SDC_LSP_COMMAND").ok().and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok()).filter(|list| !list.is_empty()) {
        return Some((custom[0].clone(), custom[1..].to_vec()));
    }

    let candidates: &[(&str, &[&str])] = match lang {
        Lang::Rust => &[("rust-analyzer", &[])],
        Lang::Python => &[("pyright-langserver", &["--stdio"]), ("pylsp", &[])],
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => &[("typescript-language-server", &["--stdio"])],
        Lang::Go => &[("gopls", &[])],
        _ => &[],
    };

    candidates.iter().find(|(program, _)| crate::host::program::resolve(program).is_some()).map(|(program, args)| (program.to_string(), args.iter().map(|arg| arg.to_string()).collect()))
}

fn language_id(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "rust",
        Lang::Python => "python",
        Lang::JavaScript => "javascript",
        Lang::TypeScript => "typescript",
        Lang::Tsx => "typescriptreact",
        Lang::Go => "go",
        Lang::Php => "php",
        Lang::Json => "json",
        Lang::Bash => "shellscript",
    }
}

fn uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");

    if text.starts_with('/') { format!("file://{text}") } else { format!("file:///{text}") }
}

pub struct Server {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    next: i64,
    open: HashMap<String, i64>,
    root: PathBuf,
}

fn reader(stdout: impl Read + Send + 'static) -> Receiver<Value> {
    let (send, receive) = channel();

    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);

        loop {
            let mut length = None;

            loop {
                let mut line = String::new();

                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }

                let line = line.trim().to_string();

                if line.is_empty() {
                    break;
                }

                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().ok();
                }
            }

            let Some(length) = length else {
                continue;
            };
            let mut body = vec![0u8; length];

            if reader.read_exact(&mut body).is_err() {
                return;
            }

            if let Ok(message) = serde_json::from_slice::<Value>(&body) {
                if send.send(message).is_err() {
                    return;
                }
            }
        }
    });

    receive
}

impl Server {
    /// Starts a server and completes the handshake. `Err` names why (not installed, would not start, did not answer).
    pub fn start(program: &str, args: &[String], root: &Path) -> Result<Self, String> {
        let mut command = Command::new(program);

        command.args(args).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());

        let mut child = command.spawn().map_err(|error| format!("{program} could not be started: {error}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut server = Self { child, stdin, messages: reader(stdout), next: 1, open: HashMap::new(), root: root.to_path_buf() };
        let id = server.request("initialize", json!({ "processId": std::process::id(), "rootUri": uri(root), "capabilities": { "textDocument": { "publishDiagnostics": { "relatedInformation": false } } }, "workspaceFolders": [{ "uri": uri(root), "name": "project" }] }))?;

        server.wait_for_response(id, Duration::from_secs(30))?;
        server.notify("initialized", json!({}))?;

        Ok(server)
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let body = message.to_string();

        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).and_then(|_| self.stdin.flush()).map_err(|error| format!("the language server stopped listening: {error}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<i64, String> {
        let id = self.next;

        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;

        Ok(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn wait_for_response(&mut self, id: i64, limit: Duration) -> Result<Value, String> {
        let deadline = Instant::now() + limit;

        loop {
            let left = deadline.saturating_duration_since(Instant::now());

            match self.messages.recv_timeout(left) {
                Ok(message) if message["id"] == id && message.get("method").is_none() => return Ok(message["result"].clone()),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => return Err("the language server did not answer in time".into()),
                Err(RecvTimeoutError::Disconnected) => return Err("the language server exited".into()),
            }
        }
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// The diagnostics for `text` as the file at `path`. Waits for the server's first report for this version of the document, then a
    /// short quiet period for a follow-up (servers publish syntax first and semantics a moment later), up to `limit`.
    pub fn diagnostics(&mut self, lang: Lang, path: &Path, text: &str, limit: Duration) -> Result<Vec<Diagnostic>, String> {
        let document = uri(path);
        let version = self.open.get(&document).copied().unwrap_or(0) + 1;

        while self.messages.try_recv().is_ok() {}

        if version == 1 {
            self.notify("textDocument/didOpen", json!({ "textDocument": { "uri": document, "languageId": language_id(lang), "version": version, "text": text } }))?;
        } else {
            self.notify("textDocument/didChange", json!({ "textDocument": { "uri": document, "version": version }, "contentChanges": [{ "text": text }] }))?;
        }

        self.open.insert(document.clone(), version);

        let deadline = Instant::now() + limit;
        let mut latest: Option<Vec<Diagnostic>> = None;
        let mut settle: Option<Instant> = None;

        loop {
            let now = Instant::now();
            let wait = match settle {
                Some(until) => until.saturating_duration_since(now).min(deadline.saturating_duration_since(now)),
                None => deadline.saturating_duration_since(now),
            };

            if wait.is_zero() {
                break;
            }

            match self.messages.recv_timeout(wait) {
                Ok(message) if message["method"] == "textDocument/publishDiagnostics" && message["params"]["uri"] == document.as_str() => {
                    latest = Some(parse_diagnostics(&message["params"]["diagnostics"]));
                    settle = Some(Instant::now() + Duration::from_millis(300));
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    if settle.is_some() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return latest.ok_or_else(|| "the language server exited before it reported".to_string());
                }
            }
        }

        latest.ok_or_else(|| "the language server did not report diagnostics in time".to_string())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn shutdown(&mut self) {
        if let Ok(id) = self.request("shutdown", json!(null)) {
            let _ = self.wait_for_response(id, Duration::from_secs(2));
        }

        let _ = self.notify("exit", json!(null));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn parse_diagnostics(list: &Value) -> Vec<Diagnostic> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|item| Diagnostic {
            line: item["range"]["start"]["line"].as_u64().unwrap_or(0) as usize + 1,
            column: item["range"]["start"]["character"].as_u64().unwrap_or(0) as usize + 1,
            severity: match item["severity"].as_u64() {
                Some(1) => "error",
                Some(2) => "warning",
                Some(3) => "info",
                Some(4) => "hint",
                _ => "error",
            },
            message: item["message"].as_str().unwrap_or_default().lines().next().unwrap_or_default().to_string(),
            source: item["source"].as_str().unwrap_or_default().to_string(),
        })
        .collect()
}

fn pool() -> &'static Mutex<HashMap<(PathBuf, &'static str), Server>> {
    static POOL: OnceLock<Mutex<HashMap<(PathBuf, &'static str), Server>>> = OnceLock::new();

    POOL.get_or_init(Default::default)
}

/// Diagnostics for a changed file from the (warm) server of its language, started on first use and started again once if it died.
/// `Ok(None)` means there is no server installed for the language (not an error); `command` overrides the lookup (tests).
pub fn check(root: &Path, relative: &str, text: &str, command: Option<(String, Vec<String>)>) -> Result<Option<Vec<Diagnostic>>, String> {
    let Some(lang) = Lang::of(relative) else {
        return Ok(None);
    };
    let Some((program, args)) = command.or_else(|| server_for(lang)) else {
        return Ok(None);
    };
    let key = (root.to_path_buf(), lang.name());
    let mut servers = pool().lock().unwrap_or_else(|poison| poison.into_inner());
    let path = root.join(relative);

    for attempt in 0..2 {
        if !servers.get_mut(&key).is_some_and(Server::alive) {
            servers.remove(&key);
            servers.insert(key.clone(), Server::start(&program, &args, root)?);
        }

        let server = servers.get_mut(&key).expect("just inserted");

        match server.diagnostics(lang, &path, text, Duration::from_secs(if attempt == 0 { 20 } else { 10 })) {
            Ok(found) => return Ok(Some(found)),
            Err(error) if attempt == 1 => return Err(error),
            Err(_) => {
                servers.remove(&key);
            }
        }
    }

    Err("the language server could not be reached".into())
}

/// Stops every warm server (at shutdown, and in tests).
pub fn stop_all() {
    for (_, mut server) in pool().lock().unwrap_or_else(|poison| poison.into_inner()).drain() {
        server.shutdown();
    }
}

/// The text the model gets back after an edit: each error with the declarations of the names it mentions, so the repair has the real
/// signature in front of it. Warnings are counted, not listed. Empty when there is nothing to report.
pub fn repair_note(root: &Path, relative: &str, diagnostics: &[Diagnostic]) -> String {
    let errors: Vec<&Diagnostic> = diagnostics.iter().filter(|diagnostic| diagnostic.severity == "error").collect();

    if errors.is_empty() {
        return String::new();
    }

    let (index, _) = crate::graph::index::refreshed(root);
    let mut lines = vec![format!("The language server reports {} error(s) in {relative}:", errors.len())];

    for diagnostic in errors.iter().take(8) {
        lines.push(format!("  line {}:{} {}", diagnostic.line, diagnostic.column, diagnostic.message));

        /* names the message puts in quotes or backticks: where they are declared, with the first line of the declaration */
        let mut seen = std::collections::HashSet::new();

        for name in quoted_names(&diagnostic.message).into_iter().filter(|name| seen.insert(name.clone())).take(3) {
            for (file, def) in index.where_is(&name).into_iter().take(2) {
                let signature = std::fs::read_to_string(root.join(&file)).ok().and_then(|text| text.get(def.start_byte..def.end_byte).map(|body| body.lines().next().unwrap_or_default().trim().to_string())).unwrap_or_default();

                lines.push(format!("    `{name}` is declared at {file}:{}: {signature}", def.start_line));
            }
        }
    }

    if errors.len() > 8 {
        lines.push(format!("  ... and {} more", errors.len() - 8));
    }

    let warnings = diagnostics.len() - errors.len();

    if warnings > 0 {
        lines.push(format!("({warnings} warning(s) not shown)"));
    }

    lines.join("\n")
}

fn quoted_names(message: &str) -> Vec<String> {
    let mut names = Vec::new();

    for quote in ['`', '\'', '"'] {
        let mut parts = message.split(quote);

        parts.next();

        for (index, part) in parts.enumerate() {
            if index % 2 == 0 && !part.is_empty() && part.len() <= 60 && part.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '$')) {
                names.push(part.rsplit(['.', ':']).next().unwrap_or(part).to_string());
            }
        }
    }

    names
}

#[cfg(test)]
mod tests;
