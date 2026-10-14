//! AUTO: an automation is a request SDC runs on a clock.
//!
//! The person writes them in `.sdc/automations.toml` - each one a name, a request, and when it runs (`every`
//! for a gap, `at` for a time of day). Whatever a run produces is written into the Inbox (`.sdc/inbox/`), so
//! the answer is read when the person is ready instead of watched while it happens, and the last run of each
//! one is remembered next to it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::TimeZone;
use serde::{Deserialize, Serialize};

/// One request SDC runs on a clock.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Automation {
    pub name: String,
    pub request: String,
    /// How often it runs: `30m`, `6h`, `2d`.
    #[serde(default)]
    pub every: Option<String>,
    /// Or a time of day, local: `09:30`.
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
struct All {
    #[serde(default, rename = "automation")]
    items: Vec<Automation>,
}

/// One run waiting in the Inbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub name: String,
    /// When the run happened (unix seconds).
    pub at: i64,
    pub request: String,
    /// What SDC answered, in short.
    pub summary: String,
    /// The file the whole answer is in, relative to the project.
    pub file: String,
}

/// The file the automations are written in.
pub fn file(root: &Path) -> PathBuf {
    root.join(".sdc/automations.toml")
}

/// Where the answers collect.
pub fn inbox(root: &Path) -> PathBuf {
    root.join(".sdc/inbox")
}

fn state_file(root: &Path) -> PathBuf {
    inbox(root).join(".state.json")
}

/// Every automation the person wrote, enabled or not.
pub fn load(root: &Path) -> Result<Vec<Automation>, String> {
    let path = file(root);

    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let all: All = toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;

    for item in &all.items {
        if item.every.is_none() && item.at.is_none() {
            return Err(format!("`{}` says neither `every` nor `at`, so it would never run", item.name));
        }

        if item.every.as_deref().is_some_and(|text| interval(text).is_none()) {
            return Err(format!("`{}` has an `every` I cannot read (use `30m`, `6h`, `2d`)", item.name));
        }

        if item.at.as_deref().is_some_and(|text| time_of_day(text).is_none()) {
            return Err(format!("`{}` has an `at` I cannot read (use `09:30`)", item.name));
        }
    }

    Ok(all.items)
}

/// `30m`, `6h`, `2d` as a number of seconds.
pub fn interval(text: &str) -> Option<i64> {
    let text = text.trim();
    let (number, unit) = text.split_at(text.len().checked_sub(1)?);
    let number: i64 = number.trim().parse().ok()?;

    match unit {
        "s" => Some(number),
        "m" => Some(number * 60),
        "h" => Some(number * 3600),
        "d" => Some(number * 86_400),
        _ => None,
    }
    .filter(|seconds| *seconds > 0)
}

/// `09:30` as seconds after local midnight.
pub fn time_of_day(text: &str) -> Option<i64> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let hours: i64 = hours.trim().parse().ok()?;
    let minutes: i64 = minutes.trim().parse().ok()?;

    ((0..24).contains(&hours) && (0..60).contains(&minutes)).then_some(hours * 3600 + minutes * 60)
}

/// The instant today's `09:30` last fell on, at or before `now`.
fn latest_at(text: &str, now: i64) -> Option<i64> {
    let seconds = time_of_day(text)?;
    let local = chrono::Local.timestamp_opt(now, 0).single()?;
    let midnight = local.date_naive().and_hms_opt(0, 0, 0)?;
    let today = chrono::Local.from_local_datetime(&midnight).single()?.timestamp();
    let candidate = today + seconds;

    Some(if candidate > now { candidate - 86_400 } else { candidate })
}

/// The automations that should run now: their gap has passed, or the time of day has come round since they
/// last ran. A brand new one runs the first time SDC looks.
pub fn due(items: &[Automation], last: &BTreeMap<String, i64>, now: i64) -> Vec<Automation> {
    items
        .iter()
        .filter(|item| item.enabled)
        .filter(|item| {
            let ran_at = last.get(&item.name).copied();

            if let Some(seconds) = item.every.as_deref().and_then(interval) {
                return ran_at.is_none_or(|at| now - at >= seconds);
            }

            latest_at(item.at.as_deref().unwrap_or_default(), now).is_some_and(|candidate| ran_at.is_none_or(|at| at < candidate))
        })
        .cloned()
        .collect()
}

/// When each automation last ran.
pub fn last_runs(root: &Path) -> BTreeMap<String, i64> {
    std::fs::read_to_string(state_file(root)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Remember that one ran at `at`.
pub fn mark(root: &Path, name: &str, at: i64) -> Result<(), String> {
    let mut last = last_runs(root);

    last.insert(name.to_string(), at);

    let text = serde_json::to_string_pretty(&last).map_err(|error| error.to_string())?;

    std::fs::create_dir_all(inbox(root)).map_err(|error| format!("{}: {error}", inbox(root).display()))?;
    std::fs::write(state_file(root), text).map_err(|error| format!("{}: {error}", state_file(root).display()))
}

/// Write one run's answer into the Inbox and remember it happened.
pub fn record(root: &Path, automation: &Automation, at: i64, summary: &str) -> Result<Note, String> {
    let dir = inbox(root);

    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;

    let name = format!("{at}-{}.md", slug(&automation.name));
    let body = format!(
        "---\nname: {}\nat: {at}\nrequest: {}\n---\n\n# {} - {}\n\n{}\n",
        automation.name,
        automation.request,
        automation.name,
        stamp(at),
        summary.trim()
    );

    std::fs::write(dir.join(&name), &body).map_err(|error| format!("{}: {error}", dir.join(&name).display()))?;
    mark(root, &automation.name, at)?;

    Ok(Note { name: automation.name.clone(), at, request: automation.request.clone(), summary: first_lines(summary, 3), file: format!(".sdc/inbox/{name}") })
}

/// What is waiting in the Inbox, newest first.
pub fn notes(root: &Path) -> Vec<Note> {
    let dir = inbox(root);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut notes: Vec<Note> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".md"))
        .filter_map(|entry| {
            let text = std::fs::read_to_string(entry.path()).ok()?;
            let field = |key: &str| text.lines().find_map(|line| line.strip_prefix(&format!("{key}: ")).map(str::to_string));
            let relative = entry.path().strip_prefix(root).unwrap_or(&entry.path()).to_string_lossy().to_string();

            Some(Note {
                name: field("name")?,
                at: field("at")?.parse().ok()?,
                request: field("request").unwrap_or_default(),
                summary: first_lines(&text.split("---").nth(2).unwrap_or_default().lines().filter(|line| !line.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n"), 3),
                file: relative,
            })
        })
        .collect();

    notes.sort_by_key(|note| std::cmp::Reverse(note.at));

    notes
}

/// `2026-10-14 09:30` in the person's own time.
fn stamp(at: i64) -> String {
    chrono::Local.timestamp_opt(at, 0).single().map(|time| time.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| at.to_string())
}

/// A name a file can carry.
fn slug(name: &str) -> String {
    name.chars().map(|letter| if letter.is_ascii_alphanumeric() { letter.to_ascii_lowercase() } else { '-' }).collect::<String>().trim_matches('-').to_string().chars().take(40).collect()
}

fn first_lines(text: &str, count: usize) -> String {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).take(count).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-auto-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sdc")).unwrap();
        root
    }

    #[test]
    fn the_gaps_and_times_a_person_writes_are_read() {
        assert_eq!(interval("30m"), Some(1800));
        assert_eq!(interval("6h"), Some(21_600));
        assert_eq!(interval("2d"), Some(172_800));
        assert_eq!(interval("0m"), None, "a zero gap would loop");
        assert_eq!(interval("soon"), None);
        assert_eq!(time_of_day("09:30"), Some(34_200));
        assert_eq!(time_of_day("24:00"), None);
        assert_eq!(time_of_day("9:70"), None);
    }

    #[test]
    fn only_what_is_due_runs_and_each_one_remembers_its_last_time() {
        let root = project("due");

        std::fs::write(
            file(&root),
            "[[automation]]\nname = \"nightly\"\nrequest = \"run the tests and fix what fails\"\nevery = \"6h\"\n\n[[automation]]\nname = \"morning\"\nrequest = \"find what broke overnight\"\nat = \"09:30\"\n\n[[automation]]\nname = \"off\"\nrequest = \"never\"\nevery = \"1h\"\nenabled = false\n",
        )
        .unwrap();

        let items = load(&root).unwrap();

        assert_eq!(items.len(), 3, "a disabled one is still the person's automation");

        /* Local noon, whatever the machine's zone: 09:30 never comes round within the next six hours, so only the
           gap decides. (A fixed instant fell either side of 09:30 depending on where the tests ran.) */
        let now = chrono::Local.with_ymd_and_hms(2025, 10, 9, 12, 0, 0).unwrap().timestamp();
        let first = due(&items, &BTreeMap::new(), now);

        assert_eq!(first.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(), vec!["nightly", "morning"], "a new one runs the first time SDC looks");

        let mut last = BTreeMap::new();

        last.insert("nightly".to_string(), now);
        last.insert("morning".to_string(), now);

        assert!(due(&items, &last, now + 3600).is_empty(), "an hour later neither is due");
        assert_eq!(due(&items, &last, now + 21_600).len(), 1, "six hours later the gap has passed");
    }

    #[test]
    fn a_broken_schedule_says_so_instead_of_never_running() {
        let root = project("broken");

        std::fs::write(file(&root), "[[automation]]\nname = \"x\"\nrequest = \"y\"\n").unwrap();
        assert!(load(&root).unwrap_err().contains("neither `every` nor `at`"));

        std::fs::write(file(&root), "[[automation]]\nname = \"x\"\nrequest = \"y\"\nevery = \"sometimes\"\n").unwrap();
        assert!(load(&root).unwrap_err().contains("cannot read"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_answer_lands_in_the_inbox_and_is_listed_newest_first() {
        let root = project("inbox");
        let automation = Automation { name: "Nightly Tests".to_string(), request: "run the tests".to_string(), every: Some("6h".to_string()), at: None, enabled: true };

        let note = record(&root, &automation, 1_760_000_000, "Two tests failed. Both were fixed.\nDetails below.").unwrap();

        assert_eq!(note.file, ".sdc/inbox/1760000000-nightly-tests.md");
        assert!(note.summary.contains("Two tests failed"));
        assert_eq!(last_runs(&root).get("Nightly Tests"), Some(&1_760_000_000));

        record(&root, &automation, 1_760_003_600, "All green.").unwrap();

        let listed = notes(&root);

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "Nightly Tests");
        assert_eq!(listed[0].summary, "All green.", "the newest is first");
        assert_eq!(listed[1].request, "run the tests");
        assert!(std::fs::read_to_string(root.join(&note.file)).unwrap().contains("run the tests"), "the whole answer is kept, not just the first lines");

        let _ = std::fs::remove_dir_all(&root);
    }
}
