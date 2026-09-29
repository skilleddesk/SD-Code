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

/// How often a connected host is probed over the network.
pub const INTERVAL: Duration = Duration::from_secs(45);

/// How often a signed-in host's master is checked **locally** (0.11.7): `-O check` talks to a socket on
/// this machine, so a master that died is on the card within seconds - before the agent's next command
/// falls back to the key and meets `Permission denied (keyboard-interactive)` four times in a row, which
/// is what the report's screenshot shows.
pub const QUICK: Duration = Duration::from_secs(5);

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
    sweep(store, notifier, true)
}

/// One pass. `full` probes every connected host over the network; otherwise only the hosts whose
/// signed-in master is gone are measured, and the check that finds them is local.
fn sweep(store: &Store, notifier: &dyn Notifier, full: bool) -> Vec<String> {
    let mut changed = Vec::new();

    for (id, name, platform, ssh) in connected_hosts(store) {
        /* A master this daemon did not start - the one a previous daemon left running across an update -
           is looked after the same way once its socket is found. */
        let socket = super::session::has_socket(&ssh);
        let tracked = super::session::was_signed_in(&ssh) || socket;
        let mut master = if tracked { super::session::state(&ssh) } else { super::session::Master::Gone };

        /* "Gone" is asked twice before it is believed (0.15.2), the same rule the network probe has
           always had: one answer on a busy machine cut a live sign-in off at the end of a long turn. */
        if tracked && master == super::session::Master::Gone {
            std::thread::sleep(CONFIRM);
            master = super::session::state(&ssh);
        }

        if master == super::session::Master::Open {
            super::session::adopt(&ssh);
        }

        /* A master that has answered nothing for STUCK_AFTER is not busy but stuck (0.15.4): it is ended,
           so what waits on it fails now with a sentence instead of never. */
        let stuck = tracked && super::session::note(&ssh, master);

        if stuck {
            super::session::kill_stuck(&ssh);

            let detail = format!(
                "the signed-in connection to {} stopped answering for {} s (its ssh process was stuck), so SDC closed it. Sign in again to go on - a running turn there has stopped; send \"continue\" after signing in.",
                ssh.label(),
                super::session::STUCK_AFTER.as_secs()
            );
            let target = ssh.target.user_host.clone();
            let _ = store.upsert_host(&id, &name, "ssh", Some(&target), "offline", platform.as_deref());

            notifier.push(event::host_status(&id, &name, "vps", "offline", platform.as_deref(), Some(&detail), None), None, None);
            changed.push(id);

            continue;
        }

        /* A master that is slow to answer is not lost, and a network probe through it would only be
           slow too. It is looked at again on the next pass. */
        if tracked && master == super::session::Master::Unsure {
            continue;
        }

        let master_lost = tracked && master == super::session::Master::Gone;

        if !full && !master_lost {
            continue;
        }

        let (mut status, mut detail) = super::ops::probe(&ssh);

        /* A master that is gone is not a hiccup: `-O check` asked this machine, not the network. */
        if status != "connected" && !master_lost {
            std::thread::sleep(CONFIRM);
            (status, detail) = super::ops::probe(&ssh);
        }

        if status == "connected" {
            /* Reached without the master (by key): the socket was a leftover, and checking it every
               few seconds would buy nothing. */
            if master_lost {
                super::session::forget(&ssh);
                super::session::discard(&ssh);
            }

            continue;
        }

        super::session::forget(&ssh);

        if master_lost {
            /* Its socket refuses, but the process may still be alive and spinning, holding every call that
               had already reached it - on 2026-09-29 the host turned offline while the turn's `claude`
               kept waiting on that process. It is ended too. */
            super::session::kill_stuck(&ssh);
            detail = format!(
                "the signed-in connection to {} closed (the network dropped, the machine slept, or the host ended it). {detail}",
                ssh.label()
            );
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
        let every = (INTERVAL.as_secs() / QUICK.as_secs()).max(1);
        let mut round: u64 = 0;

        loop {
            tokio::time::sleep(QUICK).await;
            round += 1;

            let store = store.clone();
            let notifier = notifier.clone();
            let full = round % every == 0;

            let _ = tokio::task::spawn_blocking(move || sweep(&store, &*notifier, full)).await;
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
