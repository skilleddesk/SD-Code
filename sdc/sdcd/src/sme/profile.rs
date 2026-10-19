//! **Step profiles (P1-4, switch `sme.step_profile`)**: one request's sampling and thinking are chosen by what the step is doing, not by
//! one setting for the whole turn.
//!
//! A local model's vendor documents two modes, and SDC used neither deliberately: Qwen's page asks for `temperature 0.7, top_p 0.8,
//! top_k 20, presence_penalty 1.5` for general chat and `temperature 0.6, top_p 0.95, top_k 20` for precise coding **in thinking mode**
//! (and `0.7` for non-thinking), with the page's own warning that a greedy setting (`temperature 0`) repeats itself and loops.
//!
//! So a step that **plans** thinks and samples at the coding setting; a step that **formats** (a rewrite of a file it just read, a
//! rename, a one-line edit) does not think at all - thinking there is paid for and thrown away; a step that **patches** samples low;
//! a step that is **stuck** is nudged up a little, so a loop it is in does not repeat word for word; and **review** is the only step
//! that asks for the general-chat setting, because reviewing is reading, not producing.
//!
//! A model whose family is not known gets nothing: [`Profile::neutral`] leaves the body as it was. Which numbers a given model really
//! wants is a bench question (`bench/`), which is why the switch is **off by default** - a step profile that was never measured must
//! not quietly change what the owner's turns do.

use serde_json::Value;

/// What a step is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Thinking ahead: the first step of a turn, a plan, a decision between two approaches.
    Plan,
    /// Reading: searching, grepping, opening files.
    Read,
    /// Changing code.
    Patch,
    /// Mechanical rewriting where the answer is already known: a rename, a formatting pass, a whole-file rewrite.
    Format,
    /// A step after a failure: the same thing again with a nudge.
    Debug,
    /// Reading someone else's work and saying what is wrong with it.
    Review,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Plan => "plan",
            Step::Read => "read",
            Step::Patch => "patch",
            Step::Format => "format",
            Step::Debug => "debug",
            Step::Review => "review",
        }
    }

    /// The step from the tools the previous reply called (`None` when it called none: that is a fresh plan).
    pub fn of_tools(tools: &[String], failed: bool) -> Self {
        if failed {
            return Step::Debug;
        }

        let has = |name: &str| tools.iter().any(|tool| tool == name);

        if tools.is_empty() {
            return Step::Plan;
        }

        if has("write_file") || has("apply_patch") || has("patch_symbol") || has("edit_file") {
            /* a whole-file write of content the model just read, or a symbol patch with no thinking behind it */
            return if has("write_file") { Step::Format } else { Step::Patch };
        }

        if has("review") || has("diff") || has("git_diff") {
            return Step::Review;
        }

        if has("run_command") || has("read_file") || has("grep") || has("find") || has("code_graph") || has("find_symbol") {
            return Step::Read;
        }

        Step::Plan
    }
}

/// One step's sampling. `None` for a knob means "leave the request as it is".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub step: Step,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<u32>,
    pub presence_penalty: Option<f64>,
    /// `Some(false)` turns thinking off for this step; `Some(true)` on; `None` leaves the turn's choice.
    pub thinking: Option<bool>,
    pub label: &'static str,
}

impl Profile {
    /// The name the trace shows: the step, and what the vendor's page calls the setting.
    pub fn name(&self) -> String {
        if self.temperature.is_none() {
            self.step.name().to_string()
        } else {
            format!("{} ({})", self.step.name(), self.label)
        }
    }

    /// Nothing is changed.
    pub fn neutral(step: Step) -> Self {
        Self { step, temperature: None, top_p: None, top_k: None, presence_penalty: None, thinking: None, label: "neutral" }
    }

    /// The vendor's own numbers (the Qwen3 page), read for the model family that documents them.
    pub fn documented(step: Step, thinking: bool) -> Self {
        let (temperature, top_p, top_k, presence_penalty, label) = match step {
            Step::Plan | Step::Read => (0.6, 0.95, 20, None, "thinking, precise coding"),
            Step::Patch | Step::Debug => (0.6, 0.95, 20, Some(0.5), "thinking, precise coding, low repeat"),
            Step::Review => (0.7, 0.8, 20, Some(1.5), "general chat"),
            Step::Format => (0.7, 0.8, 20, None, "non-thinking, mechanical"),
        };

        Self {
            step,
            temperature: Some(temperature),
            top_p: Some(top_p),
            top_k: Some(top_k),
            presence_penalty,
            thinking: Some(thinking && step != Step::Format),
            label,
        }
    }

    /// The step's profile for a model. Only a family that documents its settings gets numbers; everything else is left alone.
    pub fn for_step(step: Step, family: &str, thinking: bool) -> Self {
        match family {
            "qwen" | "qwen3" => Self::documented(step, thinking),
            _ => Self::neutral(step),
        }
    }
}

/// The model family, from the model's name: `qwen3:8b` and `Qwen/Qwen3-8B` are the same family.
pub fn family_of(model: &str) -> &'static str {
    let name = model.to_ascii_lowercase();

    if name.contains("qwen3") || name.contains("qwen2.5") || name.contains("qwen") {
        "qwen"
    } else if name.contains("llama") {
        "llama"
    } else if name.contains("gemma") {
        "gemma"
    } else if name.contains("deepseek") {
        "deepseek"
    } else {
        "unknown"
    }
}

/// Puts the profile on a native Ollama request body (the one `ollama_native::body` built): its `options`, and `think` for the steps
/// that do not want it. A body that is not a local one is left alone - a cloud API's knobs are not this module's to turn.
pub fn apply(body: &mut Value, profile: &Profile) {
    if body.get("options").is_none() {
        return;
    }

    let options = &mut body["options"];

    if let Some(top_k) = profile.top_k {
        options["top_k"] = serde_json::json!(top_k);
    }

    for (key, value) in [("temperature", profile.temperature), ("top_p", profile.top_p), ("presence_penalty", profile.presence_penalty)] {
        if let Some(value) = value {
            options[key] = serde_json::json!(value);
        }
    }

    if let Some(thinking) = profile.thinking {
        body["think"] = serde_json::json!(thinking);
    }
}

/// The step the turn is on, for the request being built: set by the agent before each model call, read by the request's own layer.
/// One turn runs on one thread of the runtime, and a step is set and used inside the same call.
mod current {
    use std::cell::RefCell;

    thread_local! {
        static STEP: RefCell<Option<super::Step>> = const { RefCell::new(None) };
    }

    pub fn set(step: Option<super::Step>) {
        STEP.with(|cell| *cell.borrow_mut() = step);
    }

    pub fn get() -> Option<super::Step> {
        STEP.with(|cell| *cell.borrow())
    }
}

pub use current::{get as current_step, set as set_current_step};

/// The profile for the step being taken, for this model and this turn's thinking choice. `None` when the switch is off or the model's
/// family documents nothing.
pub fn for_request(model: &str, thinking: bool) -> Option<Profile> {
    let step = current_step()?;
    let profile = Profile::for_step(step, family_of(model), thinking);

    (profile.temperature.is_some()).then_some(profile)
}

#[cfg(test)]
mod tests;
