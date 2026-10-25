//! **N4-2 - the Symbol Patcher and the edit-mode chooser** (switch `nr.symbol_patch`).
//!
//! The model names a symbol and returns the new definition as a plain code block (never inside JSON); SDC swaps that symbol's byte
//! range and re-indents. Anything that leaves the symbol alone is refused: elision markers (`// ...`, "rest unchanged"), a renamed
//! symbol, a second symbol. After the swap every other symbol of the file must be byte-identical.

use super::symbols::{self, Symbol};
use crate::graph::parse::{self, Lang};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The change touches most of one symbol.
    WholeSymbol,
    /// A few lines inside a body, or an insert.
    SearchReplace,
    /// A new file or a very small one.
    WholeFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    Elision(String),
    NoGrammar,
    NotFound(String),
    Ambiguous(String),
    /// The new text defines a different symbol (a rename) or more than one.
    Renamed { wanted: String, found: Vec<String> },
    Empty,
    /// The new file does not parse, or another symbol would change.
    Breaks(String),
    AlreadyExists(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Elision(line) => write!(f, "refused: the definition leaves code out (`{line}`); write the whole symbol"),
            Refused::NoGrammar => write!(f, "refused: this language has no grammar here, use search/replace"),
            Refused::NotFound(name) => write!(f, "refused: no symbol `{name}` in this file"),
            Refused::Ambiguous(name) => write!(f, "refused: `{name}` names more than one symbol; use the qualified name (Type::name or Type.name)"),
            Refused::Renamed { wanted, found } => write!(f, "refused: the code block must define exactly the one symbol `{wanted}`, it defines {found:?}"),
            Refused::Empty => write!(f, "refused: the code block is empty"),
            Refused::Breaks(why) => write!(f, "refused: {why}"),
            Refused::AlreadyExists(name) => write!(f, "refused: `{name}` already exists; replace it instead of inserting a copy"),
        }
    }
}

/// Words by which a model says it did not write the code.
const ELISION_PHRASES: &[&str] = &["rest unchanged", "rest of the", "existing code", "remains the same", "remain the same", "unchanged", "same as before", "code here", "omitted", "implementation here"];

/// The first line of `code` that is an elision marker, if any.
pub fn elision(code: &str) -> Option<String> {
    for line in code.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        let comment = ["//", "#", "/*", "*", "--", "<!--"].iter().any(|start| trimmed.starts_with(start));
        let only_dots = !trimmed.is_empty() && trimmed.chars().all(|c| c == '.' || c == '…' || c.is_whitespace());
        /* a comment that BEGINS or ENDS in dots stands for code left out; dots inside a sentence (`join(...)`) are just text */
        let body = trimmed.trim_start_matches(['/', '*', '#', '-', '!', '<', ' ']).trim_end_matches(['*', '/', '>', '-', ' ']);
        let dotted_comment = comment && ["...", "…"].iter().any(|dots| body.starts_with(dots) || body.ends_with(dots));
        let phrase = comment && ELISION_PHRASES.iter().any(|phrase| lower.contains(phrase));

        if only_dots || dotted_comment || phrase {
            return Some(trimmed.to_string());
        }
    }

    None
}

/// The edit mode for a change, by the plan's rule: a new or very small file is written whole; a change that touches most of one symbol
/// replaces the symbol; a few lines go through search/replace.
pub fn choose(file_lines: usize, new_file: bool, symbol_lines: usize, changed_lines: usize) -> Mode {
    if new_file || file_lines <= 30 {
        return Mode::WholeFile;
    }

    if symbol_lines == 0 {
        return Mode::SearchReplace;
    }

    /* "most of a symbol": more than half of its lines, or a symbol so short that a search/replace would copy it anyway */
    if changed_lines * 2 > symbol_lines || symbol_lines <= 6 {
        Mode::WholeSymbol
    } else {
        Mode::SearchReplace
    }
}

fn indent_of(text: &str, at: usize) -> String {
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);

    text[line_start..at].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// The code with its common leading whitespace removed.
fn dedent(code: &str) -> String {
    let common = code.lines().filter(|line| !line.trim().is_empty()).map(|line| line.len() - line.trim_start().len()).min().unwrap_or(0);

    code.lines().map(|line| if line.len() >= common { &line[common..] } else { line.trim_start() }).collect::<Vec<_>>().join("\n")
}

/// `code` indented by `indent` on every line after the first (the first is placed where the old symbol began).
fn reindent(code: &str, indent: &str, first_too: bool) -> String {
    let code = dedent(code.trim_matches('\n'));

    code.lines()
        .enumerate()
        .map(|(n, line)| if line.is_empty() || (n == 0 && !first_too) { line.to_string() } else { format!("{indent}{line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The part of a fenced block: the first ``` block of a model's answer, or the whole text when it has none.
pub fn code_block(answer: &str) -> String {
    let Some(open) = answer.find("```") else {
        return answer.trim_matches('\n').to_string();
    };
    let after = &answer[open + 3..];
    let body_start = after.find('\n').map_or(0, |n| n + 1);
    let body = &after[body_start..];
    let end = body.find("```").unwrap_or(body.len());

    body[..end].trim_end().trim_start_matches('\n').to_string()
}

/// Where an old symbol's extended range starts: the attributes, decorators, doc comments and `export` keyword that belong to it.
fn extended_start(text: &str, symbol: &Symbol) -> usize {
    let mut start = text[..symbol.start_byte].rfind('\n').map_or(0, |n| n + 1);

    loop {
        if start == 0 {
            break;
        }

        let before = &text[..start - 1];
        let line_start = before.rfind('\n').map_or(0, |n| n + 1);
        let line = text[line_start..start - 1].trim();
        let attached = line.starts_with("#[") || line.starts_with("///") || line.starts_with('@') || line.starts_with("/**") || line.starts_with('*') || line.starts_with("*/") || line.starts_with("//");

        if line.is_empty() || !attached {
            break;
        }

        start = line_start;
    }

    start
}

/// The outcome of parsing a model's definition: the one symbol it defines and where that begins.
fn defined(lang: Lang, code: &str) -> Option<Vec<(String, usize)>> {
    let tree = parse::parse(lang, code)?;
    let defs = parse::definitions(lang, &tree, code);
    /* top-level in the snippet: not inside another callable of the snippet; the impl line of a Rust snippet is not a symbol of its own */
    let tops: Vec<&parse::Def> = defs.iter().filter(|d| d.kind != "impl" && !defs.iter().any(|o| o != *d && o.start_byte <= d.start_byte && d.end_byte <= o.end_byte && matches!(o.kind.as_str(), "fn" | "method" | "def"))).collect();

    Some(tops.iter().map(|d| (d.name.clone(), d.start_byte)).collect())
}

fn others_unchanged(lang: Lang, old: &str, new: &str, target: &Symbol, expect_added: bool) -> Result<(), String> {
    let describe = |text: &str| -> Option<Vec<parse::Def>> {
        let tree = parse::parse(lang, text)?;

        Some(parse::definitions(lang, &tree, text))
    };
    let before = describe(old).ok_or("the file could not be parsed")?;
    let after = describe(new).ok_or("the new file could not be parsed")?;
    let in_target = |def: &parse::Def| target.start_byte <= def.start_byte && def.end_byte <= target.end_byte;
    /* the blocks that hold the target (an impl, a class) change size, so they are compared by their own other children only */
    let holds_target = |def: &parse::Def| def.start_byte <= target.start_byte && target.end_byte <= def.end_byte;
    let mut kept: Vec<(String, String)> = before.iter().filter(|d| !in_target(d) && !holds_target(d)).map(|d| (d.qualified.clone(), d.hash.clone())).collect();
    let mut now: Vec<(String, String)> = after.iter().filter(|d| !(d.qualified == target.qualified || d.qualified.starts_with(&format!("{}.", target.qualified)) || d.qualified.starts_with(&format!("{}::", target.qualified)))).filter(|d| !before.iter().any(|b| holds_target(b) && b.qualified == d.qualified && b.kind == d.kind && d.kind == "impl")).map(|d| (d.qualified.clone(), d.hash.clone())).collect();

    kept.sort();
    now.sort();

    if expect_added {
        now.retain(|item| kept.contains(item) || false);
    }

    let missing: Vec<&String> = kept.iter().filter(|item| !now.contains(item)).map(|item| &item.0).collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("the patch would have changed other symbols: {missing:?}"))
    }
}

fn find_one<'a>(symbols: &'a [Symbol], wanted: &str) -> Result<&'a Symbol, Refused> {
    let found = symbols::find(symbols, wanted);

    match found.len() {
        0 => Err(Refused::NotFound(wanted.to_string())),
        1 => Ok(found[0]),
        _ => Err(Refused::Ambiguous(wanted.to_string())),
    }
}

/// Replaces the symbol `wanted` of `text` with the definition in `answer` (a model's answer: a fenced block or bare code). Answers the
/// whole new file text.
pub fn replace_symbol(path: &str, text: &str, wanted: &str, answer: &str) -> Result<String, Refused> {
    let code = code_block(answer);

    if code.trim().is_empty() {
        return Err(Refused::Empty);
    }

    if let Some(line) = elision(&code) {
        return Err(Refused::Elision(line));
    }

    let lang = Lang::of(path).filter(|lang| lang.has_symbols()).ok_or(Refused::NoGrammar)?;
    let symbols = symbols::index_text(path, text).ok_or(Refused::NoGrammar)?;
    let target = find_one(&symbols, wanted)?;
    let defines = defined(lang, &code).ok_or(Refused::NoGrammar)?;

    if defines.len() != 1 || defines[0].0 != target.name {
        return Err(Refused::Renamed { wanted: target.name.clone(), found: defines.into_iter().map(|(name, _)| name).collect() });
    }

    /* keep the old attributes and doc comments unless the new text brings its own */
    let prefix = code[..defines[0].1].trim();
    let (start, indent, first_too) = if prefix.is_empty() {
        (target.start_byte, indent_of(text, target.start_byte), false)
    } else {
        let start = extended_start(text, target);
        let line_start = text[..target.start_byte].rfind('\n').map_or(0, |n| n + 1);
        /* `export function f` on one line: the prefix before the symbol is part of the replaced range too */

        (start.min(line_start), indent_of(text, target.start_byte), true)
    };
    let replacement = reindent(&code, &indent, first_too);
    let mut new = String::with_capacity(text.len() + replacement.len());

    new.push_str(&text[..start]);
    new.push_str(&replacement);
    new.push_str(&text[target.end_byte..]);

    verify(lang, text, &new, target, false)?;
    Ok(new)
}

/// Inserts the new definition in `answer` after the symbol `after`. The new symbol must be new to the file.
pub fn insert_after(path: &str, text: &str, after: &str, answer: &str) -> Result<String, Refused> {
    let symbols = symbols::index_text(path, text).ok_or(Refused::NoGrammar)?;
    let anchor = find_one(&symbols, after)?;

    insert_at(path, text, &symbols, anchor, answer, false)
}

/// Puts import lines into a file: after its last import line (one that is already there is not repeated), or at the top when it has none.
pub fn add_imports(text: &str, code: &str) -> String {
    let is_import = |line: &str| {
        let t = line.trim_start();

        !line.starts_with([' ', '\t']) && (t.starts_with("import ") || t.starts_with("from ") || t.starts_with("use ") || t.starts_with("pub use ") || t.starts_with("#include") || t.contains("require("))
    };
    let lines: Vec<&str> = text.lines().collect();
    let fresh: Vec<&str> = code.lines().filter(|line| !line.trim().is_empty() && !lines.iter().any(|old| old.trim() == line.trim())).collect();

    if fresh.is_empty() {
        return text.to_string();
    }

    /* only the leading part of the file counts, up to its first declaration */
    let first_decl = lines.iter().position(|line| ["def ", "class ", "fn ", "pub fn ", "struct ", "pub struct ", "impl ", "function ", "export ", "async def "].iter().any(|start| line.starts_with(start))).unwrap_or(lines.len());
    let after = lines[..first_decl].iter().rposition(|line| is_import(line)).map_or(0, |n| n + 1);
    let mut out: Vec<&str> = lines[..after].to_vec();

    out.extend(fresh);

    if after == 0 && lines.first().is_some_and(|line| !line.trim().is_empty()) {
        out.push("");
    }

    out.extend(&lines[after..]);

    let mut joined = out.join("\n");

    if text.ends_with('\n') {
        joined.push('\n');
    }

    joined
}

/// Adds the new definition(s) in `answer` at the end of the file: what a model means by `SYMBOL: name` for a name that is not there yet. Code that is
/// indented (methods, as a model writes the tests of a `TestCase`) goes after the last member of the last class instead.
pub fn append(path: &str, text: &str, answer: &str) -> Result<String, Refused> {
    let symbols = symbols::index_text(path, text).ok_or(Refused::NoGrammar)?;
    let indented = code_block(answer).lines().find(|line| !line.trim().is_empty()).is_some_and(|line| line.starts_with([' ', '\t']));
    let inside = |outer: &Symbol, inner: &Symbol| outer.start_byte <= inner.start_byte && inner.end_byte <= outer.end_byte;
    let member = indented.then(|| symbols.iter().filter(|outer| outer.depth == 0).filter_map(|outer| symbols.iter().filter(|inner| inner.depth == 1 && inside(outer, inner)).max_by_key(|inner| inner.end_byte)).max_by_key(|inner| inner.end_byte)).flatten();
    let last = match member {
        Some(member) => member,
        None => symbols.iter().filter(|symbol| symbol.depth == 0).max_by_key(|symbol| symbol.end_byte).ok_or(Refused::NoGrammar)?,
    };

    insert_at(path, text, &symbols, last, answer, true)
}

/// `several`: the block may bring more than one new symbol (a model's answer for `append` often carries a function and its tests).
fn insert_at(path: &str, text: &str, symbols: &[Symbol], anchor: &Symbol, answer: &str, several: bool) -> Result<String, Refused> {
    let code = code_block(answer);

    if code.trim().is_empty() {
        return Err(Refused::Empty);
    }

    if let Some(line) = elision(&code) {
        return Err(Refused::Elision(line));
    }

    let lang = Lang::of(path).filter(|lang| lang.has_symbols()).ok_or(Refused::NoGrammar)?;
    let defines = defined(lang, &dedent(&code)).ok_or(Refused::NoGrammar)?;

    if defines.is_empty() || (defines.len() != 1 && !several) {
        return Err(Refused::Renamed { wanted: "one new symbol".into(), found: defines.into_iter().map(|(name, _)| name).collect() });
    }

    if let Some((name, _)) = defines.iter().find(|(name, _)| symbols.iter().any(|symbol| &symbol.name == name && symbol.depth == anchor.depth)) {
        return Err(Refused::AlreadyExists(name.clone()));
    }

    let indent = indent_of(text, anchor.start_byte);
    let block = reindent(&code, &indent, true);
    let mut new = String::with_capacity(text.len() + block.len() + 2);

    new.push_str(&text[..anchor.end_byte]);
    new.push_str("\n\n");
    new.push_str(&block);
    new.push_str(&text[anchor.end_byte..]);

    /* the anchor stays as it was: only an addition is allowed */
    verify(lang, text, &new, anchor, true)?;
    Ok(new)
}

fn verify(lang: Lang, old: &str, new: &str, target: &Symbol, inserted: bool) -> Result<(), Refused> {
    let tree = parse::parse(lang, new).ok_or(Refused::NoGrammar)?;
    let errors = parse::syntax_errors(&tree, new, 1);

    if let Some(error) = errors.first() {
        let old_broken = parse::parse(lang, old).is_some_and(|tree| tree.root_node().has_error());

        if !old_broken {
            return Err(Refused::Breaks(format!("the patched file has a syntax error at line {} (near `{}`)", error.line, error.near)));
        }
    }

    if inserted {
        let before = parse::definitions(lang, &parse::parse(lang, old).ok_or(Refused::NoGrammar)?, old);
        let after = parse::definitions(lang, &tree, new);
        let missing: Vec<String> = before.iter().filter(|b| b.kind != "impl" && b.kind != "mod" && !(b.start_byte <= target.start_byte && target.end_byte <= b.end_byte && b.qualified != target.qualified) && !after.iter().any(|a| a.qualified == b.qualified && a.hash == b.hash)).map(|b| b.qualified.clone()).collect();

        if !missing.is_empty() {
            return Err(Refused::Breaks(format!("the patch would have changed other symbols: {missing:?}")));
        }

        return Ok(());
    }

    others_unchanged(lang, old, new, target, false).map_err(Refused::Breaks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: &str = "use std::fmt;\n\npub struct Shape {\n    side: f64,\n}\n\nimpl Shape {\n    /// the area\n    pub fn area(&self) -> f64 {\n        self.side * self.side\n    }\n\n    fn grow(&mut self, by: f64) {\n        self.side += by;\n    }\n}\n\npub fn make(side: f64) -> Shape {\n    Shape { side }\n}\n";

    #[test]
    fn a_method_is_replaced_in_place_and_re_indented_and_the_rest_is_byte_identical() {
        let new = replace_symbol("a.rs", RUST, "Shape::area", "```rust\npub fn area(&self) -> f64 {\n    let s = self.side;\n    s * s\n}\n```").unwrap();

        assert!(new.contains("    pub fn area(&self) -> f64 {\n        let s = self.side;\n        s * s\n    }\n"), "{new}");
        assert!(new.contains("    /// the area\n"), "the old doc comment stays: {new}");
        assert!(new.contains("fn grow(&mut self, by: f64) {\n        self.side += by;\n    }"));
        assert!(new.ends_with("pub fn make(side: f64) -> Shape {\n    Shape { side }\n}\n"));
        assert!(new.starts_with("use std::fmt;\n\npub struct Shape {\n    side: f64,\n}\n"));
    }

    #[test]
    fn elision_markers_are_always_rejected() {
        for marker in ["// ... existing code ...", "# rest unchanged", "// rest of the function stays the same", "/* existing code */", "..."] {
            let answer = format!("```rust\npub fn area(&self) -> f64 {{\n    {marker}\n}}\n```");

            assert!(matches!(replace_symbol("a.rs", RUST, "Shape::area", &answer), Err(Refused::Elision(_))), "{marker}");
        }

        assert!(matches!(replace_symbol("a.py", "def f():\n    return 1\n", "f", "def f():\n    # rest unchanged\n    return 2\n"), Err(Refused::Elision(_))));
        assert_eq!(elision("let x = 1;\nlet y = x + 1;"), None);
        assert_eq!(elision("# \"abc\\n\" -> \"\\n\".join(...) -> \"abc\\n\" (correct)\nx = 1\n"), None, "dots inside a comment sentence are text");
        assert!(elision("    # ... more code ...\n").is_some());
    }

    #[test]
    fn a_rename_or_a_second_symbol_is_rejected() {
        let renamed = replace_symbol("a.rs", RUST, "Shape::area", "pub fn surface(&self) -> f64 { 1.0 }");
        let two = replace_symbol("a.rs", RUST, "make", "pub fn make(side: f64) -> Shape { Shape { side } }\npub fn extra() {}");

        assert!(matches!(renamed, Err(Refused::Renamed { .. })), "{renamed:?}");
        assert!(matches!(two, Err(Refused::Renamed { .. })), "{two:?}");
        assert!(matches!(replace_symbol("a.rs", RUST, "missing", "fn missing() {}"), Err(Refused::NotFound(_))));
    }

    #[test]
    fn a_plain_name_that_two_symbols_share_must_be_qualified() {
        let two = "struct A;\nstruct B;\nimpl A { fn run(&self) {} }\nimpl B { fn run(&self) {} }\n";

        assert!(matches!(replace_symbol("a.rs", two, "run", "fn run(&self) { }"), Err(Refused::Ambiguous(_))));
        assert!(replace_symbol("a.rs", two, "B::run", "fn run(&self) { let _ = 1; }").is_ok());
    }

    #[test]
    fn a_replacement_that_breaks_the_syntax_is_rejected() {
        let broken = replace_symbol("a.rs", RUST, "make", "pub fn make(side: f64) -> Shape {\n    Shape { side \n}");

        assert!(broken.is_err(), "{broken:?}");
    }

    #[test]
    fn python_is_indented_to_the_method_and_decorators_survive() {
        let source = "class C:\n    @property\n    def size(self):\n        return 1\n\n    def other(self):\n        return 2\n";
        let new = replace_symbol("c.py", source, "C.size", "def size(self):\n    value = 3\n    return value\n").unwrap();

        assert_eq!(new, "class C:\n    @property\n    def size(self):\n        value = 3\n        return value\n\n    def other(self):\n        return 2\n");
    }

    #[test]
    fn a_typescript_export_is_replaced_with_its_keyword() {
        let source = "export function open(a: number): number {\n  return a;\n}\n\nexport function close(): void {}\n";
        let new = replace_symbol("a.ts", source, "open", "export function open(a: number): number {\n  return a + 1;\n}").unwrap();

        assert_eq!(new, "export function open(a: number): number {\n  return a + 1;\n}\n\nexport function close(): void {}\n");
    }

    #[test]
    fn insert_after_adds_a_new_symbol_and_refuses_a_copy() {
        let new = insert_after("a.rs", RUST, "make", "pub fn unit() -> Shape {\n    make(1.0)\n}").unwrap();

        assert!(new.contains("pub fn make(side: f64) -> Shape {\n    Shape { side }\n}\n\npub fn unit() -> Shape {\n    make(1.0)\n}\n"), "{new}");
        assert!(matches!(insert_after("a.rs", RUST, "make", "pub fn make() {}"), Err(Refused::AlreadyExists(_))));
    }

    #[test]
    fn imports_go_after_the_last_import_once_and_to_the_top_of_a_file_without_any() {
        let py = "import unittest\n\nfrom textkit.slug import slugify\n\n\nclass T:\n    pass\n";

        assert_eq!(
            add_imports(py, "from textkit.stats import top_words\nimport unittest\n"),
            "import unittest\n\nfrom textkit.slug import slugify\nfrom textkit.stats import top_words\n\n\nclass T:\n    pass\n"
        );
        assert_eq!(add_imports("def f():\n    pass\n", "import re"), "import re\n\ndef f():\n    pass\n");
        assert_eq!(add_imports(py, "import unittest"), py, "nothing new, nothing changed");
    }

    #[test]
    fn the_chooser_follows_the_plan() {
        assert_eq!(choose(500, true, 0, 0), Mode::WholeFile);
        assert_eq!(choose(20, false, 10, 10), Mode::WholeFile);
        assert_eq!(choose(500, false, 40, 30), Mode::WholeSymbol);
        assert_eq!(choose(500, false, 40, 3), Mode::SearchReplace);
        assert_eq!(choose(500, false, 0, 2), Mode::SearchReplace);
    }

    #[test]
    fn a_fenced_block_is_taken_out_of_the_answer() {
        assert_eq!(code_block("Here:\n```rust\nfn a() {}\n```\nDone."), "fn a() {}");
        assert_eq!(code_block("fn a() {}"), "fn a() {}");
    }

    #[test]
    fn a_name_that_is_not_in_the_file_yet_is_added_at_the_end_when_the_code_defines_exactly_it() {
        let new = append("a.rs", RUST, "pub fn unit() -> Shape {\n    make(1.0)\n}").unwrap();

        assert!(new.ends_with("pub fn make(side: f64) -> Shape {\n    Shape { side }\n}\n\npub fn unit() -> Shape {\n    make(1.0)\n}\n"), "{new}");
        assert!(matches!(append("a.rs", RUST, "pub fn make() {}"), Err(Refused::AlreadyExists(_))));
    }
}
