//! Noticing that a connection died (0.11.5).
//!
//! The report: *"suddenly SSH clash hoye jasse. reconnect ar jonno clash hole request korse nah."* The
//! host in it signs in with a password and a verification code, and SDC holds that sign-in open as a
//! master connection (`ssh::session`). When the master goes - the daemon was replaced by an update, the
//! laptop slept, the network dropped for longer than `ServerAliveInterval × ServerAliveCountMax` - nothing
//! said so: the row stayed `connected` for four hours (measured in the log, seq 22483 → 35235) and the
//! first sign of trouble was a file tree that would not load.
//!
//! This is the missing measurement. Every [`INTERVAL`], each VPS row that says `connected` is probed
//! again, and a verdict that **changed** is written to the row and pushed as a `HostStatus` - which is
//! what the window's sign-in card listens for. A verdict that did not change pushes nothing, so a quiet
//! host adds nothing to the log.
//!
//! One failed probe is not trusted on its own: a hiccup on a busy network would otherwise flip a working
//! host to `offline` and ask a person for a code they do not need to type. The probe is repeated after
//! [`CONFIRM`], and only a second failure is reported.

use std::sync::Arc;
use std::time::Duration;

use crate::auth::remote::SshTarget;
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;
use crate::store::sqlite::Store;

use super::Ssh;

/// How often a connected host is looked at. A probe through an open master costs about 30 ms.
pub const INTERVAL: Duration = Duration::from_secs(45);

/// How long to wait before believing a failed probe.
const CONFIRM: Duration = Duration::from_secs(3);

/// The rows worth looking at: VPS hosts whose stored status is `connected`, with an address to dial.
fn connected_hosts(store: &Store) -> Vec<(String, String, Option<String>, Ssh)> {
    let Ok(hosts) = store.hosts() else {
        return Vec::new();
    };

    hosts
        .into_iter()
        .filter(|host| host["hostType"].as_str() == Some("vps") && host["status"].as_str() == Some("connected"))
        .filter_map(|host| {
            let id = host["hostId"].as_str()?.to_string();
            let name = host["name"].as_str().unwrap_or(&id).to_string();
            let platform = host["platform"].as_str().map(str::to_string);
            let (Some(target), port) = store.host_address(&id).ok()?? else {
                return None;
            };

            Some((id, name, platform, Ssh::new(SshTarget { user_host: target, port })))
        })
        .collect()
}

/// One pass over the connected hosts. Blocking (it runs `ssh`), so it is called from `spawn_blocking`.
///
/// Answers with the ids whose status changed, which is what the tests assert.
pub fn tick(store: &Store, notifier: &dyn Notifier) -> Vec<String> {
    let mut changed = Vec::new();

    for (id, name, platform, ssh) in connected_hosts(store) {
        let (mut status, mut detail) = super::ops::probe(&ssh);

        if status != "connected" {
            std::thread::sleep(CONFIRM);
            (status, detail) = super::ops::probe(&ssh);
        }

        if status == "connected" {
            continue;
        }

        let target = ssh.target.user_host.clone();
        let _ = store.upsert_host(&id, &name, "ssh", Some(&target), &status, platform.as_deref());

        notifier.push(
            event::host_status(&id, &name, "vps", &status, platform.as_deref(), Some(&detail), None),
            None,
            None,
        );

        changed.push(id);
    }

    changed
}

/// Starts the watcher. The first pass waits one interval: at start the window's own requests come
/// first, and a row the previous daemon left `connected` is measured shortly after.
pub fn spawn(store: Arc<Store>, notifier: Arc<dyn Notifier>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(INTERVAL).await;

            let store = store.clone();
            let notifier = notifier.clone();

            let _ = tokio::task::spawn_blocking(move || tick(&store, &*notifier)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdcp::notifications::RecordingNotifier;

    /// A row that says `connected` but cannot be reached turns `offline`, once, with a sentence - and a
    /// row that already says `offline` is left alone, so a quiet host adds nothing to the log.
    ///
    /// `127.0.0.1:1` refuses at once on every runner, so no network and no sign-in is needed.
    #[test]
    fn a_connected_host_that_went_away_is_reported_once() {
        let store = Store::in_memory().unwrap();
        let notifier = RecordingNotifier::new();

        store.upsert_host("h1", "gone", "ssh", Some("sdc@127.0.0.1"), "connected", Some("Debian 12 · x64")).unwrap();
        store.set_host_address("h1", "sdc@127.0.0.1", Some(1)).unwrap();
        store.upsert_host("h2", "down", "ssh", Some("sdc@127.0.0.1"), "offline", None).unwrap();
        store.set_host_address("h2", "sdc@127.0.0.1", Some(1)).unwrap();

        assert_eq!(tick(&store, &notifier), vec!["h1".to_string()]);

        let events = notifier.events();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "HostStatus");
        assert_eq!(events[0]["hostId"], "h1");
        assert_eq!(events[0]["status"], "offline");
        assert_eq!(events[0]["platform"], "Debian 12 · x64");
        assert!(events[0]["detail"].as_str().is_some_and(|detail| !detail.is_empty()));
        assert_eq!(store.host("h1").unwrap().unwrap()["status"], "offline");

        /* Measured again: nothing is `connected` any more, so nothing is probed or pushed. */
        assert!(tick(&store, &notifier).is_empty());
        assert_eq!(notifier.events().len(), 1);
    }
}
