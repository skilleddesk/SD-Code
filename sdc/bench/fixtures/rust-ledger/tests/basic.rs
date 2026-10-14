use ledger_kit::{import, report, rules, AccountKind, Entry, Ledger, LedgerError, Money};

fn entry(date: &str, account: &str, amount: &str) -> Entry {
    Entry { date: date.into(), account: account.into(), amount: Money::parse(amount).unwrap(), memo: String::new() }
}

#[test]
fn money_parses_and_prints() {
    assert_eq!(Money::parse("12.34").unwrap(), Money(1234));
    assert_eq!(Money::parse("12.5").unwrap(), Money(1250));
    assert_eq!(Money::parse("-7.10").unwrap(), Money(-710));
    assert_eq!(Money(-710).to_string(), "-7.10");
    assert!(Money::parse("1.234").is_err());
}

#[test]
fn posting_checks_accounts_and_overdrafts() {
    let mut ledger = Ledger::new();

    ledger.open("cash", AccountKind::Asset);
    ledger.post(entry("2026-01-02", "cash", "100")).unwrap();
    assert_eq!(ledger.post(entry("2026-01-03", "bank", "5")), Err(LedgerError::UnknownAccount("bank".into())));
    assert!(matches!(ledger.post(entry("2026-01-04", "cash", "-150")), Err(LedgerError::Overdraft { .. })));
    assert_eq!(ledger.balance("cash"), Money(10_000));
}

#[test]
fn csv_import_reads_plain_rows() {
    let rows = import::parse_csv("date,account,amount,memo\n2026-02-01,cash,12.50,coffee\n\n2026-02-02,cash,-3,bus\n").unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].amount, Money(-300));
}

#[test]
fn reports_and_rules() {
    let mut ledger = Ledger::new();

    ledger.open("cash", AccountKind::Asset);
    ledger.open("food", AccountKind::Expense);
    ledger.post(entry("2026-03-01", "cash", "50")).unwrap();
    ledger.post(entry("2026-03-15", "food", "20")).unwrap();
    ledger.post(entry("2026-04-01", "food", "5")).unwrap();

    assert_eq!(report::monthly_totals(&ledger)["2026-03"], Money(7_000));
    assert_eq!(report::top_accounts(&ledger, 1)[0].0, "cash");
    assert_eq!(rules::categorize("Morning COFFEE"), "food-and-drink");
}
