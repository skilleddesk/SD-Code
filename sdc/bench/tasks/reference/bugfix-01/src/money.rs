use std::fmt;
use std::ops::{Add, Neg, Sub};

use crate::error::LedgerError;

/// An amount of money in cents. Cents are kept as an integer so that sums never pick up the rounding error
/// a binary float would (0.1 + 0.2 != 0.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Money(pub i64);

impl Money {
    pub const ZERO: Money = Money(0);

    pub fn cents(self) -> i64 {
        self.0
    }

    /// Reads "12", "12.5", "12.34" or "-7.10" (a leading minus, at most two decimals).
    pub fn parse(text: &str) -> Result<Money, LedgerError> {
        let text = text.trim();
        let (whole, fraction) = match text.split_once('.') {
            Some((whole, fraction)) => (whole, fraction),
            None => (text, ""),
        };

        if fraction.len() > 2 || !fraction.chars().all(|c| c.is_ascii_digit()) {
            return Err(LedgerError::Parse(format!("amount `{text}`")));
        }

        let whole: i64 = whole.parse().map_err(|_| LedgerError::Parse(format!("amount `{text}`")))?;
        let mut cents: i64 = match fraction.len() {
            0 => 0,
            1 => fraction.parse::<i64>().unwrap_or(0) * 10,
            _ => fraction.parse::<i64>().unwrap_or(0),
        };

        if whole < 0 || text.starts_with('-') {
            cents = -cents;
        }

        Ok(Money(whole * 100 + cents))
    }

    pub fn is_negative(self) -> bool {
        self.0 < 0
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.abs();

        write!(f, "{sign}{}.{:02}", abs / 100, abs % 100)
    }
}

impl Add for Money {
    type Output = Money;

    fn add(self, other: Money) -> Money {
        Money(self.0 + other.0)
    }
}

impl Sub for Money {
    type Output = Money;

    fn sub(self, other: Money) -> Money {
        Money(self.0 - other.0)
    }
}

impl Neg for Money {
    type Output = Money;

    fn neg(self) -> Money {
        Money(-self.0)
    }
}

impl std::iter::Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        iter.fold(Money::ZERO, Add::add)
    }
}
