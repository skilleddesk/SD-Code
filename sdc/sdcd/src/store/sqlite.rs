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
use rusqlite::{params, Connection, OptionalExtension};
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
    (
        /* 0.7.13 - the SSH columns a remote host needs, and the bug the first one fixes.
         *
         * `host.add` parsed `ssh -p 8443 user@host` correctly and then wrote only `user@host` into
         * `target`: the port was dropped, so a VPS added on 8443 connected once (the probe used the
         * parsed target) and every later connection for that host would have used 22. That is half of
         * the "vps connect korai jasse nah" report, and it is exactly the kind of loss a column fixes
         * rather than a comment.
         *
         * `host_key` records the fingerprint the person pinned, so a host's decision is readable from
         * the host's own row (`known_hosts` holds the pin itself; this is what a card can show). */
        "0003-host-ssh",
        r#"
ALTER TABLE hosts ADD COLUMN port INTEGER;
ALTER TABLE hosts ADD COLUMN host_key TEXT;
"#,
    ),
    (
        /* v4: one rewind is one frame - the checkpoint rows it hid (whole, so a redo puts them back as they
         * were rather than as `'restored'` rows with no hash) and the shadow commit of the folder just
         * before the rewind, which is what a redo restores. */
        "0004-rewind-frames",
        r#"
ALTER TABLE rewind_stack ADD COLUMN frame TEXT;
"#,
    ),
    (
        /* 0.12 - the Trust Kernel (docs/MASTER-PLAN-v3-TRUST-KERNEL.md).
         *
         * `audit` is the hash-chained ledger: every row carries the hash of the row before it, so a row
         * edited after the fact breaks the chain at that row and `audit.verify` names it. `usage` is the
         * cost governor's record, one row per turn, `cost_source` saying whether the number came from the
         * provider or from the price table. `trust_scores` is each turn's score with its reasons.
         * `restore_journal` makes a restore atomic across a crash: the row is written before the folder
         * moves and closed after, so a daemon that starts with an open row knows a restore was cut off.
         * `glossary` and `intents` are the Intent Engine's memory. A checkpoint gains a label a person
         * gave it and the reason it cannot fully undo what came after it. */
        "0005-trust-kernel",
        r#"
CREATE TABLE IF NOT EXISTS audit (
  seq INTEGER PRIMARY KEY, ts TEXT NOT NULL, session_id TEXT, turn_id TEXT, actor TEXT NOT NULL,
  kind TEXT NOT NULL, summary TEXT NOT NULL, detail TEXT NOT NULL, prev_hash TEXT NOT NULL, hash TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS audit_session ON audit(session_id, seq);
CREATE TABLE IF NOT EXISTS usage (
  turn_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, project_root TEXT, site_id TEXT,
  engine TEXT NOT NULL, model TEXT NOT NULL, provider TEXT,
  input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL NOT NULL DEFAULT 0, cost_source TEXT NOT NULL DEFAULT 'none',
  estimate_usd REAL, baseline_usd REAL, day TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS usage_day ON usage(day);
CREATE TABLE IF NOT EXISTS trust_scores (
  turn_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, score INTEGER NOT NULL, level TEXT NOT NULL,
  reasons TEXT NOT NULL, created_at TEXT NOT NULL);
ALTER TABLE checkpoints ADD COLUMN label TEXT;
ALTER TABLE checkpoints ADD COLUMN irreversible TEXT;
ALTER TABLE checkpoints ADD COLUMN turn_id TEXT;
CREATE TABLE IF NOT EXISTS restore_journal (
  id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, root TEXT NOT NULL, host_id TEXT,
  target_sha TEXT NOT NULL, before_sha TEXT, scope TEXT NOT NULL DEFAULT 'all', state TEXT NOT NULL,
  started_at TEXT NOT NULL, finished_at TEXT);
CREATE TABLE IF NOT EXISTS glossary (
  id INTEGER PRIMARY KEY AUTOINCREMENT, scope TEXT NOT NULL, term TEXT NOT NULL, meaning TEXT NOT NULL,
  created_at TEXT NOT NULL, UNIQUE(scope, term));
CREATE TABLE IF NOT EXISTS intents (
  id TEXT PRIMARY KEY, session_id TEXT, text TEXT NOT NULL, spec TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'parsed', corrections INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
"#,
    ),
    (
        /* 0.12 - the agency half: sites, their deploys and their health. A site is a folder on a host that
         * serves something at a URL; its `config` is the deploy recipe and the health checks as JSON. A
         * deploy row keeps every step's log and the backup it made, which is what a one-click rollback
         * restores. `approvals` is who said yes to what, for a production deploy or a client sign-off. */
        "0006-sites",
        r#"
CREATE TABLE IF NOT EXISTS sites (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, host_id TEXT NOT NULL, root TEXT NOT NULL,
  url TEXT NOT NULL DEFAULT '', config TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS deploys (
  id TEXT PRIMARY KEY, site_id TEXT NOT NULL, kind TEXT NOT NULL, state TEXT NOT NULL,
  steps TEXT NOT NULL, backup TEXT, note TEXT NOT NULL DEFAULT '', started_at TEXT NOT NULL, finished_at TEXT);
CREATE INDEX IF NOT EXISTS deploys_site ON deploys(site_id, started_at);
CREATE TABLE IF NOT EXISTS health (
  id INTEGER PRIMARY KEY AUTOINCREMENT, site_id TEXT NOT NULL, ts TEXT NOT NULL, report TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS health_site ON health(site_id, id);
CREATE TABLE IF NOT EXISTS approvals (
  id TEXT PRIMARY KEY, subject TEXT NOT NULL, kind TEXT NOT NULL, state TEXT NOT NULL,
  requested_by TEXT NOT NULL, decided_by TEXT, note TEXT NOT NULL DEFAULT '', token TEXT,
  created_at TEXT NOT NULL, decided_at TEXT);
"#,
    ),
    /* SDC Anywhere (0.17): paired browser devices, the hello nonces already seen, and the passkey counters.
       Public keys only - the daemon's own private keys live in the keychain (anywhere::identity). */
    (
        "0100-anywhere",
        r#"
CREATE TABLE IF NOT EXISTS anywhere_devices (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, user_agent TEXT NOT NULL DEFAULT '',
  sign_pub TEXT NOT NULL, passkey_id TEXT NOT NULL DEFAULT '', passkey_pub TEXT NOT NULL DEFAULT '',
  passkey_counter INTEGER NOT NULL DEFAULT 0, guest INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, last_seen INTEGER, revoked_at INTEGER, expires_at INTEGER);
CREATE TABLE IF NOT EXISTS anywhere_nonces (
  nonce TEXT PRIMARY KEY, device_id TEXT NOT NULL, seen_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS anywhere_nonces_seen ON anywhere_nonces(seen_at);
CREATE TABLE IF NOT EXISTS anywhere_pairings (
  token_hash TEXT PRIMARY KEY, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
  guest INTEGER NOT NULL DEFAULT 0, used_at INTEGER);
"#,
    ),
];

/// The daemon's database handle.
pub struct Store {
    /// Crate-visible so the Trust Kernel's tables (`store::trust`) are one more `impl Store` block
    /// rather than a second connection to the same file.
    pub(crate) connection: Mutex<Connection>,
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

/// The `kind` column, in the protocol's own vocabulary.
///
/// The column stores how this daemon *reaches* the machine (`local`, `ssh`); every event and every
/// `session.list` row says what the machine *is* from the window's point of view (`local`, `vps`).
/// Until 0.7.13 the row read the column straight into `hostType`, so the same host said `ssh` in a list
/// and `vps` in a `HostStatus` - and the sidebar drew the right icon only because anything that is not
/// `local` is a server.
fn host_type(kind: &str) -> &'static str {
    if kind == "local" {
        "local"
    } else {
        "vps"
    }
}

/// One checkpoint row, as every checkpoint query maps it: the columns of `CHECKPOINT_COLUMNS`, in order.
fn row_to_checkpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(serde_json::json!({
        "id": row.get::<_, String>(0)?,
        "sessionId": row.get::<_, String>(1)?,
        "turn": row.get::<_, i64>(2)?,
        "ts": row.get::<_, String>(3)?,
        "title": row.get::<_, String>(4)?,
        "thumbnail": row.get::<_, Option<String>>(5)?,
        "filesHash": row.get::<_, String>(6)?,
        "label": row.get::<_, Option<String>>(7)?,
        "irreversible": row.get::<_, Option<String>>(8)?,
        "turnId": row.get::<_, Option<String>>(9)?,
    }))
}

/// The columns `row_to_checkpoint` reads.
const CHECKPOINT_COLUMNS: &str = "id, session_id, turn, created_at, title, thumbnail, files_hash, label, irreversible, turn_id";


impl Store {
    /// Opens (or creates) the database and applies every migration that has not run yet.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(path)?;

        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        /* Another process holding the file (a backup, the doctor's own read) is a wait, not a failure:
           without this a locked database failed an event write at once (0.16.1). */
        connection.busy_timeout(std::time::Duration::from_secs(5))?;

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

    /// A whole, consistent copy of the database at `target` (0.15.5), written by SQLite itself
    /// (`VACUUM INTO`), so a copy taken while the daemon writes is still a database that opens.
    pub fn backup_to(&self, target: &std::path::Path) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        let _ = std::fs::remove_file(target);
        connection.execute("VACUUM INTO ?1", [target.to_string_lossy().to_string()])?;

        Ok(())
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

    /// The files a session's turns changed, as `(turn id, path)`, oldest first and each path once
    /// (0.12.5, `continuity`): the `edit` tool calls in the session's log.
    pub fn edited_files(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT turn_id, payload FROM events WHERE session_id = ?1 AND type = 'ToolCallStarted' ORDER BY seq ASC",
        )?;
        let rows = statement.query_map(params![session_id], |row| {
            Ok((row.get::<_, Option<String>>(0)?.unwrap_or_default(), row.get::<_, String>(1)?))
        })?;
        let mut files: Vec<(String, String)> = Vec::new();

        for row in rows {
            let (turn, payload) = row?;
            let event: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            let target = event["target"].as_str().unwrap_or_default().trim().to_string();

            if event["tool"] == "edit" && !target.is_empty() && !files.iter().any(|(_, path)| *path == target) {
                files.push((turn, target));
            }
        }

        Ok(files)
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
            &format!("SELECT {CHECKPOINT_COLUMNS} FROM checkpoints WHERE session_id = ?1 ORDER BY turn DESC"),
        )?;
        let rows = statement.query_map(params![session_id], row_to_checkpoint)?;

        collect_json(rows)
    }

    /// One checkpoint by id, which `checkpoint.restore` needs: the caller has the id, not the turn.
    pub fn checkpoint(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            &format!("SELECT {CHECKPOINT_COLUMNS} FROM checkpoints WHERE id = ?1"),
        )?;
        let mut rows = statement.query_map(params![id], row_to_checkpoint)?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// A checkpoint and every one after it, oldest first - what a rewind **to** that checkpoint hides.
    pub fn checkpoints_from(&self, session_id: &str, turn: i64) -> Result<Vec<Value>> {
        let mut rows: Vec<Value> = self
            .checkpoints(session_id)?
            .into_iter()
            .filter(|row| row["turn"].as_i64().unwrap_or(0) >= turn)
            .collect();

        rows.reverse();

        Ok(rows)
    }

    /// Records one rewind: the frame (what a redo needs), and the hidden checkpoints taken out of the list.
    pub fn push_rewind_frame(&self, session_id: &str, turn: i64, target_id: &str, frame: &Value, hidden: &[String]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO rewind_stack (session_id, checkpoint_id, turn, pushed_at, frame) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, target_id, turn, now, frame.to_string()],
        )?;

        for id in hidden {
            connection.execute("DELETE FROM checkpoints WHERE id = ?1", params![id])?;
        }

        Ok(())
    }

    /// Takes the newest rewind off the stack and puts its checkpoints back exactly as they were. Returns the
    /// frame, or `None` when there is nothing to redo. A frame written before v4 has no `frame` and comes
    /// back as `{}` - its rows were already lost to the old format.
    pub fn pop_rewind_frame(&self, session_id: &str) -> Result<Option<(i64, Value)>> {
        let connection = self.connection.lock().unwrap();
        let found: Option<(i64, i64, Option<String>)> = connection
            .query_row(
                "SELECT id, turn, frame FROM rewind_stack WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        let Some((row_id, turn, frame)) = found else {
            return Ok(None);
        };

        connection.execute("DELETE FROM rewind_stack WHERE id = ?1", params![row_id])?;

        let frame: Value = frame.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_else(|| serde_json::json!({}));

        for row in frame["rows"].as_array().cloned().unwrap_or_default() {
            connection.execute(
                "INSERT OR REPLACE INTO checkpoints (id, session_id, turn, title, thumbnail, files_hash, created_at, label, irreversible, turn_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    row["id"].as_str().unwrap_or_default(),
                    session_id,
                    row["turn"].as_i64().unwrap_or(0),
                    row["title"].as_str().unwrap_or_default(),
                    row["thumbnail"].as_str(),
                    row["filesHash"].as_str().unwrap_or_default(),
                    row["ts"].as_str().unwrap_or_default(),
                    row["label"].as_str(),
                    row["irreversible"].as_str(),
                    row["turnId"].as_str(),
                ],
            )?;
        }

        Ok(Some((turn, frame)))
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
        let mut connection = self.connection.lock().unwrap();
        /* One transaction, one sync to disk. Row by row, OpenRouter's 458 models were 458 autocommits
           under the store's lock - seconds on a slow disk, during which every other request waited (the
           0.9.0 Windows CI run: three lifecycle tests' 3 s reads timed out behind the startup refresh). */
        let connection = connection.transaction()?;

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

        connection.commit()?;

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
    /// A number no id this daemon handed out has used, and none it hands out later will (0.16.1).
    ///
    /// Ids used to be `events.seq() + 1`, read before anything was appended: two requests in flight
    /// (two connections, or a handler whose first event is written later by a task - `verify.run`, a
    /// pinned `host.add`) both read the same number, and the second `INSERT OR REPLACE` silently replaced
    /// the first row. This counter is read and moved under the connection's lock, so two callers can
    /// never get the same number, and it is stored, so a restart continues past it. `floor` is the
    /// smallest number that is safe given the ids the old scheme made (the event log's next seq and one).
    pub fn next_id(&self, floor: i64) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();
        let stored: Option<String> = connection
            .query_row("SELECT value FROM settings WHERE key = 'ids.next'", [], |row| row.get(0))
            .optional()?;
        let logged: i64 = connection.query_row("SELECT COALESCE(MAX(seq), 0) + 2 FROM events", [], |row| row.get(0))?;
        let next = stored.and_then(|value| value.parse::<i64>().ok()).unwrap_or(0).max(floor).max(logged);

        connection.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('ids.next', ?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?1, updated_at = ?2",
            params![(next + 1).to_string(), now],
        )?;

        Ok(next)
    }

    /// The seq a turn's `TurnStarted` was logged at - the point on the log a rewind to that turn means.
    /// Read rather than parsed from the id, because since 0.16.1 the id's number is not a seq.
    pub fn turn_started_seq(&self, turn_id: &str) -> Result<Option<i64>> {
        let connection = self.connection.lock().unwrap();

        Ok(connection.query_row(
            "SELECT MIN(seq) FROM events WHERE turn_id = ?1 AND type = 'TurnStarted'",
            params![turn_id],
            |row| row.get::<_, Option<i64>>(0),
        )?)
    }

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

    /// Creates the host row only if it is not already there.
    ///
    /// `session.open` uses this instead of `upsert_host`. It used to upsert unconditionally with the
    /// literals `Local` / `local` / `connected`, so opening a chat on a VPS **rewrote that VPS's
    /// row**: its name became `Local`, its kind became `local`, its `target` was erased and its
    /// status was claimed `connected`. That lost the host's identity, and it also broke the "same
    /// server added twice is one host" check in `host.add`, whose key is exactly that `target`.
    pub fn ensure_host(&self, id: &str, name: &str, kind: &str, status: &str) -> Result<()> {
        if self.host(id)?.is_some() {
            return Ok(());
        }

        self.upsert_host(id, name, kind, None, status, None)
    }

    pub fn hosts(&self) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT id, name, kind, status, platform, host_key FROM hosts ORDER BY created_at ASC")?;

        let rows = statement.query_map([], |row| {
            Ok(serde_json::json!({
                "hostId": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "hostType": host_type(&row.get::<_, String>(2)?),
                "status": row.get::<_, String>(3)?,
                "platform": row.get::<_, Option<String>>(4)?,
                /* The fingerprint a person pinned (0.7.13), so a window that never saw the
                   `HostStatus` that asked the question can still say which key this host is. */
                "hostKey": row.get::<_, Option<String>>(5)?,
            }))
        })?;

        let mut hosts = Vec::new();

        for row in rows {
            hosts.push(row?);
        }

        Ok(hosts)
    }
    /// One host row, or `None` when the daemon has never heard of it.
    pub fn host(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT id, name, kind, status, platform, target, port, host_key FROM hosts WHERE id = ?1")?;
        let mut rows = statement.query(params![id])?;

        match rows.next()? {
            Some(row) => Ok(Some(serde_json::json!({
                "hostId": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "hostType": host_type(&row.get::<_, String>(2)?),
                "status": row.get::<_, String>(3)?,
                "platform": row.get::<_, Option<String>>(4)?,
                "target": row.get::<_, Option<String>>(5)?,
                "port": row.get::<_, Option<i64>>(6)?,
                "hostKey": row.get::<_, Option<String>>(7)?,
            }))),
            None => Ok(None),
        }
    }

    /// The address a host was added with: its `user@host` and the port the person uses.
    ///
    /// The port is the half 0.7.0 lost (`MIGRATIONS`, `0003-host-ssh`). `None` for the row means the
    /// daemon has never heard of the host; a `None` *address* means the row is a host it knows
    /// (`local`, or one `session.open` made) that was never added as an SSH target.
    pub fn host_address(&self, id: &str) -> Result<Option<(Option<String>, Option<u16>)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT target, port FROM hosts WHERE id = ?1")?;
        let mut rows = statement.query(params![id])?;

        match rows.next()? {
            Some(row) => Ok(Some((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<i64>>(1)?.and_then(|port| u16::try_from(port).ok()),
            ))),
            None => Ok(None),
        }
    }

    /// Records the address a host was added with - the target **and** the port.
    ///
    /// `upsert_host` deliberately does not touch these columns (it is the path `host.add` uses while a
    /// probe is still running, and it must not erase an address), so this is the one writer.
    pub fn set_host_address(&self, id: &str, target: &str, port: Option<u16>) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "UPDATE hosts SET target = ?2, port = ?3 WHERE id = ?1",
            params![id, target, port.map(i64::from)],
        )?;

        Ok(())
    }

    /// Records the fingerprint a person pinned for a host.
    pub fn set_host_key(&self, id: &str, fingerprint: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute("UPDATE hosts SET host_key = ?2 WHERE id = ?1", params![id, fingerprint])?;

        Ok(())
    }

    /// The id of the host already added for this `user@host`, if there is one.
    ///
    /// `host.add` asks this before it writes. Without it, adding the same machine twice made two rows
    /// that the sidebar cannot tell apart - a list of `Website, Website, Website` with no way back.
    pub fn host_id_for_target(&self, target: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT id FROM hosts WHERE target = ?1 LIMIT 1")?;
        let mut rows = statement.query(params![target])?;

        match rows.next()? {
            Some(row) => Ok(Some(row.get::<_, String>(0)?)),
            None => Ok(None),
        }
    }

    /// One host's sessions, newest first - the rows the sidebar draws under a host.
    ///
    /// `minutesAgo` is computed here rather than in the UI: the reducer is pure, so the age of a
    /// session has to travel in the data (master spec section 3.3).
    pub fn sessions_for_host(&self, host_id: &str) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT s.id, s.host_id, s.title, s.prompt, s.state, s.unread, s.attention,
                    CAST((julianday('now') - julianday(s.updated_at)) * 1440 AS INTEGER),
                    s.project_id, p.root
             FROM sessions s LEFT JOIN projects p ON p.id = s.project_id
             WHERE s.host_id = ?1 ORDER BY s.updated_at DESC",
        )?;

        let rows = statement.query_map(params![host_id], |row| {
            Ok(serde_json::json!({
                "sessionId": row.get::<_, String>(0)?,
                "hostId": row.get::<_, String>(1)?,
                "title": row.get::<_, String>(2)?,
                "prompt": row.get::<_, String>(3)?,
                "state": row.get::<_, String>(4)?,
                "unread": row.get::<_, i64>(5)?,
                "attention": row.get::<_, Option<String>>(6)?,
                "minutesAgo": row.get::<_, i64>(7)?.max(0),
                "projectId": row.get::<_, Option<String>>(8)?,
                "projectRoot": row.get::<_, Option<String>>(9)?,
            }))
        })?;

        let sessions = collect_json(rows)?;

        Ok(sessions)
    }

    /// `session.list`: every host with its sessions - the tree a window folds when it opens.
    ///
    /// This is what makes an added host survive a restart. The window used to ask for nothing, so a
    /// host that had been added on Monday was gone on Tuesday even though its row was still here.
    ///
    /// Since 0.7.6 a row also carries the folder its chat works in (`projectId`, `projectRoot`), joined
    /// from `projects`: that is what makes "which directory am I in" answerable after a reload, and what
    /// the engines read as their working directory.
    pub fn hosts_with_sessions(&self) -> Result<Vec<Value>> {
        let mut hosts = self.hosts()?;

        for host in &mut hosts {
            let id = host.get("hostId").and_then(Value::as_str).unwrap_or_default().to_string();
            let sessions = self.sessions_for_host(&id)?;
            let address: (Option<String>, Option<i64>, Option<String>) = {
                let connection = self.connection.lock().unwrap();
                connection
                    .query_row("SELECT target, port, host_key FROM hosts WHERE id = ?1", params![id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .unwrap_or((None, None, None))
            };

            if let Value::Object(map) = host {
                map.insert("sessions".to_string(), Value::Array(sessions));
                map.insert("target".to_string(), address.0.map_or(Value::Null, Value::String));
                /* The port travels with the address (0.7.13): the window's host card says which port a
                   VPS answers on, which is the fact 0.7.0 threw away. */
                map.insert("port".to_string(), address.1.map_or(Value::Null, Value::from));
                map.insert("hostKey".to_string(), address.2.map_or(Value::Null, Value::String));
            }
        }

        Ok(hosts)
    }

    /// `host.remove`: deletes the host and everything that belongs to it, and answers how many
    /// sessions went with it.
    ///
    /// Children first, because `foreign_keys` is ON: a session references its host, a turn references
    /// its session, and deleting the parent with children still pointing at it is a foreign-key
    /// error rather than a cascade. The **event log is not touched**: it is append-only, and the
    /// removal is itself an event (`HostRemoved`), so the history of the host stays readable.
    pub fn delete_host(&self, id: &str) -> Result<i64> {
        let connection = self.connection.lock().unwrap();
        let sessions: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sessions WHERE host_id = ?1",
            params![id],
            |row| row.get(0),
        )?;

        connection.execute(
            "DELETE FROM rewind_stack WHERE session_id IN (SELECT id FROM sessions WHERE host_id = ?1)",
            params![id],
        )?;
        connection.execute(
            "DELETE FROM checkpoints WHERE session_id IN (SELECT id FROM sessions WHERE host_id = ?1)",
            params![id],
        )?;
        connection.execute(
            "DELETE FROM turns WHERE session_id IN (SELECT id FROM sessions WHERE host_id = ?1)",
            params![id],
        )?;
        connection.execute(
            "DELETE FROM permissions WHERE session_id IN (SELECT id FROM sessions WHERE host_id = ?1)",
            params![id],
        )?;
        connection.execute("DELETE FROM sessions WHERE host_id = ?1", params![id])?;
        /* The host's projects go with it - and after its sessions, because a session references its
           project and `foreign_keys` is ON. */
        connection.execute("DELETE FROM projects WHERE host_id = ?1", params![id])?;
        connection.execute("DELETE FROM hosts WHERE id = ?1", params![id])?;

        Ok(sessions)
    }



    /* ----------------------------------------------------------------------------------------------
     * Projects: the folder a chat works in (0.7.6)
     *
     * The `projects` table has been in the schema since the first migration and `sessions.project_id`
     * with it, but nothing ever wrote either: a chat had no working directory, the engines ran wherever
     * the daemon had been started, and both `checkpoint_create` and `rewind_apply` took the root as a
     * *parameter* the app never sent - so a checkpoint hashed no files and a rewind restored only the
     * conversation. These functions are the half that was missing.
     * -------------------------------------------------------------------------------------------- */

    /// The next project id: `pr<n>`, one past the highest this database has ever handed out.
    ///
    /// Read from the table rather than taken from `events.seq()`, and that is the difference between a
    /// working id and a silently destructive one. `project.add` pushes **no event** - a folder is a place,
    /// not a happening - so the event sequence does not move between two adds, and two folders opened in
    /// the same second would both have been `pr6`: the second `INSERT OR REPLACE` would have *replaced* the
    /// first project's row, changing its root and name and dragging its chats along with it. `MAX` only
    /// ever moves up, across restarts and across removals, which is what makes the id an identity.
    pub fn next_project_id(&self) -> Result<String> {
        let connection = self.connection.lock().unwrap();
        let highest: i64 = connection.query_row(
            "SELECT COALESCE(MAX(CAST(SUBSTR(id, 3) AS INTEGER)), 0) FROM projects WHERE id LIKE 'pr%'",
            [],
            |row| row.get(0),
        )?;

        Ok(format!("pr{}", highest + 1))
    }

    pub fn add_project(&self, id: &str, host_id: &str, root: &str, name: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT OR REPLACE INTO projects (id, host_id, root, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, host_id, root, name, now],
        )?;

        Ok(())
    }

    /// The project a host already has for a root, if any - what makes `project.add` idempotent, so
    /// opening the same folder twice does not leave two rows for one directory.
    pub fn project_at(&self, host_id: &str, root: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT id FROM projects WHERE host_id = ?1 AND root = ?2")?;
        let mut rows = statement.query_map(params![host_id, root], |row| row.get::<_, String>(0))?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn project(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT id, host_id, root, name FROM projects WHERE id = ?1")?;
        let mut rows = statement.query_map(params![id], |row| {
            Ok(serde_json::json!({
                "projectId": row.get::<_, String>(0)?,
                "hostId": row.get::<_, String>(1)?,
                "root": row.get::<_, String>(2)?,
                "name": row.get::<_, String>(3)?,
            }))
        })?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Every project, with how many chats are bound to it - `project.list`, and the rows a window folds
    /// so a folder survives a reload.
    pub fn projects(&self) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT p.id, p.host_id, p.root, p.name, COUNT(s.id)
             FROM projects p LEFT JOIN sessions s ON s.project_id = p.id
             GROUP BY p.id ORDER BY p.created_at",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(serde_json::json!({
                "projectId": row.get::<_, String>(0)?,
                "hostId": row.get::<_, String>(1)?,
                "root": row.get::<_, String>(2)?,
                "name": row.get::<_, String>(3)?,
                "chats": row.get::<_, i64>(4)?,
            }))
        })?;

        collect_json(rows)
    }

    /// Removes a project and **unbinds** its chats rather than deleting them, and answers how many were
    /// unbound.
    ///
    /// A chat is a conversation: closing the folder it pointed at must not throw the conversation away,
    /// which is the silent loss principle P4 forbids. `project_id` goes to `NULL`, the session rows stay,
    /// and the chats that were looking at that folder now say they have none.
    pub fn remove_project(&self, id: &str) -> Result<i64> {
        let connection = self.connection.lock().unwrap();
        let unbound = connection.execute(
            "UPDATE sessions SET project_id = NULL WHERE project_id = ?1",
            params![id],
        )? as i64;

        connection.execute("DELETE FROM projects WHERE id = ?1", params![id])?;

        Ok(unbound)
    }

    /// The folder a session works in, or `None` when it has no project.
    ///
    /// This is the one lookup the engines and the file tools need: `engine.start` uses it as the CLI's
    /// working directory, and `fs.search` / `git.*` / the checkpoint and rewind paths use it as their
    /// root when the caller does not send one.
    pub fn session_project_root(&self, session_id: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT p.root FROM sessions s JOIN projects p ON p.id = s.project_id WHERE s.id = ?1",
        )?;
        let mut rows = statement.query_map(params![session_id], |row| row.get::<_, String>(0))?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// One session row, or `None` - what `session.fork` reads before it copies anything.
    pub fn session(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT s.id, s.host_id, s.project_id, p.root, s.title, s.prompt, s.state
             FROM sessions s LEFT JOIN projects p ON p.id = s.project_id WHERE s.id = ?1",
        )?;

        let mut rows = statement.query_map(params![id], |row| {
            Ok(serde_json::json!({
                "sessionId": row.get::<_, String>(0)?,
                "hostId": row.get::<_, String>(1)?,
                "projectId": row.get::<_, Option<String>>(2)?,
                "projectRoot": row.get::<_, Option<String>>(3)?,
                "title": row.get::<_, String>(4)?,
                "prompt": row.get::<_, String>(5)?,
                "state": row.get::<_, String>(6)?,
            }))
        })?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Copies a session's turns into another session, up to and including `at_turn` (all of them when it is
    /// `None`), and answers how many were copied - what `session.fork` is.
    ///
    /// **New ids**, because `turns.id` is a primary key and a copied row that kept its id would collide with
    /// the original (and the panel's React key would be the same for two different chats). `f<parent>-<n>`
    /// is also how a log reads: this turn came from that chat.
    ///
    /// The turn *rows* are copied rather than shared, so the fork's transcript is its own: a rewind in the
    /// fork cannot reach back into the parent, and `session.list` after a reload shows the fork's
    /// conversation instead of an empty chat.
    pub fn copy_turns(&self, from_session: &str, to_session: &str, at_turn: Option<i64>) -> Result<i64> {
        let connection = self.connection.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        let copied = connection.execute(
            "INSERT OR REPLACE INTO turns (id, session_id, ordinal, engine, model, tier, prompt, answer, summary, state, started_at, finished_at)
             SELECT 'f' || session_id || '-' || ordinal, ?2, ordinal, engine, model, tier, prompt, answer, summary,
                    CASE state WHEN 'running' THEN 'done' ELSE state END, started_at, ?3
             FROM turns WHERE session_id = ?1 AND (?4 IS NULL OR ordinal <= ?4)",
            params![from_session, to_session, now, at_turn],
        )? as i64;

        if copied > 0 {
            connection.execute(
                "UPDATE sessions SET turn_count = ?2, updated_at = ?3 WHERE id = ?1",
                params![to_session, copied, now],
            )?;
        }

        Ok(copied)
    }

    pub fn insert_session(
        &self,
        id: &str,
        host_id: &str,
        title: &str,
        prompt: &str,
        project_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT OR REPLACE INTO sessions (id, host_id, project_id, title, prompt, state, turn_count, unread, updated_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'idle', 0, 0, ?6, ?6)",
            params![id, host_id, project_id, title, prompt, now],
        )?;

        Ok(())
    }

    /// Renames, re-states or re-points a session. `None` leaves a column alone, which is what makes one
    /// method serve `session.update`, the state changes a turn causes, and `Open folder` binding a chat to
    /// a project.
    pub fn update_session(
        &self,
        id: &str,
        title: Option<&str>,
        state: Option<&str>,
        attention: Option<&str>,
        unread: Option<i64>,
        project_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "UPDATE sessions SET
                title = COALESCE(?2, title),
                state = COALESCE(?3, state),
                attention = COALESCE(?4, attention),
                unread = COALESCE(?5, unread),
                project_id = COALESCE(?6, project_id),
                updated_at = ?7
             WHERE id = ?1",
            params![id, title, state, attention, unread, project_id, now],
        )?;

        Ok(())
    }

    /// `session.close`: the chat and everything hanging off it - children first.
    ///
    /// `foreign_keys` is ON, and a turn, a checkpoint, a rewind entry and a permission row each reference
    /// their session, so `DELETE FROM sessions` on a chat that had **run a turn** answered
    /// `FOREIGN KEY constraint failed` - and the window showed that sentence as a toast instead of deleting
    /// the chat. It never showed up while testing with a freshly made, unused chat, which is the only kind
    /// the 0.7.5 probe deleted; the 0.7.6 probe deleted the sidebar's first row, which has turns, and the
    /// daemon's own words came back. `delete_host` has done this correctly for a whole host since 0.6.1;
    /// this is the same four statements for one session.
    pub fn delete_session(&self, id: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute("DELETE FROM rewind_stack WHERE session_id = ?1", params![id])?;
        connection.execute("DELETE FROM checkpoints WHERE session_id = ?1", params![id])?;
        connection.execute("DELETE FROM turns WHERE session_id = ?1", params![id])?;
        connection.execute("DELETE FROM permissions WHERE session_id = ?1", params![id])?;
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

    /// The engine a turn was started on, or `None` for a turn this store has never seen.
    pub fn turn_engine(&self, id: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT engine FROM turns WHERE id = ?1")?;
        let mut rows = statement.query(params![id])?;

        Ok(match rows.next()? {
            Some(row) => Some(row.get::<_, String>(0)?),
            None => None,
        })
    }

    /// The chat a turn belongs to, or `None` for a turn this store has never seen.
    pub fn turn_session(&self, id: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().unwrap();

        Ok(connection
            .query_row("SELECT session_id FROM turns WHERE id = ?1", params![id], |row| row.get::<_, String>(0))
            .optional()?)
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
