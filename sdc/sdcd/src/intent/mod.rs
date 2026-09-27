//! **The Universal Intent Engine** (0.12, docs/MASTER-PLAN-v3-TRUST-KERNEL.md).
//!
//! Whatever language, dialect, script or mix a person writes (or says) in, a request goes through four
//! steps before any engine touches a file:
//!
//!   1. **detect** (`detect`) - the script, the language, romanized or mixed writing and a regional form,
//!      offline, with a confidence;
//!   2. **understand** (`parse_prompt` → a model → `read_spec`) - a structured **Task Spec**: the target,
//!      the goal, 3–8 acceptance conditions, what not to do, each with a confidence, at most two
//!      questions in the person's own language, and a back-translation for a low-confidence reading;
//!   3. **confirm** - the person sees the Intent Contract card, ticks or edits it, and a correction
//!      becomes a glossary term the next reading uses;
//!   4. **compile** (`compile`) - one Task Spec, a different prompt per engine: Claude Code, Codex,
//!      Gemini and the SDC Agent each get the task, the acceptance criteria, the project's rules file,
//!      a map of the repository, the policy, the glossary, "what not to do", and - when the project has
//!      tests - the instruction to write the test for each condition first.
//!
//! The same message in Banglish, Sylheti, Hinglish, Arabizi or Spanish yields the same Task Spec; the
//! person's own words always travel verbatim at the end of the compiled prompt.

pub mod detect;
pub mod voice;

use serde_json::{json, Value};

pub use detect::{detect, Detection};

/// Questions the card may ask at most.
pub const MAX_QUESTIONS: usize = 2;
/// A field below this confidence is asked about or back-translated.
pub const CONFIDENT: f64 = 0.6;

/// What the compiler knows about the folder a task runs in.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// `local`, or the host's name.
    pub place: String,
    pub root: Option<String>,
    /// The site bound to the folder, when there is one: its name and URL.
    pub site: Option<(String, String)>,
    /// The project's own rules file (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.sdc/rules.md`), trimmed.
    pub rules: Option<(String, String)>,
    /// The long-task memory the project keeps (`.sdc/memory.md`): what survives a restart.
    pub memory: Option<String>,
    /// The folder's top level, as a short map.
    pub repo_map: Vec<String>,
    /// Whether the project has tests to write first.
    pub has_tests: bool,
    pub glossary: Vec<(String, String)>,
    pub policy: Option<crate::trust::policy::Policy>,
    /// The agency's style guide, written by the owner (Settings → Agency).
    pub style_guide: Option<String>,
    /// How the person wants to be answered: `standard` or `dialect`.
    pub reply_style: String,
}

/// The rules files an engine's own tooling reads, in the order SDC looks for them.
pub const RULES_FILES: &[&str] = &[".sdc/rules.md", "AGENTS.md", "CLAUDE.md", "GEMINI.md", ".cursorrules"];

/// Reads what the compiler needs from a folder on this machine.
pub fn gather_local(root: &std::path::Path) -> Context {
    let mut context = Context { root: Some(root.display().to_string()), place: "this machine".into(), ..Context::default() };

    for name in RULES_FILES {
        if let Ok(text) = std::fs::read_to_string(root.join(name)) {
            context.rules = Some((name.to_string(), text.chars().take(6000).collect()));
            break;
        }
    }

    context.memory = std::fs::read_to_string(root.join(".sdc/memory.md")).ok().map(|text| text.chars().take(4000).collect());

    if let Ok(entries) = std::fs::read_dir(root) {
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();

                if name.starts_with('.') && name != ".sdc" || ["node_modules", "target", "dist", "vendor", "__pycache__"].contains(&name.as_str()) {
                    return None;
                }

                Some(if entry.path().is_dir() { format!("{name}/") } else { name })
            })
            .collect();

        names.sort();
        context.has_tests = names.iter().any(|name| {
            let lowered = name.to_lowercase();

            lowered.starts_with("test") || lowered.starts_with("spec") || lowered == "__tests__/" || lowered == "phpunit.xml" || lowered == "pytest.ini"
        }) || std::fs::read_to_string(root.join("package.json")).map(|text| text.contains("\"test\"")).unwrap_or(false)
            || root.join("Cargo.toml").exists()
            || root.join("go.mod").exists();
        context.repo_map = names.into_iter().take(60).collect();
    }

    context.policy = Some(crate::trust::policy::Policy::load_local(root));

    context
}

/// The same for a folder on a host, in one `ssh` round trip.
pub fn gather_remote(ssh: &crate::ssh::Ssh, root: &str) -> Context {
    let mut context = Context { root: Some(root.to_string()), place: ssh.label(), ..Context::default() };
    let Ok(quoted) = crate::ssh::ops::remote_expr(root) else {
        return context;
    };
    let rules = RULES_FILES.iter().map(|name| format!("[ -f {quoted}/{name} ] && {{ echo '@@RULES {name}'; head -c 6000 {quoted}/{name}; echo; echo '@@END'; }}")).collect::<Vec<_>>().join(" || ");
    let script = format!(
        "cd {quoted} 2>/dev/null || exit 0; echo '@@MAP'; ls -1Ap | grep -v -E '^(node_modules|vendor|target|dist)/$' | head -60; echo '@@END'; {{ {rules}; }}; [ -f .sdc/memory.md ] && {{ echo '@@MEMORY'; head -c 4000 .sdc/memory.md; echo; echo '@@END'; }}; true"
    );

    if let Ok(output) = ssh.run(&script, crate::ssh::QUICK) {
        let mut section: Option<String> = None;
        let mut body = String::new();

        for line in output.stdout.lines() {
            if let Some(rest) = line.strip_prefix("@@") {
                if rest == "END" {
                    match section.as_deref() {
                        Some("MAP") => context.repo_map = body.lines().filter(|name| !name.trim().is_empty()).map(str::to_string).collect(),
                        Some("MEMORY") => context.memory = Some(body.clone()),
                        Some(name) if name.starts_with("RULES ") => context.rules = Some((name.trim_start_matches("RULES ").to_string(), body.clone())),
                        _ => {}
                    }

                    section = None;
                    body.clear();
                } else {
                    section = Some(rest.to_string());
                }

                continue;
            }

            if section.is_some() {
                body.push_str(line);
                body.push('\n');
            }
        }
    }

    context.has_tests = context.repo_map.iter().any(|name| {
        let lowered = name.to_lowercase();

        lowered.starts_with("test") || lowered.starts_with("spec") || lowered == "phpunit.xml" || lowered == "cargo.toml" || lowered == "go.mod"
    });
    context.policy = Some(crate::trust::policy::Policy::load(Some(root), Some(ssh)));

    context
}

/// The glossary terms a message uses, as `term → meaning` lines.
fn glossary_lines(text: &str, glossary: &[(String, String)]) -> Vec<String> {
    let lowered = text.to_lowercase();

    glossary
        .iter()
        .filter(|(term, _)| lowered.contains(term.as_str()))
        .map(|(term, meaning)| format!("\"{term}\" means \"{meaning}\""))
        .collect()
}

/// The instruction a model gets to turn a message into a Task Spec - JSON only, in a fixed shape.
pub fn parse_prompt(text: &str, detection: &Detection, context: &Context) -> String {
    let target = match (&context.site, &context.root) {
        (Some((name, url)), _) => format!("the site {name} ({url}), folder {} on {}", context.root.clone().unwrap_or_default(), context.place),
        (None, Some(root)) => format!("the folder {root} on {}", context.place),
        _ => "no folder is open yet".to_string(),
    };
    let glossary = glossary_lines(text, &context.glossary);
    let reply_style = if context.reply_style == "dialect" && detection.dialect.is_some() {
        "in the same regional form the person used"
    } else {
        "in the standard written form of that language"
    };

    format!(
        "You turn a person's request to a coding assistant into a precise task specification. You do not do the task.\n\
         \n\
         The request may be in any language, dialect or script, romanized (Banglish, Hinglish, Arabizi, Roman Urdu), \
         mixed with English, misspelled, or spoken. An offline detector's first look: {label} (code {code}, script {script}{dialect}, confidence {confidence:.2}). \
         Correct it if it is wrong.\n\
         \n\
         Where the work happens: {target}.\n\
         {glossary_block}\
         \n\
         Answer with ONLY one JSON object, no prose, in exactly this shape:\n\
         {{\n\
           \"language\": {{\"code\": \"bn\", \"dialect\": \"sylheti or null\", \"script\": \"Beng\", \"label\": \"Sylheti (Bengali script)\", \"romanized\": false, \"mixed\": true}},\n\
           \"kind\": \"fix | build | change | explain | deploy | review | question | other\",\n\
           \"target\": {{\"value\": \"what the work is on: a site, a page, a file, a feature\", \"confidence\": 0.0}},\n\
           \"goal\": {{\"value\": \"the outcome the person wants, one sentence, in English\", \"confidence\": 0.0}},\n\
           \"acceptance\": [\"3 to 8 checkable conditions, in English, each one observable (a test, a page, a message)\"],\n\
           \"acceptanceConfidence\": 0.0,\n\
           \"outOfScope\": [\"what must not be changed or done\"],\n\
           \"risk\": \"low | medium | high\",\n\
           \"questions\": [\"at most 2 short questions, in the person's language {reply_style}, only when a wrong guess would be costly\"],\n\
           \"summary\": \"the request restated in one or two sentences, in the person's language {reply_style}, spoken to them as 'you'\",\n\
           \"backTranslation\": \"the goal and the conditions, in the person's language {reply_style}, so they can say 'yes, that is it'\",\n\
           \"glossary\": [{{\"term\": \"a word of theirs\", \"meaning\": \"what it means here\"}}]\n\
         }}\n\
         Confidence is 0.0-1.0: how sure you are that this is what the person meant. Keep code, commands, file paths and names exactly as written.\n\
         \n\
         The request:\n{text}",
        label = detection.label,
        code = detection.code,
        script = detection.script,
        dialect = detection.dialect.as_deref().map(|dialect| format!(", dialect {dialect}")).unwrap_or_default(),
        confidence = detection.confidence,
        glossary_block = if glossary.is_empty() {
            String::new()
        } else {
            format!("Words this person has explained before: {}.\n", glossary.join("; "))
        },
    )
}

fn confidence(value: &Value) -> f64 {
    value.as_f64().unwrap_or(0.5).clamp(0.0, 1.0)
}

fn strings(value: &Value, limit: usize) -> Vec<String> {
    value
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.as_str().or_else(|| item["text"].as_str()).map(|text| text.trim().to_string()))
        .filter(|text| !text.is_empty())
        .take(limit)
        .collect()
}

/// A model's answer as a Task Spec - normalised, clamped to the card's limits, and with the flags the
/// card needs: which fields are unsure, and whether to show the back-translation.
pub fn read_spec(answer: &str, detection: &Detection) -> Option<Value> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    let raw: Value = serde_json::from_str(answer.get(start..=end)?).ok()?;

    if raw.get("goal").is_none() && raw.get("acceptance").is_none() {
        return None;
    }

    Some(normalise(&raw, detection, "model"))
}

/// Any spec-shaped value (a model's, or the person's edited card) in the one shape the rest of SDC reads.
pub fn normalise(raw: &Value, detection: &Detection, source: &str) -> Value {
    let language = raw.get("language").filter(|language| language.is_object()).cloned().unwrap_or_else(|| detection.to_json());
    let target_confidence = confidence(&raw["target"]["confidence"]);
    let goal_confidence = confidence(&raw["goal"]["confidence"]);
    let acceptance: Vec<Value> = raw["acceptance"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|item| {
            let text = item.as_str().or_else(|| item["text"].as_str())?.trim().to_string();

            (!text.is_empty()).then(|| json!({ "text": text, "checked": item["checked"].as_bool().unwrap_or(true) }))
        })
        .take(8)
        .collect();
    let acceptance_confidence = confidence(&raw["acceptanceConfidence"]);
    let lowest = target_confidence.min(goal_confidence).min(if acceptance.is_empty() { 0.3 } else { acceptance_confidence });
    let low_resource = !matches!(language["code"].as_str(), Some("en" | "es" | "fr" | "de" | "pt" | "zh" | "ja" | "ru" | "hi" | "bn" | "ar"))
        || language["dialect"].as_str().is_some_and(|dialect| !dialect.is_empty() && dialect != "null");
    let questions = strings(&raw["questions"], MAX_QUESTIONS);
    let unsure: Vec<&str> = [("target", target_confidence), ("goal", goal_confidence), ("acceptance", acceptance_confidence)]
        .iter()
        .filter(|(_, value)| *value < CONFIDENT)
        .map(|(name, _)| *name)
        .collect();

    json!({
        "language": language,
        "kind": raw["kind"].as_str().unwrap_or("other"),
        "target": { "value": raw["target"]["value"].as_str().unwrap_or_default(), "confidence": target_confidence },
        "goal": { "value": raw["goal"]["value"].as_str().unwrap_or_default(), "confidence": goal_confidence },
        "acceptance": acceptance,
        "acceptanceConfidence": acceptance_confidence,
        "outOfScope": strings(&raw["outOfScope"], 8),
        "risk": match raw["risk"].as_str() { Some("high") => "high", Some("medium") => "medium", _ => "low" },
        "questions": if lowest < CONFIDENT { json!(questions) } else { json!(questions.into_iter().take(1).collect::<Vec<_>>()) },
        "summary": raw["summary"].as_str().unwrap_or_default(),
        "backTranslation": raw["backTranslation"].as_str().unwrap_or_default(),
        "showBackTranslation": low_resource || lowest < CONFIDENT,
        "unsure": unsure,
        "confidence": (lowest * 100.0).round() / 100.0,
        "glossary": raw["glossary"].as_array().cloned().unwrap_or_default(),
        "source": source,
    })
}

/// A Task Spec with no model to ask: the person's words as the goal, conditions any change must meet, a
/// low confidence, and the back-translation on - so the card asks rather than pretends.
pub fn heuristic(text: &str, detection: &Detection, context: &Context) -> Value {
    let target = match (&context.site, &context.root) {
        (Some((name, url)), _) => format!("{name} ({url})"),
        (None, Some(root)) => root.clone(),
        _ => "not chosen yet".to_string(),
    };
    let mut acceptance = vec!["The requested change works when it is tried the way the person described".to_string()];

    if context.has_tests {
        acceptance.push("The project's own tests and checks still pass".to_string());
    }

    acceptance.push("Nothing outside the request is changed".to_string());

    normalise(
        &json!({
            "language": detection.to_json(),
            "kind": "other",
            "target": { "value": target, "confidence": if context.root.is_some() { 0.6 } else { 0.3 } },
            "goal": { "value": text.trim(), "confidence": 0.4 },
            "acceptance": acceptance,
            "acceptanceConfidence": 0.4,
            "outOfScope": ["Protected files (.env, keys, wp-config.php, backups) unless asked"],
            "risk": "low",
            "questions": [],
            "summary": text.trim(),
            "backTranslation": "",
        }),
        detection,
        "heuristic",
    )
}

/// The engine families the compiler writes for.
fn family(engine: &str) -> &'static str {
    match engine {
        "claude_code" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        _ => "agent",
    }
}

/// **The Prompt Compiler**: one Task Spec, the prompt one engine works best with. The person's own words
/// end every version, verbatim.
pub fn compile(engine: &str, spec: &Value, original: &str, context: &Context) -> String {
    let family = family(engine);
    let reply = spec["language"]["label"].as_str().unwrap_or("the person's language");
    let reply_in = spec["language"]["code"]
        .as_str()
        .map(detect::detect_label_reply)
        .unwrap_or_else(|| "the language the person wrote in".to_string());
    let dialect_note = if context.reply_style == "dialect" {
        spec["language"]["dialect"].as_str().filter(|dialect| !dialect.is_empty() && *dialect != "null").map(|dialect| format!(" (in the {dialect} regional form, as they wrote)")).unwrap_or_default()
    } else {
        String::new()
    };
    let acceptance: Vec<String> = spec["acceptance"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|item| item["checked"].as_bool().unwrap_or(true))
        .filter_map(|item| item["text"].as_str().map(str::to_string))
        .collect();
    let out_of_scope = strings(&spec["outOfScope"], 12);
    let mut text = String::new();
    let section = |title: &str| match family {
        "claude" | "gemini" => format!("\n## {title}\n"),
        "codex" => format!("\n{}:\n", title.to_uppercase()),
        _ => format!("\n### {title}\n"),
    };

    text.push_str(&format!(
        "[Task compiled by SDC from the person's confirmed request - {reply}]\n{}\n",
        spec["goal"]["value"].as_str().unwrap_or(original)
    ));
    text.push_str(&section("Where"));
    text.push_str(&format!(
        "{}{}\n",
        spec["target"]["value"].as_str().filter(|value| !value.is_empty()).unwrap_or("the open folder"),
        context.root.as_deref().map(|root| format!(" - folder {root} on {}", context.place)).unwrap_or_default()
    ));

    if !acceptance.is_empty() {
        text.push_str(&section("Done means (acceptance criteria)"));

        for (index, condition) in acceptance.iter().enumerate() {
            text.push_str(&format!("{}. {condition}\n", index + 1));
        }

        if context.has_tests {
            text.push_str("Contract first: where the project's test setup allows, write or extend a test for each condition BEFORE changing the code, run it to see it fail, then make it pass.\n");
        }
    }

    text.push_str(&section("Do not"));

    for item in &out_of_scope {
        text.push_str(&format!("- {item}\n"));
    }

    if let Some(policy) = &context.policy {
        text.push_str(&format!(
            "- Do not touch protected paths: {}.\n",
            policy.protected_paths.iter().take(12).cloned().collect::<Vec<_>>().join(", ")
        ));

        if policy.production {
            text.push_str("- This is PRODUCTION: no destructive commands, no pushes or deploys, keep changes minimal.\n");
        }

        if policy.max_files_per_turn > 0 {
            text.push_str(&format!("- Change at most {} files in this turn; stop and explain if more are needed.\n", policy.max_files_per_turn));
        }
    }

    text.push_str("- Do not guess on anything costly (deleting data, deploying, spending money): ask instead.\n");

    /* The CLIs read their own rules files; SDC's agent and the plain API models are handed them. */
    if let Some((name, rules)) = &context.rules {
        match family {
            "claude" if name == "CLAUDE.md" => {}
            "codex" if name == "AGENTS.md" => {}
            "gemini" if name == "GEMINI.md" => {}
            _ => {
                text.push_str(&section(&format!("Project rules ({name})")));
                text.push_str(rules.trim());
                text.push('\n');
            }
        }
    }

    if let Some(memory) = &context.memory {
        text.push_str(&section("What this project remembers (.sdc/memory.md)"));
        text.push_str(memory.trim());
        text.push('\n');
    }

    if let Some(guide) = context.style_guide.as_deref().filter(|guide| !guide.trim().is_empty()) {
        text.push_str(&section("Agency style guide"));
        text.push_str(guide.trim());
        text.push('\n');
    }

    if family == "agent" && !context.repo_map.is_empty() {
        text.push_str(&section("Repository map (top level)"));
        text.push_str(&context.repo_map.join("  "));
        text.push('\n');
    }

    let glossary = glossary_lines(original, &context.glossary);

    if !glossary.is_empty() {
        text.push_str(&section("The person's words"));
        text.push_str(&glossary.join("; "));
        text.push('\n');
    }

    text.push_str(&section("Language"));
    text.push_str(&format!(
        "Start your answer with exactly one line: `{} ` followed by the task restated in {reply_in}{dialect_note}, spoken to the person as \"you\". \
         Write the rest of the answer in {reply_in} too. Code, comments, commit messages, branch names, commands and file paths stay in English, exactly as the project writes them.\n",
        crate::understand::UNDERSTOOD
    ));
    text.push_str(&section("The person's own words (verbatim)"));
    text.push_str(original.trim());

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        Context {
            place: "this machine".into(),
            root: Some("/srv/shop".into()),
            site: Some(("Shop".into(), "https://shop.test".into())),
            rules: Some(("AGENTS.md".into(), "Use pnpm. Never edit generated files.".into())),
            memory: Some("The contact form posts to /api/contact.".into()),
            repo_map: vec!["src/".into(), "tests/".into(), "package.json".into()],
            has_tests: true,
            glossary: vec![("ghor".into(), "page".into())],
            policy: Some(crate::trust::policy::Policy::default()),
            style_guide: Some("Keep functions under 40 lines.".into()),
            reply_style: "standard".into(),
        }
    }

    const ANSWER: &str = r#"Sure: {"language":{"code":"bn","dialect":"sylheti","script":"Beng","label":"Sylheti (Bengali script)","romanized":false,"mixed":true},
        "kind":"fix","target":{"value":"the contact form on Shop","confidence":0.9},"goal":{"value":"Make the contact form send its email","confidence":0.85},
        "acceptance":["Submitting the form sends an email","A success message is shown","A failed send shows an error"],"acceptanceConfidence":0.8,
        "outOfScope":["Do not change the page design"],"risk":"low","questions":["q1","q2","q3"],"summary":"আপনি চান…","backTranslation":"আমি বুঝেছি…","glossary":[]}"#;

    #[test]
    fn a_model_answer_becomes_a_normalised_spec() {
        let spec = read_spec(ANSWER, &detect("ই সাইটর contact form খান কাম করর না")).unwrap();

        assert_eq!(spec["acceptance"].as_array().unwrap().len(), 3);
        assert_eq!(spec["questions"].as_array().unwrap().len(), 1, "a confident reading asks at most one question");
        assert_eq!(spec["showBackTranslation"], true, "a dialect is low-resource: show the back-translation");
        assert!(spec["unsure"].as_array().unwrap().is_empty());
        assert!(read_spec("no json here", &detect("x")).is_none());
    }

    #[test]
    fn low_confidence_keeps_up_to_two_questions() {
        let answer = ANSWER.replace("\"confidence\":0.85", "\"confidence\":0.3");
        let spec = read_spec(&answer, &detect("x")).unwrap();

        assert_eq!(spec["questions"].as_array().unwrap().len(), 2);
        assert_eq!(spec["unsure"][0], "goal");
    }

    #[test]
    fn the_compiler_writes_a_different_prompt_per_engine_and_keeps_the_words() {
        let spec = read_spec(ANSWER, &detect("x")).unwrap();
        let original = "ই সাইটর contact form খান কাম করর না, বাইক্কা কইরা দাও";
        let claude = compile("claude_code", &spec, original, &context());
        let codex = compile("codex", &spec, original, &context());
        let agent = compile("native_api", &spec, original, &context());

        for prompt in [&claude, &codex, &agent] {
            assert!(prompt.ends_with(original), "verbatim words last");
            assert!(prompt.contains("Submitting the form sends an email"));
            assert!(prompt.contains("Contract first"));
            assert!(prompt.contains("Understood:"));
            assert!(prompt.contains("wp-config.php"), "the policy's protected paths");
        }

        assert!(claude.contains("## Done means"));
        assert!(codex.contains("DONE MEANS"));
        assert!(!codex.contains("Use pnpm"), "Codex reads AGENTS.md itself");
        assert!(agent.contains("Use pnpm"), "the agent is handed the rules file");
        assert!(agent.contains("Repository map"));
        assert!(agent.contains("Keep functions under 40 lines"));
    }

    #[test]
    fn with_no_model_the_heuristic_spec_asks_rather_than_pretends() {
        let spec = heuristic("ei site er contact form ta kaj kortese na", &detect("ei site er contact form ta kaj kortese na"), &context());

        assert_eq!(spec["source"], "heuristic");
        assert_eq!(spec["showBackTranslation"], true);
        assert!(spec["target"]["value"].as_str().unwrap().contains("Shop"));
        assert!(spec["confidence"].as_f64().unwrap() < CONFIDENT);
    }

    #[test]
    fn the_parse_prompt_names_the_target_and_the_glossary() {
        let prompt = parse_prompt("ghor ta thik koro", &detect("ghor ta thik koro"), &context());

        assert!(prompt.contains("https://shop.test"));
        assert!(prompt.contains("\"ghor\" means \"page\""));
        assert!(prompt.contains("ONLY one JSON object"));
    }
}
