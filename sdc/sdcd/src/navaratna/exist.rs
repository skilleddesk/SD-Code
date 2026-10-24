//! **N5-2 - the existence gate for packages and APIs** (switch `nr.exist_gate`, on, warn only).
//!
//! A model invents packages and methods. After an edit this looks at the new text and answers, with no model and no network:
//!
//! * a **package** that is not in the installed dependencies - the M4-1 check (`verify::api_exists`), which this builds on;
//! * a **member of a project type** that the symbol index does not know: `Shape::volume()` when `Shape` is a struct of this project and
//!   no `impl Shape` has a `volume`, `Cart.checkout()` when the class has no such method.
//!
//! The answer is a warning attached to the step; nothing is blocked (a future `block` setting would turn it into a failure). A name the
//! check cannot judge - an enum variant, a derived function, a type it has not seen - is not reported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;

use super::symbols;
use crate::graph::parse::Lang;

/// Functions every type may have without writing them (derives and the standard traits), in the languages checked.
const IMPLICIT: &[&str] = &[
    "new", "default", "clone", "from", "into", "try_from", "try_into", "fmt", "eq", "ne", "hash", "cmp", "partial_cmp", "to_string", "to_owned", "borrow", "as_ref", "as_mut", "deref", "drop", "from_str", "from_iter", "into_iter", "iter", "len", "is_empty", "constructor",
    "toString", "valueOf", "call", "apply", "bind", "prototype", "name", "__init__", "__str__", "__repr__", "__class__", "mro",
];

#[derive(Debug, Default, Clone)]
pub struct ProjectIndex {
    /// Struct/class name -> its members (methods, associated functions, fields).
    pub types: BTreeMap<String, BTreeSet<String>>,
    /// Names declared as an enum or trait: their members are not judged.
    pub opaque: BTreeSet<String>,
}

impl ProjectIndex {
    pub fn add_file(&mut self, path: &str, text: &str) {
        let Some(symbols) = symbols::index_text(path, text) else { return };
        let separator = if Lang::of(path) == Some(Lang::Rust) || Lang::of(path) == Some(Lang::Php) { "::" } else { "." };

        for symbol in &symbols {
            match symbol.kind.as_str() {
                "struct" | "class" => {
                    self.types.entry(symbol.name.clone()).or_default();
                }
                "enum" | "trait" | "interface" | "type" => {
                    self.opaque.insert(symbol.name.clone());
                }
                _ => {}
            }
        }

        for symbol in &symbols {
            if let Some((owner, member)) = symbol.qualified.rsplit_once(separator) {
                let owner = owner.rsplit(separator).next().unwrap_or(owner).trim_start_matches("impl ");

                self.types.entry(owner.to_string()).or_default().insert(member.to_string());
            }
        }

        /* fields: `name: Type` lines inside a struct, `self.x =` in Python - a field is a member too */
        let field = Regex::new(r"(?m)^\s+(?:pub(?:\([a-z]+\))?\s+)?([A-Za-z_]\w*)\s*:").expect("a valid pattern");
        let attribute = Regex::new(r"self\.([A-Za-z_]\w*)\s*=").expect("a valid pattern");

        for symbol in symbols.iter().filter(|s| s.kind == "struct" || s.kind == "class") {
            let body = &text[symbol.start_byte..symbol.end_byte];

            for capture in field.captures_iter(body) {
                self.types.entry(symbol.name.clone()).or_default().insert(capture[1].to_string());
            }

            for capture in attribute.captures_iter(body) {
                self.types.entry(symbol.name.clone()).or_default().insert(capture[1].to_string());
            }
        }
    }
}

/// The `(Type, member)` calls of a text that name a capitalised identifier followed by `::member(` or `.member(`.
fn candidate_calls(path: &str, text: &str) -> Vec<(String, String)> {
    let pattern = match Lang::of(path) {
        Some(Lang::Rust | Lang::Php) => r"\b([A-Z][A-Za-z0-9_]*)::([a-z_][A-Za-z0-9_]*)\s*\(",
        Some(Lang::Python | Lang::JavaScript | Lang::TypeScript | Lang::Tsx) => r"\b([A-Z][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)\s*\(",
        _ => return Vec::new(),
    };
    let mut found: Vec<(String, String)> = Regex::new(pattern).expect("a valid pattern").captures_iter(text).map(|c| (c[1].to_string(), c[2].to_string())).collect();

    found.sort();
    found.dedup();
    found
}

/// The warnings for members of project types that do not exist. `index` must have read the project (and the edited file's new text).
pub fn unknown_members(index: &ProjectIndex, path: &str, text: &str) -> Vec<String> {
    candidate_calls(path, text)
        .into_iter()
        .filter(|(ty, member)| !IMPLICIT.contains(&member.as_str()) && !index.opaque.contains(ty))
        .filter_map(|(ty, member)| {
            let members = index.types.get(&ty)?;

            (!members.contains(&member)).then(|| {
                let near = crate::verify::api_exists::nearest(&member, members);

                format!("`{ty}` is a type of this project and has no member `{member}`{}", if near.is_empty() { String::new() } else { format!(" - did you mean {}?", near.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")) })
            })
        })
        .collect()
}

/// Reads the project files that mention one of the candidate types, up to a cap, into an index.
pub fn index_for(root: &Path, candidate_types: &BTreeSet<String>, new_path: &str, new_text: &str) -> ProjectIndex {
    let mut index = ProjectIndex::default();

    let started = std::time::Instant::now();
    let mut read = 0;

    for relative in crate::search::files(root, 4_000) {
        /* this runs after an edit that calls a member of a project type: a bounded number of files and a bounded time */
        if read >= 60 || started.elapsed() > std::time::Duration::from_millis(250) {
            break;
        }

        if relative == new_path || symbols::no_grammar(&relative) {
            continue;
        }

        let Ok(text) = std::fs::read_to_string(root.join(&relative)) else { continue };

        if text.len() > 200_000 || !candidate_types.iter().any(|ty| text.contains(ty.as_str())) {
            continue;
        }

        index.add_file(&relative, &text);
        read += 1;
    }

    index.add_file(new_path, new_text);
    index
}

/// The whole gate for one edited file: warnings about unknown packages and unknown members, as one note, or `None`. Off: `None`.
pub fn note(root: &Path, path: &str, text: &str) -> Option<String> {
    if !crate::v3::on("nr.exist_gate") {
        return None;
    }

    let candidates = candidate_calls(path, text);

    if candidates.is_empty() {
        return None;
    }

    let types: BTreeSet<String> = candidates.iter().map(|(ty, _)| ty.clone()).collect();
    let index = index_for(root, &types, path, text);
    let lines = unknown_members(&index, path, text);

    if lines.is_empty() {
        return None;
    }

    Some(format!("[SDC check - a warning, nothing was blocked] This edit calls members the project's symbol index does not have:\n- {}\nCheck the declaration before relying on it (a method that comes from a trait or a derive is not in the index).", lines.join("\n- ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPES: &str = "pub struct Shape {\n    pub side: f64,\n}\n\nimpl Shape {\n    pub fn new(side: f64) -> Self { Shape { side } }\n    pub fn area(&self) -> f64 { self.side * self.side }\n}\n\npub enum Kind { Round, Flat }\n";

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/shape.rs"), SHAPES).unwrap();
        dir
    }

    #[test]
    fn a_made_up_method_of_a_project_type_is_flagged_and_a_real_one_is_not() {
        let dir = project();
        let fake = "fn main() {\n    let s = Shape::new(2.0);\n    let v = Shape::volume(&s);\n}\n";
        let real = "fn main() {\n    let s = Shape::new(2.0);\n    let a = Shape::area(&s);\n}\n";
        let ablated_note = crate::v3::ablating(&["nr.exist_gate"], || note(dir.path(), "src/main.rs", fake));

        assert_eq!(ablated_note, None, "off means silent");

        let flagged = note(dir.path(), "src/main.rs", fake).expect("the fake method is flagged");

        assert!(flagged.contains("`Shape` is a type of this project and has no member `volume`"), "{flagged}");
        assert_eq!(note(dir.path(), "src/main.rs", real), None, "a real member is not reported");
    }

    #[test]
    fn enums_derives_and_unknown_types_are_not_judged() {
        let dir = project();

        for text in ["fn f() { let k = Kind::Round(); }", "fn f() { Shape::default(); Shape::clone(&x); }", "fn f() { Vec::with_capacity(3); Missing::thing(); }"] {
            assert_eq!(note(dir.path(), "src/main.rs", text), None, "{text}");
        }
    }

    #[test]
    fn python_and_typescript_classes_are_checked_too() {
        let mut index = ProjectIndex::default();

        index.add_file("cart.py", "class Cart:\n    def __init__(self):\n        self.items = []\n\n    def add(self, item):\n        self.items.append(item)\n");
        index.add_file("box.ts", "export class Box {\n  size: number = 1;\n  open(): void {}\n}\n");

        assert_eq!(unknown_members(&index, "main.py", "Cart.add(c, 1)\nCart.checkout(c)\n").len(), 1);
        assert!(unknown_members(&index, "main.py", "Cart.checkout(c)").iter().any(|w| w.contains("checkout")));
        assert_eq!(unknown_members(&index, "main.ts", "Box.open(b); Box.close(b);").len(), 1);
        assert!(unknown_members(&index, "main.ts", "Box.open(b);").is_empty());
    }

    #[test]
    fn a_made_up_package_is_flagged_and_an_installed_one_is_not() {
        use crate::verify::api_exists::{check, warning, Verdict};

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("node_modules/realpkg");

        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("package.json"), "{\"name\": \"realpkg\", \"main\": \"index.js\"}").unwrap();
        std::fs::write(real.join("index.js"), "export function useThing() {}\n").unwrap();

        let fake = check(dir.path(), "src/app.js", "import { useThing } from 'realpkgg';\n");
        let fake_name = check(dir.path(), "src/app.js", "import { useThang } from 'realpkg';\n");
        let good = check(dir.path(), "src/app.js", "import { useThing } from 'realpkg';\n");

        assert!(fake.iter().any(|v| matches!(v, Verdict::ModuleMissing { module, .. } if module == "realpkgg")), "{fake:?}");
        assert!(fake_name.iter().any(|v| matches!(v, Verdict::NameMissing { name, .. } if name == "useThang")), "{fake_name:?}");
        assert!(warning(&fake).is_some());
        assert_eq!(good, vec![Verdict::Found], "{good:?}");
    }

    #[test]
    fn an_unrelated_text_costs_nothing() {
        let dir = project();

        assert_eq!(note(dir.path(), "src/main.rs", "fn main() { println!(\"hi\"); }"), None);
        assert_eq!(note(dir.path(), "notes.txt", "Shape::volume()"), None);
    }
}
