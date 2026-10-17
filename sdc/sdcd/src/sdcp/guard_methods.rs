//! The things that need the person's yes - `permission.*`, `checkpoint.*`, `rewind.*`, `duel.*`, `console.*`. Split out of `methods.rs` (P0-3).
//!
//! Nothing here changed: the arms and the bodies are the ones that used to sit in the big `impl Daemon`, and
//! the private helpers they lean on are still in `methods.rs`, which a child module can reach. The `_` arm
//! passes the method on to the next link in the chain, exactly as the single `_` arm used to.

use super::*;

impl Daemon {
    /// The methods this module owns. Moved from `methods.rs` unchanged.
    pub(super) fn dispatch_guard(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            /* Permission, checkpoints, rewind -------------------------------------------------- */
            "permission.request" => self.permission_request(envelope, &*out),
            "permission.resolve" => self.permission_resolve(envelope, &*out),
            "checkpoint.create" => self.checkpoint_create(envelope, &*out),
            "checkpoint.list" => Ok(json!({
                "checkpoints": crate::checkpoints::list(self.store(), &envelope.require_str("sessionId")?)?
            })),
            "checkpoint.restore" => self.checkpoint_restore(envelope, &*out),
            "rewind.apply" => self.rewind_apply(envelope, &*out),
            "rewind.redo" => self.rewind_redo(envelope, &*out),

            /* First-open secret sweep (Phase 0.4) ---------------------------------------------- */
            "rotation.get" => self.rotation_get(),
            "rotation.set" => self.rotation_set(envelope),
            "rotation.escalate" => self.rotation_escalate(envelope, out),
            "ship.check" => self.ship_check(envelope),
            "secrets.sweep" => self.secrets_sweep(envelope),
            "secrets.move" => self.secrets_move(envelope),

            /* Duel, console, events ----------------------------------------------------------- */
            "duel.start" => self.duel_start(envelope, &*out),
            "duel.keep" | "duel.discard" => self.duel_keep(envelope, &*out),
            "console.attach" => self.console_attach(envelope),
            "console.detach" => self.console_detach(envelope),
            _ => self.dispatch_fleet(envelope, out),
        }
    }

    /// A project's root on this machine, when the path is one the person opened (`project.add`): the sweep reads only those.
    fn local_project(&self, envelope: &Envelope) -> Result<std::path::PathBuf, ErrorObject> {
        let path = envelope.require_str("path")?;
        let wanted = std::path::Path::new(&path).canonicalize().map_err(|_| ErrorObject::bad_request("that folder does not exist on this computer"))?;
        let known = self.store().projects().map_err(ErrorObject::internal)?.iter().any(|project| {
            project["hostId"] == "local" && project["root"].as_str().is_some_and(|root| std::path::Path::new(root).canonicalize().is_ok_and(|real| real == wanted))
        });

        if known {
            Ok(wanted)
        } else {
            Err(ErrorObject::bad_request("open the folder in SDC first; the sweep reads only folders you opened"))
        }
    }

    /// `secrets.sweep`: reads an opened folder once, with no model, and lists where secrets already sit. With `firstOnly`
    /// it answers `skipped` for a folder it has swept before.
    pub(super) fn secrets_sweep(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let root = self.local_project(envelope)?;
        let first_only = envelope.opt_bool("firstOnly");
        let key = format!("sweep.done.{}", crate::trust::sweep::keychain_name(&root, "", 0));

        if first_only && self.store().setting(&key).ok().flatten().is_some() {
            return Ok(json!({ "skipped": true, "findings": [], "files": 0, "truncated": false }));
        }

        let mut answer = crate::trust::sweep::to_json(&crate::trust::sweep::run(&root));

        if first_only {
            let _ = self.store().set_setting(&key, "1");
        }

        answer["skipped"] = json!(false);

        Ok(answer)
    }

    /// `ship.check`: the five Ship Safe checks on an opened folder, and on a site address when one is given. Blocking (it asks the
    /// site and the public package registries), so the window shows a spinner and never waits on the daemon's other calls.
    pub(super) fn ship_check(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let root = self.local_project(envelope)?;
        let site = envelope.opt_str("url").map(|url| url.trim().to_string()).filter(|url| !url.is_empty());

        if let Some(site) = site.as_deref() {
            if !site.starts_with("http://") && !site.starts_with("https://") {
                return Err(ErrorObject::bad_request("the site address must start with http:// or https://"));
            }
        }

        Ok(crate::trust::shipsafe::run(&root, site.as_deref(), &crate::trust::shipsafe::Registries::default()))
    }

    /// `secrets.move`: one found value goes into the OS keychain and leaves a marker in its file.
    pub(super) fn secrets_move(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let root = self.local_project(envelope)?;
        let file = envelope.require_str("file")?;
        let line = envelope.opt_i64("line").filter(|line| *line >= 1).ok_or_else(|| ErrorObject::bad_request("`line` is required"))? as usize;

        crate::trust::sweep::move_to_keychain(&root, &file, line, |name, value| crate::auth::keychain::set(name, value).map_err(|error| error.message.clone()))
            .map_err(ErrorObject::bad_request)
    }

    pub(super) fn permission_request(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let permission_id = self.state.fresh_id("perm-");

        out.push(
            event::permission_requested(json!({
                "permissionId": permission_id,
                "sessionId": session_id,
                "turnId": envelope.opt_str("turnId"),
                /* The caller's own words. This used to answer every request with a fixed "Delete a file /
                   src/database.js / your database connection settings" card, whatever was being asked. */
                "title": envelope.opt_str("title").unwrap_or_else(|| "Allow this action?".into()),
                "sub": envelope.opt_str("sub").unwrap_or_default(),
                "action": envelope.opt_str("action").unwrap_or_else(|| "edit".into()),
                "target": envelope.opt_str("target").unwrap_or_default(),
                "risk": envelope.opt_str("risk").unwrap_or_else(|| "MUTATING".into()),
                "explain": envelope.opt_str("explain").unwrap_or_default(),
                "checkpointId": Value::Null,
            })),
            Some(session_id),
            None,
        );

        Ok(json!({ "permissionId": permission_id }))
    }

    pub(super) fn permission_resolve(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let permission_id = envelope.require_str("permissionId")?;
        let decision = envelope.opt_str("decision").unwrap_or_else(|| "deny".into());
        /* An agent may be blocked on this question (agent::gate); the answer is what lets it go on. */
        let delivered = crate::agent::gate::resolve(&permission_id, &decision);

        out.push(event::permission_resolved(&permission_id, &decision), None, None);

        /* The card is answered, so the chat that was waiting on it is working again: the amber `!` comes off
           now, not when the turn the answer set going is over. */
        if let Some(session_id) = take_card_session(&permission_id) {
            note_waiting(self.store(), &session_id, None, out);
        }

        Ok(json!({ "decision": decision, "delivered": delivered }))
    }

    pub(super) fn checkpoint_create(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let title = envelope.opt_str("title").unwrap_or_else(|| "Checkpoint".into());
        let turn = envelope.opt_i64("turn").unwrap_or_else(|| self.state.events.seq());
        /* The session's folder when the caller does not send one - see `root_for`, and on whichever
           machine that folder is (`Subject`). */
        let subject = self.subject(envelope)?;
        let fresh = crate::checkpoints::create(
            self.store(),
            &session_id,
            turn,
            &title,
            subject.snapshot(),
            crate::checkpoints::screenshot::capture(),
        )?;

        out.push(
            event::checkpoint_saved(&session_id, fresh.to_event_payload()),
            Some(session_id),
            None,
        );

        Ok(json!({ "checkpointId": fresh.id, "filesHash": fresh.files_hash }))
    }

    pub(super) fn rewind_apply(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let turn_id = envelope.require_str("turnId")?;
        /* Where the turn began on the log - its `TurnStarted` seq, which is what checkpoints are numbered
           by. The id's own number was that seq until 0.16.1 made ids unique; it stays the fallback. */
        let turn = self
            .store()
            .turn_started_seq(&turn_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| crate::checkpoints::turn_of(&turn_id));
        /* The session's folder when the caller does not send one: a rewind restores the files of the chat
           it belongs to, which is the folder that chat works in - locally or on a host (0.7.13). */
        self.refuse_while_running(&session_id)?;

        let subject = self.subject(envelope)?;
        let host = self.host_id_for(envelope)?;
        let applied = crate::rewind::apply(self.store(), &session_id, turn, subject.snapshot(), host.as_deref())?;

        /* TM-8: nothing at or after that point is nothing rewound - and saying "Rewound" anyway was the
           kind of claim P4 forbids. */
        if applied.turns == 0 {
            return Err(ErrorObject::not_found(
                "There is no checkpoint at or after that point in this chat, so nothing was rewound.",
            ));
        }

        out.push(
            applied.to_event_payload(&session_id),
            Some(session_id.clone()),
            None,
        );
        out.push(
            event::toast(
                "Rewound: the folder is back as it was at that checkpoint",
                Some("Undo this"),
                Some(10_000),
            ),
            Some(session_id),
            None,
        );

        Ok(json!({ "removedTurns": applied.turns, "restoredFiles": applied.files }))
    }

    pub(super) fn rewind_redo(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());

        self.refuse_while_running(&session_id)?;

        /* The folder, so a redo puts the files back as well as the list (it only moved rows until v4). */
        let subject = self.subject(envelope)?;
        let host = self.host_id_for(envelope)?;

        match crate::rewind::redo(self.store(), &session_id, subject.snapshot(), host.as_deref())? {
            Some(applied) => {
                out.push(applied.to_event_payload(&session_id), Some(session_id), None);

                Ok(json!({ "turn": applied.turn }))
            }
            None => Ok(json!({ "turn": Value::Null })),
        }
    }

    /// Spec section 16.6: two engines, one prompt, two panes.
    pub(super) fn duel_start(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let prompt = envelope.opt_str("prompt").unwrap_or_default();
        let requested: Vec<String> = envelope
            .params
            .get("engines")
            .and_then(Value::as_array)
            .map(|engines| {
                engines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_else(|| vec!["claude_code".to_string(), "codex".to_string()]);
        let engines = duel::plan(&requested)?;
        let duel_id = self.state.fresh_id("duel-");

        out.push(
            event::duel_started(
                &duel_id,
                &session_id,
                &prompt,
                json!(engines),
                json!(duel::pending_panes(&engines)),
            ),
            Some(session_id),
            None,
        );

        Ok(json!({ "duelId": duel_id, "engines": engines }))
    }

    /// `duel.keep` names the winner; `duel.discard` is `Keep neither`. Both archive, neither deletes.
    pub(super) fn duel_keep(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let duel_id = envelope.require_str("duelId")?;
        let keep = envelope.opt_str("keep");
        let message = match keep.as_deref() {
            Some(engine) => format!("{engine} kept; the other run is archived"),
            None => "Both runs archived".to_string(),
        };

        out.push(event::duel_resolved(&duel_id, keep.as_deref()), None, None);
        out.push(event::toast(&message, None, None), None, None);

        Ok(json!({ "duelId": duel_id, "kept": keep }))
    }

    pub(super) fn console_attach(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let url = envelope.opt_str("url").unwrap_or_else(|| "http://localhost:3000".into());

        self.state.console.attach(&session_id, &url)
    }

    pub(super) fn console_detach(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());

        self.state.console.detach(&session_id)
    }

    /// `checkpoint.restore`: the same restore as a rewind, reached by the checkpoint's id rather than
    /// by a turn number. It answers with how many turns went back, which is what `restored` means.
    pub(super) fn checkpoint_restore(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let checkpoint_id = envelope.require_str("checkpointId")?;
        let checkpoint = self
            .store()
            .checkpoint(&checkpoint_id)
            .map_err(ErrorObject::internal)?
            .ok_or_else(|| ErrorObject::not_found(format!("no checkpoint `{checkpoint_id}`")))?;
        let session_id = checkpoint["sessionId"].as_str().unwrap_or("s1").to_string();
        let turn = checkpoint["turn"].as_i64().unwrap_or(0);

        self.refuse_while_running(&session_id)?;

        let subject = self.subject(envelope)?;
        let host = self.host_id_for(envelope)?;
        let applied = crate::rewind::apply(self.store(), &session_id, turn, subject.snapshot(), host.as_deref())?;

        out.push(applied.to_event_payload(&session_id), Some(session_id), None);

        Ok(json!({ "restored": applied.turns, "turn": turn }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(daemon: &Daemon, method: &str, params: Value) -> Result<Value, ErrorObject> {
        let state = daemon.state.clone();
        let out: Arc<dyn Notifier> = Arc::new(crate::sdcp::notifications::ChannelNotifier::new(state.events.clone(), state.fanout.clone()));
        let envelope = Envelope {
            v: crate::SDCP_VERSION.to_string(),
            id: "t-1".to_string(),
            method: method.to_string(),
            params: params.as_object().cloned().unwrap_or_default(),
            host_id: None,
        };

        daemon.dispatch(&envelope, out)
    }

    #[test]
    fn the_sweep_reads_only_a_folder_the_person_opened_and_the_first_open_only_once() {
        let state = crate::DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon");
        let daemon = Daemon::new(state.clone());
        let folder = std::env::temp_dir().join(format!("sdcd-sweep-method-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(".env"), "API_KEY=FAKEvalue9Zq4Xw7Lm2Pt8\n").unwrap();

        let path = folder.to_string_lossy().to_string();
        let refused = call(&daemon, "secrets.sweep", json!({ "path": path })).expect_err("not opened yet");

        assert_eq!(refused.code, "bad_request");

        state.store.ensure_host("local", "Local", "local", "connected").unwrap();
        state.store.add_project("p-sweep", "local", &path, "x").unwrap();

        let first = call(&daemon, "secrets.sweep", json!({ "path": path, "firstOnly": true })).unwrap();

        assert_eq!(first["findings"][0]["file"], ".env");
        assert!(!first.to_string().contains("FAKEvalue"), "no value in the answer");

        let again = call(&daemon, "secrets.sweep", json!({ "path": path, "firstOnly": true })).unwrap();

        assert_eq!(again["skipped"], true, "a folder is swept on first open only");

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_chain_reads_without_a_file_and_a_local_only_project_cannot_escalate() {
        let state = crate::DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon");
        let daemon = Daemon::new(state.clone());
        let read = call(&daemon, "rotation.get", json!({})).unwrap();

        assert!(read["rows"].is_array() && read["global"]["askBeforeCloud"].is_boolean() && read["path"].as_str().unwrap().ends_with("rotation.toml"), "{read}");

        let folder = std::env::temp_dir().join(format!("sdcd-escalate-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join(".sdc")).unwrap();
        std::fs::write(folder.join(".sdc/policy.toml"), "privacy = \"local-only\"\n").unwrap();
        state.store.ensure_host("local", "Local", "local", "connected").unwrap();
        state.store.add_project("p-esc", "local", &folder.to_string_lossy(), "x").unwrap();
        state.store.insert_session("s-esc", "local", "A chat", "hello", Some("p-esc")).unwrap();

        let refused = call(&daemon, "rotation.escalate", json!({ "sessionId": "s-esc", "confirm": true })).expect_err("local-only: nothing may leave");

        assert_eq!(refused.code, "permission_denied", "{refused:?}");
        assert!(crate::data::sent::read(&folder).0.is_empty(), "and nothing was recorded as sent");

        let _ = std::fs::remove_dir_all(&folder);
    }
}
