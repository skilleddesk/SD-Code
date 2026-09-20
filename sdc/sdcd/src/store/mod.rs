//! The store: SQLite, one file, one writer (master spec section 6).
//!
//! The database is the daemon's memory for everything that outlives a run - hosts, projects,
//! sessions, turns, checkpoints, the rewind stack, permission decisions, providers and the raw event
//! log. The app's own state is *derived* from the event log, but the log is persisted here, which is
//! what makes a restart continue a conversation rather than start a new one.

pub mod sqlite;

pub use sqlite::Store;
