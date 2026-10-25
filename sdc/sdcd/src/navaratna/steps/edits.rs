//! The edit format of the step executor. A step is one model call that answers with plain text, not a tool loop, so the answer carries its edits
//! as headed code blocks - the code is never inside JSON (N3):
//!
//! ```text
//! FILE: src/shop.rs
//! SYMBOL: Cart::total          (optional; without it the block is a new file)
//! MODE: replace                (replace is the default with SYMBOL; insert_after; create is the default without)
//! (then the complete code in a fenced block)
//! ```
//!
//! [`parse`] reads the answer; [`apply`] turns the edits into whole new file texts through the symbol patcher (so elision, a rename or a second
//! symbol is refused) and answers the writes. Nothing here touches the disk.

use std::collections::BTreeMap;

use crate::navaratna::patcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Replace,
    InsertAfter,
    Create,
    /// A block with no `FILE:` line, given to the step's own file: its code goes at the end of that file (or makes it).
    Append,
    /// Import lines for the top of an existing file.
    Imports,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub path: String,
    pub symbol: Option<String>,
    pub mode: Mode,
    pub code: String,
}

fn header<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let trimmed = line.trim().trim_start_matches(['*', '#', '-', '`', ' ']);
    let (key, value) = trimmed.split_once(':')?;

    key.trim().eq_ignore_ascii_case(name).then(|| value.trim().trim_matches(['*', '`', '"', '\'', ' ']))
}

/// The name a model wrote after `SYMBOL:`: `def top_n(text, n):` and `fn total(&self)...` are the symbols `top_n` and `total`; `none` is no symbol.
fn clean_symbol(value: &str) -> Option<String> {
    let mut rest = value.trim().trim_matches(['*', '`', '"', '\'']).trim();

    while let Some((word, tail)) = rest.split_once(' ') {
        if ["pub", "async", "def", "fn", "function", "class", "struct", "impl", "static", "const"].contains(&word) {
            rest = tail.trim_start();
        } else {
            break;
        }
    }

    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | ':')).collect();
    let name = name.trim_end_matches([':', '.']).to_string();

    (!["", "none", "null", "nil", "empty"].contains(&name.to_ascii_lowercase().as_str()) && !value.trim_matches(['(', ')']).eq_ignore_ascii_case("n/a")).then_some(name)
}

/// An answer with headers but no code fence at all (a small model forgets it): the lines after a block's headers, up to the next `FILE:`, are its code.
fn fence_bare(answer: &str) -> String {
    /* a fence is a line that STARTS with three backticks; three backticks inside a line of code (`line.startswith("```")`) are not one */
    if answer.lines().any(|line| line.trim_start().starts_with("```")) || !answer.lines().any(|line| header(line, "FILE").is_some()) {
        return answer.to_string();
    }

    let mut out = String::new();
    let mut open = false;
    let mut in_headers = false;

    for line in answer.lines() {
        let is_file = header(line, "FILE").is_some();
        let is_header = is_file || header(line, "SYMBOL").is_some() || header(line, "MODE").is_some();

        if is_file && open {
            out.push_str("```\n");
            open = false;
        }

        /* a bare `CODE:` label between the headers and the code is not code */
        let is_label = in_headers && header(line, "CODE").is_some_and(str::is_empty);

        if is_label {
            continue;
        }

        if is_header && (is_file || in_headers) {
            in_headers = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_headers {
            if line.trim().is_empty() {
                continue;
            }

            out.push_str("```\n");
            open = true;
            in_headers = false;
        }

        if open {
            out.push_str(line);
            out.push('\n');
        }
    }

    if open {
        out.push_str("```\n");
    }

    out
}

/// The edits of a model's answer, in order.
pub fn parse(answer: &str) -> Result<Vec<Edit>, String> {
    parse_in(answer, None)
}

/// Like [`parse`]; the first block with no `FILE:` line belongs to `default_file` (the file the step is about) and is appended to it.
pub fn parse_in(answer: &str, default_file: Option<&str>) -> Result<Vec<Edit>, String> {
    let mut default_file = default_file;
    let fenced = fence_bare(answer);
    let answer = fenced.as_str();
    let mut edits = Vec::new();
    let (mut file, mut symbol, mut mode): (Option<String>, Option<String>, Option<Mode>) = (None, None, None);
    let mut lines = answer.lines().peekable();

    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("```") {
            let mut code = Vec::new();

            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }

                code.push(inner);
            }

            let headerless = file.is_none() && symbol.is_none() && mode.is_none();
            let Some(path) = file.take().or_else(|| headerless.then(|| default_file.take().map(str::to_string)).flatten()) else {
                return Err("a code block has no `FILE: path` line before it; every block needs FILE: (and SYMBOL: to change one declaration)".into());
            };
            let symbol_now = symbol.take();
            let mode_now = mode.take().unwrap_or(if headerless { Mode::Append } else if symbol_now.is_some() { Mode::Replace } else { Mode::Create });

            /* `insert_after` with no name to put it after is an addition */
            let mode_now = if mode_now == Mode::InsertAfter && symbol_now.is_none() { Mode::Append } else { mode_now };

            /* `MODE: replace` with no name is the whole file written again */
            let mode_now = if mode_now == Mode::Replace && symbol_now.is_none() { Mode::Create } else { mode_now };

            let mut text = code.join("\n");

            if !text.ends_with('\n') {
                text.push('\n');
            }

            edits.push(Edit { path, symbol: symbol_now, mode: mode_now, code: text });
            continue;
        }

        if let Some(value) = header(line, "FILE") {
            file = Some(value.to_string()).filter(|value| !value.is_empty());
        } else if let Some(value) = header(line, "SYMBOL") {
            symbol = clean_symbol(value);
        } else if let Some(value) = header(line, "MODE") {
            mode = match value.to_ascii_lowercase().replace('-', "_").as_str() {
                "replace" => Some(Mode::Replace),
                "insert_after" => Some(Mode::InsertAfter),
                "create" => Some(Mode::Create),
                "append" | "add" => Some(Mode::Append),
                "imports" | "import" | "prepend" => Some(Mode::Imports),
                other => return Err(format!("MODE `{other}` is not replace, insert_after, append, imports or create")),
            };
        }
    }

    if edits.is_empty() {
        let began: String = answer.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();

        return Err(format!("the answer holds no edit (it {}): write `FILE: path` (and `SYMBOL: name` to change one declaration) and then the code in a fenced block", if began.is_empty() { "was empty".to_string() } else { format!("began: `{began}`") }));
    }

    Ok(edits)
}

fn safe(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && !path.starts_with('\\') && !path.contains("..") && !path.contains(':') && !path.starts_with(".sdc/") && !path.starts_with(".git/")
}

/// Turns the edits into whole new texts: `(path, new text)` in the order the files were first touched. `read` gives a file's text now.
pub fn apply(edits: &[Edit], read: &dyn Fn(&str) -> Option<String>) -> Result<Vec<(String, String)>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut texts: BTreeMap<String, String> = BTreeMap::new();

    for edit in edits {
        if !safe(&edit.path) {
            return Err(format!("`{}` is not a path inside the project", edit.path));
        }

        if let Some(line) = patcher::elision(&edit.code) {
            return Err(format!("{}: the code leaves something out (`{line}`); write it whole", edit.path));
        }

        let current = texts.get(&edit.path).cloned().or_else(|| read(&edit.path));
        let new = match (edit.mode, &current) {
            (Mode::Create, Some(existing)) if !existing.trim().is_empty() && existing.lines().count() > 30 => {
                return Err(format!("{} already exists ({} lines): change one declaration with SYMBOL: instead of writing the whole file again", edit.path, existing.lines().count()));
            }
            (Mode::Create, _) => edit.code.clone(),
            (Mode::Replace, Some(text)) => match patcher::replace_symbol(&edit.path, text, edit.symbol.as_deref().unwrap_or_default(), &edit.code) {
                Ok(new) => new,
                /* A small model writes `SYMBOL: new_name` for a function that does not exist yet: that is an addition, so the code goes at the end of the file (a name that is already in the file is still refused). */
                Err(patcher::Refused::NotFound(_)) => patcher::append(&edit.path, text, &edit.code).map_err(|refused| format!("{}: {refused}", edit.path))?,
                Err(refused) => return Err(format!("{}: {refused}", edit.path)),
            },
            (Mode::Append, Some(text)) => patcher::append(&edit.path, text, &edit.code).map_err(|refused| match refused {
                patcher::Refused::AlreadyExists(_) => format!("{}: {refused}. With MODE: append write ONLY the new functions or methods - not the class or the file again", edit.path),
                other => format!("{}: {other}", edit.path),
            })?,
            (Mode::Append, None) => edit.code.clone(),
            (Mode::Imports, Some(text)) => patcher::add_imports(text, &edit.code),
            (Mode::Imports, None) => edit.code.clone(),
            (Mode::InsertAfter, Some(text)) => match patcher::insert_after(&edit.path, text, edit.symbol.as_deref().unwrap_or_default(), &edit.code) {
                Ok(new) => new,
                /* `insert_after` a name that is not in the file: the model means "add it", the end of the file is the place */
                Err(patcher::Refused::NotFound(_)) => patcher::append(&edit.path, text, &edit.code).map_err(|refused| format!("{}: {refused}", edit.path))?,
                Err(refused) => return Err(format!("{}: {refused}", edit.path)),
            },
            (Mode::Replace | Mode::InsertAfter, None) => return Err(format!("{} does not exist, so it has no symbol to change; create it with a block that has no SYMBOL", edit.path)),
        };

        if !texts.contains_key(&edit.path) {
            order.push(edit.path.clone());
        }

        texts.insert(edit.path.clone(), new);
    }

    link_python_imports(&order, &mut texts, read);

    Ok(order.into_iter().filter_map(|path| texts.remove(&path).map(|text| (path, text))).collect())
}

fn top_level_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix("async def ").or_else(|| line.strip_prefix("def ")).or_else(|| line.strip_prefix("class ")))
        .map(|rest| rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>())
        .filter(|name| !name.is_empty())
        .collect()
}

/// A new Python function or class that another file of the same answer uses gets its import there: a small model writes the test and forgets the `from pkg.mod import name` line.
fn link_python_imports(order: &[String], texts: &mut BTreeMap<String, String>, read: &dyn Fn(&str) -> Option<String>) {
    for source in order.iter().filter(|path| path.ends_with(".py") && !path.ends_with("__init__.py")) {
        let before = read(source).map(|text| top_level_names(&text)).unwrap_or_default();
        let added: Vec<String> = top_level_names(&texts[source]).into_iter().filter(|name| !before.contains(name) && !name.starts_with('_')).collect();

        if added.is_empty() {
            continue;
        }

        let module = source.trim_end_matches(".py").replace('/', ".");
        let module = module.strip_prefix("src.").unwrap_or(&module).to_string();

        for user in order.iter().filter(|path| path.ends_with(".py") && *path != source) {
            let text = &texts[user];
            let own = top_level_names(text);
            let missing: Vec<&String> = added
                .iter()
                .filter(|name| {
                    let word = regex::Regex::new(&format!(r"\b{}\b", regex::escape(name))).expect("an escaped word is a regex");

                    !own.contains(*name) && word.is_match(text) && !text.lines().any(|line| (line.starts_with("import ") || line.starts_with("from ")) && word.is_match(line))
                })
                .collect();

            if !missing.is_empty() {
                let line = format!("from {module} import {}\n", missing.iter().map(|name| name.as_str()).collect::<Vec<_>>().join(", "));
                let new = patcher::add_imports(text, &line);

                texts.insert(user.clone(), new);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOP: &str = "pub struct Cart {\n    pub items: Vec<u64>,\n}\n\nimpl Cart {\n    pub fn total(&self) -> u64 {\n        0\n    }\n}\n";

    fn reader(path: &str) -> Option<String> {
        (path == "src/shop.rs").then(|| SHOP.to_string())
    }

    #[test]
    fn headed_blocks_become_edits_in_order() {
        let answer = "I will change total and add a test file.\n\nFILE: src/shop.rs\nSYMBOL: Cart::total\n```rust\npub fn total(&self) -> u64 {\n    self.items.iter().sum()\n}\n```\n\n**FILE:** tests/shop.rs\n```rust\n#[test]\nfn t() {}\n```\n";
        let edits = parse(answer).unwrap();

        assert_eq!(edits.len(), 2);
        assert_eq!((edits[0].path.as_str(), edits[0].symbol.as_deref(), edits[0].mode), ("src/shop.rs", Some("Cart::total"), Mode::Replace));
        assert_eq!((edits[1].path.as_str(), edits[1].symbol.as_deref(), edits[1].mode), ("tests/shop.rs", None, Mode::Create));
        assert!(edits[0].code.contains("self.items.iter().sum()") && edits[0].code.ends_with('\n'));
    }

    #[test]
    fn an_answer_without_edits_or_with_a_block_without_a_file_is_an_error_that_teaches() {
        assert!(parse("I think it is fine as it is.").unwrap_err().contains("holds no edit"));
        assert!(parse("```rust\nfn a() {}\n```").unwrap_err().contains("FILE:"));
        assert_eq!(parse("FILE: a.rs\nMODE: replace\n```rust\nfn a() {}\n```").unwrap()[0].mode, Mode::Create);
        assert!(parse("FILE: a.rs\nMODE: erase\n```rust\nfn a() {}\n```").unwrap_err().contains("erase"));
    }

    #[test]
    fn a_symbol_replace_goes_through_the_patcher_and_leaves_the_rest_alone() {
        let edits = parse("FILE: src/shop.rs\nSYMBOL: Cart::total\n```rust\npub fn total(&self) -> u64 {\n    self.items.iter().sum()\n}\n```").unwrap();
        let writes = apply(&edits, &reader).unwrap();

        assert_eq!(writes.len(), 1);
        assert!(writes[0].1.contains("        self.items.iter().sum()") && writes[0].1.starts_with("pub struct Cart {\n    pub items: Vec<u64>,\n}\n"), "{}", writes[0].1);
    }

    #[test]
    fn an_answer_without_any_fence_is_read_from_its_headers_and_methods_go_into_the_last_class() {
        const PY: &str = "import unittest\n\n\nclass ShopTest(unittest.TestCase):\n    def test_a(self):\n        self.assertTrue(True)\n\n\nif __name__ == \"__main__\":\n    unittest.main()\n";
        let read = |path: &str| (path == "tests/test_shop.py").then(|| PY.to_string());
        let answer = "FILE: tests/test_shop.py\nMODE: append\nSYMBOL: (empty)\n\n    def test_b(self):\n        self.assertEqual(1, 1)\n\n    def test_c(self):\n        self.assertEqual(2, 2)\n\nFILE: tests/new.py\ndef t():\n    pass\n";
        let edits = parse(answer).unwrap();

        assert_eq!(edits.len(), 2);
        assert_eq!((edits[0].symbol.clone(), edits[0].mode), (None, Mode::Append));

        let writes = apply(&edits, &read).unwrap();

        assert_eq!(writes[0].1, "import unittest\n\n\nclass ShopTest(unittest.TestCase):\n    def test_a(self):\n        self.assertTrue(True)\n\n    def test_b(self):\n        self.assertEqual(1, 1)\n\n    def test_c(self):\n        self.assertEqual(2, 2)\n\n\nif __name__ == \"__main__\":\n    unittest.main()\n");
        assert_eq!(writes[1].0, "tests/new.py");
    }

    #[test]
    fn insert_after_a_name_that_is_not_there_adds_it_at_the_end() {
        let edits = parse("FILE: src/shop.rs\nSYMBOL: nothing_like_it\nMODE: insert_after\n```rust\npub fn extra() {}\n```").unwrap();

        assert!(apply(&edits, &reader).unwrap()[0].1.ends_with("pub fn extra() {}\n"));
    }

    #[test]
    fn a_new_python_function_used_by_a_test_in_the_same_answer_is_imported_there_and_replace_without_a_symbol_writes_the_file() {
        const SLUG: &str = "import re\n\n\ndef slugify(text):\n    return text\n";
        const TEST: &str = "import unittest\n\nfrom textkit.slug import slugify\n\n\nclass T(unittest.TestCase):\n    def test_a(self):\n        self.assertEqual(slugify(\"a\"), \"a\")\n";
        let read = |path: &str| match path {
            "textkit/slug.py" => Some(SLUG.to_string()),
            "tests/test_t.py" => Some(TEST.to_string()),
            _ => None,
        };
        let answer = "FILE: textkit/slug.py\nMODE: replace\n```python\nimport re\n\n\ndef slugify(text):\n    return text\n\n\ndef slugify_unique(text, taken):\n    return slugify(text)\n```\nFILE: tests/test_t.py\nMODE: append\n```python\n    def test_b(self):\n        self.assertEqual(slugify_unique(\"a\", set()), \"a\")\n```";
        let writes = apply(&parse(answer).unwrap(), &read).unwrap();

        assert!(writes[0].1.contains("def slugify_unique"));
        assert!(writes[1].1.contains("from textkit.slug import slugify\nfrom textkit.slug import slugify_unique\n"), "{}", writes[1].1);
        assert!(writes[1].1.contains("test_b"));
    }

    #[test]
    fn a_fenceless_answer_whose_code_holds_three_backticks_and_a_code_label_is_still_read() {
        let answer = "FILE: textkit/md.py\nMODE: append\nCODE:\ndef f(line):\n    return line.startswith(\"```\")\n";
        let edits = parse(answer).unwrap();

        assert_eq!(edits.len(), 1);
        assert!(edits[0].code.starts_with("def f(line):") && edits[0].code.contains("startswith"), "{}", edits[0].code);
    }

    #[test]
    fn a_symbol_written_as_a_signature_is_cut_down_to_its_name() {
        assert_eq!(clean_symbol("def top_n(text, n):").as_deref(), Some("top_n"));
        assert_eq!(clean_symbol("pub fn total(&self) -> u64").as_deref(), Some("total"));
        assert_eq!(clean_symbol("`Cart::total`").as_deref(), Some("Cart::total"));
        assert_eq!(clean_symbol("a").as_deref(), Some("a"));
        assert_eq!(clean_symbol("none"), None);
    }

    #[test]
    fn mode_imports_adds_the_missing_lines_after_the_existing_imports_and_insert_after_without_a_symbol_appends() {
        const PY: &str = "import unittest\n\n\nclass T:\n    pass\n";
        let read = |path: &str| (path == "a.py").then(|| PY.to_string());
        let edits = parse("FILE: a.py\nMODE: imports\n```python\nimport re\nimport unittest\n```").unwrap();

        assert_eq!(edits[0].mode, Mode::Imports);
        assert_eq!(apply(&edits, &read).unwrap()[0].1, "import unittest\nimport re\n\n\nclass T:\n    pass\n");

        let after = parse("FILE: src/shop.rs\nMODE: insert_after\n```rust\npub fn extra() {}\n```").unwrap();

        assert_eq!((after[0].symbol.clone(), after[0].mode), (None, Mode::Append));
    }

    #[test]
    fn mode_append_adds_to_the_file_and_a_symbol_of_none_is_no_symbol() {
        let edits = parse("FILE: src/shop.rs\nSYMBOL: None\nMODE: append\n```rust\npub fn make_cart() {}\n```").unwrap();

        assert_eq!((edits[0].symbol.clone(), edits[0].mode), (None, Mode::Append));
        assert!(apply(&edits, &reader).unwrap()[0].1.ends_with("pub fn make_cart() {}\n"));
        assert!(parse("it is fine").unwrap_err().contains("began: `it is fine`") && parse("  ").unwrap_err().contains("was empty"));
    }

    #[test]
    fn a_symbol_that_is_not_there_yet_is_an_addition_at_the_end_of_the_file() {
        let edits = parse("FILE: src/shop.rs\nSYMBOL: make_cart\n```rust\npub fn make_cart() -> Cart {\n    Cart { items: vec![] }\n}\n```").unwrap();
        let writes = apply(&edits, &reader).unwrap();

        assert!(writes[0].1.starts_with("pub struct Cart {") && writes[0].1.trim_end().ends_with("pub fn make_cart() -> Cart {\n    Cart { items: vec![] }\n}"), "{}", writes[0].1);
    }

    #[test]
    fn a_block_with_several_new_functions_is_appended_but_a_name_that_exists_is_still_refused() {
        let edits = parse("FILE: src/shop.rs\nSYMBOL: make_cart\n```rust\npub fn make_cart() {}\n\npub fn make_other() {}\n```").unwrap();
        let text = &apply(&edits, &reader).unwrap()[0].1;

        assert!(text.contains("pub fn make_cart() {}") && text.contains("pub fn make_other() {}"), "{text}");

        let clash = parse("FILE: src/shop.rs\nSYMBOL: make_cart\n```rust\npub fn make_cart() {}\n\npub fn total() {}\n```").unwrap();
        let again = apply(&clash, &|path| reader(path).map(|text| format!("{text}\nfn total() {{}}\n"))).unwrap_err();

        assert!(again.contains("already exists"), "{again}");
    }

    #[test]
    fn the_first_block_without_a_file_line_goes_to_the_steps_own_file_and_is_appended() {
        let edits = parse_in("Here it is:\n```rust\npub fn make_cart() {}\n```\n", Some("src/shop.rs")).unwrap();

        assert_eq!((edits[0].path.as_str(), edits[0].mode), ("src/shop.rs", Mode::Append));
        assert!(apply(&edits, &reader).unwrap()[0].1.starts_with("pub struct Cart {"), "the file is kept, the code comes after it");

        let two = parse_in("```rust\nfn a() {}\n```\n```rust\nfn b() {}\n```", Some("src/shop.rs")).unwrap_err();

        assert!(two.contains("FILE:"), "only the first headerless block is given a file");
        assert!(parse_in("```rust\nfn a() {}\n```", None).unwrap_err().contains("FILE:"));
    }

    #[test]
    fn two_edits_to_one_file_build_on_each_other() {
        let edits = parse("FILE: src/shop.rs\nSYMBOL: Cart::total\n```rust\npub fn total(&self) -> u64 {\n    1\n}\n```\nFILE: src/shop.rs\nSYMBOL: Cart::total\nMODE: insert_after\n```rust\npub fn count(&self) -> usize {\n    self.items.len()\n}\n```").unwrap();
        let writes = apply(&edits, &reader).unwrap();

        assert_eq!(writes.len(), 1);
        assert!(writes[0].1.contains("    1\n") && writes[0].1.contains("pub fn count(&self) -> usize"), "{}", writes[0].1);
    }

    #[test]
    fn elision_a_rename_a_missing_file_and_an_unsafe_path_are_refused_with_the_reason() {
        let refuse = |answer: &str| apply(&parse(answer).unwrap(), &reader).unwrap_err();

        assert!(refuse("FILE: src/shop.rs\nSYMBOL: Cart::total\n```rust\npub fn total(&self) -> u64 {\n    // ... existing code\n}\n```").contains("leaves something out"));
        assert!(refuse("FILE: src/shop.rs\nSYMBOL: Cart::total\n```rust\npub fn sum(&self) -> u64 { 1 }\n```").contains("exactly the one symbol"));
        assert!(refuse("FILE: src/none.rs\nSYMBOL: a\n```rust\nfn a() {}\n```").contains("does not exist"));
        assert!(refuse("FILE: ../outside.rs\n```rust\nfn a() {}\n```").contains("not a path inside"));
        assert!(refuse("FILE: .sdc/policy.toml\n```toml\nprivacy = \"cloud\"\n```").contains("not a path inside"));
    }

    #[test]
    fn a_new_file_is_written_whole_but_a_big_existing_file_is_not_rewritten() {
        let writes = apply(&parse("FILE: src/new.rs\n```rust\npub fn n() {}\n```").unwrap(), &reader).unwrap();

        assert_eq!(writes, vec![("src/new.rs".to_string(), "pub fn n() {}\n".to_string())]);

        let big = (0..40).map(|n| format!("fn f{n}() {{}}\n")).collect::<String>();
        let read_big = |_: &str| Some(big.clone());

        assert!(apply(&parse("FILE: src/big.rs\n```rust\nfn x() {}\n```").unwrap(), &read_big).unwrap_err().contains("already exists"));
    }
}
