//! The Trust Kernel's tables (0.12): the audit ledger, usage, trust scores, the restore journal, the
//! glossary and intents, and the agency half - sites, deploys, health and approvals.
//!
//! One more `impl Store`, on the same connection, so a kernel write and the event it belongs to can never
//! be split across two writers of the same file.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::sqlite::Store;

/// The hash a chain starts from.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One audit row, whole - what `audit.verify` re-hashes.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRow {
    pub seq: i64,
    pub ts: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub actor: String,
    pub kind: String,
    pub summary: String,
    pub detail: String,
    pub prev_hash: String,
    pub hash: String,
}

impl AuditRow {
    pub fn to_json(&self) -> Value {
        json!({
            "seq": self.seq,
            "ts": self.ts,
            "sessionId": self.session_id,
            "turnId": self.turn_id,
            "actor": self.actor,
            "kind": self.kind,
            "summary": self.summary,
            "detail": serde_json::from_str::<Value>(&self.detail).unwrap_or(Value::Null),
            "prevHash": self.prev_hash,
            "hash": self.hash,
        })
    }
}

/// The hash of one row: every field that means something, joined, with the row before it's hash first.
/// A change to any of them - or a row taken out - changes this row's hash or the next row's `prev`.
#[allow(clippy::too_many_arguments)]
pub fn audit_hash(
    prev: &str,
    seq: i64,
    ts: &str,
    session: Option<&str>,
    turn: Option<&str>,
    actor: &str,
    kind: &str,
    summary: &str,
    detail: &str,
) -> String {
    let mut hasher = Sha256::new();

    for part in [prev, &seq.to_string(), ts, session.unwrap_or(""), turn.unwrap_or(""), actor, kind, summary, detail] {
        hasher.update(part.as_bytes());
        hasher.update([0x1f]);
    }

    hex::encode(hasher.finalize())
}

fn row_to_audit(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRow> {
    Ok(AuditRow {
        seq: row.get(0)?,
        ts: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        actor: row.get(4)?,
        kind: row.get(5)?,
        summary: row.get(6)?,
        detail: row.get(7)?,
        prev_hash: row.get(8)?,
        hash: row.get(9)?,
    })
}

const AUDIT_COLUMNS: &str = "seq, ts, session_id, turn_id, actor, kind, summary, detail, prev_hash, hash";

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl Store {
    /* -----------------------------------------------------------------------------------------
     * The audit ledger
     * -------------------------------------------------------------------------------------- */

    /// Appends one audit row, chained to the row before it. The read of the head and the insert happen
    /// under one lock, so two writers can never chain to the same head.
    pub fn append_audit(
        &self,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        actor: &str,
        kind: &str,
        summary: &str,
        detail: &Value,
    ) -> Result<AuditRow> {
        let connection = self.connection.lock().unwrap();
        let head: Option<(i64, String)> = connection
            .query_row("SELECT seq, hash FROM audit ORDER BY seq DESC LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        let (seq, prev) = match head {
            Some((seq, hash)) => (seq + 1, hash),
            None => (1, GENESIS.to_string()),
        };
        let ts = now();
        let detail = detail.to_string();
        let summary: String = summary.chars().take(400).collect();
        let hash = audit_hash(&prev, seq, &ts, session_id, turn_id, actor, kind, &summary, &detail);

        connection.execute(
            "INSERT INTO audit (seq, ts, session_id, turn_id, actor, kind, summary, detail, prev_hash, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![seq, ts, session_id, turn_id, actor, kind, summary, detail, prev, hash],
        )?;

        Ok(AuditRow {
            seq,
            ts,
            session_id: session_id.map(str::to_string),
            turn_id: turn_id.map(str::to_string),
            actor: actor.to_string(),
            kind: kind.to_string(),
            summary,
            detail,
            prev_hash: prev,
            hash,
        })
    }

    /// Audit rows, newest first: one chat's, one turn's, or all of them.
    pub fn audit_rows(&self, session_id: Option<&str>, turn_id: Option<&str>, limit: i64) -> Result<Vec<AuditRow>> {
        let connection = self.connection.lock().unwrap();
        let limit = limit.clamp(1, 5000);
        let rows = match (session_id, turn_id) {
            (_, Some(turn)) => {
                let mut statement = connection.prepare(&format!(
                    "SELECT {AUDIT_COLUMNS} FROM audit WHERE turn_id = ?1 ORDER BY seq DESC LIMIT ?2"
                ))?;
                let mapped = statement.query_map(params![turn, limit], row_to_audit)?;

                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            }
            (Some(session), None) => {
                let mut statement = connection.prepare(&format!(
                    "SELECT {AUDIT_COLUMNS} FROM audit WHERE session_id = ?1 ORDER BY seq DESC LIMIT ?2"
                ))?;
                let mapped = statement.query_map(params![session, limit], row_to_audit)?;

                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            }
            (None, None) => {
                let mut statement =
                    connection.prepare(&format!("SELECT {AUDIT_COLUMNS} FROM audit ORDER BY seq DESC LIMIT ?1"))?;
                let mapped = statement.query_map(params![limit], row_to_audit)?;

                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };

        Ok(rows)
    }

    /// The whole ledger, oldest first - what a verification walks.
    pub fn audit_chain(&self) -> Result<Vec<AuditRow>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(&format!("SELECT {AUDIT_COLUMNS} FROM audit ORDER BY seq ASC"))?;
        let rows = statement.query_map([], row_to_audit)?.collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    }

    /// For a test that proves tampering is caught: rewrites one row's summary without re-chaining.
    #[cfg(test)]
    pub fn tamper_audit(&self, seq: i64, summary: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute("UPDATE audit SET summary = ?2 WHERE seq = ?1", params![seq, summary])?;

        Ok(())
    }

    /// One turn's row: its chat, engine, model and prompt - what the kernel needs to describe it.
    pub fn turn_row(&self, turn_id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();

        Ok(connection
            .query_row(
                "SELECT session_id, ordinal, engine, model, tier, prompt, answer, state, started_at, finished_at FROM turns WHERE id = ?1",
                params![turn_id],
                |row| {
                    Ok(json!({
                        "turnId": turn_id,
                        "sessionId": row.get::<_, String>(0)?,
                        "ordinal": row.get::<_, i64>(1)?,
                        "engine": row.get::<_, String>(2)?,
                        "model": row.get::<_, String>(3)?,
                        "tier": row.get::<_, String>(4)?,
                        "prompt": row.get::<_, String>(5)?,
                        "answer": row.get::<_, String>(6)?,
                        "state": row.get::<_, String>(7)?,
                        "startedAt": row.get::<_, String>(8)?,
                        "finishedAt": row.get::<_, Option<String>>(9)?,
                    }))
                },
            )
            .optional()?)
    }

    /// The turns that are running right now, across every chat - what the kill switch stops.
    pub fn running_turns(&self) -> Result<Vec<(String, String, String)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT id, session_id, engine FROM turns WHERE state = 'running' ORDER BY started_at DESC")?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    }

    /* -----------------------------------------------------------------------------------------
     * Usage and cost
     * -------------------------------------------------------------------------------------- */

    /// Records (or replaces) one turn's usage.
    #[allow(clippy::too_many_arguments)]
    pub fn record_usage(
        &self,
        turn_id: &str,
        session_id: &str,
        project_root: Option<&str>,
        site_id: Option<&str>,
        engine: &str,
        model: &str,
        provider: Option<&str>,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        cost_source: &str,
        estimate_usd: Option<f64>,
        baseline_usd: Option<f64>,
    ) -> Result<()> {
        let connection = self.connection.lock().unwrap();
        let ts = now();
        let day = &ts[..10];

        connection.execute(
            "INSERT OR REPLACE INTO usage (turn_id, session_id, project_root, site_id, engine, model, provider,
               input_tokens, output_tokens, cost_usd, cost_source, estimate_usd, baseline_usd, day, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                turn_id,
                session_id,
                project_root,
                site_id,
                engine,
                model,
                provider,
                input_tokens as i64,
                output_tokens as i64,
                cost_usd,
                cost_source,
                estimate_usd,
                baseline_usd,
                day,
                ts
            ],
        )?;

        Ok(())
    }

    /// Every usage row, newest first, optionally only since a day (`YYYY-MM-DD`).
    pub fn usage_rows(&self, since_day: Option<&str>) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT turn_id, session_id, project_root, site_id, engine, model, provider, input_tokens, output_tokens,
                    cost_usd, cost_source, estimate_usd, baseline_usd, day, created_at
             FROM usage WHERE day >= ?1 ORDER BY created_at DESC LIMIT 20000",
        )?;
        let rows = statement
            .query_map(params![since_day.unwrap_or("0000-00-00")], |row| {
                Ok(json!({
                    "turnId": row.get::<_, String>(0)?,
                    "sessionId": row.get::<_, String>(1)?,
                    "projectRoot": row.get::<_, Option<String>>(2)?,
                    "siteId": row.get::<_, Option<String>>(3)?,
                    "engine": row.get::<_, String>(4)?,
                    "model": row.get::<_, String>(5)?,
                    "provider": row.get::<_, Option<String>>(6)?,
                    "inputTokens": row.get::<_, i64>(7)?,
                    "outputTokens": row.get::<_, i64>(8)?,
                    "costUsd": row.get::<_, f64>(9)?,
                    "costSource": row.get::<_, String>(10)?,
                    "estimateUsd": row.get::<_, Option<f64>>(11)?,
                    "baselineUsd": row.get::<_, Option<f64>>(12)?,
                    "day": row.get::<_, String>(13)?,
                    "ts": row.get::<_, String>(14)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    }

    /// One turn's usage row.
    pub fn usage_of(&self, turn_id: &str) -> Result<Option<Value>> {
        Ok(self.usage_rows(None)?.into_iter().find(|row| row["turnId"] == turn_id))
    }

    /* -----------------------------------------------------------------------------------------
     * Trust scores
     * -------------------------------------------------------------------------------------- */

    pub fn save_trust_score(&self, turn_id: &str, session_id: &str, score: i64, level: &str, reasons: &Value) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT OR REPLACE INTO trust_scores (turn_id, session_id, score, level, reasons, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![turn_id, session_id, score, level, reasons.to_string(), now()],
        )?;

        Ok(())
    }

    pub fn trust_score(&self, turn_id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();

        Ok(connection
            .query_row(
                "SELECT session_id, score, level, reasons, created_at FROM trust_scores WHERE turn_id = ?1",
                params![turn_id],
                |row| {
                    Ok(json!({
                        "turnId": turn_id,
                        "sessionId": row.get::<_, String>(0)?,
                        "score": row.get::<_, i64>(1)?,
                        "level": row.get::<_, String>(2)?,
                        "reasons": serde_json::from_str::<Value>(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
                        "ts": row.get::<_, String>(4)?,
                    }))
                },
            )
            .optional()?)
    }

    /* -----------------------------------------------------------------------------------------
     * Checkpoint labels, the irreversible mark, and the restore journal
     * -------------------------------------------------------------------------------------- */

    pub fn set_checkpoint_label(&self, id: &str, label: Option<&str>) -> Result<bool> {
        let connection = self.connection.lock().unwrap();
        let label = label.map(str::trim).filter(|label| !label.is_empty());

        Ok(connection.execute("UPDATE checkpoints SET label = ?2 WHERE id = ?1", params![id, label])? > 0)
    }

    /// Marks the newest checkpoint of a chat: something after it cannot be undone by a rewind.
    pub fn mark_irreversible(&self, session_id: &str, reason: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let newest: Option<(String, Option<String>)> = connection
            .query_row(
                "SELECT id, irreversible FROM checkpoints WHERE session_id = ?1 ORDER BY turn DESC LIMIT 1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let Some((id, existing)) = newest else {
            return Ok(None);
        };
        let reason = match existing {
            Some(previous) if !previous.contains(reason) => format!("{previous}; {reason}"),
            Some(previous) => previous,
            None => reason.to_string(),
        };

        connection.execute("UPDATE checkpoints SET irreversible = ?2 WHERE id = ?1", params![id, reason])?;

        Ok(Some(id))
    }

    /// Which turn wrote a checkpoint - what lets a rewind take that turn out of the conversation too.
    pub fn set_checkpoint_turn(&self, id: &str, turn_id: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute("UPDATE checkpoints SET turn_id = ?2 WHERE id = ?1", params![id, turn_id])?;

        Ok(())
    }

    /// The turns a rewind to `checkpoint` takes out of the conversation, with the state each had: the
    /// turn that wrote the checkpoint and every turn after it. A checkpoint no turn wrote (a Save, a
    /// command) drops only the turns that started after it.
    pub fn turns_after_checkpoint(&self, session_id: &str, checkpoint: &Value) -> Result<Vec<(String, String)>> {
        let connection = self.connection.lock().unwrap();
        let owner_ordinal: Option<i64> = match checkpoint["turnId"].as_str() {
            Some(turn) => connection
                .query_row("SELECT ordinal FROM turns WHERE id = ?1", params![turn], |row| row.get(0))
                .optional()?,
            None => None,
        };
        let rows: Vec<(String, String, i64, String)> = {
            let mut statement =
                connection.prepare("SELECT id, state, ordinal, started_at FROM turns WHERE session_id = ?1 ORDER BY ordinal ASC")?;
            let mapped = statement.query_map(params![session_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;

            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let since = checkpoint["ts"].as_str().unwrap_or_default().to_string();

        Ok(rows
            .into_iter()
            .filter(|(_, state, ordinal, started)| {
                state != "rewound"
                    && match owner_ordinal {
                        Some(owner) => *ordinal >= owner,
                        None => started.as_str() > since.as_str(),
                    }
            })
            .map(|(id, state, _, _)| (id, state))
            .collect())
    }

    /// A chat's rewind frames, newest first: the branches of its timeline that a redo or a switch can reach.
    pub fn rewind_frames(&self, session_id: &str) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, turn, checkpoint_id, pushed_at, frame FROM rewind_stack WHERE session_id = ?1 ORDER BY id DESC",
        )?;
        let rows = statement
            .query_map(params![session_id], |row| {
                let frame: Value = row.get::<_, Option<String>>(4)?.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_else(|| json!({}));
                let checkpoints = frame["rows"].as_array().cloned().unwrap_or_default();

                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "turn": row.get::<_, i64>(1)?,
                    "checkpointId": row.get::<_, String>(2)?,
                    "pushedAt": row.get::<_, String>(3)?,
                    "branch": frame["branch"].as_bool().unwrap_or(false),
                    "checkpoints": checkpoints.len(),
                    "title": checkpoints.last().and_then(|row| row["title"].as_str()).unwrap_or("The state before a switch"),
                    "turns": frame["turns"].as_array().map(Vec::len).unwrap_or(0),
                    "hasFiles": frame["nowSha"].as_str().is_some_and(|sha| sha.len() == 40),
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    }

    /// Makes one frame the newest, so the next redo takes it: how a switch reaches any branch, not only
    /// the last one. `false` when the frame is not this chat's.
    pub fn promote_rewind_frame(&self, session_id: &str, frame_id: i64) -> Result<bool> {
        let connection = self.connection.lock().unwrap();
        let next: i64 = connection.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM rewind_stack", [], |row| row.get(0))?;

        Ok(connection.execute("UPDATE rewind_stack SET id = ?3 WHERE id = ?1 AND session_id = ?2", params![frame_id, session_id, next])? > 0)
    }

    pub fn set_turn_state(&self, turn_id: &str, state: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute("UPDATE turns SET state = ?2 WHERE id = ?1", params![turn_id, state])?;

        Ok(())
    }

    pub fn journal_open(&self, session_id: &str, root: &str, host_id: Option<&str>, target: &str, before: Option<&str>, scope: &str) -> Result<i64> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO restore_journal (session_id, root, host_id, target_sha, before_sha, scope, state, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'started', ?7)",
            params![session_id, root, host_id, target, before, scope, now()],
        )?;

        Ok(connection.last_insert_rowid())
    }

    pub fn journal_close(&self, id: i64, state: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "UPDATE restore_journal SET state = ?2, finished_at = ?3 WHERE id = ?1",
            params![id, state, now()],
        )?;

        Ok(())
    }

    /// Restores that started and never finished - a daemon that stopped in the middle of one.
    pub fn open_journals(&self) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, session_id, root, host_id, target_sha, before_sha, scope, started_at FROM restore_journal WHERE state = 'started'",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "sessionId": row.get::<_, String>(1)?,
                    "root": row.get::<_, String>(2)?,
                    "hostId": row.get::<_, Option<String>>(3)?,
                    "target": row.get::<_, String>(4)?,
                    "before": row.get::<_, Option<String>>(5)?,
                    "scope": row.get::<_, String>(6)?,
                    "startedAt": row.get::<_, String>(7)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    }

    /* -----------------------------------------------------------------------------------------
     * Glossary and intents
     * -------------------------------------------------------------------------------------- */

    pub fn glossary(&self, scope: &str) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT term, meaning, scope FROM glossary WHERE scope = ?1 OR scope = 'global' ORDER BY term ASC",
        )?;
        let rows = statement
            .query_map(params![scope], |row| {
                Ok(json!({ "term": row.get::<_, String>(0)?, "meaning": row.get::<_, String>(1)?, "scope": row.get::<_, String>(2)? }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    }

    pub fn set_glossary(&self, scope: &str, term: &str, meaning: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap();
        let term = term.trim().to_lowercase();

        if meaning.trim().is_empty() {
            connection.execute("DELETE FROM glossary WHERE scope = ?1 AND term = ?2", params![scope, term])?;
        } else {
            connection.execute(
                "INSERT INTO glossary (scope, term, meaning, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(scope, term) DO UPDATE SET meaning = ?3",
                params![scope, term, meaning.trim(), now()],
            )?;
        }

        Ok(())
    }

    pub fn save_intent(&self, id: &str, session_id: Option<&str>, text: &str, spec: &Value, state: &str, corrections: i64) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO intents (id, session_id, text, spec, state, corrections, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET spec = ?4, state = ?5, corrections = ?6",
            params![id, session_id, text, spec.to_string(), state, corrections, now()],
        )?;

        Ok(())
    }

    pub fn intent(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();

        Ok(connection
            .query_row(
                "SELECT session_id, text, spec, state, corrections, created_at FROM intents WHERE id = ?1",
                params![id],
                |row| {
                    Ok(json!({
                        "id": id,
                        "sessionId": row.get::<_, Option<String>>(0)?,
                        "text": row.get::<_, String>(1)?,
                        "spec": serde_json::from_str::<Value>(&row.get::<_, String>(2)?).unwrap_or(Value::Null),
                        "state": row.get::<_, String>(3)?,
                        "corrections": row.get::<_, i64>(4)?,
                        "ts": row.get::<_, String>(5)?,
                    }))
                },
            )
            .optional()?)
    }

    /// How the Intent Engine is doing: how many readings were confirmed as they were, corrected, or asked back.
    pub fn intent_stats(&self) -> Result<Value> {
        let connection = self.connection.lock().unwrap();
        let count = |sql: &str| -> rusqlite::Result<i64> { connection.query_row(sql, [], |row| row.get(0)) };

        Ok(json!({
            "parsed": count("SELECT COUNT(*) FROM intents")?,
            "confirmed": count("SELECT COUNT(*) FROM intents WHERE state = 'confirmed'")?,
            "corrected": count("SELECT COUNT(*) FROM intents WHERE corrections > 0")?,
            "cancelled": count("SELECT COUNT(*) FROM intents WHERE state = 'cancelled'")?,
        }))
    }

    /* -----------------------------------------------------------------------------------------
     * Sites, deploys, health and approvals
     * -------------------------------------------------------------------------------------- */

    pub fn upsert_site(&self, id: &str, name: &str, host_id: &str, root: &str, url: &str, config: &Value) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO sites (id, name, host_id, root, url, config, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET name = ?2, host_id = ?3, root = ?4, url = ?5, config = ?6",
            params![id, name, host_id, root, url, config.to_string(), now()],
        )?;

        Ok(())
    }

    fn site_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "name": row.get::<_, String>(1)?,
            "hostId": row.get::<_, String>(2)?,
            "root": row.get::<_, String>(3)?,
            "url": row.get::<_, String>(4)?,
            "config": serde_json::from_str::<Value>(&row.get::<_, String>(5)?).unwrap_or_else(|_| json!({})),
            "createdAt": row.get::<_, String>(6)?,
        }))
    }

    pub fn sites(&self) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT id, name, host_id, root, url, config, created_at FROM sites ORDER BY name ASC")?;
        let rows = statement.query_map([], Self::site_from)?.collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    }

    pub fn site(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();

        Ok(connection
            .query_row(
                "SELECT id, name, host_id, root, url, config, created_at FROM sites WHERE id = ?1",
                params![id],
                Self::site_from,
            )
            .optional()?)
    }

    pub fn remove_site(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().unwrap();

        connection.execute("DELETE FROM health WHERE site_id = ?1", params![id])?;

        Ok(connection.execute("DELETE FROM sites WHERE id = ?1", params![id])? > 0)
    }

    pub fn next_site_id(&self) -> Result<String> {
        let connection = self.connection.lock().unwrap();
        let count: i64 = connection.query_row("SELECT COUNT(*) FROM sites", [], |row| row.get(0))?;

        Ok(format!("site-{}-{}", count + 1, &uuid::Uuid::new_v4().simple().to_string()[..6]))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_deploy(&self, id: &str, site_id: &str, kind: &str, state: &str, steps: &Value, backup: Option<&Value>, note: &str, finished: bool) -> Result<()> {
        let connection = self.connection.lock().unwrap();
        let ts = now();

        connection.execute(
            "INSERT INTO deploys (id, site_id, kind, state, steps, backup, note, started_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET state = ?4, steps = ?5, backup = COALESCE(?6, backup), note = ?7, finished_at = ?9",
            params![
                id,
                site_id,
                kind,
                state,
                steps.to_string(),
                backup.map(Value::to_string),
                note,
                ts,
                if finished { Some(ts.clone()) } else { None }
            ],
        )?;

        Ok(())
    }

    fn deploy_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "siteId": row.get::<_, String>(1)?,
            "kind": row.get::<_, String>(2)?,
            "state": row.get::<_, String>(3)?,
            "steps": serde_json::from_str::<Value>(&row.get::<_, String>(4)?).unwrap_or_else(|_| json!([])),
            "backup": row.get::<_, Option<String>>(5)?.and_then(|text| serde_json::from_str::<Value>(&text).ok()),
            "note": row.get::<_, String>(6)?,
            "startedAt": row.get::<_, String>(7)?,
            "finishedAt": row.get::<_, Option<String>>(8)?,
        }))
    }

    pub fn deploys(&self, site_id: Option<&str>, limit: i64) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let columns = "id, site_id, kind, state, steps, backup, note, started_at, finished_at";
        let rows = match site_id {
            Some(site) => {
                let mut statement = connection.prepare(&format!(
                    "SELECT {columns} FROM deploys WHERE site_id = ?1 ORDER BY started_at DESC LIMIT ?2"
                ))?;
                let mapped = statement.query_map(params![site, limit], Self::deploy_from)?;

                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => {
                let mut statement =
                    connection.prepare(&format!("SELECT {columns} FROM deploys ORDER BY started_at DESC LIMIT ?1"))?;
                let mapped = statement.query_map(params![limit], Self::deploy_from)?;

                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };

        Ok(rows)
    }

    pub fn deploy(&self, id: &str) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap();

        Ok(connection
            .query_row(
                "SELECT id, site_id, kind, state, steps, backup, note, started_at, finished_at FROM deploys WHERE id = ?1",
                params![id],
                Self::deploy_from,
            )
            .optional()?)
    }

    pub fn save_health(&self, site_id: &str, report: &Value) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT INTO health (site_id, ts, report) VALUES (?1, ?2, ?3)",
            params![site_id, now(), report.to_string()],
        )?;
        /* A site checked every five minutes for a year is 105k rows; the last 2000 are the history anyone reads. */
        connection.execute(
            "DELETE FROM health WHERE site_id = ?1 AND id NOT IN (SELECT id FROM health WHERE site_id = ?1 ORDER BY id DESC LIMIT 2000)",
            params![site_id],
        )?;

        Ok(())
    }

    pub fn health_history(&self, site_id: &str, limit: i64) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT ts, report FROM health WHERE site_id = ?1 ORDER BY id DESC LIMIT ?2")?;
        let rows = statement
            .query_map(params![site_id, limit], |row| {
                let mut report = serde_json::from_str::<Value>(&row.get::<_, String>(1)?).unwrap_or_else(|_| json!({}));

                report["ts"] = json!(row.get::<_, String>(0)?);

                Ok(report)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    }

    pub fn save_approval(&self, id: &str, subject: &str, kind: &str, requested_by: &str, note: &str, token: Option<&str>) -> Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "INSERT OR REPLACE INTO approvals (id, subject, kind, state, requested_by, note, token, created_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7)",
            params![id, subject, kind, requested_by, note, token, now()],
        )?;

        Ok(())
    }

    pub fn decide_approval(&self, id: &str, state: &str, decided_by: &str, note: Option<&str>) -> Result<bool> {
        let connection = self.connection.lock().unwrap();

        Ok(connection.execute(
            "UPDATE approvals SET state = ?2, decided_by = ?3, decided_at = ?4, note = COALESCE(?5, note) WHERE id = ?1",
            params![id, state, decided_by, now(), note],
        )? > 0)
    }

    pub fn approvals(&self, subject: Option<&str>) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap();
        let map = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Value> {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "subject": row.get::<_, String>(1)?,
                "kind": row.get::<_, String>(2)?,
                "state": row.get::<_, String>(3)?,
                "requestedBy": row.get::<_, String>(4)?,
                "decidedBy": row.get::<_, Option<String>>(5)?,
                "note": row.get::<_, String>(6)?,
                "token": row.get::<_, Option<String>>(7)?,
                "createdAt": row.get::<_, String>(8)?,
                "decidedAt": row.get::<_, Option<String>>(9)?,
            }))
        };
        let columns = "id, subject, kind, state, requested_by, decided_by, note, token, created_at, decided_at";
        let rows = match subject {
            Some(subject) => {
                let mut statement = connection
                    .prepare(&format!("SELECT {columns} FROM approvals WHERE subject = ?1 ORDER BY created_at DESC"))?;
                let mapped = statement.query_map(params![subject], map)?;

                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => {
                let mut statement =
                    connection.prepare(&format!("SELECT {columns} FROM approvals ORDER BY created_at DESC LIMIT 200"))?;
                let mapped = statement.query_map([], map)?;

                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ledger_chains_each_row_to_the_one_before() {
        let store = Store::in_memory().unwrap();
        let first = store.append_audit(Some("s1"), None, "person", "SessionOpened", "Opened", &json!({})).unwrap();
        let second = store.append_audit(Some("s1"), Some("t1"), "claude_code", "TurnStarted", "Fix it", &json!({"x": 1})).unwrap();

        assert_eq!(first.prev_hash, GENESIS);
        assert_eq!(second.prev_hash, first.hash);
        assert_eq!(store.audit_chain().unwrap().len(), 2);
        assert_eq!(store.audit_rows(Some("s1"), None, 10).unwrap()[0].seq, 2, "newest first");
    }

    #[test]
    fn a_site_a_deploy_and_its_health_round_trip() {
        let store = Store::in_memory().unwrap();

        store.upsert_site("site-1", "Shop", "vps-1", "/var/www/shop", "https://shop.test", &json!({"steps": []})).unwrap();
        store.save_deploy("dep-1", "site-1", "production", "running", &json!([]), None, "", false).unwrap();
        store.save_deploy("dep-1", "site-1", "production", "success", &json!([{"name": "build"}]), Some(&json!({"path": "/b"})), "ok", true).unwrap();
        store.save_health("site-1", &json!({"http": 200})).unwrap();

        assert_eq!(store.sites().unwrap()[0]["url"], "https://shop.test");
        assert_eq!(store.deploy("dep-1").unwrap().unwrap()["state"], "success");
        assert_eq!(store.deploy("dep-1").unwrap().unwrap()["backup"]["path"], "/b");
        assert_eq!(store.health_history("site-1", 5).unwrap()[0]["http"], 200);
    }

    #[test]
    fn glossary_terms_are_kept_per_scope_and_removed_by_an_empty_meaning() {
        let store = Store::in_memory().unwrap();

        store.set_glossary("project:/a", "Ghor", "page").unwrap();
        store.set_glossary("global", "dokan", "shop").unwrap();

        let terms = store.glossary("project:/a").unwrap();

        assert_eq!(terms.len(), 2);
        assert!(terms.iter().any(|term| term["term"] == "ghor" && term["meaning"] == "page"));

        store.set_glossary("project:/a", "ghor", "").unwrap();
        assert_eq!(store.glossary("project:/a").unwrap().len(), 1);
    }
}
