//! The SQLite schema and the daemon's typed access to it (master spec section 6).
//!
//! One `Store` per daemon, one connection, `Mutex`-guarded. SQLite in WAL mode is more than enough
//! for a single-user host daemon, and a connection pool would only add a way for two writers to
//! disagree.
//!
//! The schema is created on first open and migrated additively: `migrations` records which steps
//! have run, so a version is never applied twice and an older database keeps working (spec section
//! 5.7, "versioned, additive changes").
//!
//! Passwords and API keys are **not** in here. `providers` stores what a provider *is* (kind, status,
//! label); the secret lives in the keychain (src/auth/keychain.rs) and only its reference is stored.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::Value;

use crate::sdcp::events::StoredEvent;

/// The migration steps, in order. Append only: a new step is a new entry, never an edit.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001-initial",
        r#"
CREATE TABLE IF NOT EXISTS hosts (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, target TEXT,
  status TEXT NOT NULL, platform TEXT, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY, host_id TEXT NOT NULL REFERENCES hosts(id), root TEXT NOT NULL,
  name TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY, host_id TEXT NOT NULL REFERENCES hosts(id), project_id TEXT REFERENCES projects(id),
  title TEXT NOT NULL, prompt TEXT NOT NULL DEFAULT '', state TEXT NOT NULL DEFAULT 'idle',
  turn_count INTEGER NOT NULL DEFAULT 0, unread INTEGER NOT NULL DEFAULT 0, attention TEXT,
  updated_at TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS turns (
  id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), ordinal INTEGER NOT NULL,
  engine TEXT NOT NULL, model TEXT NOT NULL, tier TEXT NOT NULL, prompt TEXT NOT NULL,
  answer TEXT NOT NULL DEFAULT '', summary TEXT NOT NULL DEFAULT '', state TEXT NOT NULL DEFAULT 'running',
  started_at TEXT NOT NULL, finished_at TEXT);
CREATE TABLE IF NOT EXISTS checkpoints (
  id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), turn INTEGER NOT NULL,
  title TEXT NOT NULL DEFAULT '', thumbnail TEXT, files_hash TEXT NOT NULL, rewind_ref INTEGER,
  created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS rewind_stack (
  id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id),
  checkpoint_id TEXT NOT NULL, turn INTEGER NOT NULL, pushed_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS permissions (
  id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), turn_id TEXT,
  action TEXT NOT NULL, target TEXT NOT NULL, risk TEXT NOT NULL, decision TEXT, scope TEXT,
  requested_at TEXT NOT NULL, resolved_at TEXT);
CREATE TABLE IF NOT EXISTS providers (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT '', account TEXT, logo TEXT, url TEXT, protocol TEXT,
  key_ref TEXT, updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS models (
  id TEXT PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id), tier TEXT NOT NULL,
  ctx INTEGER NOT NULL, cost TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, size TEXT);
CREATE TABLE IF NOT EXISTS events (
  seq INTEGER PRIMARY KEY, ts TEXT NOT NULL, session_id TEXT, turn_id TEXT, type TEXT NOT NULL,
  payload TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS events_session ON events(session_id, seq);
CREATE TABLE IF NOT EXISTS migrations (id TEXT PRIMARY KEY, applied_at TEXT NOT NULL);
"#,
    ),
    (
        "0002-model-cache",
        r#"
ALTER TABLE models ADD COLUMN source TEXT NOT NULL DEFAULT 'bundled';
ALTER TABLE models ADD COLUMN fetched_at TEXT;
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);
"#,
    ),
];

/// The daemon's database handle.
pub struct Store {
    connection: Mutex<Connection>,
    path: Option<PathBuf>,
}
/// Rows of one shape, collected - every query in this file ends this way.
fn collect_json<T>(rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>) -> Result<Vec<T>> {
    let mut collected = Vec::new();

    for row in rows {
        collected.push(row?);
    }

    Ok(collected)
}

/// One checkpoint row, as every checkpoint query maps it: the seven columns in schema order.
fn row_to_checkpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(serde_json::json!({
        "id": row.get::<_, String>(0)?,
        "sessionId": row.get::<_, String>(1)?,
        "turn": row.get::<_, i64>(2)?,
        "ts": row.get::<_, String>(3)?,
        "title": row.get::<_, String>(4)?,
        "thumbnail": row.get::<_, Option<String>>(5)?,
        "filesHash": row.get::<_, String>(6)?,
    }))
}


impl Store {
    /// Opens (or creates) the database and applies every migration that has not run yet.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(path)?;

        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;

        let store = Self { connection: Mutex::new(connection), path: Some(path.to_path_buf()) };

        store.migrate()?;

        Ok(store)
    }

    /// An in-memory database, for a test that wants the real schema without a file.
    pub fn in_memory() -> Result<Self> {
        let store = Self { connection: Mutex::new(Connection::open_in_memory()?), path: None };

        store.migrate()?;

        Ok(store)
    }

    /// Where the file is, or `:memory:` for a test store. Reported by `host.status`.
    pub fn path(&self) -> String {
        self.path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ":memory:".to_string())
    }

    fn migrate(&self) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS migrations (id TEXT PRIMARY KEY, applied_at TEXT NOT NULL)",
        )?;

        for (id, sql) in MIGRATIONS {
            let already: i64 = connection.query_row(
                "SELECT COUNT(*) FROM migrations WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )?;

            if already > 0 {
                continue;
            }

            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT OR IGNORE INTO migrations (id, applied_at) VALUES (?1, ?2)",
                params![id, chrono::Utc::now().to_rfc3339()],
            )?;
        }

        Ok(())
    }

    /* -----------------------------------------------------------------------------------------
     * The event log (spec section 5.4). Two methods, and both are reads plus one append: the log
     * itself has no update path, so the table has none either.
     * -------------------------------------------------------------------------------------- */

    /// Persists one event. `INSERT OR IGNORE` because `seq` is the primary key: a replay after a
    /// crash must not double-write.
    pub fn store_event(&self, event: &StoredEvent) -> Result<()> {
        let connection = self.connection.lock().unwrap();
        let kind = event.event.get("type").and_then(Value::as_str).unwrap_or("Unknown");

        connection.execute(
            "INSERT OR IGNORE INTO events (seq, ts, session_id, turn_id, type, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.seq,
                event.ts,
                event.session_id,
                event.turn_id,
                kind,
                serde_json::to_string(&event.event)?
            ],
        )?;

        Ok(())
    }

    /// Everything after `since`, oldest first - what `EventLog::hydrate` and `event.list` read.
    pub fn recent_events(&self, since: i64) -> Result<Vec<StoredEvent>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT seq, ts, session_id, turn_id, payload FROM events WHERE seq > ?1 ORDER BY seq ASC",
        )?;

        let rows = statement.query_map(params![since], |row| {
            let payload: String = row.get(4)?;

            Ok(StoredEvent {
                seq: row.get(0)?,
                ts: row.get(1)?,
                session_id: row.get(2)?,
                turn_id: row.get(3)?,
                event: serde_json::from_str(&payload).unwrap_or(Value::Null),
            })
        })?;

        let mut events = Vec::new();

        for row in rows {
            events.push(row?);
        }

        Ok(events)
    }

    /// Makes sure a session row exists, without disturbing one that does. A turn may arrive for a
    /// session the daemon has not seen - the app's seed creates its sessions in the *UI*, and the
    /// first thing a user does with a seeded chat is type into it.
    pub fn ensure_session(&self, session_id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT OR IGNORE INTO hosts (id, name, kind, status, created_at)
             VALUES ('local', 'Local', 'local', 'connected', ?1)",
            params![now],
        )?;
        connection.execute(
            "INSERT OR IGNORE INTO sessions (id, host_id, title, prompt, state, turn_count, unread, updated_at, created_at)
             VALUES (?1, 'local', 'New chat', '', 'idle', 0, 0, ?2, ?2)",
            params![session_id, now],
        )?;

        Ok(())
    }

    /// A session's checkpoints, newest first - the Time Machine tab's list.
    pub fn checkpoints(&self, session_id: &str) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, session_id, turn, created_at, title, thumbnail, files_hash FROM checkpoints
             WHERE session_id = ?1 ORDER BY turn DESC",
        )?;
        let rows = statement.query_map(params![session_id], row_to_checkpoint)?;

        collect_json(rows)
    }

    /// One checkpoint by id, which `checkpoint.restore` needs: the caller has the id, not the turn.
    pub fn checkpoint(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, session_id, turn, created_at, title, thumbnail, files_hash FROM checkpoints
             WHERE id = ?1",
        )?;
        let mut rows = statement.query_map(params![id], row_to_checkpoint)?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// The checkpoints a rewind to `turn` would step over, oldest first.
    pub fn checkpoints_after(&self, session_id: &str, turn: i64) -> Result<Vec<Value>> {
        let mut dropped: Vec<Value> = self
            .checkpoints(session_id)?
            .into_iter()
            .filter(|row| row["turn"].as_i64().unwrap_or(0) > turn)
            .collect();

        dropped.reverse();

        Ok(dropped)
    }

    /// Pushes the dropped checkpoints onto the rewind stack and takes them out of the visible list -
    /// which is what makes the Time Machine tab show the state you rewound *to*.
    pub fn push_rewind_stack(&self, session_id: &str, dropped: &[Value]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        for row in dropped {
            let id = row["id"].as_str().unwrap_or_default();

            connection.execute(
                "INSERT INTO rewind_stack (session_id, checkpoint_id, turn, pushed_at) VALUES (?1, ?2, ?3, ?4)",
                params![session_id, id, row["turn"].as_i64().unwrap_or(0), now],
            )?;
            connection.execute("DELETE FROM checkpoints WHERE id = ?1", params![id])?;
        }

        Ok(())
    }

    /// Pops the newest frame, moving it back into the visible list. `redo`'s whole job.
    pub fn pop_rewind_stack(&self, session_id: &str) -> Result<Option<Value>> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT rowid, checkpoint_id, turn FROM rewind_stack WHERE session_id = ?1 ORDER BY turn ASC LIMIT 1",
        )?;
        let mut rows = statement.query_map(params![session_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
        })?;

        let Some(frame) = rows.next() else {
            return Ok(None);
        };
        let (row_id, checkpoint_id, turn) = frame?;

        drop(rows);
        drop(statement);

        connection.execute("DELETE FROM rewind_stack WHERE rowid = ?1", params![row_id])?;
        connection.execute(
            "INSERT OR REPLACE INTO checkpoints (id, session_id, turn, title, thumbnail, files_hash, created_at)
             VALUES (?1, ?2, ?3, 'restored', NULL, 'restored', ?4)",
            params![checkpoint_id, session_id, turn, now],
        )?;

        Ok(Some(serde_json::json!({ "id": checkpoint_id, "turn": turn })))
    }

    /// How many frames are stacked - the "N undoable" the Time Machine tab can show.
    pub fn rewind_depth(&self, session_id: &str) -> Result<i64> {
        let connection = self.connection.lock().unwrap();

        Ok(connection.query_row(
            "SELECT COUNT(*) FROM rewind_stack WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?)
    }

    /// How many events are persisted; the doctor's disk row reports it.
    pub fn event_count(&self) -> Result<i64> {
        let connection = self.connection.lock().unwrap();

        Ok(connection.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?)
    }

    /* -----------------------------------------------------------------------------------------
     * The model cache and the settings - what "the model list is always up to date" needs
     * (spec section 9.10). The cache keeps the last *live* answer so a failed refresh does not
     * throw it away, and the settings table holds a UI preference (the selected model), which is
     * not an event on purpose (spec section 3.3).
     * -------------------------------------------------------------------------------------- */

    /// Replaces a provider's cached model rows with what its endpoint just said.
    pub fn replace_models(&self, provider_id: &str, rows: &[Value], fetched_at: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        /* The provider row is what the foreign key points at, and a provider that has never been saved
           still has a catalogue row - `models.list` runs before any provider is connected. */
        connection.execute(
            "INSERT OR IGNORE INTO providers (id, name, kind, status, detail, updated_at)
             VALUES (?1, ?1, 'api-key', 'available', '', ?2)",
            params![provider_id, fetched_at],
        )?;
        connection.execute("DELETE FROM models WHERE provider_id = ?1", params![provider_id])?;

        for row in rows {
            let id = row["id"].as_str().unwrap_or_default();

            if id.is_empty() {
                continue;
            }

            connection.execute(
                "INSERT OR REPLACE INTO models (id, provider_id, tier, ctx, cost, enabled, size, source, fetched_at)
                 VALUES (?1, ?2, 'balanced', 0, '', 1, NULL, 'live', ?3)",
                params![format!("{provider_id}/{id}"), provider_id, fetched_at],
            )?;
        }

        Ok(())
    }

    /// The cached rows of a provider, in the shape `models.list` merges: `{ id, providerId, fetchedAt }`.
    pub fn cached_models(&self, provider_id: &str) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, provider_id, fetched_at FROM models WHERE provider_id = ?1 ORDER BY id ASC",
        )?;
        let rows = statement.query_map(params![provider_id], |row| {
            let stored = row.get::<_, String>(0)?;
            /* Stored keys are `provider/id` so the primary key stays unique across providers; the id the
               caller asked about is the part after the first slash. */
            let id = stored.split_once('/').map(|(_, id)| id.to_string()).unwrap_or(stored);

            Ok(serde_json::json!({
                "id": id,
                "providerId": row.get::<_, String>(1)?,
                "fetchedAt": row.get::<_, Option<String>>(2)?,
            }))
        })?;

        collect_json(rows)
    }

    /// A setting, or `None` when it has never been set.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = statement.query_map(params![key], |row| row.get::<_, String>(0))?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Writes a setting, replacing any previous value.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
            params![key, value, now],
        )?;

        Ok(())
    }


    /* -----------------------------------------------------------------------------------------
     * Hosts, sessions and turns. Reads return `serde_json::Value` rather than a struct per table:
     * every one of them is about to be serialised into an event or a method result, so a typed
     * mirror would be a third copy of the schema (spec section 5.10, "schema-first").
     * -------------------------------------------------------------------------------------- */

    pub fn upsert_host(&self, id: &str, name: &str, kind: &str, target: Option<&str>, status: &str, platform: Option<&str>) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO hosts (id, name, kind, target, status, platform, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET name = ?2, kind = ?3, target = ?4, status = ?5, platform = ?6",
            params![id, name, kind, target, status, platform, now],
        )?;

        Ok(())
    }

    pub fn hosts(&self) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT id, name, kind, status, platform FROM hosts ORDER BY created_at ASC")?;

        let rows = statement.query_map([], |row| {
            Ok(serde_json::json!({
                "hostId": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "hostType": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "platform": row.get::<_, Option<String>>(4)?,
            }))
        })?;

        let mut hosts = Vec::new();

        for row in rows {
            hosts.push(row?);
        }

        Ok(hosts)
    }

    pub fn insert_session(&self, id: &str, host_id: &str, title: &str, prompt: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT OR REPLACE INTO sessions (id, host_id, title, prompt, state, turn_count, unread, updated_at, created_at)
             VALUES (?1, ?2, ?3, ?4, 'idle', 0, 0, ?5, ?5)",
            params![id, host_id, title, prompt, now],
        )?;

        Ok(())
    }

    /// Renames or re-states a session. `None` leaves a column alone, which is what makes one method
    /// serve both `session.update` and the state changes a turn causes.
    pub fn update_session(&self, id: &str, title: Option<&str>, state: Option<&str>, attention: Option<&str>, unread: Option<i64>) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "UPDATE sessions SET
                title = COALESCE(?2, title),
                state = COALESCE(?3, state),
                attention = COALESCE(?4, attention),
                unread = COALESCE(?5, unread),
                updated_at = ?6
             WHERE id = ?1",
            params![id, title, state, attention, unread, now],
        )?;

        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;

        Ok(())
    }

    pub fn session_count(&self) -> Result<i64> {
        let connection = self.connection.lock().unwrap();

        Ok(connection.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?)
    }

    /// Records a turn's start; `finish_turn` closes it.
    ///
    /// Seven columns is what a turn *is*, so the argument list is the row - an `InsertTurn` struct
    /// would move the same seven fields one line down and add a type nobody else constructs.
    #[allow(clippy::too_many_arguments)]
    pub fn start_turn(&self, id: &str, session_id: &str, ordinal: i64, engine: &str, model: &str, tier: &str, prompt: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT OR REPLACE INTO turns (id, session_id, ordinal, engine, model, tier, prompt, state, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', ?8)",
            params![id, session_id, ordinal, engine, model, tier, prompt, now],
        )?;

        connection.execute(
            "UPDATE sessions SET turn_count = ?2, state = 'running', updated_at = ?3 WHERE id = ?1",
            params![session_id, ordinal, now],
        )?;

        Ok(())
    }

    pub fn finish_turn(&self, id: &str, answer: &str, summary: &str, state: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "UPDATE turns SET answer = ?2, summary = ?3, state = ?4, finished_at = ?5 WHERE id = ?1",
            params![id, answer, summary, state, now],
        )?;

        Ok(())
    }

    pub fn turns(&self, session_id: &str) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, ordinal, engine, model, tier, prompt, answer, summary, state FROM turns WHERE session_id = ?1 ORDER BY ordinal ASC",
        )?;

        let rows = statement.query_map(params![session_id], |row| {
            Ok(serde_json::json!({
                "turnId": row.get::<_, String>(0)?,
                "ordinal": row.get::<_, i64>(1)?,
                "engine": row.get::<_, String>(2)?,
                "model": row.get::<_, String>(3)?,
                "tier": row.get::<_, String>(4)?,
                "prompt": row.get::<_, String>(5)?,
                "answer": row.get::<_, String>(6)?,
                "summary": row.get::<_, String>(7)?,
                "state": row.get::<_, String>(8)?,
            }))
        })?;

        let mut turns = Vec::new();

        for row in rows {
            turns.push(row?);
        }

        Ok(turns)
    }

    /* -----------------------------------------------------------------------------------------
     * Providers, checkpoints and the rewind stack: the tables the provider backend, the checkpoint
     * writer and the rewind of spec section 14 read and write.
     * -------------------------------------------------------------------------------------- */

    /// One provider row, or `None` when the daemon has never seen it.
    pub fn provider(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, name, kind, status, detail, account, logo, url, protocol FROM providers WHERE id = ?1",
        )?;
        let mut rows = statement.query_map(params![id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "kind": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "detail": row.get::<_, String>(4)?,
                "account": row.get::<_, Option<String>>(5)?,
                "logo": row.get::<_, Option<String>>(6)?,
                "url": row.get::<_, Option<String>>(7)?,
                "protocol": row.get::<_, Option<String>>(8)?,
            }))
        })?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Writes what a provider *is*. The secret is not a parameter on purpose: it belongs to the
    /// keychain, and this function should not be able to receive it.
    ///
    /// The eight arguments are the row's eight columns; a struct would only rename them.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_provider(
        &self,
        id: &str,
        name: &str,
        kind: &str,
        status: &str,
        account: Option<&str>,
        url: Option<&str>,
        protocol: Option<&str>,
        key_ref: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO providers (id, name, kind, status, detail, account, url, protocol, key_ref, updated_at)
             VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                name = ?2, kind = ?3, status = ?4, account = ?5, url = ?6, protocol = ?7,
                key_ref = COALESCE(?8, key_ref), updated_at = ?9",
            params![id, name, kind, status, account, url, protocol, key_ref, now],
        )?;

        Ok(())
    }

    /// Records a checkpoint. A re-run of the same turn replaces its row rather than stacking a
    /// duplicate: a turn has one "before".
    pub fn insert_checkpoint(
        &self,
        id: &str,
        session_id: &str,
        turn: i64,
        title: &str,
        thumbnail: Option<&str>,
        files_hash: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "DELETE FROM checkpoints WHERE session_id = ?1 AND turn = ?2",
            params![session_id, turn],
        )?;
        connection.execute(
            "INSERT INTO checkpoints (id, session_id, turn, title, thumbnail, files_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, session_id, turn, title, thumbnail, files_hash, now],
        )?;

        Ok(())
    }

}
