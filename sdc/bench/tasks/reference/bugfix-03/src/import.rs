//! CSV import: `date,account,amount,memo` with a header line. A memo may be wrapped in double quotes.

use crate::error::LedgerError;
use crate::ledger::Entry;
use crate::money::Money;

/// Splits one CSV line into its fields.
pub fn split_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;

    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut current).trim().to_string()),
            _ => current.push(c),
        }
    }

    fields.push(current.trim().to_string());
    fields
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
