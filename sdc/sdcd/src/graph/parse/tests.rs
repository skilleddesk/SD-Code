use super::*;

fn defs(path: &str, text: &str) -> Vec<(String, String, String)> {
    let lang = Lang::of(path).unwrap();
    let tree = parse(lang, text).unwrap();

    definitions(lang, &tree, text).into_iter().map(|def| (def.kind, def.name, def.qualified)).collect()
}

#[test]
fn rust_declarations_are_qualified_by_their_impl_trait_or_module() {
    let text = "pub struct Ledger { n: u32 }\nimpl Ledger {\n    pub fn total(&self) -> u32 { self.n }\n}\nimpl<T> Store<T> {\n    fn get(&self) {}\n}\npub trait Rule {\n    fn check(&self);\n}\nmod inner {\n    pub fn helper() {}\n}\nfn free() {}\n";
    let found = defs("a.rs", text);

    for expected in [("struct", "Ledger", "Ledger"), ("method", "total", "Ledger::total"), ("method", "get", "Store::get"), ("trait", "Rule", "Rule"), ("method", "check", "Rule::check"), ("mod", "inner", "inner"), ("fn", "helper", "inner::helper"), ("fn", "free", "free")] {
        assert!(found.iter().any(|(kind, name, qualified)| (kind.as_str(), name.as_str(), qualified.as_str()) == expected), "{expected:?} in {found:?}");
    }
}

#[test]
fn python_methods_are_class_dot_name_and_nested_functions_are_plain() {
    let found = defs("a.py", "class Store:\n    def add(self):\n        def inner():\n            pass\n        return inner\n\ndef top():\n    pass\n");

    assert!(found.contains(&("class".into(), "Store".into(), "Store".into())));
    assert!(found.contains(&("method".into(), "add".into(), "Store.add".into())));
    assert!(found.contains(&("def".into(), "top".into(), "top".into())));
}

#[test]
fn typescript_classes_methods_arrow_functions_and_interfaces_are_found() {
    let found = defs("a.ts", "export interface Shape { area(): number }\nexport class Circle implements Shape {\n  area(): number { return 1 }\n}\nexport const double = (n: number) => n * 2;\nfunction plain() {}\ntype Id = string;\n");

    for expected in [("interface", "Shape", "Shape"), ("class", "Circle", "Circle"), ("method", "area", "Circle.area"), ("fn", "double", "double"), ("fn", "plain", "plain"), ("type", "Id", "Id")] {
        assert!(found.iter().any(|(kind, name, qualified)| (kind.as_str(), name.as_str(), qualified.as_str()) == expected), "{expected:?} in {found:?}");
    }
}

#[test]
fn go_methods_name_their_receiver_and_php_methods_their_class() {
    let go = defs("a.go", "package main\ntype Server struct{}\nfunc (s *Server) Start() {}\nfunc main() {}\n");

    assert!(go.contains(&("method".into(), "Start".into(), "Server.Start".into())), "{go:?}");
    assert!(go.contains(&("type".into(), "Server".into(), "Server".into())));
    assert!(go.contains(&("fn".into(), "main".into(), "main".into())));

    let php = defs("a.php", "<?php\nclass Cart {\n  public function add($x) {}\n}\nfunction total() {}\n");

    assert!(php.contains(&("method".into(), "add".into(), "Cart::add".into())), "{php:?}");
    assert!(php.contains(&("fn".into(), "total".into(), "total".into())));
}

#[test]
fn a_declarations_hash_survives_a_reformat_but_not_a_change() {
    let original = "def f(a, b):\n    return a + b\n";
    let reformatted = "def f(a, b):\n\n\n    return   a   +   b\n";
    let changed = "def f(a, b):\n    return a - b\n";
    let first = |text: &str| definitions(Lang::Python, &parse(Lang::Python, text).unwrap(), text).remove(0);

    assert_ne!(first(original).hash, first(reformatted).hash, "the exact bytes differ");
    assert_eq!(first(original).norm_hash, first(reformatted).norm_hash, "a reformat is not a change");
    assert_ne!(first(original).norm_hash, first(changed).norm_hash, "a change is");
}

#[test]
fn syntax_errors_are_found_with_a_line_and_a_clean_file_has_none() {
    assert_eq!(check("a.rs", "fn ok() { let x = 1; }\n"), Some(vec![]));

    let broken = check("a.rs", "fn broken() {\n    let x = ;\n}\n").unwrap();

    assert!(!broken.is_empty() && broken[0].line == 2, "{broken:?}");
    assert!(!check("a.py", "def f(:\n    pass\n").unwrap().is_empty());
    assert!(!check("a.json", "{\"a\": 1,}").unwrap().is_empty());
    assert!(check("a.ts", "const a: number = 1;\n").unwrap().is_empty());
    assert_eq!(check("notes.txt", "anything"), None, "an unknown language has nothing to say");
}

#[test]
fn references_say_who_calls_and_imports_say_what_a_file_needs() {
    let text = "use crate::trust::policy::Policy;\nfn main() {\n    let p = Policy::default();\n    helper(p);\n}\nfn helper(_p: Policy) {}\n";
    let tree = parse(Lang::Rust, text).unwrap();
    let refs = references(&tree, text);

    assert!(refs.iter().any(|r| r.name == "helper" && r.call && r.line == 4), "{refs:?}");
    assert!(refs.iter().any(|r| r.name == "helper" && !r.call && r.line == 6), "the definition is a reference that is not a call");
    assert_eq!(imports(Lang::Rust, &tree, text), vec!["crate::trust::policy::Policy".to_string()]);

    let js = "import { a } from './store';\nconst b = require('./other');\n";
    let tree = parse(Lang::JavaScript, js).unwrap();

    assert_eq!(imports(Lang::JavaScript, &tree, js), vec!["./other".to_string(), "./store".to_string()]);

    let py = "import os.path\nfrom notes.dates import parse_due\n";
    let tree = parse(Lang::Python, py).unwrap();

    assert_eq!(imports(Lang::Python, &tree, py), vec!["notes.dates".to_string(), "os.path".to_string()]);
}
