//! **Continuation binding (P1-5, switch `padma.continue_bind`, G7)**: "ok koro", "continue" or a pasted SDC error resumes the unfinished task
//! of the session instead of starting a new one; a short question about why it stopped is answered from the ledger.

/// The unfinished task of this session, as the ledger knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTask {
    pub id: String,
    pub goal: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// Continue the task with this id.
    Resume(String),
    /// A short question about why the task stopped: answer from the ledger and offer to continue.
    Explain(String),
    /// A new request: a new contract.
    New,
}

/// Messages that only say "go on" - English, Bengali, Banglish.
const CONTINUE: &[&str] = &[
    "continue", "go on", "keep going", "carry on", "resume", "proceed", "go ahead", "ok", "okay", "yes", "yes continue", "ok continue", "ok go on", "do it", "try again", "retry", "again",
    "ok koro", "okay koro", "ok kor", "koro", "kore fel", "kore felo", "cholo", "chalu koro", "chaliye jao", "abar koro", "abar try koro", "hmm koro", "hya koro", "han koro", "thik ache koro", "thik ache",
    "চালিয়ে যাও", "চালিয়ে যান", "আবার করো", "আবার চেষ্টা করো", "করো", "ঠিক আছে করো", "হ্যাঁ করো", "চলো", "এগিয়ে যাও", "কন্টিনিউ",
];

/// The first words of the errors SDC itself writes: a person who pastes one wants the work to go on.
const SDC_ERRORS: &[&str] = &["contextoverflow", "ollama failed", "blocked:", "paused:", "the model gave no answer", "sdc:", "error: ollama", "ollama error"];

/// Words that ask *why* it stopped.
const WHY: &[&str] = &["why", "what happened", "what went wrong", "kano", "keno", "ki hoyeche", "ki holo", "কেন", "কী হয়েছে", "কি হয়েছে", "কী হলো", "কি হলো", "থামল", "থামলো", "বন্ধ হলো"];

/// Words that say it is the task that is meant.
const STOPPED: &[&str] = &["stop", "fail", "block", "pause", "error", "wrong", "happen", "didn't finish", "not finish", "thamlo", "thamle", "thame", "hoyeche", "holo", "থাম", "বন্ধ", "হয়েছে", "হলো", "ফেইল"];

fn plain(message: &str) -> String {
    message
        .trim()
        .trim_matches(|c: char| matches!(c, '.' | '!' | ',' | '।' | '…' | '"' | '\'' | '`') || c.is_whitespace())
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decides what a message is, given the session's unfinished task (if any).
pub fn resolve(message: &str, active: Option<&ActiveTask>) -> Binding {
    let Some(task) = active else {
        return Binding::New;
    };
    let text = plain(message);

    if text.is_empty() {
        return Binding::New;
    }

    if CONTINUE.iter().any(|phrase| *phrase == text) {
        return Binding::Resume(task.id.clone());
    }

    if SDC_ERRORS.iter().any(|prefix| text.starts_with(prefix)) {
        return Binding::Resume(task.id.clone());
    }

    let short = text.chars().count() <= 80;
    let question = text.ends_with('?') || text.ends_with('？') || WHY.iter().any(|word| text.starts_with(word));

    if short && question && WHY.iter().any(|word| text.contains(word)) && STOPPED.iter().any(|word| text.contains(word)) {
        return Binding::Explain(task.id.clone());
    }

    Binding::New
}

/// What the ledger says about a task that stopped, in a few words.
pub fn explain(task: &ActiveTask, note: &str) -> String {
    let why = match (&task.last_error, note.trim()) {
        (Some(error), _) => format!("it stopped with: {error}"),
        (None, note) if !note.is_empty() => format!("it stopped: {note}"),
        _ => "it did not finish".to_string(),
    };

    format!("The last task was \"{}\" - {why}. Say \"continue\" and it goes on from where it stopped.", task.goal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active() -> ActiveTask {
        ActiveTask { id: "task-1".into(), goal: "review the whole project".into(), last_error: Some("ContextOverflow: too long".into()) }
    }

    #[test]
    fn a_go_on_in_any_of_three_languages_resumes() {
        for message in ["ok koro", "Continue", "go on.", "keep going!", "ঠিক আছে করো", "চালিয়ে যাও", "abar koro"] {
            assert_eq!(resolve(message, Some(&active())), Binding::Resume("task-1".into()), "{message}");
        }
    }

    #[test]
    fn a_pasted_sdc_error_resumes() {
        assert_eq!(resolve("ollama failed: connection reset", Some(&active())), Binding::Resume("task-1".into()));
        assert_eq!(resolve("ContextOverflow: this step is about 12397 tokens", Some(&active())), Binding::Resume("task-1".into()));
    }

    #[test]
    fn a_short_question_about_the_stop_is_explained() {
        assert_eq!(resolve("why did it stop?", Some(&active())), Binding::Explain("task-1".into()));
        assert_eq!(resolve("keno thamlo?", Some(&active())), Binding::Explain("task-1".into()));
        assert_eq!(resolve("কেন থামল?", Some(&active())), Binding::Explain("task-1".into()));
    }

    #[test]
    fn anything_else_is_a_new_request() {
        for message in ["write me a haiku about rust", "why is the sky blue?", "why is the sky blue and how do airplanes fly so high above the clouds in the very cold air?", "add a login page", ""] {
            assert_eq!(resolve(message, Some(&active())), Binding::New, "{message}");
        }

        assert_eq!(resolve("continue", None), Binding::New, "nothing to resume");
    }

    #[test]
    fn the_explanation_names_the_goal_and_the_reason() {
        let text = explain(&active(), "");

        assert!(text.contains("review the whole project") && text.contains("ContextOverflow") && text.contains("continue"));
    }
}
