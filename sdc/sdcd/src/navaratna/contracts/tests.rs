use super::*;

const SHOP: &str = "pub fn total(items: &[u64]) -> u64 {\n    fn helper(x: u64) -> u64 { x }\n    items.iter().map(|i| helper(*i)).sum()\n}\n\npub struct Cart {\n    pub items: Vec<u64>,\n}\n\nimpl Cart {\n    pub fn add(&mut self, item: u64) {\n        self.items.push(item);\n    }\n}\n";
const MAIN: &str = "fn main() {\n    let t = total(&[1, 2]);\n    let u = total(&[3]);\n}\n";

fn files(shop: &str, main: &str) -> Vec<(String, String)> {
    vec![("src/shop.rs".to_string(), shop.to_string()), ("src/main.rs".to_string(), main.to_string())]
}

fn locked() -> Lock {
    let mut lock = Lock::default();

    crate::v3::forcing(&["nr.contract_lock"], || lock.publish("src/shop.rs", SHOP, "s1"));
    lock
}

#[test]
fn publishing_locks_public_signatures_but_not_helpers_inside_a_body() {
    let lock = locked();
    let keys: Vec<&str> = lock.signatures.keys().map(String::as_str).collect();

    assert!(keys.contains(&"src/shop.rs::total") && keys.contains(&"src/shop.rs::Cart::add") && keys.contains(&"src/shop.rs::Cart"), "{keys:?}");
    assert!(!keys.iter().any(|k| k.ends_with("helper")), "{keys:?}");
    assert_eq!(lock.signatures["src/shop.rs::total"].signature, "pub fn total(items: &[u64]) -> u64");
    assert_eq!(lock.signatures["src/shop.rs::total"].step, "s1");
}

#[test]
fn publishing_again_never_moves_a_lock() {
    let mut lock = locked();
    let again = SHOP.replace("-> u64 {\n    fn helper", "-> u128 {\n    fn helper");

    crate::v3::forcing(&["nr.contract_lock"], || assert_eq!(lock.publish("src/shop.rs", &again, "s2"), 0));
    assert_eq!(lock.signatures["src/shop.rs::total"].step, "s1");
}

#[test]
fn a_signature_change_outside_a_contract_change_step_is_refused() {
    crate::v3::forcing(&["nr.contract_lock"], || {
        let lock = locked();
        let changed = SHOP.replace("pub fn total(items: &[u64]) -> u64", "pub fn total(items: &[u64], tax: u64) -> u64");
        let violations = check_patch(&lock, "src/shop.rs", &changed).unwrap_err();

        assert_eq!(violations.len(), 1);
        assert!(violations[0].to_string().contains("locked contract") && violations[0].to_string().contains("step s1"), "{}", violations[0]);

        let renamed = SHOP.replace("pub fn total", "pub fn grand_total");

        assert!(matches!(&check_patch(&lock, "src/shop.rs", &renamed).unwrap_err()[0], Violation::Removed { symbol, .. } if symbol == "total"));

        let body_only = SHOP.replace("items.iter().map(|i| helper(*i)).sum()", "items.iter().sum()");

        assert!(check_patch(&lock, "src/shop.rs", &body_only).is_ok(), "a body change is not a contract change");
        assert!(check_patch(&lock, "src/other.rs", "pub fn total() {}").is_ok(), "another file holds no lock of this one");
    });
}

#[test]
fn off_by_default_nothing_is_locked_and_nothing_refused() {
    let mut lock = Lock::default();

    assert_eq!(lock.publish("src/shop.rs", SHOP, "s1"), 0);
    assert!(check_patch(&locked(), "src/shop.rs", "pub fn nothing() {}").is_ok());
}

#[test]
fn a_contract_change_step_must_list_the_symbol_and_update_every_call_site() {
    crate::v3::forcing(&["nr.contract_lock"], || {
        let before = files(SHOP, MAIN);
        let new_shop = SHOP.replace("pub fn total(items: &[u64]) -> u64", "pub fn total(items: &[u64], tax: u64) -> u64");
        let sites = call_sites(&before, "total");

        assert_eq!(sites, vec!["src/main.rs:2".to_string(), "src/main.rs:3".to_string()]);

        let change = ContractChange { symbols: vec!["src/shop.rs::total".into()] };

        /* the signature changes but the call sites do not */
        let mut lock = locked();
        let stale = apply_contract_change(&mut lock, &change, &before, &files(&new_shop, MAIN)).unwrap_err();

        assert!(stale.iter().all(|v| matches!(v, Violation::CallSiteNotUpdated { .. })) && stale.len() == 2, "{stale:?}");
        assert_eq!(lock.signatures["src/shop.rs::total"].signature, "pub fn total(items: &[u64]) -> u64", "the lock did not move");

        /* the symbol is not listed */
        let unlisted = apply_contract_change(&mut lock, &ContractChange { symbols: vec![] }, &before, &files(&new_shop, "fn main() { total(&[1], 2); total(&[3], 4); }\n")).unwrap_err();

        assert!(matches!(&unlisted[0], Violation::NotListed { .. }), "{unlisted:?}");

        /* done properly */
        let new_main = "fn main() {\n    let t = total(&[1, 2], 7);\n    let u = total(&[3], 7);\n}\n";

        apply_contract_change(&mut lock, &change, &before, &files(&new_shop, new_main)).unwrap();
        assert_eq!(lock.signatures["src/shop.rs::total"].signature, "pub fn total(items: &[u64], tax: u64) -> u64");
        assert_eq!(lock.signatures["src/shop.rs::total"].step, "contract-change");
        assert!(check_patch(&lock, "src/shop.rs", &new_shop).is_ok(), "the new signature is the locked one now");
    });
}

#[test]
fn the_lock_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();

    locked().save(dir.path()).unwrap();
    assert_eq!(Lock::load(dir.path()), locked());
}

#[test]
fn the_glossary_collects_constants_and_refuses_a_second_meaning() {
    let mut glossary = Glossary::default();

    assert!(glossary.harvest("src/c.rs", "pub const MAX_ITEMS: usize = 50;\nstatic RATE: f64 = 0.035;\nconst lower: u8 = 1;\n").is_empty());
    assert!(glossary.harvest("c.py", "TAX_RATE = 0.035\nname = 'x'\n").is_empty());
    assert!(glossary.harvest("c.ts", "export const API_URL = 'https://x.test';\n").is_empty());
    assert_eq!(glossary.entries.get("MAX_ITEMS").map(String::as_str), Some("50"));
    assert_eq!(glossary.entries.get("RATE").map(String::as_str), Some("0.035"));
    assert!(!glossary.entries.contains_key("lower") && !glossary.entries.contains_key("name"));

    let conflicts = glossary.harvest("other.rs", "const MAX_ITEMS: usize = 99;\n");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(glossary.entries["MAX_ITEMS"], "50", "the first meaning stands");
    assert!(glossary.add("cart", "the list of items").is_ok() && glossary.add("cart", "the list of items").is_ok() && glossary.add("cart", "a trolley").is_err());
    assert!(glossary.pairs().iter().any(|(t, m)| t == "cart" && m == "the list of items"));
}
