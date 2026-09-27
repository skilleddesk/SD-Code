//! **The agency layer** (0.12 → 2.0 of docs/MASTER-PLAN-v3-TRUST-KERNEL.md): the half of the pipeline
//! after the code is written - ship it, protect it, prove it.
//!
//! | module     | what it does |
//! | ---------- | ------------ |
//! | `deploy`   | Safe Deploy: preflight → backup (files + database, mandatory) → steps → health → auto-rollback |
//! | `health`   | Health Watch: uptime, response time, SSL days, disk, backup age, error log, last deploy; alerts in the person's language |
//! | `guardian` | the Night Guardian: on its own it only ever rolls back to the last good deploy; a fix waits for approval |
//! | `xray`     | Takeover X-ray: a read-only scan of an inherited server into a map, a document and a list of risks |
//! | `shadowdb` | a database migration rehearsed on a copy before it touches the real one |
//! | `staging`  | a micro-staging copy and the client's approval page |
//! | `playbook` | the same steps on many sites, each with its own backup |
//! | `team`     | roles: Owner, Developer, Reviewer, Client (read-only), and who may approve what |
//!
//! Principle P1 holds throughout: a host gets `ssh` commands and nothing installed. A command SDC wrote
//! itself (a backup, a scan) runs as written; a command a person configured (a deploy step) goes through
//! the same deny list as every other command.

pub mod deploy;
pub mod guardian;
pub mod health;
pub mod playbook;
pub mod shadowdb;
pub mod staging;
pub mod team;
pub mod xray;

use std::time::Duration;

use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;
use crate::ssh::Ssh;

/// A site: a folder on a machine that serves something at a URL, and how to ship and watch it.
#[derive(Debug, Clone)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub host_id: String,
    pub root: String,
    pub url: String,
    pub config: Value,
}

impl Site {
    pub fn from_row(row: &Value) -> Option<Self> {
        Some(Self {
            id: row["id"].as_str()?.to_string(),
            name: row["name"].as_str().unwrap_or_default().to_string(),
            host_id: row["hostId"].as_str().unwrap_or("local").to_string(),
            root: row["root"].as_str()?.to_string(),
            url: row["url"].as_str().unwrap_or_default().to_string(),
            config: row["config"].clone(),
        })
    }

    pub fn production(&self) -> bool {
        self.config["production"].as_bool().unwrap_or(true)
    }

    /// Where this site's backups live on its machine.
    pub fn backup_dir(&self) -> String {
        self.config["backup"]["dir"]
            .as_str()
            .filter(|dir| !dir.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("~/.sdc/backups/{}", self.id))
    }

    /// What a files backup leaves out: dependencies and caches the steps rebuild.
    pub fn backup_excludes(&self) -> Vec<String> {
        let configured: Vec<String> = self.config["backup"]["exclude"]
            .as_array()
            .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
            .unwrap_or_default();

        if configured.is_empty() {
            ["node_modules", ".git", "vendor", "wp-content/cache", ".next/cache", "__pycache__", ".sdc/backups"].iter().map(|item| item.to_string()).collect()
        } else {
            configured
        }
    }

    pub fn health_url(&self) -> String {
        self.config["health"]["url"].as_str().filter(|url| !url.is_empty()).unwrap_or(&self.url).to_string()
    }
}

/// Where a site's commands run: its host over `ssh`, or this machine.
pub struct Runner {
    pub remote: Option<Ssh>,
    pub root: String,
}

/// One command's outcome, in the shape every step and scan records.
#[derive(Debug, Clone)]
pub struct Ran {
    pub ok: bool,
    pub code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
    pub ms: u64,
}

impl Ran {
    /// The last lines of both streams, for a step's row.
    pub fn tail(&self, lines: usize) -> Vec<String> {
        let text = format!("{}\n{}", self.stdout, self.stderr);
        let all: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();

        all.iter().rev().take(lines).rev().map(|line| line.chars().take(300).collect()).collect()
    }
}

impl Runner {
    pub fn new(remote: Option<Ssh>, root: &str) -> Self {
        Self { remote, root: root.to_string() }
    }

    pub fn place(&self) -> String {
        self.remote.as_ref().map(Ssh::label).unwrap_or_else(|| "this machine".into())
    }

    /// Runs a script SDC wrote itself, in the site's folder (`sh -c` on a host, the platform's shell here).
    pub fn script(&self, script: &str, timeout: Duration) -> Result<Ran, ErrorObject> {
        let started = std::time::Instant::now();

        match &self.remote {
            Some(ssh) => {
                let root = crate::ssh::ops::remote_expr(&self.root)?;
                let output = ssh.run(&format!("cd {root} 2>/dev/null || true; {script}"), timeout)?;

                Ok(Ran {
                    ok: output.ok(),
                    code: output.code.map(i64::from),
                    stdout: output.stdout,
                    stderr: output.stderr,
                    ms: started.elapsed().as_millis() as u64,
                })
            }
            None => {
                let (command, args) = crate::pty::shell_for_line(script);
                let cwd = std::path::Path::new(&self.root).is_dir().then_some(self.root.as_str());
                let answer = crate::pty::PtyManager::new().run_once(&command, &args, cwd, timeout)?;

                Ok(Ran {
                    ok: answer["ok"].as_bool().unwrap_or(false),
                    code: answer["exitCode"].as_i64(),
                    stdout: answer["stdout"].as_str().unwrap_or_default().to_string(),
                    stderr: answer["stderr"].as_str().unwrap_or_default().to_string(),
                    ms: started.elapsed().as_millis() as u64,
                })
            }
        }
    }

    /// Runs a command a **person** configured (a deploy step): the deny list applies, as it does to every
    /// command that did not come from SDC's own code.
    pub fn step(&self, line: &str, timeout: Duration) -> Result<Ran, ErrorObject> {
        if let Some(reason) = crate::pty::denied_reason_line(line) {
            return Err(ErrorObject::permission_denied(format!("`{line}` is refused: {reason}")));
        }

        self.script(line, timeout)
    }

    /// Whether the site runs on a POSIX shell (a host always does; this machine unless it is Windows).
    pub fn posix(&self) -> bool {
        self.remote.is_some() || !cfg!(windows)
    }
}

/// The `ssh` side of a host, from its row: `None` for this machine or a host with no address.
pub fn remote_for_host(state: &crate::DaemonState, host_id: &str) -> Option<Ssh> {
    if host_id == "local" {
        return None;
    }

    let (target, port) = state.store.host_address(host_id).ok().flatten()?;

    Some(Ssh::new(crate::auth::remote::SshTarget { user_host: target?, port }))
}

/// A `'…'` shell word, for SDC's own scripts.
pub fn quote(value: &str) -> String {
    crate::ssh::sh_quote(value)
}

/// What a folder looks like, and the recipe SDC proposes for it: its kind, the deploy steps, the
/// database it uses and how to back it up. The person reviews and edits it before the first deploy.
pub fn detect(runner: &Runner) -> Value {
    let probe = runner.script(
        "for f in wp-config.php package.json composer.json artisan requirements.txt pyproject.toml Cargo.toml go.mod index.html .git; do [ -e \"$f\" ] && echo \"has:$f\"; done; \
         command -v wp >/dev/null 2>&1 && echo tool:wp; command -v pm2 >/dev/null 2>&1 && echo tool:pm2; command -v mysqldump >/dev/null 2>&1 && echo tool:mysqldump; \
         command -v pg_dump >/dev/null 2>&1 && echo tool:pg_dump; command -v systemctl >/dev/null 2>&1 && echo tool:systemctl; \
         [ -f package.json ] && grep -o '\"build\"' package.json | head -1 | sed 's/^/script:/'; [ -f package.json ] && grep -o '\"start\"' package.json | head -1 | sed 's/^/script:/'; \
         [ -f wp-config.php ] && sed -n \"s/.*define( *['\\\"]DB_NAME['\\\"] *, *['\\\"]\\([^'\\\"]*\\)['\\\"].*/dbname:\\1/p\" wp-config.php | head -1; true",
        crate::ssh::QUICK,
    );
    let text = probe.map(|ran| ran.stdout).unwrap_or_default();
    let has = |name: &str| text.lines().any(|line| line == format!("has:{name}"));
    let tool = |name: &str| text.lines().any(|line| line == format!("tool:{name}"));
    let script = |name: &str| text.lines().any(|line| line == format!("script:\"{name}\""));
    let db_name = text.lines().find_map(|line| line.strip_prefix("dbname:")).map(str::to_string);

    let (kind, steps, restart, db): (&str, Vec<String>, Option<String>, Value) = if has("wp-config.php") {
        (
            "wordpress",
            if has(".git") { vec!["git pull --ff-only".to_string()] } else { Vec::new() },
            None,
            json!({ "kind": "wordpress", "name": db_name }),
        )
    } else if has("artisan") {
        (
            "laravel",
            vec!["git pull --ff-only".into(), "composer install --no-dev --optimize-autoloader".into(), "php artisan migrate --force".into(), "php artisan config:cache".into()],
            None,
            json!({ "kind": "mysql" }),
        )
    } else if has("package.json") {
        let mut steps = Vec::new();

        if has(".git") {
            steps.push("git pull --ff-only".to_string());
        }

        steps.push("npm ci".to_string());

        if script("build") {
            steps.push("npm run build".to_string());
        }

        (
            "node",
            steps,
            if tool("pm2") { Some("pm2 reload all".to_string()) } else { None },
            json!({ "kind": "none" }),
        )
    } else if has("requirements.txt") || has("pyproject.toml") {
        ("python", vec!["git pull --ff-only".into(), "pip install -r requirements.txt".into()], None, json!({ "kind": "none" }))
    } else if has("index.html") {
        ("static", if has(".git") { vec!["git pull --ff-only".into()] } else { Vec::new() }, None, json!({ "kind": "none" }))
    } else {
        ("other", if has(".git") { vec!["git pull --ff-only".into()] } else { Vec::new() }, None, json!({ "kind": "none" }))
    };

    json!({
        "kind": kind,
        "production": true,
        "deploy": { "steps": steps, "restart": restart },
        "backup": { "dir": Value::Null, "exclude": Value::Null, "db": db },
        "health": { "url": Value::Null, "expectStatus": 200, "expectText": Value::Null, "interval": 300, "logFile": Value::Null, "enabled": true },
        "guardian": { "enabled": false, "autoRollback": false },
        "tools": { "wp": tool("wp"), "mysqldump": tool("mysqldump"), "pgDump": tool("pg_dump"), "systemctl": tool("systemctl") },
    })
}

/// The shell lines that dump and restore a site's database, or `None` when it has none. The password is
/// read **on the host** (from `wp-config.php`, `~/.my.cnf` or `~/.pgpass`) and never travels to SDC.
pub fn db_commands(site: &Site) -> Option<(String, String)> {
    let db = &site.config["backup"]["db"];

    if let (Some(dump), Some(restore)) = (db["dumpCommand"].as_str(), db["restoreCommand"].as_str()) {
        if !dump.trim().is_empty() && !restore.trim().is_empty() {
            return Some((dump.to_string(), restore.to_string()));
        }
    }

    match db["kind"].as_str() {
        Some("wordpress") => {
            let read = |name: &str| {
                format!("$(sed -n \"s/.*define( *['\\\"]{name}['\\\"] *, *['\\\"]\\([^'\\\"]*\\)['\\\"].*/\\1/p\" wp-config.php | head -1)")
            };
            let wp_cli = site.config["tools"]["wp"].as_bool().unwrap_or(false);

            if wp_cli {
                Some(("wp db export - --path=. --quiet".to_string(), "wp db import - --path=. --quiet".to_string()))
            } else {
                let credentials = format!("-h\"{}\" -u\"{}\" -p\"{}\" \"{}\"", read("DB_HOST"), read("DB_USER"), read("DB_PASSWORD"), read("DB_NAME"));

                Some((format!("mysqldump --single-transaction --quick {credentials}"), format!("mysql {credentials}")))
            }
        }
        Some("mysql") => {
            let name = db["name"].as_str()?;

            Some((format!("mysqldump --single-transaction --quick {}", quote(name)), format!("mysql {}", quote(name))))
        }
        Some("postgres") => {
            let name = db["name"].as_str()?;

            Some((format!("pg_dump --no-owner {}", quote(name)), format!("psql -q {}", quote(name))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(config: Value) -> Site {
        Site { id: "site-1".into(), name: "Shop".into(), host_id: "vps".into(), root: "/var/www/shop".into(), url: "https://shop.test".into(), config }
    }

    #[test]
    fn a_wordpress_site_dumps_its_database_with_credentials_read_on_the_host() {
        let (dump, restore) = db_commands(&site(json!({ "backup": { "db": { "kind": "wordpress" } } }))).unwrap();

        assert!(dump.starts_with("mysqldump --single-transaction"));
        assert!(dump.contains("DB_PASSWORD"), "the password is read from wp-config.php on the host");
        assert!(restore.starts_with("mysql "));

        let (dump, _) = db_commands(&site(json!({ "backup": { "db": { "kind": "wordpress" } }, "tools": { "wp": true } }))).unwrap();

        assert!(dump.starts_with("wp db export"));
        assert!(db_commands(&site(json!({ "backup": { "db": { "kind": "none" } } }))).is_none());
    }

    #[test]
    fn a_site_has_sensible_backup_defaults() {
        let site = site(json!({}));

        assert_eq!(site.backup_dir(), "~/.sdc/backups/site-1");
        assert!(site.backup_excludes().contains(&"node_modules".to_string()));
        assert!(site.production(), "a site is production until the person says otherwise");
        assert_eq!(site.health_url(), "https://shop.test");
    }

    #[test]
    fn a_local_folder_is_detected_by_what_it_contains() {
        let root = std::env::temp_dir().join(format!("sdc-ops-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("package.json"), "{\"scripts\":{\"build\":\"vite build\"}}").unwrap();

        let runner = Runner::new(None, root.to_str().unwrap());

        if runner.posix() {
            let found = detect(&runner);

            assert_eq!(found["kind"], "node");
            assert!(found["deploy"]["steps"].as_array().unwrap().iter().any(|step| step == "npm run build"));
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}
