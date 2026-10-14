use ledger_kit::*;

#[test]
fn the_last_day_is_included() {
    let mut ledger = Ledger::new();
    ledger.open("food", AccountKind::Expense);
    for date in ["2026-01-01", "2026-01-31", "2026-02-01"] {
        ledger.post(Entry { date: date.into(), account: "food".into(), amount: Money(100), memo: String::new() }).unwrap();
    }
    let dates: Vec<&str> = ledger.entries_between("2026-01-01", "2026-01-31").iter().map(|entry| entry.date.as_str()).collect();
    assert_eq!(dates, ["2026-01-01", "2026-01-31"]);
}
