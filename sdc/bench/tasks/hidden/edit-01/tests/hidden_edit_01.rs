use ledger_kit::*;

#[test]
fn abs_drops_the_sign() {
    assert_eq!(Money(-310).abs(), Money(310));
    assert_eq!(Money(5).abs(), Money(5));
    assert_eq!(Money::ZERO.abs(), Money::ZERO);
}
