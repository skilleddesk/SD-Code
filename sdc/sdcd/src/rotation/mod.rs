//! Rotation: which model runs this step, why it was chosen, and what happens when it fails.
//!
//! P0-3 opens the seam and moves nothing in yet. The pieces this module will own, and where they live today:
//!
//! - the model identity and capability probe (P0-9) - the model list and the wire formats are `crate::providers`
//!   and `crate::engines`
//! - the Rotation Policy Engine: the chain and the paid-continuation rule (P2-1, P2-4), which needs `rotation.toml`
//!   from P0-5
//! - the fleet scheduler that spreads a wide search over models (P2-6)
//!
//! `crate::v3::router`, `cascade` and `failover` already hold a first cut of this; they move in here when P2 gives
//! them a policy to follow, because a model choice is exactly the thing the person has to be able to audit.

pub mod capsule;
pub mod decision;
pub mod executor;
pub mod ledger;
pub mod policy;
