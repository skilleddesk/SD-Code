//! The error module: turning a machine failure into a sentence (master spec section 14.9).

pub mod translator;

pub use translator::{translate, Translation};
