//! The subject-matter expert techniques: what SDC does with what it read, so a small model can work on a large
//! repository.
//!
//! P0-3 opens the seam and moves nothing in yet. The pieces this module will own, and where they live today:
//!
//! - Working Memory: the few facts a step is allowed to carry (P1-7) - today the prompt is assembled from the
//!   whole history in `crate::agent`
//! - the Chunked Reader and the Retrieval Cascade: read a big file in pieces, best source first (P1-8)
//! - the Cut-off Guard: detect a reply that was cut off mid-thought and ask again (P1-11)
//! - the Bangla/Banglish spec brief (P1-14)
//! - the Capability Probe and model identity (P0-9) - **here now**, `sme::probe`
//!
//! Empty on purpose until those tasks land, so adding them does not mean rearranging the agent again.

pub mod ollama_probe;
pub mod probe;
