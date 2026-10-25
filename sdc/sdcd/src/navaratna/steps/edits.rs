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

/// The edits of a model's answer, in order.
pub fn parse(answer: &str) -> Result<Vec<Edit>, String> {
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

            let Some(path) = file.take() else {
                return Err("a code block has no `FILE: path` line before it; every block needs FILE: (and SYMBOL: to change one declaration)".into());
            };
            let symbol_now = symbol.take();
            let mode_now = mode.take().unwrap_or(if symbol_now.is_some() { Mode::Replace } else { Mode::Create });

            if mode_now != Mode::Create && symbol_now.is_none() {
                return Err(format!("{path}: MODE replace/insert_after needs a SYMBOL: line"));
            }

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
            symbol = Some(value.to_string()).filter(|value| !value.is_empty());
        } else if let Some(value) = header(line, "MODE") {
            mode = match value.to_ascii_lowercase().replace('-', "_").as_str() {
                "replace" => Some(Mode::Replace),
                "insert_after" => Some(Mode::InsertAfter),
                "create" => Some(Mode::Create),
                other => return Err(format!("MODE `{other}` is not replace, insert_after or create")),
            };
        }
    }

    if edits.is_empty() {
        return Err("the answer holds no edit: write `FILE: path` (and `SYMBOL: name` to change one declaration) and then the code in a fenced block".into());
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
            (Mode::Replace, Some(text)) => patcher::replace_symbol(&edit.path, text, edit.symbol.as_deref().unwrap_or_default(), &edit.code).map_err(|refused| format!("{}: {refused}", edit.path))?,
            (Mode::InsertAfter, Some(text)) => patcher::insert_after(&edit.path, text, edit.symbol.as_deref().unwrap_or_default(), &edit.code).map_err(|refused| format!("{}: {refused}", edit.path))?,
            (Mode::Replace | Mode::InsertAfter, None) => return Err(format!("{} does not exist, so it has no symbol to change; create it with a block that has no SYMBOL", edit.path)),
        };

        if !texts.contains_key(&edit.path) {
            order.push(edit.path.clone());
        }

        texts.insert(edit.path.clone(), new);
    }

    Ok(order.into_iter().filter_map(|path| texts.remove(&path).map(|text| (path, text))).collect())
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
        assert!(parse("FILE: a.rs\nMODE: replace\n```rust\nfn a() {}\n```").unwrap_err().contains("SYMBOL"));
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
