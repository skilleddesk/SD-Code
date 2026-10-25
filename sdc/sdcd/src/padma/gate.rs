//! **Finish Gate (P1-2, switch `padma.finish_gate`, G1/G3)**: a turn that was asked to *do* something does not end on the model saying "done".
//! It ends when (a) the done-when line has evidence - a file changed or a command ran this turn - or (b) the model says, in words, why it cannot
//! (a question back, a refusal, "nothing to change"), or (c) the step budget is spent (checkpoint and "continue"), or (d) the person cancelled.
//! A plain "done" with no evidence goes back to the model as a signal, twice at most, and then the turn ends *blocked with the reason* instead
//! of looking finished. A question, a chat message or an explanation is never held: it is one step, as before.

/// The most a reply is sent back for lacking evidence in one turn.
pub const MAX_RETRIES: u32 = 2;

/// What the turn showed when the model wrote a reply with no tool call.
pub struct Facts<'a> {
    /// The person's own words (not the context pack in front of them).
    pub request: &'a str,
    pub reply: &'a str,
    /// Files changed this turn.
    pub edits: usize,
    /// A command ran this turn.
    pub ran: bool,
    /// The turn is a research turn, or the person already stopped it: nothing to hold.
    pub exempt: bool,
    /// Times this turn has already been sent back.
    pub retries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The turn may end.
    Finish,
    /// Not yet: tell the model, and take another step.
    Retry(String),
    /// The turn ends, honestly: nothing was done, and this is why.
    Block(String),
}

/// Verbs that ask for work to be done (the Scope Classifier, P1-3, reads the same family for its labels). Whole words, any case.
const DO: &[&str] = &[
    "fix", "add", "create", "write", "implement", "change", "update", "rename", "refactor", "remove", "delete", "make", "install", "set up", "setup", "replace", "move", "convert", "generate", "patch", "edit", "modify", "run",
    "ঠিক করো", "ঠিক করে", "বানাও", "লেখো", "যোগ করো", "পরিবর্তন করো", "মুছে", "তৈরি করো", "চালাও",
    "thik koro", "thik kore", "banao", "likho", "joga koro", "jog koro", "poriborton koro", "muche", "toiri koro", "chalao",
];

/// A reply that says the work needs nothing, or that the model cannot or must ask: a sentence, not a "done".
const WORDS_INSTEAD: &[&str] = &[
    "already", "no change", "nothing to change", "no changes", "not needed", "no need to", "cannot", "can't", "can not", "unable", "not possible", "i need", "could you", "please provide", "which file", "which one", "do you want", "would you like",
    "আগে থেকেই", "প্রয়োজন নেই", "পারছি না", "পারবো না", "বলুন", "কোনটা",
];

fn has_word(lower: &str, needle: &str) -> bool {
    let mut from = 0;

    while let Some(at) = lower[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before = lower[..start].chars().next_back();
        let after = lower[end..].chars().next();
        let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());

        if edge(before) && (edge(after) || !needle.is_ascii()) {
            return true;
        }

        from = end;
    }

    false
}

/// Whether the request asks for work to be done (and is not a question).
pub fn asks_for_work(request: &str) -> bool {
    let head: String = request.chars().take(400).collect();
    let lower = head.to_lowercase();

    if lower.trim_start().starts_with('/') || lower.trim_end().ends_with('?') || lower.trim_end().ends_with('？') {
        return false;
    }

    DO.iter().any(|verb| has_word(&lower, verb))
}

/// The done-when line a request gets: work that is shown by a change, or an answer.
pub fn criteria(request: &str) -> Vec<String> {
    if asks_for_work(request) {
        vec!["the work is done: a file was changed or a command ran, and the result is shown".to_string()]
    } else {
        vec!["the request is answered".to_string()]
    }
}

pub fn decide(facts: &Facts) -> Verdict {
    if facts.exempt || !asks_for_work(facts.request) || facts.edits > 0 || facts.ran {
        return Verdict::Finish;
    }

    let reply = facts.reply.trim().to_lowercase();

    if reply.ends_with('?') || reply.ends_with('？') || WORDS_INSTEAD.iter().any(|words| has_word(&reply, words)) {
        return Verdict::Finish;
    }

    if facts.retries < MAX_RETRIES {
        return Verdict::Retry(
            "[SDC: you were asked to do something, and nothing has been changed or run yet. Make the change with a tool call now, or say in one sentence why it cannot be done.]".to_string(),
        );
    }

    Verdict::Block("the request asked for a change, but no file was changed and no command was run".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(request: &'a str, reply: &'a str) -> Facts<'a> {
        Facts { request, reply, edits: 0, ran: false, exempt: false, retries: 0 }
    }

    #[test]
    fn work_is_told_from_a_question_in_three_languages() {
        for work in ["fix the failing test in billing.py", "add a login page", "billing.py ঠিক করো", "login page banao", "Please update the README"] {
            assert!(asks_for_work(work), "{work}");
        }

        for other in ["check the build", "hello", "what does this function do?", "why is the build slow?", "explain the router", "thanks", "show me the prefix of this word"] {
            assert!(!asks_for_work(other), "{other}");
        }

        assert!(!asks_for_work("/plan add pagination to the orders list"), "a slash command is its own mode");
        assert!(!asks_for_work("the prefix of a word"), "a verb inside a longer word is not a verb");
    }

    #[test]
    fn done_without_evidence_goes_back_twice_then_ends_blocked() {
        let mut turn = facts("fix the bug in a.py", "Done. The bug is fixed.");

        assert!(matches!(decide(&turn), Verdict::Retry(_)));

        turn.retries = 1;
        assert!(matches!(decide(&turn), Verdict::Retry(_)));

        turn.retries = 2;
        assert!(matches!(decide(&turn), Verdict::Block(reason) if reason.contains("no file was changed")));
    }

    #[test]
    fn evidence_lets_the_turn_end() {
        let mut turn = facts("fix the bug in a.py", "Done.");

        turn.edits = 1;
        assert_eq!(decide(&turn), Verdict::Finish);

        turn.edits = 0;
        turn.ran = true;
        assert_eq!(decide(&turn), Verdict::Finish, "a command that ran is evidence for 'run the tests'");
    }

    #[test]
    fn a_question_back_a_refusal_or_nothing_to_change_is_an_answer() {
        for reply in ["Which file do you mean?", "I can't edit that file: it is protected.", "The function is already correct, no change needed.", "আমি ফাইলটা পারছি না খুলতে।"] {
            assert_eq!(decide(&facts("fix the bug", reply)), Verdict::Finish, "{reply}");
        }
    }

    #[test]
    fn a_question_or_a_chat_turn_is_never_held() {
        assert_eq!(decide(&facts("what does a.py do?", "It parses the file.")), Verdict::Finish);
        assert_eq!(decide(&facts("hello", "Hi!")), Verdict::Finish);

        let mut research = facts("fix it", "Done.");

        research.exempt = true;
        assert_eq!(decide(&research), Verdict::Finish);
    }

    #[test]
    fn the_done_when_line_follows_the_request() {
        assert!(criteria("add a button")[0].contains("changed"));
        assert!(criteria("what is a monad?")[0].contains("answered"));
    }
}
