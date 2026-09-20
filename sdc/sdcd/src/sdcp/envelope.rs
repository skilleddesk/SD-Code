//! The envelope, the response and the error object (master spec section 5.1-5.3).
//!
//! `Envelope` is what the app sends, and its `id` is echoed by the matching `Response` - that is the
//! whole correlation mechanism, deliberately, because a request table keyed by anything else would
//! be a second source of truth. `params` stays a `serde_json::Map` here and is validated per method
//! in `methods.rs`, which is where the method's contract lives.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::SDCP_VERSION;

/// A request. `method` is the dotted name (`engine.start`), `params` its object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub v: String,
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(rename = "hostId", skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
}

impl Envelope {
    /// Parses a request from one line of JSON. `v` is defaulted rather than required so a client
    /// that forgets it is answered with `bad_request` instead of being dropped at the parser.
    pub fn parse(line: &str) -> Result<Self, ErrorObject> {
        let mut envelope: Envelope = serde_json::from_str(line)
            .map_err(|error| ErrorObject::bad_request(format!("malformed envelope: {error}")))?;

        if envelope.v.is_empty() {
            envelope.v = SDCP_VERSION.to_string();
        }

        Ok(envelope)
    }

    /// A string parameter, or the method's `bad_request` when it is missing.
    pub fn require_str(&self, key: &str) -> Result<String, ErrorObject> {
        match self.params.get(key) {
            Some(Value::String(value)) => Ok(value.clone()),
            Some(_) => Err(ErrorObject::bad_request(format!("`{key}` must be a string"))),
            None => Err(ErrorObject::bad_request(format!("`{key}` is required"))),
        }
    }

    /// An optional string parameter.
    pub fn opt_str(&self, key: &str) -> Option<String> {
        match self.params.get(key) {
            Some(Value::String(value)) => Some(value.clone()),
            _ => None,
        }
    }

    /// An optional boolean, defaulting to `false`.
    pub fn opt_bool(&self, key: &str) -> bool {
        matches!(self.params.get(key), Some(Value::Bool(true)))
    }

    /// An optional integer.
    pub fn opt_i64(&self, key: &str) -> Option<i64> {
        self.params.get(key).and_then(Value::as_i64)
    }
}

/// Machine-readable failure codes (schema `$defs.error.code`). The app never parses `message`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorObject {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

impl ErrorObject {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into(), data: None, retryable: None }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new("bad_request", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    pub fn unsupported(method: &str) -> Self {
        Self::new("unsupported", format!("{method} is not implemented by this daemon"))
    }

    /// A path the file guard refused (spec section 5.4's blocked patterns).
    pub fn blocked(path: &str) -> Self {
        Self::new("blocked_path", format!("{path} is on the blocked list and is never read or written"))
    }

    /// An action the user or the daemon's own deny list refused. Distinct from `blocked_path` because
    /// a refused *command* is a permission answer, not a file-guard one.
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new("permission_denied", message)
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::new("internal", error.to_string())
    }
}

/// Exactly one of `result` / `error` is present - the schema enforces it and this type makes it
/// impossible to express anything else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub v: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

impl Response {
    pub fn ok(id: impl Into<String>, result: Value) -> Self {
        Self { v: SDCP_VERSION.to_string(), id: id.into(), result: Some(result), error: None }
    }

    pub fn fail(id: impl Into<String>, error: ErrorObject) -> Self {
        Self { v: SDCP_VERSION.to_string(), id: id.into(), result: None, error: Some(error) }
    }
}

/// One-way push. `seq` is monotonic per daemon *run*, and it survives a restart because the log is
/// hydrated from SQLite - which is what lets a reconnecting client ask for `since(n)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub v: String,
    pub seq: i64,
    pub ts: String,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(rename = "turnId")]
    pub turn_id: Option<String>,
    pub event: Value,
}
