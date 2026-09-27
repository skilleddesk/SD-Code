//! Release safety (0.12): crash reports, the CLI self-check, and the update channels.
//!
//! * **Crash reports** are written locally when the daemon panics - the version, the platform, where and
//!   why - and never sent anywhere by SDC. The window lists them; sending one is the person's click, which
//!   opens a pre-filled GitHub issue they can read and edit first (opt-in, always).
//! * **The CLI self-check** says, for each coding CLI, whether it is installed, which version, and whether
//!   its sign-in file exists - without starting a paid turn to find out.
//! * **Update channels**: `stable` is the newest full release; `beta` also takes pre-releases. The answer
//!   names the release before the current one too, which is the rollback when an update misbehaves.

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

/// The repository releases are published from.
pub const REPOSITORY: &str = "skilleddesk/SD-Code";

fn crash_dir() -> Option<PathBuf> {
    let directory = crate::paths::data_dir().ok()?.join("crash");

    std::fs::create_dir_all(&directory).ok()?;

    Some(directory)
}

/// Installs the panic hook: a report file, then the default hook (so the terminal still shows it).
pub fn install_hook() {
    let default = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        if let Some(directory) = crash_dir() {
            let at = chrono::Utc::now();
            let location = info.location().map(|location| format!("{}:{}", location.file(), location.line())).unwrap_or_default();
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|text| text.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic without a message".into());
            let report = json!({
                "version": crate::VERSION,
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "at": at.to_rfc3339(),
                "thread": std::thread::current().name().unwrap_or("unnamed"),
                "location": location,
                "message": message,
                "backtrace": std::backtrace::Backtrace::force_capture().to_string().lines().take(60).collect::<Vec<_>>().join("\n"),
            });

            let _ = std::fs::write(directory.join(format!("sdcd-{}.json", at.format("%Y%m%d-%H%M%S"))), serde_json::to_string_pretty(&report).unwrap_or_default());
        }

        default(info);
    }));
}

/// The crash reports on this machine, newest first, each with the issue link a person may choose to open.
pub fn list() -> Vec<Value> {
    let Some(directory) = crash_dir() else {
        return Vec::new();
    };
    let mut reports: Vec<(String, Value)> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let text = std::fs::read_to_string(entry.path()).ok()?;

                    Some((name, serde_json::from_str::<Value>(&text).ok()?))
                })
                .collect()
        })
        .unwrap_or_default();

    reports.sort_by(|a, b| b.0.cmp(&a.0));

    reports
        .into_iter()
        .map(|(name, report)| {
            let title = format!("sdcd {} crashed: {}", report["version"].as_str().unwrap_or("?"), report["message"].as_str().unwrap_or("?").chars().take(80).collect::<String>());
            let body = format!(
                "**Version:** {}\n**Platform:** {} {}\n**Where:** {}\n**Message:** {}\n\n```\n{}\n```\n\n(What were you doing when it happened?)",
                report["version"].as_str().unwrap_or("?"),
                report["os"].as_str().unwrap_or("?"),
                report["arch"].as_str().unwrap_or("?"),
                report["location"].as_str().unwrap_or("?"),
                report["message"].as_str().unwrap_or("?"),
                report["backtrace"].as_str().unwrap_or_default().chars().take(3000).collect::<String>()
            );
            let issue = format!("https://github.com/{REPOSITORY}/issues/new?title={}&body={}", encode(&title), encode(&body));

            json!({ "file": name, "report": report, "issueUrl": issue })
        })
        .collect()
}

pub fn clear() -> usize {
    let Some(directory) = crash_dir() else {
        return 0;
    };

    std::fs::read_dir(directory)
        .map(|entries| entries.filter_map(Result::ok).filter(|entry| std::fs::remove_file(entry.path()).is_ok()).count())
        .unwrap_or(0)
}

/// Percent-encoding for a URL's query value.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Each coding CLI: installed, its version, and whether its sign-in file is there.
pub fn cli_selfcheck() -> Vec<Value> {
    let home = dirs::home_dir().unwrap_or_default();

    [
        ("claude", "Claude Code", vec![home.join(".claude").join(".credentials.json"), home.join(".claude.json")]),
        ("codex", "Codex", vec![home.join(".codex").join("auth.json")]),
        ("gemini", "Gemini CLI", vec![home.join(".gemini").join("oauth_creds.json"), home.join(".gemini").join("settings.json")]),
    ]
    .into_iter()
    .map(|(program, label, files)| {
        let version = crate::host::doctor::version_of(program);
        let signed_in = files.iter().any(|file| file.exists());

        json!({
            "program": program,
            "label": label,
            "installed": version.is_some(),
            "version": version,
            "signedIn": signed_in,
            "sentence": match (version.is_some(), signed_in) {
                (false, _) => format!("{label} is not installed on this machine."),
                (true, false) => format!("{label} is installed but not signed in - connect it in the Provider Hub."),
                (true, true) => format!("{label} is installed and signed in."),
            },
        })
    })
    .collect()
}

/// `0.12.0` → `(0, 12, 0)`; a pre-release suffix sorts before its release.
pub fn version_key(tag: &str) -> (u64, u64, u64, u64) {
    let bare = tag.trim_start_matches('v');
    let (numbers, pre) = bare.split_once('-').map(|(numbers, pre)| (numbers, Some(pre))).unwrap_or((bare, None));
    let mut parts = numbers.split('.').map(|part| part.parse::<u64>().unwrap_or(0));

    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0), if pre.is_some() { 0 } else { 1 })
}

/// The newest release on a channel, and the one before the running version (the rollback).
pub fn choose(releases: &[Value], channel: &str, current: &str) -> Value {
    let mut usable: Vec<&Value> = releases
        .iter()
        .filter(|release| release["draft"] != true)
        .filter(|release| channel == "beta" || release["prerelease"] != true)
        .collect();

    usable.sort_by_key(|release| std::cmp::Reverse(version_key(release["tag_name"].as_str().unwrap_or("0"))));

    let current_key = version_key(current);
    let latest = usable.first();
    let previous = usable.iter().find(|release| version_key(release["tag_name"].as_str().unwrap_or("0")) < current_key);
    let describe = |release: &&Value| {
        json!({
            "tag": release["tag_name"],
            "name": release["name"],
            "url": release["html_url"],
            "prerelease": release["prerelease"],
            "publishedAt": release["published_at"],
            "assets": release["assets"].as_array().map(|assets| assets.iter().map(|asset| json!({ "name": asset["name"], "url": asset["browser_download_url"] })).collect::<Vec<_>>()),
        })
    };

    json!({
        "channel": channel,
        "current": current,
        "latest": latest.map(describe),
        "updateAvailable": latest.is_some_and(|release| version_key(release["tag_name"].as_str().unwrap_or("0")) > current_key),
        "previous": previous.map(describe),
    })
}

pub fn update_check(channel: &str) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(8)).timeout_read(Duration::from_secs(15)).build();
    let response = agent
        .get(&format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=30"))
        .set("accept", "application/vnd.github+json")
        .set("user-agent", &format!("sdcd/{}", crate::VERSION))
        .call()
        .map_err(|error| format!("GitHub could not be asked about updates: {error}"))?;
    let mut body = String::new();

    response.into_reader().take(5_000_000).read_to_string(&mut body).map_err(|error| error.to_string())?;

    let releases: Vec<Value> = serde_json::from_str(&body).map_err(|error| format!("GitHub's answer was not a release list: {error}"))?;

    Ok(choose(&releases, channel, crate::VERSION))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, pre: bool) -> Value {
        json!({ "tag_name": tag, "name": tag, "prerelease": pre, "draft": false, "html_url": format!("https://x/{tag}"), "assets": [] })
    }

    #[test]
    fn channels_pick_the_right_release_and_name_the_rollback() {
        let releases = vec![release("v0.11.9", false), release("v0.12.0", false), release("v0.13.0-beta.1", true)];
        let stable = choose(&releases, "stable", "0.12.0");
        let beta = choose(&releases, "beta", "0.12.0");

        assert_eq!(stable["latest"]["tag"], "v0.12.0");
        assert_eq!(stable["updateAvailable"], false);
        assert_eq!(stable["previous"]["tag"], "v0.11.9");
        assert_eq!(beta["latest"]["tag"], "v0.13.0-beta.1");
        assert_eq!(beta["updateAvailable"], true);
        assert!(version_key("0.13.0-beta.1") < version_key("0.13.0"));
    }

    #[test]
    fn issue_links_are_encoded() {
        assert_eq!(encode("a b/ক"), "a%20b%2F%E0%A6%95");
    }

    #[test]
    fn the_self_check_names_every_cli() {
        let clis = cli_selfcheck();

        assert_eq!(clis.len(), 3);
        assert!(clis.iter().all(|cli| cli["sentence"].as_str().is_some()));
    }
}
