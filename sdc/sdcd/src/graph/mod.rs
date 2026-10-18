//! **The Code Graph (P1-1, switch `graph.v1`)**: what a project declares, where names are used, what files import, and which tests touch
//! which code - answered from tree-sitter parses cached by content hash, with **zero model calls**.
//!
//! * [`parse`] - the grammars (Rust, Python, JavaScript, TypeScript, TSX, Go, PHP, JSON, Bash), declarations with qualified names and
//!   hashes, references, imports, and syntax errors.
//! * [`index`] - the Merkle index: each file's artifacts are keyed by its content hash, a refresh re-parses only files whose content
//!   changed, and a branch switch cannot serve stale answers because the key is the content, not the path or the time.
//! * the queries `where`, `callers`, `tests`, `changed` (`index::Index`), used by the `graph.query` method, the `code_graph` tool and the
//!   @ picker.

pub mod index;
pub mod parse;

pub use parse::{check, Def, Lang, Reference, SyntaxError};
