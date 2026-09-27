//! **Safe Deploy** (0.14 in the plan, shipped in 0.12): a deploy that cannot leave a site worse than it
//! found it without saying so, and without a way back.
//!
//! ```text
//!   preflight ─► backup ─► steps ─► restart ─► health ─► success
//!       │          │         │         │          │
//!       └ fail: nothing changed        └──────────┴─► auto-rollback ─► health ─► rolled_back | rollback_failed
//! ```
//!
//! * **preflight** - the folder is there, the disk has room, `tar` exists, and (for a site that needs one)
//!   an approval was given. A failed preflight changes nothing.
//! * **backup** - files (`tar.gz`, without dependencies and caches) and the database (`mysqldump`,
//!   `pg_dump`, `wp db export`, or the site's own command) - **mandatory**: no backup, no deploy (P5).
//! * **steps** - the person's own commands, through the deny list, in the site's folder.
//! * **health** - the URL answers with the expected status (and text), retried three times.
//! * **auto-rollback** - any failure after the first step puts the files back exactly (files the deploy
//!   added are removed, changed and deleted ones restored), runs the restart again and checks health.
//!   The database is **not** restored automatically - rows written since the backup would be lost - but
//!   its dump is there and `deploy.restoreDb` restores it on a person's word.
//!
//! Every change of state is one `DeployUpdated` snapshot, and the row in `deploys` keeps the steps' logs
//! and the backup's paths, which is what a one-click rollback later restores.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::{quote, Ran, Runner, Site};
use crate::sdcp::envelope::ErrorObject;
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;
use crate::DaemonState;

const STEP_TIMEOUT: Duration = Duration::from_secs(600);
const BACKUP_TIMEOUT: Duration = Duration::from_secs(1800);

pub struct Request {
    pub deploy_id: String,
    pub site: Site,
    pub remote: Option<crate::ssh::Ssh>,
    /// `production`, `staging`, `rollback`, `playbook`.
    pub kind: String,
    pub actor: String,
    /// Override the site's steps (a playbook's commands).
    pub steps: Option<Vec<String>>,
}

/// The backup directory as a shell expression: `~/…` becomes `"$HOME"/'…'`, anything else is quoted.
pub fn dir_expr(dir: &str) -> String {
    match dir.strip_prefix("~/") {
        Some(rest) => format!("\"$HOME\"/{}", quote(rest)),
        None => quote(dir),
    }
}

/// The `--exclude` flags for a site's files backup.
fn excludes(site: &Site) -> String {
    site.backup_excludes().iter().map(|item| format!("--exclude={}", quote(&format!("./{}", item.trim_start_matches("./"))))).collect::<Vec<_>>().join(" ")
}

/// The script that backs a site up: files, then the database when it has one. It prints what it made as
/// `made:<kind>:<path>:<bytes>` lines, which is how the backup's row learns its paths.
pub fn backup_script(site: &Site, stem: &str) -> String {
    let dir = dir_expr(&site.backup_dir());
    let mut script = format!(
        "set -e; dir={dir}; mkdir -p \"$dir\"; tar -czf \"$dir/{stem}-files.tar.gz\" {excludes} -C . . ; echo \"made:files:$dir/{stem}-files.tar.gz:$(wc -c < \"$dir/{stem}-files.tar.gz\")\"",
        excludes = excludes(site)
    );

    if let Some((dump, _)) = super::db_commands(site) {
        script.push_str(&format!(
            "; {dump} > \"$dir/{stem}-db.sql\"; gzip -f \"$dir/{stem}-db.sql\"; echo \"made:db:$dir/{stem}-db.sql.gz:$(wc -c < \"$dir/{stem}-db.sql.gz\")\""
        ));
    }

    script
}

/// The script that makes the folder exactly what a files backup holds: files the backup does not have
/// are removed (dependencies and caches excepted - they were never in it), then the backup is unpacked.
pub fn restore_script(site: &Site, archive: &str) -> String {
    let prune = site
        .backup_excludes()
        .iter()
        .map(|item| format!("-path {}", quote(&format!("./{}", item.trim_start_matches("./").trim_end_matches('/')))))
        .collect::<Vec<_>>()
        .join(" -o ");
    let archive = quote(archive);

    format!(
        "set -e; list=\"$(mktemp)\"; now=\"$(mktemp)\"; tar -tzf {archive} | sed 's#^\\./##; s#/$##' | sort -u > \"$list\"; \
         find . \\( {prune} \\) -prune -o -type f -print | sed 's#^\\./##' | sort > \"$now\"; \
         comm -13 \"$list\" \"$now\" | while IFS= read -r stray; do rm -f -- \"$stray\"; done; \
         tar -xzf {archive} -C . ; rm -f \"$list\" \"$now\"; echo restored"
    )
}

/// What a rollback to this archive would change, without changing anything: the Undo preview.
pub fn preview_script(site: &Site, archive: &str) -> String {
    let prune = site
        .backup_excludes()
        .iter()
        .map(|item| format!("-path {}", quote(&format!("./{}", item.trim_start_matches("./").trim_end_matches('/')))))
        .collect::<Vec<_>>()
        .join(" -o ");
    let archive = quote(archive);

    format!(
        "list=\"$(mktemp)\"; now=\"$(mktemp)\"; tar -tzf {archive} | sed 's#^\\./##; s#/$##' | sort -u > \"$list\"; \
         find . \\( {prune} \\) -prune -o -type f -print | sed 's#^\\./##' | sort > \"$now\"; \
         comm -13 \"$list\" \"$now\" | head -300 | sed 's/^/added:/'; \
         tar -dzf {archive} -C . 2>&1 | head -300 | sed 's/^/differs:/'; rm -f \"$list\" \"$now\""
    )
}

/// Parses a preview's lines into `[{path, change}]`: `added` (will be removed), `modified`/`deleted`
/// (will be put back).
pub fn parse_preview(output: &str) -> Vec<Value> {
    let mut changes = Vec::new();

    for line in output.lines() {
        if let Some(path) = line.strip_prefix("added:") {
            changes.push(json!({ "path": path.trim(), "change": "added" }));
        } else if let Some(rest) = line.strip_prefix("differs:") {
            /* GNU tar: `./a.txt: Contents differ`, `./b.txt: Warning: Cannot stat: No such file or directory`. */
            let Some((path, what)) = rest.trim_start_matches("tar: ").split_once(": ") else {
                continue;
            };
            let path = path.trim().trim_start_matches("./").to_string();

            if path.is_empty() || path == "." {
                continue;
            }

            let change = if what.contains("No such file") || what.contains("Cannot stat") { "deleted" } else { "modified" };

            if !changes.iter().any(|existing: &Value| existing["path"] == path.as_str()) {
                changes.push(json!({ "path": path, "change": change }));
            }
        }
    }

    changes
}

fn row(id: &str, name: &str, command: Option<&str>) -> Value {
    json!({ "id": id, "name": name, "command": command, "status": "pending", "ms": Value::Null, "tail": [] })
}

struct Snapshot<'a> {
    state: &'a Arc<DaemonState>,
    out: &'a Arc<dyn Notifier>,
    request: &'a Request,
    steps: Vec<Value>,
    backup: Option<Value>,
    phase: &'static str,
    note: String,
    health: Option<Value>,
}

impl Snapshot<'_> {
    fn push(&self, finished: bool) {
        let _ = self.state.store.save_deploy(
            &self.request.deploy_id,
            &self.request.site.id,
            &self.request.kind,
            self.phase,
            &json!(self.steps),
            self.backup.as_ref(),
            &self.note,
            finished,
        );

        self.out.push(
            event::deploy_updated(json!({
                "deployId": self.request.deploy_id,
                "siteId": self.request.site.id,
                "name": self.request.site.name,
                "kind": self.request.kind,
                "state": self.phase,
                "steps": self.steps,
                "backup": self.backup,
                "note": self.note,
                "health": self.health,
                "actor": self.request.actor,
            })),
            None,
            None,
        );
    }

    fn mark(&mut self, index: usize, status: &str, ran: Option<&Ran>, extra: Option<Vec<String>>) {
        self.steps[index]["status"] = json!(status);

        if let Some(ran) = ran {
            self.steps[index]["ms"] = json!(ran.ms);
            self.steps[index]["tail"] = json!(ran.tail(12));
        }

        if let Some(extra) = extra {
            self.steps[index]["tail"] = json!(extra);
        }

        self.push(false);
    }
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}

/// Health, retried: three tries, five seconds apart. `None` when the site has no URL to check.
pub async fn health_with_retries(site: &Site) -> Option<Value> {
    let url = site.health_url();

    if url.trim().is_empty() {
        return None;
    }

    let expect_status = site.config["health"]["expectStatus"].as_u64().unwrap_or(200) as u16;
    let expect_text = site.config["health"]["expectText"].as_str().map(str::to_string);
    let mut last = Value::Null;

    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        let (url, text) = (url.clone(), expect_text.clone());

        last = blocking(move || super::health::http(&url, expect_status, text.as_deref())).await.unwrap_or(Value::Null);

        if last["ok"].as_bool() == Some(true) {
            break;
        }
    }

    Some(last)
}

/// The whole pipeline.
pub async fn run(state: Arc<DaemonState>, out: Arc<dyn Notifier>, request: Request) {
    let runner = Arc::new(Runner::new(request.remote.clone(), &request.site.root));
    let configured: Vec<String> = request.steps.clone().unwrap_or_else(|| {
        request.site.config["deploy"]["steps"]
            .as_array()
            .map(|steps| steps.iter().filter_map(|step| step.as_str().map(str::to_string)).filter(|step| !step.trim().is_empty()).collect())
            .unwrap_or_default()
    });
    let restart = request.site.config["deploy"]["restart"].as_str().filter(|line| !line.trim().is_empty()).map(str::to_string);
    let mut steps = vec![row("preflight", "Preflight", None), row("backup", "Backup (files + database)", None)];

    for (index, line) in configured.iter().enumerate() {
        steps.push(row(&format!("step-{index}"), line, Some(line)));
    }

    if let Some(line) = &restart {
        steps.push(row("restart", "Restart", Some(line)));
    }

    steps.push(row("health", "Health check", None));

    let mut snapshot = Snapshot { state: &state, out: &out, request: &request, steps, backup: None, phase: "running", note: String::new(), health: None };

    crate::trust::kill::begin(&request.deploy_id, "deploy", None, &format!("Deploy {}", request.site.name));
    snapshot.push(false);

    let stopped = || crate::engines::cancel::requested(&request.deploy_id);
    let finish = |snapshot: &mut Snapshot, phase: &'static str, note: String| {
        snapshot.phase = phase;
        snapshot.note = note;
        snapshot.push(true);
        crate::trust::kill::end(&snapshot.request.deploy_id);
        crate::engines::cancel::clear(&snapshot.request.deploy_id);
    };

    /* 1. Preflight: nothing is changed by it, so a failure here is a clean stop. */
    snapshot.mark(0, "running", None, None);

    let preflight = {
        let runner = runner.clone();
        let root = request.site.root.clone();

        blocking(move || {
            if runner.posix() {
                runner.script(
                    &format!(
                        "[ \"$(pwd)\" = {root} ] || [ -d {root} ] || {{ echo 'missing: the site folder is not there'; exit 3; }}; \
                         df -P . | tail -1 | awk '{{print \"disk:\"$5\" free:\"$4}}'; command -v tar >/dev/null || {{ echo 'missing: tar'; exit 4; }}; \
                         [ -d .git ] && echo \"git-dirty:$(git status --porcelain 2>/dev/null | wc -l)\"; true",
                        root = quote(&root)
                    ),
                    crate::ssh::QUICK,
                )
            } else {
                runner.script("tar --version", crate::ssh::QUICK)
            }
        })
        .await
    };

    let preflight = match preflight {
        Some(Ok(ran)) if ran.ok => ran,
        Some(Ok(ran)) => {
            snapshot.mark(0, "fail", Some(&ran), None);
            finish(&mut snapshot, "failed", "Preflight failed: nothing was changed.".into());

            return;
        }
        Some(Err(error)) => {
            snapshot.mark(0, "fail", None, Some(vec![error.message]));
            finish(&mut snapshot, "failed", "Preflight could not run: nothing was changed.".into());

            return;
        }
        None => {
            finish(&mut snapshot, "failed", "Preflight could not run.".into());

            return;
        }
    };
    let disk_full = preflight
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("disk:"))
        .and_then(|rest| rest.split('%').next())
        .and_then(|percent| percent.trim().parse::<u32>().ok())
        .is_some_and(|percent| percent >= 97);

    if disk_full {
        snapshot.mark(0, "fail", Some(&preflight), None);
        finish(&mut snapshot, "failed", "The disk is at least 97% full - there is no room for a backup, so nothing was changed.".into());

        return;
    }

    snapshot.mark(0, "pass", Some(&preflight), None);

    if stopped() {
        finish(&mut snapshot, "failed", "Stopped before the backup: nothing was changed.".into());

        return;
    }

    /* 2. Backup - mandatory. */
    snapshot.mark(1, "running", None, None);

    let stem = format!("{}-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"), request.deploy_id);
    let backup = {
        let runner = runner.clone();
        let site = request.site.clone();
        let stem = stem.clone();

        blocking(move || {
            if runner.posix() {
                runner.script(&backup_script(&site, &stem), BACKUP_TIMEOUT)
            } else {
                local_windows_backup(&site, &stem)
            }
        })
        .await
    };

    match backup {
        Some(Ok(ran)) if ran.ok => {
            let mut made = json!({ "stem": stem, "files": Value::Null, "db": Value::Null });

            for line in ran.stdout.lines() {
                /* `made:<kind>:<path>:<bytes>` - the path may itself hold a colon (`C:\…`), so the kind is
                   split off the front and the byte count off the back. */
                let Some((kind, rest)) = line.strip_prefix("made:").and_then(|rest| rest.split_once(':')) else {
                    continue;
                };
                let (path, bytes) = rest.rsplit_once(':').unwrap_or((rest, "0"));

                made[kind] = json!({ "path": path, "bytes": bytes.trim().parse::<u64>().unwrap_or(0) });
            }

            if made["files"].is_null() {
                snapshot.mark(1, "fail", Some(&ran), None);
                finish(&mut snapshot, "failed", "The backup did not produce an archive, so the deploy did not start (P5: no backup, no deploy).".into());

                return;
            }

            snapshot.backup = Some(made);
            snapshot.mark(1, "pass", Some(&ran), None);
        }
        Some(Ok(ran)) => {
            snapshot.mark(1, "fail", Some(&ran), None);
            finish(&mut snapshot, "failed", "The backup failed, so the deploy did not start: nothing was changed.".into());

            return;
        }
        Some(Err(error)) => {
            snapshot.mark(1, "fail", None, Some(vec![error.message]));
            finish(&mut snapshot, "failed", "The backup could not run, so the deploy did not start.".into());

            return;
        }
        None => {
            finish(&mut snapshot, "failed", "The backup could not run.".into());

            return;
        }
    }

    /* 3. The steps, then the restart. From here on a failure means the folder may have changed: roll back. */
    let first_step = 2;
    let mut failed_at: Option<String> = None;

    for index in first_step..snapshot.steps.len() - 1 {
        if stopped() {
            failed_at = Some("stopped by the kill switch".into());
            break;
        }

        let line = snapshot.steps[index]["command"].as_str().unwrap_or_default().to_string();

        snapshot.mark(index, "running", None, None);

        let ran = {
            let runner = runner.clone();

            blocking(move || runner.step(&line, STEP_TIMEOUT)).await
        };

        match ran {
            Some(Ok(ran)) if ran.ok => snapshot.mark(index, "pass", Some(&ran), None),
            Some(Ok(ran)) => {
                snapshot.mark(index, "fail", Some(&ran), None);
                failed_at = Some(snapshot.steps[index]["name"].as_str().unwrap_or("a step").to_string());
                break;
            }
            Some(Err(error)) => {
                snapshot.mark(index, "fail", None, Some(vec![error.message]));
                failed_at = Some(snapshot.steps[index]["name"].as_str().unwrap_or("a step").to_string());
                break;
            }
            None => {
                failed_at = Some("a step could not run".into());
                break;
            }
        }
    }

    /* 4. Health. */
    let health_index = snapshot.steps.len() - 1;

    if failed_at.is_none() {
        snapshot.mark(health_index, "running", None, None);

        match health_with_retries(&request.site).await {
            None => {
                snapshot.mark(health_index, "skipped", None, Some(vec!["No URL is set for this site, so its health was not checked: the deploy is unproven.".into()]));
            }
            Some(report) => {
                let ok = report["ok"].as_bool() == Some(true);

                snapshot.health = Some(report.clone());
                snapshot.mark(health_index, if ok { "pass" } else { "fail" }, None, Some(vec![report["detail"].as_str().unwrap_or_default().to_string()]));

                if !ok {
                    failed_at = Some("the health check".into());
                }
            }
        }
    }

    let Some(reason) = failed_at else {
        let unproven = snapshot.steps[health_index]["status"] == "skipped";

        finish(
            &mut snapshot,
            "success",
            if unproven { "Deployed. No health URL is set, so the site itself was not checked." } else { "Deployed and healthy." }.into(),
        );

        return;
    };

    /* 5. Auto-rollback. */
    let archive = snapshot.backup.as_ref().and_then(|backup| backup["files"]["path"].as_str()).map(str::to_string).unwrap_or_default();

    snapshot.steps.push(row("rollback", "Auto-rollback: put the files back", None));

    let rollback_index = snapshot.steps.len() - 1;

    snapshot.mark(rollback_index, "running", None, None);

    let restored = {
        let runner = runner.clone();
        let site = request.site.clone();
        let restart = restart.clone();

        blocking(move || {
            let ran = if runner.posix() { runner.script(&restore_script(&site, &archive), BACKUP_TIMEOUT)? } else { local_windows_restore(&site, &archive)? };

            if ran.ok {
                if let Some(line) = restart {
                    let again = runner.step(&line, STEP_TIMEOUT)?;

                    return Ok::<Ran, ErrorObject>(Ran { ok: again.ok, code: again.code, stdout: format!("{}\n{}", ran.stdout, again.stdout), stderr: again.stderr, ms: ran.ms + again.ms });
                }
            }

            Ok(ran)
        })
        .await
    };

    match restored {
        Some(Ok(ran)) if ran.ok => {
            snapshot.mark(rollback_index, "pass", Some(&ran), None);

            let health = health_with_retries(&request.site).await;
            let healthy = health.as_ref().map(|report| report["ok"].as_bool() == Some(true));

            snapshot.health = health;

            finish(
                &mut snapshot,
                if healthy == Some(false) { "rollback_failed" } else { "rolled_back" },
                match healthy {
                    Some(false) => format!("{reason} failed; the files were put back, but the site is still not healthy. The database dump is kept - restore it only if the deploy changed the database."),
                    Some(true) => format!("{reason} failed; the files were put back and the site is healthy again. The database was not touched by the rollback."),
                    None => format!("{reason} failed; the files were put back. No URL is set, so health was not checked."),
                },
            );
        }
        Some(Ok(ran)) => {
            snapshot.mark(rollback_index, "fail", Some(&ran), None);

            let archive = snapshot.backup.as_ref().map(|backup| backup["files"]["path"].to_string()).unwrap_or_default();

            finish(&mut snapshot, "rollback_failed", format!("{reason} failed, and the automatic rollback failed too. The backup is at {archive}. Restore it by hand or from the deploy's Rollback button."));
        }
        Some(Err(error)) => {
            snapshot.mark(rollback_index, "fail", None, Some(vec![error.message]));
            finish(&mut snapshot, "rollback_failed", format!("{reason} failed, and the rollback could not run."));
        }
        None => finish(&mut snapshot, "rollback_failed", format!("{reason} failed, and the rollback could not run.")),
    }
}

/// A rollback to the state **before** `target` (that deploy's backup): the one-click Rollback, and the
/// only thing the Night Guardian may do on its own. The folder's current state is backed up first, so the
/// rollback itself can be undone (P5). Blocking; records its own deploy row (`kind: rollback`).
pub fn rollback(
    state: &Arc<DaemonState>,
    out: &Arc<dyn Notifier>,
    site: &Site,
    remote: Option<crate::ssh::Ssh>,
    target: &Value,
    actor: &str,
) -> Value {
    let deploy_id = format!("rb-{}", uuid::Uuid::new_v4().simple());
    let runner = Runner::new(remote, &site.root);
    let archive = target["backup"]["files"]["path"].as_str().unwrap_or_default().to_string();
    let restart = site.config["deploy"]["restart"].as_str().filter(|line| !line.trim().is_empty()).map(str::to_string);
    let request = Request { deploy_id: deploy_id.clone(), site: site.clone(), remote: None, kind: "rollback".into(), actor: actor.to_string(), steps: None };
    let mut snapshot = Snapshot {
        state,
        out,
        request: &request,
        steps: vec![row("backup", "Backup of the current state", None), row("restore", &format!("Restore the files from before {}", target["id"].as_str().unwrap_or("that deploy")), None)],
        backup: None,
        phase: "running",
        note: String::new(),
        health: None,
    };

    if restart.is_some() {
        snapshot.steps.push(row("restart", "Restart", restart.as_deref()));
    }

    snapshot.steps.push(row("health", "Health check", None));
    snapshot.push(false);

    if archive.is_empty() {
        snapshot.phase = "failed";
        snapshot.note = "That deploy has no files backup to go back to.".into();
        snapshot.push(true);

        return json!({ "deployId": deploy_id, "state": "failed" });
    }

    let stem = format!("{}-{deploy_id}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));

    snapshot.mark(0, "running", None, None);

    let backed = if runner.posix() { runner.script(&backup_script(site, &stem), BACKUP_TIMEOUT) } else { local_windows_backup(site, &stem) };

    match backed {
        Ok(ran) if ran.ok => {
            snapshot.backup = Some(json!({ "stem": stem, "files": ran.stdout.lines().find_map(|line| line.strip_prefix("made:files:")).map(|rest| json!({ "path": rest.rsplit_once(':').map(|(path, _)| path).unwrap_or(rest) })) }));
            snapshot.mark(0, "pass", Some(&ran), None);
        }
        Ok(ran) => {
            snapshot.mark(0, "fail", Some(&ran), None);
            snapshot.phase = "failed";
            snapshot.note = "The current state could not be backed up, so nothing was rolled back.".into();
            snapshot.push(true);

            return json!({ "deployId": deploy_id, "state": "failed" });
        }
        Err(error) => {
            snapshot.mark(0, "fail", None, Some(vec![error.message]));
            snapshot.phase = "failed";
            snapshot.note = "The current state could not be backed up, so nothing was rolled back.".into();
            snapshot.push(true);

            return json!({ "deployId": deploy_id, "state": "failed" });
        }
    }

    snapshot.mark(1, "running", None, None);

    let restored = if runner.posix() { runner.script(&restore_script(site, &archive), BACKUP_TIMEOUT) } else { local_windows_restore(site, &archive) };

    match restored {
        Ok(ran) if ran.ok => snapshot.mark(1, "pass", Some(&ran), None),
        Ok(ran) => {
            snapshot.mark(1, "fail", Some(&ran), None);
            snapshot.phase = "rollback_failed";
            snapshot.note = "The restore failed. The state just before it is backed up, so nothing is lost.".into();
            snapshot.push(true);

            return json!({ "deployId": deploy_id, "state": "rollback_failed" });
        }
        Err(error) => {
            snapshot.mark(1, "fail", None, Some(vec![error.message]));
            snapshot.phase = "rollback_failed";
            snapshot.note = "The restore could not run.".into();
            snapshot.push(true);

            return json!({ "deployId": deploy_id, "state": "rollback_failed" });
        }
    }

    let mut index = 2;

    if let Some(line) = &restart {
        snapshot.mark(index, "running", None, None);

        match runner.step(line, STEP_TIMEOUT) {
            Ok(ran) => snapshot.mark(index, if ran.ok { "pass" } else { "fail" }, Some(&ran), None),
            Err(error) => snapshot.mark(index, "fail", None, Some(vec![error.message])),
        }

        index += 1;
    }

    let url = site.health_url();
    let health = (!url.trim().is_empty()).then(|| {
        super::health::http(&url, site.config["health"]["expectStatus"].as_u64().unwrap_or(200) as u16, site.config["health"]["expectText"].as_str())
    });

    match &health {
        None => snapshot.mark(index, "skipped", None, Some(vec!["No URL is set, so health was not checked.".into()])),
        Some(report) => snapshot.mark(index, if report["ok"] == true { "pass" } else { "fail" }, None, Some(vec![report["detail"].as_str().unwrap_or_default().to_string()])),
    }

    let healthy = health.as_ref().map(|report| report["ok"] == true);

    snapshot.health = health;
    snapshot.phase = if healthy == Some(false) { "rollback_failed" } else { "rolled_back" };
    snapshot.note = match healthy {
        Some(false) => "The files are back as they were before that deploy, but the site is still not healthy.".into(),
        Some(true) => "The files are back as they were before that deploy, and the site is healthy.".into(),
        None => "The files are back as they were before that deploy.".into(),
    };
    snapshot.push(true);

    json!({ "deployId": deploy_id, "state": snapshot.phase })
}

/// A files backup on a Windows folder: `tar.exe` (in Windows since 1803) writes the same archive.
fn local_windows_backup(site: &Site, stem: &str) -> Result<Ran, ErrorObject> {
    let directory = crate::paths::data_dir().map_err(ErrorObject::internal)?.join("backups").join(&site.id);

    std::fs::create_dir_all(&directory).map_err(ErrorObject::internal)?;

    let archive = directory.join(format!("{stem}-files.tar.gz"));
    let mut command = std::process::Command::new("tar");

    command.arg("-czf").arg(&archive);

    for item in site.backup_excludes() {
        command.arg(format!("--exclude=./{item}"));
    }

    let started = std::time::Instant::now();
    let output = command.arg("-C").arg(&site.root).arg(".").output().map_err(|error| ErrorObject::internal(format!("tar did not start: {error}")))?;
    let bytes = std::fs::metadata(&archive).map(|meta| meta.len()).unwrap_or(0);

    Ok(Ran {
        ok: output.status.success() && bytes > 0,
        code: output.status.code().map(i64::from),
        stdout: format!("made:files:{}:{bytes}", archive.display()),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        ms: started.elapsed().as_millis() as u64,
    })
}

fn local_windows_restore(site: &Site, archive: &str) -> Result<Ran, ErrorObject> {
    let started = std::time::Instant::now();
    let output = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(&site.root)
        .output()
        .map_err(|error| ErrorObject::internal(format!("tar did not start: {error}")))?;

    Ok(Ran {
        ok: output.status.success(),
        code: output.status.code().map(i64::from),
        stdout: "restored (files the deploy added are kept on Windows)".into(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site() -> Site {
        Site {
            id: "site-1".into(),
            name: "Shop".into(),
            host_id: "local".into(),
            root: "/srv/shop".into(),
            url: String::new(),
            config: json!({ "backup": { "db": { "kind": "mysql", "name": "shop" } } }),
        }
    }

    #[test]
    fn a_backup_makes_files_and_a_database_dump_and_names_them() {
        let script = backup_script(&site(), "20260927-dep-1");

        assert!(script.contains("tar -czf \"$dir/20260927-dep-1-files.tar.gz\""));
        assert!(script.contains("--exclude='./node_modules'"));
        assert!(script.contains("mysqldump --single-transaction --quick 'shop'"));
        assert!(script.contains("made:db:"));
        assert!(script.starts_with("set -e; dir=\"$HOME\"/'.sdc/backups/site-1'"));
    }

    #[test]
    fn the_restore_removes_what_the_deploy_added_but_never_dependencies() {
        let script = restore_script(&site(), "/b/x-files.tar.gz");

        assert!(script.contains("comm -13"));
        assert!(script.contains("-path './node_modules'"));
        assert!(script.contains("tar -xzf '/b/x-files.tar.gz' -C ."));
        assert!(crate::pty::denied_reason_line(&script).is_none(), "SDC's own restore must not trip the deny list");
    }

    #[test]
    fn the_undo_preview_reads_gnu_tars_compare_output() {
        let changes = parse_preview(
            "added:new-page.php\ndiffers:./wp-config.php: Mod time differs\ndiffers:./wp-config.php: Contents differ\ndiffers:tar: ./old.css: Warning: Cannot stat: No such file or directory\n",
        );

        assert_eq!(changes.len(), 3, "{changes:?}");
        assert_eq!(changes[0], json!({ "path": "new-page.php", "change": "added" }));
        assert_eq!(changes[1]["change"], "modified");
        assert_eq!(changes[2], json!({ "path": "old.css", "change": "deleted" }));
    }

    /// The whole pipeline on a real folder: a failing step is rolled back, and the file it changed comes back.
    #[tokio::test]
    async fn a_failing_step_is_rolled_back_exactly() {
        if cfg!(windows) {
            return;
        }

        let root = std::env::temp_dir().join(format!("sdc-deploy-e2e-{}", std::process::id()));
        let backups = std::env::temp_dir().join(format!("sdc-deploy-backups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), "v1").unwrap();

        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).unwrap();
        let out: Arc<dyn Notifier> = Arc::new(crate::sdcp::notifications::RecordingNotifier::default());
        let site = Site {
            id: "site-e2e".into(),
            name: "E2E".into(),
            host_id: "local".into(),
            root: root.display().to_string(),
            url: String::new(),
            config: json!({
                "deploy": { "steps": ["echo v2 > index.html", "echo added > new.html", "exit 7"] },
                "backup": { "dir": backups.display().to_string(), "db": { "kind": "none" } },
            }),
        };

        run(state.clone(), out, Request { deploy_id: "dep-e2e".into(), site, remote: None, kind: "production".into(), actor: "person".into(), steps: None }).await;

        let row = state.store.deploy("dep-e2e").unwrap().unwrap();

        assert_eq!(row["state"], "rolled_back", "{row}");
        assert_eq!(std::fs::read_to_string(root.join("index.html")).unwrap(), "v1");
        assert!(!root.join("new.html").exists(), "a file the deploy added is removed");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&backups);
    }
}
