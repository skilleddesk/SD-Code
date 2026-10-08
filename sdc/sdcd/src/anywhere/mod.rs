//! **SDC Anywhere** (0.17+, docs/remote/SDC-ANYWHERE-PLAN-v2.md and docs/remote/DESIGN.md).
//!
//! A browser controls this daemon through a relay that only forwards ciphertext. The daemon dials
//! out; nothing here opens a port. The module is named `anywhere`, not `remote`, because `remote`
//! already means "a VPS reached over SSH" in this codebase (`auth::remote`, docs/REMOTE.md).
//!
//! | part               | file               | what it owns |
//! | ------------------ | ------------------ | ------------ |
//! | canonical JSON     | `canonical`        | the bytes a signature covers |
//! | action envelope    | `envelope`         | what the card shows and the passkey signs |
//! | end-to-end session | `crypto`           | HPKE halves, device and daemon signatures |
//!
//! `remote.enabled` is false until a person turns it on in the desktop app; with it off, none of this
//! code runs and SDC behaves exactly as before (plan principles 8 and 9).

pub mod canonical;
pub mod backend;
pub mod core;
#[cfg(test)]
mod core_tests;
pub mod crypto;
pub mod envelope;
pub mod frame;
pub mod gateway;
pub mod identity;
pub mod os;
pub mod registry;
pub mod relay;
pub mod router;
pub mod runtime;
pub mod session;
#[cfg(test)]
pub mod testkit;
pub mod webauthn;
