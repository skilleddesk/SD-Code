//! **N1-1 - the num_ctx lock and the window probe** (switch `nr.ctx_lock`).
//!
//! Ollama answers 200 and silently drops the start of a prompt that does not fit the window it was loaded with. So (1) every request to
//! Ollama carries `options.num_ctx` (checked by a test that reads the sources), (2) before a request is sent the prompt is counted with the
//! daemon's one estimator, and if it plus the answer's reserve exceeds the **usable** window it is handed back as [`NeedsSplit`] - never
//! sent - and (3) the usable window is what `sdcd probe window --model M` measured: facts planted at the start, middle and end of growing
//! prompts, the largest length at which all of them are recalled. A model with no probe gets half of the context it advertises.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::sme::probe::{Ask, Runner};

/// The prompt does not fit the window the model can really use. The text starts `ContextOverflow` so that everything that already reacts
/// to an overflow (the relay's reason, the splitter) reacts to this too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsSplit {
    pub estimated: u64,
    pub reserved: u64,
    pub usable: u64,
}

impl std::fmt::Display for NeedsSplit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ContextOverflow: NeedsSplit - this step is about {} tokens with {} reserved for the answer, and the part of the window this model has been shown to use is {}; SDC splits the work instead of sending it",
            self.estimated, self.reserved, self.usable
        )
    }
}

/// The window a request may use: the probed one when there is one, else half of what the model advertises, never more than the
/// `num_ctx` SDC asks Ollama for. A model whose card says nothing is not narrowed by a guess.
pub fn usable_window(num_ctx: u64, probed: Option<u64>, advertised: Option<u64>) -> u64 {
    match (probed.filter(|window| *window > 0), advertised.filter(|window| *window > 0)) {
        (Some(probed), _) => probed.min(num_ctx),
        (None, Some(advertised)) => (advertised / 2).min(num_ctx).max(1),
        (None, None) => num_ctx,
    }
}

/// The window measured for a model, from its stored profile.
pub fn probed_window(model: &str) -> Option<u64> {
    stored_window(model).or_else(|| crate::sme::probe::latest("ollama", model).map(|profile| profile.effective_context).filter(|window| *window > 0))
}

/// The usable window of a local model for a request sent with `num_ctx`: the probed one if there is one. Without a probe, half of the context
/// the model *card* advertises would be the answer - but the only figure SDC has offline is its own catalogue's, which is already the
/// conservative one it plans with (16K for a model whose card says 256K), so halving it again would starve ordinary turns. The card figure
/// is passed to [`usable_window`] only where a caller really has it (`ollama show`); here the answer without a probe is `num_ctx`.
pub fn usable_for(model: &str, num_ctx: u64) -> u64 {
    usable_window(num_ctx, probed_window(model), None)
}

/// Counts a request and answers its estimate - or [`NeedsSplit`] when it plus `reserved` does not fit `usable`.
pub fn preflight(body: &Value, usable: u64, reserved: u64) -> Result<u64, NeedsSplit> {
    crate::guard::context::check(body, usable, reserved).map_err(|overflow| NeedsSplit { estimated: overflow.estimated, reserved, usable })
}

/// Does a request body carry `options.num_ctx` (the native Ollama shape)?
pub fn names_num_ctx(body: &Value) -> bool {
    body["options"]["num_ctx"].as_u64().is_some_and(|n| n > 0)
}

/* ---- the probe ---------------------------------------------------------------------------------------------------------------------- */

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowProbe {
    pub model: String,
    /// The largest tested length (tokens) at which all three planted facts were recalled; 0 when even the smallest failed.
    pub usable_tokens: u64,
    pub tested: Vec<(u64, bool)>,
    pub probed_at: String,
}

const WORDS: [&str; 12] = ["amber", "birch", "cobalt", "dune", "ember", "fjord", "garnet", "harbor", "indigo", "juniper", "kelp", "lumen"];

/// A prompt of about `tokens` tokens with three key-value facts at 5%, 50% and 95% of the way in, and the question for all three.
pub fn probe_prompt(tokens: u64, seed: u64) -> (String, [String; 3]) {
    let words = (tokens * 2 / 3).saturating_sub(60).max(30);
    let values = [0, 1, 2].map(|n| format!("V{:04}{}", (seed.wrapping_mul(2_654_435_761).wrapping_add(n * 7_919 + tokens)) % 10_000, (b'A' + n as u8) as char));
    let at = [words / 20, words / 2, words * 19 / 20];
    let mut text = String::new();

    for index in 0..words {
        if let Some(n) = at.iter().position(|spot| *spot == index) {
            text.push_str(&format!("Fact {}: the key K{} has the value {}. ", n + 1, n + 1, values[n]));
        }

        text.push_str(WORDS[((index.wrapping_mul(7) + seed) % WORDS.len() as u64) as usize]);
        text.push(if index % 12 == 11 { '.' } else { ' ' });
        text.push(' ');
    }

    text.push_str("\n\nQuestion: what are the values of K1, K2 and K3? Answer with the three values only.");

    (text, values)
}

/// Runs the probe over `sizes` (ascending). The first size that fails ends it - a model that loses the middle of 8k is not "good to 32k".
pub fn probe_window(runner: &mut dyn Runner, sizes: &[u64], seed: u64, now: &str) -> Result<WindowProbe, String> {
    let key = runner.identity()?;
    let mut tested = Vec::new();
    let mut usable = 0;

    for &size in sizes {
        let (prompt, values) = probe_prompt(size, seed);
        let ask = Ask { prompt, tool: None, schema: None, num_ctx: size + 512, max_output: 48, seed };
        let recalled = runner.ask(&ask).map(|reply| values.iter().all(|value| reply.text.contains(value))).unwrap_or(false);

        tested.push((size, recalled));

        if !recalled {
            break;
        }

        usable = size;
    }

    Ok(WindowProbe { model: key.model, usable_tokens: usable, tested, probed_at: now.to_string() })
}

fn models_dir() -> Option<PathBuf> {
    crate::paths::data_dir().ok().map(|dir| dir.join("models"))
}

fn file_name(model: &str) -> String {
    let safe: String = model.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();

    format!("{safe}.toml")
}

/// Writes the probe next to the other per-model files, as `models/<model>.toml`.
pub fn save_in(dir: &Path, probe: &WindowProbe) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;

    let tested: Vec<String> = probe.tested.iter().map(|(size, ok)| format!("[{size}, {ok}]")).collect();
    let text = format!(
        "# written by `sdcd probe window` (NAVARATNA N1-1); the largest prompt length at which three planted facts were all recalled\nmodel = {:?}\nusable_tokens = {}\nprobed_at = {:?}\ntested = [{}]\n",
        probe.model,
        probe.usable_tokens,
        probe.probed_at,
        tested.join(", ")
    );
    let path = dir.join(file_name(&probe.model));

    std::fs::write(&path, text).map_err(|error| error.to_string())?;
    Ok(path)
}

fn load_from(dir: &Path, model: &str) -> Option<u64> {
    let text = std::fs::read_to_string(dir.join(file_name(model))).ok()?;
    let table: toml::Value = text.parse().ok()?;

    table.get("usable_tokens")?.as_integer().filter(|n| *n > 0).map(|n| n as u64)
}

fn stored_window(model: &str) -> Option<u64> {
    load_from(&models_dir()?, model)
}

/// `sdcd probe window --model M [--sizes 2048,4096,...]`: measures an installed Ollama model and stores the result.
pub fn main(args: &[String]) -> i32 {
    let Some(model) = args.iter().position(|arg| arg == "--model").and_then(|at| args.get(at + 1)) else {
        eprintln!("usage: sdcd probe window --model M [--sizes 2048,4096,8192,16384,32768]");

        return 3;
    };
    let sizes: Vec<u64> = args
        .iter()
        .position(|arg| arg == "--sizes")
        .and_then(|at| args.get(at + 1))
        .map(|list| list.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![2_048, 4_096, 8_192, 16_384, 32_768]);
    let mut runner = crate::sme::ollama_probe::OllamaRunner { model: model.clone() };

    match probe_window(&mut runner, &sizes, 20_261_024, &chrono::Utc::now().to_rfc3339()) {
        Ok(probe) => match models_dir().ok_or("no data directory".to_string()).and_then(|dir| save_in(&dir, &probe)) {
            Ok(path) => {
                println!("{model}: usable window {} tokens; saved {}", probe.usable_tokens, path.display());

                0
            }
            Err(error) => {
                eprintln!("could not save: {error}");

                1
            }
        },
        Err(error) => {
            eprintln!("probe failed: {error}");

            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sme::probe::{ModelKey, Reply};

    /// A fake model that recalls facts only while the prompt is shorter than `reach` words, and otherwise loses the middle one.
    struct Fake {
        reach: usize,
    }

    impl Runner for Fake {
        fn identity(&self) -> Result<ModelKey, String> {
            Ok(ModelKey { provider: "ollama".into(), model: "fake:9b".into(), digest: "d".into(), quantization: "Q4".into(), runtime: "fake".into() })
        }

        fn ask(&mut self, ask: &Ask) -> Result<Reply, String> {
            let words = ask.prompt.split_whitespace().count();
            let values: Vec<&str> = ask.prompt.split("has the value ").skip(1).map(|rest| rest.split('.').next().unwrap_or_default()).collect();
            let shown: Vec<&str> = if words <= self.reach { values } else { values.into_iter().enumerate().filter(|(n, _)| *n != 1).map(|(_, v)| v).collect() };

            Ok(Reply { text: shown.join(", "), ..Reply::default() })
        }
    }

    #[test]
    fn the_probe_finds_the_largest_length_with_full_recall_and_stops_at_the_first_failure() {
        let probe = probe_window(&mut Fake { reach: 2_000 }, &[2_048, 4_096, 8_192], 7, "now").unwrap();

        assert_eq!(probe.usable_tokens, 2_048);
        assert_eq!(probe.tested, vec![(2_048, true), (4_096, false)], "8192 is never tried after a failure");
        assert_eq!(probe_window(&mut Fake { reach: 100 }, &[2_048], 7, "now").unwrap().usable_tokens, 0);
        assert_eq!(probe_window(&mut Fake { reach: 1_000_000 }, &[2_048, 4_096], 7, "now").unwrap().usable_tokens, 4_096);
    }

    #[test]
    fn the_probe_is_stored_per_model_as_toml_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let probe = probe_window(&mut Fake { reach: 1_000_000 }, &[2_048, 4_096], 7, "2026-10-24T00:00:00Z").unwrap();
        let path = save_in(dir.path(), &probe).unwrap();

        assert!(path.ends_with("fake_9b.toml") || path.ends_with("fake_9b.toml".trim()), "{path:?}");
        assert!(std::fs::read_to_string(&path).unwrap().parse::<toml::Value>().is_ok(), "valid TOML");
        assert_eq!(load_from(dir.path(), "fake:9b"), Some(4_096));
        assert_eq!(load_from(dir.path(), "other:1b"), None);
    }

    #[test]
    fn without_a_probe_the_usable_window_is_half_of_the_advertised_one_but_never_above_num_ctx() {
        assert_eq!(usable_window(32_000, Some(12_000), Some(128_000)), 12_000);
        assert_eq!(usable_window(8_000, Some(12_000), Some(128_000)), 8_000);
        assert_eq!(usable_window(100_000, None, Some(128_000)), 64_000);
        assert_eq!(usable_window(16_000, None, Some(128_000)), 16_000);
        assert_eq!(usable_window(16_000, None, None), 16_000);
        assert_eq!(usable_window(16_000, Some(0), None), 16_000);
    }

    #[test]
    fn an_oversize_prompt_is_handed_back_as_needs_split_and_an_undersize_one_is_counted() {
        let small = serde_json::json!({ "messages": [{ "role": "user", "content": "hello" }], "tools": [] });
        let big = serde_json::json!({ "messages": [{ "role": "user", "content": "word ".repeat(6_000) }], "tools": [] });

        assert!(preflight(&small, 4_096, 512).is_ok());

        let split = preflight(&big, 4_096, 512).unwrap_err();

        assert!(split.estimated > 4_096 - 512);
        assert!(split.to_string().starts_with("ContextOverflow: NeedsSplit"), "{split}");
        assert!(preflight(&big, 40_000, 512).is_ok());
    }

    /// "Zero code paths call Ollama without num_ctx": every source that builds a request to Ollama's own chat endpoint also names `num_ctx`.
    #[test]
    fn no_source_that_calls_ollamas_chat_endpoint_leaves_num_ctx_out() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root];
        let mut callers = Vec::new();

        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();

                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") && !path.to_string_lossy().contains("tests") && !path.ends_with("navaratna/window.rs") {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    let production = text.split("#[cfg(test)]").next().unwrap_or_default();

                    if production.contains("POST /api/chat") {
                        callers.push((path.clone(), production.contains("num_ctx")));
                    }
                }
            }
        }

        assert!(callers.len() >= 2, "the test found the callers: {callers:?}");
        assert!(callers.iter().all(|(_, names)| *names), "{callers:?}");
    }

    #[test]
    fn the_native_body_names_num_ctx() {
        let body = crate::agent::ollama_native::body(serde_json::json!({ "model": "m", "messages": [] }), 8_192);

        assert!(names_num_ctx(&body));
        assert!(!names_num_ctx(&serde_json::json!({ "model": "m" })));
    }
}

