//! Files, checkpoints, git, pty and the shell - `fs.*`, `git.*`, `pty.*`, `shell.*`. Split out of `methods.rs` (P0-3).
//!
//! Nothing here changed: the arms and the bodies are the ones that used to sit in the big `impl Daemon`, and
//! the private helpers they lean on are still in `methods.rs`, which a child module can reach. The `_` arm
//! passes the method on to the next link in the chain, exactly as the single `_` arm used to.

use super::*;

impl Daemon {
    /// The methods this module owns. Moved from `methods.rs` unchanged.
    pub(super) fn dispatch_file(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            /* The engines' environment ------------------------------------------------------- */
            "fs.read" => self.fs_read(envelope),
            "fs.write" => self.fs_write(envelope, &out),
            "fs.list" => self.fs_list(envelope),
            "fs.stat" => self.fs_stat(envelope),
            "fs.search" => self.fs_search(envelope),
            "fs.rename" => self.fs_rename(envelope, &out),
            "fs.delete" => self.fs_delete(envelope, &out),
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
            _ => self.dispatch_model(envelope, out),
        }
    }

    /// `fs.read` - the file's text, its hash, its real size, and whether the text was cut.
    ///
    /// Since 0.7.13 the path may be on a **host**: `hostId` (or the session's own host) decides, and
    /// `ssh::ops::read` answers with exactly this shape from the far side of the connection. The tree,
    /// the Preview editor and the checkpoint stamp read one contract either way, which is what makes a
    /// folder on a VPS a folder rather than a special case.
    pub(super) fn fs_read(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn fs_write(&self, envelope: &Envelope, out: &Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
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
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("a file").to_string();

        /* The checkpoint of what is about to change is taken for this path alone, now, and written out in the
           background (`checkpoint_first`): Save answers when the file is written, not when the folder has been
           walked. */
        self.checkpoint_first(envelope, &format!("Before editing {name}"), &[path.as_path()], out)?;

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
    ///
    /// `touched` are the paths the change is about to alter. For a folder on this machine their pre-image is
    /// captured here, in a moment, and the checkpoint itself - which reads the whole folder - is finished by a
    /// background worker (`checkpoints::deferred`) while the change goes ahead; the commit is the same one, with the
    /// touched paths as they were. A folder on a host, or a path outside the folder, keeps the plain synchronous
    /// checkpoint: it is the network or an unusual layout that decides there, and "undoable" must not depend on a guess.
    pub(super) fn checkpoint_first(
        &self,
        envelope: &Envelope,
        title: &str,
        touched: &[&std::path::Path],
        out: &Arc<dyn Notifier>,
    ) -> Result<(), ErrorObject> {
        let Some(session) = envelope.opt_str("sessionId") else {
            return Ok(());
        };
        let subject = self.subject(envelope)?;
        let turn_id = envelope.opt_str("turnId");
        let turn = self.state.events.seq();

        if let crate::checkpoints::Snapshot::Local(root) = subject.snapshot() {
            if let Ok(image) = crate::git::preimage(root, touched) {
                if !image.paths.is_empty() {
                    let (notifier, event_session) = (out.clone(), session.clone());

                    crate::checkpoints::deferred::submit(crate::checkpoints::deferred::Pending {
                        store: self.store().clone(),
                        session_id: session.clone(),
                        turn,
                        title: title.to_string(),
                        root: root.to_path_buf(),
                        image,
                        done: Box::new(move |fresh| {
                            notifier.push(event::checkpoint_saved(&event_session, fresh.to_event_payload()), Some(event_session.clone()), turn_id);
                        }),
                    });

                    return Ok(());
                }
            }
        }

        if let Ok(fresh) = crate::checkpoints::create(
            self.store(),
            &session,
            turn,
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
    pub(super) fn inside_folder(&self, envelope: &Envelope, raw: &str) -> Result<(), ErrorObject> {
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
    pub(super) fn fs_rename(&self, envelope: &Envelope, out: &Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let from = envelope.require_str("path")?;
        let to = envelope.require_str("to")?;

        self.inside_folder(envelope, &from)?;
        self.inside_folder(envelope, &to)?;

        let name = from.rsplit(['/', '\\']).next().unwrap_or("a file").to_string();

        self.checkpoint_first(
            envelope,
            &format!("Before renaming {name}"),
            &[std::path::Path::new(&from), std::path::Path::new(&to)],
            out,
        )?;

        match self.remote_for(envelope)? {
            Some(ssh) => crate::ssh::ops::rename(&ssh, &from, &to)?,
            None => crate::fs::rename(std::path::Path::new(&from), std::path::Path::new(&to))?,
        }

        Ok(json!({ "path": to, "renamed": true }))
    }

    /// `fs.delete` (v4) - the tree's Delete, with a checkpoint first so Rewind brings it back.
    pub(super) fn fs_delete(&self, envelope: &Envelope, out: &Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let raw = envelope.require_str("path")?;

        self.inside_folder(envelope, &raw)?;

        let name = raw.rsplit(['/', '\\']).next().unwrap_or("a file").to_string();

        self.checkpoint_first(envelope, &format!("Before deleting {name}"), &[std::path::Path::new(&raw)], out)?;

        match self.remote_for(envelope)? {
            Some(ssh) => crate::ssh::ops::remove(&ssh, &raw)?,
            None => crate::fs::remove(std::path::Path::new(&raw))?,
        }

        Ok(json!({ "path": raw, "deleted": true }))
    }

    /// `fs.mkdir` (v4) - the tree's New folder.
    pub(super) fn fs_mkdir(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn fs_list(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn fs_stat(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn fs_search(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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

    /// `git.status` - which branch, and how many files the working tree has changed.
    ///
    /// Read from the **folder's own** machine, so a chat pointed at `/srv/app` on a VPS is told about
    /// that repository and not about anything on this laptop. `("", 0)` - no badge - for a folder that
    /// is not a repository, on either side.
    pub(super) fn git_status(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn git_diff(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn git_checkpoint(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    pub(super) fn git_worktree(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn pty_open(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn pty_open_shell(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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

    pub(super) fn pty_write(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let pty_id = envelope.require_str("ptyId")?;

        self.state.pty.write(&pty_id, &envelope.opt_str("data").unwrap_or_default())?;

        Ok(json!({ "written": true }))
    }

    pub(super) fn pty_close(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let pty_id = envelope.require_str("ptyId")?;

        Ok(json!({ "closed": self.state.pty.close(&pty_id) }))
    }

    /// `pty.output`: the tail of a long-running process, and whether it is still alive.
    pub(super) fn pty_output(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let since = envelope.params.get("since").and_then(Value::as_u64);

        self.state.pty.output_since(&envelope.require_str("ptyId")?, since)
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
    pub(super) fn shell_run(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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
}
