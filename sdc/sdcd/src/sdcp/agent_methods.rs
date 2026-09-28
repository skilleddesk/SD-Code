//! The 0.13 methods around a turn: the agent's questions, the memory files, the composer's `/` commands
//! and `@` files, the background processes an agent started, and the context meter.
//!
//! A child module of `methods`, like the kernel's, so it uses the same private helpers (`remote_for`,
//! `root_for`) instead of a second copy of them.

use std::sync::Arc;

use serde_json::{json, Value};

use super::Daemon;
use crate::sdcp::envelope::{Envelope, ErrorObject};
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;

/// The commands the composer offers after `/`, SDC's own. A project adds its own as Markdown files.
pub const BUILT_IN_COMMANDS: &[(&str, &str)] = &[
    ("compact", "Summarise this chat so far and continue from the summary - frees the model's context"),
    ("init", "Study this project and write .sdc/rules.md: how to build, test and run it, and its conventions"),
    ("review", "Review the uncommitted changes for bugs, security and style"),
    ("remember", "Keep a fact in this project's memory (.sdc/memory.md): /remember <text>"),
    ("memory", "Open this project's memory and SDC's global memory"),
    ("clear", "Start a new chat in the same folder"),
    ("help", "What the / commands do"),
];

/// The folders a project keeps its own commands in: one Markdown file per command, `$ARGUMENTS` where
/// the words after the command go - the shape Claude Code reads, so a project's existing commands work.
pub const COMMAND_DIRS: &[&str] = &[".sdc/commands", ".claude/commands"];

impl Daemon {
    pub(super) fn dispatch_agent(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            "question.answer" => self.question_answer(envelope, &*out),
            "memory.get" => self.memory_get(envelope),
            "memory.set" => self.memory_set(envelope),
            "memory.add" => self.memory_add(envelope),
            "commands.list" => self.commands_list(envelope),
            "files.find" => self.files_find(envelope),
            "process.list" => Ok(json!({ "processes": crate::agent::background::list(envelope.opt_str("sessionId").as_deref()) })),
            "process.stop" => {
                let id = envelope.require_str("processId")?;

                Ok(json!({ "stopped": crate::agent::background::stop(&id) }))
            }
            "context.get" => self.context_get(envelope),
            "app.erase" => self.app_erase(envelope),
            other => Err(ErrorObject::unsupported(other)),
        }
    }

    /// `question.answer`: the person's reply to an agent's `ask_user`, handed to the turn that waits.
    fn question_answer(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let question_id = envelope.require_str("questionId")?;
        let answer = envelope.opt_str("answer").unwrap_or_default();
        let answered = crate::agent::gate::resolve(&question_id, &format!("answer:{answer}"));
        let turn_id = envelope.opt_str("turnId").unwrap_or_default();

        out.push(event::question_answered(&turn_id, &question_id, &answer), envelope.opt_str("sessionId"), Some(turn_id.clone()));

        Ok(json!({ "answered": answered }))
    }

    /// Where a memory lives: the project's `.sdc/memory.md` (on its machine), or SDC's global one.
    fn memory_place(&self, envelope: &Envelope) -> Result<MemoryPlace, ErrorObject> {
        if envelope.opt_str("scope").as_deref() == Some("global") {
            return Ok(MemoryPlace::Global(global_memory_path()?));
        }

        let root = self
            .root_for(envelope)?
            .ok_or_else(|| ErrorObject::bad_request("This chat has no folder, so there is no project memory; use the global memory"))?;
        let root = root.to_string_lossy().to_string();

        Ok(match self.remote_for(envelope)? {
            Some(ssh) => MemoryPlace::Remote(ssh, format!("{}/.sdc/memory.md", root.trim_end_matches('/'))),
            None => MemoryPlace::Local(std::path::Path::new(&root).join(".sdc").join("memory.md")),
        })
    }

    fn memory_get(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let place = self.memory_place(envelope)?;

        Ok(json!({ "text": place.read(), "path": place.label() }))
    }

    fn memory_set(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let place = self.memory_place(envelope)?;
        let text = envelope.opt_str("text").unwrap_or_default();

        place.write(&text)?;

        Ok(json!({ "saved": true, "path": place.label() }))
    }

    /// `memory.add` - `/remember <text>`: one line appended, the file made when there is none.
    fn memory_add(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let place = self.memory_place(envelope)?;
        let fact = envelope.require_str("text")?;
        let text = append_fact(&place.read(), &fact);

        place.write(&text)?;

        Ok(json!({ "saved": true, "path": place.label(), "text": text }))
    }

    /// `commands.list`: SDC's own `/` commands, then the project's (`.sdc/commands/*.md`,
    /// `.claude/commands/*.md`) with the first line of each as its description.
    fn commands_list(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let mut commands: Vec<Value> = BUILT_IN_COMMANDS
            .iter()
            .map(|(name, description)| json!({ "name": name, "description": description, "source": "sdc" }))
            .collect();

        if let Some(root) = self.root_for(envelope).ok().flatten() {
            let root = root.to_string_lossy().to_string();
            let remote = self.remote_for(envelope).ok().flatten();

            for dir in COMMAND_DIRS {
                for (name, body) in read_commands(&root, dir, remote.as_ref()) {
                    if commands.iter().any(|command| command["name"] == name.as_str()) {
                        continue;
                    }

                    let description = body
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty() && *line != "---" && !line.contains(':') || line.starts_with('#'))
                        .unwrap_or_default()
                        .trim_start_matches('#')
                        .trim()
                        .chars()
                        .take(100)
                        .collect::<String>();

                    commands.push(json!({ "name": name, "description": description, "source": dir, "body": body }));
                }
            }
        }

        Ok(json!({ "commands": commands }))
    }

    /// `files.find` - the composer's `@` list: files under the chat's folder whose name contains the query.
    fn files_find(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let Some(root) = self.root_for(envelope)? else {
            return Ok(json!({ "files": [] }));
        };
        let query = envelope.opt_str("query").unwrap_or_default();
        let limit = envelope.opt_i64("limit").unwrap_or(30).clamp(1, 100) as usize;
        let root_text = root.to_string_lossy().to_string();
        let found = match self.remote_for(envelope)? {
            Some(ssh) => crate::ssh::ops::find_names(&ssh, &root_text, if query.is_empty() { "." } else { &query }, limit)?,
            None => crate::fs::find_names(&root, if query.is_empty() { "." } else { &query }, limit)?,
        };
        let files: Vec<Value> = found
            .iter()
            .filter_map(|hit| {
                let path = hit["path"].as_str()?;
                let relative = path.strip_prefix(root_text.as_str()).unwrap_or(path).trim_start_matches(['/', '\\']).replace('\\', "/");

                (!relative.is_empty()).then(|| json!({ "path": relative, "dir": hit["dir"].as_bool().unwrap_or(false) }))
            })
            .collect();

        Ok(json!({ "files": files }))
    }

    /// `app.erase` - Settings → Erase all SDC data (0.13). The keys go now; the database cannot be deleted
    /// while it is open, so a marker is left and the daemon exits: the app starts a new one, which erases
    /// the folder before it opens anything. `confirm` must say `ERASE`, so no stray call can do this.
    fn app_erase(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        if envelope.opt_str("confirm").as_deref() != Some("ERASE") {
            return Err(ErrorObject::bad_request("app.erase needs confirm: \"ERASE\""));
        }

        let dir = crate::paths::data_dir().map_err(ErrorObject::internal)?;

        std::fs::write(dir.join(crate::reset::MARKER), "").map_err(ErrorObject::internal)?;
        crate::agent::background::stop_all(None);

        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(800));
            std::process::exit(0);
        });

        Ok(json!({ "erasing": true }))
    }

    /// `context.get`: the meter before a turn - what the next turn of this chat would send to this model.
    fn context_get(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let session_id = envelope.require_str("sessionId")?;
        let engine = envelope.opt_str("engine").unwrap_or_else(|| "claude_code".into());
        let model = envelope.opt_str("model").unwrap_or_default();
        let provider = envelope.opt_str("provider");
        let window = crate::context::window_tokens(&engine, provider.as_deref(), &model);
        let fitted = crate::context::history(self.store(), &session_id, "", crate::context::history_budget(window));
        let resumed = matches!(engine.as_str(), "claude_code" | "codex" | "gemini") && crate::context::load_resume(self.store(), &session_id, &engine).is_some();

        Ok(json!({
            "usedTokens": fitted.tokens,
            "windowTokens": window,
            "percent": (fitted.tokens * 100).checked_div(window).unwrap_or(0).min(100),
            "compacted": fitted.compacted,
            "resumed": resumed,
        }))
    }

    /// The language this chat is written in, from the person's recent messages: a short English follow-up
    /// in a Bengali chat is answered in Bengali. `None` for an English chat.
    pub(super) fn chat_language(&self, session_id: &str, current_turn: &str) -> Option<&'static str> {
        let turns = crate::context::live_turns(self.store(), session_id, current_turn);

        turns
            .iter()
            .rev()
            .take(6)
            .map(|turn| crate::understand::Reading::of(&turn.prompt))
            .find(|reading| reading.code != "en")
            .map(|reading| reading.reply_in)
    }

    /// The images of `engine.start`'s `images` (base64), saved where the turn's engine can open them: this
    /// machine's attachments folder, and - for a turn on a host - that host's `~/.sdc/attachments` too.
    pub(super) fn save_attachments(&self, envelope: &Envelope, turn_id: &str, remote: Option<&crate::ssh::Ssh>) -> Vec<crate::engines::Attachment> {
        use base64::Engine as _;

        let Some(images) = envelope.params.get("images").and_then(Value::as_array) else {
            return Vec::new();
        };
        let Ok(dir) = crate::paths::data_dir().map(|dir| dir.join("attachments").join(turn_id)) else {
            return Vec::new();
        };
        let mut saved = Vec::new();

        for (index, image) in images.iter().take(8).enumerate() {
            let media_type = image["mediaType"].as_str().unwrap_or("image/png").to_string();
            /* A pasted image arrives as its bytes; a picked one as a path on this machine, read here. */
            let bytes = match image["data"].as_str() {
                Some(data) => base64::engine::general_purpose::STANDARD
                    .decode(data.split_once("base64,").map(|(_, rest)| rest).unwrap_or(data).trim())
                    .ok(),
                None => image["path"]
                    .as_str()
                    .filter(|path| crate::fs::blocked_reason(std::path::Path::new(path)).is_none())
                    .and_then(|path| std::fs::read(path).ok()),
            };
            let Some(bytes) = bytes else {
                continue;
            };

            if bytes.len() > 20 * 1024 * 1024 {
                continue;
            }

            let extension = match media_type.as_str() {
                "image/jpeg" => "jpg",
                "image/gif" => "gif",
                "image/webp" => "webp",
                _ => "png",
            };
            let name: String = image["name"]
                .as_str()
                .unwrap_or("image")
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
                .take(60)
                .collect();
            let file = format!("{}-{}.{extension}", index + 1, name.trim_end_matches(&format!(".{extension}")));

            if std::fs::create_dir_all(&dir).is_err() || std::fs::write(dir.join(&file), &bytes).is_err() {
                continue;
            }

            let path = dir.join(&file).to_string_lossy().to_string();
            let mut remote_path = None;

            /* A CLI on a host opens files there: the image is copied over, and its path is the host's. */
            if let Some(ssh) = remote {
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let target = format!("$HOME/.sdc/attachments/{turn_id}");
                let line = format!("mkdir -p \"{target}\" && base64 -d > \"{target}/{file}\" && echo \"{target}/{file}\"");

                if let Ok(output) = ssh.run_with_stdin(&line, &encoded, std::time::Duration::from_secs(60)) {
                    if let Some(found) = output.stdout.lines().last().filter(|line| line.starts_with('/')) {
                        remote_path = Some(found.trim().to_string());
                    }
                }
            }

            saved.push(crate::engines::Attachment { name: file, path, media_type, remote_path });
        }

        saved
    }
}

/// SDC's global memory: what the person wants every chat, in every project, to know.
pub fn global_memory_path() -> Result<std::path::PathBuf, ErrorObject> {
    Ok(crate::paths::data_dir().map_err(ErrorObject::internal)?.join("memory.md"))
}

/// The global memory's text, for a turn's brief (empty when there is none).
pub fn global_memory() -> String {
    global_memory_path().ok().and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default()
}

enum MemoryPlace {
    Global(std::path::PathBuf),
    Local(std::path::PathBuf),
    Remote(crate::ssh::Ssh, String),
}

impl MemoryPlace {
    fn read(&self) -> String {
        match self {
            MemoryPlace::Global(path) | MemoryPlace::Local(path) => std::fs::read_to_string(path).unwrap_or_default(),
            MemoryPlace::Remote(ssh, path) => crate::ssh::ops::read(ssh, path, 64 * 1024)
                .ok()
                .and_then(|value| value["text"].as_str().map(str::to_string))
                .unwrap_or_default(),
        }
    }

    fn write(&self, text: &str) -> Result<(), ErrorObject> {
        match self {
            /* The memory is SDC's own file: written directly, past the file guard that keeps `.sdc` from tools. */
            MemoryPlace::Global(path) | MemoryPlace::Local(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(ErrorObject::internal)?;
                }

                std::fs::write(path, text).map_err(ErrorObject::internal)
            }
            MemoryPlace::Remote(ssh, path) => crate::ssh::ops::write(ssh, path, text).map(|_| ()),
        }
    }

    fn label(&self) -> String {
        match self {
            MemoryPlace::Global(path) | MemoryPlace::Local(path) => path.to_string_lossy().to_string(),
            MemoryPlace::Remote(ssh, path) => format!("{}:{path}", ssh.label()),
        }
    }
}

/// A memory file with one more fact: a heading when the file is new, the fact as a list item.
pub fn append_fact(existing: &str, fact: &str) -> String {
    let fact = fact.trim().trim_start_matches(['-', '*', '#']).trim();
    let mut text = if existing.trim().is_empty() {
        "# Memory\n\nWhat SDC keeps in mind for every chat here. Edit freely.\n\n".to_string()
    } else {
        let mut text = existing.trim_end().to_string();

        text.push('\n');
        text
    };

    text.push_str(&format!("- {fact}\n"));
    text
}

/// A project's command files in `dir`: `(name, text)`, the name being the file's stem.
fn read_commands(root: &str, dir: &str, remote: Option<&crate::ssh::Ssh>) -> Vec<(String, String)> {
    let mut commands = Vec::new();

    match remote {
        Some(ssh) => {
            let folder = format!("{}/{dir}", root.trim_end_matches('/'));

            if let Ok((_, entries, _)) = crate::ssh::ops::list(ssh, &folder) {
                for entry in entries.iter().take(40) {
                    let name = entry["name"].as_str().unwrap_or_default();

                    if let Some(stem) = name.strip_suffix(".md") {
                        if let Ok(value) = crate::ssh::ops::read(ssh, &format!("{folder}/{name}"), 32 * 1024) {
                            commands.push((stem.to_string(), value["text"].as_str().unwrap_or_default().to_string()));
                        }
                    }
                }
            }
        }
        None => {
            let folder = std::path::Path::new(root).join(dir);

            if let Ok(entries) = std::fs::read_dir(&folder) {
                for entry in entries.flatten().take(40) {
                    let path = entry.path();

                    if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                        if let (Some(stem), Ok(text)) = (path.file_stem().and_then(|stem| stem.to_str()), std::fs::read_to_string(&path)) {
                            commands.push((stem.to_string(), text.chars().take(32 * 1024).collect()));
                        }
                    }
                }
            }
        }
    }

    commands.sort();
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fact_starts_a_memory_file_and_joins_an_existing_one() {
        let first = append_fact("", "Use pnpm, never npm");

        assert!(first.starts_with("# Memory"));
        assert!(first.ends_with("- Use pnpm, never npm\n"));

        let second = append_fact(&first, "- The site is example-shop.com");

        assert!(second.ends_with("- Use pnpm, never npm\n- The site is example-shop.com\n"), "{second}");
    }

    #[test]
    fn a_project_command_is_read_from_its_markdown_file() {
        let root = std::env::temp_dir().join(format!("sdc-commands-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".claude/commands")).unwrap();
        std::fs::write(root.join(".claude/commands/deploy.md"), "# Deploy to staging\nRun the deploy for $ARGUMENTS").unwrap();

        let found = read_commands(root.to_str().unwrap(), ".claude/commands", None);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "deploy");
        assert!(found[0].1.contains("$ARGUMENTS"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
