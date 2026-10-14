use std::fmt;

use crate::money::Money;

/// Everything that can go wrong while building a ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// A line of input that could not be read.
    Parse(String),
    /// An entry names an account the ledger does not have.
    UnknownAccount(String),
    /// Posting would take an asset account below zero.
    Overdraft { account: String, short_by: Money },
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedgerError::Parse(what) => write!(f, "could not parse: {what}"),
            LedgerError::UnknownAccount(name) => write!(f, "unknown account `{name}`"),
            LedgerError::Overdraft { account, short_by } => write!(f, "overdraft on {account}: short by {short_by}"),
        }
    }
}

impl std::error::Error for LedgerError {}
