//! **Model identity and Capability Probe (P0-9, switch `models.probe`)**: what a model can really do, measured once and
//! kept under the name of the exact file it was measured on.
//!
//! A model *tag* is not an identity: `qwen3.5:9b` can be a different file next month, and a different runtime reads
//! the same file differently. A profile is keyed by `provider + digest + quantization + runtime version`
//! ([`ModelKey`]); when any of the four changes, the stored profile is *provisional* - still usable, but due to be
//! measured again.
//!
//! The probe is generic over a [`Runner`], so what it asks and how it scores is proven without a GPU: the tests use a
//! scripted model with known behaviour. [`OllamaRunner`] is the real one. Nothing the probe sends comes from the
//! person: every prompt is built from a seed and a fixed word list ([`Plan::prompt_text`]), never from a project, a
//! key or the environment, so a probe cannot leak anything and two runs with one seed ask identical questions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The four things a profile depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelKey {
    pub provider: String,
    /// The model's own name (`qwen3.5:9b`) - for people; it is *not* part of the identity.
    pub model: String,
    /// Ollama's content digest, or an API's version id.
    pub digest: String,
    pub quantization: String,
    pub runtime: String,
}

impl ModelKey {
    /// The identity: change any of these and it is a different model for SDC's purposes.
    pub fn id(&self) -> String {
        format!("{}|{}|{}|{}", self.provider, self.digest, self.quantization, self.runtime)
    }

    /// A file-name-safe form of [`ModelKey::id`].
    pub fn file_stem(&self) -> String {
        let readable: String = self.model.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
        let digest: String = self.digest.chars().filter(char::is_ascii_alphanumeric).take(12).collect();
        let mut hash = 0xcbf29ce484222325u64;

        for byte in self.id().bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }

        format!("{readable}-{digest}-{hash:08x}")
    }
}

/// One answer from the model under test.
#[derive(Debug, Clone, Default)]
pub struct Reply {
    pub text: String,
    /// Structured tool calls: `(name, arguments)`.
    pub tool_calls: Vec<(String, Value)>,
    pub prompt_tokens: u64,
    pub output_tokens: u64,
    pub first_token_ms: u64,
    pub total_ms: u64,
    pub had_logprobs: bool,
    pub had_thinking: bool,
}

/// What the probe asks the model, in the terms every runtime shares.
#[derive(Debug, Clone)]
pub struct Ask {
    pub prompt: String,
    /// A tool the model is offered (name, JSON schema of its arguments); none for a plain question.
    pub tool: Option<(String, Value)>,
    /// A JSON schema the answer must follow; none for free text.
    pub schema: Option<Value>,
    pub num_ctx: u64,
    pub max_output: u64,
    pub seed: u64,
    /// `Some(false)` keeps a thinking model (Qwen3.5) from spending the whole `max_output` on hidden reasoning and answering with nothing; `None` leaves the model's own default.
    pub think: Option<bool>,
}

/// What a runtime has to be able to do to be probed.
pub trait Runner {
    fn identity(&self) -> Result<ModelKey, String>;
    fn ask(&mut self, ask: &Ask) -> Result<Reply, String>;
    /// Bytes of video memory the loaded model uses and the share of it on the GPU (0..=1), when the runtime says.
    fn memory(&mut self) -> Option<(u64, f64)> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Class {
    S,
    M,
    L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Measured on exactly this identity.
    Probed,
    /// Measured on an earlier identity of this model; usable, due to be measured again.
    Provisional,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub key: ModelKey,
    pub status: Status,
    pub class: Class,
    /// The largest tested context at which every needle was found, 0 when even the smallest failed.
    pub effective_context: u64,
    /// The largest tested length at which the model wrote every line it was asked for.
    pub max_reliable_output: u64,
    /// Share of the tool calls that came back as a valid structured call.
    pub tool_call_reliability: f64,
    /// Tool calls that were printed as text (`<tool_call>…`) instead of being made - Tool-call Rescue's input.
    pub tool_calls_printed_as_text: u32,
    pub json_adherence: f64,
    pub tokens_per_second: f64,
    pub first_token_ms: u64,
    pub vram_bytes: Option<u64>,
    pub gpu_share: Option<f64>,
    pub logprobs: bool,
    pub thinking: bool,
    pub seed: u64,
    pub probed_at: String,
}

/// The size of the probe. The defaults are the plan's; a test uses a smaller one.
#[derive(Debug, Clone)]
pub struct Plan {
    pub seed: u64,
    /// Candidate window sizes in tokens, ascending; each is tried with a needle at 25/50/75/100% of its depth.
    pub contexts: Vec<u64>,
    pub tool_trials: u32,
    pub schema_trials: u32,
    /// Line counts the model is asked to write, ascending.
    pub outputs: Vec<u64>,
}

impl Default for Plan {
    fn default() -> Self {
        Self { seed: 20_261_017, contexts: vec![2_048, 4_096, 8_192, 16_384, 32_768], tool_trials: 20, schema_trials: 10, outputs: vec![128, 256, 512, 1_024, 2_048] }
    }
}

/// Words that carry no meaning to a model and nothing about a person.
const WORDS: [&str; 24] = [
    "amber", "birch", "cobalt", "dune", "ember", "fjord", "garnet", "harbor", "indigo", "juniper", "kelp", "lumen", "meadow", "nectar", "onyx", "prairie", "quartz", "reef", "spruce",
    "tundra", "umber", "velvet", "willow", "zephyr",
];

/// What a filler word was assumed to cost before it was measured.
pub const DEFAULT_TOKENS_PER_WORD: f64 = 1.5;

/// Tokens per filler word on this model's tokenizer, measured with one small request that counts `sample`, a prompt of `words` filler words.
///
/// A prompt sized by a guessed ratio overflows `num_ctx` on a tokenizer that splits more finely (Qwen3.5 costs about 2.8 tokens a word of this
/// filler, not 1.5), and Ollama then drops the start of it without a word - so every needle "is lost" at the smallest size and the model measures 0.
/// A runtime that does not count gets 3.0: prompts come out shorter than named, never longer. The measured ratio is padded by 5%.
pub fn tokens_per_word(runner: &mut dyn Runner, sample: &str, words: u64, seed: u64) -> f64 {
    /* a unique first line, so a prompt cache that remembers an earlier run cannot make the count come out low */
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
    let ask = Ask { prompt: format!("Run {nonce}.\n{sample}"), tool: None, schema: None, num_ctx: 4_096, max_output: 1, seed, think: Some(false) };

    match runner.ask(&ask) {
        Ok(reply) if reply.prompt_tokens > 0 => (reply.prompt_tokens as f64 / words.max(1) as f64 * 1.05).clamp(1.0, 8.0),
        _ => 3.0,
    }
}

/// Words of filler a calibration request carries: about 850 tokens on a fine tokenizer, well inside the 4,096 it is sent with.
const CALIBRATION_WORDS: u64 = 300;

/// A small generator, so the probe asks the same questions for the same seed on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;

        self.0
    }
}

impl Plan {
    /// About `tokens` tokens of filler with `needle` at `depth` (0.0..=1.0) of the way in, ending in the question.
    /// A pure function of its arguments - the probe never reads anything of the person's.
    pub fn prompt_text(&self, tokens: u64, depth: f64, needle: &str) -> String {
        self.prompt_text_with(tokens, depth, needle, DEFAULT_TOKENS_PER_WORD)
    }

    /// [`Plan::prompt_text`] for a model whose tokenizer costs `tokens_per_word` tokens a filler word (measured by [`tokens_per_word`]; Qwen3.5 costs about 2.8, not 1.5).
    pub fn prompt_text_with(&self, tokens: u64, depth: f64, needle: &str, tokens_per_word: f64) -> String {
        let mut rng = Rng(self.seed ^ tokens.rotate_left(17) ^ ((depth * 100.0) as u64).rotate_left(41) | 1);
        let words = ((tokens as f64 / tokens_per_word.max(1.0)) as u64).saturating_sub(40).max(10);
        let at = ((words as f64) * depth.clamp(0.0, 1.0)) as u64;
        let mut text = String::new();

        for index in 0..words {
            if index == at.min(words - 1) {
                text.push_str(&format!("The access code is {needle}. "));
            }

            text.push_str(WORDS[(rng.next() % WORDS.len() as u64) as usize]);
            text.push(if index % 12 == 11 { '.' } else { ' ' });
        }

        text.push_str("\n\nQuestion: what is the access code? Answer with the code only.");

        text
    }

    /// `words` filler words and nothing else - what [`tokens_per_word`] counts.
    pub fn filler_text(&self, words: u64) -> String {
        let mut rng = Rng(self.seed | 1);
        let mut text = String::new();

        for index in 0..words {
            text.push_str(WORDS[(rng.next() % WORDS.len() as u64) as usize]);
            text.push(if index % 12 == 11 { '.' } else { ' ' });
        }

        text
    }

    /// The needle for one context size and depth: letters and digits that appear nowhere in the filler.
    pub fn needle(&self, tokens: u64, depth: f64) -> String {
        let mut rng = Rng(self.seed ^ tokens.rotate_left(5) ^ ((depth * 1000.0) as u64).rotate_left(29) | 1);

        format!("QX{:05}Z{}", rng.next() % 100_000, (b'A' + (rng.next() % 26) as u8) as char)
    }
}

fn add_tool() -> (String, Value) {
    ("add".to_string(), json!({ "type": "object", "properties": { "a": { "type": "integer" }, "b": { "type": "integer" } }, "required": ["a", "b"] }))
}

fn person_schema() -> Value {
    json!({ "type": "object", "properties": { "name": { "type": "string" }, "age": { "type": "integer" }, "tags": { "type": "array", "items": { "type": "string" } } }, "required": ["name", "age", "tags"] })
}

/// A printed tool call: `<tool_call>…</tool_call>` or a bare JSON object naming the tool.
pub fn printed_tool_call(text: &str) -> bool {
    text.contains("<tool_call>") || (text.contains("\"name\"") && text.contains("\"add\"") && text.contains("\"arguments\""))
}

/// Does `value` hold what `person_schema` requires?
pub fn follows_person_schema(text: &str) -> bool {
    let start = text.find('{');
    let end = text.rfind('}');
    let Some(value) = start.zip(end).and_then(|(s, e)| serde_json::from_str::<Value>(&text[s..=e]).ok()) else {
        return false;
    };

    value["name"].is_string() && value["age"].is_i64() && value["tags"].as_array().is_some_and(|tags| tags.iter().all(Value::is_string))
}

/// Measures a model. Each part is independent: a runtime that cannot do one (no tools, no logprobs) gets the answer
/// "no" for it, not a failed probe.
pub fn run(runner: &mut dyn Runner, plan: &Plan, now: &str) -> Result<Profile, String> {
    let key = runner.identity()?;
    let largest = plan.contexts.last().copied().unwrap_or(4_096);
    let per_word = tokens_per_word(runner, &plan.filler_text(CALIBRATION_WORDS), CALIBRATION_WORDS, plan.seed);

    /* 1. Effective context: needles at 25/50/75/100% of each candidate size; the largest size where all four are
       found (and no smaller size failed first - a model that loses the middle of 8k is not "good to 32k"). */
    let mut effective = 0u64;

    for &tokens in &plan.contexts {
        let mut all = true;

        for depth in [0.25, 0.5, 0.75, 1.0] {
            let needle = plan.needle(tokens, depth);
            let ask = Ask { prompt: plan.prompt_text_with(tokens, depth, &needle, per_word), tool: None, schema: None, num_ctx: tokens + 512, max_output: 24, seed: plan.seed, think: Some(false) };
            let found = runner.ask(&ask).map(|reply| reply.text.contains(&needle)).unwrap_or(false);

            if !found {
                all = false;
                break;
            }
        }

        if !all {
            break;
        }

        effective = tokens;
    }

    /* 2. Tool calls: structured ones and ones printed as text. */
    let (mut structured, mut printed) = (0u32, 0u32);

    for trial in 0..plan.tool_trials {
        let (a, b) = (3 + trial, 40 + 2 * trial);
        let ask = Ask { prompt: format!("Use the add tool to add {a} and {b}."), tool: Some(add_tool()), schema: None, num_ctx: 2_048, max_output: 128, seed: plan.seed + u64::from(trial), think: Some(false) };

        if let Ok(reply) = runner.ask(&ask) {
            let valid = reply.tool_calls.iter().any(|(name, args)| name == "add" && args["a"].as_i64() == Some(i64::from(a)) && args["b"].as_i64() == Some(i64::from(b)));

            if valid {
                structured += 1;
            } else if printed_tool_call(&reply.text) {
                printed += 1;
            }
        }
    }

    /* 3. JSON-schema adherence. */
    let mut followed = 0u32;

    for trial in 0..plan.schema_trials {
        let ask = Ask { prompt: format!("Describe a fictional person number {trial} as JSON."), tool: None, schema: Some(person_schema()), num_ctx: 2_048, max_output: 200, seed: plan.seed + u64::from(trial), think: Some(false) };

        if runner.ask(&ask).map(|reply| follows_person_schema(&reply.text)).unwrap_or(false) {
            followed += 1;
        }
    }

    /* 4. How long an answer the model finishes: every numbered line present, the last one included. */
    let mut longest = 0u64;

    for &lines in &plan.outputs {
        let ask = Ask { prompt: format!("Write the numbers from 1 to {lines}, one per line, as `N. word`."), tool: None, schema: None, num_ctx: (lines * 12).max(2_048), max_output: lines * 10, seed: plan.seed, think: Some(false) };
        let complete = runner.ask(&ask).map(|reply| reply.text.lines().filter(|line| line.trim_start().starts_with(|c: char| c.is_ascii_digit())).count() as u64 >= lines && reply.text.contains(&format!("{lines}."))).unwrap_or(false);

        if !complete {
            break;
        }

        longest = lines;
    }

    /* 5. Speed, logprobs and thinking, from one ordinary request; memory from the runtime. */
    let speed = runner
        .ask(&Ask { prompt: "Count from one to thirty in English words, separated by commas.".into(), tool: None, schema: None, num_ctx: 2_048, max_output: 200, seed: plan.seed, think: None })
        .unwrap_or_default();
    let generating = speed.total_ms.saturating_sub(speed.first_token_ms).max(1);
    let tokens_per_second = if speed.output_tokens == 0 { 0.0 } else { speed.output_tokens as f64 * 1000.0 / generating as f64 };
    let memory = runner.memory();
    let tool_call_reliability = f64::from(structured) / f64::from(plan.tool_trials.max(1));
    let json_adherence = f64::from(followed) / f64::from(plan.schema_trials.max(1));

    Ok(Profile {
        key,
        status: Status::Probed,
        class: classify(effective, tool_call_reliability, largest),
        effective_context: effective,
        max_reliable_output: longest,
        tool_call_reliability,
        tool_calls_printed_as_text: printed,
        json_adherence,
        tokens_per_second,
        first_token_ms: speed.first_token_ms,
        vram_bytes: memory.map(|(bytes, _)| bytes),
        gpu_share: memory.map(|(_, share)| share),
        logprobs: speed.had_logprobs,
        thinking: speed.had_thinking,
        seed: plan.seed,
        probed_at: now.to_string(),
    })
}

/// S, M or L from what was measured: a model that cannot hold ~12k tokens or make tool calls reliably is S (the
/// small-model engine's full treatment); one that holds ~40k and calls tools well is L.
pub fn classify(effective_context: u64, tool_reliability: f64, _largest_tested: u64) -> Class {
    if effective_context < 12_000 || tool_reliability < 0.8 {
        Class::S
    } else if effective_context < 40_000 || tool_reliability < 0.95 {
        Class::M
    } else {
        Class::L
    }
}

fn dir() -> Option<PathBuf> {
    crate::paths::data_dir().ok().map(|dir| dir.join("profiles"))
}

/// Stores a profile under its identity.
pub fn save(profile: &Profile) -> Result<PathBuf, String> {
    let folder = dir().ok_or("no data directory")?;

    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;

    let path = folder.join(format!("{}.json", profile.key.file_stem()));

    std::fs::write(&path, serde_json::to_vec_pretty(profile).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;

    Ok(path)
}

/// The stored profile for a model *as it is now*: `Probed` when the identity is the same, `Provisional` when the
/// only profile on file was measured on an earlier identity of the same model name, `None` when there is none.
pub fn lookup(current: &ModelKey) -> Option<Profile> {
    let folder = dir()?;

    if let Ok(text) = std::fs::read_to_string(folder.join(format!("{}.json", current.file_stem()))) {
        if let Ok(profile) = serde_json::from_str::<Profile>(&text) {
            if profile.key.id() == current.id() {
                return Some(profile);
            }
        }
    }

    /* An older identity of the same model: newest first. */
    let mut older: Vec<Profile> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<Profile>(&text).ok())
        .filter(|profile| profile.key.provider == current.provider && profile.key.model == current.model)
        .collect();

    older.sort_by(|a, b| b.probed_at.cmp(&a.probed_at));

    older.into_iter().next().map(|mut profile| {
        profile.status = Status::Provisional;

        profile
    })
}

/// The newest profile on file for a model *name*, without asking the runtime what it is now (a model list must not
/// call Ollama for every row). Its status is what it was stored with; `lookup` is the exact-identity question.
pub fn latest(provider: &str, model: &str) -> Option<Profile> {
    std::fs::read_dir(dir()?)
        .ok()?
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<Profile>(&text).ok())
        .filter(|profile| profile.key.provider == provider && profile.key.model == model)
        .max_by(|a, b| a.probed_at.cmp(&b.probed_at))
}

/// The row a model list carries for a profile.
pub fn row(profile: &Profile) -> Value {
    json!({
        "class": label(profile),
        "status": if profile.status == Status::Probed { "probed" } else { "provisional" },
        "effectiveContext": profile.effective_context,
        "toolCallReliability": profile.tool_call_reliability,
        /* N1-2: a code the UI turns into one line of advice (`quant4`, `kv4`, `quant4kv4`), or null */
        "advice": crate::navaratna::advice::code_for(&profile.key.model, Some(&profile.key.quantization)),
    })
}

/// The class a model selector shows, with a mark when the profile is only provisional.
pub fn label(profile: &Profile) -> String {
    let class = match profile.class {
        Class::S => "S",
        Class::M => "M",
        Class::L => "L",
    };

    if profile.status == Status::Provisional {
        format!("{class}?")
    } else {
        class.to_string()
    }
}

/// The quick plan: three windows, fewer trials - what a probe that runs by itself after a turn uses, so it holds the
/// graphics card for seconds rather than minutes. `sdcd probe` without `--quick` is the full measurement.
pub fn quick_plan() -> Plan {
    Plan { contexts: vec![2_048, 4_096, 8_192], tool_trials: 6, schema_trials: 4, outputs: vec![128, 512], ..Plan::default() }
}

/// Whether a model needs measuring: no profile on file for its identity as it is now, or only one for an earlier
/// file or runtime.
pub fn due(stored: Option<&Profile>) -> bool {
    stored.is_none_or(|profile| profile.status != Status::Probed)
}

/// The once-per-model limiter of [`after_use`]: true the first time a model is asked about in this daemon run, false
/// every time after.
fn first_ask(model: &str) -> bool {
    static ASKED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();

    ASKED.get_or_init(Default::default).lock().map(|mut asked| asked.insert(model.to_string())).unwrap_or(false)
}

/// Switch `models.probe`: after a turn on a local model, measures that model in the background when it has never been
/// measured, or its file or runtime changed since. Once per model per daemon run, so a probe that fails (the server
/// went away) is not retried on every turn; nothing happens while the switch is off.
pub fn after_use(model: &str) {
    if !crate::v3::on("models.probe") || model.trim().is_empty() || !first_ask(model) {
        return;
    }

    let model = model.to_string();

    std::thread::spawn(move || {
        let mut runner = super::ollama_probe::OllamaRunner { model: model.clone() };
        let Ok(key) = runner.identity() else {
            return;
        };

        if !due(lookup(&key).as_ref()) {
            crate::padma::model::probe_if_missing(&model);

            return;
        }

        match run(&mut runner, &quick_plan(), &chrono::Utc::now().to_rfc3339()) {
            Ok(profile) => {
                if let Err(error) = save(&profile) {
                    eprintln!("sdcd: the probe of {model} could not save its profile: {error}");
                }
            }
            Err(error) => eprintln!("sdcd: the probe of {model} failed: {error}"),
        }

        crate::padma::model::probe_if_missing(&model);
    });
}

/// `sdcd probe MODEL [--seed N] [--quick]`: measures an installed Ollama model and stores the profile. The only
/// entry point that talks to a live runtime; everything it decides is in [`run`].
pub fn main(args: &[String]) -> i32 {
    let Some(model) = args.first().filter(|arg| !arg.starts_with("--")) else {
        eprintln!("usage: sdcd probe MODEL [--seed N] [--quick]   (MODEL is an installed Ollama model, e.g. qwen3.5:9b)");

        return 3;
    };
    let mut plan = Plan::default();

    if let Some(seed) = args.iter().position(|arg| arg == "--seed").and_then(|at| args.get(at + 1)).and_then(|n| n.parse().ok()) {
        plan.seed = seed;
    }

    if args.iter().any(|arg| arg == "--quick") {
        plan = Plan { seed: plan.seed, ..quick_plan() };
    }

    let mut runner = super::ollama_probe::OllamaRunner { model: model.clone() };

    match run(&mut runner, &plan, &chrono::Utc::now().to_rfc3339()) {
        Ok(profile) => {
            match save(&profile) {
                Ok(path) => println!("saved {}", path.display()),
                Err(error) => eprintln!("could not save the profile: {error}"),
            }

            println!("{}", serde_json::to_string_pretty(&profile).unwrap_or_default());

            0
        }
        Err(error) => {
            eprintln!("probe failed: {error}");

            1
        }
    }
}

/// Tests that point `SDC_DATA_DIR` somewhere take this first: the environment is shared by every test thread.
#[cfg(test)]
pub static DATA_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests;