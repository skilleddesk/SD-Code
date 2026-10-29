//! The host, preview and ssh methods - `host.*`, `preview.*`, `ssh.*`. Split out of `methods.rs` (P0-3).
//!
//! Nothing here changed: the arms and the bodies are the ones that used to sit in the big `impl Daemon`, and
//! the private helpers they lean on are still in `methods.rs`, which a child module can reach. The `_` arm
//! passes the method on to the next link in the chain, exactly as the single `_` arm used to.

use super::*;

impl Daemon {
    /// The methods this module owns. Moved from `methods.rs` unchanged.
    pub(super) fn dispatch_host(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
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
            _ => self.dispatch_workspace(envelope, out),
        }
    }

    pub(super) fn host_status(&self, out: &dyn Notifier) -> Value {
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
    pub(super) fn host_shutdown(&self) -> Value {
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
    pub(super) fn host_add(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
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
        /* "Give SDC sudo permission on this server?" answered yes (1.0.4). Absent means no. */
        let grant_sudo = envelope.opt_bool("grantSudo");

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
                finish_connection(state, notifier, host_id, name, ssh, Secrets { password, code, keep: keep.clone(), stay: stay.clone(), grant_sudo }, key_note).await;

                return;
            }

            let scan_target = ssh.target.clone();
            let seen = tokio::task::spawn_blocking(move || crate::ssh::hostkey::inspect(&scan_target))
                .await
                .unwrap_or_else(|_| Err(ErrorObject::internal("the host key scan could not be run")));

            match seen {
                Ok(crate::ssh::hostkey::Trust::Pinned(_)) => {
                    finish_connection(state, notifier, host_id, name, ssh, Secrets { password, code, keep: keep.clone(), stay: stay.clone(), grant_sudo }, key_note).await;
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
    pub(super) fn host_trust(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
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

        spawn_finish(state, notifier, host_id.clone(), name, ssh, Secrets { password, code, keep, stay: Stay::from_envelope(envelope), grant_sudo: envelope.opt_bool("grantSudo") }, None);

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
    pub(super) fn host_probe(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
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
    pub(super) fn host_doctor(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn ssh_key(&self, _envelope: &Envelope) -> Result<Value, ErrorObject> {
        let public = crate::auth::remote::public_key();
        let path = crate::auth::remote::key_path();

        Ok(json!({
            "publicKey": public,
            "path": path.map(|path| path.display().to_string()),
            "exists": public.is_some(),
        }))
    }

    pub(super) fn host_key(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    /// The host a file, git or shell method works on: the one the envelope names, or - since 0.7.6 -
    /// the host the **session** belongs to.
    ///
    /// The envelope first, because a caller that names a host means that host; the session is the
    /// fallback that makes `fs.list { path }` work on a remote chat: the app knows the chat, the daemon
    /// knows which machine that chat's folder is on.
    pub(super) fn host_id_for(&self, envelope: &Envelope) -> Result<Option<String>, ErrorObject> {
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
    pub(super) fn remote_for(&self, envelope: &Envelope) -> Result<Option<crate::ssh::Ssh>, ErrorObject> {
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
    pub(super) fn subject(&self, envelope: &Envelope) -> Result<Subject, ErrorObject> {
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
    pub(super) fn host_password(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn preview_forward(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn preview_dev(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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

    pub(super) fn host_remove(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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
}
