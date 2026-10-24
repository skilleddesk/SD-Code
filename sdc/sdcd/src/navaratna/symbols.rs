//! **N4-1 - the symbol index and the skeleton generator** (switch `nr.symbol_index`, index only: no prompt changes).
//!
//! It adds nothing to the Code Graph's grammars (`graph::parse`); it reads the same declarations and adds what a surgical edit needs:
//! the **signature** of each symbol and a **skeleton** of a file (imports and signatures with line numbers, no bodies). A language
//! without a grammar answers `None` - the caller falls back to whole-file or search/replace edits.

use crate::graph::parse::{self, Def, Lang};

/// One symbol of a file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    pub kind: String,
    pub name: String,
    pub qualified: String,
    pub file: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    /// The declaration up to its body: `pub fn area(&self) -> f64`, `def area(self):`, `class Shape`.
    pub signature: String,
    /// Nesting depth: 0 for a top-level symbol, 1 for a method of a class or impl, and so on.
    pub depth: usize,
}

fn is_callable(kind: &str) -> bool {
    matches!(kind, "fn" | "method" | "def")
}

/// The text of a declaration up to where its body begins.
pub fn signature_of(lang: Lang, text: &str) -> String {
    let head = match lang {
        Lang::Python => {
            let mut end = text.len();
            let mut depth = 0i32;

            for (at, c) in text.char_indices() {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    ':' if depth == 0 => {
                        end = at + 1;
                        break;
                    }
                    _ => {}
                }
            }

            &text[..end]
        }
        _ => {
            let mut end = text.len();
            let mut depth = 0i32;

            for (at, c) in text.char_indices() {
                match c {
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth -= 1,
                    '{' | ';' if depth <= 0 => {
                        end = at;
                        break;
                    }
                    _ => {}
                }
            }

            &text[..end]
        }
    };

    head.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn contains(outer: &Def, inner: &Def) -> bool {
    outer.start_byte <= inner.start_byte && inner.end_byte <= outer.end_byte && (outer.start_byte, outer.end_byte) != (inner.start_byte, inner.end_byte)
}

/// Every symbol of a file, in source order. `None` when the file's language has no grammar (the caller falls back).
pub fn index_text(path: &str, text: &str) -> Option<Vec<Symbol>> {
    /* the switch is the index itself: the patcher, the skeleton pack, the existence gate and the registry all read symbols through here */
    if !crate::v3::on("nr.symbol_index") {
        return None;
    }

    let lang = Lang::of(path).filter(|lang| lang.has_symbols())?;
    let tree = parse::parse(lang, text)?;
    let defs = parse::definitions(lang, &tree, text);
    let mut out = Vec::with_capacity(defs.len());

    for def in &defs {
        /* a symbol declared inside a function body is private to it: it is not part of the file's surface */
        let inside_callable = defs.iter().any(|outer| is_callable(&outer.kind) && contains(outer, def));

        if inside_callable {
            continue;
        }

        let depth = defs.iter().filter(|outer| contains(outer, def)).count();
        let slice = text.get(def.start_byte..def.end_byte).unwrap_or_default();

        out.push(Symbol {
            kind: def.kind.clone(),
            name: def.name.clone(),
            qualified: def.qualified.clone(),
            file: path.to_string(),
            start_byte: def.start_byte,
            end_byte: def.end_byte,
            start_line: def.start_line,
            end_line: def.end_line,
            signature: signature_of(lang, slice),
            depth,
        });
    }

    Some(out)
}

/// The symbols of a file whose qualified name is `wanted`, or - when none is - whose plain name is.
pub fn find<'a>(symbols: &'a [Symbol], wanted: &str) -> Vec<&'a Symbol> {
    let exact: Vec<&Symbol> = symbols.iter().filter(|symbol| symbol.qualified == wanted && symbol.kind != "impl").collect();

    if !exact.is_empty() {
        return exact;
    }

    symbols.iter().filter(|symbol| symbol.name == wanted && symbol.kind != "impl").collect()
}

fn is_import_line(lang: Lang, line: &str) -> bool {
    let line = line.trim_start();

    match lang {
        Lang::Rust => line.starts_with("use ") || line.starts_with("pub use ") || (line.starts_with("mod ") && line.ends_with(';')) || line.starts_with("extern crate"),
        Lang::Python => line.starts_with("import ") || line.starts_with("from "),
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => line.starts_with("import ") || line.contains("require("),
        Lang::Go => line.starts_with("import ") || line.starts_with("package "),
        Lang::Php => line.starts_with("use ") || line.starts_with("namespace "),
        _ => false,
    }
}

/// The skeleton of a file: its imports, then every symbol's signature behind its line number, indented by nesting. No bodies.
pub fn skeleton(path: &str, text: &str) -> Option<String> {
    let lang = Lang::of(path).filter(|lang| lang.has_symbols())?;
    let symbols = index_text(path, text)?;
    let mut out = String::new();
    let imports: Vec<&str> = text.lines().filter(|line| !line.starts_with(char::is_whitespace) && is_import_line(lang, line)).collect();

    for line in imports {
        out.push_str(line.trim_end());
        out.push('\n');
    }

    if !out.is_empty() {
        out.push('\n');
    }

    for symbol in &symbols {
        out.push_str(&format!("{}L{}: {}\n", "  ".repeat(symbol.depth), symbol.start_line, symbol.signature));
    }

    Some(out)
}

/// A language the index cannot read: the honest label the caller shows instead of a skeleton.
pub fn no_grammar(path: &str) -> bool {
    !Lang::of(path).is_some_and(|lang| lang.has_symbols())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: &str = "use std::fmt;\n\npub struct Shape {\n    side: f64,\n}\n\nimpl Shape {\n    /// the area\n    pub fn area(&self) -> f64 {\n        fn square(x: f64) -> f64 { x * x }\n        square(self.side)\n    }\n\n    fn grow(&mut self, by: f64) {\n        self.side += by;\n    }\n}\n\npub fn make(side: f64) -> Shape {\n    Shape { side }\n}\n";
    const PYTHON: &str = "import os\nfrom math import pi\n\nclass Circle:\n    def __init__(self, r):\n        self.r = r\n\n    def area(self):\n        def helper():\n            return 1\n        return pi * self.r ** 2\n\ndef make(r):\n    return Circle(r)\n";
    const TS: &str = "import { a } from './a';\n\nexport class Box {\n  size(): number { return 1; }\n}\n\nexport function open(box: Box): number {\n  return box.size();\n}\n\nconst arrow = (x: number) => x + 1;\n";

    #[test]
    fn every_top_level_function_and_method_is_found_with_its_byte_range() {
        for (path, text, expected) in [
            ("a.rs", RUST, vec!["area", "grow", "make"]),
            ("a.py", PYTHON, vec!["__init__", "area", "make"]),
            ("a.ts", TS, vec!["size", "open", "arrow"]),
        ] {
            let symbols = index_text(path, text).expect("a grammar exists");

            for name in &expected {
                let symbol = symbols.iter().find(|symbol| &symbol.name == name).unwrap_or_else(|| panic!("{path}: {name} not found in {symbols:?}"));
                let slice = &text[symbol.start_byte..symbol.end_byte];

                assert!(slice.contains(name), "{path}: the byte range of {name} is wrong: {slice:?}");
                assert!(!slice.starts_with(char::is_whitespace), "{path}: {name} starts on whitespace");
            }

            /* a function declared inside a body is not part of the file's surface */
            assert!(!symbols.iter().any(|symbol| symbol.name == "square" || symbol.name == "helper"), "{path}: {symbols:?}");
        }
    }

    #[test]
    fn a_signature_stops_where_the_body_begins() {
        let rust = index_text("a.rs", RUST).unwrap();
        let area = find(&rust, "Shape::area");

        assert_eq!(area.len(), 1);
        assert_eq!(area[0].signature, "pub fn area(&self) -> f64");
        assert_eq!(area[0].depth, 1);

        let python = index_text("a.py", PYTHON).unwrap();

        assert_eq!(find(&python, "Circle.area")[0].signature, "def area(self):");
        assert_eq!(find(&python, "make")[0].signature, "def make(r):");
    }

    #[test]
    fn the_skeleton_has_imports_and_signatures_with_line_numbers_and_no_bodies() {
        let skeleton = skeleton("a.rs", RUST).unwrap();

        assert!(skeleton.starts_with("use std::fmt;"), "{skeleton}");
        assert!(skeleton.contains("L9: pub fn area(&self) -> f64"), "{skeleton}");
        assert!(skeleton.contains("  L14: fn grow(&mut self, by: f64)"), "{skeleton}");
        assert!(!skeleton.contains("self.side += by"), "{skeleton}");
        assert!(!skeleton.contains("square"), "{skeleton}");
        assert_eq!(skeleton, super::skeleton("a.rs", RUST).unwrap(), "stable");
    }

    #[test]
    fn with_the_switch_off_there_is_no_index_and_everything_that_needs_it_says_so() {
        crate::v3::ablating(&["nr.symbol_index"], || {
            assert!(index_text("a.rs", RUST).is_none() && skeleton("a.rs", RUST).is_none());
            assert!(super::super::patcher::replace_symbol("a.rs", RUST, "make", "pub fn make(side: f64) -> Shape { Shape { side } }").is_err());
        });
    }

    #[test]
    fn a_language_without_a_grammar_is_labelled_not_guessed() {
        assert!(index_text("main.zig", "pub fn main() void {}").is_none());
        assert!(skeleton("notes.txt", "hello").is_none());
        assert!(no_grammar("main.zig") && no_grammar("data.json") && !no_grammar("a.rs"));
    }
}
