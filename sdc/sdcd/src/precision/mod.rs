//! Precision Verify: the checks that decide whether a claim is true before the work is called done.
//!
//! P0-3 opens the seam and moves nothing in yet. The pieces this module will own, and where they live today:
//!
//! - the Source Quote Verifier: every factual claim carries a file:line that is read back (P1-9)
//! - the LSP Bridge: the editor's own diagnostics, not a guess (P1-12)
//! - the Hallucination Guard: a fabricated path or API stops the step (P3-3)
//! - Test Impact: run the tests the change can reach, not the whole suite (P3-4)
//!
//! Today the verification gate is `crate::verify`, which keeps running the plan/verdict flow the person already
//! uses. It moves here when there is a second verifier to sit beside it.
