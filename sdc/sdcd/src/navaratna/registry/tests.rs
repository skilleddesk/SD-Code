use super::*;

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn plan(steps: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    steps.iter().map(|(s, r)| (s.to_string(), ids(r))).collect()
}

#[test]
fn a_plan_with_one_owner_per_requirement_is_accepted() {
    assert!(one_owner(&ids(&["R-001", "R-002", "R-003"]), &plan(&[("s1", &["R-001"]), ("s2", &["R-002", "R-003"])])).is_ok());
}

#[test]
fn double_ownership_a_missing_owner_and_a_made_up_requirement_are_rejected_by_name() {
    let problems = one_owner(&ids(&["R-001", "R-002", "R-003"]), &plan(&[("s1", &["R-001", "R-002"]), ("s2", &["R-002", "R-009"])])).unwrap_err();

    assert_eq!(problems.unowned, ids(&["R-003"]));
    assert_eq!(problems.shared, vec![("R-002".to_string(), ids(&["s1", "s2"]))]);
    assert_eq!(problems.unknown, vec![("R-009".to_string(), "s2".to_string())]);

    let text = problems.to_string();

    assert!(text.starts_with("the plan is rejected") && text.contains("R-003") && text.contains("R-002 is owned by 2 steps"), "{text}");
}

#[test]
fn a_create_step_becomes_an_edit_when_the_name_exists_and_the_reason_is_recorded() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/shop.rs"), "pub fn total(items: &[u64]) -> u64 {\n    items.iter().sum()\n}\n").unwrap();

    let mut registry = Registry::default();

    registry.record(Entry { artifact: "src/cart.rs".into(), r_id: "R-001".into(), step: "s1".into(), model: "local".into(), hash: "h".into(), evidence: "src/cart.rs:1".into() });

    assert_eq!(before_create(&mut registry, dir.path(), "src/new.rs"), CreateDecision::Create);
    assert!(matches!(before_create(&mut registry, dir.path(), "src/shop.rs::total"), CreateDecision::ConvertToEdit { why, .. } if why.contains("src/shop.rs:1")));
    assert!(matches!(before_create(&mut registry, dir.path(), "src/cart.rs"), CreateDecision::ConvertToEdit { why, .. } if why.contains("step s1")));
    assert!(matches!(before_create(&mut registry, dir.path(), "src/shop.rs"), CreateDecision::ConvertToEdit { .. }));
    assert_eq!(registry.conversions.len(), 3);
}

#[test]
fn a_retry_replaces_its_row_and_the_registry_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::default();
    let row = |hash: &str| Entry { artifact: "src/a.rs::f".into(), r_id: "R-001".into(), step: "s1".into(), model: "local".into(), hash: hash.into(), evidence: "t".into() };

    registry.record(row("one"));
    registry.record(row("two"));
    registry.save(dir.path()).unwrap();

    let back = Registry::load(dir.path());

    assert_eq!(back.entries.len(), 1);
    assert_eq!(back.entries[0].hash, "two");
}

const FIRST: &str = "pub fn compute_tax(amount: f64, rate: f64) -> f64 {\n    let base = amount * rate;\n    let rounded = (base * 100.0).round() / 100.0;\n    rounded\n}\n";

#[test]
fn a_duplicate_body_and_a_same_name_symbol_are_detected_and_nothing_is_deleted() {
    let reformatted = "pub fn compute_tax(amount: f64, rate: f64) -> f64 {\n        let base = amount * rate;\n\n    let rounded = (base * 100.0).round() / 100.0;\n    rounded\n}\n";
    let renamed_copy = FIRST.replace("compute_tax", "tax_of");
    let files = vec![("src/new.rs".to_string(), renamed_copy.clone()), ("src/old.rs".to_string(), FIRST.to_string())];
    let copy = find_duplicates(&files, "src/new.rs");

    assert!(copy.iter().any(|d| d.kind == DuplicateKind::NearIdentical), "a copy under another name is still a copy: {copy:?}");
    assert!(find_duplicates(&[("src/new.rs".to_string(), reformatted.to_string()), ("src/old.rs".to_string(), FIRST.to_string())], "src/new.rs").iter().any(|d| d.kind == DuplicateKind::NearIdentical && d.a == "src/new.rs:1" && d.b == "src/old.rs:1"));

    let same_name = find_duplicates(&[("src/new.rs".to_string(), "pub fn compute_tax(a: u64) -> u64 { a }\n".to_string()), ("src/old.rs".to_string(), FIRST.to_string())], "src/new.rs");

    assert_eq!(same_name.len(), 1);
    assert_eq!(same_name[0].kind, DuplicateKind::SameName);
    assert!(same_name[0].to_string().contains("declared twice"));
}

#[test]
fn common_names_tests_and_short_bodies_are_not_reported() {
    let main_a = ("a/main.rs".to_string(), "fn main() {\n    println!(\"a\");\n}\n".to_string());
    let main_b = ("b/main.rs".to_string(), "fn main() {\n    println!(\"b\");\n}\n".to_string());

    assert!(find_duplicates(&[main_a, main_b], "a/main.rs").is_empty());

    let short_a = ("a.rs".to_string(), "pub fn one() -> u64 { 1 }\n".to_string());
    let short_b = ("b.rs".to_string(), "pub fn two() -> u64 { 1 }\n".to_string());

    assert!(find_duplicates(&[short_a, short_b], "a.rs").is_empty());
}

#[test]
fn the_note_reads_only_files_that_mention_the_name_and_warns_without_touching_anything() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/old.rs"), FIRST).unwrap();
    std::fs::write(dir.path().join("src/unrelated.rs"), "pub fn other() {}\n").unwrap();

    let note = duplicate_note(dir.path(), "src/new.rs", &FIRST.replace("    rounded\n", "    rounded // copy\n")).expect("a warning");

    assert!(note.contains("compute_tax") && note.contains("src/old.rs:1") && note.contains("nothing was changed or deleted"), "{note}");
    assert!(dir.path().join("src/old.rs").exists());
    assert_eq!(duplicate_note(dir.path(), "src/new2.rs", "pub fn something_else() { }\n"), None);
    crate::v3::ablating(&["nr.registry"], || assert_eq!(duplicate_note(dir.path(), "src/new.rs", FIRST), None));
}

#[test]
fn the_final_receipt_lists_every_requirement_with_step_model_and_evidence() {
    let mut registry = Registry::default();

    registry.record(Entry { artifact: "src/a.rs::f".into(), r_id: "R-001".into(), step: "s1".into(), model: "ollama:qwen3.5:9b".into(), hash: "h".into(), evidence: "src/a.rs:12".into() });
    registry.record(Entry { artifact: "tests/t.rs".into(), r_id: "R-002".into(), step: "s2".into(), model: "deepseek:deepseek-chat".into(), hash: "h".into(), evidence: "test_total".into() });

    let lines = receipt(&ids(&["R-001", "R-002", "R-003"]), &registry);

    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], ReceiptLine { r_id: "R-001".into(), step: "s1".into(), model: "ollama:qwen3.5:9b".into(), evidence: "src/a.rs:12".into() });
    assert_eq!(lines[1].evidence, "test_total");
    assert_eq!(lines[2].evidence, "none recorded", "a requirement with no proof says so");
}
