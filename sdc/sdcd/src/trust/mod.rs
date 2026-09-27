//! **The Trust Kernel** (0.12, docs/MASTER-PLAN-v3-TRUST-KERNEL.md).
//!
//! Time Machine, Verify, guard rails, the audit log and rollback used to be separate features. The kernel
//! makes them one layer that every AI action passes through, whichever engine runs it:
//!
//! | part          | module     | what it guarantees |
//! | ------------- | ---------- | ------------------ |
//! | Audit ledger  | `ledger`   | every action is recorded, hash-chained, and tampering is detectable |
//! | Policy engine | `policy`   | `.sdc/policy.toml`: protected paths, commands that always ask, production, privacy, blast radius |
//! | Verify engine | `scan` + `crate::verify` | secrets and risky code patterns in a change are found by rule, not by opinion |
//! | Time Machine  | `crate::checkpoints`, `crate::rewind` | a checkpoint before every change; journaled, atomic restore |
//! | Cost governor | `cost`     | every turn's cost measured or labelled as an estimate; budgets; runaway loops stopped |
//! | Kill switch   | `kill`     | one keystroke stops every running turn, check, deploy and command |
//! | Scores        | `score`    | a Trust score per turn and an Ops score per site, each with its reasons |
//!
//! The six steps of the pipeline - understand, build, verify, ship, protect, prove - each go through it:
//! the Intent Engine (`crate::intent`) understands, the engines build, `crate::verify` verifies,
//! `crate::ops` ships and protects, and `proof` proves.

pub mod cost;
pub mod kill;
pub mod ledger;
pub mod policy;
pub mod proof;
pub mod scan;
pub mod score;
