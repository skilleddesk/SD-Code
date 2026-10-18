//! A turn nobody typed: the daemon's own clock (AUTO) and the board's Start (PAR).
//!
//! Both features end the same way - a folder of its own, a chat in it, a turn started there - and both need
//! the piece a window cannot provide: something that is still there when the turn ends, to write down what
//! came of it. That is [`watch`]: the daemon keeps the events of a turn in memory, so a small thread reading
//! `TurnCompleted` (or `ErrorRaised`) for one turn id is enough to put the answer in the Inbox or on a card.
//!
//! The clock is [`spawn`], which calls [`crate::sdcp::methods::Daemon::run_automation`] - the same method the
//! Inbox's Run button calls, so a run started by hand and a run started by the clock cannot drift apart.
//!
//! Nothing here runs a turn of its own: an automation that is not due starts nothing, and the whole clock is
//! behind `v3::on("automations")` (2.0).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::sdcp::events::StoredEvent;
use crate::sdcp::methods::Daemon;
use crate::sdcp::notifications::ChannelNotifier;
use crate::sdcp::Notifier;
use crate::DaemonState;

/// How long the watcher listens for its turn before it stops. A long turn is not a fault - this is the point
/// at which the daemon stops holding a thread for it, not a limit on the turn.
const WATCH_FOR: Duration = Duration::from_secs(4 * 60 * 60);

/// How often the watcher reads the events. They are in memory, so this is a lock and a scan.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// How often the clock asks whether anything is due. A minute is the finest `every`/`at` express anyway.
pub const TICK_EVERY: Duration = Duration::from_secs(60);

/// How a turn ended, as the watcher saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Ended {
    /// The turn's own closing words - empty when it never finished.
    pub summary: String,
    /// The error it ended on, when it ended on one.
    pub error: Option<String>,
}

impl Ended {
    /// Whether the turn got to the end of its work.
    pub fn finished(&self) -> bool {
        self.error.is_none() && !self.summary.is_empty()
    }
}

/// Read the end of `turn_id` and hand it to `then`. Returns at once: the turn is already running, and the
/// caller has already answered its connection.
pub fn watch<F: FnOnce(Ended) + Send + 'static>(state: Arc<DaemonState>, turn_id: String, then: F) {
    /* Read the log's position here, before the thread exists: a turn can end before the thread has started
       (a refusal comes back in milliseconds), and a watcher that began reading after that would wait forever
       for an event that had already gone past. */
    let since = state.events.seq();

    std::thread::spawn(move || {
        let mut since = since;
        let deadline = Instant::now() + WATCH_FOR;

        loop {
            std::thread::sleep(LOOK_EVERY);

            for StoredEvent { seq, event, .. } in state.events.since(since) {
                since = since.max(seq);

                if event.get("turnId").and_then(Value::as_str) != Some(turn_id.as_str()) {
                    continue;
                }

                match event.get("type").and_then(Value::as_str) {
                    Some("TurnCompleted") => {
                        then(Ended {
                            summary: event.get("summary").and_then(Value::as_str).unwrap_or_default().to_string(),
                            error: None,
                        });

                        return;
                    }
                    Some("ErrorRaised") => {
                        /* A turn that failed says so here, with the turn id it happened in - the card and the
                           Inbox note have to know, or a failure reads as "still working". */
                        then(Ended {
                            summary: event.get("explanation").and_then(Value::as_str).unwrap_or_default().to_string(),
                            error: Some(event.get("title").and_then(Value::as_str).unwrap_or("the turn failed").to_string()),
                        });

                        return;
                    }
                    _ => {}
                }
            }

            /* A daemon that is shutting down must not leave a thread behind per run. */
            if Instant::now() > deadline {
                return;
            }
        }
    });
}

/// The daemon's clock: every minute, start what is due. Keeps running until the process ends.
pub fn spawn(state: Arc<DaemonState>) {
    std::thread::spawn(move || {
        let mut last = Instant::now() - TICK_EVERY;

        loop {
            std::thread::sleep(Duration::from_secs(2));

            if last.elapsed() < TICK_EVERY {
                continue;
            }

            last = Instant::now();
            run_due(&state);
            nightly_pass(&state);
            daily_brief(&state, chrono::Local::now());
        }
    });
}

/// The clock's other job (Phase 0.5b): every project with knowledge candidates gets the nightly pass once a day.
/// Answers how many projects ran it, which is what the tests read.
pub fn nightly_pass(state: &Arc<DaemonState>) -> usize {
    let now = chrono::Utc::now().timestamp();

    state
        .store
        .projects()
        .unwrap_or_default()
        .iter()
        .filter_map(|project| project["root"].as_str())
        .filter(|root| crate::v3::knowledge_core::nightly_if_due(std::path::Path::new(root), now).is_some())
        .count()
}

/// The clock's third job (Phase 4.8): once a day, from 07:00 local time, when the person turned it on (`brief.daily` = `on`), the morning brief
/// is composed and pushed as a notification (`HealthAlert` on the `brief` pseudo-site, which the window turns into a desktop alert). Answers
/// whether it ran. Off by default: a notification nobody asked for is a nuisance.
pub fn daily_brief(state: &Arc<DaemonState>, now: chrono::DateTime<chrono::Local>) -> bool {
    use chrono::Timelike;

    if state.store.setting("brief.daily").ok().flatten().as_deref() != Some("on") || now.hour() < 7 {
        return false;
    }

    let today = now.format("%Y-%m-%d").to_string();

    if state.store.setting("brief.last").ok().flatten().as_deref() == Some(today.as_str()) {
        return false;
    }

    let _ = state.store.set_setting("brief.last", &today);

    let lang = state.store.setting("ui.language").ok().flatten().unwrap_or_else(|| "en".into());
    let brief = crate::ops::brief::morning(&state.store, chrono::Utc::now());
    let out = ChannelNotifier::new(state.events.clone(), state.fanout.clone());

    out.push(crate::sdcp::events::event::health_alert("brief", "Morning brief", if brief["attention"].as_u64().unwrap_or(0) > 0 { "warn" } else { "info" }, &crate::ops::brief::headline(&brief, &lang)), None, None);

    true
}

/// One turn of the clock: every project whose folder has automations, and every automation of it that is due.
/// Answers how many runs it started, which is what the tests read.
pub fn run_due(state: &Arc<DaemonState>) -> usize {
    if !crate::v3::on("automations") {
        return 0;
    }

    let projects = state.store.projects().unwrap_or_default();
    let daemon = Daemon::new(state.clone());
    let out: Arc<dyn Notifier> = Arc::new(ChannelNotifier::new(state.events.clone(), state.fanout.clone()));
    let now = chrono::Utc::now().timestamp();
    let mut started = 0;

    for project in projects {
        let Some(root) = project["root"].as_str().map(std::path::PathBuf::from) else {
            continue;
        };

        let Ok(written) = crate::v3::automations::load(&root) else {
            continue;
        };

        if written.is_empty() {
            continue;
        }

        let due = crate::v3::automations::due(&written, &crate::v3::automations::last_runs(&root), now);

        for automation in due {
            /* A run the person can see: the same method the Inbox's Run button calls. A refusal - a project
               that has since gone, an engine that is not connected - is left for the note that run writes. */
            if let Err(error) = daemon.run_automation(&root, &automation.name, out.clone()) {
                /* Marked even though nothing ran: without the mark the clock asks again every minute for as
                   long as the reason lasts (an unplugged engine, a folder that has gone), and a daemon that
                   retries a broken run every minute is worse than one that waits for its interval. */
                eprintln!("automation `{}` did not start: {}", automation.name, error.message);

                if let Err(error) = crate::v3::automations::mark(&root, &automation.name, now) {
                    eprintln!("automation `{}` could not be marked as tried: {error}", automation.name);
                }

                continue;
            }

            started += 1;
        }
    }

    started
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdcp::notifications::ChannelNotifier;
    use serde_json::json;
    use std::sync::mpsc;

    fn state() -> Arc<DaemonState> {
        DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon for the test")
    }

    /// The watcher is the only thing that turns a turn's last event into a card or an Inbox note, and it runs
    /// on another thread: a test that only checked the turn had started would pass with the note never written.
    #[test]
    fn the_watcher_reads_the_end_of_its_own_turn_and_no_other() {
        let daemon = state();
        let (send, receive) = mpsc::channel();
        let notifier = ChannelNotifier::new(daemon.events.clone(), daemon.fanout.clone());

        watch(daemon.clone(), "turn-1".to_string(), move |ended| {
            let _ = send.send(ended);
        });

        notifier.push(
            json!({ "type": "TurnCompleted", "sessionId": "s1", "turnId": "turn-2", "summary": "another chat's turn" }),
            None,
            Some("turn-2".to_string()),
        );
        notifier.push(
            json!({ "type": "TurnCompleted", "sessionId": "s1", "turnId": "turn-1", "summary": "The suite is green." }),
            None,
            Some("turn-1".to_string()),
        );

        let ended = receive.recv_timeout(Duration::from_secs(30)).expect("the watcher never answered");

        assert_eq!(
            ended,
            Ended { summary: "The suite is green.".to_string(), error: None },
            "the watcher answered with the wrong turn"
        );
        assert!(ended.finished(), "a turn that reached its end is finished");
    }

    /// A turn that fails ends with `ErrorRaised`, not `TurnCompleted`. Reading only the latter would leave the
    /// card saying "running" and the Inbox empty, which is the failure a person is most likely to meet.
    #[test]
    fn a_turn_that_failed_reaches_the_caller_as_a_failure() {
        let daemon = state();
        let (send, receive) = mpsc::channel();
        let notifier = ChannelNotifier::new(daemon.events.clone(), daemon.fanout.clone());

        watch(daemon.clone(), "turn-9".to_string(), move |ended| {
            let _ = send.send(ended);
        });

        notifier.push(
            json!({
                "type": "ErrorRaised",
                "turnId": "turn-9",
                "title": "claude_code failed",
                "explanation": "You've hit your session limit",
            }),
            None,
            Some("turn-9".to_string()),
        );

        let ended = receive.recv_timeout(Duration::from_secs(30)).expect("the watcher never answered");

        assert!(!ended.finished(), "a failed turn is not finished work");
        assert_eq!(ended.error.as_deref(), Some("claude_code failed"));
        assert_eq!(ended.summary, "You've hit your session limit");
    }

    /// The clock runs over every project the daemon knows. A project with no automations file is the common
    /// case, and it must cost nothing and start nothing.
    #[test]
    fn the_clock_starts_nothing_for_a_project_with_no_automations() {
        let daemon = state();
        let folder = std::env::temp_dir().join(format!("sdcd-run-{}", std::process::id()));

        std::fs::create_dir_all(&folder).expect("a project folder");
        daemon.store.ensure_host("local", "Local", "local", "connected").expect("the local host");
        daemon.store.add_project("p1", "local", &folder.to_string_lossy(), "somewhere").expect("a project row");

        crate::v3::forcing(&["automations"], || {
            assert_eq!(run_due(&daemon), 0, "a project with no automations started a run");
        });

        std::fs::remove_dir_all(&folder).ok();
    }

    /// What the clock is for: an automation that is due is picked up without anyone pressing Run. This one's
    /// engine is not connected, so the run is refused - and still counted as tried, because the alternative is
    /// a daemon asking the same broken question every minute for as long as the reason lasts.
    /// Phase 0.5(b): the clock reaches `knowledge_core::nightly` for a registered project that has candidates.
    #[test]
    fn the_clock_runs_the_nightly_knowledge_pass_for_a_registered_project() {
        let daemon = state();
        let folder = std::env::temp_dir().join(format!("sdcd-nightly-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join(".sdc/knowledge")).expect("a project folder");
        std::fs::write(folder.join(crate::v3::knowledge_core::CANDIDATES), "{\"kind\":\"lesson\",\"text\":\"once seen\"}\n").expect("a candidate");
        daemon.store.ensure_host("local", "Local", "local", "connected").expect("the local host");
        daemon.store.add_project("p-night", "local", &folder.to_string_lossy(), "somewhere").expect("a project row");

        assert_eq!(nightly_pass(&daemon), 1);
        assert_eq!(nightly_pass(&daemon), 0, "not again within the day");
        assert!(folder.join(crate::v3::knowledge_core::NIGHTLY_MARK).exists());

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_morning_brief_is_pushed_once_a_day_after_seven_and_only_when_turned_on() {
        use chrono::TimeZone;

        let daemon = state();
        let at = |hour| chrono::Local.with_ymd_and_hms(2026, 10, 18, hour, 0, 0).unwrap();

        assert!(!daily_brief(&daemon, at(8)), "off by default");

        daemon.store.set_setting("brief.daily", "on").unwrap();

        assert!(!daily_brief(&daemon, at(6)), "not before 07:00");
        assert!(daily_brief(&daemon, at(8)), "from 07:00");
        assert!(!daily_brief(&daemon, at(9)), "once a day");
    }

    #[test]
    fn the_clock_takes_up_an_automation_that_is_due() {
        let daemon = state();
        let folder = std::env::temp_dir().join(format!("sdcd-due-{}", std::process::id()));

        std::fs::create_dir_all(folder.join(".sdc")).expect("a project folder");
        std::fs::write(
            folder.join(".sdc/automations.toml"),
            "[[automation]]\nname = \"morning\"\nrequest = \"Check the suite\"\nevery = \"6h\"\n",
        )
        .expect("an automations file");

        daemon.store.ensure_host("local", "Local", "local", "connected").expect("the local host");
        daemon.store.add_project("p1", "local", &folder.to_string_lossy(), "somewhere").expect("a project row");
        daemon.store.set_setting("automation.engine", "no-such-engine").expect("a setting");

        crate::v3::forcing(&["automations"], || {
            assert_eq!(run_due(&daemon), 0, "an engine that is not connected cannot start a run");
        });

        assert!(
            crate::v3::automations::last_runs(&folder).contains_key("morning"),
            "the clock did not mark the automation it took up, so it will ask again every minute"
        );
        assert!(
            !crate::v3::automations::due(
                &crate::v3::automations::load(&folder).expect("the file it just read"),
                &crate::v3::automations::last_runs(&folder),
                chrono::Utc::now().timestamp()
            )
            .iter()
            .any(|automation| automation.name == "morning"),
            "the automation is still due right after the clock tried it"
        );

        std::fs::remove_dir_all(&folder).ok();
    }
}
