//! **Model Profile + Capability Probe (P2-1, switch `padma.model_profile`)**: any small model goes through one adapter, and what is special about
//! it is *measured*, not guessed. A profile is a TOML file per model - `<data folder>/models/<model>.toml` - with how the model makes tool calls,
//! whether it should think for each kind of step, its sampling, its real window, its quantization and how many tool calls it reliably makes in one
//! reply. Where the person has no file yet, a built-in starting profile for the model's family is used (`profiles/*.toml`, matched by name prefix
//! *in that data*, so no model name is written in code that runs a task), and if there is none either, a neutral one.
//!
//! The probe is generic over the same [`Runner`] the Capability Probe (P0-9) uses, so what it asks and how it scores is proven with a scripted
//! model; `sdcd models probe MODEL` runs it against a live Ollama (OWNER-LATER: not run on the build machine).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::dialect::ToolSpec;
use crate::agent::rescue;
use crate::sme::probe::{Ask, Runner};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sampling {
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: u32,
    pub presence_penalty: f64,
}

impl Default for Sampling {
    fn default() -> Self {
        Self { temperature: 0.6, top_p: 0.95, top_k: 40, presence_penalty: 0.0 }
    }
}

/// Whether the model thinks, for each kind of step (P2-2 reads this).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Thinking {
    pub tool_call: bool,
    pub map_review: bool,
    pub boilerplate: bool,
    pub blueprint: bool,
    pub repair_after_fail: bool,
    pub reduce_report: bool,
}

impl Default for Thinking {
    fn default() -> Self {
        Self { tool_call: false, map_review: false, boilerplate: false, blueprint: true, repair_after_fail: true, reduce_report: true }
    }
}

impl Thinking {
    /// The setting for a step kind; an unknown kind does not think.
    pub fn for_kind(&self, kind: &str) -> bool {
        match kind {
            "tool_call" => self.tool_call,
            "map_review" => self.map_review,
            "boilerplate" => self.boilerplate,
            "blueprint" => self.blueprint,
            "repair_after_fail" => self.repair_after_fail,
            "reduce_report" => self.reduce_report,
            _ => false,
        }
    }
}

/// What a probe measured. Absent (`None`) until a probe has run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measured {
    /// Share of the tool tasks that ended in a valid call, in any form.
    pub tool_call_success: f64,
    /// Share of the tool tasks whose valid call was printed as text instead of made.
    pub text_tool_call_rate: f64,
    /// Share of all the probe's questions that came back empty.
    pub empty_reply_rate: f64,
    pub probed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelProfile {
    pub model: String,
    /// Name prefixes (lower case) a built-in profile stands for. Unused in a stored profile.
    pub matches: Vec<String>,
    /// `native`, `hermes-json` or `qwen3-coder-xml`.
    pub tool_format: String,
    pub thinking: Thinking,
    pub sampling: Sampling,
    pub effective_ctx: u64,
    pub quant: String,
    pub max_tools_per_call: u32,
    pub measured: Option<Measured>,
}

impl Default for ModelProfile {
    fn default() -> Self {
        Self { model: String::new(), matches: Vec::new(), tool_format: "native".into(), thinking: Thinking::default(), sampling: Sampling::default(), effective_ctx: 0, quant: String::new(), max_tools_per_call: 1, measured: None }
    }
}

const BUILT_IN: [&str; 2] = [include_str!("profiles/qwen3.5.toml"), include_str!("profiles/llama3.toml")];

pub const FORMATS: [&str; 3] = ["native", "hermes-json", "qwen3-coder-xml"];

fn built_in(model: &str) -> Option<ModelProfile> {
    let name = model.to_lowercase();

    BUILT_IN
        .iter()
        .filter_map(|text| toml::from_str::<ModelProfile>(text).ok())
        .filter_map(|profile| profile.matches.iter().filter(|prefix| name.starts_with(prefix.as_str())).map(|prefix| prefix.len()).max().map(|longest| (longest, profile)))
        .max_by_key(|(longest, _)| *longest)
        .map(|(_, profile)| profile)
}

fn folder() -> Option<PathBuf> {
    crate::paths::data_dir().ok().map(|dir| dir.join("models"))
}

fn file_name(model: &str) -> String {
    let stem: String = model.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();

    format!("{stem}.toml")
}

/// The profile a model has now: its stored file, else the built-in one for its family, else a neutral one. The window is filled from the
/// Capability Probe (P0-9) when the profile does not know it.
pub fn profile_for(model: &str) -> ModelProfile {
    let stored = folder().and_then(|dir| std::fs::read_to_string(dir.join(file_name(model))).ok()).and_then(|text| toml::from_str::<ModelProfile>(&text).ok());
    let mut profile = stored.or_else(|| built_in(model)).unwrap_or_default();

    profile.model = model.to_string();

    if profile.effective_ctx == 0 {
        profile.effective_ctx = crate::sme::probe::latest("ollama", model).map(|measured| measured.effective_context).unwrap_or(0);
    }

    profile
}

/// Stores a profile as `<data folder>/models/<model>.toml`.
pub fn save(profile: &ModelProfile) -> Result<PathBuf, String> {
    let dir = folder().ok_or("no data directory")?;

    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;

    let path = dir.join(file_name(&profile.model));
    let text = toml::to_string_pretty(profile).map_err(|error| error.to_string())?;

    std::fs::write(&path, text).map_err(|error| error.to_string())?;

    Ok(path)
}

/// One line for Settings -> Models.
pub fn summary(profile: &ModelProfile) -> String {
    let tools = match profile.tool_format.as_str() {
        "native" => "native tool calls",
        "hermes-json" => "tool calls as JSON text",
        "qwen3-coder-xml" => "tool calls as XML text",
        other => other,
    };
    let window = if profile.effective_ctx == 0 { "window unknown".to_string() } else { format!("{}k window", profile.effective_ctx / 1024) };
    let measured = match &profile.measured {
        Some(measured) => format!("{}% of tool calls valid, {}% empty replies", (measured.tool_call_success * 100.0).round(), (measured.empty_reply_rate * 100.0).round()),
        None => "not probed yet".to_string(),
    };

    format!("{tools} · {window} · {measured}")
}

/// Whether the profiles are in use at all (read-only: nothing about a turn changes because of this).
pub fn active() -> bool {
    crate::v3::on("padma.model_profile")
}

/// Whether this model already has a profile file of its own.
pub fn stored(model: &str) -> bool {
    folder().is_some_and(|dir| dir.join(file_name(model)).exists())
}

/// The one-line summary a model row carries for Settings -> Models, or `None` when the profiles are off.
pub fn row_summary(model: &str) -> Option<String> {
    active().then(|| summary(&profile_for(model)))
}

/// After a turn on a local model that has no profile file: measures it in the background (called from the Capability Probe's own after-use thread).
pub fn probe_if_missing(model: &str) {
    if !active() || stored(model) {
        return;
    }

    let mut runner = crate::sme::ollama_probe::OllamaRunner { model: model.to_string() };

    match probe(&mut runner, model, &chrono::Utc::now().to_rfc3339()) {
        Ok(profile) => {
            if let Err(error) = save(&profile) {
                eprintln!("sdcd: the profile of {model} could not be saved: {error}");
            }
        }
        Err(error) => eprintln!("sdcd: the profile probe of {model} failed: {error}"),
    }
}

fn add_spec() -> ToolSpec {
    ToolSpec {
        name: "add",
        description: "Adds two integers.",
        schema: json!({ "type": "object", "properties": { "a": { "type": "integer" }, "b": { "type": "integer" } }, "required": ["a", "b"] }),
    }
}

/// How a reply made its tool calls: `(valid calls for the numbers asked, form)`; the form is the profile's `tool_format` word.
fn calls_in(reply: &crate::sme::probe::Reply, wanted: &[(i64, i64)]) -> (usize, Option<&'static str>) {
    let right = |args: &Value| wanted.iter().any(|(a, b)| args["a"].as_i64() == Some(*a) && args["b"].as_i64() == Some(*b));
    let native = reply.tool_calls.iter().filter(|(name, args)| name == "add" && right(args)).count();

    if native > 0 {
        return (native, Some("native"));
    }

    let found = rescue::rescue_with(&reply.text, &[add_spec()], true);
    let text = found.calls.iter().filter(|call| call.name == "add" && right(&call.input)).count();

    if text == 0 {
        return (0, None);
    }

    (text, Some(if reply.text.contains("<function=") { "qwen3-coder-xml" } else { "hermes-json" }))
}

/// Runs the probe: 5 single-tool tasks, 3 questions that need no tool, 2 tasks that need two calls in one reply. Scores the tool-call success,
/// how many of the valid calls were printed as text, the share of empty replies, the best tool format and how many calls one reply holds.
pub fn probe(runner: &mut dyn Runner, model: &str, now: &str) -> Result<ModelProfile, String> {
    let key = runner.identity()?;
    let mut profile = built_in(model).unwrap_or_default();
    let ask = |prompt: String, tool: bool, seed: u64| Ask {
        prompt,
        tool: if tool { Some(("add".to_string(), add_spec().schema)) } else { None },
        schema: None,
        num_ctx: 4_096,
        max_output: 256,
        seed,
        think: Some(false),
    };
    let (mut valid, mut printed, mut empty, mut asked) = (0u32, 0u32, 0u32, 0u32);
    let mut forms: Vec<&'static str> = Vec::new();
    let mut note = |reply: &crate::sme::probe::Reply| {
        asked += 1;

        if reply.text.trim().is_empty() && reply.tool_calls.is_empty() {
            empty += 1;
        }
    };

    for task in 0..5u64 {
        let (a, b) = (3 + task as i64, 40 + 2 * task as i64);

        if let Ok(reply) = runner.ask(&ask(format!("Use the add tool to add {a} and {b}."), true, 11 + task)) {
            note(&reply);

            if let (1.., Some(form)) = calls_in(&reply, &[(a, b)]) {
                valid += 1;
                forms.push(form);

                if form != "native" {
                    printed += 1;
                }
            }
        } else {
            note(&Default::default());
        }
    }

    for question in ["What is the capital of France?", "Name the three primary colours.", "Say hello in one short sentence."] {
        match runner.ask(&ask(question.to_string(), false, 21)) {
            Ok(reply) => note(&reply),
            Err(_) => note(&Default::default()),
        }
    }

    let mut together = 0u32;

    for task in 0..2u64 {
        let (a, b, c, d) = (2 + task as i64, 7 + task as i64, 11 + task as i64, 20 + task as i64);

        if let Ok(reply) = runner.ask(&ask(format!("Use the add tool twice in your reply: once for {a} and {b}, once for {c} and {d}."), true, 31 + task)) {
            note(&reply);

            let (count, form) = calls_in(&reply, &[(a, b), (c, d)]);

            if count >= 2 {
                together += 1;
            }

            if let (1.., Some(form)) = (count, form) {
                forms.push(form);
            }
        } else {
            note(&Default::default());
        }
    }

    let tool_tasks = 5.0;
    let best = FORMATS.iter().max_by_key(|format| forms.iter().filter(|form| form == format).count()).filter(|format| forms.iter().any(|form| form == *format));

    if let Some(format) = best {
        profile.tool_format = (*format).to_string();
    }

    profile.model = model.to_string();
    profile.quant = key.quantization.clone();
    profile.max_tools_per_call = if together == 2 { 4 } else if together == 1 { 2 } else { 1 };

    if let Some(window) = crate::sme::probe::latest("ollama", model).map(|measured| measured.effective_context).filter(|window| *window > 0) {
        profile.effective_ctx = window;
    }

    profile.measured = Some(Measured {
        tool_call_success: f64::from(valid) / tool_tasks,
        text_tool_call_rate: f64::from(printed) / tool_tasks,
        empty_reply_rate: f64::from(empty) / f64::from(asked.max(1)),
        probed_at: now.to_string(),
    });

    Ok(profile)
}

/// `sdcd models probe MODEL` and `sdcd models show MODEL`.
pub fn main(args: &[String]) -> i32 {
    let (Some(verb), Some(model)) = (args.first(), args.get(1).filter(|arg| !arg.starts_with("--"))) else {
        eprintln!("usage: sdcd models probe MODEL | sdcd models show MODEL   (MODEL is an installed Ollama model, e.g. qwen3.5:9b)");

        return 3;
    };

    match verb.as_str() {
        "show" => {
            let profile = profile_for(model);

            println!("{}", summary(&profile));
            println!("{}", toml::to_string_pretty(&profile).unwrap_or_default());

            0
        }
        "probe" => {
            let mut runner = crate::sme::ollama_probe::OllamaRunner { model: model.clone() };

            match probe(&mut runner, model, &chrono::Utc::now().to_rfc3339()) {
                Ok(profile) => {
                    match save(&profile) {
                        Ok(path) => println!("saved {}", path.display()),
                        Err(error) => eprintln!("could not save the profile: {error}"),
                    }

                    println!("{}", summary(&profile));

                    0
                }
                Err(error) => {
                    eprintln!("probe failed: {error}");

                    1
                }
            }
        }
        _ => {
            eprintln!("usage: sdcd models probe MODEL | sdcd models show MODEL");

            3
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sme::probe::{ModelKey, Reply};

    /// A scripted model: how it makes tool calls, and whether it sometimes answers with nothing.
    struct Scripted {
        form: &'static str,
        dumb: bool,
    }

    impl Runner for Scripted {
        fn identity(&self) -> Result<ModelKey, String> {
            Ok(ModelKey { provider: "ollama".into(), model: "fixture".into(), digest: "abc".into(), quantization: "Q4_K_M".into(), runtime: "0".into() })
        }

        fn ask(&mut self, ask: &Ask) -> Result<Reply, String> {
            let Some(_) = ask.tool else {
                return Ok(Reply { text: if self.dumb { String::new() } else { "An answer.".into() }, ..Default::default() });
            };

            let numbers: Vec<i64> = ask.prompt.split(|c: char| !c.is_ascii_digit()).filter_map(|word| word.parse().ok()).collect();
            let pairs: Vec<(i64, i64)> = numbers.chunks(2).filter(|pair| pair.len() == 2).map(|pair| (pair[0], pair[1])).collect();
            let pairs = if self.dumb { pairs.into_iter().take(1).collect() } else { pairs };

            Ok(match self.form {
                "native" => Reply { tool_calls: pairs.iter().map(|(a, b)| ("add".to_string(), json!({ "a": a, "b": b }))).collect(), ..Default::default() },
                "hermes-json" => Reply { text: pairs.iter().map(|(a, b)| format!("<tool_call>{{\"name\": \"add\", \"arguments\": {{\"a\": {a}, \"b\": {b}}}}}</tool_call>")).collect::<Vec<_>>().join("\n"), ..Default::default() },
                _ => Reply { text: pairs.iter().map(|(a, b)| format!("<tool_call><function=add><parameter=a>{a}</parameter><parameter=b>{b}</parameter></function></tool_call>")).collect::<Vec<_>>().join("\n"), ..Default::default() },
            })
        }
    }

    #[test]
    fn built_in_profiles_load_for_the_qwen_family_and_one_other_model() {
        let qwen = built_in("qwen3.5:9b").expect("a Qwen3.5 profile");

        assert_eq!(qwen.tool_format, "hermes-json");
        assert!(!qwen.thinking.for_kind("tool_call") && qwen.thinking.for_kind("blueprint"));
        assert_eq!(qwen.sampling.top_k, 20);

        let other = built_in("llama3.1:8b").expect("a second family");

        assert_eq!(other.tool_format, "native");
        assert!(built_in("some-unknown-model").is_none());
    }

    #[test]
    fn a_model_with_no_file_and_no_family_gets_a_neutral_profile() {
        let profile = profile_for("nothing-like-this-model:1b");

        assert_eq!(profile.model, "nothing-like-this-model:1b");
        assert_eq!(profile.tool_format, "native");
        assert!(summary(&profile).contains("not probed yet"));
    }

    #[test]
    fn the_probe_picks_the_format_the_model_really_uses() {
        for form in FORMATS {
            let profile = probe(&mut Scripted { form, dumb: false }, "fixture-model", "2026-01-01T00:00:00Z").unwrap();
            let measured = profile.measured.as_ref().unwrap();

            assert_eq!(profile.tool_format, form);
            assert!((measured.tool_call_success - 1.0).abs() < 1e-9, "{form}: {measured:?}");
            assert_eq!(profile.max_tools_per_call, 4, "{form}: two calls in one reply");
            assert_eq!(profile.quant, "Q4_K_M");
            assert!(if form == "native" { measured.text_tool_call_rate == 0.0 } else { (measured.text_tool_call_rate - 1.0).abs() < 1e-9 }, "{form}");
            assert_eq!(measured.empty_reply_rate, 0.0);
        }
    }

    #[test]
    fn a_model_that_makes_one_call_and_answers_nothing_is_scored_as_that() {
        let profile = probe(&mut Scripted { form: "native", dumb: true }, "fixture-model", "t").unwrap();
        let measured = profile.measured.unwrap();

        assert_eq!(profile.max_tools_per_call, 1);
        assert!((measured.empty_reply_rate - 0.3).abs() < 1e-9, "the three plain questions came back empty: {measured:?}");
    }

    #[test]
    fn a_stored_profile_wins_and_the_summary_is_one_line() {
        let _lock = crate::sme::probe::DATA_DIR_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut profile = probe(&mut Scripted { form: "hermes-json", dumb: false }, "qwen3.5:9b", "t").unwrap();

        profile.effective_ctx = 12_288;
        save(&profile).unwrap();

        let loaded = profile_for("qwen3.5:9b");

        assert_eq!(loaded.effective_ctx, 12_288);
        assert!(loaded.measured.is_some());

        let line = summary(&loaded);

        assert!(!line.contains('\n') && line.contains("12k window") && line.contains("100% of tool calls valid"), "{line}");
    }

    #[test]
    fn a_model_row_carries_the_line_only_while_the_switch_is_on() {
        assert!(crate::v3::forcing(&["padma.model_profile"], || row_summary("qwen3.5:9b")).is_some_and(|line| line.contains("JSON text")));
        assert!(crate::v3::ablating(&["padma.model_profile"], || row_summary("qwen3.5:9b")).is_none());
    }

    #[test]
    fn the_conductor_and_recipe_code_names_no_model() {
        for file in ["task.rs", "gate.rs", "scope.rs", "ledger.rs", "bind.rs"] {
            let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/padma").join(file)).unwrap();
            let production = text.split("#[cfg(test)]").next().unwrap_or("").to_lowercase().replace("ollama", "");

            for name in ["qwen", "llama", "gemma", "mistral", "deepseek"] {
                assert!(!production.contains(name), "{file} names a model: {name}");
            }
        }
    }
}
