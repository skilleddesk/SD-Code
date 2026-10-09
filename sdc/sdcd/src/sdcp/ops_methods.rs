//! The agency methods (0.12): sites, Safe Deploy, Health Watch, approvals, playbooks, the Night Guardian,
//! Takeover X-ray, shadow database migrations, staging, the team - and the release-safety methods
//! (updates, crash reports, the CLI self-check) and the Time Machine's branches.
//!
//! Anything that talks to a server answers at once and streams its result as events (`DeployUpdated`,
//! `HealthUpdated`, `XrayReady`, `ShadowDbUpdated`, `StagingUpdated`), so a slow host never blocks the
//! window's other calls.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::Daemon;
use crate::ops::{self, Runner, Site};
use crate::sdcp::envelope::{Envelope, ErrorObject};
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;

impl Daemon {
    pub(super) fn dispatch_ops(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            "site.list" => self.site_list(),
            "site.detect" => self.site_detect(envelope),
            "site.save" => self.site_save(envelope, &*out),
            "site.remove" => {
                let id = envelope.require_str("siteId")?;

                Ok(json!({ "removed": self.store().remove_site(&id).map_err(ErrorObject::internal)? }))
            }
            "deploy.run" => self.deploy_run(envelope, out),
            "deploy.list" => Ok(json!({ "deploys": self.store().deploys(envelope.opt_str("siteId").as_deref(), envelope.opt_i64("limit").unwrap_or(30)).map_err(ErrorObject::internal)? })),
            "deploy.get" => Ok(json!({ "deploy": self.store().deploy(&envelope.require_str("deployId")?).map_err(ErrorObject::internal)? })),
            "deploy.preview" => self.deploy_preview(envelope),
            "deploy.rollback" => self.deploy_rollback(envelope, out),
            "deploy.restoreDb" => self.deploy_restore_db(envelope, &*out),
            "health.check" => self.health_check(envelope, out),
            "health.history" => Ok(json!({ "reports": self.store().health_history(&envelope.require_str("siteId")?, envelope.opt_i64("limit").unwrap_or(50)).map_err(ErrorObject::internal)? })),
            "guardian.set" => self.guardian_set(envelope),
            "approval.list" => Ok(json!({ "approvals": self.store().approvals(envelope.opt_str("subject").as_deref()).map_err(ErrorObject::internal)? })),
            "approval.request" => self.approval_request(envelope, &*out),
            "approval.decide" => self.approval_decide(envelope, &*out),
            "approval.poll" => self.approval_poll(envelope, &*out),
            "xray.scan" => self.xray_scan(envelope, out),
            "xray.get" => {
                let host = envelope.require_str("hostId")?;
                let saved = self.store().setting(&format!("xray.{host}")).map_err(ErrorObject::internal)?;

                Ok(json!({ "map": saved.and_then(|text| serde_json::from_str::<Value>(&text).ok()) }))
            }
            "shadowdb.run" => self.shadowdb_run(envelope, out),
            "staging.create" => self.staging_create(envelope, out),
            "staging.stop" => self.staging_stop(envelope),
            "playbook.list" => Ok(json!({ "playbooks": ops::playbook::all(self.store()) })),
            "playbook.save" => Ok(json!({ "playbooks": ops::playbook::save(self.store(), envelope.params.get("playbook").unwrap_or(&Value::Null)).map_err(ErrorObject::internal)? })),
            "playbook.remove" => Ok(json!({ "playbooks": ops::playbook::remove(self.store(), &envelope.require_str("playbookId")?).map_err(ErrorObject::internal)? })),
            "playbook.run" => self.playbook_run(envelope, out),
            "team.get" => Ok(ops::team::to_json(self.store())),
            "team.set" => self.team_set(envelope),
            "settings.get" => {
                let key = envelope.require_str("key")?;

                Ok(json!({ "key": key, "value": self.store().setting(&key).map_err(ErrorObject::internal)? }))
            }
            "settings.set" => self.settings_set(envelope),
            "update.check" => self.update_check(envelope),

            /* 0.21: SDC installs and runs what it needs itself - no terminal on any platform. */
            "tool.list" => Ok(crate::host::tools::list()),
            "tool.install" => {
                let id = envelope.require_str("id")?;

                Ok(crate::host::tools::install(&id).map_err(ErrorObject::bad_request)?.to_json())
            }
            "tool.status" => {
                let id = envelope.require_str("jobId")?;

                crate::host::tools::status(&id).map(|job| job.to_json()).ok_or_else(|| ErrorObject::not_found(format!("no install job {id}")))
            }
            "ollama.start" => Ok(json!({ "running": crate::host::tools::ensure_ollama() })),
            /* 0.21: the person is typing - open the provider connection the turn will use. */
            "provider.warm" => {
                let model = envelope.require_str("model")?;

                Ok(json!({ "warming": crate::engines::native_api::warm(&model, envelope.opt_str("provider").as_deref()) }))
            }
            "ollama.pull" => Ok(crate::host::tools::pull_model(&envelope.require_str("model")?).to_json()),
            "host.port.free" => {
                let port = envelope.params.get("port").and_then(Value::as_u64).filter(|port| (1..=65535).contains(port)).ok_or_else(|| ErrorObject::bad_request("port must be 1-65535"))?;

                crate::host::tools::free_port(port as u16).map(|stopped| json!({ "stopped": stopped })).map_err(ErrorObject::internal)
            }
            "crash.list" => Ok(json!({ "reports": crate::crash::list() })),
            "crash.clear" => Ok(json!({ "cleared": crate::crash::clear() })),
            "cli.selfcheck" => Ok(json!({ "clis": crate::crash::cli_selfcheck() })),
            "status.share" => self.status_share(envelope),
            "timeline.branches" => self.timeline_branches(envelope),
            "timeline.switch" => self.timeline_switch(envelope, &*out),
            _ => self.dispatch_agent(envelope, out),
        }
    }

    fn site(&self, envelope: &Envelope) -> Result<Site, ErrorObject> {
        let id = envelope.require_str("siteId")?;
        let row = self.store().site(&id).map_err(ErrorObject::internal)?.ok_or_else(|| ErrorObject::not_found(format!("no site `{id}`")))?;

        Site::from_row(&row).ok_or_else(|| ErrorObject::internal("the site's row is incomplete"))
    }

    fn site_list(&self) -> Result<Value, ErrorObject> {
        let sites: Vec<Value> = self
            .store()
            .sites()
            .map_err(ErrorObject::internal)?
            .into_iter()
            .map(|mut site| {
                let id = site["id"].as_str().unwrap_or_default().to_string();

                site["health"] = self.store().health_history(&id, 1).ok().and_then(|rows| rows.into_iter().next()).unwrap_or(Value::Null);
                site["lastDeploy"] = self.store().deploys(Some(&id), 1).ok().and_then(|rows| rows.into_iter().next()).unwrap_or(Value::Null);
                site
            })
            .collect();

        Ok(json!({ "sites": sites }))
    }

    fn site_detect(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let host = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());
        let root = envelope.require_str("root")?;
        let runner = Runner::new(self.ssh_for(&host)?, &root);

        Ok(json!({ "config": ops::detect(&runner) }))
    }

    fn site_save(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = match envelope.opt_str("siteId").filter(|id| !id.is_empty()) {
            Some(id) => id,
            None => self.store().next_site_id().map_err(ErrorObject::internal)?,
        };
        let name = envelope.require_str("name")?;
        let host = envelope.opt_str("hostId").unwrap_or_else(|| "local".into());
        let root = envelope.require_str("root")?;
        let url = envelope.opt_str("url").unwrap_or_default();
        let config = envelope.params.get("config").cloned().unwrap_or_else(|| json!({}));

        if !url.is_empty() && !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ErrorObject::bad_request("The site's URL starts with http:// or https://"));
        }

        self.store().upsert_site(&id, &name, &host, &root, &url, &config).map_err(ErrorObject::internal)?;
        out.push(event::toast(&format!("Site saved: {name}"), None, None), None, None);

        Ok(json!({ "siteId": id, "site": self.store().site(&id).map_err(ErrorObject::internal)? }))
    }

    /* -----------------------------------------------------------------------------------------
     * Safe Deploy
     * -------------------------------------------------------------------------------------- */

    fn actor(&self) -> String {
        self.store().setting("team.current").ok().flatten().filter(|name| !name.is_empty()).unwrap_or_else(|| "person".into())
    }

    fn deploy_run(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let site = self.site(envelope)?;
        let kind = envelope.opt_str("kind").unwrap_or_else(|| "production".into());

        if crate::trust::kill::active().iter().any(|work| work.kind == "deploy" && work.label == format!("Deploy {}", site.name)) {
            return Err(ErrorObject::bad_request(format!("A deploy of {} is already running.", site.name)));
        }

        /* Agency Mode: a production deploy of a site that asks for one needs an approval first. */
        if kind == "production" && site.config["requireApproval"].as_bool().unwrap_or(false) {
            let subject = format!("deploy:{}", site.id);
            let approved = self
                .store()
                .approvals(Some(&subject))
                .map_err(ErrorObject::internal)?
                .into_iter()
                .find(|approval| approval["state"] == "approved");

            match approved {
                Some(approval) => {
                    let _ = self.store().decide_approval(approval["id"].as_str().unwrap_or_default(), "used", &self.actor(), None);
                }
                None => {
                    let pending = self.store().approvals(Some(&subject)).map_err(ErrorObject::internal)?.into_iter().find(|approval| approval["state"] == "pending");
                    let id = match pending {
                        Some(approval) => approval["id"].as_str().unwrap_or_default().to_string(),
                        None => {
                            let id = format!("appr-{}", uuid::Uuid::new_v4().simple());

                            self.store().save_approval(&id, &subject, "deploy", &self.actor(), &format!("Deploy {} to production", site.name), None).map_err(ErrorObject::internal)?;

                            if let Ok(Some(row)) = self.store().approvals(Some(&subject)).map(|rows| rows.into_iter().find(|row| row["id"] == id.as_str())) {
                                out.push(event::approval_recorded(row), None, None);
                            }

                            id
                        }
                    };

                    return Ok(json!({ "state": "awaiting_approval", "approvalId": id }));
                }
            }
        }

        let deploy_id = format!("dep-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let request = ops::deploy::Request {
            deploy_id: deploy_id.clone(),
            remote: self.ssh_for(&site.host_id)?,
            site,
            kind,
            actor: self.actor(),
            steps: None,
        };
        let state = self.state.clone();

        tokio::spawn(async move {
            ops::deploy::run(state, out, request).await;
        });

        Ok(json!({ "deployId": deploy_id, "state": "running" }))
    }

    fn deploy_target(&self, envelope: &Envelope) -> Result<(Site, Value), ErrorObject> {
        let id = envelope.require_str("deployId")?;
        let deploy = self.store().deploy(&id).map_err(ErrorObject::internal)?.ok_or_else(|| ErrorObject::not_found(format!("no deploy `{id}`")))?;
        let site_id = deploy["siteId"].as_str().unwrap_or_default().to_string();
        let row = self.store().site(&site_id).map_err(ErrorObject::internal)?.ok_or_else(|| ErrorObject::not_found("that deploy's site is gone"))?;

        Ok((Site::from_row(&row).ok_or_else(|| ErrorObject::internal("the site's row is incomplete"))?, deploy))
    }

    /// The Undo preview: what a rollback to the state before this deploy would change, file by file.
    fn deploy_preview(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let (site, deploy) = self.deploy_target(envelope)?;
        let archive = deploy["backup"]["files"]["path"].as_str().ok_or_else(|| ErrorObject::bad_request("That deploy has no files backup."))?;
        let runner = Runner::new(self.ssh_for(&site.host_id)?, &site.root);

        if !runner.posix() {
            return Err(ErrorObject::unsupported("the Undo preview needs a POSIX shell (a VPS, macOS or Linux)"));
        }

        let ran = runner.script(&ops::deploy::preview_script(&site, archive), Duration::from_secs(300))?;

        Ok(json!({ "changes": ops::deploy::parse_preview(&ran.stdout), "archive": archive }))
    }

    fn deploy_rollback(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let (site, deploy) = self.deploy_target(envelope)?;
        let remote = self.ssh_for(&site.host_id)?;
        let state = self.state.clone();
        let actor = self.actor();

        tokio::task::spawn_blocking(move || {
            ops::deploy::rollback(&state, &out, &site, remote, &deploy, &actor);
        });

        Ok(json!({ "state": "running" }))
    }

    /// Restores a deploy's database dump - only with the word `RESTORE`, and only after the database as it
    /// is now has been dumped too (P5): rows written since that backup would otherwise be gone for good.
    fn deploy_restore_db(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        if envelope.opt_str("confirm").as_deref() != Some("RESTORE") {
            return Err(ErrorObject::bad_request("Restoring a database replaces every row written since that backup. Send confirm: \"RESTORE\" to go ahead."));
        }

        let (site, deploy) = self.deploy_target(envelope)?;
        let dump = deploy["backup"]["db"]["path"].as_str().ok_or_else(|| ErrorObject::bad_request("That deploy has no database backup."))?;
        let (dump_command, restore_command) = ops::db_commands(&site).ok_or_else(|| ErrorObject::bad_request("This site has no database configured."))?;
        let runner = Runner::new(self.ssh_for(&site.host_id)?, &site.root);
        let dir = ops::deploy::dir_expr(&site.backup_dir());
        let stem = format!("{}-before-db-restore", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
        let script = format!(
            "set -e; dir={dir}; mkdir -p \"$dir\"; {dump_command} > \"$dir/{stem}.sql\"; gzip -f \"$dir/{stem}.sql\"; gunzip -c {dump} | {restore_command}; echo \"restored; the database as it was is at $dir/{stem}.sql.gz\"",
            dump = ops::quote(dump)
        );
        let ran = runner.script(&script, Duration::from_secs(1800))?;

        if !ran.ok {
            return Err(ErrorObject::internal(format!("The database restore failed: {}", ran.tail(6).join(" | "))));
        }

        let _ = self.store().append_audit(None, None, &format!("person:{}", self.actor()), "DatabaseRestored", &format!("Database of {} restored from {}", site.name, deploy["id"]), &json!({ "dump": dump }));
        out.push(event::toast(&format!("Database of {} restored. The state before the restore is backed up.", site.name), None, Some(8000)), None, None);

        Ok(json!({ "restored": true, "output": ran.tail(4) }))
    }

    fn health_check(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let site = self.site(envelope)?;
        let remote = self.ssh_for(&site.host_id)?;
        let state = self.state.clone();

        tokio::task::spawn_blocking(move || {
            let report = ops::health::run_once(&state, &*out, &site, remote.clone());

            ops::guardian::observe(&state, &out, &site, remote, &report);
        });

        Ok(json!({ "queued": true }))
    }

    fn guardian_set(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let site = self.site(envelope)?;
        let mut config = site.config.clone();

        config["guardian"] = json!({
            "enabled": envelope.opt_bool("enabled"),
            "autoRollback": envelope.opt_bool("autoRollback"),
        });

        self.store().upsert_site(&site.id, &site.name, &site.host_id, &site.root, &site.url, &config).map_err(ErrorObject::internal)?;

        Ok(json!({ "guardian": config["guardian"] }))
    }

    /* -----------------------------------------------------------------------------------------
     * Approvals
     * -------------------------------------------------------------------------------------- */

    fn approval_request(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = format!("appr-{}", uuid::Uuid::new_v4().simple());
        let subject = envelope.require_str("subject")?;

        self.store()
            .save_approval(&id, &subject, &envelope.opt_str("kind").unwrap_or_else(|| "general".into()), &self.actor(), &envelope.opt_str("note").unwrap_or_default(), envelope.opt_str("token").as_deref())
            .map_err(ErrorObject::internal)?;

        let row = self.store().approvals(Some(&subject)).map_err(ErrorObject::internal)?.into_iter().find(|row| row["id"] == id.as_str()).unwrap_or(Value::Null);

        out.push(event::approval_recorded(row.clone()), None, None);

        Ok(json!({ "approval": row }))
    }

    fn approval_decide(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("approvalId")?;
        let decision = match envelope.require_str("decision")?.as_str() {
            "approved" | "approve" => "approved",
            "declined" | "decline" | "deny" => "declined",
            other => return Err(ErrorObject::bad_request(format!("`{other}` is not a decision: approved or declined"))),
        };
        let approval = self.store().approvals(None).map_err(ErrorObject::internal)?.into_iter().find(|row| row["id"] == id.as_str()).ok_or_else(|| ErrorObject::not_found(format!("no approval `{id}`")))?;

        ops::team::may_decide(self.store(), &approval).map_err(ErrorObject::permission_denied)?;
        self.store().decide_approval(&id, decision, &self.actor(), envelope.opt_str("note").as_deref()).map_err(ErrorObject::internal)?;

        let row = self.store().approvals(None).map_err(ErrorObject::internal)?.into_iter().find(|row| row["id"] == id.as_str()).unwrap_or(Value::Null);

        out.push(event::approval_recorded(row.clone()), None, None);

        Ok(json!({ "approval": row }))
    }

    /// A client's answer on a staging page, read back from the host.
    fn approval_poll(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("approvalId")?;
        let approval = self.store().approvals(None).map_err(ErrorObject::internal)?.into_iter().find(|row| row["id"] == id.as_str()).ok_or_else(|| ErrorObject::not_found(format!("no approval `{id}`")))?;

        if approval["state"] != "pending" {
            return Ok(json!({ "approval": approval }));
        }

        let site_id = approval["subject"].as_str().unwrap_or_default().trim_start_matches("staging:").to_string();
        let row = self.store().site(&site_id).map_err(ErrorObject::internal)?.ok_or_else(|| ErrorObject::not_found("that approval's site is gone"))?;
        let site = Site::from_row(&row).ok_or_else(|| ErrorObject::internal("the site's row is incomplete"))?;
        let runner = Runner::new(self.ssh_for(&site.host_id)?, &site.root);
        let folder = ops::deploy::dir_expr(&ops::staging::folder(&site));
        let ran = runner.script(&format!("cat {folder}/.sdc-approval.json 2>/dev/null || true"), crate::ssh::QUICK)?;
        let token = approval["token"].as_str().unwrap_or_default();

        match ops::staging::read_answer(&ran.stdout, token) {
            Some(answer) => {
                let decision = if answer["decision"] == "approved" { "approved" } else { "question" };

                self.store().decide_approval(&id, decision, "client", answer["note"].as_str()).map_err(ErrorObject::internal)?;

                let row = self.store().approvals(None).map_err(ErrorObject::internal)?.into_iter().find(|row| row["id"] == id.as_str()).unwrap_or(Value::Null);

                out.push(event::approval_recorded(row.clone()), None, None);

                Ok(json!({ "approval": row, "answered": true }))
            }
            None => Ok(json!({ "approval": approval, "answered": false })),
        }
    }

    /* -----------------------------------------------------------------------------------------
     * Takeover X-ray, shadow migrations, staging, playbooks
     * -------------------------------------------------------------------------------------- */

    fn xray_scan(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let host = envelope.require_str("hostId")?;
        let ssh = self.ssh_for(&host)?.ok_or_else(|| ErrorObject::bad_request("Takeover X-ray scans a server: pick a VPS, not this machine."))?;
        let state = self.state.clone();
        let host_id = host.clone();

        tokio::task::spawn_blocking(move || {
            let payload = match ssh.run(ops::xray::SCAN, Duration::from_secs(180)) {
                Ok(output) => {
                    let map = ops::xray::analyse(&ssh.label(), &output.stdout);
                    let doc = ops::xray::document(&map);
                    let path = crate::paths::data_dir().ok().map(|dir| dir.join("xray")).and_then(|dir| {
                        std::fs::create_dir_all(&dir).ok()?;

                        let file = dir.join(format!("{host_id}-{}.md", chrono::Utc::now().format("%Y%m%d-%H%M")));

                        std::fs::write(&file, &doc).ok()?;

                        Some(file.display().to_string())
                    });

                    let _ = state.store.set_setting(&format!("xray.{host_id}"), &map.to_string());

                    json!({ "type": "XrayReady", "hostId": host_id, "map": map, "document": doc, "path": path, "error": Value::Null })
                }
                Err(error) => json!({ "type": "XrayReady", "hostId": host_id, "map": Value::Null, "error": error.message }),
            };

            out.push(payload, None, None);
        });

        Ok(json!({ "queued": true }))
    }

    fn shadowdb_run(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let site = self.site(envelope)?;
        let command = envelope.require_str("command")?;
        let shadow = format!("sdc_shadow_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let script = ops::shadowdb::script(&site, &command, &shadow).map_err(ErrorObject::bad_request)?;
        let runner = Runner::new(self.ssh_for(&site.host_id)?, &site.root);
        let run_id = format!("shadow-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let id = run_id.clone();

        tokio::task::spawn_blocking(move || {
            out.push(json!({ "type": "ShadowDbUpdated", "runId": id, "siteId": site.id, "state": "running", "command": command }), None, None);

            let result = match runner.script(&script, Duration::from_secs(1800)) {
                Ok(ran) => ops::shadowdb::read(&format!("{}\n{}", ran.stdout, ran.stderr)),
                Err(error) => json!({ "passed": false, "error": error.message, "sentence": error.message }),
            };

            out.push(json!({ "type": "ShadowDbUpdated", "runId": id, "siteId": site.id, "state": "done", "command": command, "result": result }), None, None);
        });

        Ok(json!({ "runId": run_id }))
    }

    fn staging_create(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let site = self.site(envelope)?;
        let remote = self.ssh_for(&site.host_id)?;
        let host = remote.as_ref().map(|ssh| ssh.target.user_host.rsplit('@').next().unwrap_or("localhost").to_string()).unwrap_or_else(|| "localhost".into());
        let script = ops::staging::create_script(&site, &host).map_err(ErrorObject::bad_request)?;
        let runner = Runner::new(remote, &site.root);
        let lang = envelope.opt_str("lang").unwrap_or_else(|| self.store().setting("ui.language").ok().flatten().unwrap_or_else(|| "en".into()));
        let summary = envelope.opt_str("summary").unwrap_or_default();
        let changes: Vec<String> = envelope.params.get("changes").and_then(Value::as_array).map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let before = envelope.opt_str("before");
        let after = envelope.opt_str("after");
        let email = self.store().setting("agency.email").ok().flatten();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let approval_id = format!("appr-{}", uuid::Uuid::new_v4().simple());

        self.store()
            .save_approval(&approval_id, &format!("staging:{}", site.id), "client", &self.actor(), &summary, Some(&token))
            .map_err(ErrorObject::internal)?;

        let id = approval_id.clone();

        tokio::task::spawn_blocking(move || {
            let push = |state: &str, extra: Value| {
                let mut payload = json!({ "type": "StagingUpdated", "siteId": site.id, "approvalId": id, "state": state });

                if let (Value::Object(fields), Some(object)) = (extra, payload.as_object_mut()) {
                    object.extend(fields);
                }

                out.push(payload, None, None);
            };

            push("copying", json!({}));

            let ran = match runner.script(&script, Duration::from_secs(1800)) {
                Ok(ran) if ran.ok => ran,
                Ok(ran) => return push("failed", json!({ "error": ran.tail(8).join("\n") })),
                Err(error) => return push("failed", json!({ "error": error.message })),
            };
            let url = ran.stdout.lines().find_map(|line| line.strip_prefix("url:")).unwrap_or_default().trim().to_string();
            let php = runner.script("command -v php >/dev/null 2>&1 && echo yes", crate::ssh::QUICK).map(|ran| ran.stdout.contains("yes")).unwrap_or(false);
            let page = ops::staging::page(&site, &lang, &summary, &changes, before.as_deref(), after.as_deref(), &token, email.as_deref(), php);
            let folder = ops::deploy::dir_expr(&ops::staging::folder(&site));
            let written = write_file(&runner, &format!("{folder}/sdc-approve.html"), &page)
                .and_then(|_| if php { write_file(&runner, &format!("{folder}/sdc-approve.php"), &ops::staging::receiver(&token, &lang)) } else { Ok(()) });

            match written {
                Ok(()) => push("ready", json!({ "url": url, "page": format!("{}/sdc-approve.html", url.trim_end_matches('/')), "php": php })),
                Err(error) => push("failed", json!({ "error": error })),
            }
        });

        Ok(json!({ "approvalId": approval_id }))
    }

    fn staging_stop(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let site = self.site(envelope)?;
        let runner = Runner::new(self.ssh_for(&site.host_id)?, &site.root);
        let ran = runner.script(&ops::staging::stop_script(&site, envelope.opt_bool("remove")), Duration::from_secs(120))?;

        Ok(json!({ "stopped": ran.ok }))
    }

    fn playbook_run(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("playbookId")?;
        let playbook = ops::playbook::all(self.store()).into_iter().find(|playbook| playbook["id"] == id.as_str()).ok_or_else(|| ErrorObject::not_found(format!("no playbook `{id}`")))?;
        let (commands, prompts) = ops::playbook::split(&playbook);
        let sites: Vec<String> = envelope.params.get("siteIds").and_then(Value::as_array).map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let mut deploys = Vec::new();

        if !commands.is_empty() {
            for site_id in &sites {
                let Some(row) = self.store().site(site_id).map_err(ErrorObject::internal)? else {
                    continue;
                };
                let Some(site) = Site::from_row(&row) else {
                    continue;
                };
                let deploy_id = format!("pb-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
                let request = ops::deploy::Request {
                    deploy_id: deploy_id.clone(),
                    remote: self.ssh_for(&site.host_id)?,
                    site,
                    kind: "playbook".into(),
                    actor: self.actor(),
                    steps: Some(commands.clone()),
                };
                let (state, out) = (self.state.clone(), out.clone());

                /* One site at a time would take an hour for twenty sites; all at once would take down a
                   shared server. Each site runs on its own task; a site's own steps stay in order. */
                tokio::spawn(async move {
                    ops::deploy::run(state, out, request).await;
                });

                deploys.push(json!({ "siteId": site_id, "deployId": deploy_id }));
            }
        }

        Ok(json!({ "deploys": deploys, "prompts": prompts }))
    }

    fn team_set(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        if let Some(members) = envelope.params.get("members").and_then(Value::as_array) {
            let clean: Vec<Value> = members
                .iter()
                .filter_map(|member| {
                    let name = member["name"].as_str()?.trim().to_string();
                    let role = member["role"].as_str().filter(|role| ops::team::ROLES.contains(role))?;

                    (!name.is_empty()).then(|| json!({ "name": name, "role": role }))
                })
                .collect();

            if !clean.is_empty() && !clean.iter().any(|member| member["role"] == "owner") {
                return Err(ErrorObject::bad_request("A team needs at least one owner."));
            }

            self.store().set_setting("team.members", &Value::Array(clean).to_string()).map_err(ErrorObject::internal)?;
        }

        if let Some(current) = envelope.opt_str("current") {
            let current = current.trim();
            let members = ops::team::members(self.store());

            /* A name the team does not have was a role of "client" - and a client cannot call
               `team.set`, so one typo locked the owner out of their own Settings for good (0.16.1). */
            if !members.is_empty() && !current.is_empty() && !members.iter().any(|member| member["name"].as_str() == Some(current)) {
                return Err(ErrorObject::bad_request(format!(
                    "{current} is not on this team. Pick one of: {}.",
                    members.iter().filter_map(|member| member["name"].as_str()).collect::<Vec<_>>().join(", ")
                )));
            }

            self.store().set_setting("team.current", current).map_err(ErrorObject::internal)?;
        }

        Ok(ops::team::to_json(self.store()))
    }

    /// The settings the daemon itself reads (the language alerts are written in, the agency's email and
    /// style guide, the Intent Engine's reply style, low-bandwidth mode). Anything else is refused, so this
    /// cannot become a way around the methods that check what they write.
    fn settings_set(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let key = envelope.require_str("key")?;
        let allowed = [
            "ui.language", "agency.email", "agency.styleGuide", "intent.replyStyle", "net.lowBandwidth", "update.channel", "crash.optIn",
            /* 0.16.1: the local model's context, and Settings → Research. A search service's key is not a
               setting - it goes to the keychain through `research.key.set`. */
            "ollama.contextTokens", "research.searchProvider", "research.searxngUrl", "research.maxSearches", "research.maxPages",
            "research.maxMinutes", "research.localWebOnly", "research.synthesisModel", "research.synthesisProvider",
        ];

        if !allowed.contains(&key.as_str()) {
            return Err(ErrorObject::bad_request(format!("`{key}` is not a setting this method writes")));
        }

        let value = envelope.params.get("value").map(|value| value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string())).unwrap_or_default();

        self.store().set_setting(&key, &value).map_err(ErrorObject::internal)?;

        if key.starts_with("research.") || key.starts_with("ollama.") {
            crate::agent::research::configure(self.store());
        }

        Ok(json!({ "key": key, "value": value }))
    }

    /* -----------------------------------------------------------------------------------------
     * Release safety
     * -------------------------------------------------------------------------------------- */

    /// Is there a newer release on this channel - and which release is the one to roll back to?
    fn update_check(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let channel = envelope.opt_str("channel").or_else(|| self.store().setting("update.channel").ok().flatten()).unwrap_or_else(|| "stable".into());

        crate::crash::update_check(&channel).map_err(ErrorObject::internal)
    }

    fn status_share(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let enabled = envelope.opt_bool("enabled");

        self.store().set_setting("status.share", if enabled { "on" } else { "off" }).map_err(ErrorObject::internal)?;

        Ok(crate::status::configure(self.state.clone(), enabled, envelope.opt_i64("port").map(|port| port as u16)))
    }

    /* -----------------------------------------------------------------------------------------
     * The Time Machine's branches (the multiverse timeline)
     * -------------------------------------------------------------------------------------- */

    fn timeline_branches(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let session = envelope.require_str("sessionId")?;

        Ok(json!({ "branches": self.store().rewind_frames(&session).map_err(ErrorObject::internal)? }))
    }

    /// Switches to another branch of a chat's history: the folder becomes what that branch left it as, and
    /// the branch being left is kept as a branch of its own - so switching is always reversible.
    fn timeline_switch(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let session = envelope.require_str("sessionId")?;
        let frame = envelope.opt_i64("frameId").ok_or_else(|| ErrorObject::bad_request("frameId is required"))?;

        self.refuse_while_running(&session)?;

        let subject = self.subject(envelope)?;
        let host = self.host_id_for(envelope)?;

        if !self.store().rewind_frames(&session).map_err(ErrorObject::internal)?.iter().any(|branch| branch["id"] == frame) {
            return Err(ErrorObject::not_found("That branch is not in this chat's timeline."));
        }

        /* Keep where we are as a branch of its own - pushed first, so the promotion below puts the chosen
           branch on top of it and the redo takes the chosen one. */
        let now = crate::rewind::commit_now(&subject.snapshot())?;

        self.store()
            .push_rewind_frame(&session, 0, "branch", &json!({ "rows": [], "nowSha": now, "turns": [], "branch": true }), &[])
            .map_err(ErrorObject::internal)?;
        self.store().promote_rewind_frame(&session, frame).map_err(ErrorObject::internal)?;

        match crate::rewind::redo(self.store(), &session, subject.snapshot(), host.as_deref())? {
            Some(applied) => {
                out.push(applied.to_event_payload(&session), Some(session.clone()), None);

                Ok(json!({ "switched": true, "turn": applied.turn }))
            }
            None => Ok(json!({ "switched": false })),
        }
    }
}

/// Writes a text file through a runner - `cat > file` on stdin for a host, a plain write here.
fn write_file(runner: &Runner, path_expr: &str, text: &str) -> Result<(), String> {
    match &runner.remote {
        Some(ssh) => {
            let output = ssh.run_with_stdin(&format!("cat > {path_expr}"), text, Duration::from_secs(60)).map_err(|error| error.message)?;

            if output.ok() {
                Ok(())
            } else {
                Err(output.reason())
            }
        }
        None => {
            let path = path_expr.replace("\"$HOME\"/", &format!("{}/", dirs::home_dir().map(|home| home.display().to_string()).unwrap_or_default())).replace('\'', "");

            std::fs::write(path, text).map_err(|error| error.to_string())
        }
    }
}
