use ledger_kit::*;

#[test]
fn negative_amounts_under_one_keep_their_sign() {
    assert_eq!(Money::parse("-0.50").unwrap(), Money(-50));
    assert_eq!(Money::parse("-0.5").unwrap(), Money(-50));
    assert_eq!(Money::parse("-1.25").unwrap(), Money(-125));
    assert_eq!(Money::parse("0.75").unwrap(), Money(75));
}
