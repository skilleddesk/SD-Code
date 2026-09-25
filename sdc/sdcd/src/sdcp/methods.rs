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
            "host.doctor" => self.host_doctor(envelope),
            "host.add" => self.host_add(envelope, out),
            "host.trust" => self.host_trust(envelope, out),
            "host.key" => self.host_key(envelope, &*out),
            "ssh.key" => self.ssh_key(envelope),
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
            "verify.run" => self.verify_run(envelope, out),

            /* The engines' environment ------------------------------------------------------- */
            "fs.read" => self.fs_read(envelope),
            "fs.write" => self.fs_write(envelope, &*out),
            "fs.list" => self.fs_list(envelope),
            "fs.stat" => self.fs_stat(envelope),
            "fs.search" => self.fs_search(envelope),
            "fs.rename" => self.fs_rename(envelope, &*out),
            "fs.delete" => self.fs_delete(envelope, &*out),
            "fs.mkdir" => self.fs_mkdir(envelope),
            "git.status" => self.git_status(envelope),
            "git.diff" => self.git_diff(envelope),
            "git.checkpoint" => self.git_checkpoint(envelope, &*out),
            "git.worktree" => self.git_worktree(envelope),
            "pty.open" => self.pty_open(envelope),
            "pty.write" => self.pty_write(envelope),
            /* `pty.resize` answers `unsupported` (0.7.10): this build's "pty" is a pipe runner with no window to
               resize, and answering `{}` was a promise it did not keep. The empty `result` is that decision, said
               in the schema rather than discovered by a caller. */
            "pty.resize" => Err(crate::sdcp::envelope::ErrorObject::unsupported(
                "this build does not resize a pty: the pane re-reads `pty.output` instead",
            )),
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

        out.push(event::host_status("local", "Local", "local", "connected", Some(&platform), None, None), None, None);

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
            /* How the fallback file is protected, when the fallback is what is in use: `acl` on Windows since
               0.7.10, `mode` on unix, `os` when the OS store answered. "file" alone hid the difference between a
               0600 file and one every account in `Users` could read. */
            "keyProtection": crate::auth::keychain::protection(),
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

    /// `host.add` - a machine joins the list, and the layers of `docs/REMOTE.md` run in order.
    ///
    /// 1. **the target is parsed**, and this time the port is **stored** with it. 0.7.0 parsed
    ///    `ssh -p 8443 user@host` correctly and then wrote only `user@host` into the row, so the port
    ///    was used by the probe and lost for everything after it - half of "vps connect korai jasse nah".
    /// 2. **SDC's key is made**, because `-i` plus `IdentitiesOnly=yes` means that key is the only
    ///    credential this daemon will ever offer.
    /// 3. **the host's key is scanned and checked against SDC's pin** (`ssh::hostkey`): a key exchange
    ///    with no authentication, so nothing is offered before the machine's identity is decided. A key
    ///    that is not the pinned one is refused; a host with no pin is recorded `untrusted` and the
    ///    answer carries its fingerprint, which is what the dialog asks about.
    /// 4. **the password is spent last**, and only against a pinned key. A password typed for a host
    ///    whose key is unknown is dropped here rather than sent to whatever answered the port.
    fn host_add(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        if envelope.opt_str("type").unwrap_or_else(|| "ssh".into()) == "local" {
            return Ok(json!({ "hostId": "local" }));
        }

        let raw_target = envelope.opt_str("target").unwrap_or_default().trim().to_string();

        if raw_target.is_empty() {
            return Err(ErrorObject::bad_request("`target` is required for an SSH host"));
        }

        /* What the user typed is parsed, not trusted - and this is a fix, not tidiness. The report
           pasted `ssh -p 8443 deploy@203.0.113.10`, which is what a person types into their
           own shell: the daemon used the whole string as a hostname (`ssh` was asked for a machine
           called `ssh`) and the port was never used. `parse_target` pulls both out, `0003-host-ssh`
           stores both, and the port travels with every `ssh` call this daemon makes for that host. */
        let ssh = crate::ssh::Ssh::parse(&raw_target)?;
        let target = ssh.target.user_host.clone();
        /* Used for one install and dropped. It is never stored, never logged, and never part of a
           sentence the UI shows. */
        let password = envelope.opt_str("password").unwrap_or_default().trim().to_string();

        let label = envelope
            .opt_str("label")
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| target.clone());

        /* The key is made here, once. A machine without `ssh-keygen` is told about rather than ignored:
           with no key and no pin a probe cannot succeed, and the sentence names which of the two is
           missing instead of leaving a red dot with no explanation. */
        let key_note = crate::auth::remote::ensure_key().err();

        /*
         * The same machine added twice is one host.
         *
         * A sidebar of `Website, Website, Website` is what the alternative looks like in practice: a
         * second row for the same `user@host` says nothing the first one did not. The row's id comes
         * back with `reused: true`, so a caller can say "already there" instead of pretending it just
         * connected - and its **address is refreshed**, because a person who re-adds a host with its
         * port meant that port.
         */
        let existing = self.store().host_id_for_target(&target).map_err(ErrorObject::internal)?;
        let reused = existing.is_some();
        let host_id = match existing {
            Some(existing) => {
                self.store()
                    .set_host_address(&existing, &target, ssh.target.port)
                    .map_err(ErrorObject::internal)?;

                existing
            }
            None => {
                let host_id = format!("h{}", self.state.events.seq() + 1);

                self.store()
                    .upsert_host(&host_id, &label, "ssh", Some(&target), "connecting", None)
                    .map_err(ErrorObject::internal)?;
                self.store()
                    .set_host_address(&host_id, &target, ssh.target.port)
                    .map_err(ErrorObject::internal)?;

                host_id
            }
        };
        let name = self
            .store()
            .host(&host_id)
            .map_err(ErrorObject::internal)?
            .and_then(|row| row["name"].as_str().map(str::to_string))
            .unwrap_or(label);

        out.push(
            event::host_status(
                &host_id,
                &name,
                "vps",
                "connecting",
                None,
                Some(&format!("checking {target}'s host key…")),
                None,
            ),
            None,
            None,
        );

        let state = self.state.clone();
        let notifier = out.clone();
        /* The id the answer needs, taken before the task takes its own copy of everything else. */
        let answer_host_id = host_id.clone();

        /*
         * The measurement runs *after* the answer, which is the `engine.start` shape: the dialog has a
         * host on screen and the sentences arrive when there is something to attach them to.
         *
         * Three endings, and each one says what happened rather than "it did not work":
         *   * **pinned** - the key matches SDC's own record, so the one-time install (when a password
         *     was given) and the probe run;
         *   * **unknown** - this machine has never been pinned, so it is recorded `untrusted`, the
         *     fingerprint travels with the event, and the dialog asks. The password is *not* used;
         *   * **changed** - the host presents a key that is not the pinned one. Nothing is sent, and the
         *     sentence says so with both fingerprints.
         */
        tokio::spawn(async move {
            let scan_target = ssh.target.clone();
            let seen = tokio::task::spawn_blocking(move || crate::ssh::hostkey::inspect(&scan_target))
                .await
                .unwrap_or_else(|_| Err(ErrorObject::internal("the host key scan could not be run")));

            match seen {
                Ok(crate::ssh::hostkey::Trust::Pinned(_)) => {
                    finish_connection(state, notifier, host_id, name, ssh, password, key_note).await;
                }
                Ok(crate::ssh::hostkey::Trust::Unknown(keys)) => {
                    let fingerprint = crate::ssh::hostkey::primary(&keys)
                        .map(|key| key.fingerprint.clone())
                        .unwrap_or_default();

                    let _ = state
                        .store
                        .upsert_host(&host_id, &name, "ssh", Some(&target), "untrusted", None);

                    notifier.push(
                        event::host_status(
                            &host_id,
                            &name,
                            "vps",
                            "untrusted",
                            None,
                            Some(&untrusted_sentence(&target, &fingerprint, key_note.as_deref())),
                            Some(&fingerprint),
                        ),
                        None,
                        None,
                    );
                }
                Ok(crate::ssh::hostkey::Trust::Changed { pinned, seen }) => {
                    let sentence = changed_sentence(&target, &pinned, &seen);

                    let _ = state
                        .store
                        .upsert_host(&host_id, &name, "ssh", Some(&target), "offline", None);

                    notifier.push(
                        event::host_status(&host_id, &name, "vps", "offline", None, Some(&sentence), None),
                        None,
                        None,
                    );
                }
                Err(error) => {
                    let _ = state
                        .store
                        .upsert_host(&host_id, &name, "ssh", Some(&target), "offline", None);

                    notifier.push(
                        event::host_status(&host_id, &name, "vps", "offline", None, Some(&error.message), None),
                        None,
                        None,
                    );
                }
            }
        });

        Ok(json!({ "hostId": answer_host_id, "reused": reused }))
    }

    /// `host.trust` - the one question a remote connection asks a person, and the answer to it.
    ///
    /// The dialog shows a fingerprint (`SHA256:…`) that `host.add` scanned, a person decides, and this
    /// method pins it. Three rules make it a decision rather than a formality:
    ///
    /// * the machine is **scanned again**, and the fingerprint being pinned has to be one of the keys
    ///   it presents *now* (`hostkey::confirm`). A key that changes between the question and the answer
    ///   is exactly the case a pin exists for, and it is refused with both fingerprints;
    /// * only the key whose fingerprint was shown is pinned - never the other types the machine offers,
    ///   which nobody looked at;
    /// * the password, when the dialog still has it, is spent **after** the pin, so the credential
    ///   reaches a host whose identity has been checked. If the pin fails, it is never sent.
    fn host_trust(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let host_id = envelope.require_str("hostId")?;
        let fingerprint = envelope.require_str("fingerprint")?;
        let password = envelope.opt_str("password").unwrap_or_default().trim().to_string();

        let Some(ssh) = self.ssh_for(&host_id)? else {
            return Err(ErrorObject::bad_request(format!(
                "`{host_id}` is not an SSH host this daemon can reach, so there is no key to trust"
            )));
        };

        let name = self
            .store()
            .host(&host_id)
            .map_err(ErrorObject::internal)?
            .and_then(|row| row["name"].as_str().map(str::to_string))
            .unwrap_or_else(|| ssh.label());

        /* The scan and the pin happen here rather than in a task on purpose: a failure has to be an
           *answer* the dialog can show (`the key is not the one you confirmed`), not an event that
           arrives after it has closed. */
        let keys = crate::ssh::hostkey::confirm(&ssh.target, &fingerprint)?;

        crate::ssh::hostkey::pin_into(&crate::ssh::hostkey::known_hosts_path()?, &keys)?;
        self.store().set_host_key(&host_id, &fingerprint).map_err(ErrorObject::internal)?;

        out.push(
            event::host_status(
                &host_id,
                &name,
                "vps",
                "connecting",
                None,
                Some(&format!("{fingerprint} pinned · connecting…")),
                Some(&fingerprint),
            ),
            None,
            None,
        );

        let state = self.state.clone();
        let notifier = out.clone();

        spawn_finish(state, notifier, host_id.clone(), name, ssh, password, None);

        Ok(json!({ "trusted": true, "hostId": host_id, "fingerprint": fingerprint }))
    }

    /// `host.doctor` - the environment checks, about the machine the caller names (0.7.13).
    ///
    /// Before this release the ten local checks answered for **any** `hostId`, so asking about a VPS
    /// returned ten rows about the laptop: whether *this* machine has `claude` installed, under a
    /// heading that said the host's name. A host now gets rows about itself (`host::remote_checks`),
    /// including the two questions that only make sense for one - is its key the pinned one, and can the
    /// engines run *there*.
    fn host_doctor(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let host_id = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());

        let Some(ssh) = self.ssh_for(&host_id)? else {
            return Ok(json!({ "checks": host::checks(self.store()) }));
        };

        /* The chat's folder, when the caller knows the chat: a folder that is not there is the most
           actionable row of the lot, and it is the one thing only the caller can name. */
        let root = match envelope.opt_str("sessionId") {
            Some(session_id) => self
                .store()
                .session_project_root(&session_id)
                .map_err(ErrorObject::internal)?,
            None => None,
        };

        Ok(json!({ "checks": host::remote_checks(&ssh, root.as_deref()) }))
    }

    ///
    /// `host.add` asks it once, in the same breath as adding the host, and pushes the answer as a
    /// `HostStatus`. A window that was not open at that moment - a relaunch, a second window, a host
    /// added days ago - has the row (`untrusted` or `offline`) and the *sentence* (which is also in the
    /// event), but the fingerprint is a value a button needs, and a sentence is not a value. That is the
    /// gap this method closes, and it closes the re-pin case with the same call: a host whose key
    /// **changed** answers `matches: false` with the fingerprint it presents now, which is exactly what
    /// `Re-pin` has to confirm.
    ///
    /// It is a measurement, not a tab open: it scans (`ssh-keyscan`, no authentication) and pushes a
    /// `HostStatus` only when the answer makes the host's row say something it did not already say.
    /// `ssh.key` - the **public** half of the key SDC uses for the hosts it adds (0.7.13).
    ///
    /// Read-only, and it never creates a key: making one is `host.add`'s job, on the path where a key is
    /// actually needed. It exists because one case cannot be automated and must not be a dead end - a host
    /// that requires a **verification code** cannot be set up by this daemon (a one-time code is a second
    /// factor, and a daemon holding one would defeat it), so the window has to be able to show the line a
    /// person pastes into `~/.ssh/authorized_keys` themselves. A public key is meant to be shown; it is
    /// the private half that never leaves `~/.ssh`.
    fn ssh_key(&self, _envelope: &Envelope) -> Result<Value, ErrorObject> {
        let public = crate::auth::remote::public_key();
        let path = crate::auth::remote::key_path();

        Ok(json!({
            "publicKey": public,
            "path": path.map(|path| path.display().to_string()),
            "exists": public.is_some(),
        }))
    }

    fn host_key(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let host_id = envelope.require_str("hostId")?;

        let Some(ssh) = self.ssh_for(&host_id)? else {
            return Err(ErrorObject::bad_request(format!(
                "`{host_id}` is not an SSH host, so it has no key to look at"
            )));
        };

        let name = self
            .store()
            .host(&host_id)
            .map_err(ErrorObject::internal)?
            .and_then(|row| row["name"].as_str().map(str::to_string))
            .unwrap_or_else(|| ssh.label());
        let answer = crate::ssh::hostkey::inspect(&ssh.target)?;
        let fingerprint_of = |keys: &[crate::ssh::hostkey::HostKey]| {
            crate::ssh::hostkey::primary(keys)
                .map(|key| key.fingerprint.clone())
                .unwrap_or_default()
        };
        let key_type_of = |keys: &[crate::ssh::hostkey::HostKey]| {
            crate::ssh::hostkey::primary(keys)
                .map(|key| key.key_type.clone())
                .unwrap_or_default()
        };

        match answer {
            crate::ssh::hostkey::Trust::Pinned(key) => Ok(json!({
                "hostId": host_id,
                "hostKey": key.fingerprint,
                "keyType": key.key_type,
                "pinned": true,
                "matches": true,
                "pinnedKey": key.fingerprint,
            })),
            crate::ssh::hostkey::Trust::Unknown(keys) => {
                let fingerprint = fingerprint_of(&keys);

                /* The row's story has not changed (it was `untrusted` or it is being added), so this is
                   an answer rather than news - but a host that had *no* fingerprint on its row gets one
                   now, which is the sentence the card needs. */
                out.push(
                    event::host_status(
                        &host_id,
                        &name,
                        "vps",
                        "untrusted",
                        None,
                        Some(&untrusted_sentence(&ssh.target.user_host, &fingerprint, None)),
                        Some(&fingerprint),
                    ),
                    None,
                    None,
                );

                Ok(json!({
                    "hostId": host_id,
                    "hostKey": fingerprint,
                    "keyType": key_type_of(&keys),
                    "pinned": false,
                    "matches": Value::Null,
                    "pinnedKey": Value::Null,
                }))
            }
            crate::ssh::hostkey::Trust::Changed { pinned, seen } => {
                let sentence = changed_sentence(&ssh.target.user_host, &pinned, &seen);
                let fingerprint = fingerprint_of(&seen);

                /* A changed key *is* news, and it is the loud kind: the host goes `offline` with both
                   fingerprints named, on its own row, where a person will see it without asking. */
                let _ = self
                    .store()
                    .upsert_host(&host_id, &name, "ssh", Some(&ssh.target.user_host), "offline", None);

                out.push(
                    event::host_status(&host_id, &name, "vps", "offline", None, Some(&sentence), None),
                    None,
                    None,
                );

                Ok(json!({
                    "hostId": host_id,
                    "hostKey": fingerprint,
                    "keyType": key_type_of(&seen),
                    "pinned": true,
                    "matches": false,
                    "pinnedKey": pinned.first().cloned().unwrap_or_default(),
                }))
            }
        }
    }

    /// The SSH side of a host, when the daemon has an address for it: `None` for `local`, and `None`
    /// for a host row that was never added as an SSH target.
    fn ssh_for(&self, host_id: &str) -> Result<Option<crate::ssh::Ssh>, ErrorObject> {
        if host_id == "local" {
            return Ok(None);
        }

        let Some((Some(target), port)) = self.store().host_address(host_id).map_err(ErrorObject::internal)? else {
            return Ok(None);
        };

        Ok(Some(crate::ssh::Ssh::new(crate::auth::remote::SshTarget { user_host: target, port })))
    }

    /// The host a file, git or shell method works on: the one the envelope names, or - since 0.7.6 -
    /// the host the **session** belongs to.
    ///
    /// The envelope first, because a caller that names a host means that host; the session is the
    /// fallback that makes `fs.list { path }` work on a remote chat: the app knows the chat, the daemon
    /// knows which machine that chat's folder is on.
    fn host_id_for(&self, envelope: &Envelope) -> Result<Option<String>, ErrorObject> {
        if let Some(host_id) = envelope.opt_str("hostId").filter(|host_id| !host_id.trim().is_empty()) {
            return Ok(Some(host_id));
        }

        let Some(session_id) = envelope.opt_str("sessionId") else {
            return Ok(None);
        };

        Ok(self
            .store()
            .session(&session_id)
            .map_err(ErrorObject::internal)?
            .and_then(|session| session["hostId"].as_str().map(str::to_string)))
    }

    /// The remote connection a method should use, or `None` when the work is local.
    fn remote_for(&self, envelope: &Envelope) -> Result<Option<crate::ssh::Ssh>, ErrorObject> {
        match self.host_id_for(envelope)? {
            Some(host_id) => self.ssh_for(&host_id),
            None => Ok(None),
        }
    }

    /// Where a checkpoint's or a rewind's files are - the host and the folder, **owned**.
    ///
    /// `checkpoints::Snapshot` borrows its two halves, and a call site that had to build one would have
    /// to keep an `Option<Ssh>` and an `Option<PathBuf>` alive next to it and remember which of the three
    /// cases it was in. This holds both, so a site is one line: `self.subject(envelope)?.snapshot()`.
    fn subject(&self, envelope: &Envelope) -> Result<Subject, ErrorObject> {
        let remote = self.remote_for(envelope)?;
        let root = self.root_for(envelope)?;
        let root_text = root
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();

        Ok(Subject { remote, root, root_text })
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
        /*
         * Agent mode (v4). The three CLIs are agents already - a turn on `claude_code` reads, edits and
         * runs by itself - so for them the switch changes nothing. An API model or a local one is a chat
         * unless the daemon runs the loop for it: that is `agent::SdcAgent`, built per turn with the
         * autonomy level the person chose, on the same turn pipeline (events, checkpoint, Stop).
         */
        let agent_mode = envelope.params.get("agent").and_then(Value::as_bool).unwrap_or(false);
        let backend = match engine_id.as_str() {
            "native_api" => Some(crate::agent::Backend::Api),
            "ollama" => Some(crate::agent::Backend::Ollama),
            _ => None,
        };
        let agent = backend.filter(|_| agent_mode).map(|backend| {
            crate::agent::SdcAgent::new(
                backend,
                crate::agent::gate::Autonomy::parse(&envelope.opt_str("autonomy").unwrap_or_default()),
                envelope
                    .opt_i64("maxSteps")
                    .map(|steps| steps.max(1) as usize)
                    .unwrap_or(crate::agent::DEFAULT_STEPS),
            )
        });

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
        /* And **which machine** that folder is on (0.7.13): with a host here, the adapter runs the CLI
           over `ssh` in that folder rather than locally (see `engines::cli::remote_command`). */
        let remote = self.remote_for(envelope)?;

        /*
         * An agent takes its own checkpoint, synchronously, before its first change (agent::tools::Checkpointer):
         * a checkpoint written when the turn loop *received* the agent's ToolStarted was written after the file,
         * because the agent does not wait for the loop. The closure owns what it needs - the store, the
         * notifier, which chat, which turn, and where the folder is.
         */
        let self_checkpointing = agent.is_some();
        let engine: Arc<dyn crate::engines::Engine> = match agent {
            Some(agent) => {
                let store = self.state.store.clone();
                let notifier = out.clone();
                let (session, turn) = (session_id.clone(), turn_id.clone());
                let (root, ssh) = (project_root.clone(), remote.clone());
                let events = self.state.events.clone();
                let checkpoint: crate::agent::tools::Checkpointer = Arc::new(move |title: &str| {
                    let snapshot = match (&ssh, root.as_deref()) {
                        (Some(ssh), Some(root)) => crate::checkpoints::Snapshot::Remote(ssh, root),
                        (None, Some(root)) => crate::checkpoints::Snapshot::Local(std::path::Path::new(root)),
                        _ => crate::checkpoints::Snapshot::Unbound,
                    };

                    if let Ok(fresh) = crate::checkpoints::create(
                        &store,
                        &session,
                        events.seq(),
                        title,
                        snapshot,
                        crate::checkpoints::screenshot::capture(),
                    ) {
                        notifier.push(
                            event::checkpoint_saved(&session, fresh.to_event_payload()),
                            Some(session.clone()),
                            Some(turn.clone()),
                        );
                    }
                });

                Arc::new(agent.with_checkpoint(checkpoint))
            }
            None => engine,
        };

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
            remote,
            self_checkpointing,
        };

        tokio::spawn(async move {
            run_turn(state, engine, plan, notifier).await;
        });

        Ok(json!({ "turnId": answer_turn_id }))
    }

    /// `verify.run` (v4): the folder's own checks, then a review of the change by another engine.
    ///
    /// Answers with the run's id at once; the run itself streams as `VerifyUpdated` snapshots. The app
    /// sends what it knows and the daemon cannot derive: the turn's first checkpoint (`since`, a shadow
    /// commit - the review reads everything after it) and the turn's own prompt (`task`).
    fn verify_run(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let session_id = envelope.require_str("sessionId")?;
        let root = self
            .root_for(envelope)?
            .and_then(|root| root.to_str().map(str::to_string))
            .ok_or_else(|| ErrorObject::bad_request("Verify needs a folder: open one for this chat first"))?;
        let since = envelope
            .opt_str("since")
            .filter(|sha| sha.len() == 40 && sha.chars().all(|character| character.is_ascii_hexdigit()));
        let reviewer = envelope.params.get("reviewer").and_then(|reviewer| {
            let engine = reviewer["engine"].as_str().filter(|engine| !engine.is_empty())?;

            Some(crate::verify::Reviewer {
                engine: engine.to_string(),
                model: reviewer["model"].as_str().unwrap_or_default().to_string(),
                provider: reviewer["provider"].as_str().filter(|id| !id.is_empty()).map(str::to_string),
            })
        });
        let verify_id = format!("verify-{}", self.state.events.seq() + 1);
        let request = crate::verify::Request {
            verify_id: verify_id.clone(),
            session_id,
            turn_id: envelope.opt_str("turnId"),
            since,
            task: envelope.opt_str("task").unwrap_or_default(),
            root,
            remote: self.remote_for(envelope)?,
            reviewer,
            review_failing: envelope.params.get("reviewFailing").and_then(Value::as_bool).unwrap_or(false),
        };
        let state = self.state.clone();

        tokio::spawn(async move {
            crate::verify::run(state, out, request).await;
        });

        Ok(json!({ "verifyId": verify_id }))
    }

    fn engine_stop(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let turn_id = envelope.require_str("turnId")?;
        let killed = envelope.method == "engine.kill";
        let summary = if killed { "Force killed" } else { "Interrupted" };

        /*
         * Stop the turn for real. This method used to push the `TurnCompleted` below and nothing else: the
         * engine was never told, so the model kept answering (and billing) behind a turn the window
         * already called interrupted - and its next delta flipped the turn back to `running`.
         *
         * The mark comes first, so the turn loop drops whatever the engine still emits; then the engine
         * that is running the turn is asked to stop (a CLI's process group is killed, here or on the
         * host), on a task of its own because a remote kill is an `ssh` round trip.
         */
        crate::engines::cancel::request(&turn_id);

        let engine = self
            .store()
            .turn_engine(&turn_id)
            .ok()
            .flatten()
            .and_then(|engine_id| self.state.engines.get(&engine_id));
        let stopped = engine.is_some();

        if let Some(engine) = engine {
            let turn = turn_id.clone();

            tokio::spawn(async move {
                engine.cancel(&turn).await;
            });
        }

        out.push(event::turn_completed(&turn_id, summary, "", Some(false)), None, Some(turn_id.clone()));

        Ok(json!({ "state": "killed", "engine": if killed { "kill" } else { "cancel" }, "stopped": stopped }))
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

    /// `fs.read` - the file's text, its hash, its real size, and whether the text was cut.
    ///
    /// Since 0.7.13 the path may be on a **host**: `hostId` (or the session's own host) decides, and
    /// `ssh::ops::read` answers with exactly this shape from the far side of the connection. The tree,
    /// the Preview editor and the checkpoint stamp read one contract either way, which is what makes a
    /// folder on a VPS a folder rather than a special case.
    fn fs_read(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let raw = envelope.require_str("path")?;

        if let Some(ssh) = self.remote_for(envelope)? {
            return crate::ssh::ops::read(&ssh, &raw, Self::MAX_READ_BYTES);
        }

        let path = std::path::PathBuf::from(&raw);
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

    /// `fs.write` - the one method that changes a file, and therefore the one that must not change it without
    /// a checkpoint first (principle P5).
    ///
    /// Until 0.7.9 nothing could reach this method from the window - it was implemented and had no caller -
    /// and the checkpoint-before-mutation rule lived only on the tool-call path. Now the file view's Save
    /// calls it, and the rule is enforced **here**: for this file, in this place, whatever asked for the
    /// write. `sessionId` is optional, because a caller with no chat (a probe, a script) has nothing to
    /// checkpoint *against*; with one, the checkpoint is taken first and pushed as `CheckpointSaved` before a
    /// single byte is written - a checkpoint taken afterwards would be a photograph of the damage.
    ///
    /// **On a host the checkpoint is the host's**: its shadow repository at `$HOME/.sdc/git/<hash>` there
    /// (0.7.13), so a remote save is rewound the same way a local one is. Until v4 this branch skipped the
    /// checkpoint while the window's toast said one had been taken.
    fn fs_write(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let raw = envelope.require_str("path")?;
        let text = envelope.opt_str("text").unwrap_or_default();
        let session_id = envelope.opt_str("sessionId");
        let turn_id = envelope.opt_str("turnId");

        if let Some(ssh) = self.remote_for(envelope)? {
            /* The checkpoint first, on the host's own shadow repository (0.7.13 made one). This branch used
               to write straight away, while the window's toast said "a checkpoint was taken on that host" -
               so a Save on a VPS was the one change Rewind could not undo. */
            if let Some(session) = session_id.as_deref() {
                let subject = self.subject(envelope)?;
                let name = raw.rsplit('/').next().unwrap_or("a file").to_string();

                if let Ok(fresh) = crate::checkpoints::create(
                    self.store(),
                    session,
                    self.state.events.seq(),
                    &format!("Before editing {name}"),
                    subject.snapshot(),
                    crate::checkpoints::screenshot::capture(),
                ) {
                    out.push(
                        event::checkpoint_saved(session, fresh.to_event_payload()),
                        Some(session.to_string()),
                        turn_id.clone(),
                    );
                }
            }

            let sha256 = crate::ssh::ops::write(&ssh, &raw, &text)?;

            return Ok(json!({ "path": raw, "sha256": sha256, "bytes": text.len() }));
        }

        let path = std::path::PathBuf::from(&raw);

        if let Some(session) = session_id.as_deref() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("a file")
                .to_string();
            /* The root comes from the session (0.7.6), so the checkpoint hashes the files that are about to
               change rather than nothing - and since 0.7.13 it is the same folder whether that is a local
               path or a path on a host, because the subject says which. */
            let subject = self.subject(envelope)?;

            if let Ok(fresh) = crate::checkpoints::create(
                self.store(),
                session,
                self.state.events.seq(),
                &format!("Before editing {name}"),
                subject.snapshot(),
                crate::checkpoints::screenshot::capture(),
            ) {
                out.push(
                    event::checkpoint_saved(session, fresh.to_event_payload()),
                    Some(session.to_string()),
                    turn_id.clone(),
                );
            }
        }

        let sha256 = crate::fs::write(&path, &text)?;

        Ok(json!({
            "path": path.display().to_string(),
            "sha256": sha256,
            "bytes": text.len(),
        }))
    }

    /// A checkpoint of the chat's folder before a change a person made from the window (P5): the same one
    /// `fs.write` takes, on whichever machine the folder is. Without a session there is nothing to
    /// checkpoint against, and nothing is taken.
    fn checkpoint_first(&self, envelope: &Envelope, title: &str, out: &dyn Notifier) -> Result<(), ErrorObject> {
        let Some(session) = envelope.opt_str("sessionId") else {
            return Ok(());
        };
        let subject = self.subject(envelope)?;

        if let Ok(fresh) = crate::checkpoints::create(
            self.store(),
            &session,
            self.state.events.seq(),
            title,
            subject.snapshot(),
            crate::checkpoints::screenshot::capture(),
        ) {
            out.push(
                event::checkpoint_saved(&session, fresh.to_event_payload()),
                Some(session.clone()),
                envelope.opt_str("turnId"),
            );
        }

        Ok(())
    }

    /// Refuses a path that is not strictly inside the chat's folder - what `fs.rename` and `fs.delete`
    /// need, because a tree row is always inside it and a request that is not is not from the tree.
    fn inside_folder(&self, envelope: &Envelope, raw: &str) -> Result<(), ErrorObject> {
        let Some(root) = self.root_for(envelope)? else {
            return Err(ErrorObject::bad_request("This needs a chat with a folder (`sessionId`), so the change stays inside it"));
        };
        let root_text = root.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
        let inside = if self.remote_for(envelope)?.is_some() {
            raw.starts_with(&format!("{root_text}/")) && !raw.split('/').any(|part| part == "..")
        } else {
            crate::fs::strictly_inside(&root, std::path::Path::new(raw))
        };

        if inside {
            Ok(())
        } else {
            Err(ErrorObject::blocked(&format!("`{raw}` is not inside this chat's folder ({root_text})")))
        }
    }

    /// `fs.rename` (v4) - the tree's Rename, with a checkpoint first.
    fn fs_rename(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let from = envelope.require_str("path")?;
        let to = envelope.require_str("to")?;

        self.inside_folder(envelope, &from)?;
        self.inside_folder(envelope, &to)?;

        let name = from.rsplit(['/', '\\']).next().unwrap_or("a file").to_string();

        self.checkpoint_first(envelope, &format!("Before renaming {name}"), out)?;

        match self.remote_for(envelope)? {
            Some(ssh) => crate::ssh::ops::rename(&ssh, &from, &to)?,
            None => crate::fs::rename(std::path::Path::new(&from), std::path::Path::new(&to))?,
        }

        Ok(json!({ "path": to, "renamed": true }))
    }

    /// `fs.delete` (v4) - the tree's Delete, with a checkpoint first so Rewind brings it back.
    fn fs_delete(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let raw = envelope.require_str("path")?;

        self.inside_folder(envelope, &raw)?;

        let name = raw.rsplit(['/', '\\']).next().unwrap_or("a file").to_string();

        self.checkpoint_first(envelope, &format!("Before deleting {name}"), out)?;

        match self.remote_for(envelope)? {
            Some(ssh) => crate::ssh::ops::remove(&ssh, &raw)?,
            None => crate::fs::remove(std::path::Path::new(&raw))?,
        }

        Ok(json!({ "path": raw, "deleted": true }))
    }

    /// `fs.mkdir` (v4) - the tree's New folder.
    fn fs_mkdir(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let raw = envelope.require_str("path")?;

        self.inside_folder(envelope, &raw)?;

        match self.remote_for(envelope)? {
            Some(ssh) => crate::ssh::ops::mkdir(&ssh, &raw)?,
            None => crate::fs::mkdir(std::path::Path::new(&raw))?,
        }

        Ok(json!({ "path": raw, "created": true }))
    }

    /// `fs.list` - one level of a directory, for the window's file tree.
    ///
    /// `path` is **optional** since 0.7.7: without one the *session's* folder is listed, which is what
    /// the tree asks for - it knows the chat, not the directory - and it is the same rule `root_for`
    /// applies to `git.*` and `fs.search`. The answer names the directory it listed, so a caller that
    /// gave only a session id can say which folder it is looking at, and counts the names the guard hid.
    ///
    /// On a host the same contract is answered from the far side (`ssh::ops::list`), and the path comes
    /// back **absolute** even when the caller named it as `~/app` - which is how the tree stops needing
    /// to know that homes exist.
    fn fs_list(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let remote = self.remote_for(envelope)?;
        let raw = match envelope.opt_str("path") {
            Some(path) => path,
            None => self
                .root_required(envelope)?
                .to_str()
                .map(str::to_string)
                .unwrap_or_default(),
        };

        if let Some(ssh) = remote {
            let (path, entries, hidden) = crate::ssh::ops::list(&ssh, &raw)?;

            return Ok(json!({ "path": path, "entries": entries, "hidden": hidden }));
        }

        let path = std::path::PathBuf::from(&raw);
        let (entries, hidden) = crate::fs::list(&path)?;

        Ok(json!({
            "path": path.display().to_string(),
            "entries": entries,
            "hidden": hidden,
        }))
    }

    /// `fs.stat` - `{path, size, dir, sha256}` on whichever machine the path is on.
    fn fs_stat(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let raw = envelope.require_str("path")?;

        if let Some(ssh) = self.remote_for(envelope)? {
            return crate::ssh::ops::stat(&ssh, &raw);
        }

        crate::fs::stat(&std::path::PathBuf::from(&raw))
    }

    /// `fs.search` - a literal search, capped, on the folder's own machine.
    ///
    /// Locally this is a small walk (`fs::search`) because `rg` may be missing; on a host it is `grep
    /// -rnI --fixed-strings`, which every Linux and macOS box has. Both answer `{path, line, text}`, and
    /// both honour the guard on the names they walk past.
    fn fs_search(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let remote = self.remote_for(envelope)?;
        let root = self.root_for(envelope)?;
        let query = envelope.require_str("query")?;
        let glob = envelope.opt_str("glob");
        let limit = envelope.opt_i64("limit").unwrap_or(50).clamp(1, 500) as usize;

        if let Some(ssh) = remote {
            let root = match root {
                Some(root) => root.to_str().map(str::to_string).unwrap_or_default(),
                None => crate::ssh::ops::home(&ssh)?,
            };

            return Ok(json!({ "hits": crate::ssh::ops::search(&ssh, &root, &query, glob.as_deref(), limit)? }));
        }

        let root = root.unwrap_or_else(|| std::path::PathBuf::from("."));

        Ok(json!({ "hits": crate::fs::search(&root, &query, glob.as_deref(), limit)? }))
    }

    /// `git.status` - which branch, and how many files the working tree has changed.
    ///
    /// Read from the **folder's own** machine, so a chat pointed at `/srv/app` on a VPS is told about
    /// that repository and not about anything on this laptop. `("", 0)` - no badge - for a folder that
    /// is not a repository, on either side.
    fn git_status(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let remote = self.remote_for(envelope)?;
        let root = self.root_required(envelope)?;

        if let Some(ssh) = remote {
            let root = root.to_str().map(str::to_string).unwrap_or_default();
            let (branch, dirty) = crate::ssh::ops::git_status(&ssh, &root)?;

            return Ok(json!({ "branch": branch, "dirty": dirty }));
        }

        let (branch, dirty) = crate::git::status(&root)?;

        Ok(json!({ "branch": branch, "dirty": dirty }))
    }

    /// `git.diff` - the working tree's patch, from the folder's own machine.
    fn git_diff(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let remote = self.remote_for(envelope)?;
        let root = self.root_required(envelope)?;
        let since = envelope.opt_str("sha");

        if let Some(ssh) = remote {
            let root = root.to_str().map(str::to_string).unwrap_or_default();

            return Ok(json!({ "patch": crate::ssh::ops::git_diff(&ssh, &root, since.as_deref())? }));
        }

        let patch = crate::git::diff(&root, since.as_deref())?;

        Ok(json!({ "patch": patch }))
    }

    /// `git.checkpoint`: a checkpoint whose hash comes from the shadow repository's commit, and which
    /// emits the same `CheckpointSaved` event `checkpoint.create` does.
    ///
    /// The shadow repository is on the **folder's own machine** (0.7.13): a chat on a host gets its
    /// history in `$HOME/.sdc/git/<project hash>` *there*, which is what makes a rewind able to restore
    /// files that only exist on that host.
    fn git_checkpoint(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let subject = self.subject(envelope)?;
        let turn = envelope.opt_i64("turn").unwrap_or_else(|| self.state.events.seq());
        let fresh = crate::checkpoints::create(
            self.store(),
            &session_id,
            turn,
            &envelope.opt_str("title").unwrap_or_else(|| "Checkpoint".into()),
            subject.snapshot(),
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

    /// `pty.open` - a long-running process, here or **on a host** (0.7.13).
    ///
    /// With a `hostId` (or a session whose folder is on one) the child is an `ssh` whose remote command
    /// is `cd <cwd> && sh -c 'echo $$ > <pid>; exec setsid … <command> …'`, which makes the far side of
    /// this method behave exactly like the near side: `pty.output` reads the process's output tail,
    /// `pty.write` sends bytes to its stdin (that is what an `ssh` forwards), and `pty.close` signals its
    /// **process group** there through the pid file rather than only closing the connection.
    ///
    /// It is not a terminal (`tty: false` still - no `-tt`, so no full-screen programs), and the answer
    /// says so rather than leaving a caller to discover it.
    fn pty_open(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
        /*
         * Two ways to say what to run, and the difference is who owns the line (0.7.13):
         *
         *  * `command` + `args` - a program SDC already knows (`cli.login` starts a CLI this way);
         *  * `line` - a whole command as a person typed it, which is what the Terminal's `Run in
         *    background` sends. **Here** the local platform's shell runs it (`sh -c` / `cmd /C`), and on
         *    a host the *host's* shell does, because someone typing about a VPS means that machine's
         *    shell. The line is guarded by `denied_reason_line` before anything is started.
         */
        let line = envelope.opt_str("line");
        let (command, args) = match line.as_deref() {
            Some(line) => {
                if let Some(reason) = crate::pty::denied_reason_line(line) {
                    return Err(ErrorObject::permission_denied(format!("{line}: {reason}")));
                }

                crate::pty::shell_for_line(line)
            }
            None => (envelope.require_str("command")?, args),
        };
        let display = line.clone().unwrap_or_else(|| command.clone());

        let Some(ssh) = self.remote_for(envelope)? else {
            return match line {
                /* `command`/`args` already name the line's shell here; `display` is what a UI shows. */
                Some(_) => self.state.pty.open(&command, &args, cwd.as_deref(), Some(&display), None),
                None => self.state.pty.open(&command, &args, cwd.as_deref(), None, None),
            };
        };

        /* The same deny list the local runner applies: a long-running process on somebody's server is
           exactly where a `shutdown` would be worst. */
        if let Some(reason) = crate::pty::denied_reason(&command, &args) {
            return Err(ErrorObject::permission_denied(format!("{command}: {reason}")));
        }

        let pid_file = crate::ssh::ops::pid_file(&format!("pty-{}", self.state.events.seq() + 1));
        let remote_line = match line.as_deref() {
            Some(raw) => crate::ssh::ops::process_raw_line(raw, cwd.as_deref(), &pid_file)?,
            None => crate::ssh::ops::process_line(&command, &args, cwd.as_deref(), &pid_file)?,
        };
        let mut ssh_args = ssh.base_args()?;

        ssh_args.push(remote_line);

        let label = format!("{display} on {}", ssh.label());
        let opened = self.state.pty.open("ssh", &ssh_args, None, Some(&label), Some((ssh, pid_file)))?;

        Ok(json!({ "ptyId": opened["ptyId"], "command": display, "tty": false, "hostId": self.host_id_for(envelope)? }))
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
        /*
         * A whole command **line** instead of a program plus arguments (0.7.13).
         *
         * This is what a terminal surface means: a person types `git status -s`, not a program and six
         * arguments, and splitting their line in the window would be guesswork about quoting. So the
         * platform's own shell runs it - `sh -c` here, `cmd /C` on Windows, and on a host `ops::shell`
         * sends the very same `sh -c <line>` with the folder prepended.
         *
         * The line goes through `pty::denied_reason_line` first, which checks every *statement* of it and
         * not only its first word, and the checkpoint and the tool-call pair below are unchanged: a
         * command typed into SDC is a step in the chat, exactly like a command the engine ran.
         */
        let line = envelope.opt_str("line");
        let (command, args) = match line.as_deref() {
            Some(line) => {
                if let Some(reason) = crate::pty::denied_reason_line(line) {
                    return Err(ErrorObject::permission_denied(format!("{line}: {reason}")));
                }

                crate::pty::shell_for_line(line)
            }
            None => (envelope.require_str("command")?, args),
        };
        /* What the checkpoint title, the tool call and the log say: the line as typed, or the program. */
        let display = line.unwrap_or_else(|| command.clone());
        let cwd = envelope.opt_str("cwd");
        let session_id = envelope.opt_str("sessionId");
        let turn_id = envelope.opt_str("turnId");
        let timeout = Duration::from_secs(
            envelope.opt_i64("timeoutMs").map(|ms| (ms as u64 / 1000).max(1)).unwrap_or(120),
        );
        let call_id = format!("shell-{}", self.state.events.seq() + 1);

        /*
         * A `shell.run` for a chat whose folder is on a host: the command runs **there**, in that
         * folder, with the same deny list (`pty::denied_reason`, inside `ops::shell`) - a `shutdown`
         * that is refused on this laptop must not be allowed to reach a VPS. The tool-call pair still
         * lands in the log, because an engine's `run` step is a step in the conversation wherever the
         * process ran - and so does the checkpoint, whose shadow commit is on that host (0.7.13).
         */
        if let Some(ssh) = self.remote_for(envelope)? {
            if let Some(session) = session_id.as_deref() {
                let subject = self.subject(envelope)?;

                if let Ok(fresh) = crate::checkpoints::create(
                    self.store(),
                    session,
                    self.state.events.seq(),
                    &format!("Before `{display}`"),
                    subject.snapshot(),
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
                    event::tool_call_started(turn, &call_id, "run", &display, cwd.as_deref().unwrap_or(".")),
                    session_id.clone(),
                    turn_id.clone(),
                );
            }

            let result = crate::ssh::ops::shell(&ssh, &command, &args, cwd.as_deref(), timeout)?;

            if let Some(turn) = turn_id.as_deref() {
                let status = if result["ok"] == Value::Bool(true) { "done" } else { "failed" };
                let meta = format!("exit {} · {}ms", result["exitCode"], result["durationMs"].as_u64().unwrap_or(0));

                out.push(
                    event::tool_call_completed(turn, &call_id, status, &meta, None),
                    session_id.clone(),
                    turn_id.clone(),
                );
            }

            return Ok(result);
        }

        if let Some(session) = session_id.as_deref() {
            let ordinal = self.state.events.seq();
            /* The same subject a file save uses: the folder's own machine, so a `shell.run` on a host
               checkpoints the host's files (0.7.13). A command with no session has nothing to checkpoint
               against - see the note on `fs.write`. */
            let subject = self.subject(envelope)?;

            if let Ok(fresh) = crate::checkpoints::create(
                self.store(),
                session,
                ordinal,
                &format!("Before `{display}`"),
                subject.snapshot(),
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
                event::tool_call_started(turn, &call_id, "run", &display, cwd.as_deref().unwrap_or(".")),
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

    fn permission_resolve(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let permission_id = envelope.require_str("permissionId")?;
        let decision = envelope.opt_str("decision").unwrap_or_else(|| "deny".into());
        /* An agent may be blocked on this question (agent::gate); the answer is what lets it go on. */
        let delivered = crate::agent::gate::resolve(&permission_id, &decision);

        out.push(event::permission_resolved(&permission_id, &decision), None, None);

        Ok(json!({ "decision": decision, "delivered": delivered }))
    }

    fn checkpoint_create(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    fn rewind_apply(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let turn = crate::checkpoints::turn_of(&envelope.require_str("turnId")?);
        /* The session's folder when the caller does not send one: a rewind restores the files of the chat
           it belongs to, which is the folder that chat works in - locally or on a host (0.7.13). */
        let subject = self.subject(envelope)?;
        let applied = crate::rewind::apply(self.store(), &session_id, turn, subject.snapshot())?;

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

    fn rewind_redo(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());

        /* The folder, so a redo puts the files back as well as the list (it only moved rows until v4). */
        let subject = self.subject(envelope)?;

        match crate::rewind::redo(self.store(), &session_id, subject.snapshot())? {
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
        let subject = self.subject(envelope)?;
        let applied = crate::rewind::apply(self.store(), &session_id, turn, subject.snapshot())?;

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
    history: Vec<crate::engines::Message>,
    /// The folder the chat works in, or `None` for a chat that has no project. It reaches the adapters
    /// inside the `Prompt`, which is where every fact about the session that the engine cannot guess
    /// travels - the same reason the model and the provider are there.
    project_root: Option<String>,
    /// The host that folder is on, when it is not this machine (0.7.13). It travels the same way and for
    /// the same reason: `cli.rs` uses it to run the CLI there instead of here.
    remote: Option<crate::ssh::Ssh>,
    /// The engine takes its own checkpoint before its first change (the SDC Agent), so the loop below
    /// must not take a second, late one when it sees the ToolStarted.
    self_checkpointing: bool,
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
        remote: plan.remote.clone(),
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
        /* A stopped turn is over as far as the window is concerned: `engine.cancel` already pushed its
           `TurnCompleted`, and anything the engine still says would reopen it. The stream is still
           drained, so the engine task can finish and be joined below. */
        if crate::engines::cancel::requested(&plan.turn_id) {
            continue;
        }

        match event {
            crate::engines::EngineEvent::Delta(delta) => {
                answer.push_str(&delta);
                out.push(event::turn_delta(&plan.turn_id, &delta), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::Thinking(text) => {
                out.push(event::thinking_delta(&plan.turn_id, &text), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::ToolStarted { call_id, tool, name, target } => {
                if !plan.self_checkpointing && !checkpoint_written && ["edit", "write", "delete", "run"].contains(&tool.as_str()) {
                    let ordinal = state.events.seq();
                    /* The chat's own folder, so a checkpoint written before a mutating tool hashes the
                       files that tool is about to touch. Until 0.7.6 this passed `None`, which meant the
                       checkpoint recorded a conversation and no files at all - and since 0.7.13 the folder
                       may be on a **host**, where the hash is the host's own shadow commit. */
                    let snapshot = match (&plan.remote, plan.project_root.as_deref()) {
                        (Some(ssh), Some(root)) => crate::checkpoints::Snapshot::Remote(ssh, root),
                        (None, Some(root)) => crate::checkpoints::Snapshot::Local(std::path::Path::new(root)),
                        _ => crate::checkpoints::Snapshot::Unbound,
                    };

                    if let Ok(fresh) = crate::checkpoints::create(
                        &state.store,
                        &plan.session_id,
                        ordinal,
                        &format!("Before {name} {target}"),
                        snapshot,
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
            crate::engines::EngineEvent::ToolCompleted { call_id, status, meta, diff } => {
                out.push(
                    event::tool_call_completed(&plan.turn_id, &call_id, &status, &meta, diff),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Permission { permission_id, title, sub, action, target, risk, explain } => {
                /* The agent is blocked on this question until `permission.resolve` answers it (agent::gate). */
                out.push(
                    event::permission_requested(json!({
                        "permissionId": permission_id,
                        "sessionId": plan.session_id,
                        "turnId": plan.turn_id,
                        "title": title,
                        "sub": sub,
                        "action": action,
                        "target": target,
                        "risk": risk,
                        "explain": explain,
                        "checkpointId": Value::Null,
                    })),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Plan(steps) => {
                out.push(event::plan_updated(&plan.turn_id, steps), session.clone(), turn.clone());
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

    let interrupted = crate::engines::cancel::requested(&plan.turn_id);
    let failed = answer.is_empty();
    let state_name = if interrupted { "idle" } else if failed { "error" } else { "success" };
    let summary = if interrupted { "Interrupted" } else { "Done" };

    crate::engines::cancel::clear(&plan.turn_id);

    let _ = state.store.finish_turn(&plan.turn_id, &answer, summary, state_name);
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

/// One checkpoint's or one rewind's subject: the host (when the folder is not here) and the folder.
///
/// It exists because `checkpoints::Snapshot` borrows what it describes and a method handler owns both
/// halves for only part of its body - see `Daemon::subject`.
struct Subject {
    remote: Option<crate::ssh::Ssh>,
    root: Option<std::path::PathBuf>,
    /// The same root as a string, for the remote case (a host's path is a `String`, not a `PathBuf`).
    root_text: String,
}

impl Subject {
    /// The borrowed view the checkpoint and rewind modules take.
    fn snapshot(&self) -> crate::checkpoints::Snapshot<'_> {
        match (&self.remote, &self.root) {
            (Some(ssh), Some(_)) => crate::checkpoints::Snapshot::Remote(ssh, &self.root_text),
            (None, Some(root)) => crate::checkpoints::Snapshot::Local(root),
            _ => crate::checkpoints::Snapshot::Unbound,
        }
    }
}

/// The two steps a host still needs once its key is trusted: the one-time key install, and the probe.
///
/// Shared by `host.add` (a host whose key was already pinned) and `host.trust` (a host a person has
/// just decided about), because the two differ in *when* they get here and in nothing else.
///
/// The password is spent here and only here, and this function is only ever reached with a pinned key:
/// the probe is `ssh` with `BatchMode=yes`, which by design cannot answer a prompt, so the install is
/// what makes every later connection passwordless. `install_key` runs on the daemon's PTY (the one
/// terminal it has) and types the password into `ssh`'s own prompt.
async fn finish_connection(
    state: Arc<DaemonState>,
    notifier: Arc<dyn Notifier>,
    host_id: String,
    name: String,
    ssh: crate::ssh::Ssh,
    password: String,
    key_note: Option<String>,
) {
    let target = ssh.target.user_host.clone();

    if let Some(note) = &key_note {
        notifier.push(
            event::host_status(&host_id, &name, "vps", "connecting", None, Some(note), None),
            None,
            None,
        );
    }

    if !password.is_empty() {
        notifier.push(
            event::host_status(
                &host_id,
                &name,
                "vps",
                "connecting",
                None,
                Some("copying SDC's key with that password…"),
                None,
            ),
            None,
            None,
        );

        let pty = state.pty.clone();
        let install_target = ssh.target.clone();

        let installed = tokio::task::spawn_blocking(move || {
            crate::auth::remote::install_key(&pty, &install_target, &password)
        })
        .await
        .unwrap_or_else(|_| Err(ErrorObject::internal("the key install could not be run")));

        match installed {
            Ok(sentence) => {
                /* The sentence goes on the *status* line rather than into a `Toast`. A toast is written
                   to the event log and replayed on the next launch, so a four-line explanation became
                   four lines of furniture over the Provider Hub every time the app started. The host's
                   own row is where a fact about a host belongs. */
                notifier.push(
                    event::host_status(&host_id, &name, "vps", "connecting", None, Some(&sentence), None),
                    None,
                    None,
                );
            }
            Err(error) => {
                /* The install is the whole reason the password was asked for, so its failure is the
                   host's status - and its sentence is the actionable one. */
                let _ = state.store.upsert_host(&host_id, &name, "ssh", Some(&target), "offline", None);

                notifier.push(
                    event::host_status(&host_id, &name, "vps", "offline", None, Some(&error.message), None),
                    None,
                    None,
                );

                return;
            }
        }
    }

    let probe_target = ssh.clone();

    let (status, detail) = tokio::task::spawn_blocking(move || crate::ssh::ops::probe(&probe_target))
        .await
        .unwrap_or_else(|_| {
            ("offline".to_string(), format!("{target} could not be measured; check the daemon's log"))
        });

    let _ = state.store.upsert_host(&host_id, &name, "ssh", Some(&target), &status, None);

    /* One event, not two. The `HostStatus` carries the sentence and the host's row renders it; the
       `Toast` that used to accompany it was written to the log as well, so a machine that could not be
       reached produced the same four-line paragraph again on every launch. */
    notifier.push(
        event::host_status(&host_id, &name, "vps", &status, None, Some(&detail), None),
        None,
        None,
    );
}

/// [`finish_connection`] on its own task, for the caller that is not already in one (`host.trust`).
fn spawn_finish(
    state: Arc<DaemonState>,
    notifier: Arc<dyn Notifier>,
    host_id: String,
    name: String,
    ssh: crate::ssh::Ssh,
    password: String,
    key_note: Option<String>,
) {
    tokio::spawn(finish_connection(state, notifier, host_id, name, ssh, password, key_note));
}

/// The sentence for a host whose key SDC has never seen: it is a **question**, and it says so.
///
/// This is the layer 0.7.0 did not have. The old probe ran with `accept-new`, so the first key ever
/// seen was trusted for ever and nobody was asked anything. The fingerprint in this sentence is what
/// the dialog puts on screen, and `host.trust` is how it is answered.
fn untrusted_sentence(target: &str, fingerprint: &str, key_note: Option<&str>) -> String {
    let host = crate::ssh::hostkey::host_of(target);
    /*
     * The command to check the fingerprint is the one that **works on that machine**, which is not always
     * `ssh-keyscan`: against OpenSSH 10.2 on Ubuntu, `ssh-keyscan` fails the key exchange while a real
     * `ssh` completes it, so the sentence used to send a person to a command that prints nothing - and the
     * one thing a pin must never do is make its own verification look broken. The handshake form is named
     * first for that reason (it is also what SDC's own scan falls back to), and the `ssh-keyscan` form is
     * offered as the shorter one where it negotiates.
     */
    let port = crate::auth::remote::parse_target(target)
        .ok()
        .and_then(|parsed| parsed.port)
        .unwrap_or(22);

    let base = format!(
        "{target} is reachable, and its host key is {fingerprint} - a key SDC has never seen. Nothing has been sent to it yet: no key was offered and no password was typed. Trust the key to pin it, and SDC finishes the connection. To check that fingerprint in your own terminal, this prints the same string on any machine: `ssh -p {port} -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=check {target} true`, then `ssh-keygen -lf check`. Where `ssh-keyscan` can negotiate with that server, the shorter `ssh-keyscan -p {port} {host} | ssh-keygen -lf -` says the same thing."
    );

    match key_note {
        Some(note) => format!("{base} One thing to fix first: {note}."),
        None => base,
    }
}

/// The sentence for a host presenting a key which is **not** the pinned one - the alarm.
///
/// Both fingerprints are named, because a person has to be able to see which is which, and there is
/// deliberately no "continue anyway" anywhere in this daemon: that is the one thing a man-in-the-middle
/// needs, and a machine whose key changed is a machine something happened to.
fn changed_sentence(target: &str, pinned: &[String], seen: &[crate::ssh::hostkey::HostKey]) -> String {
    format!(
        "{target} presented a host key that is not the one SDC pinned for it. SDC pinned {} and it now presents {}. Nothing was sent to it. If you changed the machine's keys yourself, remove the host and add it again to see and pin the new fingerprint - and if you did not, find out what did.",
        if pinned.is_empty() { "no key".to_string() } else { pinned.join(", ") },
        seen.iter().map(|key| key.fingerprint.clone()).collect::<Vec<_>>().join(", ")
    )
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
///
/// (0.7.13 moved the refusal sentences to `ssh::ops`, which reads `ssh`'s own words itself, so this
/// helper has no caller left here. It is kept only while a later change needs it - deleting it and
/// re-adding the same three lines twice is the alternative.)
#[allow(dead_code)]
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
    /// sentence and leaves them nothing to do. The sentence now lives in `ssh::ops::refusal` (0.7.13),
    /// where the same rules cover the probe, a file read and a git call; this test is the daemon-level
    /// guarantee that the *method* still answers with it rather than with "offline".
    #[test]
    fn a_host_that_wants_a_password_is_not_reported_as_unreachable() {
        let sentence = crate::ssh::ops::refusal(
            "root@vps.example",
            "root@vps.example: Permission denied (keyboard-interactive,publickey).",
        );

        assert!(sentence.contains("answered"), "{sentence}");
        assert!(sentence.contains("password"), "{sentence}");
        assert!(sentence.contains("authorized_keys"), "{sentence}");
        assert!(!sentence.contains("did not answer"), "{sentence}");
    }

    /// Everything else keeps `ssh`'s own first line, because that is what a person can act on.
    #[test]
    fn every_other_refusal_keeps_ssh_own_words() {
        let timed_out = crate::ssh::ops::refusal("h", "\nssh: connect to host h port 22: Connection timed out\nmore\n");

        assert_eq!(timed_out, "h did not answer: ssh: connect to host h port 22: Connection timed out");

        let host_key = crate::ssh::ops::refusal("h", "Host key verification failed.");

        assert!(host_key.contains("host key is not the one SDC pinned"), "{host_key}");
        assert!(host_key.contains("Add the host again"), "{host_key}");
    }

    /// The sentence a `host.add` puts on screen when the machine's key is one SDC has never seen, and
    /// the one it puts there when that key is *wrong*. Two different questions, two different actions.
    #[test]
    fn a_trust_question_and_a_changed_key_are_two_different_sentences() {
        let asked = untrusted_sentence("ssh -p 8443 root@vps.example", "SHA256:abc", None);

        assert!(asked.contains("SHA256:abc"), "{asked}");
        assert!(asked.contains("never seen"), "{asked}");
        assert!(asked.contains("Nothing has been sent"), "{asked}");
        /* The command a person can check the fingerprint with has to be one that **works there**: the
           handshake form first, because `ssh-keyscan` cannot negotiate with every server (it fails on the
           host in the bug report) - and the port from the target, not a guessed 22. */
        assert!(asked.contains("-o StrictHostKeyChecking=accept-new"), "{asked}");
        assert!(asked.contains("ssh-keygen -lf check"), "{asked}");
        assert!(asked.contains("-p 8443"), "{asked}");
        assert!(asked.contains("ssh-keyscan"), "the shorter form is offered too: {asked}");

        let with_note = untrusted_sentence("root@vps.example", "SHA256:abc", Some("ssh-keygen is missing"));
        assert!(with_note.ends_with("One thing to fix first: ssh-keygen is missing."), "{with_note}");

        let alarmed = changed_sentence(
            "root@vps.example",
            &["SHA256:pin".to_string()],
            &[crate::ssh::hostkey::HostKey {
                key_type: "ssh-ed25519".into(),
                base64: "AAAA".into(),
                fingerprint: "SHA256:now".into(),
                line: "root@vps.example ssh-ed25519 AAAA".into(),
            }],
        );

        assert!(alarmed.contains("SHA256:pin"), "{alarmed}");
        assert!(alarmed.contains("SHA256:now"), "{alarmed}");
        assert!(alarmed.contains("Nothing was sent"), "{alarmed}");
        assert!(!alarmed.to_lowercase().contains("continue anyway"), "{alarmed}");
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
            remote: None,
            self_checkpointing: false,
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
