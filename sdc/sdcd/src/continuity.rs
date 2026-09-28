//! Continuity - one project, many models, one style (0.12.5).
//!
//! SDC lets a chat move between models: Claude writes the first half, DeepSeek or Qwen the second. The
//! report: *"akta model diye code likhe then onno model a switch korle o jano abr vull na kore. je shape je
//! style a ak model likse o jano sai rokom follow koro"*. A model only knows what it is shown, and the next
//! one had been shown the conversation, not the code's conventions or the fact that someone else wrote it.
//!
//! So every turn that works in a folder carries a short brief in front of the person's words:
//!
//! * **the project's conventions**: its formatter and editor settings (`.editorconfig`, Prettier, ESLint,
//!   `rustfmt.toml`, `pyproject.toml`'s tool sections), the stack its manifest names, and the rules file the
//!   engine's own tooling would not read by itself;
//! * **a hand-over**, when the model differs from the one that wrote the earlier turns: who wrote them, the
//!   files they changed, and the instruction to read those files first and match them rather than restyle.
//!
//! The brief is for this turn only; the stored prompt stays the person's own.

use serde_json::Value;

/// Config files whose settings decide how code looks, in the order they are shown.
pub const STYLE_FILES: &[&str] = &[
    ".editorconfig",
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.js",
    ".prettierrc.cjs",
    "prettier.config.js",
    "prettier.config.mjs",
    ".eslintrc",
    ".eslintrc.json",
    ".eslintrc.cjs",
    "eslint.config.js",
    "eslint.config.mjs",
    "biome.json",
    "rustfmt.toml",
    ".rustfmt.toml",
    ".clang-format",
    "phpcs.xml",
    ".php-cs-fixer.php",
    "ruff.toml",
    ".flake8",
];

/// What the brief is built from; gathered locally or over ssh, and pure from here on.
#[derive(Debug, Default, Clone)]
pub struct Facts {
    /// `(file name, first lines)` of each style config present.
    pub style: Vec<(String, String)>,
    /// The stack, from the manifest: `package.json` deps, `composer.json`, `Cargo.toml`, `pyproject.toml`.
    pub stack: Vec<String>,
    /// The rules file (`AGENTS.md`, `CLAUDE.md`, ...) and its text, when the engine would not read it itself.
    pub rules: Option<(String, String)>,
}

/// One earlier turn of the chat: which engine and model wrote it.
#[derive(Debug, Clone, PartialEq)]
pub struct Author {
    pub turn_id: String,
    pub engine: String,
    pub model: String,
}

/// The brief, or `None` when there is nothing worth saying (a first turn in a folder with no config).
pub fn brief(facts: &Facts, earlier: &[Author], edited: &[(String, String)], engine: &str, model: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    /* The hand-over first: it is the instruction; the conventions below are the evidence. */
    let others: Vec<&Author> = earlier.iter().filter(|author| author.model != model || author.engine != engine).collect();

    if !others.is_empty() {
        let mut names: Vec<String> = Vec::new();

        for author in &others {
            let name = format!("{} ({})", author.model, author.engine);

            if !names.contains(&name) {
                names.push(name);
            }
        }

        let theirs: Vec<&str> = edited
            .iter()
            .filter(|(turn, _)| others.iter().any(|author| &author.turn_id == turn))
            .map(|(_, path)| path.as_str())
            .rev()
            .take(15)
            .collect();
        let mut text = format!(
            "Hand-over: earlier turns in this chat were written by {}. You are continuing their work, not starting over.\n\
             - Before you change anything, read the files they wrote or changed, and match what you find: file layout, \
             naming, formatting, comment style, error handling, the libraries and patterns already in use.\n\
             - Extend the existing structure; do not rename, reformat or rewrite working code to your own taste.\n\
             - If something they did looks wrong, fix that one thing and say why - do not restyle around it.",
            names.join(", ")
        );

        if !theirs.is_empty() {
            text.push_str(&format!("\nFiles they changed (newest first): {}", theirs.join(", ")));
        }

        parts.push(text);
    }

    if !facts.stack.is_empty() {
        parts.push(format!("Project stack: {}.", facts.stack.join(", ")));
    }

    if !facts.style.is_empty() {
        let configs: Vec<String> = facts
            .style
            .iter()
            .map(|(name, text)| format!("--- {name}\n{}", text.trim()))
            .collect();

        parts.push(format!(
            "The project's formatting rules - follow them in every file you write:\n{}",
            configs.join("\n")
        ));
    }

    if let Some((name, text)) = &facts.rules {
        parts.push(format!("The project's own rules ({name}):\n{}", text.trim()));
    }

    if parts.is_empty() {
        return None;
    }

    Some(format!("[Project continuity - SDC]\n{}\n[End of continuity]", parts.join("\n\n")))
}

/// The rules file this engine's own CLI already reads, so it is not sent twice.
fn read_by_engine(engine: &str, file: &str) -> bool {
    matches!((engine, file), ("claude_code", "CLAUDE.md") | ("codex", "AGENTS.md") | ("gemini", "GEMINI.md"))
}

/// The stack a manifest names: a few words, not the dependency list.
pub fn stack_of(manifest: &str, text: &str) -> Vec<String> {
    let mut stack = Vec::new();

    match manifest {
        "package.json" => {
            let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
            let mut names: Vec<String> = Vec::new();

            for group in ["dependencies", "devDependencies"] {
                if let Some(map) = parsed.get(group).and_then(Value::as_object) {
                    names.extend(map.keys().cloned());
                }
            }

            let known = [
                ("next", "Next.js"),
                ("react", "React"),
                ("vue", "Vue"),
                ("svelte", "Svelte"),
                ("@angular/core", "Angular"),
                ("express", "Express"),
                ("fastify", "Fastify"),
                ("@nestjs/core", "NestJS"),
                ("tailwindcss", "Tailwind CSS"),
                ("typescript", "TypeScript"),
                ("vite", "Vite"),
                ("jest", "Jest"),
                ("vitest", "Vitest"),
                ("prisma", "Prisma"),
                ("zustand", "Zustand"),
            ];

            for (package, label) in known {
                if names.iter().any(|name| name == package) {
                    stack.push(label.to_string());
                }
            }

            if parsed.get("type").and_then(Value::as_str) == Some("module") {
                stack.push("ES modules".into());
            } else if !names.is_empty() || parsed.is_object() {
                stack.push("Node.js".into());
            }
        }
        "composer.json" => {
            stack.push("PHP".into());

            if text.contains("laravel/framework") {
                stack.push("Laravel".into());
            }

            if text.contains("symfony/") {
                stack.push("Symfony".into());
            }
        }
        "wp-config.php" => stack.push("WordPress".into()),
        "Cargo.toml" => stack.push("Rust".into()),
        "pyproject.toml" | "requirements.txt" => {
            stack.push("Python".into());

            for (package, label) in [("django", "Django"), ("fastapi", "FastAPI"), ("flask", "Flask")] {
                if text.to_lowercase().contains(package) {
                    stack.push(label.into());
                }
            }
        }
        "go.mod" => stack.push("Go".into()),
        _ => {}
    }

    stack
}

/// The manifests read for the stack.
pub const MANIFESTS: &[&str] = &["package.json", "composer.json", "wp-config.php", "Cargo.toml", "pyproject.toml", "requirements.txt", "go.mod"];

/// First lines of a config: enough to show its settings, not a whole file.
fn head(text: &str) -> String {
    text.lines().take(40).collect::<Vec<_>>().join("\n").chars().take(1500).collect()
}

/// The facts of a folder on this machine.
pub fn gather_local(root: &std::path::Path, engine: &str) -> Facts {
    let mut facts = Facts::default();

    for name in STYLE_FILES {
        if let Ok(text) = std::fs::read_to_string(root.join(name)) {
            facts.style.push((name.to_string(), head(&text)));
        }
    }

    for name in MANIFESTS {
        if let Ok(text) = std::fs::read_to_string(root.join(name)) {
            if *name == "package.json" {
                if let Some(prettier) = serde_json::from_str::<Value>(&text).ok().and_then(|value| value.get("prettier").cloned()) {
                    facts.style.push(("package.json → prettier".into(), prettier.to_string()));
                }
            }

            for item in stack_of(name, &text) {
                if !facts.stack.contains(&item) {
                    facts.stack.push(item);
                }
            }
        }
    }

    for name in crate::intent::RULES_FILES {
        if let Ok(text) = std::fs::read_to_string(root.join(name)) {
            if !read_by_engine(engine, name) {
                facts.rules = Some((name.to_string(), text.chars().take(4000).collect()));
            }

            break;
        }
    }

    facts
}

/// The same facts from a folder on a host, in one round trip.
pub fn gather_remote(ssh: &crate::ssh::Ssh, root: &str, engine: &str) -> Facts {
    let mut facts = Facts::default();
    let Ok(quoted) = crate::ssh::ops::remote_expr(root) else {
        return facts;
    };
    let files: Vec<&str> = STYLE_FILES.iter().chain(MANIFESTS.iter()).chain(crate::intent::RULES_FILES.iter()).copied().collect();
    let reads: Vec<String> = files
        .iter()
        .map(|name| format!("[ -f {name} ] && {{ echo '@@FILE {name}'; head -c 6000 {name}; echo; echo '@@END'; }}"))
        .collect();
    let script = format!("cd {quoted} 2>/dev/null || exit 0; {}; true", reads.join("; "));
    let Ok(output) = ssh.run(&script, crate::ssh::QUICK) else {
        return facts;
    };
    let mut current: Option<String> = None;
    let mut buffer = String::new();

    for line in output.stdout.lines() {
        if let Some(name) = line.strip_prefix("@@FILE ") {
            current = Some(name.to_string());
            buffer.clear();
        } else if line == "@@END" {
            if let Some(name) = current.take() {
                if STYLE_FILES.contains(&name.as_str()) {
                    facts.style.push((name.clone(), head(&buffer)));
                } else if MANIFESTS.contains(&name.as_str()) {
                    for item in stack_of(&name, &buffer) {
                        if !facts.stack.contains(&item) {
                            facts.stack.push(item);
                        }
                    }
                } else if facts.rules.is_none() && !read_by_engine(engine, &name) {
                    facts.rules = Some((name.clone(), buffer.chars().take(4000).collect()));
                }
            }
        } else if current.is_some() {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }

    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn author(turn: &str, engine: &str, model: &str) -> Author {
        Author { turn_id: turn.into(), engine: engine.into(), model: model.into() }
    }

    /// The report's case: Claude wrote it, Qwen continues - Qwen is told who, what, and to match it.
    #[test]
    fn a_new_model_is_handed_the_earlier_models_work() {
        let earlier = [author("t1", "claude_code", "sonnet"), author("t2", "claude_code", "sonnet")];
        let edited = [("t1".to_string(), "src/api.ts".to_string()), ("t2".to_string(), "src/db.ts".to_string())];
        let text = brief(&Facts::default(), &earlier, &edited, "native_api", "qwen3.8-max").unwrap();

        assert!(text.contains("sonnet (claude_code)"), "{text}");
        assert!(text.contains("Files they changed (newest first): src/db.ts, src/api.ts"), "{text}");
        assert!(text.contains("do not rename, reformat or rewrite"), "{text}");
    }

    #[test]
    fn the_same_model_gets_no_hand_over_and_an_empty_folder_no_brief() {
        let earlier = [author("t1", "native_api", "qwen3.8-max")];

        assert_eq!(brief(&Facts::default(), &earlier, &[], "native_api", "qwen3.8-max"), None);
    }

    #[test]
    fn conventions_travel_with_every_turn() {
        let facts = Facts {
            style: vec![(".editorconfig".into(), "indent_style = space\nindent_size = 2".into())],
            stack: stack_of("package.json", r#"{"type":"module","dependencies":{"react":"19","typescript":"5"}}"#),
            rules: None,
        };
        let text = brief(&facts, &[], &[], "native_api", "deepseek-v4-pro").unwrap();

        assert!(text.contains("Project stack: React, TypeScript, ES modules."), "{text}");
        assert!(text.contains("indent_size = 2"), "{text}");
        assert!(!text.contains("Hand-over"), "{text}");
    }

    #[test]
    fn a_rules_file_the_cli_reads_itself_is_not_sent_twice() {
        assert!(read_by_engine("claude_code", "CLAUDE.md"));
        assert!(!read_by_engine("native_api", "CLAUDE.md"));
    }
}
