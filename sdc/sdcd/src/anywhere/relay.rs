//! The relay driver: the only async code in SDC Anywhere.
//!
//! It dials **out** to the relay (a Durable Object on `sdc.skilleddesk.com`), proves it is the daemon
//! whose id is in the URL, and then does three jobs in one loop: move relay messages into the
//! [`Core`], move what the core returns back out, and run tunnelled calls on the blocking pool so one
//! slow call can never hold up a Kill. Nothing here decides anything; every rule is in `core`.
//!
//! If the relay is unreachable the loop backs off and retries. Nothing else in SDC notices: the
//! desktop app keeps working exactly as before (plan principle 9).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

use crate::DaemonState;

use super::backend;
use super::gateway;
use super::core::{Core, Out};
use super::crypto::b64u;
use super::os;

/// Messages a connection may have waiting for the relay before it is let go. A browser that cannot
/// keep up is closed and reconnects; it is never allowed to grow the daemon's memory without bound.
const WRITE_QUEUE: usize = 1024;

/// How long the relay may be silent before the link is called dead.
const SILENCE: Duration = Duration::from_secs(75);

const HEARTBEAT: Duration = Duration::from_secs(30);

/// What the Settings page shows about the relay link.
#[derive(Debug, Clone, Default)]
pub struct LinkStatus {
    pub connected: bool,
    pub last_error: Option<String>,
    pub connected_at: Option<i64>,
    pub attempts: u64,
}

/// Everything the driver shares with the runtime handle.
pub struct Shared {
    pub core: Mutex<Core>,
    pub status: Mutex<LinkStatus>,
    pub notify_when_idle: AtomicBool,
    pub idle_minutes: Mutex<u64>,
    /// Where push and email nudges and sign-in links go. Empty means the owner has not asked for email, so a request from the
    /// relay to make a sign-in pairing offer is refused.
    pub email: Mutex<String>,
    /// Woken when a task other than the driver's own loop may have left frames waiting to be sent.
    pub wake: tokio::sync::Notify,
    /// Where a finished call sends what the core returned for it (replies, and the next call in line). The driver's loop
    /// reads it, so no task ever has to call back into `dispatch`.
    pub feedback: mpsc::UnboundedSender<Vec<Out>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Reconnect delays: 1 s, 2 s, 4 s ... capped at a minute, with up to a quarter of jitter so a relay
/// that restarts is not hit by every daemon at the same instant.
pub fn backoff(attempt: u64) -> Duration {
    let base = 1_u64 << attempt.min(6);
    let capped = base.min(60);
    let jitter = u64::from(super::crypto::random::<1>()[0]) * capped * 250 / 255;

    Duration::from_millis(capped * 1000 + jitter)
}

/// Runs until `stop` flips. `out_rx` carries the results of calls made from the desktop (pairing,
/// revoking), which must be sent to the relay by this task.
pub async fn run(state: Arc<DaemonState>, shared: Arc<Shared>, url: String, mut stop: watch::Receiver<bool>, mut desktop_out: mpsc::UnboundedReceiver<Vec<Out>>) {
    let mut attempt = 0_u64;
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    let (subscriber, backlog) = state.fanout.subscribe_counted(line_tx);

    while !*stop.borrow() {
        if let Ok(mut status) = shared.status.lock() {
            status.attempts += 1;
        }

        let result = session(&state, &shared, &url, &mut stop, &mut desktop_out, &mut line_rx, &backlog).await;

        if let Ok(mut core) = shared.core.lock() {
            core.reset_connections();
        }

        if let Ok(mut status) = shared.status.lock() {
            status.connected = false;

            if let Err(error) = &result {
                status.last_error = Some(error.clone());
            }
        }

        match result {
            /* A clean stop. */
            Ok(()) => break,
            Err(_) => {
                attempt = if shared.status.lock().map(|status| status.connected_at.is_some_and(|at| now_ms() - at > 60_000)).unwrap_or(false) { 0 } else { attempt + 1 };

                tokio::select! {
                    _ = tokio::time::sleep(backoff(attempt)) => {}
                    _ = stop.changed() => {}
                }
            }
        }
    }

    state.fanout.unsubscribe(subscriber);
}

#[allow(clippy::too_many_arguments)]
async fn session(
    state: &Arc<DaemonState>,
    shared: &Arc<Shared>,
    url: &str,
    stop: &mut watch::Receiver<bool>,
    desktop_out: &mut mpsc::UnboundedReceiver<Vec<Out>>,
    lines: &mut mpsc::UnboundedReceiver<String>,
    backlog: &Arc<std::sync::atomic::AtomicUsize>,
) -> Result<(), String> {
    let daemon_id = shared.core.lock().map_err(|_| "core poisoned")?.daemon_id().to_string();
    let target = format!("{}/d/{}", url.trim_end_matches('/'), daemon_id);
    let (socket, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(&target))
        .await
        .map_err(|_| "the relay did not answer in 15 seconds".to_string())?
        .map_err(|error| format!("could not reach the relay: {error}"))?;
    let (mut sink, mut stream) = socket.split();

    /* The whole sign-in has 30 seconds, however the relay trickles messages: a relay that never finishes is a relay
       this daemon stops waiting for (it retries with backoff). */
    let signin_deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    /* The relay speaks first: a challenge. Answering it proves this daemon holds the key behind its id. */
    let challenge = loop {
        if tokio::time::Instant::now() > signin_deadline {
            return Err("the relay did not finish signing in within 30 seconds".into());
        }

        match tokio::time::timeout(Duration::from_secs(15), stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).map_err(|_| "the relay sent something that is not JSON")?;

                if value["t"] == "challenge" {
                    break value["nonce"].as_str().unwrap_or_default().to_string();
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => return Err("the relay closed before it sent a challenge".into()),
        }
    };
    let (public, signature) = {
        let core = shared.core.lock().map_err(|_| "core poisoned")?;

        (core.identity_public(), core.sign_hub_auth(&challenge))
    };

    sink.send(Message::Text(json!({ "t": "auth", "pub": b64u(&public), "sig": b64u(&signature) }).to_string().into())).await.map_err(|error| error.to_string())?;

    loop {
        if tokio::time::Instant::now() > signin_deadline {
            return Err("the relay did not finish signing in within 30 seconds".into());
        }

        match tokio::time::timeout(Duration::from_secs(15), stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

                match value["t"].as_str() {
                    Some("ready") => break,
                    Some("denied") => return Err(format!("the relay refused this daemon: {}", value["why"].as_str().unwrap_or("no reason given"))),
                    _ => continue,
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => return Err("the relay closed during authentication".into()),
        }
    }

    if let Ok(mut status) = shared.status.lock() {
        status.connected = true;
        status.connected_at = Some(now_ms());
        status.last_error = None;
    }

    let (write_tx, mut write_rx) = mpsc::channel::<String>(WRITE_QUEUE);
    let writer = tokio::spawn(async move {
        while let Some(text) = write_rx.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }

        let _ = sink.close().await;
    });

    /* Re-teach the relay which devices exist (it may have restarted and forgotten them). */
    let devices = shared.core.lock().map_err(|_| "core poisoned")?.devices_for_sync();

    let _ = write_tx.send(json!({ "ctl": "devices.sync", "devices": devices }).to_string()).await;

    /* ... and the address to nudge, which the relay keeps only so it can send the mail. */
    let address = shared.email.lock().map(|email| email.clone()).unwrap_or_default();

    if !address.is_empty() {
        let _ = write_tx.send(json!({ "ctl": "email.set", "address": address }).to_string()).await;
    }

    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    let mut last_heard = tokio::time::Instant::now();
    let mut hold: Option<os::SleepGuard> = None;
    /* While frames wait to be sealed (a big reply, a transfer) the core is asked for the next round every 2 ms, as long as
       the relay link has room. Between rounds, anything of higher priority that arrived goes first. */
    let mut pump = tokio::time::interval(Duration::from_millis(2));

    pump.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let outcome: Result<(), String> = loop {
        let waiting_frames = shared.core.lock().map(|core| core.has_backlog()).unwrap_or(false);

        tokio::select! {
            _ = shared.wake.notified() => {}
            _ = pump.tick(), if waiting_frames && write_tx.capacity() > WRITE_QUEUE / 4 => {
                let outs = shared.core.lock().map(|mut core| { let outs = core.pump(); emit(&write_tx, &mut core, outs) }).unwrap_or_default();

                dispatch(state, shared, &write_tx, outs).await;
            }
            _ = stop.changed() => {
                if *stop.borrow() {
                    break Ok(());
                }
            }
            message = stream.next() => {
                last_heard = tokio::time::Instant::now();

                match message {
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                        let outs = from_relay(shared, &write_tx, &value);

                        dispatch(state, shared, &write_tx, outs).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break Err("the relay closed the connection".into()),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => break Err(format!("the relay link failed: {error}")),
                }
            }
            line = lines.recv() => {
                match line {
                    Some(line) => {
                        backlog.fetch_sub(1, Ordering::SeqCst);

                        let outs = shared.core.lock().map(|mut core| { let outs = core.on_event(&line); emit(&write_tx, &mut core, outs) }).unwrap_or_default();

                        dispatch(state, shared, &write_tx, outs).await;
                    }
                    None => break Err("the daemon's event feed ended".into()),
                }
            }
            outs = desktop_out.recv() => {
                if let Some(outs) = outs {
                    dispatch(state, shared, &write_tx, outs).await;
                }
            }
            _ = tick.tick() => {
                if last_heard.elapsed() > SILENCE {
                    break Err("the relay has been silent too long".into());
                }

                let outs = shared.core.lock().map(|mut core| { let outs = core.tick(); emit(&write_tx, &mut core, outs) }).unwrap_or_default();
                let busy = shared.core.lock().map(|core| core.open_requests() > 0 || core.connections() > 0).unwrap_or(false) || !crate::trust::kill::active().is_empty();

                dispatch(state, shared, &write_tx, outs).await;

                /* Keep the machine awake while there is something to wait for; let it sleep otherwise. */
                if busy && hold.is_none() {
                    hold = os::SleepGuard::acquire();
                } else if !busy {
                    hold = None;
                }
            }
            _ = heartbeat.tick() => {
                let _ = write_tx.send(json!({ "ctl": "ping" }).to_string()).await;
            }
        }
    };

    drop(write_tx);
    writer.abort();

    outcome
}

/// Turns one relay message into core input.
fn from_relay(shared: &Arc<Shared>, write_tx: &mpsc::Sender<String>, value: &Value) -> Vec<Out> {
    /* A sign-in link was spent: the relay wants a pairing offer for whoever holds it. */
    if value["t"] == "magic" {
        return answer_magic(shared, value["ticket"].as_str().unwrap_or_default());
    }

    let Some(conn) = value["conn"].as_str() else { return Vec::new() };
    let Ok(mut core) = shared.core.lock() else { return Vec::new() };

    if let Some(open) = value.get("open") {
        core.open(conn, open["pairing"].as_bool().unwrap_or(false));

        return Vec::new();
    }

    if value.get("close").is_some() {
        core.closed(conn);

        return Vec::new();
    }

    match value.get("msg") {
        Some(msg) => {
            let outs = core.on_message(conn, msg);

            emit(write_tx, &mut core, outs)
        }
        None => Vec::new(),
    }
}

/// Only a computer whose owner gave an address for email accepts sign-in links at all, and then no faster than the core allows.
/// The relay is not trusted: the offer is just a pairing token, and the new browser still has to be approved here.
pub fn answer_magic(shared: &Arc<Shared>, ticket: &str) -> Vec<Out> {
    let enabled = shared.email.lock().map(|email| !email.is_empty()).unwrap_or(false);

    if ticket.is_empty() || ticket.len() > 64 || !enabled {
        return vec![Out::Ctl(json!({ "ctl": "offer", "ticket": ticket, "error": "not_enabled" }))];
    }

    match shared.core.lock() {
        Ok(mut core) => core.offer_for_link(ticket),
        Err(_) => vec![Out::Ctl(json!({ "ctl": "offer", "ticket": ticket, "error": "failed" }))],
    }
}

/// Queues the frames in `outs` for the relay **while the core is still locked**, and returns the rest.
///
/// A frame is numbered when it is sealed, and the receiver opens them strictly in number order. If sealing happened under the
/// lock but queuing after it, a call finishing on another task could seal frame 7 and queue it after the main loop had sealed
/// and queued frame 8: the browser would see 8 first and end the session ("frame out of order"). So the two happen together.
fn emit(write_tx: &mpsc::Sender<String>, core: &mut Core, outs: Vec<Out>) -> Vec<Out> {
    let mut rest = Vec::new();

    for out in outs {
        match out {
            Out::Send { conn, msg } => {
                if write_tx.try_send(json!({ "conn": conn, "msg": msg }).to_string()).is_err() {
                    /* The relay link cannot take more: this browser is cut loose rather than buffered without bound. */
                    core.closed(&conn);
                    let _ = write_tx.try_send(json!({ "conn": conn, "close": true }).to_string());
                }
            }
            Out::Close { conn } => {
                let _ = write_tx.try_send(json!({ "conn": conn, "close": true }).to_string());
                core.closed(&conn);
            }
            other => rest.push(other),
        }
    }

    rest
}

/// Carries out what the core asked for.
pub async fn dispatch(state: &Arc<DaemonState>, shared: &Arc<Shared>, write_tx: &mpsc::Sender<String>, outs: Vec<Out>) {
    for out in outs {
        match out {
            Out::Send { conn, msg } => {
                let line = json!({ "conn": conn, "msg": msg }).to_string();

                /* A full queue means this browser cannot keep up. Cut it rather than buffer without end. */
                if write_tx.try_send(line).is_err() {
                    if let Ok(mut core) = shared.core.lock() {
                        core.closed(&conn);
                    }

                    let _ = write_tx.try_send(json!({ "conn": conn, "close": true }).to_string());
                }
            }
            Out::Close { conn } => {
                let _ = write_tx.send(json!({ "conn": conn, "close": true }).to_string()).await;

                if let Ok(mut core) = shared.core.lock() {
                    core.closed(&conn);
                }
            }
            Out::Ctl(ctl) => {
                /* "Tell my phone only when I am away": a push or an email for a card is sent when the
                   person is not at the keyboard. The card is on the desktop either way. */
                if ctl["ctl"] == "notify" && shared.notify_when_idle.load(Ordering::SeqCst) {
                    let minutes = shared.idle_minutes.lock().map(|m| *m).unwrap_or(5);

                    if !os::away_for(minutes) {
                        continue;
                    }
                }

                let _ = write_tx.send(ctl.to_string()).await;
            }
            Out::Rpc { conn, id, method, params, paths, critical } => {
                let state = state.clone();
                let shared = shared.clone();
                let write_tx = write_tx.clone();

                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || match paths {
                        /* The file gateway: ids, roots and protected paths are its business. */
                        Some(table) => gateway::call(&gateway::Ctx { state: &state, table: &table, critical }, &method, &params),
                        None => backend::call(&state, &method, params),
                    })
                        .await
                        .unwrap_or_else(|_| Err(crate::sdcp::envelope::ErrorObject::internal("the call stopped unexpectedly")))
                        .map_err(|error| (error.code, error.message));
                    let outs = shared.core.lock().map(|mut core| { let outs = core.rpc_done(&conn, &id, result); emit(&write_tx, &mut core, outs) }).unwrap_or_default();

                    /* Finishing a call can start the next one in line: the driver's loop carries that out. */
                    let _ = shared.feedback.send(outs);
                    /* Anything left over after this round (a big reply) is for the driver's loop to keep sending. */
                    shared.wake.notify_one();
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anywhere::testkit::Rig;

    fn shared_with(email: &str) -> Arc<Shared> {
        let (feedback, _) = mpsc::unbounded_channel();

        Arc::new(Shared {
            core: Mutex::new(Rig::new().core),
            status: Mutex::new(LinkStatus::default()),
            notify_when_idle: AtomicBool::new(false),
            idle_minutes: Mutex::new(5),
            email: Mutex::new(email.to_string()),
            wake: tokio::sync::Notify::new(),
            feedback,
        })
    }

    fn offer(outs: &[Out]) -> Value {
        outs.iter().find_map(|out| match out { Out::Ctl(ctl) if ctl["ctl"] == "offer" => Some(ctl.clone()), _ => None }).expect("an offer message")
    }

    #[test]
    fn a_computer_without_an_email_address_refuses_sign_in_links() {
        let shared = shared_with("");
        let answer = offer(&answer_magic(&shared, "t1"));

        assert_eq!(answer["error"], "not_enabled");
        assert_eq!(answer["ticket"], "t1");
        assert!(answer.get("fragment").is_none(), "no pairing token is made for a computer that did not ask for this");
    }

    #[test]
    fn a_computer_with_an_address_answers_a_sign_in_link_once_in_a_while() {
        let shared = shared_with("owner@example.com");

        assert!(offer(&answer_magic(&shared, "t1")).get("fragment").is_some());
        assert_eq!(offer(&answer_magic(&shared, "t2"))["error"], "busy");
    }

    #[test]
    fn a_ticket_that_is_empty_or_huge_is_refused() {
        let shared = shared_with("owner@example.com");

        assert_eq!(offer(&answer_magic(&shared, ""))["error"], "not_enabled");
        assert_eq!(offer(&answer_magic(&shared, &"x".repeat(65)))["error"], "not_enabled");
    }

    #[test]
    fn the_relay_message_for_a_link_is_answered_and_other_messages_are_not() {
        let shared = shared_with("owner@example.com");
        let (write_tx, _rx) = mpsc::channel::<String>(8);

        assert!(offer(&from_relay(&shared, &write_tx, &json!({ "t": "magic", "ticket": "abc" }))).get("fragment").is_some());
        assert!(from_relay(&shared, &write_tx, &json!({ "t": "pong" })).is_empty());
    }

    #[test]
    fn backoff_grows_then_stops_at_about_a_minute() {
        let first = backoff(0);
        let later = backoff(10);

        assert!(first >= Duration::from_secs(1) && first < Duration::from_millis(1300));
        assert!(later >= Duration::from_secs(60) && later <= Duration::from_secs(76));
        assert!(backoff(3) >= Duration::from_secs(8));
    }
}
