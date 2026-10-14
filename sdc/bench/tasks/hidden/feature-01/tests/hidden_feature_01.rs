use ledger_kit::*;

fn ledger() -> Ledger {
    let mut ledger = Ledger::new();
    ledger.open("cash", AccountKind::Asset);
    ledger.open("bank", AccountKind::Asset);
    ledger.post(Entry { date: "2026-01-01".into(), account: "bank".into(), amount: Money(10_000), memo: String::new() }).unwrap();
    ledger
}

#[test]
fn a_transfer_moves_money() {
    let mut ledger = ledger();
    ledger.transfer("2026-01-02", "bank", "cash", Money(2_500), "atm").unwrap();
    assert_eq!(ledger.balance("bank"), Money(7_500));
    assert_eq!(ledger.balance("cash"), Money(2_500));
}

#[test]
fn a_failed_transfer_posts_nothing() {
    let mut ledger = ledger();
    assert!(matches!(ledger.transfer("2026-01-02", "bank", "nowhere", Money(100), ""), Err(LedgerError::UnknownAccount(_))));
    assert!(matches!(ledger.transfer("2026-01-02", "cash", "bank", Money(100), ""), Err(LedgerError::Overdraft { .. })));
    assert_eq!(ledger.entries().len(), 1);
}
