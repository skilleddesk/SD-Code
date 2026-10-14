use std::collections::BTreeMap;

use crate::error::LedgerError;
use crate::money::Money;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    /// Cash, bank: may not go below zero.
    Asset,
    Expense,
    Income,
}

/// One posting. Dates are ISO `YYYY-MM-DD`, so they compare correctly as strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub date: String,
    pub account: String,
    pub amount: Money,
    pub memo: String,
}

#[derive(Debug, Default)]
pub struct Ledger {
    accounts: BTreeMap<String, AccountKind>,
    entries: Vec<Entry>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, name: &str, kind: AccountKind) {
        self.accounts.insert(name.to_string(), kind);
    }

    pub fn kind(&self, name: &str) -> Option<AccountKind> {
        self.accounts.get(name).copied()
    }

    /// Adds an entry after checking it: the account must exist, and an asset account may not go below zero.
    pub fn post(&mut self, entry: Entry) -> Result<(), LedgerError> {
        let kind = self.kind(&entry.account).ok_or_else(|| LedgerError::UnknownAccount(entry.account.clone()))?;

        if kind == AccountKind::Asset {
            let after = self.balance(&entry.account) + entry.amount;

            if after.is_negative() {
                return Err(LedgerError::Overdraft { account: entry.account.clone(), short_by: -after });
            }
        }

        self.entries.push(entry);

        Ok(())
    }

    pub fn balance(&self, account: &str) -> Money {
        self.entries.iter().filter(|entry| entry.account == account).map(|entry| entry.amount).sum()
    }

    pub fn balances(&self) -> BTreeMap<String, Money> {
        self.accounts.keys().map(|name| (name.clone(), self.balance(name))).collect()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entries dated from `from` to `to`, both days included.
    pub fn entries_between(&self, from: &str, to: &str) -> Vec<&Entry> {
        self.entries.iter().filter(|entry| entry.date.as_str() >= from && entry.date.as_str() <= to).collect()
    }
}
