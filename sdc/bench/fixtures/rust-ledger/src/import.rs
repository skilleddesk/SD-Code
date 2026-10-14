//! CSV import: `date,account,amount,memo` with a header line. A memo may be wrapped in double quotes.

use crate::error::LedgerError;
use crate::ledger::Entry;
use crate::money::Money;

/// Splits one CSV line into its fields.
pub fn split_fields(line: &str) -> Vec<String> {
    line.split(',').map(|field| field.trim().trim_matches('"').to_string()).collect()
}

/// Reads every entry of a CSV export. Blank lines are skipped.
pub fn parse_csv(text: &str) -> Result<Vec<Entry>, LedgerError> {
    let mut entries = Vec::new();

    for (number, line) in text.lines().enumerate().skip(1) {
        if line.trim().is_empty() {
            continue;
        }

        let fields = split_fields(line);

        if fields.len() != 4 {
            return Err(LedgerError::Parse(format!("line {}: expected 4 fields, found {}", number + 1, fields.len())));
        }

        entries.push(Entry {
            date: fields[0].clone(),
            account: fields[1].clone(),
            amount: Money::parse(&fields[2])?,
            memo: fields[3].clone(),
        });
    }

    Ok(entries)
}
