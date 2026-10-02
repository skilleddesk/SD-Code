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

/* The Trust Kernel's, the Intent Engine's and the agency's methods (0.12) - child modules, so they share
   this file's private helpers rather than copying them. */
#[path = "kernel.rs"]
mod kernel;
#[path = "intent_methods.rs"]
mod intent_methods;
#[path = "ops_methods.rs"]
mod ops_methods;
#[path = "agent_methods.rs"]
pub(crate) mod agent_methods;

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
        /* Agency Mode (0.12): the person at the desk has a role, and the daemon - not the window - refuses
           what the role may not do. With no team configured the one person is the owner. */
        let role = crate::ops::team::current_role(self.store());

        /* Switching who is at the desk (`team.set` with only `current`) is open to every role - it is what
           the refusal below tells a person to do, and a role that could not do it was stuck in it for
           good (0.16.1). The members and their roles stay the owner's to change. */
        let switching = envelope.method == "team.set" && envelope.params.get("members").is_none();

        if role != "owner" && !switching {
            crate::ops::team::allowed(&role, &envelope.method).map_err(ErrorObject::permission_denied)?;
        }

        match envelope.method.as_str() {
            /* Host ---------------------------------------------------------------------------- */
            "host.status" => Ok(self.host_status(&*out)),
            "host.doctor" => self.host_doctor(envelope),
            "host.add" => self.host_add(envelope, out),
            "host.trust" => self.host_trust(envelope, out),
            "host.probe" => self.host_probe(envelope, out),
            "host.key" => self.host_key(envelope, &*out),
            "host.password" => self.host_password(envelope),
            "preview.forward" => self.preview_forward(envelope),
            /* A site that forbids framing, shown in the preview through a loopback proxy (0.14.4). */
            "preview.open" => Ok(json!({ "url": crate::preview::open(&envelope.require_str("url")?)? })),
            /* 0.15.4: whether the live site has the page yet, and the project's own dev server as the preview. */
            "preview.status" => Ok(json!({ "status": crate::preview::status(&envelope.require_str("url")?) })),
            "preview.dev" => self.preview_dev(envelope),
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
            "project.scaffold" => self.project_scaffold(envelope),
            "project.locate" => self.project_locate(envelope),
            "project.list" => Ok(json!({ "projects": self.store().projects().map_err(ErrorObject::internal)? })),
            "project.remove" => self.project_remove(envelope, &*out),

            /* Engines ------------------------------------------------------------------------- */
            "engine.start" => self.engine_start(envelope, out),
            "engine.cancel" | "engine.kill" => self.engine_stop(envelope, out),
            /* 0.12.5: words for a turn that is still running - `accepted: false` when it cannot take them. */
            "engine.steer" => {
                let turn_id = envelope.require_str("turnId")?;
                let text = envelope.require_str("text")?;

                Ok(json!({ "accepted": crate::engines::steer::push(&turn_id, &text) }))
            }
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
            "provider.save" => self.provider_save(envelope, out.clone()),
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

            /* The Trust Kernel, then the Intent Engine, then the agency methods - each falls through to the
               next, and the last one answers `unsupported`. */
            _ => self.dispatch_kernel(envelope, out),
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

        let mut raw_target = envelope.opt_str("target").unwrap_or_default().trim().to_string();

        /* A sign-in from a host's card may name the host alone (0.15.2): the window's copy of its address
           can be empty, and the card then did nothing at all - no call, no sentence - which is how "right
           password and code, login hoi nah" looked. The daemon's own row has the address. */
        if raw_target.is_empty() {
            if let Some(id) = envelope.opt_str("hostId") {
                if let Ok(Some((Some(target), port))) = self.store().host_address(&id) {
                    raw_target = match port {
                        Some(port) => format!("{target}:{port}"),
                        None => target,
                    };
                }
            }
        }

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
        /* Exactly as typed (0.16.1): a space at either end is part of a password, and trimming it saved
           a different password than the host has. Only a pasted line break is dropped. */
        let password = exact_password(envelope.opt_str("password").unwrap_or_default());
        /* The host's one-time code, for a host that asks for one (0.8.1). Spent with the password, kept
           nowhere. */
        let code = envelope.opt_str("code").unwrap_or_default().trim().to_string();
        /* 0.14.4: the password can be kept in the OS keychain, when the person ticks "Remember" - then a
           dropped connection asks only for the verification code, which is still typed every time. */
        let remember = envelope.opt_bool("remember");
        let secret_ref = host_secret_ref(&ssh);
        /* With "Stay signed in" kept, nothing has to be typed at all (0.16.0). */
        let kept_key = crate::auth::keychain::get(&crate::ssh::watch::totp_ref(&ssh)).is_some_and(|text| !text.is_empty());
        let (password, remembered) = if password.is_empty() && (!code.is_empty() || kept_key) {
            match crate::auth::keychain::get(&secret_ref).filter(|saved| !saved.is_empty()) {
                Some(saved) => (saved, true),
                None => (password, false),
            }
        } else {
            (password, false)
        };
        /* "Stay signed in" (0.16.0) keeps the password too: signing in again needs both. */
        let stay = Stay::from_envelope(envelope);
        let remember = remember || matches!(stay, Stay::On(_));
        /* Typing it again with the box unticked means "do not keep it". */
        if !password.is_empty() && !remembered && !remember {
            let _ = crate::auth::keychain::delete(&secret_ref);
        }
        let keep = (!remembered && remember && !password.is_empty()).then(|| secret_ref.clone());

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
                let host_id = self.state.fresh_id("h");

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

        /*
         * A sign-in to a host whose key is already pinned goes straight to the sign-in (0.11.7).
         *
         * The report: *"login hote onk time lage"*. The log shows why - 13 s of `checking host key…`
         * before every sign-in, because `ssh-keyscan` fails that host's key exchange and the scan then
         * falls back to a full handshake. The sign-in itself runs with `StrictHostKeyChecking=yes`
         * against the same pins, so for a pinned host the scan decided nothing the sign-in does not
         * decide again - and a changed key still stops the sign-in before a password is offered. The
         * 13 s also mattered for the verification code: an authenticator code lives about 30 s.
         */
        let pinned = crate::ssh::hostkey::known_hosts_path()
            .ok()
            .and_then(|pins| crate::ssh::hostkey::pinned_in(&pins, &ssh.target).ok())
            .is_some_and(|keys| !keys.is_empty());
        let straight = pinned && (!password.is_empty() || !code.is_empty());

        if !straight {
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
        }

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
            if straight {
                finish_connection(state, notifier, host_id, name, ssh, Secrets { password, code, keep: keep.clone(), stay: stay.clone() }, key_note).await;

                return;
            }

            let scan_target = ssh.target.clone();
            let seen = tokio::task::spawn_blocking(move || crate::ssh::hostkey::inspect(&scan_target))
                .await
                .unwrap_or_else(|_| Err(ErrorObject::internal("the host key scan could not be run")));

            match seen {
                Ok(crate::ssh::hostkey::Trust::Pinned(_)) => {
                    finish_connection(state, notifier, host_id, name, ssh, Secrets { password, code, keep: keep.clone(), stay: stay.clone() }, key_note).await;
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
        /* Exactly as typed (0.16.1): a space at either end is part of a password, and trimming it saved
           a different password than the host has. Only a pasted line break is dropped. */
        let password = exact_password(envelope.opt_str("password").unwrap_or_default());
        let code = envelope.opt_str("code").unwrap_or_default().trim().to_string();

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

        let keep = ((envelope.opt_bool("remember") || matches!(Stay::from_envelope(envelope), Stay::On(_))) && !password.is_empty())
            .then(|| host_secret_ref(&ssh));

        spawn_finish(state, notifier, host_id.clone(), name, ssh, Secrets { password, code, keep, stay: Stay::from_envelope(envelope) }, None);

        Ok(json!({ "trusted": true, "hostId": host_id, "fingerprint": fingerprint }))
    }

    /// `host.probe` - measure a host again, and say the result on its own row (0.11.3).
    ///
    /// The window's `Reconnect` button had nothing behind it: it was a `toast` saying `Reconnected`
    /// while nothing was measured, so a machine that had come back - a VPS rebooted, a route repaired, a
    /// laptop reopened - stayed `offline` in the sidebar until the whole app was relaunched and
    /// `host.add` ran again. This is the daemon's half of that button.
    ///
    /// It is deliberately the **probe** and not the whole [`finish_connection`]: a reconnect has no
    /// password to spend and no key to install (the key is already in the host's `authorized_keys`), and
    /// a button that asked for one would be a sign-in card wearing a different label.
    ///
    /// Three answers, decided in this order:
    ///
    /// * `local` - the daemon's own machine, `connected` without dialing anything;
    /// * a row with no address - a bad request, because there is nothing to measure. An *unreachable*
    ///   host is not this case: it has an address, and this probe is what turns a failed dial into the
    ///   sentence its row shows;
    /// * a real address - `connecting` first, so the dot moves while `ssh` dials, then the probe's
    ///   verdict, stored on the row and pushed as the same `HostStatus` the add/trust path pushes.
    ///
    /// The measurement answers as an *event* rather than as this call's result, which is how `host.add`
    /// does it and for the same two reasons: a synchronous probe would hold the dispatcher for as long as
    /// `ssh` takes to time out, and a fact about a host belongs on the host's row, not in a `Toast` that
    /// the next launch replays (the sentence would be replayed about a machine that is fine by then).
    fn host_probe(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let host_id = envelope.require_str("hostId")?;

        let row = self.store().host(&host_id).map_err(ErrorObject::internal)?;
        let name = row.as_ref().and_then(|row| row["name"].as_str().map(str::to_string));
        /*
         * The machine line the row already carries, kept next to the name so the verdict's upsert below
         * writes it back.
         *
         * `upsert_host` sets `platform = ?6` on conflict, so the `None` the add path passes is correct
         * there (the row is new) and wrong here: re-measuring a host that is already on screen would
         * erase the `Debian 12 · x64` line from a row that is merely being looked at again.
         */
        let platform = row.as_ref().and_then(|row| row["platform"].as_str().map(str::to_string));

        if host_id == "local" {
            let name = name.unwrap_or_else(|| "Local".to_string());
            let detail = "this machine is where the daemon runs";

            out.push(
                event::host_status(&host_id, &name, "local", "connected", None, Some(detail), None),
                None,
                None,
            );

            return Ok(json!({ "hostId": host_id, "status": "connected", "detail": detail }));
        }

        let Some(ssh) = self.ssh_for(&host_id)? else {
            return Err(ErrorObject::bad_request(format!(
                "`{host_id}` has no SSH address stored, so there is nothing to measure"
            )));
        };

        let name = name.unwrap_or_else(|| ssh.label());
        let target = ssh.target.user_host.clone();

        out.push(
            event::host_status(
                &host_id,
                &name,
                "vps",
                "connecting",
                None,
                Some(&format!("reconnecting to {}…", ssh.label())),
                None,
            ),
            None,
            None,
        );

        let state = self.state.clone();
        let notifier = out.clone();
        let probing = ssh.clone();
        /* The id the answer needs, taken before the task takes its own copy of everything else. */
        let answer_host_id = host_id.clone();

        tokio::spawn(async move {
            let (status, detail) = tokio::task::spawn_blocking(move || crate::ssh::ops::probe(&probing))
                .await
                .unwrap_or_else(|_| {
                    (
                        "offline".to_string(),
                        format!("{target} could not be measured; check the daemon's log"),
                    )
                });

            let _ = state
                .store
                .upsert_host(&host_id, &name, "ssh", Some(&target), &status, platform.as_deref());

            notifier.push(
                event::host_status(
                    &host_id,
                    &name,
                    "vps",
                    &status,
                    platform.as_deref(),
                    Some(&detail),
                    None,
                ),
                None,
                None,
            );
        });

        Ok(json!({ "hostId": answer_host_id, "status": "connecting" }))
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
    /// `host.password` (0.14.4): whether this host has a remembered password - so the Sign in card can
    /// ask for the verification code alone - and, with `forget: true`, deletes it. The password itself
    /// never leaves the keychain through this or any other method.
    fn host_password(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let host_id = envelope.require_str("hostId")?;

        let Some(ssh) = self.ssh_for(&host_id)? else {
            return Ok(json!({ "saved": false }));
        };
        let entry = host_secret_ref(&ssh);

        let key_entry = crate::ssh::watch::totp_ref(&ssh);

        if envelope.opt_bool("forget") {
            crate::auth::keychain::delete(&entry)?;
            /* Without the password, the kept authenticator key cannot sign in on its own either. */
            let _ = crate::auth::keychain::delete(&key_entry);
            crate::ssh::native::set_totp(&ssh, None);
        }

        let saved = crate::auth::keychain::get(&entry).is_some_and(|saved| !saved.is_empty());
        let stays = saved && crate::auth::keychain::get(&key_entry).is_some_and(|key| !key.is_empty());

        Ok(json!({ "saved": saved, "staysSignedIn": stays }))
    }

    /// `preview.forward` (0.14.4): a dev server's port on the chat's host, as an address this machine can
    /// open - the host's own `localhost:3000` for a chat on a VPS, through the signed-in connection. A
    /// local chat's port is already here and comes back as it is.
    fn preview_forward(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let port = envelope
            .opt_i64("port")
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port > 0)
            .ok_or_else(|| ErrorObject::bad_request("`port` is required (1-65535)"))?;

        let Some(ssh) = self.remote_for(envelope)? else {
            return Ok(json!({ "url": format!("http://localhost:{port}/"), "forwarded": false }));
        };
        let local = crate::ssh::session::forward(&ssh, port)?;

        Ok(json!({ "url": format!("http://127.0.0.1:{local}/"), "forwarded": true, "localPort": local }))
    }

    /// `preview.dev` (0.15.4): starts the chat project's `npm run dev` - on its host for a VPS chat - or
    /// reports on the one already running; `stop: true` ends it. Called again until `state` is `ready`.
    fn preview_dev(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let stop = envelope.opt_bool("stop");
        let root = self.root_required(envelope)?;
        let root = root.to_string_lossy().to_string();
        let remote = self.remote_for(envelope)?;
        let report = match &remote {
            Some(ssh) => {
                let script = crate::preview::dev_script(&crate::ssh::ops::remote_expr(&root)?, stop);
                let ran = ssh.run(&script, std::time::Duration::from_secs(40))?;

                crate::preview::parse_dev(&ran.stdout)
            }
            None => crate::preview::dev_local(std::path::Path::new(&root), stop),
        };

        let Some(dir) = report.dir.clone() else {
            return Err(ErrorObject::bad_request(format!(
                "No package.json with a \"dev\" script in {root} or the folders just inside it, so there is no dev server to start. The preview can still show the live site."
            )));
        };

        if report.stopped {
            return Ok(json!({ "state": "stopped", "dir": dir }));
        }

        let Some(port) = report.port else {
            return Ok(json!({ "state": "failed", "dir": dir, "log": report.log }));
        };

        if !report.alive && !report.answering {
            return Ok(json!({ "state": "failed", "dir": dir, "port": port, "log": report.log }));
        }

        if !report.answering {
            return Ok(json!({ "state": "starting", "dir": dir, "port": port, "log": report.log }));
        }

        let url = match &remote {
            Some(ssh) => format!("http://127.0.0.1:{}/", crate::ssh::session::forward(ssh, port)?),
            None => format!("http://localhost:{port}/"),
        };

        Ok(json!({ "state": "ready", "dir": dir, "port": port, "url": url, "log": report.log }))
    }

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
        /* A removed host keeps no signed-in connection behind it. */
        if let Some(ssh) = self.ssh_for(&host_id)? {
            crate::ssh::session::close(&ssh);
            let _ = crate::auth::keychain::delete(&host_secret_ref(&ssh));
        }

        let sessions = self.store().delete_host(&host_id).map_err(ErrorObject::internal)?;

        out.push(event::host_removed(&host_id, &name, sessions), None, None);

        Ok(json!({ "removed": true, "name": name, "sessions": sessions }))
    }

    fn session_open(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    /// A workspace for a chat that has none - the folder an agent turn gets instead of a refusal.
    ///
    /// The folder is `~/SDC Workspaces/<title>-<session id>` on the machine the chat lives on (the
    /// session id keeps two chats called "New chat" out of each other's files), created if it is not
    /// there, added as a project (reusing the row when it already is one) and bound to the session.
    /// The `SessionUpdated` at the end is what moves the window's folder chip, so the person sees
    /// where the agent is working the moment the turn starts.
    fn provision_workspace(
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

    fn session_update(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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
    fn project_scaffold(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
        /* A model picked from the Local (Ollama) group arrives as `native_api` with `provider: "ollama"` -
           the window has four engines, and Ollama is a provider of the API one. Routed as an API turn it
           skipped Ollama (`endpoint_for` never sends a key-less turn there) and failed on `custom`'s
           missing key (0.16.1). It is the local engine's turn. */
        let engine_id = if provider.as_deref() == Some("ollama") { "ollama".to_string() } else { engine_id };
        let turn_id = self.state.fresh_id("turn-");
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
        /* `/compact` (0.13): the chat's own model writes the summary later turns start from - as a plain
           answer, never with tools. */
        let compact = envelope.params.get("compact").and_then(Value::as_bool).unwrap_or(false);
        /* `/research` (0.16.1): an agent turn with the research brief, tools, limits and sources. */
        let research = envelope.params.get("research").and_then(Value::as_bool).unwrap_or(false) && !compact;
        let agent_mode = (envelope.params.get("agent").and_then(Value::as_bool).unwrap_or(false) || research) && !compact;
        let requested_autonomy = crate::agent::gate::Autonomy::parse(&envelope.opt_str("autonomy").unwrap_or_default());

        /*
         * The Trust Kernel decides before anything is recorded (0.12): the folder's policy (privacy, the
         * production cap on Auto), and the cost governor (a budget already spent, or a turn estimated far
         * past its cap). A refusal here is a sentence and no turn - never a turn that starts and is then
         * cut off.
         */
        let known_root = self.store().session_project_root(&session_id).map_err(ErrorObject::internal)?;
        let early_remote = self.remote_for(envelope)?;
        let policy = crate::trust::policy::Policy::load(known_root.as_deref(), early_remote.as_ref());

        policy
            .allows_engine(&engine_id, provider.as_deref())
            .map_err(ErrorObject::permission_denied)?;

        let (autonomy, capped) = policy.cap_autonomy(requested_autonomy);
        /* What the conversation is sent as (0.13, `context`): fitted to this model's window - a `/compact`
           summary, a digest of the oldest turns, the newest verbatim - instead of the whole chat every time. */
        let window_tokens = crate::context::window_tokens(&engine_id, provider.as_deref(), &model);
        let history_budget = crate::context::history_budget(window_tokens);
        let fitted = crate::context::history(self.store(), &session_id, "", history_budget);
        let history_chars: usize = fitted.messages.iter().map(|message| message.text.len()).sum();
        let estimate = crate::trust::cost::estimate(&engine_id, provider.as_deref(), &model, &prompt_text, history_chars, agent_mode);
        let estimate_usd = estimate["usd"].as_f64();

        crate::trust::cost::check_before(self.store(), &session_id, policy.max_turn_usd, estimate_usd)
            .map_err(ErrorObject::permission_denied)?;

        if let Some(sentence) = capped.as_deref().or(policy.error.as_deref()) {
            out.push(event::toast(sentence, None, Some(8_000)), Some(session_id.clone()), None);
        }

        let backend = match engine_id.as_str() {
            "native_api" => Some(crate::agent::Backend::Api),
            "ollama" => Some(crate::agent::Backend::Ollama),
            _ => None,
        };
        let agent = backend.filter(|_| agent_mode).map(|backend| {
            crate::agent::SdcAgent::new(
                backend,
                autonomy,
                envelope
                    .opt_i64("maxSteps")
                    .map(|steps| steps.max(1) as usize)
                    .unwrap_or(crate::agent::DEFAULT_STEPS),
            )
            .with_policy(policy.clone())
            /* Settings → Agent → "Check the work when it says it is done" (0.13), on unless turned off. */
            .with_auto_check(!matches!(self.store().setting("agent.autoCheck").ok().flatten().as_deref(), Some("false" | "off")))
        })
        .map(|agent| if research { agent.with_research(crate::agent::research::config().limits) } else { agent });

        let history = fitted.messages.clone();

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

        /* How the message is read (0.11.8, `understand`): the chip on the person's message, and the brief
           the engine gets in front of it. `understand: false` sends the text exactly as typed. */
        let understand = envelope.params.get("understand").and_then(Value::as_bool).unwrap_or(true);
        let reading = crate::understand::Reading::of(&prompt_text);
        let briefed = understand && crate::understand::wants_brief(&prompt_text, &reading);
        let mut started = event::turn_started(&turn_id, &session_id, &engine_id, &model, &tier, &prompt_text);

        if briefed {
            started["reading"] = reading.to_json();
        }

        /* What the turn will probably cost, labelled as an estimate - the window shows it next to the live meter. */
        started["estimate"] = estimate.clone();

        if let Some(intent_id) = envelope.opt_str("intentId") {
            started["intentId"] = json!(intent_id);
        }

        let first_seq = self.state.events.seq();

        out.push(started, Some(session_id.clone()), Some(turn_id.clone()));

        /* The folder this chat works in, resolved *now* from the session's project and carried in the
           plan, so the engine can be started inside it. A chat bound to a folder runs there; a chat with
           none runs wherever the daemon was started, which is what every chat did before 0.7.6. */
        let project_root = self
            .store()
            .session_project_root(&session_id)
            .map_err(ErrorObject::internal)?;
        /* And **which machine** that folder is on (0.7.13): with a host here, the adapter runs the CLI
           over `ssh` in that folder rather than locally (see `engines::cli::remote_command`). */
        let remote = early_remote;
        /* An agent turn in a chat with no folder used to end in a refusal ("Agent mode works inside a
           folder, and this chat has none"). 0.10.0 provisions one instead - a workspace under the
           person's home, on whichever machine the chat lives on, bound to the chat exactly the way
           `Open folder` binds one - so typing is enough. A provisioning that fails falls back to the
           old refusal, whose sentence still says what to do by hand. */
        let project_root = match project_root {
            Some(root) => Some(root),
            None if agent_mode => {
                self.provision_workspace(&session_id, remote.as_ref(), out.as_ref()).ok()
            }
            None => None,
        };

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
                        let _ = store.set_checkpoint_turn(&fresh.id, &turn);

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
        /*
         * The CLI's own conversation (0.13): Claude Code and Codex continue the chat's earlier conversation
         * by id, on the machine and in the folder it ran in, and are sent only the turns they have not seen.
         * Their memory of what they read and ran survives from turn to turn, and a long chat stops being
         * pasted in again at every message.
         */
        let place = remote.as_ref().map(|ssh| ssh.label()).unwrap_or_else(|| "local".to_string());
        let resume = if matches!(engine_id.as_str(), "claude_code" | "codex" | "gemini") {
            crate::context::load_resume(self.store(), &session_id, &engine_id).and_then(|record| {
                let turns = crate::context::live_turns(self.store(), &session_id, &turn_id);
                let missed: Vec<crate::context::TurnRecord> =
                    crate::context::resumable(&record, &place, project_root.as_deref().unwrap_or_default(), &turns)?.into_iter().cloned().collect();

                Some(crate::engines::ResumeRef { id: record.id, unseen: crate::context::fit(&missed, None, history_budget).messages })
            })
        } else {
            None
        };
        let images = self.save_attachments(envelope, &turn_id, remote.as_ref());
        let sent_tokens = match &resume {
            Some(resume) => resume.unseen.iter().map(|message| crate::context::tokens_of(&message.text)).sum::<u64>(),
            None => fitted.tokens,
        } + crate::context::tokens_of(&prompt_text);

        out.push(
            event::context_updated(&session_id, &turn_id, sent_tokens, window_tokens, fitted.compacted, resume.is_some()),
            Some(session_id.clone()),
            Some(turn_id.clone()),
        );

        /* The engine reads the brief and the message; the store and the history keep the message alone. */
        /* A confirmed Intent Contract (0.12) wins over the reading brief: the person already saw and agreed
           to what SDC understood, and the Prompt Compiler writes it the way this engine works best. */
        let compiled = envelope
            .opt_str("intentId")
            .and_then(|intent_id| self.compiled_prompt(&intent_id, &engine_id, envelope).ok());
        let include_memory_file = compiled.is_none();
        let prompt_text = match compiled {
            Some(compiled) => compiled,
            None if briefed => crate::understand::shape(&prompt_text, &reading),
            /* A short follow-up ("ok", "continue") in a chat the person writes in Bengali is still answered
               in Bengali (0.13): the chat's language, not the four words', decides. */
            None => match self.chat_language(&session_id, &turn_id) {
                Some(language) if understand && !prompt_text.trim().starts_with('/') => {
                    format!("{prompt_text}\n\n[SDC: this chat is in {language}; answer in it.]")
                }
                _ => prompt_text,
            },
        };
        /* Long-task memory (0.12): an unfinished plan and the project's .sdc/memory.md travel in front of the
           words - for this turn only; the stored prompt stays the person's own. */
        /* Continuity (0.12.5): the project's conventions, and - when this model is not the one that wrote
           the earlier turns - the hand-over, so a second model follows the first one's shape and style.
           On a host both are a round trip each, so they are asked at the same time (0.15.4). */
        let (memory, brief) = std::thread::scope(|scope| {
            let brief = scope.spawn(|| self.continuity_brief(&session_id, &turn_id, project_root.as_deref(), remote.as_ref(), &engine_id, &model));
            let memory = self.long_task_memory(&session_id, project_root.as_deref(), remote.as_ref(), include_memory_file);

            (memory, brief.join().ok().flatten())
        });
        let prompt_text = match memory {
            Some(memory) => format!("{memory}\n\n{prompt_text}"),
            None => prompt_text,
        };
        let prompt_text = match brief {
            Some(brief) => format!("{brief}\n\n{prompt_text}"),
            None => prompt_text,
        };
        let prompt_text = if compact { crate::context::COMPACT_PROMPT.to_string() } else { prompt_text };
        /* A CLI has its own web tools and loop; `/research` there is the same brief, in front of the words. */
        let prompt_text = if research && !self_checkpointing {
            format!(
                "{}\n\n[The person's message]\n{prompt_text}",
                crate::agent::research::system_prompt(&reading, &crate::agent::research::config().limits, "your own web search", false)
                    .replace("Do not list the sources yourself - SDC adds the numbered list under your answer.", "End with the numbered list of sources: [n] title - address (date).")
            )
        } else {
            prompt_text
        };
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
            autonomy,
            policy,
            first_seq,
            estimate_usd,
            resume,
            images,
            compact,
            place,
            effort: crate::engines::effort_of(envelope.opt_str("effort").as_deref()),
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
        let verify_id = self.state.fresh_id("verify-");
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
            /* The conditions the person agreed to: sent by the window, or read from the turn's Intent Contract. */
            acceptance: {
                let sent: Vec<String> = envelope
                    .params
                    .get("acceptance")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();

                if sent.is_empty() {
                    envelope
                        .opt_str("intentId")
                        .and_then(|id| self.store().intent(&id).ok().flatten())
                        .map(|intent| {
                            intent["spec"]["acceptance"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default()
                                .iter()
                                .filter(|item| item["checked"].as_bool().unwrap_or(true))
                                .filter_map(|item| item["text"].as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    sent
                }
            },
            focus: envelope.opt_str("focus"),
        };
        let state = self.state.clone();

        tokio::spawn(async move {
            crate::verify::run(state, out, request).await;
        });

        Ok(json!({ "verifyId": verify_id }))
    }

    fn engine_stop(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
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
            let notifier = out.clone();
            let session = self.store().turn_session(&turn_id).ok().flatten();

            tokio::spawn(async move {
                engine.cancel(&turn).await;

                /* A stop sent to a host goes on a thread of its own; give it the time its two tries take,
                   then say so if it did not arrive. */
                for _ in 0..40 {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                    if let Some(reason) = crate::engines::cancel::take_remote_failure(&turn) {
                        notifier.push(
                            event::toast(
                                &format!("The stop did not reach the host ({reason}). The command may still be running there - check it from the Terminal, or press Stop again."),
                                None,
                                Some(12_000),
                            ),
                            session,
                            None,
                        );

                        return;
                    }
                }
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

    fn provider_save(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let asked = envelope.require_str("id")?;
        let kind = envelope.opt_str("kind").unwrap_or_else(|| "api-key".into());
        let key = envelope.opt_str("key");
        let mut id = asked.clone();
        let mut url = envelope.opt_str("url");

        /* 0.14.4: the provider judges the key before it is saved, and an Alibaba key goes to the card and
           region that accept it (`providers::place_key`). */
        if kind == "api-key" {
            if let Some(key) = key.as_deref().filter(|key| !key.trim().is_empty()) {
                match providers::place_key(&asked, key, url.as_deref()) {
                    providers::Placement::Rejected(reason) => return Err(ErrorObject::bad_request(reason)),
                    providers::Placement::Home { id: home, base } => {
                        match base {
                            providers::Base::At(base) => url = Some(base),
                            providers::Base::BuiltIn => {
                                crate::providers::models::set_endpoint_override(&home, None).map_err(ErrorObject::internal)?;
                                url = None;
                            }
                            providers::Base::Keep => {}
                        }

                        id = home;
                    }
                    providers::Placement::Unchecked => {}
                }
            }
        }

        let mut saved = providers::save(
            self.store(),
            &id,
            &kind,
            key.as_deref(),
            envelope.opt_str("label").as_deref(),
            url.as_deref(),
            envelope.opt_str("protocol").as_deref(),
        )?;

        if id != asked {
            saved["movedFrom"] = json!(asked);
        }

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

        /* A key that was just saved is a provider that can be asked now: its live list lands in the
           cache and every window hears about it - the moment "connect korlei model gula chole ashe". */
        crate::providers::models::refresh_and_tell(self.state.store.clone(), out.clone());

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

            return Ok(json!({
                "hits": crate::ssh::ops::search(&ssh, &root, &query, glob.as_deref(), limit)?,
                /* Names too (0.11.0): the sidebar's search finds `index.php` by its name, not only
                   the lines inside it. */
                "files": crate::ssh::ops::find_names(&ssh, &root, &query, limit)?,
            }));
        }

        let root = root.unwrap_or_else(|| std::path::PathBuf::from("."));

        Ok(json!({
            "hits": crate::fs::search(&root, &query, glob.as_deref(), limit)?,
            "files": crate::fs::find_names(&root, &query, limit)?,
        }))
    }

    /// `project.locate` (0.11.0): where a named thing - a domain, a project - lives on a machine.
    ///
    /// This is the daemon half of *"ami skilleddesk.com er project file access chai"*: the window
    /// routes the prompt to the host, and this answers **which folder** on it holds the site, so the
    /// chat can be bound there without anyone browsing for it. On a host the web server's config is
    /// read first (`ssh::ops::locate_project`); locally the usual code folders are checked.
    fn project_locate(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    /// is `cd <cwd> && sh -c 'echo $$ > <pid>; setsid … <command> …'`, which makes the far side of
    /// this method behave exactly like the near side: `pty.output` reads the process's output tail,
    /// `pty.write` sends bytes to its stdin (that is what an `ssh` forwards), and `pty.close` signals its
    /// **process group** there through the pid file rather than only closing the connection.
    ///
    /// It is not a terminal (`tty: false` still - no `-tt`, so no full-screen programs), and the answer
    /// says so rather than leaving a caller to discover it.
    fn pty_open(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        if envelope.opt_bool("shell") {
            return self.pty_open_shell(envelope);
        }

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

        let pid_file = crate::ssh::ops::pid_file(&self.state.fresh_id("pty-"));
        let remote_line = match line.as_deref() {
            Some(raw) => crate::ssh::ops::process_raw_line(raw, cwd.as_deref(), &pid_file)?,
            None => crate::ssh::ops::process_line(&command, &args, cwd.as_deref(), &pid_file)?,
        };
        /* The same connection every other call uses, so the signed-in connection carries this one too. */
        let (program, mut ssh_args) = ssh.launcher(None)?;

        ssh_args.push(remote_line);

        let label = format!("{display} on {}", ssh.label());
        let opened = self.state.pty.open(&program, &ssh_args, None, Some(&label), Some((ssh, pid_file)))?;

        Ok(json!({ "ptyId": opened["ptyId"], "command": display, "tty": false, "hostId": self.host_id_for(envelope)? }))
    }

    /// `pty.open` with `shell: true` - the Terminal's interactive shell (0.11.7).
    ///
    /// The report: *"aikhane terminal nai"*. The Terminal tab ran one command at a time, so a prompt, a
    /// `cd` that stays, `top`, `nano`, a REPL - everything a person opens a terminal for - was out of
    /// reach. On a host this is `ssh -tt` through the **same signed-in connection** every other call
    /// uses, so the host allocates a real terminal (prompt, colours, full-screen programs) and nothing is
    /// asked again; here it is the platform's shell on pipes, which the window drives line by line.
    fn pty_open_shell(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let cwd = envelope.opt_str("cwd").filter(|cwd| !cwd.trim().is_empty());
        let cols = envelope.params.get("cols").and_then(Value::as_u64).unwrap_or(100).clamp(20, 400);
        let rows = envelope.params.get("rows").and_then(Value::as_u64).unwrap_or(30).clamp(5, 200);

        let Some(ssh) = self.remote_for(envelope)? else {
            let (command, args): (String, Vec<String>) = if cfg!(windows) {
                ("powershell.exe".to_string(), vec!["-NoLogo".to_string(), "-NoProfile".to_string(), "-Command".to_string(), "-".to_string()])
            } else {
                ("sh".to_string(), vec!["-i".to_string()])
            };
            let cwd = cwd.filter(|cwd| std::path::Path::new(cwd).is_dir());
            let opened = self.state.pty.open(&command, &args, cwd.as_deref(), Some("shell on this computer"), None)?;

            return Ok(json!({ "ptyId": opened["ptyId"], "command": "shell", "tty": false, "hostId": Value::Null }));
        };

        let mut script = format!("stty cols {cols} rows {rows} 2>/dev/null; export TERM=xterm-256color COLORTERM=truecolor; ");

        if let Some(cwd) = cwd.as_deref() {
            script.push_str(&format!("cd {} 2>/dev/null; ", crate::ssh::ops::remote_expr(cwd)?));
        }

        script.push_str("exec \"${SHELL:-/bin/sh}\" -l");

        let (program, mut ssh_args) = ssh.launcher(Some((cols, rows)))?;

        ssh_args.push(script);

        let label = format!("shell on {}", ssh.label());
        let opened = self.state.pty.open(&program, &ssh_args, None, Some(&label), None)?;

        Ok(json!({ "ptyId": opened["ptyId"], "command": "shell", "tty": true, "hostId": self.host_id_for(envelope)? }))
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
        let since = envelope.params.get("since").and_then(Value::as_u64);

        self.state.pty.output_since(&envelope.require_str("ptyId")?, since)
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
        let provider_id = _envelope.opt_str("providerId");

        /* `remove: true` takes the model out of use (0.12.4); without it, `Use` adds it and chooses it. */
        if _envelope.params.get("remove").and_then(Value::as_bool).unwrap_or(false) {
            return crate::providers::models::release(self.store(), &model_id, provider_id.as_deref());
        }

        crate::providers::models::select(self.store(), &model_id, provider_id.as_deref())
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

                /* On a host the host's shell runs it (0.11.9). This took the *local* platform's shell, so on
                   Windows a VPS was sent `cmd /C <line>` and answered `cmd: command not found` - every
                   line typed into the Terminal's Commands for a VPS chat failed. And a *local* chat's session
                   carries `hostId: "local"`, which `host_id_for` returns as-is - so asking it sent every local
                   line to `sh -c`, which Windows does not have (0.15.10). Only a real remote counts. */
                if self.remote_for(envelope)?.is_some() {
                    ("sh".to_string(), vec!["-c".to_string(), line.to_string()])
                } else {
                    crate::pty::shell_for_line(line)
                }
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
        let call_id = self.state.fresh_id("shell-");

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

    fn rewind_redo(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

        self.refuse_while_running(&session_id)?;

        let subject = self.subject(envelope)?;
        let host = self.host_id_for(envelope)?;
        let applied = crate::rewind::apply(self.store(), &session_id, turn, subject.snapshot(), host.as_deref())?;

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
#[derive(Default)]
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
    /// The autonomy the person chose, for the engines that run their own agent (the CLIs).
    autonomy: crate::agent::gate::Autonomy,
    /// The folder's policy, loaded when the turn started (0.12, the Trust Kernel).
    policy: crate::trust::policy::Policy,
    /// The log's sequence number just before the turn, so its score reads only its own events.
    first_seq: i64,
    /// What the turn was estimated to cost before it ran - kept to compare with what it did cost.
    estimate_usd: Option<f64>,
    /// The CLI conversation this turn continues (0.13).
    resume: Option<crate::engines::ResumeRef>,
    /// The images attached to the turn, saved where the engine can open them (0.13).
    images: Vec<crate::engines::Attachment>,
    /// A `/compact` turn: its answer becomes the chat's summary (0.13).
    compact: bool,
    /// `local` or the host - where a CLI conversation this turn starts lives.
    place: String,
    /// How hard the model should think (0.14).
    effort: Option<String>,
}

/// How long streamed text is gathered before it is pushed as one delta (0.11.7).
const DELTA_WINDOW: std::time::Duration = std::time::Duration::from_millis(50);

/// The gathering window on a slow link (0.12, Settings → Low-bandwidth mode): five times fewer pushes.
const LOW_BANDWIDTH_WINDOW: std::time::Duration = std::time::Duration::from_millis(250);

/// Streamed text that has not been pushed yet: at most one of the two is non-empty at a time.
struct Pending {
    answer: String,
    thinking: String,
    since: Option<std::time::Instant>,
    window: std::time::Duration,
}

impl Default for Pending {
    fn default() -> Self {
        Self { answer: String::new(), thinking: String::new(), since: None, window: DELTA_WINDOW }
    }
}

impl Pending {
    /// Old enough, or big enough, to go now.
    fn due(&self) -> bool {
        self.since.is_some_and(|since| since.elapsed() >= self.window) || self.answer.len() + self.thinking.len() >= 4096
    }

    fn flush(&mut self, out: &dyn Notifier, turn_id: &str, session: &Option<String>, turn: &Option<String>) {
        if !self.thinking.is_empty() {
            out.push(event::thinking_delta(turn_id, &std::mem::take(&mut self.thinking)), session.clone(), turn.clone());
        }

        if !self.answer.is_empty() {
            out.push(event::turn_delta(turn_id, &std::mem::take(&mut self.answer)), session.clone(), turn.clone());
        }

        self.since = None;
    }
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
        autonomy: plan.autonomy,
        resume: plan.resume.clone(),
        images: plan.images.clone(),
        effort: plan.effort.clone(),
    };
    let mut answer = String::new();
    let mut checkpoint_written = false;
    /* What the turn did with its tools, one line per call - kept with the answer for the next turn. */
    let mut tools: Vec<(String, String)> = Vec::new();
    /* The Trust Kernel's view of the turn as it runs (0.12): what it cost so far, the files it changed,
       the shapes of a runaway, and whether the kernel already stopped it. */
    let mut usage = crate::trust::cost::Usage::default();
    let mut changed: Vec<String> = Vec::new();
    let mut runaway = crate::trust::cost::Runaway::default();
    let mut halted = false;
    /* The CLI's conversation id, when it said one (0.13): the next turn resumes it. */
    let mut conversation: Option<String> = None;
    let mut completed: Option<Value> = None;
    /* Why the turn ended early, when it did - told to the next turn so it continues instead of starting over. */
    let mut unfinished: Option<String> = None;
    let turn_cap = crate::trust::cost::turn_cap(&state.store, plan.policy.max_turn_usd);
    let stopper = engine.clone();

    crate::trust::kill::begin(&plan.turn_id, "turn", Some(&plan.session_id), &plan.prompt_text.chars().take(80).collect::<String>());

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

    /*
     * Deltas are gathered for up to [`DELTA_WINDOW`] and pushed as one (0.11.7).
     *
     * An engine streams a token - often a single Bengali grapheme - at a time, and each one used to be
     * its own event: one turn in the report's log was 14,000 `TurnDelta`s, every one written to the
     * database, sent over the bridge and folded by the window, which re-drew the whole answer each time.
     * Fifty milliseconds is below what reads as a pause, so the stream looks exactly as live, at a
     * fraction of the events. Order is kept: the other kind of delta, and every non-delta event, flushes
     * what is pending first.
     */
    let low_bandwidth = matches!(state.store.setting("net.lowBandwidth").ok().flatten().as_deref(), Some("true" | "on"));
    let mut pending = Pending { window: if low_bandwidth { LOW_BANDWIDTH_WINDOW } else { DELTA_WINDOW }, ..Pending::default() };

    loop {
        let next = match pending.since {
            None => stream.recv().await,
            Some(since) => match tokio::time::timeout(pending.window.saturating_sub(since.elapsed()), stream.recv()).await {
                Ok(next) => next,
                Err(_) => {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                    continue;
                }
            },
        };

        let Some(event) = next else {
            break;
        };

        /* A stopped turn is over as far as the window is concerned: `engine.cancel` already pushed its
           `TurnCompleted`, and anything the engine still says would reopen it. The stream is still
           drained, so the engine task can finish and be joined below. */
        if crate::engines::cancel::requested(&plan.turn_id) {
            pending = Pending::default();
            continue;
        }

        match &event {
            crate::engines::EngineEvent::Delta(_) => {
                if !pending.thinking.is_empty() {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                }
            }
            crate::engines::EngineEvent::Thinking(_) => {
                if !pending.answer.is_empty() {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                }
            }
            _ => pending.flush(&*out, &plan.turn_id, &session, &turn),
        }

        match event {
            crate::engines::EngineEvent::Delta(delta) => {
                answer.push_str(&delta);
                pending.answer.push_str(&delta);
                pending.since.get_or_insert_with(std::time::Instant::now);

                if pending.due() {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                }
            }
            /* A slow link keeps the answer and drops the running commentary. */
            crate::engines::EngineEvent::Thinking(_) if low_bandwidth => {}
            crate::engines::EngineEvent::Thinking(text) => {
                pending.thinking.push_str(&text);
                pending.since.get_or_insert_with(std::time::Instant::now);

                if pending.due() {
                    pending.flush(&*out, &plan.turn_id, &session, &turn);
                }
            }
            crate::engines::EngineEvent::Usage { input_tokens, output_tokens, cost_usd } => {
                usage.merge(crate::trust::cost::Usage { input_tokens, output_tokens, cost_usd });

                /* A turn that crosses its budget while running is stopped here, whichever engine runs it. */
                if let Some(cap) = turn_cap {
                    let (spent, _) = crate::trust::cost::settle(&plan.engine_id, plan.provider.as_deref(), &plan.model, &usage);

                    if spent > cap && !halted {
                        halted = true;
                        halt(
                            &*out,
                            &stopper,
                            &plan,
                            "BudgetStop",
                            "budget",
                            &format!("Stopped at ${spent:.2}: this turn crossed its ${cap:.2} budget. The work so far is kept; raise the budget or continue with a cheaper model."),
                        );
                    }
                }
            }
            /* One card per call: an engine that names the same call twice must not open a second card or
               count twice toward the loop guard. */
            crate::engines::EngineEvent::ToolStarted { call_id, .. } if tools.iter().any(|(id, _)| *id == call_id) => {}
            crate::engines::EngineEvent::ToolStarted { call_id, tool, name, target } => {
                /* The kernel looks first (0.12). A runaway loop stops whichever engine runs it; for the
                   engines that run their own agent (the CLIs) a protected path, a denied command or a
                   turn past its blast radius is stopped the moment the stream shows it - the SDC Agent
                   asks *before* acting, in its own tools, so it is not stopped here. */
                if !halted {
                    if let Some(reason) = runaway.observe(&name, &target) {
                        halted = true;
                        halt(&*out, &stopper, &plan, "BudgetStop", "runaway", &reason);
                    }
                }

                if tool == "edit" && !target.is_empty() && !changed.contains(&target) {
                    changed.push(target.clone());
                }

                if !halted && !plan.self_checkpointing {
                    let root = plan.project_root.as_deref();
                    let verdict = match tool.as_str() {
                        "edit" => plan
                            .policy
                            .protected(&target, root)
                            .map(|pattern| ("protected-path", format!("`{target}` is protected by the policy (`{pattern}`). The turn was stopped before it could go further; the checkpoint before it can put the file back.")))
                            .or_else(|| {
                                (plan.policy.max_files_per_turn > 0 && changed.len() > plan.policy.max_files_per_turn).then(|| {
                                    ("blast-radius", format!("This turn changed {} files, over the policy's limit of {}. It was stopped so you can look at the change before it grows.", changed.len(), plan.policy.max_files_per_turn))
                                })
                            }),
                        "run" => plan
                            .policy
                            .denies(&target)
                            .map(|rule| ("denied-command", format!("`{target}` matches `{rule}`, which the policy denies. The turn was stopped.")) ),
                        _ => None,
                    };

                    if let Some((rule, sentence)) = verdict {
                        halted = true;
                        out.push(
                            event::policy_violation(&plan.session_id, Some(&plan.turn_id), rule, &target, "stopped", &sentence),
                            session.clone(),
                            turn.clone(),
                        );
                        halt(&*out, &stopper, &plan, "", rule, &sentence);
                    }
                }

                /* A command that reaches past the folder cannot be undone by a rewind: the newest checkpoint
                   says so, so the Time Machine never promises more than it can give (TM-6). */
                if tool == "run" && (crate::agent::gate::looks_dangerous(&target) || plan.policy.always_asks(&target).is_some()) {
                    if let Ok(Some(id)) = state.store.mark_irreversible(&plan.session_id, &format!("ran `{}`", target.chars().take(80).collect::<String>())) {
                        if let Ok(Some(row)) = state.store.checkpoint(&id) {
                            out.push(event::checkpoint_updated(&plan.session_id, row), session.clone(), turn.clone());
                        }
                    }
                }

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
                        let _ = state.store.set_checkpoint_turn(&fresh.id, &plan.turn_id);

                        out.push(
                            event::checkpoint_saved(&plan.session_id, fresh.to_event_payload()),
                            session.clone(),
                            turn.clone(),
                        );
                    }

                    checkpoint_written = true;
                }

                tools.push((call_id.clone(), format!("{name} {target}")));

                out.push(
                    event::tool_call_started(&plan.turn_id, &call_id, &tool, &name, &target),
                    session.clone(),
                    turn.clone(),
                );
            }
            /* A draft is the live view of a call still being written (0.14.2): not once its card is open,
               and not on a slow link, where it is commentary. */
            crate::engines::EngineEvent::ToolDraft { .. } if low_bandwidth => {}
            crate::engines::EngineEvent::ToolDraft { call_id, .. } if tools.iter().any(|(id, _)| *id == call_id) => {}
            crate::engines::EngineEvent::ToolDraft { call_id, name, target, chars, preview } => {
                out.push_live(
                    event::tool_call_drafting(&plan.turn_id, &call_id, &name, &target, chars, &preview),
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
                if status == "done" && diff.as_ref().is_some_and(|diff| diff.as_array().is_none_or(|lines| !lines.is_empty())) {
                    runaway.progress();
                }

                if let Some(line) = tools.iter_mut().find(|(id, _)| *id == call_id) {
                    line.1 = format!("{} - {}", line.1, if meta.is_empty() { status.clone() } else { meta.clone() });
                }

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
            crate::engines::EngineEvent::Steered(text) => {
                out.push(event::turn_steered(&plan.turn_id, &text), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::SessionRef(id) => {
                /* Kept the moment the CLI names it, not when the turn ends: the next message can be sent
                   the instant this one completes (a queued prompt is), and it must find the record. */
                if !plan.compact {
                    let turns = crate::context::live_turns(&state.store, &plan.session_id, "").into_iter().map(|turn| turn.id).collect();

                    crate::context::save_resume(
                        &state.store,
                        &plan.session_id,
                        &plan.engine_id,
                        &crate::context::ResumeRecord { id: id.clone(), place: plan.place.clone(), root: plan.project_root.clone().unwrap_or_default(), turns },
                    );
                }

                conversation = Some(id);
            }
            crate::engines::EngineEvent::Question { question_id, question, options } => {
                out.push(
                    event::question_asked(&plan.session_id, &plan.turn_id, &question_id, &question, &options),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Context { used_tokens, window_tokens, compacted } => {
                out.push(
                    event::context_updated(&plan.session_id, &plan.turn_id, used_tokens, window_tokens, compacted, false),
                    session.clone(),
                    turn.clone(),
                );
            }
            crate::engines::EngineEvent::Sources(sources) => {
                out.push(event::research_sources(&plan.turn_id, &plan.session_id, sources), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::Plan(steps) => {
                /* The plan outlives the turn and the daemon (long-task memory, 0.12): the next agent turn in
                   this chat is told where it left off. */
                let _ = state.store.set_setting(&format!("plan.{}", plan.session_id), &steps.to_string());

                out.push(event::plan_updated(&plan.turn_id, steps), session.clone(), turn.clone());
            }
            crate::engines::EngineEvent::Failed(reason) => {
                unfinished = Some(reason.clone());
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
                if summary.starts_with("Cut off") || summary.starts_with("Paused") {
                    unfinished = Some(summary.clone());
                }
                /* Pushed after the answer is stored (0.13, below): the window sends a queued prompt the
                   moment a turn completes, and that turn must already see this one's answer. */
                completed = Some(event::turn_completed(&plan.turn_id, &summary, &meta, pass));
            }
        }
    }

    pending.flush(&*out, &plan.turn_id, &session, &turn);

    /* The task is joined so the turn's own bookkeeping cannot race it: a `finish_turn` before the
    engine's last event was pushed would be a stored answer that is missing its tail. */
    let _ = running.await;

    let interrupted = crate::engines::cancel::requested(&plan.turn_id);
    /* 0.15.4: a turn that failed after it had written something used to be stored as a success. */
    let failed = answer.is_empty() || (unfinished.is_some() && completed.is_none());
    let state_name = if halted { "error" } else if interrupted { "idle" } else if failed { "error" } else { "success" };
    let summary = if halted { "Stopped by SDC" } else if interrupted { "Interrupted" } else { "Done" };

    crate::engines::cancel::clear(&plan.turn_id);
    crate::trust::kill::end(&plan.turn_id);

    /*
     * The stored answer is what the *next* turn is told this one said (session_bridge::history_for), and
     * a turn is more than its last paragraph: without its tool calls the next turn read "I ran the tests,
     * 5 passed" with no trace of a test run, and a model decided it had invented the result and apologised
     * for work it had really done. The window draws the turn from its events, so this record is for the
     * conversation, not for the screen.
     */
    let mut recorded = if tools.is_empty() {
        answer.clone()
    } else {
        let lines: Vec<String> = tools.iter().map(|(_, line)| format!("- {line}")).collect();

        format!("{answer}\n\n[Tool calls in this turn, as SDC recorded them:\n{}]", lines.join("\n"))
    };

    /*
     * 0.15.4: a turn that stopped part-way says so. The person stops a turn (or a provider drops it) and
     * sends "continue" to the same or another model; without this line the next model read a turn that
     * looked finished, and either said the work was done or began the whole task again from step one.
     */
    let why = if halted {
        Some("SDC's Trust Kernel stopped it".to_string())
    } else if interrupted {
        Some("the person stopped it".to_string())
    } else {
        unfinished.as_ref().map(|reason| reason.chars().take(300).collect())
    };

    if let Some(why) = why {
        recorded.push_str(&format!(
            "\n\n[SDC: this turn did NOT finish - {why}. Everything listed above really happened and is on disk. \
             When asked to continue, pick up from the last step above: do not redo finished steps, and do not start the task again.]"
        ));
    }

    let _ = state.store.finish_turn(&plan.turn_id, &recorded, summary, state_name);

    /*
     * The chat's memory after the turn (0.13). A `/compact` answer becomes the summary every later turn
     * starts from, and the CLI conversations end with it - that is the point of compacting. Otherwise a
     * CLI that named its conversation is resumed next time, holding every live turn of the chat so far.
     */
    if plan.compact {
        if state_name == "success" && !answer.trim().is_empty() {
            crate::context::save_compaction(&state.store, &plan.session_id, &plan.turn_id, &answer);
            crate::context::forget_resume(&state.store, &plan.session_id);
        }
    } else if conversation.is_some() {
        /* The record written when the CLI named its conversation already holds this turn. */
    }

    if let Some(done) = completed {
        out.push(done, session.clone(), turn.clone());
    }


    /* The cost governor's record (0.12): measured when the provider said, priced from the catalogue when it
       sent only tokens, and never a made-up number. "Saved" exists only against a baseline and measured tokens. */
    {
        let (cost, source) = crate::trust::cost::settle(&plan.engine_id, plan.provider.as_deref(), &plan.model, &usage);
        let baseline = crate::trust::cost::baseline(&state.store, &usage);
        let site = plan
            .project_root
            .as_deref()
            .and_then(|root| state.store.sites().ok()?.into_iter().find(|site| site["root"].as_str() == Some(root)))
            .and_then(|site| site["id"].as_str().map(str::to_string));

        let _ = state.store.record_usage(
            &plan.turn_id,
            &plan.session_id,
            plan.project_root.as_deref(),
            site.as_deref(),
            &plan.engine_id,
            &plan.model,
            plan.provider.as_deref(),
            usage.input_tokens,
            usage.output_tokens,
            cost,
            source,
            plan.estimate_usd,
            baseline,
        );

        out.push(
            event::cost_updated(
                &plan.session_id,
                &plan.turn_id,
                &plan.engine_id,
                &plan.model,
                usage.input_tokens,
                usage.output_tokens,
                cost,
                source,
                plan.estimate_usd,
                baseline.map(|base| (base - cost).max(0.0)),
            ),
            session.clone(),
            turn.clone(),
        );
    }

    /* The turn's Trust score, from its own events - recomputed when Verify runs for it. */
    score_turn(&state, &*out, &plan.session_id, &plan.turn_id, plan.first_seq, &plan.policy, plan.project_root.as_deref());

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

/// The kernel stops a running turn (0.12): the cancel mark first, so the loop drops whatever the engine
/// still says; the engine told to stop; and the turn closed with the kernel's own sentence, so the window
/// never shows a turn that was stopped as one that finished.
fn halt(out: &dyn Notifier, engine: &Arc<dyn crate::engines::Engine>, plan: &RunPlan, event_kind: &str, kind: &str, sentence: &str) {
    crate::engines::cancel::request(&plan.turn_id);

    let engine = engine.clone();
    let turn_id = plan.turn_id.clone();

    tokio::spawn(async move {
        engine.cancel(&turn_id).await;
    });

    if event_kind == "BudgetStop" {
        out.push(
            event::budget_stop(&plan.session_id, &plan.turn_id, kind, sentence),
            Some(plan.session_id.clone()),
            Some(plan.turn_id.clone()),
        );
    }

    out.push(
        event::turn_completed(&plan.turn_id, "Stopped by SDC", sentence, Some(false)),
        Some(plan.session_id.clone()),
        Some(plan.turn_id.clone()),
    );
}

/// Scores one turn from its own events and pushes `TrustScored`. Called when the turn ends and again when
/// a Verify run for it finishes, which is what lifts an unverified turn's cap.
pub(crate) fn score_turn(
    state: &Arc<DaemonState>,
    out: &dyn Notifier,
    session_id: &str,
    turn_id: &str,
    first_seq: i64,
    policy: &crate::trust::policy::Policy,
    root: Option<&str>,
) {
    let events = state.events.since(first_seq.max(0));
    let facts = crate::trust::score::facts(&events, turn_id, policy, root);
    let (score, level, reasons) = crate::trust::score::turn(&facts, policy.max_files_per_turn);
    let reasons = json!(reasons);

    let _ = state.store.save_trust_score(turn_id, session_id, score, level, &reasons);

    out.push(
        event::trust_scored(session_id, turn_id, score, level, reasons),
        Some(session_id.to_string()),
        Some(turn_id.to_string()),
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
/// What a person typed to sign in to a host: used once by [`finish_connection`], kept nowhere - unless
/// they ticked Remember (0.14.4), and then only the password, only in the OS keychain, and only after
/// the host accepted it.
struct Secrets {
    password: String,
    /// The verification code, for a host that asks for one (0.8.1). Never kept.
    code: String,
    /// The keychain entry the password goes to once the sign-in succeeds.
    keep: Option<String>,
    /// "Stay signed in" (0.16.0).
    stay: Stay,
}

/// "Stay signed in" on the Sign in card (0.16.0): keep the password and the authenticator's key in the
/// OS keychain, so a dropped connection - or a restarted SDC - is signed in again without a person.
#[derive(Clone, Debug, PartialEq)]
enum Stay {
    /// The card did not say; what is kept stays kept.
    Unchanged,
    /// Ticked off: forget the key.
    Off,
    /// Ticked on, with the setup key typed (empty: SDC reads it from `~/.google_authenticator`).
    On(String),
}

impl Stay {
    fn from_envelope(envelope: &Envelope) -> Self {
        match envelope.params.get("staySignedIn") {
            Some(Value::Bool(true)) => Stay::On(envelope.opt_str("totpSecret").unwrap_or_default().trim().to_string()),
            Some(Value::Bool(false)) => Stay::Off,
            _ => Stay::Unchanged,
        }
    }
}

/// The keychain entry a host's remembered password lives in, keyed by `user@host:port` rather than by
/// the host's id, which changes when the host is added again.
fn host_secret_ref(ssh: &crate::ssh::Ssh) -> String {
    format!("sdc.host.{}", ssh.label())
}

async fn finish_connection(
    state: Arc<DaemonState>,
    notifier: Arc<dyn Notifier>,
    host_id: String,
    name: String,
    ssh: crate::ssh::Ssh,
    secrets: Secrets,
    key_note: Option<String>,
) {
    let Secrets { password, code, keep, stay } = secrets;
    let target = ssh.target.user_host.clone();

    if let Some(note) = &key_note {
        notifier.push(
            event::host_status(&host_id, &name, "vps", "connecting", None, Some(note), None),
            None,
            None,
        );
    }

    /*
     * Sign in first (0.8.1): one authenticated connection, kept open, that every later call goes through.
     * This is the only way into a host that has public-key login switched off and asks for a
     * verification code and a password on every session - the host in the report. A machine without an
     * `ssh` that can hold a connection open falls back to the 0.7.13 key install below.
     */
    let mut install_key = !password.is_empty();

    if !password.is_empty() || !code.is_empty() {
        notifier.push(
            event::host_status(
                &host_id,
                &name,
                "vps",
                "connecting",
                None,
                Some(&format!("signing in to {}…", ssh.label())),
                None,
            ),
            None,
            None,
        );

        let signing = ssh.clone();
        let (secret, one_time) = (password.clone(), code.clone());
        /* The authenticator key: the one just typed, or the one "Stay signed in" kept (0.16.0). */
        let totp_text = match &stay {
            Stay::On(text) if !text.is_empty() => Some(text.clone()),
            Stay::Off => None,
            _ => crate::auth::keychain::get(&crate::ssh::watch::totp_ref(&ssh)),
        };
        let totp = totp_text.as_deref().and_then(crate::ssh::totp::decode_secret);

        if matches!(&stay, Stay::On(text) if !text.is_empty()) && totp.is_none() {
            let sentence = "That authenticator setup key is not one: it is the base32 text (letters A-Z and digits 2-7) shown when the authenticator was set up, or the first line of ~/.google_authenticator on the host. Leave the box empty and SDC reads it from the host.";

            notifier.push(event::host_status(&host_id, &name, "vps", "offline", None, Some(sentence), None), None, None);

            return;
        }

        let signed = tokio::task::spawn_blocking(move || crate::ssh::session::sign_in_with(&signing, &secret, &one_time, totp))
            .await
            .unwrap_or_else(|_| Err(crate::ssh::session::SignInError::Failed("the sign-in could not be run".into())));

        let refused = match signed {
            Ok(sentence) => {
                install_key = false;

                if let Some(entry) = &keep {
                    let _ = crate::auth::keychain::set(entry, &password);
                }

                let sentence = match stay_signed_in(&ssh, &stay, totp_text.as_deref()).await {
                    Some(note) => format!("{sentence} · {note}"),
                    None => sentence,
                };

                notifier.push(
                    event::host_status(&host_id, &name, "vps", "connecting", None, Some(&sentence), None),
                    None,
                    None,
                );

                None
            }
            Err(crate::ssh::session::SignInError::Unsupported) => None,
            Err(crate::ssh::session::SignInError::NeedsCode) => Some(format!(
                "{} asks for a verification code. Open Sign in, type the password and the code your authenticator shows right now, and press Sign in.",
                ssh.label()
            )),
            /* The sign-in checks the pin itself; a changed key ends it before anything is offered, and
               it gets the pin's own sentence rather than "refused the password". */
            Err(crate::ssh::session::SignInError::Failed(sentence)) if sentence.contains("Host key verification failed") => {
                Some(crate::ssh::ops::refusal(&ssh.label(), "Host key verification failed."))
            }
            Err(crate::ssh::session::SignInError::Failed(sentence)) => Some(sentence),
        };

        if let Some(sentence) = refused {
            let _ = state.store.upsert_host(&host_id, &name, "ssh", Some(&target), "offline", None);

            notifier.push(
                event::host_status(&host_id, &name, "vps", "offline", None, Some(&sentence), None),
                None,
                None,
            );

            return;
        }
    }

    if install_key {
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

/// After a sign-in, what "Stay signed in" asks for (0.16.0): the authenticator key goes to the keychain - the
/// one typed, or the first line of the host's own `~/.google_authenticator`, read over the connection that
/// was just opened. Off forgets it. The answer is a clause for the host's status line.
async fn stay_signed_in(ssh: &crate::ssh::Ssh, stay: &Stay, typed: Option<&str>) -> Option<String> {
    let entry = crate::ssh::watch::totp_ref(ssh);

    match stay {
        Stay::Unchanged => None,
        Stay::Off => {
            let _ = crate::auth::keychain::delete(&entry);
            crate::ssh::native::set_totp(ssh, None);

            None
        }
        Stay::On(_) => {
            let text = match typed.filter(|text| !text.is_empty()) {
                Some(text) => Some(text.to_string()),
                None => {
                    let reading = ssh.clone();

                    tokio::task::spawn_blocking(move || {
                        reading
                            .run("head -n 1 ~/.google_authenticator 2>/dev/null", crate::ssh::QUICK)
                            .ok()
                            .and_then(|output| crate::ssh::totp::secret_from_file(&output.stdout))
                    })
                    .await
                    .ok()
                    .flatten()
                }
            };

            let Some(secret) = text.as_deref().and_then(crate::ssh::totp::decode_secret) else {
                return Some("Stay signed in is NOT on: SDC found no ~/.google_authenticator on the host. Paste the authenticator's setup key in the Sign in card".to_string());
            };

            if crate::auth::keychain::set(&entry, text.as_deref().unwrap_or_default()).is_err() {
                return Some("Stay signed in is NOT on: this PC's keychain refused the key".to_string());
            }

            crate::ssh::native::set_totp(ssh, Some(secret));

            Some("Stay signed in is on: a dropped connection, or a restarted SDC, signs in again by itself".to_string())
        }
    }
}

/// [`finish_connection`] on its own task, for the caller that is not already in one (`host.trust`).
fn spawn_finish(
    state: Arc<DaemonState>,
    notifier: Arc<dyn Notifier>,
    host_id: String,
    name: String,
    ssh: crate::ssh::Ssh,
    secrets: Secrets,
    key_note: Option<String>,
) {
    tokio::spawn(finish_connection(state, notifier, host_id, name, ssh, secrets, key_note));
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
/// A password as the person typed it. Whitespace-only is no password; otherwise only the line break a
/// paste can bring along is removed - spaces are characters of the password.
fn exact_password(raw: String) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }

    raw.trim_end_matches(['\r', '\n']).to_string()
}

fn folder_name(root: &str) -> String {
    std::path::Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| root.to_string())
}

/// `project.locate` on this machine (0.11.0): the usual places a project sits, checked rather than
/// guessed. `<base>/<query>` exactly first, then one level of each base for a folder whose name
/// contains the query (or the domain's first label - `skilleddesk.com` finds `skilleddesk-site`).
fn locate_local(query: &str) -> Vec<Value> {
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return Vec::new();
    };
    let home = std::path::PathBuf::from(home);
    let bases: Vec<std::path::PathBuf> = ["", "Projects", "projects", "code", "dev", "src", "www", "sites", "htdocs", "SDC Workspaces"]
        .iter()
        .map(|base| if base.is_empty() { home.clone() } else { home.join(base) })
        .collect();
    let label = query.split('.').next().unwrap_or(query).to_lowercase();
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |path: std::path::PathBuf, source: &str, candidates: &mut Vec<Value>| {
        let text = path.to_string_lossy().replace('\\', "/");

        if seen.insert(text.clone()) {
            candidates.push(json!({ "root": text, "source": source }));
        }
    };

    for base in &bases {
        let exact = base.join(query);

        if exact.is_dir() {
            push(exact, "an exact folder name", &mut candidates);
        }
    }

    for base in &bases {
        if candidates.len() >= 6 {
            break;
        }

        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };

        for entry in entries.flatten() {
            if candidates.len() >= 6 {
                break;
            }

            let path = entry.path();
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_lowercase();

            if path.is_dir() && label.len() >= 4 && name.contains(&label) {
                push(path, "a folder named like it", &mut candidates);
            }
        }
    }

    candidates
}

/// The folder name a provisioned workspace gets: the chat's title as a slug, with the session id
/// stapled on so two chats called "New chat" never share files. `Fix the login page` and session
/// `n42` become `fix-the-login-page-n42`; a title with nothing usable in it leaves just `chat-n42`.
fn workspace_name(title: &str, session_id: &str) -> String {
    let mut slug = String::new();

    for character in title.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }

        if slug.len() >= 40 {
            break;
        }
    }

    let slug = slug.trim_matches('-');

    if slug.is_empty() {
        format!("chat-{session_id}")
    } else {
        format!("{slug}-{session_id}")
    }
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

    /// 0.16.1: a password keeps its spaces; only a pasted line break goes, and blank is no password.
    #[test]
    fn a_password_is_kept_as_typed() {
        assert_eq!(exact_password(" pass word ".into()), " pass word ");
        assert_eq!(exact_password("secret\r\n".into()), "secret");
        assert_eq!(exact_password("   ".into()), "");
    }

    /// 0.10.0: an agent turn in a folderless chat gets a provisioned workspace instead of a refusal,
    /// and the folder's name is the chat's, sluggified, with the session id keeping two "New chat"s
    /// apart.
    #[test]
    fn a_workspace_is_named_after_the_chat_and_the_session() {
        assert_eq!(workspace_name("Fix the login page", "n42"), "fix-the-login-page-n42");
        assert_eq!(workspace_name("New chat", "n7"), "new-chat-n7");
        assert_eq!(workspace_name("New chat", "n8"), "new-chat-n8");
        assert_eq!(workspace_name("!!!", "n9"), "chat-n9");
        assert_eq!(workspace_name("", "n10"), "chat-n10");

        /* A very long title stays a folder name, not a path problem. */
        assert!(workspace_name(&"word ".repeat(30), "n11").len() <= 48);
    }

    /// `project.locate` on this machine finds a folder that really exists and never invents one:
    /// the exact name wins, a containing name follows, and an empty home answers with nothing.
    #[test]
    fn locate_local_finds_the_exact_folder_first() {
        let home = std::env::temp_dir().join("sdc-locate-test-home");
        let projects = home.join("Projects");

        std::fs::create_dir_all(projects.join("skilleddesk.com")).unwrap();
        std::fs::create_dir_all(projects.join("skilleddesk-old")).unwrap();

        /* The helper reads the home from the environment, so the test lends it one. */
        let saved = std::env::var_os("USERPROFILE");

        std::env::set_var("USERPROFILE", &home);

        let candidates = locate_local("skilleddesk.com");

        match saved {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }

        assert!(!candidates.is_empty(), "the exact folder exists and must be found");
        assert!(
            candidates[0]["root"].as_str().unwrap().ends_with("skilleddesk.com"),
            "exact name first: {candidates:?}"
        );
        assert_eq!(candidates[0]["source"], json!("an exact folder name"));
        assert!(
            candidates.iter().any(|candidate| candidate["root"].as_str().unwrap().ends_with("skilleddesk-old")),
            "the containing name follows: {candidates:?}"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

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

    /// The delta limiter (0.11.7): gathers streamed text for up to [`DELTA_WINDOW`], or until it grows
    /// past 4096 bytes, whichever comes first - the guard against the report's 14,000-event turn.
    #[test]
    fn the_delta_limiter_is_due_on_size_or_time_and_flushes_only_what_is_not_empty() {
        use crate::sdcp::notifications::RecordingNotifier;

        let mut pending = Pending::default();

        assert!(!pending.due(), "nothing pending is never due");

        pending.answer.push_str("hi");
        pending.since = Some(std::time::Instant::now());

        assert!(!pending.due(), "well under both the window and the size cap");

        pending.answer = "x".repeat(4096);

        assert!(pending.due(), "the size cap trips regardless of how young `since` still is");

        pending.answer.clear();
        pending.thinking.push_str("thinking");
        pending.since = Some(std::time::Instant::now() - DELTA_WINDOW);

        assert!(pending.due(), "the time window alone is enough, with nothing in `answer`");

        let notifier = RecordingNotifier::new();

        pending.flush(&notifier, "turn-1", &None, &None);

        assert_eq!(notifier.kinds(), vec!["ThinkingDelta"], "an empty answer is never pushed as its own delta");
        assert!(pending.thinking.is_empty(), "flush takes what it pushed");
        assert!(pending.since.is_none(), "flush resets the clock so the next byte starts a fresh window");
    }

    /// The size cap is on the *combined* length, not either field alone - and when both kinds have
    /// gathered text, `flush` pushes both, thinking first (an engine's thinking always precedes its
    /// answer within the same window).
    #[test]
    fn the_delta_limiter_caps_the_combined_length_and_flushes_both_kinds_in_order() {
        use crate::sdcp::notifications::RecordingNotifier;

        let mut pending = Pending { answer: "a".repeat(2048), thinking: "t".repeat(2047), ..Pending::default() };

        assert!(!pending.due(), "one byte under the combined cap");

        pending.thinking.push('t');

        assert!(pending.due(), "the combined length alone trips the cap, with neither field at 4096 on its own");

        let notifier = RecordingNotifier::new();

        pending.flush(&notifier, "turn-1", &None, &None);

        assert_eq!(notifier.kinds(), vec!["ThinkingDelta", "TurnDelta"], "thinking is flushed before the answer");
        assert!(pending.answer.is_empty() && pending.thinking.is_empty(), "flush takes both");
    }

    /// A custom window - what low-bandwidth mode sets `Pending::window` to - governs `due`, not the
    /// constant [`DELTA_WINDOW`].
    #[test]
    fn the_delta_limiter_is_due_on_its_own_window_not_the_default() {
        let mut pending = Pending { window: LOW_BANDWIDTH_WINDOW, ..Pending::default() };

        pending.answer.push_str("hi");
        pending.since = Some(std::time::Instant::now() - DELTA_WINDOW);

        assert!(!pending.due(), "past the default window, but the wider low-bandwidth window has not elapsed yet");

        pending.since = Some(std::time::Instant::now() - LOW_BANDWIDTH_WINDOW);

        assert!(pending.due(), "the wider window has now elapsed");
    }

    /// The other half of `the_delta_limiter_is_due_on_size_or_time_and_flushes_only_what_is_not_empty`:
    /// an answer alone is pushed as its own delta with nothing for `thinking`, and a `flush` with
    /// neither field holding anything pushes nothing at all - the no-op the loop falls into on every
    /// tick that finds no pending bytes.
    #[test]
    fn the_delta_limiter_flushes_answer_alone_and_nothing_when_both_are_empty() {
        use crate::sdcp::notifications::RecordingNotifier;

        let mut pending = Pending { answer: "hi".to_string(), ..Pending::default() };

        let notifier = RecordingNotifier::new();

        pending.flush(&notifier, "turn-1", &None, &None);

        assert_eq!(notifier.kinds(), vec!["TurnDelta"], "a thinking-less answer is never pushed as a ThinkingDelta");
        assert_eq!(notifier.streamed_text(), "hi");
        assert!(pending.answer.is_empty(), "flush takes what it pushed");

        let notifier = RecordingNotifier::new();

        pending.flush(&notifier, "turn-1", &None, &None);

        assert!(notifier.kinds().is_empty(), "nothing pending means nothing pushed");
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
            autonomy: Default::default(),
            ..Default::default()
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
            vec!["TurnDelta", "TurnDelta", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"]
        );
    }

    /// The limiter earns its keep here: an engine that sends its deltas back-to-back, with nothing
    /// gating it between them, still reaches the window as **one** `TurnDelta` - the coalescing
    /// `the_delta_limiter_is_due_on_size_or_time_and_flushes_only_what_is_not_empty` checks on `Pending`
    /// alone, this time through `run_turn` itself, where the 14,000-event turn actually happened.
    #[tokio::test]
    async fn rapid_deltas_are_coalesced_into_one_push_by_run_turn() {
        use crate::sdcp::notifications::RecordingNotifier;

        struct Rapid;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Rapid {
            fn id(&self) -> &'static str {
                "rapid"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                for piece in ["Hel", "lo", ", ", "wor", "ld"] {
                    sink.send(crate::engines::EngineEvent::Delta(piece.to_string()));
                }

                sink.send(crate::engines::EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let notifier = Arc::new(RecordingNotifier::new());
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "rapid".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(Rapid);
        let out: Arc<dyn Notifier> = notifier.clone();

        run_turn(state, engine, plan, out).await;

        assert_eq!(
            notifier.kinds(),
            vec!["TurnDelta", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"],
            "five deltas that never waited for the window should still land as a single push"
        );
        assert_eq!(notifier.streamed_text(), "Hello, world");
    }

    /// The limiter's other trigger, also through `run_turn`: a delta that alone crosses the 4096-byte
    /// cap must not sit and wait for [`DELTA_WINDOW`] - it goes out the moment it lands, same as the
    /// unit test on `Pending` checks, but here through the loop that decides when `flush` runs.
    #[tokio::test]
    async fn an_oversized_delta_is_flushed_by_run_turn_without_waiting_for_the_window() {
        use crate::sdcp::notifications::RecordingNotifier;

        struct Oversized;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Oversized {
            fn id(&self) -> &'static str {
                "oversized"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                sink.send(crate::engines::EngineEvent::Delta("x".repeat(4096)));
                sink.send(crate::engines::EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let notifier = Arc::new(RecordingNotifier::new());
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "oversized".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(Oversized);
        let out: Arc<dyn Notifier> = notifier.clone();

        let ran = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            run_turn(state, engine, plan, out),
        )
        .await;

        assert!(
            ran.is_ok(),
            "a delta past the size cap should flush on arrival, not sit until the window elapses"
        );
        assert_eq!(
            notifier.kinds(),
            vec!["TurnDelta", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"],
            "the oversized delta is its own push, not folded into whatever follows"
        );
        assert_eq!(notifier.streamed_text(), "x".repeat(4096));
    }

    /// The limiter must not reorder the stream it is smoothing: a delta sat buffered under the window
    /// has to reach the notifier *before* whatever non-delta event the engine sends next, or a person
    /// would see `ToolCallStarted` on screen ahead of the words that, in the engine's own timeline,
    /// came first. That is the `_ => pending.flush(...)` arm above, and until now nothing exercised it.
    #[tokio::test]
    async fn a_pending_delta_flushes_before_the_next_non_delta_event() {
        use crate::sdcp::notifications::RecordingNotifier;

        struct ThenATool;

        #[async_trait::async_trait]
        impl crate::engines::Engine for ThenATool {
            fn id(&self) -> &'static str {
                "then-a-tool"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                sink.send(crate::engines::EngineEvent::Delta("checking the tests".to_string()));
                sink.send(crate::engines::EngineEvent::ToolStarted {
                    call_id: "c1".into(),
                    tool: "read".into(),
                    name: "Read".into(),
                    target: "tests.rs".into(),
                });
                sink.send(crate::engines::EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let notifier = Arc::new(RecordingNotifier::new());
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "then-a-tool".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(ThenATool);
        let out: Arc<dyn Notifier> = notifier.clone();

        run_turn(state, engine, plan, out).await;

        assert_eq!(
            notifier.kinds(),
            vec!["TurnDelta", "ToolCallStarted", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"],
            "the buffered delta must be flushed ahead of the tool event, not left to trail in behind it"
        );
        assert_eq!(notifier.streamed_text(), "checking the tests");
    }

    /// The dedup arm above (0.14.1): an engine that names the same call_id again - Claude Code resends a
    /// finished tool_use block on occasion - must not open a second card, and must not feed the repeat to
    /// [`crate::trust::cost::Runaway`] a second time. Five resends of the same call_id would cross
    /// [`crate::trust::cost::REPEAT_LIMIT`] if each were counted, and stop the turn as a loop that never happened.
    #[tokio::test]
    async fn a_repeated_call_id_opens_one_card_and_is_not_counted_twice() {
        use crate::sdcp::notifications::RecordingNotifier;

        struct Repeater;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Repeater {
            fn id(&self) -> &'static str {
                "repeater"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                use crate::engines::EngineEvent;

                for _ in 0..5 {
                    sink.send(EngineEvent::ToolStarted {
                        call_id: "c1".into(),
                        tool: "run".into(),
                        name: "Bash".into(),
                        target: "ls".into(),
                    });
                }
                sink.send(EngineEvent::Done { summary: "Done".to_string(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let notifier = Arc::new(RecordingNotifier::new());
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:")))
            .expect("bootstrapping a daemon for the test");
        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            engine_id: "repeater".to_string(),
            prompt_text: "hi".to_string(),
            model: "sonnet".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };
        let engine: Arc<dyn crate::engines::Engine> = Arc::new(Repeater);
        let out: Arc<dyn Notifier> = notifier.clone();

        run_turn(state, engine, plan, out).await;

        assert_eq!(
            notifier.kinds(),
            vec!["ToolCallStarted", "TurnCompleted", "CostUpdated", "TrustScored", "SessionUpdated"],
            "five resends of the same call_id must open exactly one card and never trip the runaway stop"
        );
    }

    /// The answer stored for the next turn carries the tool calls: without them a model read its own
    /// "I ran the tests" with no run in sight and apologised for inventing work it had really done.
    #[tokio::test]
    async fn the_stored_answer_remembers_the_turns_tool_calls() {
        struct Worker;

        #[async_trait::async_trait]
        impl crate::engines::Engine for Worker {
            fn id(&self) -> &'static str {
                "worker"
            }

            async fn start(&self, _prompt: Prompt, sink: &EventSink) {
                use crate::engines::EngineEvent;

                sink.send(EngineEvent::ToolStarted { call_id: "c1".into(), tool: "run".into(), name: "Run".into(), target: "npm test".into() });
                sink.send(EngineEvent::ToolCompleted { call_id: "c1".into(), status: "done".into(), meta: "exit 0 · 400ms".into(), diff: None });
                sink.send(EngineEvent::Delta("All five tests pass.".into()));
                sink.send(EngineEvent::Done { summary: "Done".into(), meta: String::new(), pass: None });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                false
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        struct Quiet;

        impl Notifier for Quiet {
            fn push(&self, _event: Value, _session: Option<String>, _turn: Option<String>) -> Option<i64> {
                None
            }
        }

        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon for the test");

        state.store.ensure_session("s1").unwrap();
        state.store.start_turn("turn-7", "s1", 1, "worker", "m", "Balanced", "run the tests").unwrap();

        let plan = RunPlan {
            session_id: "s1".to_string(),
            turn_id: "turn-7".to_string(),
            engine_id: "worker".to_string(),
            prompt_text: "run the tests".to_string(),
            model: "m".to_string(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            self_checkpointing: false,
            autonomy: Default::default(),
            ..Default::default()
        };

        run_turn(state.clone(), Arc::new(Worker), plan, Arc::new(Quiet)).await;

        let history = session_bridge::history_for(&state.store, "s1").unwrap();
        let said = &history.last().unwrap().text;

        assert!(said.starts_with("All five tests pass."), "{said}");
        assert!(said.contains("- Run npm test - exit 0 · 400ms"), "{said}");
    }
}
