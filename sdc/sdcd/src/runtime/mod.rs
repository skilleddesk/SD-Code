//! The step runtime: one agent step, and the state it carries between steps.
//!
//! P0-3 opens the seam and moves nothing in yet. Today a turn is run by `crate::agent` (the loop, the engine
//! adapters and the dialects), and the parts that belong here are still there:
//!
//! - the step trace, one row per step, and the speed baseline that reads it (P0-12)
//! - the Step Transaction (P0-14) - **here now**, `runtime::transaction`; the Sandbox Runner is `guard::sandbox`
//! - the Step Runtime itself: budget, effort and profile per step (P1-2, P1-4)
//! - the fast path that skips a model call when the answer is already known (P1-15)
//!
//! The module exists now because P0-4 onwards add step machinery, and a second reshuffle of `agent/mod.rs` after
//! those changes land would be a behaviour change waiting to happen. Nothing here is a facade over anything: it is
//! empty on purpose, and it stays empty until the task that owns each piece moves the code in.

pub mod decompose;
pub mod step;
pub mod transaction;
