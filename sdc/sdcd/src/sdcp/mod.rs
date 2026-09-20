//! The SDCP wire format (master spec section 5).
//!
//! Three shapes, and only three: a request `Envelope`, a `Response` (exactly one of `result` /
//! `error`), and a one-way `Notification` carrying exactly one event from the append-only catalogue.
//! Both transports - the local socket/pipe and the remote tunnel - carry these same bytes, which is
//! the property spec section 3.1 exists to guarantee.

pub mod envelope;
pub mod events;
pub mod methods;
pub mod notifications;

pub use envelope::{Envelope, ErrorObject, Notification, Response};
pub use events::{EventLog, StoredEvent};
pub use notifications::Notifier;
