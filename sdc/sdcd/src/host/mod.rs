//! The host module: what the daemon knows about the machine it runs on - and, since 0.7.13, about the
//! machines it reaches (`doctor::remote_checks`).

pub mod doctor;
pub mod program;

pub use doctor::{checks, remote_checks};
