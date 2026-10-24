//! **N9-1 - the Golden Trace** (switch `nr.golden_trace`, off).
//!
//! When a cloud model fixes a step and **every test passes**, the problem and its fix are kept in a local SQLite table: the problem's
//! signature (error class, symbol kind, language), the sliced error and the final diff. The next time a step has the same signature, at most
//! **one** such example goes into its prompt - so the cloud is asked less often for the same kind of problem. Three rules keep it honest:
//!
//! * only test-passing fixes are stored, and never a secret, a redacted placeholder, or anything touching a local-only project's secret paths;
//! * an example may be used only once the knowledge core's two-witness rule is met - two separate turns saw the same fix;
//! * an example that is followed by a failure twice is dropped.
//!
//! Each use is counted, so the receipt can say how often it helped.

use std::path::Path;

use rusqlite::{params, Connection};

use super::errslice::Found;
use crate::v3::knowledge_core::{self, Candidate};

const FILE: &str = ".sdc/golden.db";
/// Failures after use that drop an example.
pub const MAX_FAILURES: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub signature: String,
    pub sliced_error: String,
    pub final_diff: String,
    /// The turn that saw this fix - a witness.
    pub turn: String,
    /// Every test passed after the fix.
    pub tests_passed: bool,
    /// Files the fix touched (checked against the secret paths).
    pub touched: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    Stored { witnesses: usize },
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    pub id: i64,
    pub sliced_error: String,
    pub final_diff: String,
}

impl Example {
    /// The text that goes into the prompt.
    pub fn render(&self) -> String {
        format!("A like problem was solved before.\nError:\n{}\nThe fix that passed the tests:\n{}", self.sliced_error.trim(), self.final_diff.trim())
    }
}

fn open(root: &Path) -> Result<Connection, String> {
    let path = root.join(FILE);

    std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|error| error.to_string())?;

    let conn = Connection::open(path).map_err(|error| error.to_string())?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS traces (
            id INTEGER PRIMARY KEY,
            signature TEXT NOT NULL,
            diff_hash TEXT NOT NULL,
            sliced_error TEXT NOT NULL,
            final_diff TEXT NOT NULL,
            failures INTEGER NOT NULL DEFAULT 0,
            hits INTEGER NOT NULL DEFAULT 0,
            UNIQUE(signature, diff_hash)
        );
        CREATE TABLE IF NOT EXISTS witnesses (trace_id INTEGER NOT NULL, source TEXT NOT NULL, UNIQUE(trace_id, source));",
    )
    .map_err(|error| error.to_string())?;

    Ok(conn)
}

/// The error class of a failure: the compiler's code (`E0308`, `TS2322`), the exception's name (`AssertionError`), else the first words.
pub fn error_class(found: &Found) -> String {
    let message = &found.message;

    if let Some(code) = message.split(|c: char| !c.is_ascii_alphanumeric()).find(|word| (word.starts_with('E') && word.len() == 5 && word[1..].chars().all(|c| c.is_ascii_digit())) || (word.starts_with("TS") && word.len() == 6 && word[2..].chars().all(|c| c.is_ascii_digit()))) {
        return code.to_string();
    }

    if let Some(name) = message.split(|c: char| !c.is_ascii_alphanumeric()).find(|word| word.ends_with("Error") || word.ends_with("Exception")) {
        return name.to_string();
    }

    message.split_whitespace().take(3).collect::<Vec<_>>().join(" ").to_lowercase().chars().filter(|c| !c.is_ascii_digit()).collect()
}

/// `error class | symbol kind | language`.
pub fn signature(found: &Found, symbol_kind: &str, path: &str) -> String {
    let language = crate::graph::parse::Lang::of(path).map_or("other", |lang| lang.name());

    format!("{}|{}|{}", error_class(found), symbol_kind, language)
}

fn diff_hash(diff: &str) -> String {
    crate::graph::parse::sha(&crate::graph::parse::normalised(diff))
}

/// May this text be kept? Not when it holds a secret or a redacted placeholder.
fn clean(text: &str) -> bool {
    !text.contains("‹secret:") && crate::trust::secretguard::scan("", text).is_empty()
}

/// Stores a fix that passed the tests. `secret_path` says whether a path is one the project keeps off the cloud; `local_only` is the project's
/// privacy. A second witness of the same fix (another turn) adds to the first row.
pub fn record(root: &Path, fix: &Fix, secret_path: &dyn Fn(&str) -> bool, local_only: bool) -> Recorded {
    if !crate::v3::on("nr.golden_trace") {
        return Recorded::Refused("nr.golden_trace is off".into());
    }

    if !fix.tests_passed {
        return Recorded::Refused("only a fix that passed every test is kept".into());
    }

    if !clean(&fix.sliced_error) || !clean(&fix.final_diff) || !clean(&fix.signature) {
        return Recorded::Refused("it holds a secret or a redacted placeholder".into());
    }

    if local_only && fix.touched.iter().any(|path| secret_path(path)) {
        return Recorded::Refused("it touches a secret path of a local-only project".into());
    }

    if fix.final_diff.trim().is_empty() || fix.signature.trim().is_empty() {
        return Recorded::Refused("an empty fix is not an example".into());
    }

    let Ok(conn) = open(root) else { return Recorded::Refused("the Golden Trace store could not be opened".into()) };
    let hash = diff_hash(&fix.final_diff);
    let inserted = conn.execute("INSERT OR IGNORE INTO traces (signature, diff_hash, sliced_error, final_diff) VALUES (?1, ?2, ?3, ?4)", params![fix.signature, hash, fix.sliced_error, fix.final_diff]);

    if inserted.is_err() {
        return Recorded::Refused("the Golden Trace store could not be written".into());
    }

    let Ok(id) = conn.query_row("SELECT id FROM traces WHERE signature = ?1 AND diff_hash = ?2", params![fix.signature, hash], |row| row.get::<_, i64>(0)) else {
        return Recorded::Refused("the Golden Trace store could not be read".into());
    };

    let _ = conn.execute("INSERT OR IGNORE INTO witnesses (trace_id, source) VALUES (?1, ?2)", params![id, fix.turn]);

    let witnesses = conn.query_row("SELECT COUNT(*) FROM witnesses WHERE trace_id = ?1", params![id], |row| row.get::<_, i64>(0)).unwrap_or(0) as usize;

    Recorded::Stored { witnesses }
}

fn sources(conn: &Connection, id: i64) -> Vec<String> {
    let Ok(mut statement) = conn.prepare("SELECT source FROM witnesses WHERE trace_id = ?1") else { return Vec::new() };

    statement.query_map(params![id], |row| row.get::<_, String>(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

/// The signature of the problem a relayed step had: the tool's error class, the kind of symbol the error line is in, the file's language.
/// `None` when the log is not one this reads.
pub fn signature_for(root: &Path, error_log: &str, edited_file: &str) -> Option<(String, Found)> {
    let tool = super::errslice::detect(error_log)?;
    let found = super::errslice::find(tool, error_log)?;
    let kind = found
        .file
        .as_deref()
        .zip(found.line)
        .and_then(|(file, line)| {
            let text = std::fs::read_to_string(root.join(file)).ok()?;
            let all = super::symbols::index_text(file, &text)?;

            all.iter().filter(|symbol| symbol.start_line <= line && line <= symbol.end_line).max_by_key(|symbol| symbol.depth).map(|symbol| symbol.kind.clone())
        })
        .unwrap_or_else(|| "fn".to_string());

    Some((signature(&found, &kind, found.file.as_deref().unwrap_or(edited_file)), found))
}

/// The one example for a signature, if there is one that meets the two-witness rule and has not failed twice. The use is counted.
pub fn lookup(root: &Path, signature: &str) -> Option<Example> {
    if !crate::v3::on("nr.golden_trace") {
        return None;
    }

    let conn = open(root).ok()?;
    let mut statement = conn.prepare("SELECT id, sliced_error, final_diff FROM traces WHERE signature = ?1 AND failures < ?2 ORDER BY hits DESC, id ASC").ok()?;
    let rows: Vec<(i64, String, String)> = statement.query_map(params![signature, MAX_FAILURES], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).ok()?.flatten().collect();

    for (id, sliced_error, final_diff) in rows {
        /* the knowledge core's door: two separate turns must have seen it */
        let candidate = Candidate { kind: "lesson".into(), text: format!("For `{signature}` the fix was: {}", final_diff.trim()), observations: sources(&conn, id) };

        if candidate.observations.len() >= 2 && two_witnesses(&candidate) {
            let _ = conn.execute("UPDATE traces SET hits = hits + 1 WHERE id = ?1", params![id]);

            return Some(Example { id, sliced_error, final_diff });
        }
    }

    None
}

/// The knowledge core's two-witness rule for a lesson, without its other checks (a diff is not a sentence about the project).
fn two_witnesses(candidate: &Candidate) -> bool {
    let distinct: std::collections::HashSet<&String> = candidate.observations.iter().collect();

    distinct.len() >= 2 && candidate.kind == "lesson" && knowledge_core::KINDS.contains(&candidate.kind.as_str())
}

/// The example was used and the step still failed: one strike; at two the example is dropped.
pub fn followed_by_failure(root: &Path, id: i64) -> bool {
    let Ok(conn) = open(root) else { return false };
    let _ = conn.execute("UPDATE traces SET failures = failures + 1 WHERE id = ?1", params![id]);

    conn.query_row("SELECT failures FROM traces WHERE id = ?1", params![id], |row| row.get::<_, i64>(0)).is_ok_and(|failures| failures >= MAX_FAILURES)
}

/// How many times examples were handed out - for the receipt.
pub fn hit_count(root: &Path) -> i64 {
    open(root).ok().and_then(|conn| conn.query_row("SELECT COALESCE(SUM(hits), 0) FROM traces", [], |row| row.get(0)).ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests;
