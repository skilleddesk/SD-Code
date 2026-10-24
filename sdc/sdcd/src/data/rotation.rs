//! `rotation.toml` in the data directory (`crate::paths::data_dir()`: `~/Library/Application Support/sdc` on macOS, `%APPDATA%\\sdc` on Windows, `~/.local/share/sdc` on Linux) (P0-5): the model chain, and the rules that follow it.
//!
//! ```toml
//! # The chain, in the order SDC may try it.
//! [[chain]]
//! model = "ollama:qwen2.5-coder:14b"
//! enabled = true
//! allowed_steps = ["plan", "edit", "repair"]      # empty means every step
//! max_cost_per_step = 0.0                          # dollars; a local model costs nothing
//! max_repair_retries = 2
//!
//! [[chain]]
//! model = "deepseek:deepseek-chat"
//! enabled = true
//! allowed_steps = ["plan", "review"]
//! max_cost_per_step = 0.05
//! max_repair_retries = 1
//!
//! [global]
//! daily_usd = 2.0        # 0 means no cap, not "spend nothing"
//! monthly_usd = 40.0
//! ask_before_cloud = true
//! offload_limit = 8      # how many steps in a row may go to a model that is not local
//! ```
//!
//! # What this file is, and what it is not
//!
//! It is a **preference**: the order SDC looks for a model, and the ceiling each row is allowed to reach. It is not
//! permission. `.sdc/policy.toml` is permission, and `data`'s precedence rule is that an instruction beats a
//! preference - so a project marked `local-only` gets no remote model whichever way this file is written. This
//! module reads the file; `crate::rotation` (P2-1) is where the chain is walked, and it asks `policy` first.
//!
//! # Defaults
//!
//! A missing file is not an error and not a warning: it means "no preference", and the chain is empty. An empty
//! chain is what keeps a fresh install from spending money it was never told it could spend. A file that exists
//! but does not parse **is** reported - with the parser's own message, which names the line - and then the same
//! empty default is used, so a typo costs a warning and not the ability to start.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the file lives under the home folder.
pub const FILE: &str = "rotation.toml";

/// One row of the chain: a model, and what it is allowed to be used for.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub model: String,
    /// A row can be kept and switched off, which is how a person says "not this one for now" without losing the
    /// settings they thought about.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The steps this model may serve. Empty means every step: the common case is a local model that may do
    /// anything, and a remote one that may only plan.
    #[serde(default)]
    pub allowed_steps: Vec<String>,
    /// Dollars for a single step. `0` means nothing may be spent, which is what makes a local row local.
    #[serde(default)]
    pub max_cost_per_step: f64,
    /// How many times a failed step may be handed back to this model before the chain moves on.
    #[serde(default)]
    pub max_repair_retries: u32,
}

fn yes() -> bool {
    true
}

/// The ceilings that apply to the chain as a whole.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Global {
    /// Dollars a day. `0` means **no cap**, not "spend nothing" - a cap of zero would be a chain that can never
    /// run, which is not what someone writing `daily_usd = 0` means.
    #[serde(default)]
    pub daily_usd: f64,
    #[serde(default)]
    pub monthly_usd: f64,
    /// Whether a step that would leave this computer for a cloud model waits for a yes.
    #[serde(default = "yes")]
    pub ask_before_cloud: bool,
    /// How many steps in a row may go to a model that is not local before SDC stops and says so.
    #[serde(default)]
    pub offload_limit: u32,
}

impl Default for Global {
    fn default() -> Self {
        /* Defaults are on the cautious side of every switch a default can be cautious about: ask before spending,
           and no offload allowance at all until a person writes one. */
        Self { daily_usd: 0.0, monthly_usd: 0.0, ask_before_cloud: true, offload_limit: 0 }
    }
}

/// The whole file.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rotation {
    #[serde(default, rename = "chain")]
    pub chain: Vec<Row>,
    #[serde(default, rename = "global")]
    pub global: Global,
    /// Silent Relay (M1-1): a failed local step goes to the chain by itself, inside the limits below.
    #[serde(default, rename = "relay")]
    pub relay: crate::rotation::policy::RelayPolicy,
}

impl Rotation {
    /// The rows SDC may actually use: enabled, and with a model name.
    pub fn usable(&self) -> Vec<&Row> {
        self.chain.iter().filter(|row| row.enabled && !row.model.trim().is_empty()).collect()
    }

    /// The rows that may serve `step`. An empty `allowed_steps` is every step.
    pub fn for_step(&self, step: &str) -> Vec<&Row> {
        self.usable().into_iter().filter(|row| row.allowed_steps.is_empty() || row.allowed_steps.iter().any(|name| name == step)).collect()
    }

    /// The first local row that may serve `step`, if any. `ollama:` and `local:` are how a local model is named.
    pub fn local_for_step(&self, step: &str) -> Option<&Row> {
        self.for_step(step).into_iter().find(|row| is_local(&row.model))
    }

    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }
}

/// Whether a model name means "runs on this computer". Named in one place because both the chain and the policy
/// check have to agree on the answer.
pub fn is_local(model: &str) -> bool {
    let name = model.to_ascii_lowercase();

    name.starts_with("ollama:") || name.starts_with("local:") || name.starts_with("llama") || name.starts_with("vllm:")
}

pub fn path(home: &Path) -> PathBuf {
    home.join(FILE)
}

/// The file as written, or the reason it could not be read. Distinguishing "missing" from "broken" is the point:
/// a missing file is a normal install, a broken one is a person who needs to know.
pub enum Read {
    /// No file: the empty default, and nothing to say.
    Missing,
    /// The file read and parsed.
    Ok(Rotation),
    /// The file is there and does not parse. The string is for the person, not a log.
    Broken(String),
}

/// Read `rotation.toml` from a home folder. Never returns an error: a broken file is a `Read::Broken`, because a
/// caller that has to handle `Err` will eventually swallow it.
pub fn read(home: &Path) -> Read {
    let file = path(home);

    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Read::Missing,
        Err(error) => return Read::Broken(format!("{} could not be read: {error}", file.display())),
    };

    match toml::from_str::<Rotation>(&text) {
        Ok(rotation) => Read::Ok(rotation),
        Err(error) => Read::Broken(format!("{} is not valid: {error}", file.display())),
    }
}

/// The most rows a chain may have: a chain is something a person reads and orders by hand.
pub const MAX_ROWS: usize = 12;

/// Checks a rotation a person (or the Agents tab) wants to save. The same rules for both, so a file the app wrote always reads back.
pub fn validate(rotation: &Rotation) -> Result<(), String> {
    if rotation.chain.len() > MAX_ROWS {
        return Err(format!("A chain may have at most {MAX_ROWS} models."));
    }

    let mut seen = std::collections::HashSet::new();

    for (index, row) in rotation.chain.iter().enumerate() {
        let model = row.model.trim();
        let number = index + 1;

        match model.split_once(':') {
            Some((provider, name)) if !provider.is_empty() && !name.trim().is_empty() && !model.contains(char::is_whitespace) => {}
            _ => return Err(format!("Entry {number}: the model must be written as provider:model, for example deepseek:deepseek-chat.")),
        }

        if !seen.insert(model.to_ascii_lowercase()) {
            return Err(format!("Entry {number}: {model} is already in the chain."));
        }

        if !row.max_cost_per_step.is_finite() || row.max_cost_per_step < 0.0 || row.max_cost_per_step > 100.0 {
            return Err(format!("Entry {number}: the cost limit per step must be between 0 and 100 dollars."));
        }

        if row.max_repair_retries > 10 {
            return Err(format!("Entry {number}: at most 10 retries."));
        }
    }

    let global = &rotation.global;

    for (name, value) in [("daily", global.daily_usd), ("monthly", global.monthly_usd)] {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("The {name} dollar limit cannot be negative."));
        }
    }

    if global.offload_limit > 1000 {
        return Err("The offload limit is at most 1000 steps.".into());
    }

    rotation.relay.validate()?;

    Ok(())
}

/// Writes `rotation.toml` after validating it: the new text goes beside the file and is renamed over it (a crash leaves the old file
/// or the new one), and the file it replaces is kept as `rotation.toml.bak`.
pub fn save(home: &Path, rotation: &Rotation) -> Result<(), String> {
    validate(rotation)?;

    let text = toml::to_string_pretty(rotation).map_err(|error| error.to_string())?;
    /* The text must read back as the same rotation, or it is not written. */
    let back: Rotation = toml::from_str(&text).map_err(|error| error.to_string())?;

    if &back != rotation {
        return Err("The chain could not be written faithfully; nothing was saved.".into());
    }

    std::fs::create_dir_all(home).map_err(|error| format!("{}: {error}", home.display()))?;

    let file = path(home);
    let partial = home.join(format!("{FILE}.partial"));

    std::fs::write(&partial, format!("# Written by SDC (Settings -> Agents). The order of the [[chain]] rows is the order models are offered.\n{text}")).map_err(|error| error.to_string())?;

    if file.exists() {
        let _ = std::fs::copy(&file, home.join(format!("{FILE}.bak")));
    }

    std::fs::rename(&partial, &file).map_err(|error| error.to_string())
}

#[cfg(test)]
thread_local! {
    /// A rotation a test turn runs with, instead of the data directory's file (which every test in the process shares).
    static FORCED: std::cell::RefCell<Option<Rotation>> = const { std::cell::RefCell::new(None) };
}

/// Runs `body` on this thread with `rotation` as the person's `rotation.toml` (tests).
#[cfg(test)]
pub fn forcing<T>(rotation: Rotation, body: impl FnOnce() -> T) -> T {
    FORCED.with(|forced| *forced.borrow_mut() = Some(rotation));

    let result = body();

    FORCED.with(|forced| *forced.borrow_mut() = None);
    result
}

/// The rotation to use, plus the one-line warning to show if the file was broken.
pub fn load(home: &Path) -> (Rotation, Option<String>) {
    #[cfg(test)]
    if let Some(forced) = FORCED.with(|forced| forced.borrow().clone()) {
        return (forced, None);
    }

    match read(home) {
        Read::Ok(rotation) => (rotation, None),
        Read::Broken(reason) => (Rotation::default(), Some(reason)),
        Read::Missing => (Rotation::default(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(home: &Path, text: &str) {
        std::fs::create_dir_all(home).unwrap();
        std::fs::write(path(home), text).unwrap();
    }

    #[test]
    fn no_file_is_an_empty_chain_and_no_warning() {
        let dir = tempfile::tempdir().unwrap();
        let (rotation, warning) = load(dir.path());

        assert!(rotation.is_empty(), "a fresh install has no preference, which is why it spends nothing");
        assert_eq!(warning, None, "a file nobody wrote is not a problem to report");
    }

    #[test]
    fn a_chain_row_carries_the_five_things_the_plan_names() {
        let dir = tempfile::tempdir().unwrap();

        write(
            dir.path(),
            r#"
[[chain]]
model = "ollama:qwen2.5-coder:14b"
enabled = true
allowed_steps = ["plan", "edit", "repair"]
max_cost_per_step = 0.0
max_repair_retries = 2

[[chain]]
model = "deepseek:deepseek-chat"
allowed_steps = ["plan"]
max_cost_per_step = 0.05

[global]
daily_usd = 2.0
monthly_usd = 40.0
ask_before_cloud = true
offload_limit = 8
"#,
        );

        let (rotation, warning) = load(dir.path());

        assert_eq!(warning, None);
        assert_eq!(rotation.chain.len(), 2);
        assert_eq!(rotation.chain[0].model, "ollama:qwen2.5-coder:14b");
        assert_eq!(rotation.chain[0].allowed_steps, ["plan", "edit", "repair"]);
        assert_eq!(rotation.chain[0].max_repair_retries, 2);
        /* `enabled` defaults to true: writing a row is asking for it. */
        assert!(rotation.chain[1].enabled);
        assert_eq!(rotation.global.offload_limit, 8);
        assert_eq!(rotation.global.daily_usd, 2.0);
    }

    #[test]
    fn a_disabled_row_is_kept_but_not_used() {
        let dir = tempfile::tempdir().unwrap();

        write(dir.path(), "[[chain]]\nmodel = \"deepseek:deepseek-chat\"\nenabled = false\n\n[[chain]]\nmodel = \"ollama:qwen3:8b\"\n");

        let (rotation, _) = load(dir.path());

        assert_eq!(rotation.chain.len(), 2, "the settings a person thought about are not thrown away");
        assert_eq!(rotation.usable().len(), 1);
        assert_eq!(rotation.usable()[0].model, "ollama:qwen3:8b");
    }

    #[test]
    fn an_empty_allowed_steps_means_every_step_and_a_named_one_means_that_step() {
        let dir = tempfile::tempdir().unwrap();

        write(dir.path(), "[[chain]]\nmodel = \"ollama:qwen3:8b\"\n\n[[chain]]\nmodel = \"deepseek:deepseek-chat\"\nallowed_steps = [\"plan\", \"review\"]\n");

        let (rotation, _) = load(dir.path());

        assert_eq!(rotation.for_step("edit").len(), 1, "only the row that allows everything");
        assert_eq!(rotation.for_step("edit")[0].model, "ollama:qwen3:8b");
        assert_eq!(rotation.for_step("plan").len(), 2);
        assert_eq!(rotation.for_step("review")[1].model, "deepseek:deepseek-chat");
        assert!(rotation.for_step("deploy").iter().all(|row| is_local(&row.model)));
    }

    #[test]
    fn a_local_row_is_the_one_that_costs_nothing_to_run() {
        let dir = tempfile::tempdir().unwrap();

        write(dir.path(), "[[chain]]\nmodel = \"deepseek:deepseek-chat\"\n\n[[chain]]\nmodel = \"ollama:qwen3:8b\"\n");

        let (rotation, _) = load(dir.path());

        assert_eq!(rotation.local_for_step("edit").map(|row| row.model.as_str()), Some("ollama:qwen3:8b"));
        assert!(is_local("local:anything"));
        assert!(!is_local("deepseek:deepseek-chat"));
    }

    #[test]
    fn a_broken_file_is_reported_and_the_empty_default_is_used() {
        let dir = tempfile::tempdir().unwrap();

        /* `max_cost_per_step = "cheap"` is a string where a number belongs. */
        write(dir.path(), "[[chain]]\nmodel = \"ollama:qwen3:8b\"\nmax_cost_per_step = \"cheap\"\n");

        let (rotation, warning) = load(dir.path());

        assert!(rotation.is_empty(), "a file that does not parse decides nothing");
        let warning = warning.expect("a person who wrote the file is told the file is wrong");
        assert!(warning.contains("rotation.toml"), "{warning}");
        assert!(warning.contains("max_cost_per_step"), "the parser names the key it choked on: {warning}");
    }

    #[test]
    fn an_unknown_key_is_reported_rather_than_ignored() {
        let dir = tempfile::tempdir().unwrap();

        /* A typo in a key name is the mistake a person cannot see: `max_cost_per_steps` would silently do nothing. */
        write(dir.path(), "[[chain]]\nmodel = \"ollama:qwen3:8b\"\nmax_cost_per_steps = 1.0\n");

        let (rotation, warning) = load(dir.path());

        assert!(rotation.is_empty());
        assert!(warning.unwrap().contains("unknown field"));
    }

    #[test]
    fn the_cautious_defaults_ask_before_cloud_and_allow_no_offload() {
        let global = Global::default();

        assert!(global.ask_before_cloud);
        assert_eq!(global.offload_limit, 0, "no allowance is the safe reading of a file that says nothing");
        assert_eq!(global.daily_usd, 0.0, "and 0 here means no cap, which is the existing meaning of 0");
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;

    fn row(model: &str) -> Row {
        Row { model: model.into(), enabled: true, allowed_steps: Vec::new(), max_cost_per_step: 0.05, max_repair_retries: 1 }
    }

    #[test]
    fn a_saved_chain_reads_back_exactly_and_keeps_its_order_and_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let first = Rotation { chain: vec![row("deepseek:deepseek-chat"), row("anthropic-api:claude-sonnet-5-5")], global: Global { offload_limit: 3, ..Global::default() }, relay: crate::rotation::policy::RelayPolicy { auto: true, max_cloud_steps_per_turn: 4, ..Default::default() } };

        save(dir.path(), &first).unwrap();

        let (back, warning) = load(dir.path());

        assert_eq!(warning, None);
        assert_eq!(back, first);
        assert_eq!(back.chain[0].model, "deepseek:deepseek-chat", "the order is the person's order");

        let second = Rotation { chain: vec![row("anthropic-api:claude-sonnet-5-5")], ..first.clone() };

        save(dir.path(), &second).unwrap();

        assert_eq!(load(dir.path()).0, second);
        assert!(dir.path().join("rotation.toml.bak").exists(), "the file it replaced is kept");
    }

    #[test]
    fn a_chain_that_would_not_read_back_or_make_sense_is_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut bad = Rotation { chain: vec![row("no-provider-here")], ..Rotation::default() };

        assert!(save(dir.path(), &bad).unwrap_err().contains("provider:model"));

        bad.chain = vec![row("a:b"), row("A:B")];
        assert!(save(dir.path(), &bad).unwrap_err().contains("already in the chain"));

        bad.chain = vec![Row { max_cost_per_step: -1.0, ..row("a:b") }];
        assert!(save(dir.path(), &bad).is_err());

        bad.chain = (0..13).map(|n| row(&format!("p:m{n}"))).collect();
        assert!(save(dir.path(), &bad).unwrap_err().contains("at most"));

        assert!(!path(dir.path()).exists(), "a refused save writes nothing");
    }

    #[test]
    fn a_broken_file_is_reported_and_never_crashes_the_load() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(path(dir.path()), "[[chain]]\nmodel = \n").unwrap();

        let (rotation, warning) = load(dir.path());

        assert!(rotation.is_empty());
        assert!(warning.is_some_and(|text| text.contains("is not valid")));
    }
}
