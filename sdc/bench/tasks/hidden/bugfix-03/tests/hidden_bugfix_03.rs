use ledger_kit::*;

#[test]
fn a_quoted_memo_may_hold_commas() {
    let rows = import::parse_csv("date,account,amount,memo\n2026-05-02,cash,-12.00,\"Lunch, team\"\n2026-05-03,cash,4,plain\n").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].memo, "Lunch, team");
    assert_eq!(rows[0].amount, Money(-1200));
    assert_eq!(rows[1].memo, "plain");
}
