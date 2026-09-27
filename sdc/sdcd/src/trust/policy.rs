//! Policy-as-code: `.sdc/policy.toml` (Trust Kernel, part 2).
//!
//! A project says in one file what an AI may and may not do in it, and the daemon enforces it on every
//! engine - the SDC Agent before it acts, the CLIs as soon as their stream shows the action:
//!
//! ```toml
//! production = true            # Auto mode is capped at Pro here, and deploys need an approval
//! max_files_per_turn = 25      # the blast radius: a turn that touches more stops and asks
//! privacy = "local-only"       # this repository never leaves the machine: local models only
//! protected_paths = ["wp-config.php", "*.pem", "backup/**"]
//! always_ask = ["git push", "systemctl"]
//! deny_commands = ["DROP DATABASE"]
//! max_turn_usd = 0.50
//!
//! [guardian]
//! auto_rollback = true         # the Night Guardian may roll back to the last good deploy on its own
//! ```
//!
//! A file's lists **extend** the defaults below, so a project cannot forget to protect `.env` by writing
//! its own list; `replace_defaults = true` is the explicit way to start from nothing.

use serde::{Deserialize, Serialize};

use crate::agent::gate::Autonomy;

/// Where a project keeps its policy, relative to its folder.
pub const POLICY_FILE: &str = ".sdc/policy.toml";

/// Paths an AI must not change without a person saying so, in any project: secrets, keys, a WordPress
/// site's database credentials, and backups (the one copy that must survive a bad turn).
pub const DEFAULT_PROTECTED: &[&str] = &[
    ".env",
    ".env.*",
    "wp-config.php",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa*",
    "id_ed25519*",
    ".htpasswd",
    "*.sql",
    "*.sql.gz",
    "backup/**",
    "backups/**",
    ".git/**",
    ".sdc/policy.toml",
];

/// Commands that always wait for a person, even in Auto: they reach beyond the folder (a push, a
/// publish, a service, a database, a remote copy), so a rewind cannot take them back.
pub const DEFAULT_ALWAYS_ASK: &[&str] = &[
    "git push",
    "npm publish",
    "cargo publish",
    "docker ",
    "systemctl ",
    "service ",
    "mysql ",
    "mysqldump",
    "psql ",
    "migrate",
    "scp ",
    "rsync ",
    "kubectl ",
    "terraform apply",
    "wp db",
    "wp plugin",
    "crontab",
];

/// The number of files one turn may change before it stops and asks.
pub const DEFAULT_MAX_FILES: usize = 25;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub production: bool,
    pub max_files_per_turn: usize,
    /// `any`, or `local-only`: only a model on this machine (Ollama) may read this project.
    pub privacy: String,
    pub protected_paths: Vec<String>,
    pub always_ask: Vec<String>,
    pub deny_commands: Vec<String>,
    /// The most one turn may cost before the governor stops it; `None` is no cap.
    pub max_turn_usd: Option<f64>,
    /// The Night Guardian may roll back to the last good deploy without asking.
    pub auto_rollback: bool,
    /// Where this policy came from: `default`, or the file's path.
    pub source: String,
    /// The file's own text had a problem; the defaults are in force and this says why.
    pub error: Option<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            production: false,
            max_files_per_turn: DEFAULT_MAX_FILES,
            privacy: "any".to_string(),
            protected_paths: DEFAULT_PROTECTED.iter().map(|path| path.to_string()).collect(),
            always_ask: DEFAULT_ALWAYS_ASK.iter().map(|line| line.to_string()).collect(),
            deny_commands: Vec::new(),
            max_turn_usd: None,
            auto_rollback: false,
            source: "default".to_string(),
            error: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    production: Option<bool>,
    max_files_per_turn: Option<usize>,
    privacy: Option<String>,
    protected_paths: Option<Vec<String>>,
    always_ask: Option<Vec<String>>,
    deny_commands: Option<Vec<String>>,
    replace_defaults: Option<bool>,
    max_turn_usd: Option<f64>,
    guardian: Option<GuardianFile>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardianFile {
    auto_rollback: Option<bool>,
}

fn extend(defaults: &mut Vec<String>, more: Option<Vec<String>>, replace: bool) {
    if replace {
        defaults.clear();
    }

    for item in more.unwrap_or_default() {
        let item = item.trim().to_string();

        if !item.is_empty() && !defaults.contains(&item) {
            defaults.push(item);
        }
    }
}

impl Policy {
    /// A policy from a file's text. A file that does not parse is **not** a policy that allows
    /// everything: the defaults stay in force and `error` says what was wrong with the file.
    pub fn parse(text: &str, source: &str) -> Self {
        let file: PolicyFile = match toml::from_str(text) {
            Ok(file) => file,
            Err(error) => {
                return Self {
                    source: source.to_string(),
                    error: Some(format!("{source} could not be read, so the default policy applies: {}", error.message())),
                    ..Self::default()
                };
            }
        };
        let replace = file.replace_defaults.unwrap_or(false);
        let mut policy = Self { source: source.to_string(), ..Self::default() };

        policy.production = file.production.unwrap_or(false);
        policy.max_files_per_turn = file.max_files_per_turn.unwrap_or(DEFAULT_MAX_FILES);
        policy.privacy = match file.privacy.as_deref().map(str::trim) {
            Some("local-only" | "local") => "local-only".to_string(),
            _ => "any".to_string(),
        };
        extend(&mut policy.protected_paths, file.protected_paths, replace);
        extend(&mut policy.always_ask, file.always_ask, replace);
        extend(&mut policy.deny_commands, file.deny_commands, false);
        policy.max_turn_usd = file.max_turn_usd.filter(|usd| *usd > 0.0);
        policy.auto_rollback = file.guardian.and_then(|guardian| guardian.auto_rollback).unwrap_or(false);

        /* The policy file itself is always protected: an AI that can rewrite its own rules has none. */
        if !policy.protected_paths.iter().any(|path| path == POLICY_FILE) {
            policy.protected_paths.push(POLICY_FILE.to_string());
        }

        policy
    }

    /// The policy of a folder on this machine, or the defaults when it has no file.
    pub fn load_local(root: &std::path::Path) -> Self {
        let path = root.join(POLICY_FILE);

        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text, POLICY_FILE),
            Err(_) => Self::default(),
        }
    }

    /// The policy of a chat's folder, wherever it is.
    pub fn load(root: Option<&str>, remote: Option<&crate::ssh::Ssh>) -> Self {
        let Some(root) = root.filter(|root| !root.trim().is_empty()) else {
            return Self::default();
        };

        match remote {
            None => Self::load_local(std::path::Path::new(root)),
            Some(ssh) => {
                let path = format!("{}/{POLICY_FILE}", root.trim_end_matches('/'));

                match crate::ssh::ops::read(ssh, &path, 64 * 1024) {
                    Ok(value) => match value["text"].as_str() {
                        Some(text) => Self::parse(text, POLICY_FILE),
                        None => Self::default(),
                    },
                    Err(_) => Self::default(),
                }
            }
        }
    }

    /// The file a policy is written as - every field, so the file says exactly what is in force.
    pub fn to_toml(&self) -> String {
        let list = |items: &[String]| {
            let quoted: Vec<String> = items.iter().map(|item| format!("{item:?}")).collect();

            format!("[{}]", quoted.join(", "))
        };
        let defaults_protected: Vec<String> = DEFAULT_PROTECTED.iter().map(|path| path.to_string()).collect();
        let defaults_ask: Vec<String> = DEFAULT_ALWAYS_ASK.iter().map(|line| line.to_string()).collect();
        /* Only what the project added is written: the defaults are always there, and repeating them would
           make a later default the project never asked for impossible to add. */
        let extra_protected: Vec<String> = self
            .protected_paths
            .iter()
            .filter(|path| !defaults_protected.contains(path))
            .cloned()
            .collect();
        let extra_ask: Vec<String> = self.always_ask.iter().filter(|line| !defaults_ask.contains(line)).cloned().collect();

        let mut text = String::from("# SDC policy - what an AI may do in this project. See docs/MASTER-PLAN-v3-TRUST-KERNEL.md.\n");

        text.push_str(&format!("production = {}\n", self.production));
        text.push_str(&format!("max_files_per_turn = {}\n", self.max_files_per_turn));
        text.push_str(&format!("privacy = {:?}\n", self.privacy));
        text.push_str(&format!("protected_paths = {}\n", list(&extra_protected)));
        text.push_str(&format!("always_ask = {}\n", list(&extra_ask)));
        text.push_str(&format!("deny_commands = {}\n", list(&self.deny_commands)));

        if let Some(usd) = self.max_turn_usd {
            text.push_str(&format!("max_turn_usd = {usd}\n"));
        }

        text.push_str(&format!("\n[guardian]\nauto_rollback = {}\n", self.auto_rollback));

        text
    }

    /// The protected pattern a path matches, when it matches one. `path` may be absolute; `root` makes it
    /// relative so a pattern like `backup/**` means the project's backup folder.
    pub fn protected(&self, path: &str, root: Option<&str>) -> Option<String> {
        let relative = relative_to(path, root);

        self.protected_paths.iter().find(|pattern| glob_match(pattern, &relative)).cloned()
    }

    /// The `always_ask` entry a command line contains, when it contains one.
    pub fn always_asks(&self, line: &str) -> Option<String> {
        let lowered = format!(" {} ", line.to_lowercase());

        self.always_ask
            .iter()
            .find(|entry| {
                let needle = entry.to_lowercase();

                !needle.trim().is_empty() && (lowered.contains(&format!(" {}", needle.trim_start())) || lowered.contains(&needle))
            })
            .cloned()
    }

    /// The `deny_commands` entry a command line contains, when it contains one.
    pub fn denies(&self, line: &str) -> Option<String> {
        let lowered = line.to_lowercase();

        self.deny_commands.iter().find(|entry| !entry.trim().is_empty() && lowered.contains(&entry.to_lowercase())).cloned()
    }

    /// The autonomy a turn may have here: production caps Auto at Pro, and says so.
    pub fn cap_autonomy(&self, requested: Autonomy) -> (Autonomy, Option<String>) {
        if self.production && requested == Autonomy::Auto {
            return (
                Autonomy::Pro,
                Some("This project is marked production in .sdc/policy.toml, so Auto runs as Pro: commands wait for you.".to_string()),
            );
        }

        (requested, None)
    }

    /// Whether an engine may read this project. A `local-only` project refuses anything that sends it off
    /// the machine, with the sentence that says how to continue.
    pub fn allows_engine(&self, engine: &str, provider: Option<&str>) -> Result<(), String> {
        if self.privacy != "local-only" {
            return Ok(());
        }

        let local = engine == "ollama" || provider == Some("ollama");

        if local {
            Ok(())
        } else {
            Err("This project is private (privacy = \"local-only\" in .sdc/policy.toml): only a local model may read it. Pick an Ollama model, or change the policy.".to_string())
        }
    }
}

/// `path` relative to `root`, with forward slashes - what the patterns are written against.
pub fn relative_to(path: &str, root: Option<&str>) -> String {
    let normal = path.replace('\\', "/");
    let trimmed = match root {
        Some(root) => {
            let root = root.replace('\\', "/");
            let root = root.trim_end_matches('/');

            if !root.is_empty() && normal.to_lowercase().starts_with(&root.to_lowercase()) {
                normal[root.len()..].trim_start_matches('/').to_string()
            } else {
                normal
            }
        }
        None => normal,
    };

    trimmed.trim_start_matches("./").to_string()
}

/// A small glob: `*` is any run of characters inside one path segment, `**` any number of segments, `?`
/// one character. A pattern without a slash matches the file's own name anywhere in the tree (so `*.pem`
/// protects every key file); a pattern with one is matched against the whole relative path.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim().replace('\\', "/");
    let path = path.trim_start_matches('/');

    if pattern.is_empty() {
        return false;
    }

    if !pattern.contains('/') {
        let name = path.rsplit('/').next().unwrap_or(path);

        return segment_match(&pattern.to_lowercase(), &name.to_lowercase())
            /* `backup` also names a folder: anything under a segment so named. */
            || path.split('/').rev().skip(1).any(|segment| segment_match(&pattern.to_lowercase(), &segment.to_lowercase()));
    }

    let anchored = pattern.starts_with('/');
    let pattern_parts: Vec<String> = pattern.trim_start_matches('/').split('/').map(str::to_lowercase).collect();
    let path_parts: Vec<String> = path.split('/').map(str::to_lowercase).collect();

    // A `backup/` pattern protects a backup folder wherever it is (`var/backups/…` too); a leading slash
    // anchors a pattern to the project's own top level.
    if anchored {
        return parts_match(&pattern_parts, &path_parts);
    }

    (0..path_parts.len()).any(|start| parts_match(&pattern_parts, &path_parts[start..]))
}

fn parts_match(pattern: &[String], path: &[String]) -> bool {
    match pattern.first() {
        None => path.is_empty(),
        Some(first) if first == "**" => (0..=path.len()).any(|skip| parts_match(&pattern[1..], &path[skip..])),
        Some(first) => !path.is_empty() && segment_match(first, &path[0]) && parts_match(&pattern[1..], &path[1..]),
    }
}

fn segment_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }

    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_protect_secrets_keys_backups_and_wordpress_credentials() {
        let policy = Policy::default();

        for path in ["wp-config.php", "public/wp-config.php", "certs/site.pem", ".env", ".env.production", "backup/db.sql", "var/backups/a.tar"] {
            assert!(policy.protected(path, None).is_some(), "{path}");
        }

        for path in ["src/app.ts", "wp-content/themes/x/style.css", "README.md"] {
            assert!(policy.protected(path, None).is_none(), "{path}");
        }
    }

    #[test]
    fn absolute_paths_are_matched_relative_to_the_folder() {
        let policy = Policy::default();

        assert_eq!(policy.protected("/var/www/shop/wp-config.php", Some("/var/www/shop")).as_deref(), Some("wp-config.php"));
        assert_eq!(policy.protected("C:\\site\\backup\\x.zip", Some("C:\\site")).as_deref(), Some("backup/**"));
    }

    #[test]
    fn a_file_extends_the_defaults_and_a_broken_file_keeps_them() {
        let policy = Policy::parse(
            "production = true\nmax_files_per_turn = 5\nprivacy = \"local-only\"\nprotected_paths = [\"config/*.yml\"]\nalways_ask = [\"php artisan\"]\ndeny_commands = [\"DROP DATABASE\"]\nmax_turn_usd = 0.5\n[guardian]\nauto_rollback = true\n",
            POLICY_FILE,
        );

        assert!(policy.production);
        assert_eq!(policy.max_files_per_turn, 5);
        assert!(policy.protected("config/database.yml", None).is_some());
        assert!(policy.protected(".env", None).is_some(), "the defaults are still there");
        assert!(policy.always_asks("php artisan migrate").is_some());
        assert!(policy.denies("mysql -e 'drop database shop'").is_some());
        assert!(policy.auto_rollback);
        assert_eq!(policy.max_turn_usd, Some(0.5));
        assert!(policy.error.is_none());

        let broken = Policy::parse("production = maybe", POLICY_FILE);

        assert!(broken.error.is_some());
        assert!(broken.protected(".env", None).is_some());
    }

    #[test]
    fn unknown_keys_are_an_error_rather_than_a_silent_no_op() {
        assert!(Policy::parse("protect = [\"x\"]", POLICY_FILE).error.is_some());
    }

    #[test]
    fn production_caps_auto_and_privacy_keeps_code_local() {
        let policy = Policy::parse("production = true\nprivacy = \"local-only\"", POLICY_FILE);

        assert_eq!(policy.cap_autonomy(Autonomy::Auto).0, Autonomy::Pro);
        assert_eq!(policy.cap_autonomy(Autonomy::Ask).0, Autonomy::Ask);
        assert!(policy.allows_engine("ollama", None).is_ok());
        assert!(policy.allows_engine("claude_code", None).is_err());
        assert!(Policy::default().allows_engine("claude_code", None).is_ok());
    }

    #[test]
    fn commands_that_reach_past_the_folder_always_ask() {
        let policy = Policy::default();

        assert!(policy.always_asks("git push origin main").is_some());
        assert!(policy.always_asks("sudo systemctl restart nginx").is_some());
        assert!(policy.always_asks("npm test").is_none());
        assert!(policy.always_asks("git status").is_none());
    }

    #[test]
    fn the_policy_round_trips_through_its_file() {
        let mut policy = Policy::parse("production = true\nprotected_paths = [\"secrets/**\"]", POLICY_FILE);

        policy.max_turn_usd = Some(1.25);

        let again = Policy::parse(&policy.to_toml(), POLICY_FILE);

        assert!(again.production);
        assert!(again.protected("secrets/a.json", None).is_some());
        assert_eq!(again.max_turn_usd, Some(1.25));
        assert!(again.error.is_none(), "{:?}", again.error);
    }

    #[test]
    fn globs_behave() {
        assert!(glob_match("*.pem", "a/b/c.pem"));
        assert!(glob_match("backup/**", "backup/x/y.sql"));
        assert!(glob_match("**/secret?.txt", "a/b/secret1.txt"));
        assert!(!glob_match("src/*.ts", "src/a/b.ts"));
        assert!(glob_match("src/**/*.ts", "src/a/b.ts"));
    }
}
