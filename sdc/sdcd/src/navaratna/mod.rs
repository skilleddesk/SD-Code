//! **NAVARATNA**: the nine pillars that let a small local model finish a large job. The plan is `NAVARATNA.md`; every behaviour here
//! sits behind an `nr.*` switch (see `v3::FEATURES`), and everything is a pure function or a small file format - the model's part is
//! always given in as a closure or a trait object, so the tests run the scripted way (no live model).
//!
//! | pillar | modules |
//! | ------ | ------- |
//! | 1 window truth | [`window`], [`advice`], [`membudget`] |
//! | 2 spec vault | [`spec`], [`langspec`], [`blueprint`] |
//! | 3 clean context | [`pack`], [`errslice`] |
//! | 4 surgical edit | [`symbols`], [`patcher`], [`gate`], [`chunk`] |
//! | 5 proof first | [`testfirst`], [`exist`] |
//! | 6 exactly once | [`registry`], [`shadow`], [`contracts`] |
//! | 7 many tries | [`bestofn`], [`envelope`], [`think`] |
//! | 8 smart relay | [`route`], [`packet`], [`roles`] |
//! | 9 learn and measure | [`golden`], [`bench`] |

pub mod advice;
pub mod bench;
pub mod bestofn;
pub mod blueprint;
pub mod chunk;
pub mod contracts;
pub mod envelope;
pub mod errslice;
pub mod exist;
pub mod gate;
pub mod langspec;
pub mod membudget;
pub mod pack;
pub mod packet;
pub mod golden;
pub mod patcher;
pub mod registry;
pub mod shadow;
pub mod roles;
pub mod route;
pub mod spec;
pub mod symbols;
pub mod think;
pub mod testfirst;
pub mod vault;
pub mod window;
