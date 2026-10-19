//! The working-memory table's access (P1-7): one more `impl Store` block over the same connection, beside `store::trust`.

use anyhow::Result;
use rusqlite::params;

use super::sqlite::Store;

/// One remembered line, as the table holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub seq: i64,
    pub kind: String,
    pub text: String,
    /// The file or tool the note came from (empty for what the person said).
    pub source: String,
    /// The quote from `source` the note rests on (empty when the note is not about a file's text).
    pub quote: String,
    pub pinned: bool,
}

/// How many of each kind are kept. A note past the cap replaces the oldest unpinned one of its kind.
pub fn cap_of(kind: &str) -> usize {
    match kind {
        "fact" => 1,
        "decision" => 5,
        "failure" => 8,
        _ => 5,
    }
}

/// The kinds a note may be.
pub fn known_kind(kind: &str) -> bool {
    matches!(kind, "fact" | "decision" | "failure" | "open")
}

impl Store {
    /// Adds a note to a task's memory and keeps the kind's cap (the oldest unpinned note of that kind goes). Answers the note's `seq`.
    pub fn add_note(&self, task: &str, kind: &str, text: &str, source: &str, quote: &str, pinned: bool) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();
        let next: i64 = connection.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM working_memory WHERE task = ?1", params![task], |row| row.get(0))?;

        connection.execute(
            "INSERT INTO working_memory (task, seq, kind, text, source, quote, pinned, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![task, next, kind, text.trim(), source, quote, i64::from(pinned), now],
        )?;

        /* the cap counts the pinned notes too: a pinned fact is the fact, so a second unpinned one is what goes */
        let pinned_count: i64 = connection.query_row("SELECT COUNT(*) FROM working_memory WHERE task = ?1 AND kind = ?2 AND pinned = 1", params![task, kind], |row| row.get(0))?;
        let keep_unpinned = (cap_of(kind) as i64 - pinned_count).max(0);

        connection.execute(
            "DELETE FROM working_memory WHERE task = ?1 AND kind = ?2 AND pinned = 0 AND seq NOT IN (
                 SELECT seq FROM working_memory WHERE task = ?1 AND kind = ?2 AND pinned = 0 ORDER BY seq DESC LIMIT ?3)",
            params![task, kind, keep_unpinned],
        )?;

        Ok(next)
    }

    /// A task's notes, oldest first.
    pub fn notes(&self, task: &str) -> Result<Vec<Note>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT seq, kind, text, source, quote, pinned FROM working_memory WHERE task = ?1 ORDER BY seq")?;
        let rows = statement.query_map(params![task], |row| {
            Ok(Note { seq: row.get(0)?, kind: row.get(1)?, text: row.get(2)?, source: row.get(3)?, quote: row.get(4)?, pinned: row.get::<_, i64>(5)? != 0 })
        })?;
        let mut notes = Vec::new();

        for row in rows {
            notes.push(row?);
        }

        Ok(notes)
    }

    /// Pins or unpins a note (the app's memory panel). A pinned note is never replaced by a newer one of its kind.
    pub fn set_note_pinned(&self, task: &str, seq: i64, pinned: bool) -> Result<()> {
        self.connection.lock().unwrap().execute("UPDATE working_memory SET pinned = ?3 WHERE task = ?1 AND seq = ?2", params![task, seq, i64::from(pinned)])?;

        Ok(())
    }

    /// Drops a note (the person correcting what SDC remembers). `false` when there was no such note.
    pub fn drop_note(&self, task: &str, seq: i64) -> Result<bool> {
        Ok(self.connection.lock().unwrap().execute("DELETE FROM working_memory WHERE task = ?1 AND seq = ?2", params![task, seq])? > 0)
    }

    /// Forgets a task's memory, and answers how many notes went.
    pub fn clear_notes(&self, task: &str) -> Result<usize> {
        Ok(self.connection.lock().unwrap().execute("DELETE FROM working_memory WHERE task = ?1", params![task])?)
    }
}
