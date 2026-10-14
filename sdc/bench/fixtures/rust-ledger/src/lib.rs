//! ledger_kit - a small double-entry-ish ledger: money parsing, posting rules, CSV import and reports.

pub mod error;
pub mod import;
pub mod ledger;
pub mod money;
pub mod report;
pub mod rules;

pub use error::LedgerError;
pub use ledger::{AccountKind, Entry, Ledger};
pub use money::Money;
