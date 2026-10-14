//! Categories for a memo, by keyword. The first keyword found wins, so the table is ordered from specific
//! to general.

const RULES: &[(&str, &str)] = &[
    ("salary", "income"),
    ("refund", "income"),
    ("rent", "housing"),
    ("electric", "utilities"),
    ("internet", "utilities"),
    ("coffee", "food-and-drink"),
    ("restaurant", "food-and-drink"),
    ("grocer", "groceries"),
    ("uber", "transport"),
    ("fuel", "transport"),
];

/// The category of a memo, or `"uncategorised"`.
pub fn categorize(memo: &str) -> &'static str {
    let memo = memo.to_lowercase();

    RULES.iter().find(|(keyword, _)| memo.contains(keyword)).map(|(_, category)| *category).unwrap_or("uncategorised")
}
