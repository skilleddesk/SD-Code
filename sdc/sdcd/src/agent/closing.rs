//! The end of a turn, accounted for.
//!
//! Goal 1 asks for one last check after the model stops: "did I do what you asked?". `turn.contract`
//! gets that answer by asking the model for three parts, and on a 9B model that is a whole extra
//! call - about fifteen seconds - at the end of every turn that touched a file, spent describing
//! work the harness watched happen.
//!
//! The harness already holds those three parts: how many files the turn wrote, which checks passed
//! afterwards, and which steps of the model's own checklist were never closed. So the answer is
//! read here instead of asked: it cannot be flattered, it cannot be forgotten, and it costs nothing.
//!
//! The line is deliberately short - it sits at the end of the footer, beside the token count.

/// What a finished turn leaves behind that is worth reading back.
///
/// `steps::Turn` is a turn in progress; this is the small part of a finished one that is worth
/// saying out loud, and the name keeps the two apart at a call site.
pub struct Ended<'a> {
    /// How many files the turn wrote (the same set the finish gate and the edit guard use).
    pub files: usize,
    /// Checks that passed after a change: a test run, a build, a lint.
    pub passed: &'a [String],
    /// Steps of the model's own checklist that are still `pending` or `in_progress`.
    pub open: &'a [String],
}

/// The steps of an `update_plan` checklist that were never closed.
///
/// A step with no text is skipped: the tool accepts one so that a model can clear a row, and an
/// empty row is not something a person asked for.
pub fn open_steps(plan: Option<&serde_json::Value>) -> Vec<String> {
    let Some(steps) = plan.and_then(|plan| plan.as_array()) else { return Vec::new() };
    steps
        .iter()
        .filter(|step| step.get("status").and_then(|status| status.as_str()) != Some("done"))
        .filter_map(|step| step.get("text").and_then(|text| text.as_str()))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .collect()
}

/// The closing line, or `None` for a turn with nothing to account for: a question, a read, a look.
pub fn line(turn: Ended<'_>) -> Option<String> {
    if turn.files == 0 && turn.passed.is_empty() && turn.open.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    if turn.files > 0 {
        parts.push(format!("{} file{}", turn.files, if turn.files == 1 { "" } else { "s" }));
    }
    if !turn.passed.is_empty() {
        let checks = turn.passed.len();
        parts.push(format!("{checks} check{} passed", if checks == 1 { "" } else { "s" }));
    }
    /* The part that makes this a check and not a boast: what the model planned and did not do. */
    if !turn.open.is_empty() {
        parts.push(format!("still open: {}", open_rest(turn.open)));
    }

    Some(format!(" · closed: {}", parts.join(", ")))
}

/// The first open step, shortened to stay on one line, and how many others there are.
fn open_rest(open: &[String]) -> String {
    let first = &open[0];
    let short = if first.chars().count() > 40 {
        format!("{}…", first.chars().take(39).collect::<String>())
    } else {
        first.clone()
    };
    match open.len() - 1 {
        0 => short,
        rest => format!("{short} (+{rest})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn a_question_has_nothing_to_account_for() {
        assert!(line(Ended { files: 0, passed: &[], open: &[] }).is_none());
    }

    #[test]
    fn a_change_with_a_check_after_it_is_counted() {
        let line = line(Ended { files: 1, passed: &owned(&["cargo test -q"]), open: &[] }).unwrap();
        assert_eq!(line, " · closed: 1 file, 1 check passed");
    }

    #[test]
    fn a_step_the_model_left_open_is_named() {
        let line = line(Ended {
            files: 2,
            passed: &[],
            open: &owned(&["Run the tests", "Ship it"]),
        })
        .unwrap();
        assert_eq!(line, " · closed: 2 files, still open: Run the tests (+1)");
    }

    #[test]
    fn a_long_open_step_stays_on_one_line() {
        let long = "x".repeat(90);
        let line = line(Ended { files: 0, passed: &[], open: &[long] }).unwrap();
        assert!(line.ends_with('…'), "{line}");
        assert!(line.chars().count() < 70, "{line}");
    }

    #[test]
    fn open_steps_reads_the_checklist_and_skips_what_is_done() {
        let plan = json!([
            { "text": "Read the guard", "status": "done" },
            { "text": "Fix the retry", "status": "in_progress" },
            { "text": " ", "status": "pending" },
            { "text": "Run the tests", "status": "pending" },
        ]);
        assert_eq!(open_steps(Some(&plan)), owned(&["Fix the retry", "Run the tests"]));
    }

    #[test]
    fn open_steps_of_no_plan_is_no_steps() {
        assert!(open_steps(None).is_empty());
    }
}
