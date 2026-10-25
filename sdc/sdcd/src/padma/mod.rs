//! **PADMA**: the Conductor, the Finish Gate, the Recovery Ladder and the rest of the layer that makes a turn end only when the work
//! is done (see `docs/padma/PADMA.md`). Every part sits behind a `padma.*` switch (`v3::FEATURES`).

pub mod english;
pub mod evidence;
pub mod extract;
pub mod gate;
pub mod hints;
pub mod intent;
pub mod ladder;
pub mod ledger;
pub mod model;
pub mod bind;
pub mod codefns;
pub mod engines;
pub mod conductor;
pub mod budget;
pub mod notice;
pub mod fixfns;
pub mod pagefns;
pub mod recipe;
pub mod siteprobe;
pub mod scope;
pub mod think;
pub mod tools;
pub mod task;
