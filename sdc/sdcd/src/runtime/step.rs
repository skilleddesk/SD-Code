//! **Step Runtime v1 (P1-2, switch `edit.transaction`)**: every edit the agent makes goes through the Step Transaction.
//!
//! One call that changes files - `write_file`, `edit_file`, `apply_patch` - is one step. Before it touches the project:
//!
//! 1. **The syntax gate (fail closed).** Each file the step writes is parsed by the Code Graph's grammar. If the new text has a syntax
//!    error the old text did not, the whole step is refused and *nothing* is written; the answer names the file, the line and what is
//!    there. A file that was already broken may be written (a repair in progress must not be blocked). A file in a language the graph
//!    does not know is not judged. To keep a grammar's gap from ever deadlocking an agent, sending the **same text again** after a
//!    refusal writes it - and the result says it was written over the gate.
//! 2. **The commit.** The edits go to the real tree all at once or not at all: the base of every file is checked (a file changed meanwhile
//!    is a refusal, not an overwrite), a journal with pre-images is made durable first, each file is renamed into place, and a crash at
//!    any instant is undone at the next start ([`super::transaction::recover`], which the agent calls before a turn begins).
//!
//! A step that changes five files and fails on the fourth no longer leaves three of them changed - which is what `apply_patch` used to do.
//! A remote (SSH) folder cannot be transacted from here and keeps writing file by file; the gate does not apply there either.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use super::transaction::{Outcome, Transaction, TxError};
use crate::graph::parse::{self, Lang};

/// A step refused, with the sentence the model is given.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal(pub String);

/// Files above this are not parsed for the gate.
const GATE_LIMIT: usize = 1_000_000;

/// Languages whose grammar is trusted to judge a file. (Bash is left out: heredocs and exotic syntax make false alarms likely.)
fn gated(lang: Lang) -> bool {
    !matches!(lang, Lang::Bash)
}

/// Texts the agent insisted on after a refusal, keyed by file and content: the second time it is let through.
fn insisted() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

    SET.get_or_init(Default::default)
}

fn key(path: &str, text: &str) -> String {
    parse::sha(&format!("{path}\u{0}{text}"))
}

/// The syntax gate for one file: `Some(problem)` when writing `new` over `old` would newly break it.
pub fn gate(path: &str, old: Option<&str>, new: &str) -> Option<String> {
    let lang = Lang::of(path).filter(|lang| gated(*lang))?;

    if new.len() > GATE_LIMIT {
        return None;
    }

    let tree = parse::parse(lang, new)?;
    let errors = parse::syntax_errors(&tree, new, 3);

    if errors.is_empty() {
        /* N4-3: Python's grammar lets `def f():` with nothing indented under it parse; Python itself does not */
        if crate::v3::on("nr.edit_gate") && old.and_then(|old| crate::navaratna::gate::empty_python_block(path, old)).is_none() {
            if let Some(line) = crate::navaratna::gate::empty_python_block(path, new) {
                return Some(format!("{path} would have an IndentationError: the block opened before line {line} has nothing indented in it"));
            }
        }

        return None;
    }

    if old.is_some_and(|old| old.len() <= GATE_LIMIT && parse::parse(lang, old).is_some_and(|tree| tree.root_node().has_error())) {
        return None;
    }

    let first = &errors[0];
    let more = if errors.len() > 1 { format!(" (and {} more)", errors.len() - 1) } else { String::new() };

    Some(format!("{path} would have a syntax error at line {} (near `{}`){more}", first.line, first.near))
}

/// Applies one step: `edits` are `(project-relative path, Some(new text) | None to delete)`.
/// Answers the files that really changed, or the refusal. `id` names the transaction (a step id).
pub fn apply(root: &Path, id: &str, edits: &[(String, Option<String>)]) -> Result<Vec<String>, Refusal> {
    let mut problems = Vec::new();

    for (path, text) in edits {
        let Some(text) = text else {
            continue;
        };
        let old = std::fs::read(root.join(path)).ok().map(|bytes| String::from_utf8_lossy(&bytes).to_string());

        if let Some(problem) = gate(path, old.as_deref(), text) {
            let wanted = key(path, text);
            let mut again = insisted().lock().unwrap_or_else(|poison| poison.into_inner());

            /* N4-3: for a trusted grammar a broken edit is never let through, not even sent twice */
            if !crate::navaratna::gate::may_insist(path) || !again.remove(&wanted) {
                if again.len() > 512 {
                    again.clear();
                }

                again.insert(wanted);
                problems.push(problem);
            }
        }
    }

    if !problems.is_empty() {
        return Err(Refusal(format!(
            "Not written - nothing in this step was changed. {}. Fix the code and send it again. (If you are sure it is valid and the checker is wrong, send exactly the same text again and it will be written.)",
            problems.join("; ")
        )));
    }

    let mut transaction = Transaction::begin_light(root, id).map_err(|error| Refusal(error.to_string()))?;

    for (path, text) in edits {
        let staged = match text {
            Some(text) => transaction.write(path, text.as_bytes()),
            None => transaction.delete(path),
        };

        staged.map_err(|error| Refusal(error.to_string()))?;
    }

    match transaction.finish(true) {
        Ok(receipt) if receipt.outcome == Outcome::Committed => Ok(receipt.changed),
        Ok(_) => Ok(Vec::new()),
        Err(TxError::Conflict(paths)) => Err(Refusal(format!("Not written: {} changed on disk while this step was being made, so overwriting it could lose someone's work. Read the file again and redo the edit.", paths.join(", ")))),
        Err(error) => Err(Refusal(error.to_string())),
    }
}

#[cfg(test)]
mod tests;
