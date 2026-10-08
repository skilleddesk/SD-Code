//! SDC Anywhere's settings, as SDCP methods (0.17). Only the desktop app calls these.
//!
//! Every method here is in the never-remote list (`anywhere::session::REMOTE_FORBIDDEN_PREFIX`), so a
//! browser session cannot turn SDC Anywhere on or off, pair a device, or change a limit: it can only
//! use what the person set up here.

use std::sync::Arc;

use serde_json::{json, Value};

use super::Daemon;
use crate::sdcp::envelope::{Envelope, ErrorObject};
use crate::sdcp::notifications::Notifier;

impl Daemon {
    pub(super) fn dispatch_anywhere(&self, envelope: &Envelope, _out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let state = &self.state;
        let anywhere = &state.anywhere;

        match envelope.method.as_str() {
            "anywhere.status" => Ok(anywhere.status(state)),
            "anywhere.enable" => {
                anywhere.start(state)?;

                Ok(anywhere.status(state))
            }
            "anywhere.disable" => {
                anywhere.stop(state)?;

                Ok(anywhere.status(state))
            }
            "anywhere.configure" => anywhere.configure(state, &envelope.params),
            "anywhere.pair.begin" => anywhere.begin_pairing(state, envelope.opt_bool("guest")),
            "anywhere.pair.requests" => anywhere.pair_requests(),
            "anywhere.pair.confirm" => anywhere.confirm_pairing(&envelope.require_str("deviceId")?, envelope.params.get("accept").and_then(Value::as_bool).unwrap_or(true)),
            "anywhere.devices.list" => Ok(anywhere.devices(state)),
            "anywhere.devices.revoke" => anywhere.revoke(&envelope.require_str("deviceId")?),
            "anywhere.reset" => {
                anywhere.reset_identity(state)?;

                Ok(json!({ "reset": true }))
            }
            other => Err(ErrorObject::unsupported(other)),
        }
    }
}
