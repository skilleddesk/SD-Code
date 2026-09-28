//! The Trust Kernel's SDCP methods (0.12): the audit ledger, the policy, the kill switch, the cost
//! governor, the scores, the Time Machine's per-file tools and the Proof Pack.
//!
//! A child module of `methods`, so these handlers use the same private helpers (`subject`,
//! `remote_for`, `host_id_for`) as every other handler instead of a second copy of them.

use std::sync::Arc;

use serde_json::{json, Value};

use super::{Daemon, Subject};
use crate::sdcp::envelope::{Envelope, ErrorObject};
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;
use crate::trust::{cost, kill, ledger, policy::Policy, proof};

impl Daemon {
    /// The kernel's methods; anything else goes on to the Intent Engine and the agency methods.
    pub(super) fn dispatch_kernel(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            "audit.list" => self.audit_list(envelope),
            "audit.verify" => Ok(ledger::verify(self.store())),
            "policy.get" => self.policy_get(envelope),
            "policy.set" => self.policy_set(envelope, &*out),
            "kill.all" => Ok(self.kill_all(&*out)),
            "kill.list" => Ok(json!({ "active": kill::active().iter().map(kill::to_json).collect::<Vec<_>>() })),
            "cost.summary" => Ok(cost::summary(self.store())),
            "cost.estimate" => self.cost_estimate(envelope),
            "cost.budget.set" => self.cost_budget_set(envelope),
            "trust.score" => self.trust_score(envelope, &*out),
            "checkpoint.label" => self.checkpoint_label(envelope, &*out),
            "checkpoint.files" => self.checkpoint_files(envelope),
            "checkpoint.fileDiff" => self.checkpoint_file_diff(envelope),
            "checkpoint.restoreFile" => self.checkpoint_restore_file(envelope, &*out),
            "proof.export" => self.proof_export(envelope),
            _ => self.dispatch_intent(envelope, out),
        }
    }

    /// **Long-task memory** (0.12): what a turn should know that the conversation alone may not carry - the
    /// plan the agent left unfinished in this chat (kept by the daemon, so it survives a restart) and the
    /// project's own `.sdc/memory.md`. `None` when there is nothing to remember.
    /// The continuity brief of one turn (0.12.5, `crate::continuity`): conventions from the folder, and a
    /// hand-over when earlier turns of the chat were written by another model. `None` without a folder.
    pub(super) fn continuity_brief(
        &self,
        session_id: &str,
        turn_id: &str,
        root: Option<&str>,
        remote: Option<&crate::ssh::Ssh>,
        engine: &str,
        model: &str,
    ) -> Option<String> {
        let root = root?;
        let facts = match remote {
            Some(ssh) => crate::continuity::gather_remote(ssh, root, engine),
            None => crate::continuity::gather_local(std::path::Path::new(root), engine),
        };
        let earlier: Vec<crate::continuity::Author> = self
            .store()
            .turns(session_id)
            .unwrap_or_default()
            .iter()
            .filter(|turn| turn["turnId"] != turn_id && turn["state"] != "rewound")
            .map(|turn| crate::continuity::Author {
                turn_id: turn["turnId"].as_str().unwrap_or_default().to_string(),
                engine: turn["engine"].as_str().unwrap_or_default().to_string(),
                model: turn["model"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
        let edited = self.store().edited_files(session_id).unwrap_or_default();

        crate::continuity::brief(&facts, &earlier, &edited, engine, model)
    }

    pub(super) fn long_task_memory(&self, session_id: &str, root: Option<&str>, remote: Option<&crate::ssh::Ssh>, include_file: bool) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(steps) = self
            .store()
            .setting(&format!("plan.{session_id}"))
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
        {
            let open = steps.iter().any(|step| step["status"] != "done");

            if open {
                let lines: Vec<String> = steps
                    .iter()
                    .map(|step| format!("- [{}] {}", if step["status"] == "done" { "x" } else if step["status"] == "in_progress" { "~" } else { " " }, step["text"].as_str().unwrap_or_default()))
                    .collect();

                parts.push(format!("Your plan in this chat so far ([x] done, [~] in progress):\n{}", lines.join("\n")));
            }
        }

        if include_file {
            if let Some(root) = root {
                let text = match remote {
                    Some(ssh) => crate::ssh::ops::read(ssh, &format!("{}/.sdc/memory.md", root.trim_end_matches('/')), 16 * 1024)
                        .ok()
                        .and_then(|value| value["text"].as_str().map(str::to_string)),
                    None => std::fs::read_to_string(std::path::Path::new(root).join(".sdc").join("memory.md")).ok(),
                };

                if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
                    parts.push(format!("What this project remembers (.sdc/memory.md):\n{}", text.chars().take(4000).collect::<String>()));
                }
            }
        }

        /* The global memory (0.13): what the person asked SDC to keep in mind in every chat, every project. */
        let global = super::agent_methods::global_memory();

        if !global.trim().is_empty() {
            parts.push(format!("What the person asked SDC to remember everywhere (global memory):\n{}", global.chars().take(3000).collect::<String>()));
        }

        (!parts.is_empty()).then(|| format!("[Long-task memory - from SDC, not from the person]\n{}\n[End of memory]", parts.join("\n\n")))
    }

    /// A rewind or a restore while a turn is still writing to the same folder would race it (TM-7): the
    /// turn's next write lands on the restored files. The person stops the turn first.
    pub(super) fn refuse_while_running(&self, session_id: &str) -> Result<(), ErrorObject> {
        if kill::active().iter().any(|work| work.kind == "turn" && work.session_id.as_deref() == Some(session_id)) {
            return Err(ErrorObject::bad_request(
                "A turn is still running in this chat. Stop it first (Esc), then rewind - otherwise its next change would land on the restored files.",
            ));
        }

        Ok(())
    }

    fn audit_list(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let rows = self
            .store()
            .audit_rows(envelope.opt_str("sessionId").as_deref(), envelope.opt_str("turnId").as_deref(), envelope.opt_i64("limit").unwrap_or(200))
            .map_err(ErrorObject::internal)?;

        Ok(json!({ "entries": rows.iter().map(|row| row.to_json()).collect::<Vec<_>>(), "chain": ledger::verify(self.store()) }))
    }

    /* -----------------------------------------------------------------------------------------
     * Policy
     * -------------------------------------------------------------------------------------- */

    fn policy_where(&self, envelope: &Envelope) -> Result<(Option<String>, Option<crate::ssh::Ssh>), ErrorObject> {
        let root = self.root_for(envelope)?.map(|root| root.to_string_lossy().to_string());

        Ok((root, self.remote_for(envelope)?))
    }

    fn policy_get(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let (root, remote) = self.policy_where(envelope)?;
        let policy = Policy::load(root.as_deref(), remote.as_ref());

        Ok(json!({
            "root": root,
            "path": root.as_ref().map(|root| format!("{}/{}", root.trim_end_matches(['/', '\\']), crate::trust::policy::POLICY_FILE)),
            "exists": policy.source != "default",
            "policy": policy,
            "text": policy.to_toml(),
            "defaults": { "protected": crate::trust::policy::DEFAULT_PROTECTED, "alwaysAsk": crate::trust::policy::DEFAULT_ALWAYS_ASK },
        }))
    }

    /// Writes the folder's `.sdc/policy.toml` - from the person's own text (checked first: a file that
    /// does not parse is refused, not saved), or from the fields the policy screen edits.
    fn policy_set(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let (root, remote) = self.policy_where(envelope)?;
        let root = root.ok_or_else(|| ErrorObject::bad_request("A policy belongs to a folder: open one for this chat first."))?;
        let text = match envelope.opt_str("text") {
            Some(text) => {
                let parsed = Policy::parse(&text, crate::trust::policy::POLICY_FILE);

                if let Some(error) = parsed.error {
                    return Err(ErrorObject::bad_request(error));
                }

                text
            }
            None => {
                let fields = envelope.params.get("policy").cloned().ok_or_else(|| ErrorObject::bad_request("send `text` or `policy`"))?;
                let mut policy: Policy = serde_json::from_value(fields).map_err(|error| ErrorObject::bad_request(format!("the policy is not valid: {error}")))?;

                policy.source = crate::trust::policy::POLICY_FILE.to_string();
                policy.to_toml()
            }
        };
        let path = format!("{}/{}", root.trim_end_matches(['/', '\\']), crate::trust::policy::POLICY_FILE);

        match &remote {
            Some(ssh) => {
                crate::ssh::ops::write(ssh, &path, &text)?;
            }
            None => {
                let path = std::path::Path::new(&path);

                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(ErrorObject::internal)?;
                }

                std::fs::write(path, &text).map_err(ErrorObject::internal)?;
            }
        }

        let policy = Policy::load(Some(&root), remote.as_ref());
        let session = envelope.opt_str("sessionId");

        let _ = self.store().append_audit(session.as_deref(), None, "person", "PolicySaved", &format!("Policy saved for {root}"), &json!({ "text": text }));
        out.push(event::toast("Policy saved: .sdc/policy.toml", None, None), session, None);

        Ok(json!({ "policy": policy, "path": path }))
    }

    /* -----------------------------------------------------------------------------------------
     * The kill switch
     * -------------------------------------------------------------------------------------- */

    /// Stops everything: every running turn (its engine told to stop, here or on a host), every Verify
    /// run, deploy and playbook, and every process the daemon started. Then each chat that was working
    /// gets a checkpoint of its folder **as it was when it stopped**, so nothing is lost and a rewind is
    /// one click away.
    fn kill_all(&self, out: &dyn Notifier) -> Value {
        let stopped = kill::cancel_all();
        let mut sessions: Vec<String> = Vec::new();

        for work in &stopped {
            if work.kind == "turn" {
                if let Some(engine) = self.store().turn_engine(&work.id).ok().flatten().and_then(|engine| self.state.engines.get(&engine)) {
                    let turn = work.id.clone();

                    tokio::spawn(async move {
                        engine.cancel(&turn).await;
                    });
                }

                out.push(
                    event::turn_completed(&work.id, "Stopped by the kill switch", "", Some(false)),
                    work.session_id.clone(),
                    Some(work.id.clone()),
                );
            }

            if let Some(session) = &work.session_id {
                if !sessions.contains(session) {
                    sessions.push(session.clone());
                }
            }
        }

        let processes = self.state.pty.running();

        self.state.pty.close_all();

        let mut checkpoints = Vec::new();

        for session in &sessions {
            let envelope = Envelope {
                v: String::new(),
                id: String::new(),
                method: "kill.all".into(),
                params: serde_json::Map::from_iter([("sessionId".to_string(), json!(session))]),
                host_id: None,
            };

            let Ok(subject) = self.subject(&envelope) else {
                continue;
            };

            if subject.root.is_none() {
                continue;
            }

            if let Ok(fresh) = crate::checkpoints::create(
                self.store(),
                session,
                self.state.events.seq(),
                "Kill switch: the folder as it was when everything stopped",
                subject.snapshot(),
                None,
            ) {
                let _ = self.store().set_checkpoint_label(&fresh.id, Some("Kill switch"));

                checkpoints.push(json!({ "sessionId": session, "checkpointId": fresh.id }));
                out.push(event::checkpoint_saved(session, fresh.to_event_payload()), Some(session.clone()), None);
            }
        }

        let stopped_json: Vec<Value> = stopped.iter().map(kill::to_json).collect();

        out.push(event::kill_switch(json!(stopped_json), json!(checkpoints)), None, None);

        json!({ "stopped": stopped_json, "processes": processes, "checkpoints": checkpoints })
    }

    /* -----------------------------------------------------------------------------------------
     * The cost governor
     * -------------------------------------------------------------------------------------- */

    /// Connected models the router may suggest: every connected provider's catalogue rows, and the models
    /// Ollama actually has installed.
    fn connected_models(&self) -> Vec<(String, String)> {
        let providers = crate::providers::list(self.store());
        let connected: Vec<String> = providers
            .iter()
            .filter(|provider| provider["status"] == "connected")
            .filter_map(|provider| provider["id"].as_str().map(str::to_string))
            .collect();
        let mut models = Vec::new();

        for block in crate::providers::models::blocked() {
            if !connected.contains(&block.id) {
                continue;
            }

            for row in &block.models {
                if let Some(id) = row["id"].as_str() {
                    models.push((block.id.clone(), id.to_string()));
                }
            }
        }

        if connected.iter().any(|id| id == "ollama") {
            for model in crate::engines::ollama::list_models() {
                models.push(("ollama".to_string(), model));
            }
        }

        models
    }

    fn cost_estimate(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let prompt = envelope.opt_str("prompt").unwrap_or_default();
        let engine = envelope.opt_str("engine").unwrap_or_else(|| "native_api".into());
        let model = envelope.opt_str("model").unwrap_or_default();
        let provider = envelope.opt_str("provider").filter(|id| !id.is_empty());
        let history_chars: usize = envelope
            .opt_str("sessionId")
            .and_then(|session| crate::session_bridge::history_for(self.store(), &session).ok())
            .map(|history| history.iter().map(|message| message.text.len()).sum())
            .unwrap_or(0);
        let estimate = cost::estimate(&engine, provider.as_deref(), &model, &prompt, history_chars, envelope.opt_bool("agent"));
        let cheaper = cost::cheaper(&prompt, provider.as_deref(), &model, &self.connected_models());

        Ok(json!({ "estimate": estimate, "cheaper": cheaper, "spent": cost::spent(self.store(), envelope.opt_str("sessionId").as_deref()), "budgets": cost::budgets(self.store()) }))
    }

    fn cost_budget_set(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let mut budgets = cost::budgets(self.store());

        for key in ["turn", "chat", "day", "month"] {
            if let Some(value) = envelope.params.get(key) {
                budgets[key] = match value.as_f64() {
                    Some(usd) if usd > 0.0 => json!(usd),
                    _ => Value::Null,
                };
            }
        }

        self.store().set_setting("cost.budget", &budgets.to_string()).map_err(ErrorObject::internal)?;

        if let Some(baseline) = envelope.opt_str("baseline") {
            self.store().set_setting("cost.baseline", baseline.trim()).map_err(ErrorObject::internal)?;
        }

        Ok(cost::summary(self.store()))
    }

    fn trust_score(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let turn_id = envelope.require_str("turnId")?;
        let row = self
            .store()
            .turn_row(&turn_id)
            .map_err(ErrorObject::internal)?
            .ok_or_else(|| ErrorObject::not_found(format!("no turn `{turn_id}`")))?;
        let session = row["sessionId"].as_str().unwrap_or_default().to_string();
        let root = self.store().session_project_root(&session).map_err(ErrorObject::internal)?;
        let policy = Policy::load(root.as_deref(), self.remote_for(envelope)?.as_ref());

        super::score_turn(&self.state, out, &session, &turn_id, 0, &policy, root.as_deref());

        Ok(self.store().trust_score(&turn_id).map_err(ErrorObject::internal)?.unwrap_or(Value::Null))
    }

    /* -----------------------------------------------------------------------------------------
     * The Time Machine, one file at a time
     * -------------------------------------------------------------------------------------- */

    /// A checkpoint and the chat it belongs to, with the folder's subject resolved from **that** chat.
    fn checkpoint_subject(&self, envelope: &Envelope) -> Result<(Value, String, Subject), ErrorObject> {
        let id = envelope.require_str("checkpointId")?;
        let checkpoint = self
            .store()
            .checkpoint(&id)
            .map_err(ErrorObject::internal)?
            .ok_or_else(|| ErrorObject::not_found(format!("no checkpoint `{id}`")))?;
        let session = checkpoint["sessionId"].as_str().unwrap_or_default().to_string();
        let mut scoped = envelope.clone();

        scoped.params.insert("sessionId".into(), json!(session));

        let subject = self.subject(&scoped)?;

        if subject.root.is_none() {
            return Err(ErrorObject::bad_request("This checkpoint's chat has no folder, so there are no files to compare."));
        }

        Ok((checkpoint, session, subject))
    }

    fn sha_of(checkpoint: &Value) -> Result<String, ErrorObject> {
        let sha = checkpoint["filesHash"].as_str().unwrap_or_default();

        if sha.len() == 40 && sha.chars().all(|character| character.is_ascii_hexdigit()) {
            Ok(sha.to_string())
        } else {
            Err(ErrorObject::bad_request("This checkpoint recorded a conversation, not files (the chat had no folder then)."))
        }
    }

    fn checkpoint_label(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("checkpointId")?;
        let label = envelope.opt_str("label");

        if !self.store().set_checkpoint_label(&id, label.as_deref()).map_err(ErrorObject::internal)? {
            return Err(ErrorObject::not_found(format!("no checkpoint `{id}`")));
        }

        let row = self.store().checkpoint(&id).map_err(ErrorObject::internal)?.unwrap_or(Value::Null);
        let session = row["sessionId"].as_str().unwrap_or_default().to_string();

        out.push(event::checkpoint_updated(&session, row.clone()), Some(session), None);

        Ok(json!({ "checkpoint": row }))
    }

    /// What changed since a checkpoint, file by file: the Time Machine's before/after list (TM-5).
    fn checkpoint_files(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let (checkpoint, _, subject) = self.checkpoint_subject(envelope)?;
        let sha = Self::sha_of(&checkpoint)?;
        let files = match (&subject.remote, &subject.root) {
            (Some(ssh), Some(_)) => crate::ssh::ops::shadow_files_since(ssh, &subject.root_text, &sha)?,
            (None, Some(root)) => crate::git::files_since(root, &sha)?,
            _ => Vec::new(),
        };

        Ok(json!({
            "checkpoint": checkpoint,
            "files": files.iter().map(|(status, path)| json!({ "status": status, "path": path })).collect::<Vec<_>>(),
        }))
    }

    fn checkpoint_file_diff(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let (checkpoint, _, subject) = self.checkpoint_subject(envelope)?;
        let sha = Self::sha_of(&checkpoint)?;
        let path = envelope.require_str("path")?;
        let diff = match (&subject.remote, &subject.root) {
            (Some(ssh), Some(_)) => crate::ssh::ops::shadow_file_diff(ssh, &subject.root_text, &sha, &path)?,
            (None, Some(root)) => crate::git::file_diff(root, &sha, &path)?,
            _ => String::new(),
        };

        Ok(json!({ "path": path, "diff": diff }))
    }

    /// Puts one file back as a checkpoint had it (TM-3). The folder's state is checkpointed first, so the
    /// restore itself can be undone (P5), and the restore runs inside the journal.
    fn checkpoint_restore_file(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let (checkpoint, session, subject) = self.checkpoint_subject(envelope)?;
        let sha = Self::sha_of(&checkpoint)?;
        let path = envelope.require_str("path")?;
        let clean = crate::git::clean_relative(&path)?;

        self.refuse_while_running(&session)?;

        let undo = crate::checkpoints::create(
            self.store(),
            &session,
            self.state.events.seq(),
            &format!("Before restoring {clean}"),
            subject.snapshot(),
            None,
        )?;

        out.push(event::checkpoint_saved(&session, undo.to_event_payload()), Some(session.clone()), None);

        let host = self.host_id_for(envelope)?;
        let mut outcome = "restored";

        crate::rewind::journaled(self.store(), &session, &subject.snapshot(), host.as_deref(), &sha, Some(&undo.files_hash), &clean, || {
            outcome = match (&subject.remote, &subject.root) {
                (Some(ssh), Some(_)) => crate::ssh::ops::shadow_restore_file(ssh, &subject.root_text, &sha, &clean)?,
                (None, Some(root)) => crate::git::restore_file(root, &sha, &clean)?,
                _ => "restored",
            };

            Ok(())
        })?;

        let checkpoint_id = checkpoint["id"].as_str().unwrap_or_default();

        out.push(event::file_restored(&session, checkpoint_id, &clean, outcome, Some(&undo.id)), Some(session.clone()), None);

        Ok(json!({ "path": clean, "outcome": outcome, "undoCheckpointId": undo.id }))
    }

    /* -----------------------------------------------------------------------------------------
     * The Proof Pack
     * -------------------------------------------------------------------------------------- */

    fn proof_export(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let session = envelope.require_str("sessionId")?;
        let turn = envelope.opt_str("turnId");
        let lang = envelope.opt_str("lang").unwrap_or_else(|| "en".into());
        let log: Vec<crate::sdcp::events::StoredEvent> = self
            .state
            .events
            .since(0)
            .into_iter()
            .filter(|entry| entry.session_id.as_deref() == Some(session.as_str()) || entry.event["sessionId"] == session.as_str())
            .collect();
        let pack = proof::build(self.store(), &log, &session, turn.as_deref());
        let html = proof::html(&pack, &lang);
        let directory = crate::paths::data_dir().map_err(ErrorObject::internal)?.join("proof");

        std::fs::create_dir_all(&directory).map_err(ErrorObject::internal)?;

        let stem = format!(
            "{}-{}-{}",
            session,
            turn.as_deref().unwrap_or("chat"),
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );
        let json_path = directory.join(format!("{stem}.json"));
        let html_path = directory.join(format!("{stem}.html"));

        std::fs::write(&json_path, serde_json::to_string_pretty(&pack).unwrap_or_default()).map_err(ErrorObject::internal)?;
        std::fs::write(&html_path, &html).map_err(ErrorObject::internal)?;

        Ok(json!({
            "jsonPath": json_path.display().to_string(),
            "htmlPath": html_path.display().to_string(),
            "html": html,
            "pack": pack,
        }))
    }
}
