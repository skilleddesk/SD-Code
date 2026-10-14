//! **Budget contract** (v3 I5, switch `v3.budget`): `@budget 40k` (tokens) or `@budget $0.10` (dollars) in a
//! message. The model is told the budget at the start; at 80% it is asked to converge (finish the essential part,
//! skip the optional); at 100% the turn stops and says how to continue. Nothing changes without `@budget`.

use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Budget {
    Tokens(u64),
    Dollars(f64),
}

/// The budget a message sets, if any.
pub fn parse(text: &str) -> Option<Budget> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();

    let pattern = PATTERN.get_or_init(|| Regex::new(r"(?i)@budget\s+(\$)?\s*(\d+(?:\.\d+)?)\s*(k|m)?\b").expect("a valid budget pattern"));
    let captures = pattern.captures(text)?;
    let number: f64 = captures[2].parse().ok()?;

    if captures.get(1).is_some() {
        return (number > 0.0).then_some(Budget::Dollars(number));
    }

    let scale = match captures.get(3).map(|unit| unit.as_str().to_ascii_lowercase()) {
        Some(unit) if unit == "k" => 1_000.0,
        Some(unit) if unit == "m" => 1_000_000.0,
        _ => 1.0,
    };
    let tokens = (number * scale) as u64;

    (tokens > 0).then_some(Budget::Tokens(tokens))
}

impl Budget {
    /// How much of the budget is used, 0.0-1.0+, from the turn's totals and the model's price (per million).
    pub fn used(&self, input: u64, output: u64, price: Option<(f64, f64)>) -> Option<f64> {
        match *self {
            Budget::Tokens(limit) => Some((input + output) as f64 / limit as f64),
            Budget::Dollars(limit) => price.map(|(input_price, output_price)| (input as f64 * input_price + output as f64 * output_price) / 1e6 / limit),
        }
    }

    pub fn label(&self) -> String {
        match *self {
            Budget::Tokens(limit) if limit >= 1_000 => format!("{}k tokens", limit / 1_000),
            Budget::Tokens(limit) => format!("{limit} tokens"),
            Budget::Dollars(limit) => format!("${limit:.2}"),
        }
    }

    /// The note the turn starts with.
    pub fn opening(&self) -> String {
        format!(
            "[SDC budget: this task has a budget of {} for everything you read and write. Plan for it: go straight to what matters, read ranges rather than whole files, and skip optional exploration. SDC will tell you at 80%.]",
            self.label()
        )
    }

    pub const CONVERGE: &'static str = "[SDC budget: 80% of the budget is used. Finish the essential part now, skip optional checks and polish, and give your answer.]";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_are_read_in_tokens_or_dollars() {
        assert_eq!(parse("fix it @budget 40k please"), Some(Budget::Tokens(40_000)));
        assert_eq!(parse("@BUDGET 1.5M"), Some(Budget::Tokens(1_500_000)));
        assert_eq!(parse("@budget 900"), Some(Budget::Tokens(900)));
        assert_eq!(parse("@budget $0.10 for this"), Some(Budget::Dollars(0.10)));
        assert_eq!(parse("no budget here"), None);
        assert_eq!(parse("@budget 0"), None);
    }

    #[test]
    fn usage_is_a_share_of_the_budget() {
        assert_eq!(Budget::Tokens(1_000).used(600, 200, None), Some(0.8));
        /* $1/M in, $4/M out: 100k in + 10k out = $0.14 of $0.28 = 50%. */
        assert!((Budget::Dollars(0.28).used(100_000, 10_000, Some((1.0, 4.0))).unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(Budget::Dollars(1.0).used(1, 1, None), None, "dollars need a price");
        assert_eq!(Budget::Tokens(40_000).label(), "40k tokens");
    }
}
