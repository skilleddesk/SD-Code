//! Engines, providers, CLI sign-in and the model list - `engine.*`, `provider.*`, `cli.*`, `models.*`. Split out of `methods.rs` (P0-3).
//!
//! Nothing here changed: the arms and the bodies are the ones that used to sit in the big `impl Daemon`, and
//! the private helpers they lean on are still in `methods.rs`, which a child module can reach. The `_` arm
//! passes the method on to the next link in the chain, exactly as the single `_` arm used to.

use super::*;

impl Daemon {
    /// The methods this module owns. Moved from `methods.rs` unchanged.
    pub(super) fn dispatch_model(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
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
            /* Signing a CLI in from the app: the daemon drives the CLI's own login and never sees the
               credential (spec sections 9.10, 15.1). */
            "cli.login" => self.cli_login_start(envelope, &*out),
            "cli.login.status" => self.cli_login_status(envelope, &*out),
            "cli.login.code" => self.cli_login_code(envelope, &*out),
            "cli.login.cancel" => self.cli_login_cancel(envelope),
            /* 3.2: the way out of a sign-in. See `auth::cli_login::logout`. */
            "cli.logout" => self.cli_logout(envelope, &*out),
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
            "models.list" => {
                /* A read of the catalogue is also the moment to check it is fresh: when the last ask is older than a
                   few minutes the providers are asked again in the background, and `ModelsUpdated` brings any new
                   version to the open menu. This answer never waits for it. */
                if !envelope.opt_bool("refresh") {
                    crate::providers::models::refresh_if_stale(
                        self.store().clone(),
                        out.clone(),
                        crate::providers::models::STALE_AFTER_SECS,
                    );
                }

                self.models_list(envelope, &*out)
            }
            "models.select" => self.models_select(envelope, &*out),
            /* P0-12: the Step Trace - the newest model calls with their tokens, times, mode and verify result. */
            /* P0-15: a turn's receipt - every model it used, local or chain, with its time, tokens and cost. */
            "turn.receipt" => Ok(crate::trace::receipt(self.store(), &envelope.require_str("turnId")?)),
            "trace.list" => Ok(json!({ "steps": crate::trace::recent(self.store(), envelope.opt_str("sessionId").as_deref(), envelope.opt_i64("limit").map_or(100, |limit| limit.clamp(1, 500) as usize)) })),

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
            "provider.local.doctor" => {
                let doctor = providers::local_doctor();
                let running = doctor["daemon"].as_bool() == Some(true);

                /* 1.0: the card follows the answer at once - Connect used to find Ollama running and leave
                   the card "not connected", because nothing told the window. */
                out.push(
                    event::provider_status(json!({
                        "id": "ollama",
                        "status": if running { "connected" } else { "needs-auth" },
                        "detail": if running { format!("{} models installed", doctor["models"].as_array().map_or(0, Vec::len)) } else { doctor["detail"].as_str().unwrap_or_default().to_string() },
                    })),
                    None,
                    None,
                );

                Ok(doctor)
            }
            "provider.registry.list" => Ok(json!({ "models": providers::registry(&[]) })),
            "provider.registry.set" => self.provider_registry_set(envelope, &*out),
            _ => self.dispatch_guard(envelope, out),
        }
    }

    /// `engine.start`: answer now, stream later.
    pub(super) fn engine_start(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let session_id = envelope.opt_str("sessionId").unwrap_or_else(|| "s1".into());
        let engine_id = envelope.opt_str("engine").unwrap_or_else(|| "claude_code".into());

        self.count("turn_run");

        /* Phase 3.5: a regular turn ends the escalation count of the step before it; an escalation turn itself does not. */
        if !envelope.opt_bool("escalation") {
            super::rotation_methods::note_regular_turn(&session_id);
        }

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
        /* 1.0: the model's copy served in the key's own region, when the provider has one - see `region_twin`. */
        let model = match (engine_id.as_str(), provider.as_deref()) {
            ("native_api", Some(provider_id)) => self.region_model(model, provider_id, &*out),
            _ => model,
        };
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
        /* Pace (0.20): one line in front of the message when it is a plain question or a large task - read from
           the person's own words, locally. A confirmed contract, research and a compaction carry their own brief. */
        let pace_note = crate::agent::orient::pace_note(crate::agent::orient::pace_of(&prompt_text)).filter(|_| !research && !compact);
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
        let prompt_text = match pace_note.filter(|_| include_memory_file) {
            Some(note) => format!("{note}

{prompt_text}"),
            None => prompt_text,
        };
        /* Long-task memory (0.12): an unfinished plan and the project's .sdc/memory.md travel in front of the
           words - for this turn only; the stored prompt stays the person's own. */
        /* Continuity (0.12.5): the project's conventions, and - when this model is not the one that wrote
           the earlier turns - the hand-over, so a second model follows the first one's shape and style.
           On a host both are a round trip each, so they are asked at the same time (0.15.4). */
        let (memory, brief) = std::thread::scope(|scope| {
            let brief = scope.spawn(|| self.continuity_brief(&session_id, &turn_id, project_root.as_deref(), remote.as_ref(), &engine_id, &model));
            let memory = self.long_task_memory(&session_id, project_root.as_deref(), remote.as_ref(), include_memory_file, &prompt_text);

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
    pub(super) fn verify_run(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        self.count("verify_run");

        let session_id = envelope.require_str("sessionId")?;
        let root = self
            .root_for(envelope)?
            .and_then(|root| root.to_str().map(str::to_string))
            .ok_or_else(|| ErrorObject::bad_request("Verify needs a folder: open one for this chat first"))?;
        let since = envelope
            .opt_str("since")
            .filter(|sha| sha.len() == 40 && sha.chars().all(|character| character.is_ascii_hexdigit()));
        let requested = envelope.params.get("reviewer").and_then(|reviewer| {
            let engine = reviewer["engine"].as_str().filter(|engine| !engine.is_empty())?;

            Some(crate::verify::Reviewer {
                engine: engine.to_string(),
                model: reviewer["model"].as_str().unwrap_or_default().to_string(),
                provider: reviewer["provider"].as_str().filter(|id| !id.is_empty()).map(str::to_string),
            })
        });
        /* Phase 0.10: the review is by the Reviewer model the person chose, or it is off - never a model the app picked. */
        let chosen = self.store().setting(crate::guard::reviewer::SETTING).ok().flatten().and_then(|text| crate::guard::reviewer::parse(&text));
        let resolution = crate::guard::reviewer::resolve(requested, chosen, crate::v3::on("models.contract"));
        let (reviewer, review_off) = (resolution.reviewer, (resolution.mode == "off").then_some(resolution.note));
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
            review_off,
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

    pub(super) fn engine_stop(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
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

    pub(super) fn engine_status(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
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
    pub(super) fn engine_switch(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    pub(super) fn provider_save(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
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

    pub(super) fn provider_oauth_callback(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("id")?;
        let state = envelope.opt_str("state").unwrap_or_default();

        /* The token exchange is a later step; the answer says so rather than inventing a token. */
        Ok(providers::oauth_callback(&id, &state))
    }

    pub(super) fn provider_registry_set(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    /// `cli.login`: start the CLI's own sign-in and put its URL in front of the user.
    ///
    /// The answer arrives immediately with a `loginId`; the URL is what `cli.login.status` reports a
    /// moment later, because a CLI prints it once its screen is drawn. `program`/`args`/`pump` are the
    /// escape hatch: they let the flow be pointed at any CLI, which is also how the mechanism is tested
    /// without Claude installed.
    pub(super) fn cli_login_start(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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
    pub(super) fn cli_login_status(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let (answer, announce) = self
            .state
            .logins
            .status_announcing(&envelope.require_str("loginId")?)?;

        if announce {
            let detail = format!(
                "signed in through `{}`'s own CLI",
                answer["providerLabel"].as_str().unwrap_or("the")
            );

            /* 3.2: written down, not only announced. A finished sign-in used to live in this one event, so a restart
               left the card with nothing but a guess from a credential file - the silent sign-in the Gemini card is
               no longer allowed to make. The row is the consent; `cli.logout` takes it back. */
            if let Some(id) = answer["providerId"].as_str() {
                providers::mark_signed_in(self.store(), id, &detail);
            }

            out.push(
                event::provider_status(json!({
                    "id": answer["providerId"],
                    "status": "connected",
                    "detail": detail,
                })),
                None,
                None,
            );
        }

        Ok(answer)
    }

    /// `cli.login.code`: hand the pasted code to the CLI. The daemon passes it through and keeps
    /// nothing - the credential is written by the CLI, in the CLI's own store.
    pub(super) fn cli_login_code(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

    pub(super) fn cli_login_cancel(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        self.state.logins.cancel(&envelope.require_str("loginId")?)
    }

    /// `cli.logout`: sign a CLI out, and forget that SDC ever signed it in (3.2).
    ///
    /// Both halves matter. The CLI's credential is taken away (its own `logout` command, or - for Gemini, which has
    /// none - the files its Google sign-in wrote), and the row that records the person's consent is set back to
    /// `needs-auth`, so the card asks to sign in again instead of finding the old credential and going quiet.
    pub(super) fn cli_logout(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let provider_id = envelope.require_str("providerId")?;
        let done = crate::auth::cli_login::logout(&provider_id).map_err(ErrorObject::bad_request)?;

        providers::mark_signed_out(self.store(), &provider_id);

        /* 1.0.3: a sign-in SDC started and never finished is ended too, so the next Connect starts afresh. */
        self.state.logins.cancel_for(&provider_id);

        out.push(
            event::provider_status(json!({
                "id": provider_id,
                "status": "needs-auth",
                "detail": "signed out · Connect starts the CLI's own sign-in again",
            })),
            None,
            None,
        );

        Ok(json!({ "providerId": provider_id, "signedOut": true, "did": done }))
    }

    /// `models.list`: what the providers have, from the provider when it can be reached, from the cache
    /// when it cannot, and from the bundle underneath both.
    pub(super) fn models_list(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
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

        Ok(with_probe_classes(listed))
    }

    /// `models.select`: record the chosen model. A setting, not an event - see `providers::models`.
    pub(super) fn models_select(&self, _envelope: &Envelope, _out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let model_id = _envelope.require_str("modelId")?;
        let provider_id = _envelope.opt_str("providerId");

        /* `remove: true` takes the model out of use (0.12.4); without it, `Use` adds it and chooses it. */
        if _envelope.params.get("remove").and_then(Value::as_bool).unwrap_or(false) {
            return crate::providers::models::release(self.store(), &model_id, provider_id.as_deref());
        }

        crate::providers::models::select(self.store(), &model_id, provider_id.as_deref())
    }

    /// Flow 1's `Remove`: the secret leaves the keychain and the card goes back to `available`.
    pub(super) fn provider_remove(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("id")?;
        let removed = providers::remove(self.store(), &id)?;

        /* 1.0.3: nothing of the key stays behind - its model list goes with it, so the menu does not offer models
           that can no longer run. */
        let _ = self.store().replace_models(&id, &[], &chrono::Utc::now().to_rfc3339());

        out.push(
            event::provider_status(json!({ "id": id, "status": "available", "account": Value::Null })),
            None,
            None,
        );

        Ok(removed)
    }
}

/// P0-9: a local model that has been probed carries its class (S/M/L, `?` when only provisional) so the selector can
/// show it. Reads the profile files only - listing models never calls the runtime.
fn with_probe_classes(mut listed: Value) -> Value {
    /* P0-10: how many rows of the Rotation Chain are enabled on this computer - what the mode badge needs to tell
       Local Rotation (a chain) from Local-only (none). */
    let rotation = crate::data::home().map(|home| crate::data::rotation::load(&home).0).unwrap_or_default();

    listed["rotation"] = json!({ "enabledRows": rotation.usable().len(), "relayAuto": rotation.relay.auto, "showChip": rotation.relay.show_chip });

    if let Some(rows) = listed["models"].as_array_mut() {
        for row in rows.iter_mut().filter(|row| row["providerId"] == "ollama") {
            if let Some(profile) = row["id"].as_str().and_then(|id| crate::sme::probe::latest("ollama", id)) {
                row["probe"] = crate::sme::probe::row(&profile);
            }

            /* PADMA P2-1: one line of what is known about how this model makes tool calls (Settings -> Models). */
            if let Some(line) = row["id"].as_str().and_then(crate::padma::model::row_summary) {
                row["profileLine"] = json!(line);
            }
        }
    }

    listed
}

#[cfg(test)]
mod probe_class_tests {
    use super::*;
    use crate::sme::probe::{save, Class, ModelKey, Profile, Status};

    /// A probed local model shows its class in the list; a model with no profile, and every cloud model, shows none.
    #[test]
    fn a_probed_local_model_carries_its_class_and_nothing_else_does() {
        let _guard = crate::sme::probe::DATA_DIR_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());

        let key = ModelKey { provider: "ollama".into(), model: "qwen3.5:9b".into(), digest: "sha256:aa".into(), quantization: "Q4_K_M".into(), runtime: "0.12".into() };

        save(&Profile { key, status: Status::Provisional, class: Class::M, effective_context: 16_384, max_reliable_output: 512, tool_call_reliability: 0.9, tool_calls_printed_as_text: 0, json_adherence: 1.0, tokens_per_second: 30.0, first_token_ms: 200, vram_bytes: None, gpu_share: None, logprobs: false, thinking: true, seed: 1, probed_at: "2026-10-17".into() }).unwrap();

        let listed = with_probe_classes(json!({ "models": [
            { "id": "qwen3.5:9b", "providerId": "ollama" },
            { "id": "llama3.2:3b", "providerId": "ollama" },
            { "id": "deepseek-chat", "providerId": "deepseek" },
        ] }));
        let rows = listed["models"].as_array().unwrap();

        assert_eq!(rows[0]["probe"]["class"], "M?", "provisional shows a question mark");
        assert_eq!(rows[0]["probe"]["effectiveContext"], 16_384);
        assert!(rows[1].get("probe").is_none() && rows[2].get("probe").is_none());

        std::env::remove_var("SDC_DATA_DIR");
    }
}
