//! Sessions, projects and the event log - `session.*`, `project.*`, `event.*`. Split out of `methods.rs` (P0-3).
//!
//! Nothing here changed: the arms and the bodies are the ones that used to sit in the big `impl Daemon`, and
//! the private helpers they lean on are still in `methods.rs`, which a child module can reach. The `_` arm
//! passes the method on to the next link in the chain, exactly as the single `_` arm used to.

use super::*;

impl Daemon {
    /// The methods this module owns. Moved from `methods.rs` unchanged.
    pub(super) fn dispatch_workspace(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            /* Sessions ------------------------------------------------------------------------ */
            "session.open" => self.session_open(envelope, &*out),
            "session.update" => self.session_update(envelope, &*out),
            "session.fork" => self.session_fork(envelope, &*out),
            "session.close" => self.session_close(envelope, &*out),
            "session.list" => Ok(json!({ "hosts": self.store().hosts_with_sessions().map_err(ErrorObject::internal)? })),

            /* The folder a chat works in (0.7.6). `add` and `list` are answers; `remove` also pushes a
               toast, because closing a folder that had chats in it is a thing the person should see. */
            "project.add" => self.project_add(envelope),
            "project.scaffold" => self.project_scaffold(envelope),
            "project.locate" => self.project_locate(envelope),
            "project.list" => Ok(json!({ "projects": self.store().projects().map_err(ErrorObject::internal)? })),
            "project.remove" => self.project_remove(envelope, &*out),

            "event.list" => Ok(self.event_list(envelope)),
            "event.subscribe" => Ok(json!({ "fromSeq": self.state.events.seq() + 1 })),
            "event.append" => self.event_append(envelope, &*out),
            _ => self.dispatch_file(envelope, out),
        }
    }

    pub(super) fn session_open(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let host_id = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());
        let title = envelope.opt_str("title").unwrap_or_else(|| "New chat".into());
        let prompt = envelope.opt_str("prompt").unwrap_or_default();
        let project_id = envelope.opt_str("projectId").filter(|id| !id.trim().is_empty());
        let session_id = self.state.fresh_id("n");

        let host_name = if host_id == "local" { "Local" } else { host_id.as_str() };

        self.store()
            .ensure_host(&host_id, host_name, "local", "connected")
            .map_err(ErrorObject::internal)?;
        self.store()
            .insert_session(&session_id, &host_id, &title, &prompt, project_id.as_deref())
            .map_err(ErrorObject::internal)?;

        let root = self.project_root_of(project_id.as_deref())?;

        out.push(
            event::session_opened(
                &session_id,
                &host_id,
                &title,
                &prompt,
                project_id.as_deref(),
                root.as_deref(),
            ),
            Some(session_id.clone()),
            None,
        );

        Ok(json!({ "sessionId": session_id, "projectId": project_id, "projectRoot": root }))
    }

    /// The root of a project id, when there is one. Shared by `session.open`, `session.update` and the
    /// two places that answer with a folder.
    pub(super) fn project_root_of(&self, project_id: Option<&str>) -> Result<Option<String>, ErrorObject> {
        let Some(id) = project_id else {
            return Ok(None);
        };

        Ok(self
            .store()
            .project(id)
            .map_err(ErrorObject::internal)?
            .and_then(|project| project.get("root").and_then(Value::as_str).map(str::to_string)))
    }

    /// A workspace for a chat that has none - the folder an agent turn gets instead of a refusal.
    ///
    /// The folder is `~/SDC Workspaces/<title>-<session id>` on the machine the chat lives on (the
    /// session id keeps two chats called "New chat" out of each other's files), created if it is not
    /// there, added as a project (reusing the row when it already is one) and bound to the session.
    /// The `SessionUpdated` at the end is what moves the window's folder chip, so the person sees
    /// where the agent is working the moment the turn starts.
    pub(super) fn provision_workspace(
        &self,
        session_id: &str,
        remote: Option<&crate::ssh::Ssh>,
        out: &dyn Notifier,
    ) -> Result<String, ErrorObject> {
        let session = self.store().session(session_id).map_err(ErrorObject::internal)?;
        let host_id = session
            .as_ref()
            .and_then(|session| session["hostId"].as_str().map(str::to_string))
            .unwrap_or_else(|| "local".to_string());
        let title = session
            .as_ref()
            .and_then(|session| session["title"].as_str().map(str::to_string))
            .unwrap_or_default();
        let name = workspace_name(&title, session_id);

        let root = match remote {
            Some(ssh) => {
                let home = crate::ssh::ops::home(ssh)?;
                let root = format!("{}/SDC Workspaces/{name}", home.trim_end_matches('/'));

                crate::ssh::ops::mkdir(ssh, &root)?;

                root
            }
            None => {
                let home = std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .ok_or_else(|| ErrorObject::internal("no home directory to put a workspace in"))?;
                let path = std::path::PathBuf::from(home).join("SDC Workspaces").join(&name);

                std::fs::create_dir_all(&path)
                    .map_err(|error| ErrorObject::internal(format!("{}: {error}", path.display())))?;

                /* One separator on both machines - the same rule `project.scaffold` follows. */
                path.to_string_lossy().replace('\\', "/")
            }
        };

        let project_id = match self.store().project_at(&host_id, &root).map_err(ErrorObject::internal)? {
            Some(existing) => existing,
            None => {
                let id = self.store().next_project_id().map_err(ErrorObject::internal)?;

                self.store().add_project(&id, &host_id, &root, &name).map_err(ErrorObject::internal)?;

                id
            }
        };

        self.store()
            .update_session(session_id, None, None, None, None, Some(&project_id))
            .map_err(ErrorObject::internal)?;

        out.push(
            event::session_updated(json!({
                "sessionId": session_id,
                "minutesAgo": 0,
                "projectId": project_id,
                "projectRoot": root,
            })),
            Some(session_id.to_string()),
            None,
        );

        Ok(root)
    }

    pub(super) fn session_update(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.require_str("sessionId")?;
        let title = envelope.opt_str("title");
        let state = envelope.opt_str("state");
        /* `Open folder` on an existing chat: the folder it works in changes, and the window's chip
           follows the `SessionUpdated` below rather than guessing. */
        let project_id = envelope.opt_str("projectId").filter(|id| !id.trim().is_empty());
        let root = self.project_root_of(project_id.as_deref())?;

        /* A chat that was closed (or never existed) is refused (0.12.8): an update to it used to be
           accepted and announced, and a folder bound to a deleted chat looked like a fix that never
           reached the chat on screen. */
        let Some(session) = self.store().session(&session_id).map_err(ErrorObject::internal)? else {
            return Err(ErrorObject::not_found(format!("no chat `{session_id}` - it was closed, or never opened")));
        };

        /* A chat works on its own machine's folders (0.16.1). The window's routing could bind a folder on
           host B to a chat that stayed on host A when the move to B failed - a chat that then ran in a
           path that machine does not have. Refused here, where it cannot be skipped. */
        if let Some(id) = project_id.as_deref() {
            let project = self.store().project(id).map_err(ErrorObject::internal)?;
            let chat_host = session["hostId"].as_str().unwrap_or("local");

            if let Some(folder_host) = project.as_ref().and_then(|project| project["hostId"].as_str()) {
                if folder_host != chat_host {
                    return Err(ErrorObject::bad_request(format!(
                        "That folder is on another machine ({folder_host}) than this chat ({chat_host}). Open a chat on {folder_host} to work in it."
                    )));
                }
            }
        }

        self.store()
            .update_session(
                &session_id,
                title.as_deref(),
                state.as_deref(),
                None,
                None,
                project_id.as_deref(),
            )
            .map_err(ErrorObject::internal)?;
        out.push(
            event::session_updated(json!({
                "sessionId": session_id,
                "title": title,
                "state": state,
                "minutesAgo": 0,
                "projectId": project_id,
                "projectRoot": root,
            })),
            Some(session_id),
            None,
        );

        Ok(json!({ "projectId": project_id, "projectRoot": root }))
    }

    /// `session.fork` - declared in the schema and in `protocol/types.ts` since they were written, and
    /// answered `unknown method` until now.
    ///
    /// A fork is a new chat with the same folder and the same conversation **up to a turn**, so a person can
    /// take a different branch without losing where they were. Three things make it honest:
    ///
    ///   * the turns are **copied into the fork's own rows** (`Store::copy_turns`), so a rewind in the fork
    ///     cannot reach back into the parent and a reload shows the fork's conversation rather than an empty
    ///     chat;
    ///   * `atTurn` is **inclusive**, because that is what "fork from here" means when a person points at a
    ///     turn on screen, and omitting it forks the whole conversation;
    ///   * the copied turns are **replayed into the log** (`TurnStarted` / `TurnDelta` / `TurnCompleted` per
    ///     turn). The window's transcript *is* the log - `session.list` carries titles, not turns - so a fork
    ///     whose history lived only in the database would look like an empty chat until an engine was asked
    ///     something. A turn that was `running` in the parent is copied and replayed as `done`: nothing is
    ///     running in the fork.
    pub(super) fn session_fork(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let parent_id = envelope.require_str("sessionId")?;
        let at_turn = envelope.opt_i64("atTurn");
        let parent = self.store().session(&parent_id).map_err(ErrorObject::internal)?;

        let Some(parent) = parent else {
            return Err(ErrorObject::not_found(format!("no session `{parent_id}`")));
        };

        let host_id = parent["hostId"].as_str().unwrap_or("local").to_string();
        let project_id = parent["projectId"].as_str().map(str::to_string);
        let root = parent["projectRoot"].as_str().map(str::to_string);
        let title = parent["title"].as_str().unwrap_or("Chat").to_string();
        let prompt = parent["prompt"].as_str().unwrap_or_default().to_string();
        let fork_id = self.state.fresh_id("n");
        let fork_title = format!("{title} (fork)");

        self.store()
            .insert_session(&fork_id, &host_id, &fork_title, &prompt, project_id.as_deref())
            .map_err(ErrorObject::internal)?;

        let copied = self.store().copy_turns(&parent_id, &fork_id, at_turn).map_err(ErrorObject::internal)?;

        out.push(
            event::session_opened(
                &fork_id,
                &host_id,
                &fork_title,
                &prompt,
                project_id.as_deref(),
                root.as_deref(),
            ),
            Some(fork_id.clone()),
            None,
        );

        /* The transcript, replayed - see the note above. `ordinal` is kept, so the fork's turns are numbered
           the way the parent's were and a rewind to `turn-3` means the same thing in both. */
        for turn in self.store().turns(&fork_id).map_err(ErrorObject::internal)? {
            let ordinal = turn["ordinal"].as_i64().unwrap_or(0);
            let turn_id = turn["turnId"].as_str().unwrap_or_default().to_string();
            let engine = turn["engine"].as_str().unwrap_or("claude_code");
            let model = turn["model"].as_str().unwrap_or("default");
            let tier = turn["tier"].as_str().unwrap_or("Balanced");
            let asked = turn["prompt"].as_str().unwrap_or_default();
            let answer = turn["answer"].as_str().unwrap_or_default();
            let summary = turn["summary"].as_str().unwrap_or_default();
            let state = turn["state"].as_str().unwrap_or("done");
            let session = Some(fork_id.clone());

            out.push(
                event::turn_started(&turn_id, &fork_id, engine, model, tier, asked),
                session.clone(),
                Some(turn_id.clone()),
            );

            if !answer.is_empty() {
                out.push(event::turn_delta(&turn_id, answer), session.clone(), Some(turn_id.clone()));
            }

            out.push(
                event::turn_completed(
                    &turn_id,
                    summary,
                    &json!({ "ordinal": ordinal, "state": state, "replayed": true }).to_string(),
                    None,
                ),
                session,
                Some(turn_id),
            );
        }

        Ok(json!({ "sessionId": fork_id, "turns": copied, "title": fork_title }))
    }

    pub(super) fn session_close(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.require_str("sessionId")?;

        out.push(event::session_closed(&session_id), Some(session_id.clone()), None);
        self.store().delete_session(&session_id).map_err(ErrorObject::internal)?;

        Ok(json!({}))
    }

    /// `project.add`: what `Open folder` calls. Answers with the project, and **reuses** the row when the
    /// host already has that root, because opening the same folder twice must not leave two rows for one
    /// directory - which is also what makes the button safe to press again.
    ///
    /// The validation is the reason this is a method rather than a row the app writes: `is_dir` is asked
    /// here, so a path that is a file, a typo, or a folder on a host that cannot see it is refused in the
    /// daemon's own words instead of becoming a chat whose working directory does not exist.
    pub(super) fn project_add(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let host_id = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());
        let root = envelope.require_str("root")?;

        /*
         * The folder is validated **on the machine that has it**, which is the whole point of a remote
         * project: `/srv/app` is not a folder on this laptop, so asking this laptop would refuse every
         * remote folder there is. `test -d` over `ssh` answers the same question where the answer
         * exists, and the sentence that comes back is the host's own.
         */
        if let Some(ssh) = self.ssh_for(&host_id)? {
            if !crate::ssh::ops::is_dir(&ssh, &root)? {
                return Err(ErrorObject::bad_request(format!("`{root}` is not a folder on {}", ssh.label())));
            }
        } else if !std::path::Path::new(&root).is_dir() {
            return Err(ErrorObject::bad_request(format!("`{root}` is not a folder")));
        }

        if let Some(existing) = self.store().project_at(&host_id, &root).map_err(ErrorObject::internal)? {
            return Ok(self
                .store()
                .project(&existing)
                .map_err(ErrorObject::internal)?
                .unwrap_or_else(|| json!({ "projectId": existing, "hostId": host_id, "root": root })));
        }

        self.store()
            .ensure_host(&host_id, if host_id == "local" { "Local" } else { host_id.as_str() }, "local", "connected")
            .map_err(ErrorObject::internal)?;

        let id = self.store().next_project_id().map_err(ErrorObject::internal)?;
        let name = envelope
            .opt_str("name")
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| folder_name(&root));

        self.store().add_project(&id, &host_id, &root, &name).map_err(ErrorObject::internal)?;

        Ok(json!({ "projectId": id, "hostId": host_id, "root": root, "name": name }))
    }

    /// `project.scaffold` - a project from nothing (0.9.0).
    ///
    /// "Scratch thake kaj suru korbe tokhon sudu command dilai jano kora jai": starting from an empty
    /// folder was four surfaces (a file manager to make the folder, Add project, a new chat, the
    /// prompt). This is the daemon half of making it one step: the folder is **created** - on this
    /// machine or on a host - and added as a project in the same call. The window then opens a chat on
    /// it and, when the person typed what to build, starts the agent turn.
    ///
    /// `name` becomes the folder, so it must be a plain name: separators and `..` are refused rather
    /// than resolved, because "make me a folder" must never mean "write outside `parent`".
    pub(super) fn project_scaffold(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let host_id = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());
        let parent = envelope.require_str("parent")?.trim().to_string();
        let name = envelope.require_str("name")?.trim().to_string();

        if parent.is_empty() {
            return Err(ErrorObject::bad_request("`parent` is required: where should the folder be made?"));
        }

        if name.is_empty()
            || name == ".."
            || name == "."
            || name.contains(['/', '\\'])
            || name.contains("..")
        {
            return Err(ErrorObject::bad_request(format!(
                "`{name}` cannot be a folder name here: one plain name, no separators"
            )));
        }

        let remote = self.ssh_for(&host_id)?;
        /* One separator for both machines: Windows' own APIs take `/`, and a root spelled
           `C:/parent\name` (what `Path::join` writes after a forward-slash parent) reads as broken. */
        let root = format!("{}/{name}", parent.trim_end_matches(['/', '\\']));

        match &remote {
            Some(ssh) => {
                if !crate::ssh::ops::is_dir(ssh, &parent)? {
                    return Err(ErrorObject::bad_request(format!(
                        "`{parent}` is not a folder on {}",
                        ssh.label()
                    )));
                }

                crate::ssh::ops::mkdir(ssh, &root)?;
            }
            None => {
                if !std::path::Path::new(&parent).is_dir() {
                    return Err(ErrorObject::bad_request(format!("`{parent}` is not a folder")));
                }

                crate::fs::mkdir(std::path::Path::new(&root))?;
            }
        }

        /* The same row `project.add` writes - including the "same folder twice is one project" rule,
           which `mkdir`'s idempotence extends to the scaffold: running it twice is one folder, one row. */
        if let Some(existing) = self.store().project_at(&host_id, &root).map_err(ErrorObject::internal)? {
            return Ok(self
                .store()
                .project(&existing)
                .map_err(ErrorObject::internal)?
                .unwrap_or_else(|| json!({ "projectId": existing, "hostId": host_id, "root": root })));
        }

        self.store()
            .ensure_host(&host_id, if host_id == "local" { "Local" } else { host_id.as_str() }, "local", "connected")
            .map_err(ErrorObject::internal)?;

        let id = self.store().next_project_id().map_err(ErrorObject::internal)?;

        self.store().add_project(&id, &host_id, &root, &name).map_err(ErrorObject::internal)?;

        Ok(json!({ "projectId": id, "hostId": host_id, "root": root, "name": name, "created": true }))
    }

    /// `project.remove`: closes the folder. Its chats are **unbound**, not deleted - a chat is a
    /// conversation, and a folder is a place to have it. The answer says how many chats were unbound.
    pub(super) fn project_remove(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let project_id = envelope.require_str("projectId")?;
        let project = self.store().project(&project_id).map_err(ErrorObject::internal)?;

        if project.is_none() {
            return Err(ErrorObject::not_found(format!("no project `{project_id}`")));
        }

        let name = project
            .as_ref()
            .and_then(|project| project.get("name").and_then(Value::as_str))
            .unwrap_or("the folder")
            .to_string();
        let chats = self.store().remove_project(&project_id).map_err(ErrorObject::internal)?;

        if chats > 0 {
            out.push(
                event::toast(
                    &format!("Closed {name} · {chats} chat{} kept their conversation", if chats == 1 { "" } else { "s" }),
                    None,
                    None,
                ),
                None,
                None,
            );
        }

        Ok(json!({ "removed": true, "chats": chats }))
    }

    pub(super) fn project_root(&self, envelope: &Envelope) -> Option<std::path::PathBuf> {
        envelope
            .opt_str("root")
            .or_else(|| envelope.opt_str("projectRoot"))
            .map(std::path::PathBuf::from)
    }

    /// The root a file or git method works in: the one the envelope names, or the one the **session** has.
    ///
    /// The envelope stays first, because a caller that names a directory means that directory. The session
    /// is the fallback 0.7.6 added, and it is the one that matters in practice: the app knows a chat's id,
    /// the daemon knows which folder that chat was pointed at, and a tool that had to be told both would
    /// be asking the caller to repeat a fact the daemon already holds.
    pub(super) fn root_for(&self, envelope: &Envelope) -> Result<Option<std::path::PathBuf>, ErrorObject> {
        if let Some(root) = self.project_root(envelope) {
            return Ok(Some(root));
        }

        let Some(session_id) = envelope.opt_str("sessionId") else {
            return Ok(None);
        };

        Ok(self
            .store()
            .session_project_root(&session_id)
            .map_err(ErrorObject::internal)?
            .map(std::path::PathBuf::from))
    }

    /// The root these methods need, or the sentence that says what to send instead.
    pub(super) fn root_required(&self, envelope: &Envelope) -> Result<std::path::PathBuf, ErrorObject> {
        self.root_for(envelope)?.ok_or_else(|| {
            ErrorObject::bad_request("`root` is required, or a `sessionId` whose chat has a folder")
        })
    }

    /// `project.locate` (0.11.0): where a named thing - a domain, a project - lives on a machine.
    ///
    /// This is the daemon half of *"ami skilleddesk.com er project file access chai"*: the window
    /// routes the prompt to the host, and this answers **which folder** on it holds the site, so the
    /// chat can be bound there without anyone browsing for it. On a host the web server's config is
    /// read first (`ssh::ops::locate_project`); locally the usual code folders are checked.
    pub(super) fn project_locate(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let query = envelope.require_str("query")?.trim().to_string();

        if query.is_empty() {
            return Err(ErrorObject::bad_request("`query` is empty: a domain or a folder name finds a project"));
        }

        let host_id = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());
        let candidates = match self.ssh_for(&host_id)? {
            Some(ssh) => crate::ssh::ops::locate_project(&ssh, &query)?,
            None => locate_local(&query),
        };

        Ok(json!({ "candidates": candidates }))
    }

    /// `event.append`: a client records an event of its own - the log is the state, so a UI-only fact
    /// that matters (a toast the user acted on) belongs in it (spec section 3.3). Every subscriber
    /// sees it, exactly like an event the daemon raised.
    pub(super) fn event_append(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let event = envelope
            .params
            .get("event")
            .filter(|event| event.is_object())
            .cloned()
            .ok_or_else(|| ErrorObject::bad_request("`event` must be an object with a `type`"))?;

        if event.get("type").and_then(Value::as_str).is_none() {
            return Err(ErrorObject::bad_request("`event.type` is required"));
        }

        let seq = out.push(
            event,
            envelope.opt_str("sessionId"),
            envelope.opt_str("turnId"),
        );

        Ok(json!({ "seq": seq.unwrap_or_else(|| self.state.events.seq()) }))
    }

    /// `event.list`: the replay a reconnecting client asks for (spec section 5.4).
    pub(super) fn event_list(&self, envelope: &Envelope) -> Value {
        let since = envelope.opt_i64("since").unwrap_or(0);

        json!({
            "events": self
                .state
                .events
                .since(since)
                .into_iter()
                .map(|entry| json!({
                    "seq": entry.seq,
                    "ts": entry.ts,
                    "sessionId": entry.session_id,
                    "turnId": entry.turn_id,
                    "event": entry.event,
                }))
                .collect::<Vec<Value>>()
        })
    }
}
