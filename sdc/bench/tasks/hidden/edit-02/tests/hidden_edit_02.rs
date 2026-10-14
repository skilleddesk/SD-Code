use ledger_kit::*;

fn entry(account: &str, amount: i64) -> Entry {
    Entry { date: "2026-01-01".into(), account: account.into(), amount: Money(amount), memo: String::new() }
}

#[test]
fn zero_balances_are_not_top_accounts() {
    let mut ledger = Ledger::new();
    for name in ["cash", "idle", "food"] {
        ledger.open(name, AccountKind::Expense);
    }
    ledger.post(entry("cash", 500)).unwrap();
    ledger.post(entry("food", 200)).unwrap();
    let top = report::top_accounts(&ledger, 5);
    assert_eq!(top.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["cash", "food"]);
}
