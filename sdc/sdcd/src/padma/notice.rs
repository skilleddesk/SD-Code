//! **UI notices (P0-6, switch `padma.clean_history`)**: what SDC itself says to the person - the GPU share of the model, a slow model - is
//! shown in the window and is never part of the answer, the stored turn or the model's history.

/// The GPU-offload notice for a loaded model, read once per turn from `ollama ps`; `None` when the model is fully on the GPU or the
/// machine has unified memory.
pub fn gpu_notice(model: &str) -> Option<String> {
    #[cfg(test)]
    if let Some(forced) = FORCED_GPU.with(|forced| forced.borrow().clone()) {
        return Some(forced);
    }

    let ps: Option<serde_json::Value> = crate::engines::ollama::call("/api/ps", None).ok().and_then(|text| serde_json::from_str(&text).ok());
    let unified = crate::sme::tuner::detect().unified;

    ps.and_then(|ps| crate::sme::tuner::gpu_share(&ps, model)).and_then(|share| crate::sme::tuner::spill_warning(share, unified))
}

#[cfg(test)]
thread_local! {
    static FORCED_GPU: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Test seam: what `gpu_notice` answers on this thread (the real one asks the local Ollama).
#[cfg(test)]
pub fn force_gpu_notice(text: Option<&str>) {
    FORCED_GPU.with(|forced| *forced.borrow_mut() = text.map(str::to_string));
}

/// The openings of the italic lines SDC itself used to write into the answer text (before `padma.clean_history`).
const LEGACY: [&str; 4] = ["SDC finished a step that was interrupted", "SDC put the files back after an interrupted step", "Only ", "This model just slowed"];

/// An answer stored before notices became events, without the `_(...)_` lines SDC itself put into it. The model never sees them again.
pub fn strip(answer: &str) -> String {
    if !answer.contains("_(") {
        return answer.to_string();
    }

    let ours = |paragraph: &str| {
        let line = paragraph.trim();

        line.starts_with("_(") && line.ends_with(")_") && LEGACY.iter().any(|opening| line[2..].starts_with(opening))
    };

    let kept: Vec<&str> = answer.split("

").filter(|paragraph| !ours(paragraph)).collect();

    kept.join("

").trim_start().to_string()
}

#[cfg(test)]
mod strip_tests {
    use super::strip;

    #[test]
    fn our_notice_lines_are_removed_and_the_answer_is_not_touched() {
        let stored = "_(Only 86% of this model is on the GPU; the rest runs on the CPU.)_

It says hi.

_(an aside the model wrote)_

Bye.";

        assert_eq!(strip(stored), "It says hi.

_(an aside the model wrote)_

Bye.");
        assert_eq!(strip("plain"), "plain");
    }
}
