//! The push side of SDCP (master spec section 5.5).
//!
//! A request handler must be able to *push* while it works - `engine.start` returns a `turnId`
//! immediately and then streams `TurnDelta`s for as long as the engine talks. That is what the
//! `Notifier` trait is: a handler is given a sink, and whatever it pushes is wrapped in a
//! `Notification` (sequence number, timestamp, routing ids) and **broadcast** to every connected
//! client.
//!
//! ## Why a broadcast and not a per-request reply channel
//!
//! The app opens two sockets: one for requests (`sdcp_call`) and one that only listens, which is what
//! `sdcp_subscribe` feeds into the window's `sdcp://event`. A per-connection sink would deliver an
//! event only to whichever connection happened to make the request, so the listening socket would
//! receive nothing at all - a turn's stream would reach no one. `Fanout` is that fix: one registry of
//! subscribers, and every event goes to all of them, the requester included. It also makes a second
//! window (or a host on the far side of a tunnel) see the same stream, which is what "the event log
//! is the state" (spec section 3.3) means once there is more than one reader.
//!
//! Three implementations exist, and the last two are why a daemon can be tested without a socket:
//!
//! * `ChannelNotifier` - the real one: appends to the log, serializes once, broadcasts.
//! * `Fanout` - the subscriber registry that `ChannelNotifier` writes through.
//! * `RecordingNotifier` - a `Vec` behind a mutex, used by the unit tests to assert the exact event
//!   sequence of a turn.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::sdcp::envelope::Notification;
use crate::sdcp::events::EventLog;

/// A sink a handler may push events into. `session_id` / `turn_id` are the envelope's routing
/// fields; passing them makes a session-scoped `event.list` possible.
///
/// `push` answers with the sequence number the event was given, which is what `event.append` reports
/// back to a client that recorded an event of its own (spec section 3.3). A notifier that does not
/// keep a log answers `None`.
pub trait Notifier: Send + Sync {
    fn push(&self, event: Value, session_id: Option<String>, turn_id: Option<String>) -> Option<i64>;

    /// An event for the windows watching now, **not for the record** (0.14.2): a tool call still being
    /// written (`ToolCallDrafting`) is reported a few times a second, and the finished card replaces it.
    /// Logged, one large file would add a hundred rows that every start-up replays and the daemon keeps in
    /// memory. It carries the log's current `seq` (no new number, so no gap), and the app folds it without
    /// advancing its own. A notifier without a live channel records it like any other event.
    fn push_live(&self, event: Value, session_id: Option<String>, turn_id: Option<String>) {
        let _ = self.push(event, session_id, turn_id);
    }
}

/// How many lines a connection may have waiting before it is let go (0.16.1). The queue used to grow
/// without a bound behind a reader that stopped reading, and the daemon's memory with it. A dropped
/// connection is not a lost event: the client reconnects and replays what it missed (`event.list`).
pub const MAX_BACKLOG: usize = 50_000;

struct Subscriber {
    sender: UnboundedSender<String>,
    /// Lines sent and not yet written to the socket. The connection's writer counts them down.
    backlog: Arc<AtomicUsize>,
}

/// The subscriber registry: every connection that wants notifications, by id.
#[derive(Default)]
pub struct Fanout {
    next: AtomicU64,
    subscribers: Mutex<HashMap<u64, Subscriber>>,
}

impl Fanout {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Registers a connection's queue and returns its id, which `unsubscribe` needs.
    pub fn subscribe(&self, sender: UnboundedSender<String>) -> u64 {
        self.subscribe_counted(sender).0
    }

    /// `subscribe`, and the counter of lines waiting for this connection - the writer decrements it as
    /// it writes each one, so the registry can tell a reader that has stopped.
    pub fn subscribe_counted(&self, sender: UnboundedSender<String>) -> (u64, Arc<AtomicUsize>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let backlog = Arc::new(AtomicUsize::new(0));

        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.insert(id, Subscriber { sender, backlog: backlog.clone() });
        }

        (id, backlog)
    }

    /// Forgets a connection. Called when its loop ends, so a closed socket does not accumulate.
    pub fn unsubscribe(&self, id: u64) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.remove(&id);
        }
    }

    /// Sends one already-serialized line to every subscriber. A send that fails means that client is
    /// gone: it is dropped from the registry, and the event stays in the log for its `event.list`
    /// replay (spec section 5.4).
    pub fn broadcast(&self, line: &str) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };

        subscribers.retain(|id, subscriber| {
            if subscriber.backlog.load(Ordering::SeqCst) >= MAX_BACKLOG {
                eprintln!("sdcd: connection {id} stopped reading ({MAX_BACKLOG} lines waiting); it is let go and will catch up when it reconnects");

                return false;
            }

            let sent = subscriber.sender.send(line.to_string()).is_ok();

            if sent {
                subscriber.backlog.fetch_add(1, Ordering::SeqCst);
            }

            sent
        });
    }

    /// How many connections are listening; `host.status` reports it.
    pub fn listeners(&self) -> usize {
        self.subscribers.lock().map(|subscribers| subscribers.len()).unwrap_or(0)
    }
}

/// The real notifier: appends to the log and broadcasts to every subscriber.
pub struct ChannelNotifier {
    log: Arc<EventLog>,
    fanout: Arc<Fanout>,
}

impl ChannelNotifier {
    pub fn new(log: Arc<EventLog>, fanout: Arc<Fanout>) -> Self {
        Self { log, fanout }
    }
}

impl Notifier for ChannelNotifier {
    fn push(&self, event: Value, session_id: Option<String>, turn_id: Option<String>) -> Option<i64> {
        let stored = self.log.append(event, session_id, turn_id);
        let notification = Notification {
            v: crate::SDCP_VERSION.to_string(),
            seq: stored.seq,
            ts: stored.ts,
            session_id: stored.session_id,
            turn_id: stored.turn_id,
            event: stored.event,
        };

        if let Ok(line) = serde_json::to_string(&notification) {
            self.fanout.broadcast(&line);
        }

        Some(stored.seq)
    }

    fn push_live(&self, event: Value, session_id: Option<String>, turn_id: Option<String>) {
        let notification = Notification {
            v: crate::SDCP_VERSION.to_string(),
            seq: self.log.seq(),
            ts: chrono::Utc::now().to_rfc3339(),
            session_id,
            turn_id,
            event,
        };

        if let Ok(line) = serde_json::to_string(&notification) {
            self.fanout.broadcast(&line);
        }
    }
}

/// The test notifier: records everything so a test can assert the sequence.
#[derive(Default)]
pub struct RecordingNotifier {
    log: Option<Arc<EventLog>>,
    events: Mutex<Vec<Value>>,
}

impl RecordingNotifier {
    pub fn new() -> Self {
        Self { log: None, events: Mutex::new(Vec::new()) }
    }

    /// A recording notifier that also appends to a real log, for a test that wants both.
    pub fn with_log(log: Arc<EventLog>) -> Self {
        Self { log: Some(log), events: Mutex::new(Vec::new()) }
    }

    /// Everything pushed, in order.
    pub fn events(&self) -> Vec<Value> {
        self.events.lock().map(|events| events.clone()).unwrap_or_default()
    }

    /// The `type` of every pushed event, which is what a fixture comparison asserts.
    pub fn kinds(&self) -> Vec<String> {
        self.events()
            .iter()
            .filter_map(|event| event.get("type").and_then(Value::as_str).map(str::to_string))
            .collect()
    }

    /// The concatenated `delta` of every `TurnDelta` - the answer, as the app would build it.
    pub fn streamed_text(&self) -> String {
        self.events()
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some("TurnDelta"))
            .filter_map(|event| event.get("delta").and_then(Value::as_str))
            .collect()
    }
}

impl Notifier for RecordingNotifier {
    fn push(&self, event: Value, session_id: Option<String>, turn_id: Option<String>) -> Option<i64> {
        let seq = self
            .log
            .as_ref()
            .map(|log| log.append(event.clone(), session_id, turn_id).seq);

        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }

        seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    /// 0.14.2: a draft reaches the window and not the record - the log neither grows nor renumbers.
    #[test]
    fn a_live_only_event_is_broadcast_with_the_current_seq_and_not_logged() {
        let log = Arc::new(EventLog::empty());
        let fanout = Fanout::new();
        let (window, mut window_rx) = unbounded_channel();

        fanout.subscribe(window);

        let notifier = ChannelNotifier::new(log.clone(), fanout);

        notifier.push(serde_json::json!({ "type": "TurnStarted" }), None, Some("t1".into()));
        notifier.push_live(serde_json::json!({ "type": "ToolCallDrafting" }), None, Some("t1".into()));

        assert_eq!(log.len(), 1, "the draft is not in the log");
        assert_eq!(log.seq(), 1, "and it took no number");

        let _started = window_rx.try_recv().expect("the logged event");
        let draft: Value = serde_json::from_str(&window_rx.try_recv().expect("the draft reached the window")).unwrap();

        assert_eq!(draft["seq"], 1);
        assert_eq!(draft["event"]["type"], "ToolCallDrafting");
    }

    /// The regression test for the bug this module's design note describes: an event caused by one
    /// connection has to reach *every* subscriber, not only the one that asked.
    #[test]
    fn a_broadcast_reaches_every_subscriber_and_not_only_the_requester() {
        let fanout = Fanout::new();
        let (requester, mut requester_rx) = unbounded_channel();
        let (listener, mut listener_rx) = unbounded_channel();
        let (departing, mut departing_rx) = unbounded_channel();

        fanout.subscribe(requester);
        fanout.subscribe(listener);
        let leaving = fanout.subscribe(departing);

        assert_eq!(fanout.listeners(), 3);

        /* The requester's own notification: the message the smoke test caught going missing. */
        fanout.broadcast(r#"{"seq":1,"event":{"type":"TurnStarted"}}"#);

        assert!(requester_rx.try_recv().is_ok(), "the requester receives its own event");
        assert!(listener_rx.try_recv().is_ok(), "and so does a socket that only listens");
        assert!(departing_rx.try_recv().is_ok());

        /* A client that went away is dropped from the registry instead of accumulating. */
        drop(departing_rx);
        fanout.broadcast(r#"{"seq":2,"event":{"type":"TurnDelta"}}"#);

        assert!(requester_rx.try_recv().is_ok());
        assert!(listener_rx.try_recv().is_ok());
        assert_eq!(fanout.listeners(), 2);

        fanout.unsubscribe(leaving);
        assert_eq!(fanout.listeners(), 2, "an id that was already dropped is not counted twice");

        fanout.unsubscribe(1);
        assert_eq!(fanout.listeners(), 1);
    }

    /// The backlog limiter (0.16.1): a connection whose writer has stopped draining it is dropped once
    /// its queue reaches [`MAX_BACKLOG`], the same counter its writer decrements as it sends each line -
    /// set here directly rather than by broadcasting 50,000 lines. A connection that is keeping up is
    /// untouched.
    #[test]
    fn a_connection_past_max_backlog_is_dropped_and_others_keep_receiving() {
        let fanout = Fanout::new();
        let (slow, mut slow_rx) = unbounded_channel();
        let (fast, mut fast_rx) = unbounded_channel();

        let (_slow_id, backlog) = fanout.subscribe_counted(slow);
        fanout.subscribe(fast);

        backlog.store(MAX_BACKLOG, Ordering::SeqCst);
        assert_eq!(fanout.listeners(), 2);

        fanout.broadcast(r#"{"seq":1,"event":{"type":"TurnStarted"}}"#);

        assert_eq!(fanout.listeners(), 1, "the stalled connection is dropped, not just skipped");
        assert!(slow_rx.try_recv().is_err(), "nothing more is queued for a connection that is already too far behind");
        assert!(fast_rx.try_recv().is_ok(), "a connection that is keeping up still receives");
    }
}
