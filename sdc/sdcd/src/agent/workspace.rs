//! The folder an agent works in - on this machine or on a host - and the only doors out of it.
//!
//! Every file the agent reads or writes is resolved against the chat's folder and refused when it lands
//! outside it, so `../../.ssh/id_ed25519` is a sentence, not a file read. The daemon's own file guard
//! (`fs::guard` locally, `ssh::ops`' copy of it remotely) still runs underneath: `.env`, `*.pem` and
//! the daemon's data directory are refused even inside the folder. Commands go through the same deny
//! list `shell.run` uses.
//!
//! The two machines answer the same way, because the agent should not have to know which one it is on:
//! the same text for a read, the same listing format, the same command report.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::sdcp::envelope::ErrorObject;
use crate::ssh::Ssh;

/// The most of one file the agent is handed at once. A model reading a 5 MB log is a turn that costs a
/// fortune and learns nothing; the read says it was cut and where, so it can ask for a range.
pub const READ_CAP: usize = 256 * 1024;

/// The longest a command may run before it is stopped, and the default when the model does not say.
pub const MAX_COMMAND: Duration = Duration::from_secs(600);
pub const DEFAULT_COMMAND: Duration = Duration::from_secs(120);

pub struct Workspace {
    /// The folder, as the machine it is on spells it.
    root: String,
    remote: Option<Ssh>,
}

/// What a command did, in the shape the agent and the tool card both read.
pub struct CommandReport {
    pub ok: bool,
    pub exit_code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}

impl Workspace {
    pub fn new(root: &str, remote: Option<Ssh>) -> Self {
        Self { root: root.trim_end_matches(['/', '\\']).to_string(), remote }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// Where the folder is, for the system prompt and the permission dialog: `this machine` or the host.
    pub fn place(&self) -> String {
        match &self.remote {
            Some(ssh) => ssh.label(),
            None => "this machine".to_string(),
        }
    }

    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    /// The shell a command line runs in, for the system prompt.
    pub fn shell(&self) -> &'static str {
        if self.remote.is_some() || !cfg!(windows) {
            "sh (POSIX)"
        } else {
            "cmd.exe (Windows)"
        }
    }

    /// A path the model named, resolved inside the folder - or the sentence that refuses it.
    pub fn resolve(&self, raw: &str) -> Result<String, ErrorObject> {
        let raw = raw.trim();
        let raw = if raw.is_empty() { "." } else { raw };

        if self.remote.is_some() {
            return resolve_posix(&self.root, raw);
        }

        let root = normalize(Path::new(&self.root));
        let candidate = if Path::new(raw).is_absolute() { PathBuf::from(raw) } else { root.join(raw) };
        let resolved = normalize(&candidate);

        if !resolved.starts_with(&root) {
            return Err(outside(raw, &self.root));
        }

        Ok(resolved.to_string_lossy().to_string())
    }

    /// The path as the model should see it: relative to the folder.
    pub fn relative(&self, absolute: &str) -> String {
        let trimmed = absolute
            .strip_prefix(&self.root)
            .map(|rest| rest.trim_start_matches(['/', '\\']))
            .unwrap_or(absolute);

        if trimmed.is_empty() { ".".to_string() } else { trimmed.replace('\\', "/") }
    }

    /// A file's text, capped at `READ_CAP`, and whether it was cut.
    pub fn read(&self, path: &str) -> Result<(String, bool), ErrorObject> {
        let resolved = self.resolve(path)?;

        if let Some(ssh) = &self.remote {
            let answer = crate::ssh::ops::read(ssh, &resolved, READ_CAP)?;

            return Ok((
                answer["text"].as_str().unwrap_or_default().to_string(),
                answer["truncated"].as_bool().unwrap_or(false),
            ));
        }

        let path = PathBuf::from(&resolved);
        let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);

        if size as usize > READ_CAP {
            let (text, _) = crate::fs::read_capped(&path, READ_CAP)?;

            return Ok((text, true));
        }

        let (text, _) = crate::fs::read(&path)?;

        Ok((text, false))
    }

    /// Whether a file exists - a write tells the model "created" or "replaced" from this.
    pub fn exists(&self, path: &str) -> bool {
        let Ok(resolved) = self.resolve(path) else {
            return false;
        };

        match &self.remote {
            Some(ssh) => crate::ssh::ops::stat(ssh, &resolved).is_ok(),
            None => Path::new(&resolved).is_file(),
        }
    }

    pub fn write(&self, path: &str, text: &str) -> Result<(), ErrorObject> {
        let resolved = self.resolve(path)?;

        match &self.remote {
            Some(ssh) => crate::ssh::ops::write(ssh, &resolved, text).map(|_| ()),
            None => crate::fs::write(Path::new(&resolved), text).map(|_| ()),
        }
    }

    /// One level of a directory, as `dir/` and `file (bytes)` lines, and how many names the guard hid.
    pub fn list(&self, path: &str) -> Result<(Vec<String>, i64), ErrorObject> {
        let resolved = self.resolve(path)?;
        let (entries, hidden) = match &self.remote {
            Some(ssh) => {
                let (_, entries, hidden) = crate::ssh::ops::list(ssh, &resolved)?;

                (entries, hidden)
            }
            None => {
                let (entries, hidden) = crate::fs::list(Path::new(&resolved))?;

                (entries, hidden as i64)
            }
        };

        let lines = entries
            .iter()
            .map(|entry| {
                let name = entry["name"].as_str().unwrap_or_default();

                if entry["dir"].as_bool().unwrap_or(false) {
                    format!("{name}/")
                } else {
                    format!("{name} ({} bytes)", entry["size"].as_u64().unwrap_or(0))
                }
            })
            .collect();

        Ok((lines, hidden))
    }

    /// A literal search under a folder: `path:line: text` lines, capped at `limit`.
    pub fn search(&self, query: &str, path: &str, glob: Option<&str>, limit: usize) -> Result<Vec<String>, ErrorObject> {
        let resolved = self.resolve(path)?;
        let hits = match &self.remote {
            Some(ssh) => crate::ssh::ops::search(ssh, &resolved, query, glob, limit)?,
            None => crate::fs::search(Path::new(&resolved), query, glob, limit)?,
        };

        Ok(hits
            .iter()
            .map(|hit| {
                format!(
                    "{}:{}: {}",
                    self.relative(hit["path"].as_str().unwrap_or_default()),
                    hit["line"].as_u64().unwrap_or(0),
                    hit["text"].as_str().unwrap_or_default()
                )
            })
            .collect())
    }

    /// The folder's `git diff HEAD`, on its own machine.
    pub fn diff(&self) -> Result<String, ErrorObject> {
        match &self.remote {
            Some(ssh) => crate::ssh::ops::git_diff(ssh, &self.root, None),
            None => crate::git::diff(Path::new(&self.root), None),
        }
    }

    /// Runs one command line in the folder, with the deny list first.
    pub fn run(&self, line: &str, timeout: Duration) -> Result<CommandReport, ErrorObject> {
        if let Some(reason) = crate::pty::denied_reason_line(line) {
            return Err(ErrorObject::permission_denied(format!("{line}: {reason}")));
        }

        let timeout = timeout.min(MAX_COMMAND);
        let answer: Value = match &self.remote {
            /* The host's own shell: someone asking for a command on a VPS means that machine's `sh`,
               whatever this laptop runs. */
            Some(ssh) => crate::ssh::ops::shell(ssh, "sh", &["-c".to_string(), line.to_string()], Some(&self.root), timeout)?,
            None => {
                let (command, args) = crate::pty::shell_for_line(line);

                crate::pty::PtyManager::new().run_once(&command, &args, Some(&self.root), timeout)?
            }
        };

        Ok(CommandReport {
            ok: answer["ok"].as_bool().unwrap_or(false),
            exit_code: answer["exitCode"].as_i64(),
            stdout: answer["stdout"].as_str().unwrap_or_default().to_string(),
            stderr: answer["stderr"].as_str().unwrap_or_default().to_string(),
            duration_ms: answer["durationMs"].as_u64().unwrap_or(0),
            timed_out: answer["timedOut"].as_bool().unwrap_or(false),
        })
    }
}

fn outside(raw: &str, root: &str) -> ErrorObject {
    ErrorObject::blocked(&format!(
        "`{raw}` is outside the folder this chat works in ({root}); the agent can only read and write inside it"
    ))
}

/// `..` and `.` removed without touching the disk - a path that does not exist yet (a file about to be
/// created) has to be checked too, so `canonicalize` is not an option.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();

    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }

    out
}

/// The same rule for a host's paths, which are POSIX whatever this machine is.
fn resolve_posix(root: &str, raw: &str) -> Result<String, ErrorObject> {
    let joined = if raw.starts_with('/') { raw.to_string() } else { format!("{root}/{raw}") };
    let mut parts: Vec<&str> = Vec::new();

    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }

    let resolved = format!("/{}", parts.join("/"));
    let root_clean = if root.starts_with('/') { root.to_string() } else { format!("/{root}") };

    if resolved != root_clean && !resolved.starts_with(&format!("{root_clean}/")) {
        return Err(outside(raw, root));
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_lands_inside_the_folder() {
        let root = std::env::temp_dir().join("sdc-agent-ws");
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let resolved = workspace.resolve("src/pay.js").unwrap();

        assert!(Path::new(&resolved).starts_with(normalize(&root)), "{resolved}");
        assert_eq!(workspace.relative(&resolved), "src/pay.js");
    }

    #[test]
    fn climbing_out_of_the_folder_is_refused() {
        let root = std::env::temp_dir().join("sdc-agent-ws");
        let workspace = Workspace::new(root.to_str().unwrap(), None);

        assert!(workspace.resolve("../../etc/passwd").is_err());
        assert!(workspace.resolve("src/../../outside.txt").is_err());
        assert!(workspace.resolve("src/../inside.txt").is_ok());
    }

    #[test]
    fn a_hosts_paths_follow_posix_rules_whatever_this_machine_is() {
        assert_eq!(resolve_posix("/srv/app", "src/pay.js").unwrap(), "/srv/app/src/pay.js");
        assert_eq!(resolve_posix("/srv/app", "/srv/app/x").unwrap(), "/srv/app/x");
        assert_eq!(resolve_posix("/srv/app", ".").unwrap(), "/srv/app");
        assert!(resolve_posix("/srv/app", "../other/x").is_err());
        assert!(resolve_posix("/srv/app", "/srv/application/x").is_err(), "a sibling with a shared prefix is outside");
        assert!(resolve_posix("/srv/app", "/etc/passwd").is_err());
    }

    #[test]
    fn a_file_is_written_read_listed_and_found_inside_the_folder() {
        let root = std::env::temp_dir().join(format!("sdc-agent-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::new(root.to_str().unwrap(), None);

        assert!(!workspace.exists("src/a.txt"));
        workspace.write("src/a.txt", "hello agent\nsecond line\n").unwrap();
        assert!(workspace.exists("src/a.txt"));
        assert_eq!(workspace.read("src/a.txt").unwrap(), ("hello agent\nsecond line\n".to_string(), false));

        let (entries, _) = workspace.list(".").unwrap();
        assert_eq!(entries, vec!["src/".to_string()]);

        let hits = workspace.search("second", ".", None, 10).unwrap();
        assert_eq!(hits, vec!["src/a.txt:2: second line".to_string()]);

        /* The file guard still runs inside the folder. */
        assert!(workspace.write(".env", "SECRET=1").is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_command_runs_in_the_folder_and_the_deny_list_still_applies() {
        let root = std::env::temp_dir();
        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let report = workspace.run("echo sdc-agent", Duration::from_secs(20)).unwrap();

        assert!(report.ok, "{}", report.stderr);
        assert!(report.stdout.contains("sdc-agent"));
        assert!(workspace.run("shutdown -h now", Duration::from_secs(5)).is_err());
    }
}
