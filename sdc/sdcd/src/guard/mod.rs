//! Guards: the limits a step is not allowed to cross, and the switch that turns each one off.
//!
//! P0-3 opened the seam and moved nothing in yet. The pieces this module will own, and where they live today:
//!
//! - the daemon's door: per-client tokens, `Host` and `Origin` checks, rate limiting (P0-6) - **here now**
//! - the Local Runtime Guard: what a local runtime is really asked to carry (P0-8) - **here now**, `guard::context`
//! - the Pin Guard: a pinned model stays pinned (P0-10) - `crate::v3::modelgate`, `crate::v3::effort`
//! - the step, budget and cost guards (P1-3, P1-5) - `crate::trust::policy`, `crate::trust::cost`
//! - the Output Compressor and the OS-normaliser for commands (P0-13)
//! - the sandbox policy a command runs under (P0-14)
//!
//! The permission gate the person already sees is `crate::agent::gate`; it moves in here when P0-13 and P0-14 give
//! it neighbours, so that everything allowed to say "no" to a step can be read in one place.

pub mod compress;
pub mod context;
pub mod daemon;
pub mod osnorm;
pub mod runner;
