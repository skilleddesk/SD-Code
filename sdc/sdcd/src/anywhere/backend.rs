//! The real [`Backend`](super::core::Backend): the Anywhere core talking to this daemon.
//!
//! Everything goes through the existing paths. A decision resolves the permission gate the same way
//! `permission.resolve` does; a tunnelled call runs through `Daemon::handle`, so the team-role check and
//! every handler's own guard apply; an event that should be remembered is appended to the event log,
//! which feeds the audit ledger.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::sdcp::envelope::{Envelope, ErrorObject};
use crate::sdcp::events::event;
use crate::sdcp::notifications::{ChannelNotifier, Notifier};
use crate::DaemonState;

use super::core::{Backend, DecisionBy, StreamEvent};

pub struct DaemonBackend {
    state: Arc<DaemonState>,
}

impl DaemonBackend {
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    fn notifier(&self) -> Arc<dyn Notifier> {
        Arc::new(ChannelNotifier::new(self.state.events.clone(), self.state.fanout.clone()))
    }
}

/// Runs one SDCP method as the daemon's own handler would. Blocking: call it on the blocking pool.
pub fn call(state: &Arc<DaemonState>, method: &str, params: Value) -> Result<Value, ErrorObject> {
    let params = params.as_object().cloned().unwrap_or_default();
    let host_id = params.get("hostId").and_then(Value::as_str).map(str::to_string);
    let envelope = Envelope { v: crate::SDCP_VERSION.to_string(), id: "anywhere".into(), method: method.to_string(), params, host_id };
    let notifier: Arc<dyn Notifier> = Arc::new(ChannelNotifier::new(state.events.clone(), state.fanout.clone()));
    let response = state.handler().handle(&envelope, notifier);

    match (response.result, response.error) {
        (Some(result), _) => Ok(result),
        (None, Some(error)) => Err(error),
        (None, None) => Ok(Value::Null),
    }
}

impl Backend for DaemonBackend {
    fn resolve_permission(&self, permission_id: &str, decision: &str, by: &DecisionBy) -> bool {
        let delivered = crate::agent::gate::resolve(permission_id, decision);
        let mut resolved = event::permission_resolved(permission_id, &by.label);

        /* Who decided, so the ledger can say "allowed from Pixel", not just "allowed". */
        if let Some(map) = resolved.as_object_mut() {
            map.insert("via".into(), json!("anywhere"));
            map.insert("deviceId".into(), json!(by.device_id));
            map.insert("deviceName".into(), json!(by.device_name));

            /* The person's own words and choices, so the ledger can say what was decided and not only that something was. */
            if let Some(extra) = by.extra.as_object() {
                for (key, value) in extra {
                    map.insert(key.clone(), value.clone());
                }
            }
        }

        self.notifier().push(resolved, None, None);

        delivered
    }

    fn events_since(&self, seq: i64, limit: usize) -> Vec<StreamEvent> {
        self.state
            .events
            .since(seq)
            .into_iter()
            .take(limit)
            .map(|stored| StreamEvent { seq: stored.seq, event: stored.event, session_id: stored.session_id, turn_id: stored.turn_id })
            .collect()
    }

    fn current_seq(&self) -> i64 {
        self.state.events.seq()
    }

    fn kill_all(&self) -> Value {
        call(&self.state, "kill.all", json!({})).unwrap_or_else(|error| json!({ "error": error.message }))
    }

    fn note(&self, what: &str, detail: Value) {
        self.notifier().push(event::remote_activity(what, detail), None, None);
    }
}
