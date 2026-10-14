use std::collections::BTreeMap;

use crate::ledger::Ledger;
use crate::money::Money;

/// Net movement per month (`YYYY-MM`), all accounts together.
pub fn monthly_totals(ledger: &Ledger) -> BTreeMap<String, Money> {
    let mut totals: BTreeMap<String, Money> = BTreeMap::new();

    for entry in ledger.entries() {
        let month = entry.date.get(..7).unwrap_or(&entry.date).to_string();

        *totals.entry(month).or_default() = totals.get(&entry.date[..7]).copied().unwrap_or_default() + entry.amount;
    }

    totals
}

/// The `n` accounts that moved the most money (by absolute balance), largest first.
pub fn top_accounts(ledger: &Ledger, n: usize) -> Vec<(String, Money)> {
    let mut all: Vec<(String, Money)> = ledger.balances().into_iter().filter(|(_, money)| money.0 != 0).collect();

    all.sort_by(|a, b| b.1 .0.abs().cmp(&a.1 .0.abs()));
    all.truncate(n);
    all
}
