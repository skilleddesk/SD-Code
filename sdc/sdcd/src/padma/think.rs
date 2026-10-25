//! **Thinking policy per step kind (P2-2, switch `padma.think_policy`, off until the bench says)**: a small model that thinks on every step
//! spends its window and its minutes on reasoning that decides nothing - and a thinking model sometimes spends the whole answer on it and writes
//! nothing. Each step has a kind; the model's profile (P2-1) says for each kind whether it thinks, and that decision goes to the model through its
//! own switch (Ollama's `think` flag). Tool-call, map-review and boilerplate steps do not think; blueprint, repair-after-fail and reduce-report
//! steps do, inside the N7-3 budget (what thinking may take counts toward the answer's reserve). A model that is not known to think is left alone.

use std::cell::Cell;

/// What a model step is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    ToolCall,
    MapReview,
    Boilerplate,
    Blueprint,
    RepairAfterFail,
    ReduceReport,
}

impl StepKind {
    /// The word the profile file uses for this kind.
    pub fn word(self) -> &'static str {
        match self {
            StepKind::ToolCall => "tool_call",
            StepKind::MapReview => "map_review",
            StepKind::Boilerplate => "boilerplate",
            StepKind::Blueprint => "blueprint",
            StepKind::RepairAfterFail => "repair_after_fail",
            StepKind::ReduceReport => "reduce_report",
        }
    }
}

impl StepKind {
    /// The kind a recipe file names, or `None` for a word that is no kind.
    pub fn from_word(word: &str) -> Option<StepKind> {
        [StepKind::ToolCall, StepKind::MapReview, StepKind::Boilerplate, StepKind::Blueprint, StepKind::RepairAfterFail, StepKind::ReduceReport].into_iter().find(|kind| kind.word() == word)
    }
}

thread_local! {
    static KIND: Cell<Option<StepKind>> = const { Cell::new(None) };
}

/// Says what the next model call is for (a recipe step names its own kind; the loop names a repair after a failed tool).
pub fn set(kind: Option<StepKind>) {
    KIND.with(|current| current.set(kind));
}

pub fn current() -> Option<StepKind> {
    KIND.with(Cell::get)
}

/// The kind of an ordinary loop step: a repair when the step before it failed, else a tool call when tools are offered, else none.
pub fn kind_in_loop(last_failed: bool, tools_offered: bool) -> Option<StepKind> {
    if last_failed {
        Some(StepKind::RepairAfterFail)
    } else if tools_offered {
        Some(StepKind::ToolCall)
    } else {
        None
    }
}

/// What to tell the model: `Some(true)` think, `Some(false)` do not, `None` leave its own default (also for a model not known to think).
pub fn decision(profile: &super::model::ModelProfile, kind: Option<StepKind>, capable: bool) -> Option<bool> {
    let kind = kind?;

    capable.then(|| profile.thinking.for_kind(kind.word()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::padma::model::ModelProfile;

    #[test]
    fn the_right_flag_goes_with_each_step_kind() {
        let profile = ModelProfile::default();

        for (kind, expected) in [
            (StepKind::ToolCall, false),
            (StepKind::MapReview, false),
            (StepKind::Boilerplate, false),
            (StepKind::Blueprint, true),
            (StepKind::RepairAfterFail, true),
            (StepKind::ReduceReport, true),
        ] {
            assert_eq!(decision(&profile, Some(kind), true), Some(expected), "{kind:?}");
        }
    }

    #[test]
    fn a_model_not_known_to_think_and_a_step_with_no_kind_are_left_alone() {
        let profile = ModelProfile::default();

        assert_eq!(decision(&profile, Some(StepKind::Blueprint), false), None);
        assert_eq!(decision(&profile, None, true), None);
    }

    #[test]
    fn the_profile_can_change_a_kind() {
        let mut profile = ModelProfile::default();

        profile.thinking.map_review = true;

        assert_eq!(decision(&profile, Some(StepKind::MapReview), true), Some(true));
    }

    #[test]
    fn the_loop_names_a_repair_after_a_failure_and_a_tool_call_otherwise() {
        assert_eq!(kind_in_loop(true, true), Some(StepKind::RepairAfterFail));
        assert_eq!(kind_in_loop(false, true), Some(StepKind::ToolCall));
        assert_eq!(kind_in_loop(false, false), None);
    }

    #[test]
    fn the_kind_is_remembered_for_the_next_call_only_on_this_thread() {
        set(Some(StepKind::Blueprint));
        assert_eq!(current(), Some(StepKind::Blueprint));
        assert_eq!(std::thread::spawn(current).join().unwrap(), None);

        set(None);
        assert_eq!(current(), None);
    }
}
