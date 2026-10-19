//! **The confidence signal (P1-13, switch `sme.confidence`)**: how likely this step is to be wrong, from what the turn has already shown -
//! and what SDC does about it. Calibrated, so it only ever *adds* caution: a signal can make SDC slower and more explicit, never more
//! confident than it was.
//!
//! Two families of signal, because a local model exposes different things:
//!
//! * **When the server gives numbers** (token log-probabilities, `ollama`'s `logprobs` when a model supports them), the reply's mean
//!   log-probability is the model's own doubt, and its entropy is its spread - [`Signals::entropy`]. A model that is guessing has a flat
//!   distribution, and a flat distribution on a code identifier is worth a second look.
//! * **When it gives none** (the usual case), the structure of the turn is the signal: the same command failing with the same error a
//!   second time, an edit that puts back what the last edit took away (oscillation), a reply whose tool arguments did not parse
//!   (a schema break), the same tool on the same input twice.
//!
//! What is done with it: [`Caution::steer`] is one sentence to the model ("you have run this twice with the same failure: read the file
//! instead of guessing"), and [`Caution::stop`] is the point at which the turn should end with what is known rather than loop. A
//! malformed reply is not a caution but a **repair**: the same call is retried once with thinking off and temperature 0, which is the
//! cheapest fix for a model that thought itself into an invalid JSON.
//!
//! The calibration is measured ([`auroc`]) on the bench's step traces: the signal must lift the failures above the successes, and the test
//! below requires an area under the ROC curve of at least 0.7 on a synthetic trace with the shapes this module watches for.

/// One step of the turn, as the detector sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The tool called (empty for a reply that called none).
    pub tool: String,
    /// A short identity of its input: the command, the path.
    pub target: String,
    /// The result was a refusal or an error.
    pub failed: bool,
    /// The first line of what went wrong (empty when nothing did).
    pub error: String,
    /// The reply's tool arguments did not parse (a malformed reply).
    pub malformed: bool,
}

impl Step {
    pub fn new(tool: &str, target: &str, failed: bool, error: &str) -> Self {
        Self { tool: tool.to_string(), target: target.to_string(), failed, error: error.lines().next().unwrap_or_default().to_string(), malformed: false }
    }

    pub fn malformed(tool: &str) -> Self {
        Self { tool: tool.to_string(), target: String::new(), failed: true, error: String::new(), malformed: true }
    }
}

/// What the turn's own history says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signals {
    /// The same tool on the same target, failing with the same error, more than once.
    pub repeated_error: usize,
    /// Edits that put back what an earlier edit of the turn took away.
    pub oscillation: usize,
    /// Replies whose arguments did not parse.
    pub schema_breaks: usize,
    /// The same tool on the same target twice in a row, whatever the result.
    pub repeats: usize,
    /// The reply's mean log-probability's entropy, when the server reports log-probabilities.
    pub entropy: Option<f32>,
}

/// How much caution, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Caution {
    /// 0: nothing to say. 1: say one sentence. 2: the next step should be a different kind of step. 3: stop and report.
    pub level: u8,
    pub reasons: Vec<String>,
    pub signals: Signals,
}

impl Caution {
    pub fn is_quiet(&self) -> bool {
        self.level == 0
    }

    /// The sentence for the model - once, when there is something to say.
    pub fn steer(&self) -> Option<String> {
        if self.level == 0 || self.reasons.is_empty() {
            return None;
        }

        Some(match self.level {
            1 => format!("Careful: {}. Change one thing before the next attempt.", self.reasons.join("; ")),
            2 => format!("This step is going in circles: {}. Do something different: read the file or the error again before changing anything.", self.reasons.join("; ")),
            _ => format!("This is not working: {}. Stop and say plainly what is known, what was tried and what is still unknown.", self.reasons.join("; ")),
        })
    }

    /// Should the turn end here with what is known?
    pub fn stop(&self) -> bool {
        self.level >= 3
    }
}

/// Reads a history into signals and a level. Only structural signals that are cheap and certain; a doubt is never inferred from prose.
pub fn assess(history: &[Step], entropy: Option<f32>) -> Caution {
    let mut signals = Signals { entropy, ..Default::default() };

    for (index, step) in history.iter().enumerate() {
        if step.malformed {
            signals.schema_breaks += 1;
        }

        /* the same tool, target and error, seen before in this turn */
        if step.failed && history[..index].iter().any(|earlier| earlier.failed && earlier.tool == step.tool && earlier.target == step.target && earlier.error == step.error && !step.error.is_empty()) {
            signals.repeated_error += 1;
        }

        if index > 0 {
            let previous = &history[index - 1];

            if previous.tool == step.tool && previous.target == step.target {
                signals.repeats += 1;
            }

            /* an edit to A, then to B, then to A again: the turn is undoing itself */
            if index >= 2 && is_edit(&step.tool) && is_edit(&history[index - 1].tool) && is_edit(&history[index - 2].tool) && step.target == history[index - 2].target && step.target != history[index - 1].target {
                signals.oscillation += 1;
            }
        }
    }

    let mut reasons = Vec::new();

    if signals.schema_breaks > 0 {
        reasons.push(format!("a reply did not parse as a tool call ({} time(s))", signals.schema_breaks));
    }

    if signals.repeated_error > 0 {
        reasons.push(format!("the same error came back {} time(s)", signals.repeated_error + 1));
    }

    if signals.oscillation > 0 {
        reasons.push(format!("an edit is undoing an earlier one ({} time(s))", signals.oscillation));
    }

    if signals.repeats > 1 {
        reasons.push("the same call was made twice in a row".to_string());
    }

    if entropy.is_some_and(|entropy| entropy > 2.0) {
        reasons.push("the model's own distribution is flat, which is a guess".to_string());
    }

    /* a failure on its own is not a doubt - it is what work looks like - but the model should hear it in one sentence */
    if reasons.is_empty() && history.iter().any(|step| step.failed) {
        reasons.push("a step failed".to_string());
    }

    let level = match (&signals, reasons.len()) {
        (_, 0) => 0,
        /* `repeated_error` counts the repeats after the first sighting: 1 is the same error twice, 2 is three times */
        (signals, _) if signals.repeated_error >= 2 || signals.repeats >= 3 => 3,
        (signals, _) if signals.repeated_error >= 1 || signals.oscillation > 0 || signals.schema_breaks >= 2 => 2,
        _ => 1,
    };

    Caution { level, reasons, signals }
}

/// A tool that changes files.
fn is_edit(tool: &str) -> bool {
    matches!(tool, "write_file" | "edit_file" | "apply_patch" | "patch_symbol")
}

/// A malformed reply's repair: the same call again, without thinking and at temperature 0, once. Answers whether a reply should be
/// repaired at all - a second malformed reply is a failed step, not a third try.
pub fn repair_once(attempts: usize) -> bool {
    attempts == 0
}

/// The area under the ROC curve of a signal: how well a score separates the failures (true) from the others (false).
/// 0.5 is a coin toss, 1.0 is perfect. Ties count half, which is what makes it a fair measure of a coarse signal.
pub fn auroc(scored: &[(f32, bool)]) -> f32 {
    let positives = scored.iter().filter(|(_, failed)| *failed).count();
    let negatives = scored.len() - positives;

    if positives == 0 || negatives == 0 {
        return 0.5;
    }

    let mut area = 0.0;

    for (score, _) in scored.iter().filter(|(_, failed)| *failed) {
        for (other, other_failed) in scored.iter().filter(|(_, failed)| !*failed) {
            area += if score > other {
                1.0
            } else if *score == *other {
                0.5
            } else {
                0.0
            };

            let _ = other_failed;
        }
    }

    area / (positives * negatives) as f32
}

#[cfg(test)]
mod tests;
