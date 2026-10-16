//! The agent fleet and automations - `runner.*`, `agents.*`, `automation.*`, `turn.*`, `inbox.*`. Split out of `methods.rs` (P0-3).
//!
//! Nothing here changed: the arms and the bodies are the ones that used to sit in the big `impl Daemon`, and
//! the private helpers they lean on are still in `methods.rs`, which a child module can reach. The `_` arm
//! passes the method on to the next link in the chain, exactly as the single `_` arm used to.

use super::*;

impl Daemon {
    /// The methods this module owns. Moved from `methods.rs` unchanged.
    pub(super) fn dispatch_fleet(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            /* v3 R4/PAR/AUTO: the runner this project's turns go to, the agents on the board, and the Inbox. */
            "runner.list" => self.runner_list(envelope),
            "agents.board" => self.agents_board(envelope),
            "agents.start" => self.agents_start(envelope, out.clone()),
            "agents.stop" => self.agents_stop(envelope, out.clone()),
            "inbox.list" => self.inbox_list(envelope),
            "automation.run" => self.automation_run(envelope, out.clone()),
            _ => self.dispatch_kernel(envelope, out),
        }
    }

    /// `host.status` - the acceptance item that names it. It also *pushes* the same fact as an event,
    /// because the app's host list is folded from `HostStatus`, never read from a result.
    /// `runner.list` (R4): the hosts this project's turns can run on, and which one is chosen for them.
    pub(super) fn runner_list(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        /* `root_for`, not `project_root`: the app knows a chat's id and the daemon knows its folder, and the
           schema declares `sessionId` for these three - `project_root` reads only `root`/`projectRoot`, so a
           caller that sent what the contract asks for was answered "needs a project whose folder I can see". */
        let Some(root) = self.root_for(envelope)? else {
            return Err(ErrorObject::new("invalid_request", "runner.list needs a project whose folder I can see"));
        };
        let named = crate::v3::runner::load(&root).map_err(|error| ErrorObject::new("runner_unreadable", error))?;
        let chosen = crate::v3::runner::chosen(&root, crate::v3::runner::default_name().as_deref()).map_err(|error| ErrorObject::new("runner_unreadable", error))?;

        Ok(json!({
            "runners": named.iter().map(|runner| json!({ "name": runner.name, "host": runner.host, "dir": runner.dir })).collect::<Vec<_>>(),
            "chosen": chosen.map(|runner| runner.name),
            "setting": crate::v3::runner::default_name(),
        }))
    }

    /// `agents.board` (PAR): the agents working now and those that finished, newest first.
    pub(super) fn agents_board(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        /* The session's folder when the envelope names none - see `runner_list`. */
        let Some(root) = self.root_for(envelope)? else {
            return Err(ErrorObject::new("invalid_request", "agents.board needs a project whose folder I can see"));
        };

        Ok(json!({
            "agents": crate::v3::parallel::board(&root).iter().map(|agent| json!({
                "id": agent.id,
                "request": agent.request,
                "worktree": agent.worktree,
                "state": match agent.state {
                    crate::v3::parallel::State::Running => "running",
                    crate::v3::parallel::State::Done => "done",
                    crate::v3::parallel::State::Failed => "failed",
                    crate::v3::parallel::State::Stopped => "stopped",
                },
                "note": agent.note,
                "startedAt": agent.started_at,
            })).collect::<Vec<_>>(),
        }))
    }

    /// `agents.start` (PAR): put an agent on the board - its own worktree of this project's repository, a chat
    /// in it, and a turn running the request there. The card is what the window reads; the turn is what makes
    /// it more than a folder.
    pub(super) fn agents_start(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        if !crate::v3::on("parallel") {
            return Err(ErrorObject::new("parallel_off", "Parallel agents are switched off in Settings"));
        }

        let Some(root) = self.root_for(envelope)? else {
            return Err(ErrorObject::new("invalid_request", "agents.start needs a project whose folder I can see"));
        };
        let request = envelope.require_str("request")?;

        if request.trim().is_empty() {
            return Err(ErrorObject::new("invalid_request", "agents.start needs something to work on"));
        }

        /* A cap, because each agent is a whole turn of its own: ten cards is ten model conversations at once,
           ten worktrees, and a machine that stops answering. Three by default, and the person's own number in
           `parallel.max` when three is wrong for their machine. */
        let most = self
            .store()
            .setting("parallel.max")
            .ok()
            .flatten()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(3);
        let running = crate::v3::parallel::board(&root)
            .iter()
            .filter(|card| card.state == crate::v3::parallel::State::Running)
            .count();

        if running >= most {
            return Err(ErrorObject::new(
                "too_many_agents",
                format!("{running} agents are already running - stop one, or raise `parallel.max` past {most}"),
            ));
        }

        let now = chrono::Utc::now().timestamp();
        let id = crate::v3::parallel::id_for(&root, &request, now);
        let agent = crate::v3::parallel::start(&root, &request, &id, now).map_err(|error| ErrorObject::new("agent_not_started", error))?;
        /* Where the agent works - see `provision_workspace` for the same three steps: a project row for the
           folder, then a chat in it. Without them `engine.start` has nowhere to look, and a worktree the
           daemon cannot reach is a card that never moves. */
        let host_id = self
            .store()
            .session(&envelope.opt_str("sessionId").unwrap_or_default())
            .ok()
            .flatten()
            .and_then(|session| session["hostId"].as_str().map(str::to_string))
            .unwrap_or_else(|| "local".to_string());
        let worktree = agent.worktree.replace('\\', "/");
        /* The host row has to exist before a project can point at it: the app's `host.status` writes the
           local one, and the daemon's own clock has no window to have asked - a foreign key that is missing
           turns into "agent_not_started" for a reason with nothing to do with the agent. */
        self.store()
            .ensure_host(&host_id, if host_id == "local" { "Local" } else { &host_id }, "local", "connected")
            .map_err(ErrorObject::internal)?;
        let project_id = match self.store().project_at(&host_id, &worktree).map_err(ErrorObject::internal)? {
            Some(existing) => existing,
            None => {
                let project = self.store().next_project_id().map_err(ErrorObject::internal)?;

                self.store().add_project(&project, &host_id, &worktree, &id).map_err(ErrorObject::internal)?;

                project
            }
        };
        let session_id = self.state.fresh_id("agent-");

        self.store()
            .insert_session(&session_id, &host_id, &format!("⚡ {id}"), &request, Some(&project_id))
            .map_err(ErrorObject::internal)?;

        let turn = self.engine_start(&self.turn_envelope("agents", &session_id, &request, envelope), out.clone())?;
        let turn_id = turn["turnId"].as_str().unwrap_or_default().to_string();

        /* The card remembers the turn, so Stop can stop the work and not just the label. */
        let mut card = agent.clone();
        card.session_id = Some(session_id.clone());
        card.turn_id = Some(turn_id.clone());

        crate::v3::parallel::save(&root, &card).map_err(|error| ErrorObject::new("agent_not_saved", error))?;

        let watching = root.clone();
        let watching_id = id.clone();

        crate::v3::run::watch(self.state.clone(), turn_id, move |ended| {
            let (state, note) = match ended.error {
                Some(error) => (crate::v3::parallel::State::Failed, error),
                None => (crate::v3::parallel::State::Done, ended.summary),
            };

            if let Err(error) = crate::v3::parallel::finish(&watching, &watching_id, state, note.trim()) {
                eprintln!("agent `{watching_id}` was not written down: {error}");
            }
        });

        Ok(json!({
            "agent": {
                "id": card.id,
                "request": card.request,
                "worktree": card.worktree,
                "state": "running",
                "note": card.note,
                "startedAt": card.started_at,
                "sessionId": card.session_id,
            },
            "sessionId": session_id,
        }))
    }

    /// `agents.stop` (PAR): stop one agent - its turn and its card. Its worktree is left where it is: a person's
    /// work is never deleted by a button (take it off the board with `agents.forget` when it is read).
    pub(super) fn agents_stop(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let Some(root) = self.root_for(envelope)? else {
            return Err(ErrorObject::new("invalid_request", "agents.stop needs a project whose folder I can see"));
        };
        let id = envelope.require_str("id")?;
        let agent = crate::v3::parallel::load(&root, &id)
            .ok_or_else(|| ErrorObject::not_found(format!("no agent `{id}` on the board")))?;

        if agent.state != crate::v3::parallel::State::Running {
            return Ok(json!({ "stopped": false, "state": agent.state, "note": format!("`{id}` is not running") }));
        }

        /* The turn first, so the model stops being asked for more, then the card. */
        if let Some(turn_id) = &agent.turn_id {
            let stop = Envelope {
                v: crate::SDCP_VERSION.to_string(),
                id: format!("stop-{turn_id}"),
                method: "engine.kill".to_string(),
                params: serde_json::Map::from_iter([("turnId".to_string(), json!(turn_id))]),
                host_id: None,
            };

            self.engine_stop(&stop, out)?;
        }

        let card = crate::v3::parallel::finish(&root, &id, crate::v3::parallel::State::Stopped, "Stopped by hand")
            .map_err(|error| ErrorObject::new("agent_not_saved", error))?;

        Ok(json!({ "stopped": true, "state": card.state, "note": card.note }))
    }

    /// `automation.run` (AUTO): run one automation of this project now, due or not.
    pub(super) fn automation_run(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let Some(root) = self.root_for(envelope)? else {
            return Err(ErrorObject::new("invalid_request", "automation.run needs a project whose folder I can see"));
        };

        self.run_automation(&root, &envelope.require_str("name")?, out)
    }

    /// Run one automation of `root`: a chat of its own, the turn, and a note in the Inbox when it ends.
    ///
    /// Public because the daemon's clock runs it too ([`crate::v3::run`]): the Inbox's Run button and a run the
    /// clock decided on have to be the same run, or the two drift into different behaviour.
    pub fn run_automation(
        &self,
        root: &std::path::Path,
        name: &str,
        out: Arc<dyn Notifier>,
    ) -> Result<Value, ErrorObject> {
        if !crate::v3::on("automations") {
            return Err(ErrorObject::new("automations_off", "Automations are switched off in Settings"));
        }

        let written = crate::v3::automations::load(root).map_err(|error| ErrorObject::new("automations_unreadable", error))?;
        let automation = written
            .into_iter()
            .find(|item| item.name == name)
            .ok_or_else(|| ErrorObject::not_found(format!("`{name}` is not an automation of {}", root.display())))?;

        if !automation.enabled {
            return Err(ErrorObject::new("automation_disabled", format!("`{name}` is switched off, so nothing ran")));
        }

        let now = chrono::Utc::now().timestamp();
        let host_id = "local".to_string();
        let title = format!("⏱ {name}");
        /* One chat per automation, reused: an automation that runs every morning should not leave a chat per
           morning behind. Folded from the store rather than searched for, because the row it needs is the
           projectId, which only the store has. */
        let project_id = {
            let path = root.to_string_lossy().replace('\\', "/");

            /* See `agents_start`: the row a project hangs off is written by the window, not by the clock. */
            self.store().ensure_host(&host_id, "Local", "local", "connected").map_err(ErrorObject::internal)?;

            match self.store().project_at(&host_id, &path).map_err(ErrorObject::internal)? {
                Some(existing) => existing,
                None => {
                    let project = self.store().next_project_id().map_err(ErrorObject::internal)?;

                    self.store().add_project(&project, &host_id, &path, name).map_err(ErrorObject::internal)?;

                    project
                }
            }
        };
        let existing = self
            .store()
            .sessions_for_host(&host_id)
            .map_err(ErrorObject::internal)?
            .into_iter()
            .find(|session| {
                session["title"].as_str() == Some(title.as_str()) && session["projectId"].as_str() == Some(project_id.as_str())
            })
            .and_then(|session| session["id"].as_str().map(str::to_string));
        let session_id = match existing {
            Some(session_id) => session_id,
            None => {
                let session_id = self.state.fresh_id("auto-");

                self.store()
                    .insert_session(&session_id, &host_id, &title, &automation.request, Some(&project_id))
                    .map_err(ErrorObject::internal)?;

                session_id
            }
        };

        let turn = self.engine_start(&self.automation_envelope(&session_id, &automation), out.clone())?;
        let turn_id = turn["turnId"].as_str().unwrap_or_default().to_string();

        crate::v3::automations::mark(root, name, now).map_err(|error| ErrorObject::new("automation_not_marked", error))?;

        let watching = root.to_path_buf();
        let watched = automation.clone();

        crate::v3::run::watch(self.state.clone(), turn_id.clone(), move |ended| {
            let summary = match ended.error {
                Some(error) => format!("Did not finish: {error}. {}\n", ended.summary),
                None => ended.summary,
            };

            if let Err(error) = crate::v3::automations::record(&watching, &watched, now, &summary) {
                eprintln!("the Inbox note for `{}` was not written: {error}", watched.name);
            }
        });

        Ok(json!({ "name": automation.name, "sessionId": session_id, "turnId": turn_id, "at": now }))
    }

    /// The envelope an automation's turn is sent in: the person's own engine and model when they have said
    /// which (`automation.engine`, `automation.model`, `automation.provider`), otherwise the agent default -
    /// the same defaults a chat opened in that project would get.
    pub(super) fn automation_envelope(&self, session_id: &str, automation: &crate::v3::automations::Automation) -> Envelope {
        let engine = self
            .store()
            .setting("automation.engine")
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "claude_code".to_string());
        let mut params = serde_json::Map::from_iter([
            ("sessionId".to_string(), json!(session_id)),
            ("prompt".to_string(), json!(automation.request)),
            ("engine".to_string(), json!(engine)),
            ("agent".to_string(), json!(true)),
        ]);

        for (param, key) in [("model", "automation.model"), ("provider", "automation.provider")] {
            if let Some(value) = self.store().setting(key).ok().flatten().filter(|value| !value.trim().is_empty()) {
                params.insert(param.to_string(), json!(value));
            }
        }

        Envelope {
            v: crate::SDCP_VERSION.to_string(),
            id: format!("{}-{}", automation.name, chrono::Utc::now().timestamp_millis()),
            method: "engine.start".to_string(),
            params,
            host_id: None,
        }
    }

    /// The envelope an agent's turn is sent in - the engine and model the person has connected to this chat.
    pub(super) fn turn_envelope(&self, prefix: &str, session_id: &str, prompt: &str, from: &Envelope) -> Envelope {
        let mut params = serde_json::Map::from_iter([
            ("sessionId".to_string(), json!(session_id)),
            ("prompt".to_string(), json!(prompt)),
            ("agent".to_string(), json!(true)),
        ]);

        for key in ["engine", "model", "provider", "tier"] {
            if let Some(value) = from.opt_str(key).filter(|value| !value.trim().is_empty()) {
                params.insert(key.to_string(), json!(value));
            }
        }

        Envelope {
            v: crate::SDCP_VERSION.to_string(),
            id: self.state.fresh_id(prefix),
            method: "engine.start".to_string(),
            params,
            host_id: from.host_id.clone(),
        }
    }

    /// `inbox.list` (AUTO): what the automations produced, and when each one last ran.
    pub(super) fn inbox_list(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        /* The session's folder when the envelope names none - see `runner_list`. */
        let Some(root) = self.root_for(envelope)? else {
            return Err(ErrorObject::new("invalid_request", "inbox.list needs a project whose folder I can see"));
        };
        let written = crate::v3::automations::load(&root).map_err(|error| ErrorObject::new("automations_unreadable", error))?;
        let last = crate::v3::automations::last_runs(&root);

        Ok(json!({
            "automations": written.iter().map(|item| json!({
                "name": item.name,
                "request": item.request,
                "every": item.every,
                "at": item.at,
                "enabled": item.enabled,
                "lastRun": last.get(&item.name),
            })).collect::<Vec<_>>(),
            "notes": crate::v3::automations::notes(&root).iter().map(|note| json!({
                "name": note.name,
                "at": note.at,
                "request": note.request,
                "summary": note.summary,
                "file": note.file,
            })).collect::<Vec<_>>(),
        }))
    }

    /// The model a turn runs on, given the region the key's endpoint is in (1.0): the same model's regional copy
    /// is used and said in a toast; a nearby regional model is only suggested, once per model per run.
    pub(super) fn region_model(&self, model: String, provider_id: &str, out: &dyn Notifier) -> String {
        static SUGGESTED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();

        let bare = crate::engines::native_api::api_model(&model).to_string();
        let url = crate::engines::native_api::endpoint_for(&model, Some(provider_id)).url;
        let offered: Vec<String> = self
            .store()
            .cached_models(provider_id)
            .unwrap_or_default()
            .iter()
            .filter_map(|row| row["id"].as_str().map(str::to_string))
            .collect();

        match crate::providers::models::region_twin(&url, &bare, &offered) {
            Some(crate::providers::models::Twin::Same(twin)) => {
                out.push(event::toast(&format!("Using {twin}: the same model, served in your key's region - each step starts in about a second instead of 10-20."), None, None), None, None);

                twin
            }
            Some(crate::providers::models::Twin::Near(near)) => {
                let first = SUGGESTED.get_or_init(Default::default).lock().map(|mut seen| seen.insert(bare.clone())).unwrap_or(false);

                if first {
                    out.push(
                        event::toast(&format!("{bare} is served outside your key's region, so every step waits 10-20 s before it starts. {near} runs in your region and starts in about a second - pick it in the model menu for speed."), None, Some(15_000)),
                        None,
                        None,
                    );
                }

                model
            }
            None => model,
        }
    }
}
