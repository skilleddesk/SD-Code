use ledger_kit::*;

#[test]
fn overdraft_says_how_much() {
    let error = LedgerError::Overdraft { account: "cash".into(), short_by: Money(5000) };
    assert_eq!(error.to_string(), "overdraft on cash: short by 50.00");
}
