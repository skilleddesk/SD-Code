//! The request handlers - one arm per SDCP method (master spec section 5.2).
//!
//! A handler validates its params, appends the events the request implies, and answers with a
//! `Response`. It never mutates state directly: the event log *is* the state change, which is the
//! daemon's half of spec section 3.3.
//!
//! `engine.start` is the interesting one. It answers with a `turnId` immediately and then, in a task
//! of its own, runs the engine adapter and pushes the whole turn through the notifier -
//! `TurnStarted`, `ThinkingDelta`, the tool calls, a `CheckpointSaved` *before* the mutating tool
//! (principle P5) and the `TurnDelta` stream, **each event as the engine produced it**. That is the
//! acceptance item "a fake `engine.start` streams a few TurnDelta events to the UI store", and it is
//! also how the real CLIs behave: the call returns, the answer arrives later.
//!
//! Every module of the daemon is reached from here, which is the point of the file - it is the one
//! place where a protocol method is mapped onto a capability.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::duel;
use crate::engines::{EngineStatus, EventSink, Prompt};
use crate::errors::translator;
use crate::host;
use crate::providers;
use crate::sdcp::envelope::{Envelope, ErrorObject, Response};
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;
use crate::session_bridge;
use crate::store::sqlite::Store;
use crate::{DaemonState, SDCP_VERSION, VERSION};

/// One connection's request handler. It holds the shared daemon state and nothing else, so a second
/// connection is a second `Daemon` over the same state rather than a second source of truth.
pub struct Daemon {
    state: Arc<DaemonState>,
}

impl Daemon {
    /// Takes the shared state by value: the handler is cheap to clone and a turn keeps one.
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    /// Dispatches one envelope. The `Notifier` is where every event the request implies goes; it is
    /// behind an `Arc` because a turn outlives the call that started it.
    pub fn handle(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Response {
        match self.dispatch(envelope, out) {
            Ok(result) => Response::ok(envelope.id.clone(), result),
            Err(error) => Response::fail(envelope.id.clone(), error),
        }
    }

    fn store(&self) -> &Arc<Store> {
        &self.state.store
    }

    fn dispatch(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            /* Host ---------------------------------------------------------------------------- */
            "host.status" => Ok(self.host_status(&*out)),
            "host.doctor" => Ok(json!({ "checks": host::checks(self.store()) })),
            "host.add" => self.host_add(envelope, out),
            "host.remove" => self.host_remove(envelope, &*out),
            "host.shutdown" => Ok(self.host_shutdown()),

            /* Sessions ------------------------------------------------------------------------ */
            "session.open" => self.session_open(envelope, &*out),
            "session.update" => self.session_update(envelope, &*out),
            "session.fork" => self.session_fork(envelope, &*out),
            "session.close" => self.session_close(envelope, &*out),
            "session.list" => Ok(json!({ "hosts": self.store().hosts_with_sessions().map_err(ErrorObject::internal)? })),

            /* The folder a chat works in (0.7.6). `add` and `list` are answers; `remove` also pushes a
               toast, because closing a folder that had chats in it is a thing the person should see. */
            "project.add" => self.project_add(envelope),
            "project.list" => Ok(json!({ "projects": self.store().projects().map_err(ErrorObject::internal)? })),
            "project.remove" => self.project_remove(envelope, &*out),

            /* Engines ------------------------------------------------------------------------- */
            "engine.start" => self.engine_start(envelope, out),
            "engine.cancel" | "engine.kill" => self.engine_stop(envelope, &*out),
            "engine.status" => self.engine_status(envelope),
            "engine.switch" => self.engine_switch(envelope, &*out),

            /* The engines' environment ------------------------------------------------------- */
            "fs.read" => self.fs_read(envelope),
            "fs.write" => self.fs_write(envelope),
            "fs.list" => self.fs_list(envelope),
            "fs.stat" => self.fs_stat(envelope),
            "fs.search" => self.fs_search(envelope),
            "git.status" => self.git_status(envelope),
            "git.diff" => self.git_diff(envelope),
            "git.checkpoint" => self.git_checkpoint(envelope, &*out),
            "git.worktree" => self.git_worktree(envelope),
            "pty.open" => self.pty_open(envelope),
            "pty.write" => self.pty_write(envelope),
            "pty.resize" => Ok(json!({})),
            "pty.close" => self.pty_close(envelope),
            "pty.output" => self.pty_output(envelope),
            "shell.run" => self.shell_run(envelope, &*out),

            /* Signing a CLI in from the app: the daemon drives the CLI's own login and never sees the
               credential (spec sections 9.10, 15.1). */
            "cli.login" => self.cli_login_start(envelope, &*out),
            "cli.login.status" => self.cli_login_status(envelope, &*out),
            "cli.login.code" => self.cli_login_code(envelope, &*out),
            "cli.login.cancel" => self.cli_login_cancel(envelope),
            "cli.recipes" => Ok(json!({
                "recipes": crate::auth::cli_login::RECIPES
                    .iter()
                    .map(|recipe| json!({
                        "providerId": recipe.provider_id,
                        "label": recipe.label,
                        "program": recipe.program,
                        "note": recipe.note,
                        "installed": crate::host::doctor::has(recipe.program),
                    }))
                    .collect::<Vec<_>>()
            })),

            /* The model catalogue: live, cached or bundled, and never needing a code change. */
            "models.list" => self.models_list(envelope, &*out),
            "models.select" => self.models_select(envelope, &*out),

            /* Providers ------------------------------------------------------------------------ */
            "provider.list" => Ok(json!({ "providers": providers::list(self.store()) })),
            "provider.test" => Ok(providers::test(
                &envelope.require_str("id")?,
                envelope.opt_str("key").as_deref(),
            )),
            "provider.save" => self.provider_save(envelope, &*out),
            /* Provider, checkpoint and rewind the session inherits, then its turns. */
            "provider.remove" => self.provider_remove(envelope, &*out),
            "provider.oauth.open" => Ok(providers::oauth_open(&envelope.require_str("id")?)),
            "provider.oauth.callback" => self.provider_oauth_callback(envelope),
            "provider.local.doctor" => Ok(providers::local_doctor()),
            "provider.registry.list" => Ok(json!({ "models": providers::registry(&[]) })),
            "provider.registry.set" => self.provider_registry_set(envelope, &*out),

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

            /* Duel, console, events ----------------------------------------------------------- */
            "duel.start" => self.duel_start(envelope, &*out),
            "duel.keep" | "duel.discard" => self.duel_keep(envelope, &*out),
            "console.attach" => self.console_attach(envelope),
            "console.detach" => self.console_detach(envelope),
            "event.list" => Ok(self.event_list(envelope)),
            "event.subscribe" => Ok(json!({ "fromSeq": self.state.events.seq() + 1 })),
            "event.append" => self.event_append(envelope, &*out),

            other => Err(ErrorObject::unsupported(other)),
        }
    }

    /// `host.status` - the acceptance item that names it. It also *pushes* the same fact as an event,
    /// because the app's host list is folded from `HostStatus`, never read from a result.
    fn host_status(&self, out: &dyn Notifier) -> Value {
        let platform = format!("{} · {}", std::env::consts::OS, std::env::consts::ARCH);

        /* The event is the app's view; this is the row. `session.list` answers from the rows, so a
           window that asks for the list right after this call has to find this machine in it - and
           a window that asks *without* this call (a second one, a reload) already will. */
        let _ = self.store().ensure_host("local", "Local", "local", "connected");

        out.push(event::host_status("local", "Local", "local", "connected", Some(&platform)), None, None);

        json!({
            "hostId": "local",
            "name": "Local",
            "type": "local",
            "status": "connected",
            "sdcd": VERSION,
            "sdcp": SDCP_VERSION,
            "platform": platform,
            "database": self.store().path(),
            "events": self.store().event_count().unwrap_or(0),
            "sessions": self.store().session_count().unwrap_or(0),
            "keychain": crate::auth::keychain::backend(),
            "engines": self.state.engines.ids(),
            "pty": self.state.pty.running(),
            "console": self.state.console.attached_count(),
            "subscribers": self.state.fanout.listeners(),
        })
    }

    /// `host.shutdown` - asks this daemon to stop after the current request.
    ///
    /// It exists because the app is the daemon's owner and needs the code to say so over the wire:
    /// when the daemon answering on the port is not the version the app ships, the app stops it and
    /// starts its own, so installing a new build never means restarting a machine by hand. The run
    /// loop polls the flag this sets (`main.rs#watch`), so the answer goes out before the exit.
    fn host_shutdown(&self) -> Value {
        self.state.request_stop();

        json!({
            "stopping": true,
            "sdcd": VERSION,
            "clients": self.state.clients(),
        })
    }

    fn host_add(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        if envelope.opt_str("type").unwrap_or_else(|| "ssh".into()) == "local" {
            return Ok(json!({ "hostId": "local" }));
        }

        let raw_target = envelope.opt_str("target").unwrap_or_default().trim().to_string();

        if raw_target.is_empty() {
            return Err(ErrorObject::bad_request("`target` is required for an SSH host"));
        }

        /*
         * What the user typed is parsed, not trusted - and that is a fix, not tidiness.
         *
         * The report pasted `ssh -p 8443 mehedi105117@109.199.108.216`, which is exactly what a person
         * types into their own shell. The daemon used that whole string as a hostname: `ssh` was asked
         * for a machine called `ssh`, and the port was never used, so a VPS that answers on 8443 could
         * not be reached however correct the key was. `parse_target` pulls the address and the port out,
         * and the port travels with every `ssh` call this daemon makes for that host.
         */
        let parsed = crate::auth::remote::parse_target(&raw_target).map_err(ErrorObject::bad_request)?;
        let target = parsed.user_host.clone();
        /* Used for this one install and dropped. It is never stored, never logged, and never part of a
           sentence the UI shows. */
        let password = envelope.opt_str("password").unwrap_or_default().trim().to_string();

        let label = envelope
            .opt_str("label")
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| target.clone());

        /*
         * The same machine added twice is one host.
         *
         * A sidebar of `Website, Website, Website` is what the alternative looks like in practice:
         * a second row for the same `user@host` says nothing the first one did not, and it can never
         * be told apart from it afterwards. The row's id comes back with `reused: true`, so a caller
         * can say "already there" instead of pretending it just connected.
         */
        if let Some(existing) = self.store().host_id_for_target(&target).map_err(ErrorObject::internal)? {
            let row = self.store().host(&existing).map_err(ErrorObject::internal)?;
            let name = row
                .as_ref()
                .and_then(|row| row["name"].as_str())
                .unwrap_or(&label)
                .to_string();
            let status = row
                .as_ref()
                .and_then(|row| row["status"].as_str())
                .unwrap_or("connecting")
                .to_string();

            out.push(event::host_status(&existing, &name, "vps", &status, Some(&target)), None, None);

            return Ok(json!({ "hostId": existing, "reused": true }));
        }

        let host_id = format!("h{}", self.state.events.seq() + 1);

        out.push(event::host_status(&host_id, &label, "vps", "connecting", Some("linux · x64")), None, None);
        self.store()
            .upsert_host(&host_id, &label, "ssh", Some(&target), "connecting", None)
            .map_err(ErrorObject::internal)?;

        /*
         * The probe runs *after* the answer, which is the `engine.start` shape: the dialog closes
         * with a host on screen, and the sentence about whether that host can be reached arrives
         * when there is something to attach it to.
         *
         * `connecting` is therefore not a placeholder for a status that never comes. Every added
         * host gets a second `HostStatus` - `connected` when `ssh` answered, `offline` when it did
         * not - and a `Toast` beside it, because a 7px dot is not a notification.
         */
        let state = self.state.clone();
        let notifier = out.clone();
        let probe_host_id = host_id.clone();
        let probe_label = label.clone();
        let probe_target = parsed.clone();

        tokio::spawn(async move {
            /*
             * The password, when one was given, is spent here - before the probe - because the probe is
             * `ssh` with `BatchMode=yes`, which by design cannot answer a prompt. One install is enough
             * for every connection after it: the key is in `authorized_keys` and the password is gone.
             */
            if !password.is_empty() {
                notifier.push(
                    event::host_status(
                        &probe_host_id,
                        &probe_label,
                        "vps",
                        "connecting",
                        Some("copying SDC's key with that password…"),
                    ),
                    None,
                    None,
                );

                let pty = state.pty.clone();
                let install_target = probe_target.clone();

                let installed = tokio::task::spawn_blocking(move || {
                    crate::auth::remote::install_key(&pty, &install_target, &password)
                })
                .await
                .unwrap_or_else(|_| Err(ErrorObject::internal("the key install could not be run")));

                match installed {
                    Ok(sentence) => {
                        /* The sentence goes on the *status* line rather than into a `Toast`.
                           A toast is written to the event log and replayed on the next launch, so a
                           four-line explanation became four lines of furniture over the Provider Hub
                           every time the app started. The card shows the detail itself, which is where
                           a fact about a host belongs. */
                        notifier.push(
                            event::host_status(&probe_host_id, &probe_label, "vps", "connecting", Some(&sentence)),
                            None,
                            None,
                        );
                    }
                    Err(error) => {
                        /* The install is the whole reason the password was asked for, so its failure is
                           the host's status - and the sentence is the actionable one. */
                        let _ = state
                            .store
                            .upsert_host(&probe_host_id, &probe_label, "ssh", Some(&probe_target.user_host), "offline", None);

                        notifier.push(
                            event::host_status(&probe_host_id, &probe_label, "vps", "offline", Some(&error.message)),
                            None,
                            None,
                        );

                        return;
                    }
                }
            }

            let (status, detail) = tokio::task::spawn_blocking(move || probe_ssh(&probe_target))
                .await
                .unwrap_or_else(|_| ("offline".to_string(), unreachable_sentence(&target)));

            let _ = state.store.upsert_host(&probe_host_id, &probe_label, "ssh", Some(&target), &status, None);

            /* One event, not two. The `HostStatus` carries the sentence and the card renders it; the
               `Toast` that used to accompany it was written to the log as well, so a machine that could
               not be reached produced the same four-line paragraph again on every launch. A fact about a
               host belongs on the host's row. */
            notifier.push(
                event::host_status(&probe_host_id, &probe_label, "vps", &status, Some(&detail)),
                None,
                None,
            );
        });

        Ok(json!({ "hostId": host_id, "reused": false }))
    }

    /// `host.remove` - the other half of `host.add`, and the one that was missing.
    ///
    /// `protocol/types.ts` has declared this method, its `{ hostId }` param and its
    /// `{ removed: boolean }` result since the schema was written; the daemon answered `unsupported`
    /// and the UI had no button, so a host added by mistake was permanent. The row, its sessions and
    /// everything hanging off them go; the **event log does not**, because it is append-only and the
    /// removal is itself an event (`HostRemoved`) that a reconnecting window replays.
    fn host_remove(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let host_id = envelope.require_str("hostId")?;

        if host_id == "local" {
            return Err(ErrorObject::bad_request(
                "`local` is the machine this daemon is running on, so it cannot be removed",
            ));
        }

        let row = self.store().host(&host_id).map_err(ErrorObject::internal)?;

        let Some(row) = row else {
            return Err(ErrorObject::not_found(format!("`{host_id}` is not a host this daemon knows")));
        };

        let name = row["name"].as_str().unwrap_or(&host_id).to_string();
        let sessions = self.store().delete_host(&host_id).map_err(ErrorObject::internal)?;

        out.push(event::host_removed(&host_id, &name, sessions), None, None);

        Ok(json!({ "removed": true, "name": name, "sessions": sessions }))
    }

    fn session_open(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let host_id = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());
        let title = envelope.opt_str("title").unwrap_or_else(|| "New chat".into());
        let prompt = envelope.opt_str("prompt").unwrap_or_default();
        let project_id = envelope.opt_str("projectId").filter(|id| !id.trim().is_empty());
        let session_id = format!("n{}", self.state.events.seq() + 1);

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
    fn project_root_of(&self, project_id: Option<&str>) -> Result<Option<String>, ErrorObject> {
        let Some(id) = project_id else {
            return Ok(None);
        };

        Ok(self
            .store()
            .project(id)
            .map_err(ErrorObject::internal)?
            .and_then(|project| project.get("root").and_then(Value::as_str).map(str::to_string)))
    }

    fn session_update(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.require_str("sessionId")?;
        let title = envelope.opt_str("title");
        let state = envelope.opt_str("state");
        /* `Open folder` on an existing chat: the folder it works in changes, and the window's chip
           follows the `SessionUpdated` below rather than guessing. */
        let project_id = envelope.opt_str("projectId").filter(|id| !id.trim().is_empty());
        let root = self.project_root_of(project_id.as_deref())?;

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
    fn session_fork(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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
        let fork_id = format!("n{}", self.state.events.seq() + 1);
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

    fn session_close(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.require_str("sessionId")?;

        out.push(event::session_closed(&session_id), Some(session_id.clone()), None);
        self.store().delete_session(&session_id).map_err(ErrorObject::internal)?;

        Ok(json!({}))
    }

    /* -----------------------------------------------------------------------------------------
     * Projects: the folder a chat works in (0.7.6)
     * -------------------------------------------------------------------------------------- */

    /// `project.add`: what `Open folder` calls. Answers with the project, and **reuses** the row when the
    /// host already has that root, because opening the same folder twice must not leave two rows for one
    /// directory - which is also what makes the button safe to press again.
    ///
    /// The validation is the reason this is a method rather than a row the app writes: `is_dir` is asked
    /// here, so a path that is a file, a typo, or a folder on a host that cannot see it is refused in the
    /// daemon's own words instead of becoming a chat whose working directory does not exist.
    fn project_add(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let host_id = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());
        let root = envelope.require_str("root")?;

        if !std::path::Path::new(&root).is_dir() {
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

    /// `project.remove`: closes the folder. Its chats are **unbound**, not deleted - a chat is a
    /// conversation, and a folder is a place to have it. The answer says how many chats were unbound.
    fn project_remove(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    /// `engine.start`: answer now, stream later.
    fn engine_start(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let engine_id = envelope.opt_str("engine").unwrap_or_else(|| "claude_code".into());
        let model = envelope
            .opt_str("model")
            .unwrap_or_else(|| crate::duel::model_for(&engine_id).to_string());
        let tier = envelope
            .opt_str("tier")
            .unwrap_or_else(|| crate::engines::tier_for(&engine_id).to_string());
        let prompt_text = envelope.opt_str("prompt").unwrap_or_default();
        /* The provider the model was picked from. It is what makes a live-only model id
           (`deepseek-v4-pro`, which this build's catalogue has never seen) resolvable to a real
           endpoint instead of the loopback `custom` one. */
        let provider = envelope.opt_str("provider").filter(|id| !id.trim().is_empty());
        let turn_id = format!("turn-{}", self.state.events.seq() + 1);
        let engine = self.state.engines.get(&engine_id).ok_or_else(|| {
            ErrorObject::not_found(format!(
                "`{engine_id}` is not an engine this daemon has; known: {}",
                self.state.engines.ids().join(", ")
            ))
        })?;

        let history = session_bridge::history_for(self.store(), &session_id)?;

        /* A turn may arrive for a session the daemon has not seen yet (the app seeds its chats in the
           UI), so the row is made to exist before the turn references it. */
        self.store().ensure_session(&session_id).map_err(ErrorObject::internal)?;
        let ordinal = self
            .store()
            .turns(&session_id)
            .map(|turns| turns.len() as i64 + 1)
            .unwrap_or(1);

        self.store()
            .start_turn(&turn_id, &session_id, ordinal, &engine_id, &model, &tier, &prompt_text)
            .map_err(ErrorObject::internal)?;

        out.push(
            event::turn_started(&turn_id, &session_id, &engine_id, &model, &tier, &prompt_text),
            Some(session_id.clone()),
            Some(turn_id.clone()),
        );

        /* The folder this chat works in, resolved *now* from the session's project and carried in the
           plan, so the engine can be started inside it. A chat bound to a folder runs there; a chat with
           none runs wherever the daemon was started, which is what every chat did before 0.7.6. */
        let project_root = self
            .store()
            .session_project_root(&session_id)
            .map_err(ErrorObject::internal)?;

        /* The turn runs on its own task, so the response can go back before the first token does. */
        let state = self.state.clone();
        let notifier = out.clone();
        let answer_turn_id = turn_id.clone();
        let plan = RunPlan {
            session_id,
            turn_id,
            engine_id,
            prompt_text,
            model,
            provider,
            history,
            project_root,
        };

        tokio::spawn(async move {
            run_turn(state, engine, plan, notifier).await;
        });

        Ok(json!({ "turnId": answer_turn_id }))
    }

    fn engine_stop(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let turn_id = envelope.require_str("turnId")?;
        let killed = envelope.method == "engine.kill";
        let summary = if killed { "Force killed" } else { "Interrupted" };

        out.push(event::turn_completed(&turn_id, summary, "", Some(false)), None, Some(turn_id.clone()));

        Ok(json!({ "state": "killed", "engine": if killed { "kill" } else { "cancel" } }))
    }

    fn engine_status(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let turn_id = envelope.opt_str("turnId").unwrap_or_default();
        let engine_id = envelope.opt_str("engine").unwrap_or_else(|| "claude_code".into());
        let status = self
            .state
            .engines
            .get(&engine_id)
            .map(|engine| engine.status(&turn_id))
            .unwrap_or(EngineStatus::Idle);

        Ok(json!({ "state": status.as_str(), "engine": engine_id }))
    }

    /// Spec section 16.5: switch engines mid-turn and replay the conversation into the new one.
    fn engine_switch(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let turn_id = envelope.require_str("turnId")?;
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let to = envelope.opt_str("engine").unwrap_or_else(|| "codex".into());
        let model = envelope
            .opt_str("model")
            .unwrap_or_else(|| crate::duel::model_for(&to).to_string());
        let snapshot = session_bridge::snapshot(self.store(), &session_id, &turn_id, &to, &model)?;

        out.push(
            session_bridge::frame(&session_id, &turn_id, "claude_code", &to, &model, envelope.opt_str("reason").as_deref()),
            Some(session_id),
            Some(turn_id.clone()),
        );

        Ok(json!({ "turnId": turn_id, "bridgedFrom": "claude_code", "snapshot": snapshot }))
    }

    /* -----------------------------------------------------------------------------------------
     * Providers, permission, checkpoints, rewind, duel, console and the event replay
     * -------------------------------------------------------------------------------------- */

    fn provider_save(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("id")?;
        let kind = envelope.opt_str("kind").unwrap_or_else(|| "api-key".into());
        let saved = providers::save(
            self.store(),
            &id,
            &kind,
            envelope.opt_str("key").as_deref(),
            envelope.opt_str("label").as_deref(),
            envelope.opt_str("url").as_deref(),
            envelope.opt_str("protocol").as_deref(),
        )?;

        out.push(
            event::provider_status(json!({
                "id": id,
                "status": "connected",
                "account": saved["account"],
                "kind": kind,
                "models": providers::MODELS.len(),
            })),
            None,
            None,
        );

        Ok(saved)
    }

    fn provider_oauth_callback(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("id")?;
        let state = envelope.opt_str("state").unwrap_or_default();

        /* The token exchange is a later step; the answer says so rather than inventing a token. */
        Ok(providers::oauth_callback(&id, &state))
    }

    fn provider_registry_set(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("id")?;
        let enabled = envelope.opt_bool("enabled");
        /* Named first, so the `json!` macro is never asked to parse an `if` expression. */
        let disabled: Vec<String> = if enabled { Vec::new() } else { vec![id.clone()] };

        out.push(
            event::registry_loaded(json!(providers::registry(&disabled))),
            None,
            None,
        );

        Ok(json!({ "id": id, "enabled": enabled }))
    }

    /* -----------------------------------------------------------------------------------------
     * The engines' environment: files, git, processes
     * -------------------------------------------------------------------------------------- */

    fn project_root(&self, envelope: &Envelope) -> Option<std::path::PathBuf> {
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
    fn root_for(&self, envelope: &Envelope) -> Result<Option<std::path::PathBuf>, ErrorObject> {
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
    fn root_required(&self, envelope: &Envelope) -> Result<std::path::PathBuf, ErrorObject> {
        self.root_for(envelope)?.ok_or_else(|| {
            ErrorObject::bad_request("`root` is required, or a `sessionId` whose chat has a folder")
        })
    }

    /// How much of a file `fs.read` will put in one answer: 1 MiB.
    ///
    /// The window's file view reads through this, and a 200 MB log is not something to hand a WebView.
    /// What comes back says it was cut (`truncated: true`, and `bytes` is the file's real size), and the
    /// `sha256` is still of the **whole file** - a hash of the first megabyte would be a hash of
    /// something that is not the file.
    const MAX_READ_BYTES: usize = 1024 * 1024;

    fn fs_read(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let path = std::path::PathBuf::from(envelope.require_str("path")?);
        let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);

        if size <= Self::MAX_READ_BYTES as u64 {
            let (text, sha256) = crate::fs::read(&path)?;

            return Ok(json!({
                "path": path.display().to_string(),
                "text": text,
                "sha256": sha256,
                "bytes": size,
                "truncated": false,
            }));
        }

        let (text, bytes) = crate::fs::read_capped(&path, Self::MAX_READ_BYTES)?;
        let sha256 = crate::fs::hash_file(&path)?;

        Ok(json!({
            "path": path.display().to_string(),
            "text": text,
            "sha256": sha256,
            "bytes": bytes,
            "truncated": true,
        }))
    }

    fn fs_write(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let path = std::path::PathBuf::from(envelope.require_str("path")?);
        let text = envelope.opt_str("text").unwrap_or_default();
        let sha256 = crate::fs::write(&path, &text)?;

        Ok(json!({ "path": path.display().to_string(), "sha256": sha256, "bytes": text.len() }))
    }

    /// `fs.list` - one level of a directory, for the window's file tree.
    ///
    /// `path` is **optional** since 0.7.7: without one the *session's* folder is listed, which is what
    /// the tree asks for - it knows the chat, not the directory - and it is the same rule `root_for`
    /// applies to `git.*` and `fs.search`. The answer names the directory it listed, so a caller that
    /// gave only a session id can say which folder it is looking at, and counts the names the guard hid.
    fn fs_list(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let path = match envelope.opt_str("path") {
            Some(path) => std::path::PathBuf::from(path),
            None => self.root_required(envelope)?,
        };
        let (entries, hidden) = crate::fs::list(&path)?;

        Ok(json!({
            "path": path.display().to_string(),
            "entries": entries,
            "hidden": hidden,
        }))
    }

    fn fs_stat(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        crate::fs::stat(&std::path::PathBuf::from(envelope.require_str("path")?))
    }

    fn fs_search(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let root = self.root_for(envelope)?.unwrap_or_else(|| std::path::PathBuf::from("."));
        let query = envelope.require_str("query")?;
        let glob = envelope.opt_str("glob");
        let limit = envelope.opt_i64("limit").unwrap_or(50).clamp(1, 500) as usize;

        Ok(json!({ "hits": crate::fs::search(&root, &query, glob.as_deref(), limit)? }))
    }

    fn git_status(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let root = self.root_required(envelope)?;
        let (branch, dirty) = crate::git::status(&root)?;

        Ok(json!({ "branch": branch, "dirty": dirty }))
    }

    fn git_diff(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let root = self.root_required(envelope)?;
        let patch = crate::git::diff(&root, envelope.opt_str("sha").as_deref())?;

        Ok(json!({ "patch": patch }))
    }

    /// `git.checkpoint`: a checkpoint whose hash comes from the shadow repository's commit, and which
    /// emits the same `CheckpointSaved` event `checkpoint.create` does.
    fn git_checkpoint(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let root = self.project_root(envelope);
        let turn = envelope.opt_i64("turn").unwrap_or_else(|| self.state.events.seq());
        let fresh = crate::checkpoints::create(
            self.store(),
            &session_id,
            turn,
            &envelope.opt_str("title").unwrap_or_else(|| "Checkpoint".into()),
            root.as_deref(),
            crate::checkpoints::screenshot::capture(),
        )?;

        out.push(event::checkpoint_saved(&session_id, fresh.to_event_payload()), Some(session_id), None);

        Ok(json!({ "checkpointId": fresh.id, "sha": fresh.files_hash }))
    }

    fn git_worktree(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let root = self.project_root(envelope).ok_or_else(|| ErrorObject::bad_request("`root` is required"))?;
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());

        Ok(json!({ "path": crate::git::worktree(&root, &session_id)?.display().to_string() }))
    }

    fn pty_open(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let command = envelope.require_str("command")?;
        let args: Vec<String> = envelope
            .params
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        self.state.pty.open(&command, &args, envelope.opt_str("cwd").as_deref())
    }

    fn pty_write(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let pty_id = envelope.require_str("ptyId")?;

        self.state.pty.write(&pty_id, &envelope.opt_str("data").unwrap_or_default())?;

        Ok(json!({ "written": true }))
    }

    fn pty_close(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let pty_id = envelope.require_str("ptyId")?;

        Ok(json!({ "closed": self.state.pty.close(&pty_id) }))
    }

    /// `pty.output`: the tail of a long-running process, and whether it is still alive.
    fn pty_output(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        self.state.pty.output(&envelope.require_str("ptyId")?)
    }

    /// `cli.login`: start the CLI's own sign-in and put its URL in front of the user.
    ///
    /// The answer arrives immediately with a `loginId`; the URL is what `cli.login.status` reports a
    /// moment later, because a CLI prints it once its screen is drawn. `program`/`args`/`pump` are the
    /// escape hatch: they let the flow be pointed at any CLI, which is also how the mechanism is tested
    /// without Claude installed.
    fn cli_login_start(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let provider_id = envelope.require_str("providerId")?;
        let args = envelope
            .params
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            });
        let pump = envelope
            .params
            .get("pump")
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            });
        let started = self.state.logins.start(
            &provider_id,
            envelope.opt_str("program").as_deref(),
            args,
            pump,
        )?;

        /* The step before the login, when the CLI needs one: Gemini CLI will not start a sign-in until
           its settings name an auth method (`Prepare::GeminiOauth`). It happens *before* `start`, and a
           failure is reported as the error it is - writing the user's settings file is not something to
           do quietly and then fail at. */
        let prepared = match crate::auth::cli_login::recipe(&provider_id) {
            Some(recipe) => match crate::auth::cli_login::prepare(recipe) {
                Ok(path) => path,
                Err(reason) => {
                    return Err(ErrorObject::internal(format!(
                        "the CLI's sign-in needs a setting first, and it could not be written: {reason}"
                    )))
                }
            },
            None => None,
        };

        out.push(
            event::provider_status(json!({
                "id": provider_id,
                "status": "connecting",
                "detail": "signing in through the CLI",
            })),
            None,
            None,
        );

        /* `prepared` names the file the prepare step touched, so the modal can say what was written
           rather than changing a user's configuration silently. */
        Ok(match prepared {
            Some(path) => {
                let mut answer = started;

                if let Some(object) = answer.as_object_mut() {
                    object.insert("prepared".to_string(), json!(path));
                }

                answer
            }
            None => started,
        })
    }

    /// `cli.login.status`: the URL, the CLI's own output, and where the sign-in has got to.
    ///
    /// It also announces a success the moment it sees one - see `status_announcing` for why that is
    /// the difference between a card that flips to `connected` and a card that stays `connecting`
    /// under a login that worked.
    fn cli_login_status(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let (answer, announce) = self
            .state
            .logins
            .status_announcing(&envelope.require_str("loginId")?)?;

        if announce {
            out.push(
                event::provider_status(json!({
                    "id": answer["providerId"],
                    "status": "connected",
                    "detail": format!(
                        "signed in through `{}`'s own CLI",
                        answer["providerLabel"].as_str().unwrap_or("the")
                    ),
                })),
                None,
                None,
            );
        }

        Ok(answer)
    }

    /// `cli.login.code`: hand the pasted code to the CLI. The daemon passes it through and keeps
    /// nothing - the credential is written by the CLI, in the CLI's own store.
    fn cli_login_code(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let login_id = envelope.require_str("loginId")?;
        let submitted = self.state.logins.submit_code(&login_id, &envelope.require_str("code")?)?;
        let status = self.state.logins.status(&login_id)?;

        /* The provider card follows the login: `connecting` while the page is open, `connected` the
           moment the CLI says it signed in. The card is folded from this event, so a UI that reloads
           mid-login still ends up right. */
        out.push(
            event::provider_status(json!({
                "id": status["providerId"],
                "status": if status["authenticated"] == Value::Bool(true) { "connected" } else { "connecting" },
                "detail": if status["authenticated"] == Value::Bool(true) {
                    "signed in through the CLI"
                } else {
                    "waiting for the CLI to confirm"
                },
            })),
            None,
            None,
        );

        Ok(submitted)
    }

    fn cli_login_cancel(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        self.state.logins.cancel(&envelope.require_str("loginId")?)
    }

    /// `models.list`: what the providers have, from the provider when it can be reached, from the cache
    /// when it cannot, and from the bundle underneath both.
    fn models_list(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let refresh = envelope.opt_bool("refresh");
        let listed = crate::providers::models::list(
            self.store(),
            envelope.opt_str("providerId").as_deref(),
            refresh,
        )?;

        if refresh {
            let notes = listed["notes"].as_array().cloned().unwrap_or_default();

            out.push(
                event::registry_loaded(listed["models"].clone()),
                None,
                None,
            );

            if !notes.is_empty() {
                out.push(
                    event::toast(
                        &format!(
                            "Model list refreshed · {} {} from the bundle",
                            listed["snapshot"].as_str().unwrap_or("?"),
                            "rows"
                        ),
                        None,
                        None,
                    ),
                    None,
                    None,
                );
            }
        }

        Ok(listed)
    }

    /// `models.select`: record the chosen model. A setting, not an event - see `providers::models`.
    fn models_select(&self, _envelope: &Envelope, _out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let model_id = _envelope.require_str("modelId")?;

        crate::providers::models::select(self.store(), &model_id, _envelope.opt_str("providerId").as_deref())
    }

    /// `shell.run`: one command, run to completion, with its output captured and its failure
    /// translated.
    ///
    /// This is the daemon's **execute** step - the one an agent loop needs in order to *do* something
    /// rather than describe it - and it is also what a user can drive directly. Two rules from the
    /// rest of the daemon apply here, and they are why this is a handler rather than a pass-through:
    ///
    /// * a command may mutate the tree, so a checkpoint is written **before** it runs (principle P5);
    /// * the run is announced as a tool call, so the turn stream shows it the way it shows an engine's
    ///   own tool calls - the app needs no second way to render "a command ran".
    fn shell_run(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let command = envelope.require_str("command")?;
        let args: Vec<String> = envelope
            .params
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let cwd = envelope.opt_str("cwd");
        let session_id = envelope.opt_str("sessionId");
        let turn_id = envelope.opt_str("turnId");
        let timeout = Duration::from_secs(
            envelope.opt_i64("timeoutMs").map(|ms| (ms as u64 / 1000).max(1)).unwrap_or(120),
        );
        let call_id = format!("shell-{}", self.state.events.seq() + 1);

        if let (Some(session), Some(root)) = (session_id.as_deref(), self.root_for(envelope)?.as_deref()) {
            let ordinal = self.state.events.seq();

            if let Ok(fresh) = crate::checkpoints::create(
                self.store(),
                session,
                ordinal,
                &format!("Before `{command}`"),
                Some(root),
                crate::checkpoints::screenshot::capture(),
            ) {
                out.push(
                    event::checkpoint_saved(session, fresh.to_event_payload()),
                    Some(session.to_string()),
                    turn_id.clone(),
                );
            }
        }

        if let Some(turn) = turn_id.as_deref() {
            out.push(
                event::tool_call_started(turn, &call_id, "run", &command, cwd.as_deref().unwrap_or(".")),
                session_id.clone(),
                turn_id.clone(),
            );
        }

        let result = self.state.pty.run_once(&command, &args, cwd.as_deref(), timeout)?;

        if let Some(turn) = turn_id.as_deref() {
            let status = if result["ok"] == Value::Bool(true) { "done" } else { "failed" };
            let meta = format!("exit {} · {}ms", result["exitCode"], result["durationMs"].as_u64().unwrap_or(0));

            out.push(
                event::tool_call_completed(turn, &call_id, status, &meta, None),
                session_id.clone(),
                turn_id.clone(),
            );
        }

        Ok(result)
    }

    fn permission_request(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let permission_id = format!("perm-{}", self.state.events.seq() + 1);

        out.push(
            event::permission_requested(json!({
                "permissionId": permission_id,
                "sessionId": session_id,
                "turnId": envelope.opt_str("turnId"),
                "title": "Delete a file",
                "sub": "Claude wants to perform a mutating action",
                "action": envelope.opt_str("action").unwrap_or_else(|| "delete".into()),
                "target": envelope.opt_str("target").unwrap_or_else(|| "src/database.js".into()),
                "risk": envelope.opt_str("risk").unwrap_or_else(|| "MUTATING".into()),
                "explain": "Your database connection settings. If this is deleted, your app will stop loading data.",
                "checkpointId": Value::Null,
            })),
            Some(session_id),
            None,
        );

        Ok(json!({ "permissionId": permission_id }))
    }

    fn permission_resolve(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let permission_id = envelope.require_str("permissionId")?;
        let decision = envelope.opt_str("decision").unwrap_or_else(|| "deny".into());

        out.push(event::permission_resolved(&permission_id, &decision), None, None);

        Ok(json!({ "decision": decision }))
    }

    fn checkpoint_create(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let title = envelope.opt_str("title").unwrap_or_else(|| "Checkpoint".into());
        let turn = envelope.opt_i64("turn").unwrap_or_else(|| self.state.events.seq());
        /* The session's folder when the caller does not send one - see `root_for`. */
        let root = self.root_for(envelope)?;
        let fresh = crate::checkpoints::create(
            self.store(),
            &session_id,
            turn,
            &title,
            root.as_deref(),
            crate::checkpoints::screenshot::capture(),
        )?;

        out.push(
            event::checkpoint_saved(&session_id, fresh.to_event_payload()),
            Some(session_id),
            None,
        );

        Ok(json!({ "checkpointId": fresh.id, "filesHash": fresh.files_hash }))
    }

    fn rewind_apply(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let turn = crate::checkpoints::turn_of(&envelope.require_str("turnId")?);
        /* The session's folder when the caller does not send one: a rewind restores the files of the chat
           it belongs to, which is the folder that chat works in. */
        let root = self.root_for(envelope)?;
        let applied = crate::rewind::apply(self.store(), &session_id, turn, root.as_deref())?;

        out.push(
            applied.to_event_payload(&session_id),
            Some(session_id.clone()),
            None,
        );
        out.push(
            event::toast(
                &format!("Rewound to turn {turn}"),
                Some("Undo this"),
                Some(10_000),
            ),
            Some(session_id),
            None,
        );

        Ok(json!({ "removedTurns": applied.turns, "restoredFiles": applied.files }))
    }

    fn rewind_redo(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());

        match crate::rewind::redo(self.store(), &session_id)? {
            Some(applied) => {
                out.push(applied.to_event_payload(&session_id), Some(session_id), None);

                Ok(json!({ "turn": applied.turn }))
            }
            None => Ok(json!({ "turn": Value::Null })),
        }
    }

    /* -----------------------------------------------------------------------------------------
     * Duel, the console bridge, and the event replay
     * -------------------------------------------------------------------------------------- */

    /// Spec section 16.6: two engines, one prompt, two panes.
    fn duel_start(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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
        let duel_id = format!("duel-{}", self.state.events.seq() + 1);

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
    fn duel_keep(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    fn console_attach(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let url = envelope.opt_str("url").unwrap_or_else(|| "http://localhost:3000".into());

        self.state.console.attach(&session_id, &url)
    }

    fn console_detach(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());

        self.state.console.detach(&session_id)
    }

    /// Flow 1's `Remove`: the secret leaves the keychain and the card goes back to `available`.
    fn provider_remove(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("id")?;
        let removed = providers::remove(self.store(), &id)?;

        out.push(
            event::provider_status(json!({ "id": id, "status": "available", "account": Value::Null })),
            None,
            None,
        );

        Ok(removed)
    }

    /// `checkpoint.restore`: the same restore as a rewind, reached by the checkpoint's id rather than
    /// by a turn number. It answers with how many turns went back, which is what `restored` means.
    fn checkpoint_restore(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let checkpoint_id = envelope.require_str("checkpointId")?;
        let checkpoint = self
            .store()
            .checkpoint(&checkpoint_id)
            .map_err(ErrorObject::internal)?
            .ok_or_else(|| ErrorObject::not_found(format!("no checkpoint `{checkpoint_id}`")))?;
        let session_id = checkpoint["sessionId"].as_str().unwrap_or("s1").to_string();
        let turn = checkpoint["turn"].as_i64().unwrap_or(0);
        let root = self.project_root(envelope);
        let applied = crate::rewind::apply(self.store(), &session_id, turn, root.as_deref())?;

        out.push(applied.to_event_payload(&session_id), Some(session_id), None);

        Ok(json!({ "restored": applied.turns, "turn": turn }))
    }

    /// `event.append`: a client records an event of its own - the log is the state, so a UI-only fact
    /// that matters (a toast the user acted on) belongs in it (spec section 3.3). Every subscriber
    /// sees it, exactly like an event the daemon raised.
    fn event_append(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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
    fn event_list(&self, envelope: &Envelope) -> Value {
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

/* ------------------------------------------------------------------------------------------------
 * The turn task. It is outside the `impl` because it owns nothing but the plan and the notifier: the
 * engine runs here, the events go out from here, and the state it touches is reached through the
 * daemon state it was handed.
 * ---------------------------------------------------------------------------------------------- */

/// Everything a turn needs after the call has already answered.
struct RunPlan {
    session_id: String,
    turn_id: String,
    engine_id: String,
    prompt_text: String,
    /// The model the turn runs on, resolved by `engine_start` from the request, the session or the
    /// registry. It travels to the engine inside the `Prompt`, because the engine cannot guess it -
    /// see the note on `Prompt::model`.
    model: String,
    /// The provider the model belongs to, when the caller said. `native_api` needs it to find the
    /// endpoint (and therefore the key entry) of a model id this build's catalogue has never seen.
    provider: Option<String>,
    history: Vec<String>,
    /// The folder the chat works in, or `None` for a chat that has no project. It reaches the adapters
    /// inside the `Prompt`, which is where every fact about the session that the engine cannot guess
    /// travels - the same reason the model and the provider are there.
    project_root: Option<String>,
}

/// Runs one turn and pushes its events - including the checkpoint that must exist *before* a mutating
/// tool runs (principle P5).
///
/// The engine's stream is *followed*, not awaited: each item is pushed the moment it arrives, which is
/// what makes the turn stream live (see the note on the channel below).
async fn run_turn(
    state: Arc<DaemonState>,
    engine: Arc<dyn crate::engines::Engine>,
    plan: RunPlan,
    out: Arc<dyn Notifier>,
) {
    let session = Some(plan.session_id.clone());
    let turn = Some(plan.turn_id.clone());
    let prompt = Prompt {
        session_id: plan.session_id.clone(),
        turn_id: plan.turn_id.clone(),
        text: plan.prompt_text.clone(),
        model: plan.model.clone(),
        provider: plan.provider.clone(),
        history: plan.history.clone(),
        project_root: plan.project_root.clone(),
    };
    let mut answer = String::new();
    let mut checkpoint_written = false;

    /* The engine runs on its own task and writes into a channel; this loop reads it and pushes each
      event out while the engine is still talking.
    *
    * `let events = engine.start(prompt).await;` is what used to be here, and it is the whole defect
    * the report *"akbare answare disse"* names: the adapter's stream was complete before the first
    * notification left this function, so a two-minute turn arrived as one lump - thinking, tool
    * calls and answer together - with the window looking idle until the end.
    *
    * The channel keeps the two properties the checkpoint rule needs: **order** (one producer, one
    * consumer, so a `ToolCallStarted` cannot overtake the `CheckpointSaved` written for it below) and
    * **a single place that talks to the notifier** (this loop, never the engine). */
    let (sink, mut stream) = EventSink::channel();
    let running = tokio::spawn(async move {
        engine.start(prompt, &sink).await;

        /* `sink` is dropped here, which is what ends the loop below: no separate "done" signal can be
        lost on the way. */
    });

    while let Some(event) = stream.recv().await {
        match event {
            crate::engines::EngineEvent::Delta(delta) => {
                answer.push_str(&delta);
                out.push(event::turn_delta(&plan.turn_id, &delta), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::Thinking(text) => {
                out.push(event::thinking_delta(&plan.turn_id, &text), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::ToolStarted { call_id, tool, name, target } => {
                if !checkpoint_written && ["edit", "write", "delete"].contains(&tool.as_str()) {
                    let ordinal = state.events.seq();
                    /* The chat's own folder, so a checkpoint written before a mutating tool hashes the
                       files that tool is about to touch. Until 0.7.6 this passed `None`, which meant the
                       checkpoint recorded a conversation and no files at all. */
                    let root = plan.project_root.as_deref().map(std::path::Path::new);

                    if let Ok(fresh) = crate::checkpoints::create(
                        &state.store,
                        &plan.session_id,
                        ordinal,
                        &format!("Before {name}"),
                        root,
                        crate::checkpoints::screenshot::capture(),
                    ) {
                        out.push(
                            event::checkpoint_saved(&plan.session_id, fresh.to_event_payload()),
                            session.clone(),
                            turn.clone(),
                        );
                    }

                    checkpoint_written = true;
                }

                out.push(
                    event::tool_call_started(&plan.turn_id, &call_id, &tool, &name, &target),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::ToolOutput { call_id, level, text } => {
                out.push(
                    event::tool_call_output(&plan.turn_id, &call_id, &level, &text),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::ToolCompleted { call_id, status, meta } => {
                out.push(
                    event::tool_call_completed(&plan.turn_id, &call_id, &status, &meta, None),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Failed(reason) => {
                /* Every failure goes through the translator, so the card always has a sentence -
                   `ErrorRaised` carries the title and the explanation, never a raw stack. The
                   `event::error_raised` constructor supplies the catalogue's `type` field. */
                let translated = translator::translate(&plan.engine_id, &reason);

                out.push(
                    event::error_raised(
                        &plan.session_id,
                        Some(&plan.turn_id),
                        &translated.title,
                        &translated.explanation,
                        Some(&plan.engine_id),
                    ),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Done { summary, meta, pass } => {
                out.push(
                    event::turn_completed(&plan.turn_id, &summary, &meta, pass),
                    session.clone(),
                    turn.clone(),
                );
            }
        }
    }

    /* The task is joined so the turn's own bookkeeping cannot race it: a `finish_turn` before the
    engine's last event was pushed would be a stored answer that is missing its tail. */
    let _ = running.await;

    let failed = answer.is_empty();
    let state_name = if failed { "error" } else { "success" };

    let _ = state.store.finish_turn(&plan.turn_id, &answer, "Done", state_name);
    let _ = state.store.update_session(&plan.session_id, None, Some(state_name), None, Some(0), None);

    out.push(
        event::session_updated(json!({
            "sessionId": plan.session_id,
            "state": state_name,
            "minutesAgo": 0,
        })),
        session,
        None,
    );
}

/* ----------------------------------------------------------------------------------------------------
 * Whether a host can actually be reached
 *
 * `host.add` used to answer `connecting` and leave it there, which is why adding a server felt like
 * nothing happened: the row appeared, the dot stayed blue, and the same host could be added again
 * and again until the sidebar was four copies of the same name. The two functions below turn that
 * into a measurement with a sentence attached.
 * -------------------------------------------------------------------------------------------------- */

/// The sentence for a probe that could not even be started.
fn unreachable_sentence(target: &str) -> String {
    format!("{target} was added, but the probe could not be run; check the daemon's log")
}

/// Can this machine reach an SSH target? Returns the status and the sentence that goes with it.
///
/// Three flags carry the whole point:
///
/// * `BatchMode=yes` - without it `ssh` asks for a password on a stdin that has nobody attached and
///   the daemon waits for a human who is not there;
/// * `ConnectTimeout=8` - bounds the TCP step, so a black-holed address costs seconds rather than a
///   stuck turn;
/// * `StrictHostKeyChecking=accept-new` - a first connection to a fresh VPS is the normal case, and
///   the alternative is an interactive prompt with no terminal behind it.
///
/// The command is `true`, i.e. "can I get a shell", which is exactly the question. It runs through
/// `host::program::command`, so a Windows `ssh.exe` is resolved the same way every other program in
/// this daemon is.
fn probe_ssh(target: &crate::auth::remote::SshTarget) -> (String, String) {
    let Some(mut command) = host::program::command("ssh") else {
        return (
            "offline".to_string(),
            format!("{} was added, but `ssh` is not installed on this machine", target.user_host),
        );
    };

    let mut args = target.port_args();

    args.extend([
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=8".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        target.user_host.clone(),
        "true".to_string(),
    ]);

    let output = command.args(&args).output();

    match output {
        Ok(output) if output.status.success() => (
            "connected".to_string(),
            format!("{} is reachable", target.user_host),
        ),
        Ok(output) => (
            "offline".to_string(),
            ssh_refusal(&target.user_host, &String::from_utf8_lossy(&output.stderr)),
        ),
        Err(reason) => (
            "offline".to_string(),
            format!("{} could not be contacted: {reason}", target.user_host),
        ),
    }
}

/// The sentence for an `ssh` that ran and refused - and the reason this function exists.
///
/// The common refusal by far is a host that wants a password or a one-time verification code: the
/// daemon runs `ssh` in batch mode on purpose (there is no terminal behind this call, so a prompt
/// would hang it), which means SDC cannot type that code, and reporting "unreachable" for a machine
/// the user logs into from a terminal every day is both wrong and impossible to act on. The target
/// answered; it asked for something this call cannot give.
///
/// The way out is what it always was for a headless tool: a key. A host that accepts one connects
/// without a prompt, and the sentence says so where the user is looking.
fn ssh_refusal(target: &str, stderr: &str) -> String {
    let line = first_line(stderr);
    let lowered = line.to_lowercase();

    if lowered.contains("keyboard-interactive") || lowered.contains("permission denied") {
        return format!(
            "{target} answered, but it asks for a password or a verification code. Add this host again with its password filled in (Add a host → SSH / VPS) and SDC copies its key over once, after which every connection is passwordless - or add a key to `~/.ssh/authorized_keys` yourself."
        );
    }

    if lowered.contains("host key verification failed") {
        return format!(
            "{target} answered, but its host key is not in `known_hosts` and the check cannot be answered here. Run `ssh {target}` once in a terminal to accept it."
        );
    }

    /* Everything else is reported in `ssh`'s own words: a timeout, a refused port, a bad key. */
    format!("{target} did not answer: {line}")
}

/// The last segment of a path: the name `Open folder` gives a project without asking the user for one.
/// `H:\SDC` becomes `SDC`, `/home/me/app` becomes `app`, and a path with no last segment (a drive root,
/// `/`) keeps itself rather than becoming an empty label.
fn folder_name(root: &str) -> String {
    std::path::Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| root.to_string())
}

/// The first non-empty line of a program's output - a sentence, not a wall of stderr.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_string()
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The VPS in the bug report: `ssh` refused because the host wants a password or a one-time
    /// verification code - the user logs in from a terminal every day, so "unreachable" is the wrong
    /// sentence and leaves them nothing to do.
    #[test]
    fn a_host_that_wants_a_password_is_not_reported_as_unreachable() {
        let sentence = ssh_refusal(
            "root@vps.example",
            "root@vps.example: Permission denied (keyboard-interactive,publickey).\n",
        );

        assert!(sentence.contains("answered"), "{sentence}");
        assert!(sentence.contains("verification code"), "{sentence}");
        assert!(sentence.contains("authorized_keys"), "{sentence}");
        assert!(!sentence.contains("did not answer"), "{sentence}");
    }

    /// Everything else keeps `ssh`'s own first line, because that is what a person can act on.
    #[test]
    fn every_other_refusal_keeps_ssh_own_words() {
        let timed_out = ssh_refusal("h", "\nssh: connect to host h port 22: Connection timed out\nmore\n");

        assert_eq!(timed_out, "h did not answer: ssh: connect to host h port 22: Connection timed out");

        let host_key = ssh_refusal("h", "Host key verification failed.\n");

        assert!(host_key.contains("known_hosts"), "{host_key}");
    }

    /// The acceptance test for live streaming - the report *"akbare answare disse"*, in code.
    ///
    /// The fake engine **cannot finish until the daemon has already pushed its first delta**: it sends
    /// `Hel`, then waits for a signal that only the notifier's `push` of that delta can open, and only
    /// then sends the rest. If `run_turn` still collected the stream before forwarding it - which is
    /// exactly what it did until 0.7.4 - the turn would deadlock here, and the timeout below would fail
    /// the suite instead of hanging it.
    ///
    /// So the assertion is about **when** the event left the daemon, not only about its text: the
    /// second half of the answer exists only because the first half had already been delivered.
    #[tokio::test]
    async fn a_turn_streams_its_deltas_while_the_engine_is_still_running() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::sync::oneshot;

        /// An engine whose second half is gated on the daemon's delivery of its first half.
        struct Halfway {
            release: std::sync::Mutex<Option<oneshot::Receiver<()>>>,
        }

        #[async_trait::async_trait]
        impl crate::engines::Engine for Halfway {
            fn id(&self) -> &'static str {
                "halfway"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                sink.send(crate::engines::EngineEvent::Delta("Hel".to_string()));

                let release = self
                    .release
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .take();

                if let Some(release) = release {
                    let _ = release.await;
                }

                sink.send(crate::engines::EngineEvent::Delta("lo".to_string()));
                sink.send(crate::engines::EngineEvent::Done {
                    summary: "Done".to_string(),
                    meta: String::new(),
                    pass: Some(true),
                });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Running
            }
        }

        /// A notifier that opens the gate on the first `TurnDelta` it is given.
        struct GateNotifier {
            opened: AtomicBool,
            release: std::sync::Mutex<Option<oneshot::Sender<()>>>,
            events: std::sync::Mutex<Vec<Value>>,
        }

        impl GateNotifier {
            fn kinds(&self) -> Vec<String> {
                self.events
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .iter()
                    .filter_map(|event| {
                        event
                            .get("type")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect()
            }
        }

        impl Notifier for GateNotifier {
            fn push(
                &self,
                event: Value,
                _session: Option<String>,
                _turn: Option<String>,
            ) -> Option<i64> {
                let first_delta = event.get("type").and_then(Value::as_str) == Some("TurnDelta")
                    && !self.opened.swap(true, Ordering::SeqCst);

                if first_delta {
                    let release = self
                        .release
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .take();

                    if let Some(release) = release {
                        let _ = release.send(());
                    }
                }

                self.events
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .push(event);

                None
            }
        }

        let (release, gated) = oneshot::channel();
        let notifier = Arc::new(GateNotifier {
            opened: AtomicBool::new(false),
            release: std::sync::Mutex::new(Some(release)),
            events: std::sync::Mutex::new(Vec::new()),
        });
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "halfway".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(Halfway {
            release: std::sync::Mutex::new(Some(gated)),
        });
        let out: Arc<dyn Notifier> = notifier.clone();

        let ran = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_turn(state, engine, plan, out),
        )
        .await;

        assert!(
            ran.is_ok(),
            "the turn never ended: the daemon was not forwarding deltas as they arrived"
        );
        assert_eq!(
            notifier.kinds(),
            vec!["TurnDelta", "TurnDelta", "TurnCompleted", "SessionUpdated"]
        );
    }
}
