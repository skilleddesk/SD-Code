//! **Plan mode** (v3 W7b): the person opens a request with `/plan` (or `@plan`) and SDC answers with the plan
//! instead of doing the work - it may read anything it likes, and every tool that changes something is refused
//! with a word to the person. The plan comes back as the turn's answer; the next message ("go on", or a plain
//! request without the marker) runs it.
//!
//! Plan mode is not a switch: it is what the person asked for in this message, and it holds only for that turn.

/// Does this message ask for a plan?
pub fn is_plan(text: &str) -> bool {
    let trimmed = text.trim_start();

    trimmed.starts_with("/plan") || trimmed.starts_with("@plan") || trimmed.to_lowercase().starts_with("plan mode:")
}

/// The request without the marker.
pub fn strip(text: &str) -> String {
    let trimmed = text.trim_start();
    let lowered = trimmed.to_lowercase();

    for marker in ["/plan", "@plan", "plan mode:"] {
        if lowered.starts_with(marker) {
            return trimmed[marker.len()..].trim_start_matches([':', ',', ' ']).trim().to_string();
        }
    }

    trimmed.to_string()
}

/// What the turn is told.
pub fn instruction() -> &'static str {
    "[SDC plan mode: the person asked for a plan, not for the work. Read whatever you need, but change nothing - every tool that writes, edits or runs a command that changes files or servers is refused while you are in plan mode. Answer with the plan itself: the files you would touch and what you would do in each, the order, how you would check it worked, and the one part you are unsure about. Keep it short enough to read at once.]"
}

/// The refusal a writing tool answers with while plan mode is on.
pub fn refusal(tool: &str) -> String {
    format!("[SDC plan mode: `{tool}` changes something, so it was not run. Plan mode means the plan comes first - the person reads it, then sends the go-ahead.]")
}

#[cfg(test)]
thread_local! {
    /// A test's own plan-mode state, so tests running side by side do not see each other's turns.
    static TEST_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    static ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Is the turn running now in plan mode?
pub fn active() -> bool {
    #[cfg(test)]
    if TEST_ACTIVE.with(|active| active.get()) {
        return true;
    }

    ACTIVE.with(|active| active.get())
}

/// Holds plan mode for as long as the turn runs; leaving it (or the turn failing) puts the tools back.
pub struct PlanMode {
    was: bool,
}

impl PlanMode {
    pub fn begin(on: bool) -> Self {
        let was = ACTIVE.with(|active| active.get());

        if on {
            ACTIVE.with(|active| active.set(true));

            #[cfg(test)]
            TEST_ACTIVE.with(|active| active.set(true));
        }

        Self { was }
    }
}

impl Drop for PlanMode {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(self.was));

        #[cfg(test)]
        TEST_ACTIVE.with(|active| active.set(false));
    }
}

/// Does this tool change something? (Plan mode refuses exactly these.)
pub fn changes_something(tool: &str) -> bool {
    matches!(
        tool,
        "write_file" | "edit_file" | "apply_patch" | "run_command" | "start_process" | "fleet_run" | "remember" | "delegate" | "parallel_agents" | "spawn_agent"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_is_the_persons_own_marker_and_holds_only_for_that_message() {
        assert!(is_plan("/plan add pagination to the orders list"));
        assert!(is_plan("  @plan make the tests pass"));
        assert!(!is_plan("add pagination to the orders list"));

        assert_eq!(strip("/plan add pagination"), "add pagination");
        assert_eq!(strip("@plan:  fix the build"), "fix the build");
        assert_eq!(strip("just do it"), "just do it");
        assert!(instruction().contains("change nothing"));
        assert!(changes_something("edit_file") && changes_something("run_command") && !changes_something("read_file") && !changes_something("grep"));
        assert!(refusal("edit_file").contains("plan mode"));
    }
}
