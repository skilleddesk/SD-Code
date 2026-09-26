//! The environment doctor - the ten checks of spec section 9.10.
//!
//! Each check is a probe of *this* machine, not a line in a table, and each one ends in one of three
//! states: `ok` (it works), `warn` (it works, but not the way we would like) or `fail` (it does not
//! work, and here is the button that tries to fix it). The `fix` field is what the row's button says:
//! `Install`, `Kill process`, `Re-pin`.
//!
//! The ten, in the order the Provider Hub lists them:
//!
//!   1 node   2 claude   3 codex   4 gemini   5 ollama   6 ripgrep   7 port 3000   8 disk
//!   9 git   10 ssh
//!
//! "Fails gracefully if a tool is missing" is the acceptance item, and it is why every probe returns
//! a `Result`-shaped `Option` instead of unwrapping: a machine with none of the six CLIs installed
//! gets ten honest rows, not a panic.

use std::process::Command;

use serde_json::{json, Value};

use crate::store::sqlite::Store;

/// `(id, label, program, args)`. The args are there because `node -v` is not `node --version` on
/// every tool, and a check that reports the wrong thing is worse than no check.
const TOOLS: &[(&str, &str, &str)] = &[
    ("node", "Node.js", "node"),
    ("claude", "Claude Code CLI", "claude"),
    ("codex", "Codex CLI", "codex"),
    ("gemini", "Gemini CLI", "gemini"),
    ("ollama", "Ollama", "ollama"),
    ("ripgrep", "ripgrep", "rg"),
    ("git", "Git", "git"),
    ("ssh", "SSH", "ssh"),
];

/// The ten checks for this machine. `store` is here so the disk row can name the database file and
/// its size - the one piece of the environment the daemon is uniquely able to report.
pub fn checks(store: &Store) -> Vec<Value> {
    let mut rows: Vec<Value> = TOOLS
        .iter()
        .map(|(id, label, program)| match version_of(program) {
            Some(version) => json!({ "id": id, "label": label, "state": "ok", "detail": version }),
            None => json!({
                "id": id,
                "label": label,
                "state": "fail",
                "detail": "not installed",
                "fix": "Install",
            }),
        })
        .collect();

    /* 7 - the busy port the prototype's example complains about. */
    rows.push(match std::net::TcpListener::bind(("127.0.0.1", 3000)) {
        Ok(_) => json!({ "id": "port3000", "label": "Port 3000", "state": "ok", "detail": "free" }),
        Err(_) => json!({
            "id": "port3000",
            "label": "Port 3000",
            "state": "warn",
            "detail": "already in use",
            "fix": "Kill process",
        }),
    });

    /* 8 - the disk, and with it the daemon's own footprint. */
    let database = store.path();
    let size: u64 = std::fs::metadata(&database).map(|meta| meta.len()).unwrap_or(0);

    rows.push(json!({
        "id": "disk",
        "label": "Disk space",
        "state": "ok",
        "detail": format!("{database} · {} KB · {} events", size / 1024, store.event_count().unwrap_or(0)),
    }));

    rows
}

/// Probes one program's version. A missing program is not an error here: it is a `fail` row.
///
/// The lookup goes through `host::program`, which follows the platform's own rules - and that matters
/// most on Windows, where an npm-installed CLI is a `.cmd` shim that `Command::new("claude")` cannot
/// find. Before that fix this function reported `claude`, `codex` and `gemini` as missing on a machine
/// where all three ran.
pub fn version_of(program: &str) -> Option<String> {
    let (executable, prefix) = crate::host::program::launch(program)?;
    let mut command = Command::new(executable);

    let output = command.args(prefix).arg("--version").output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = stdout.lines().next().unwrap_or("").trim();

    Some(if first.is_empty() { format!("{program} present") } else { first.to_string() })
}

/// True when a program can be found and run. Used by the provider flows to decide whether a CLI
/// provider can be connected.
pub fn has(program: &str) -> bool {
    version_of(program).is_some()
}

/// The checks for a **host**: the same question the local ten ask, answered about that machine (0.7.13).
///
/// The local list probes programs on this computer, which is exactly the wrong answer for a VPS - and it
/// is what `host.doctor { hostId }` returned for a host before this release: ten rows about the laptop,
/// under a heading that said the host's name. Now a remote host gets rows about *it*:
///
/// | id | what it asks | how |
/// | -- | ------------ | --- |
/// | `ssh` | can this machine be reached | the probe (`ssh::ops::probe`), whose sentence is the detail |
/// | `hostkey` | is its key the pinned one | `ssh::hostkey::inspect` - and `Re-pin` is the fix when it is not |
/// | `git` | is `git` there | one `--version` |
/// | `claude`, `codex`, `gemini` | can a turn run there | one `--version` each, which is the question a chat on that host will ask later |
/// | `home` | is `$HOME` writable | `test -w`, because the shadow repository lives under it |
/// | `folder` | does the chat's folder exist there | `test -d`, when the caller named one |
///
/// The `fix` values are the two the window can actually perform (`Trust`, `Re-pin` - the Add-host
/// dialog's trust card) and nothing else: a `Fix` button that toasts "Install: done" while installing
/// nothing is the kind of small lie this build keeps removing, and `detail` is where an instruction
/// belongs.
pub fn remote_checks(ssh: &crate::ssh::Ssh, root: Option<&str>) -> Vec<Value> {
    let mut rows: Vec<Value> = Vec::new();
    /* Asked once, because two rows are decided by it: `ssh` (can a session be made at all) and `hostkey`
       (is the machine the one SDC pinned). */
    let trust = crate::ssh::hostkey::inspect(&ssh.target);
    let (status, detail) = crate::ssh::ops::probe(ssh);

    /*
     * The `ssh` row's **fix**, when there is one a surface can carry out (0.7.13 - after the report that
     * this row was red with nothing to click).
     *
     * It is derived from the trust state, not from the sentence: a host whose key is unknown needs the
     * `Trust` decision, a changed one needs `Re-pin`, and a host whose pin is *in place* and whose probe
     * still failed is a host that does not accept SDC's key yet - the one case where a password is needed
     * once, and the card is where that field lives. A machine that is simply down gets no button, because
     * there is no button for "the machine is down".
     */
    let ssh_fix = match (&trust, status.as_str()) {
        (_, "connected") => None,
        (Ok(crate::ssh::hostkey::Trust::Unknown(_)), _) => Some("Trust"),
        (Ok(crate::ssh::hostkey::Trust::Changed { .. }), _) => Some("Re-pin"),
        /* With an `ssh` that can hold a connection open, the card signs in (0.8.1); without one it can
           only copy SDC's key over. */
        (Ok(crate::ssh::hostkey::Trust::Pinned(_)), _) if crate::ssh::session::program().is_some() => Some("Sign in"),
        (Ok(crate::ssh::hostkey::Trust::Pinned(_)), _) => Some("Install key"),
        (Err(_), _) => None,
    };

    let mut ssh_row = json!({
        "id": "ssh",
        "label": format!("SSH to {}", ssh.label()),
        "state": if status == "connected" { "ok" } else { "fail" },
        "detail": detail,
    });

    if let Some(fix) = ssh_fix {
        ssh_row["fix"] = json!(fix);
    }

    rows.push(ssh_row);

    rows.push(match &trust {
        Ok(crate::ssh::hostkey::Trust::Pinned(key)) => json!({
            "id": "hostkey",
            "label": "Host key",
            "state": "ok",
            "detail": format!("pinned · {}", key.fingerprint),
        }),
        Ok(crate::ssh::hostkey::Trust::Unknown(keys)) => {
            let fingerprint = crate::ssh::hostkey::primary(keys)
                .map(|key| key.fingerprint.clone())
                .unwrap_or_default();

            json!({
                "id": "hostkey",
                "label": "Host key",
                "state": "fail",
                "detail": format!("{fingerprint} · never trusted"),
                "fix": "Trust",
            })
        }
        Ok(crate::ssh::hostkey::Trust::Changed { pinned, seen }) => json!({
            "id": "hostkey",
            "label": "Host key",
            "state": "fail",
            "detail": format!(
                "changed — needs re-pin · pinned {}{}",
                pinned.first().cloned().unwrap_or_else(|| "nothing".to_string()),
                crate::ssh::hostkey::primary(seen)
                    .map(|key| format!(", now {}", key.fingerprint))
                    .unwrap_or_default()
            ),
            "fix": "Re-pin",
        }),
        Err(error) => json!({
            "id": "hostkey",
            "label": "Host key",
            "state": "warn",
            "detail": error.message,
        }),
    });

    rows.extend(if status == "connected" {
        let mut checked = tool_rows(ssh);

        checked.push(match crate::ssh::ops::writable_home(ssh) {
            Ok(true) => json!({ "id": "home", "label": "Home directory", "state": "ok", "detail": "writable" }),
            Ok(false) => json!({
                "id": "home",
                "label": "Home directory",
                "state": "fail",
                "detail": "not writable, so SDC cannot keep a checkpoint there",
            }),
            Err(error) => json!({ "id": "home", "label": "Home directory", "state": "warn", "detail": error.message }),
        });

        checked
    } else {
        /* Nothing else can be measured without a session, and pretending otherwise is how a doctor
           ends up saying "not installed" about programs on a machine it never logged into: the one row
           that matters (`hostkey`, which carries `Trust`/`Re-pin`) is above, and this says the rest was
           not asked. */
        vec![json!({
            "id": "notreached",
            "label": "Everything else",
            "state": "warn",
            "detail": "not checked — SDC could not get a session on that host",
        })]
    });

    if status == "connected" {
        if let Some(root) = root {
            rows.push(match crate::ssh::ops::is_dir(ssh, root) {
                Ok(true) => json!({ "id": "folder", "label": "Chat's folder", "state": "ok", "detail": root }),
                Ok(false) => json!({
                    "id": "folder",
                    "label": "Chat's folder",
                    "state": "fail",
                    "detail": format!("{root} is not a folder on that host"),
                }),
                Err(error) => json!({ "id": "folder", "label": "Chat's folder", "state": "warn", "detail": error.message }),
            });
        }
    }

    rows
}

/// The programs a turn on that host needs, asked in **one** round trip: `name=version` per line, or
/// `name=not installed`.
fn tool_rows(ssh: &crate::ssh::Ssh) -> Vec<Value> {
    let script = "for p in git claude codex gemini rg node; do printf '%s=' \"$p\"; if command -v \"$p\" >/dev/null 2>&1; then \"$p\" --version 2>/dev/null | head -n 1 || echo present; else echo 'not installed'; fi; done";
    let labels = [
        ("git", "Git"),
        ("claude", "Claude Code CLI"),
        ("codex", "Codex CLI"),
        ("gemini", "Gemini CLI"),
        ("rg", "ripgrep"),
        ("node", "Node.js"),
    ];

    let Ok(output) = ssh.run(script, std::time::Duration::from_secs(30)) else {
        return Vec::new();
    };

    if !output.ok() {
        return Vec::new();
    }

    labels
        .iter()
        .map(|(id, label)| {
            let version = output
                .stdout
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{id}=")))
                .unwrap_or("")
                .trim()
                .to_string();
            let missing = version.is_empty() || version.eq_ignore_ascii_case("not installed");

            json!({
                "id": id,
                "label": format!("{label} (on the host)"),
                "state": if missing { "warn" } else { "ok" },
                "detail": if missing { "not installed".to_string() } else { version },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_exactly_ten_checks_with_the_five_required_fields() {
        let store = Store::in_memory().unwrap();
        let rows = checks(&store);

        assert_eq!(rows.len(), 10);

        for row in &rows {
            assert!(row["id"].is_string(), "every check has an id");
            assert!(row["label"].is_string());
            assert!(row["detail"].is_string());

            let state = row["state"].as_str().unwrap_or_default();

            assert!(["ok", "warn", "fail"].contains(&state));
        }
    }

    #[test]
    fn a_missing_tool_is_a_fail_row_and_not_a_panic() {
        let store = Store::in_memory().unwrap();
        let rows = checks(&store);
        let node = rows.iter().find(|row| row["id"] == "node").unwrap();

        /* Whatever this machine has, the row is well formed - that is the graceful failure. */
        assert!(node["state"] == "ok" || node["detail"] == "not installed");
        assert!(version_of("definitely-not-a-program-sdcd").is_none());
    }
}
