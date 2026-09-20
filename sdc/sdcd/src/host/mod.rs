//! The host module: what the daemon knows about the machine it runs on.

pub mod doctor;

pub use doctor::checks;
