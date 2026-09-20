//! The console module: turning a preview's errors into turns (master spec section 15.4).

pub mod bridge;

pub use bridge::{parse_console_line, ConsoleBridge, ConsoleLine};
