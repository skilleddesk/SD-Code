//! **Code intelligence** (v3 W4): where the project's declarations are, and a map of them small enough to travel
//! with a request.
//!
//! `symbols` reads a file's declarations - name, kind, line - in the twelve language families SDC's users write,
//! and `repo_map` gives the most useful files' declarations, ranked: the files this turn has changed first, then
//! the files whose declarations the request names, then the files with the most declarations.
//!
//! The extractor is line-pattern based (the same shapes v1's reading already recognised). Tree-sitter grammars are
//! the plan's target for W4; they replace `decls` behind this same `symbols()` seam and change nothing else.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

/// One declaration: what it is, its name, and the line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub kind: String,
    pub name: String,
    pub line: usize,
}

/// At most this many declarations from one file (a generated file can hold tens of thousands).
const MAX_SYMBOLS: usize = 2_000;

/// The language family of a path, from its extension - the key the declaration patterns are chosen by.
pub fn language(path: &str) -> Option<&'static str> {
    let extension = path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();

    match extension.as_str() {
        "rs" => Some("rust"),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "svelte" => Some("typescript"),
        "py" | "pyi" => Some("python"),
        "go" => Some("go"),
        "java" | "kt" | "kts" | "cs" | "scala" | "swift" | "dart" => Some("jvm"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" => Some("c"),
        "md" | "mdx" | "rst" => Some("markdown"),
        "json" | "yaml" | "yml" | "toml" | "ini" => Some("data"),
        _ => None,
    }
}

/// The declaration patterns of a family: the kind first, then a pattern whose first group is the name.
fn decls(family: &str) -> &'static [( &'static str, Regex)] {
    static RUST: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static TYPESCRIPT: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static PYTHON: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static GO: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static JVM: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static RUBY: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static PHP: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static C: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static MARKDOWN: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    static DATA: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();

    let build = |list: &[(&'static str, &str)]| list.iter().map(|(kind, pattern)| (*kind, Regex::new(pattern).expect("a valid declaration pattern"))).collect::<Vec<_>>();
    let set = match family {
        "rust" => RUST.get_or_init(|| {
            build(&[
                ("fn", r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([A-Za-z_]\w*)"#),
                ("struct", r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_]\w*)"),
                ("enum", r"^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+([A-Za-z_]\w*)"),
                ("trait", r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+([A-Za-z_]\w*)"),
                ("impl", r"^\s*impl(?:<[^>]*>)?\s+(?:[A-Za-z_]\w*\s+for\s+)?([A-Za-z_]\w*)"),
                ("mod", r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_]\w*)"),
                ("type", r"^\s*(?:pub(?:\([^)]*\))?\s+)?type\s+([A-Za-z_]\w*)"),
                ("macro", r"^\s*macro_rules!\s+([A-Za-z_]\w*)"),
                ("const", r"^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z_]\w*)"),
            ])
        }),
        "typescript" => TYPESCRIPT.get_or_init(|| {
            build(&[
                ("class", r"^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][\w$]*)"),
                ("interface", r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][\w$]*)"),
                ("type", r"^\s*(?:export\s+)?type\s+([A-Za-z_$][\w$]*)"),
                ("enum", r"^\s*(?:export\s+)?(?:const\s+)?enum\s+([A-Za-z_$][\w$]*)"),
                ("fn", r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)"),
                ("fn", r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*(?::[^=]+)?=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][\w$]*)\s*(?::[^=]+)?=>"),
                ("method", r"^\s*(?:public|private|protected|static|readonly|async|get|set|\s)*([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*(?::\s*[^{]+)?\{\s*$"),
            ])
        }),
        "python" => PYTHON.get_or_init(|| build(&[("def", r"^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)"), ("class", r"^\s*class\s+([A-Za-z_]\w*)")])),
        "go" => GO.get_or_init(|| {
            build(&[
                ("fn", r"^func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)"),
                ("type", r"^type\s+([A-Za-z_]\w*)"),
                ("method", r"^\s+([A-Za-z_]\w*)\s+(?:struct|interface)\s*\{"),
            ])
        }),
        "jvm" => JVM.get_or_init(|| {
            build(&[
                ("class", r"^\s*(?:(?:public|private|protected|internal|static|final|abstract|sealed|open|override|data|async|suspend|virtual|partial|export)\s+)*(?:class|record|struct|object|protocol|extension)\s+([A-Za-z_]\w*)"),
                ("interface", r"^\s*(?:(?:public|private|protected|internal|static|final|abstract|sealed)\s+)*interface\s+([A-Za-z_]\w*)"),
                ("enum", r"^\s*(?:(?:public|private|protected|internal|static|final|abstract)\s+)*enum\s+([A-Za-z_]\w*)"),
                ("fn", r"^\s*(?:(?:public|private|protected|internal|static|final|abstract|override|async|suspend|open|virtual|fun|func)\s+)+(?:[\w<>\[\],?.]+\s+)?([A-Za-z_]\w*)\s*\("),
            ])
        }),
        "ruby" => RUBY.get_or_init(|| build(&[("def", r"^\s*def\s+([A-Za-z_]\w*[?!]?)"), ("class", r"^\s*class\s+([A-Za-z_]\w*)"), ("module", r"^\s*module\s+([A-Za-z_]\w*)")])),
        "php" => PHP.get_or_init(|| {
            build(&[
                ("class", r"^\s*(?:(?:final|abstract|readonly)\s+)*class\s+([A-Za-z_]\w*)"),
                ("interface", r"^\s*interface\s+([A-Za-z_]\w*)"),
                ("trait", r"^\s*trait\s+([A-Za-z_]\w*)"),
                ("fn", r"^\s*(?:(?:public|private|protected|static|abstract|final)\s+)*function\s+([A-Za-z_]\w*)"),
            ])
        }),
        "c" => C.get_or_init(|| {
            build(&[
                ("struct", r"^\s*(?:typedef\s+)?(?:struct|class|enum|union)\s+([A-Za-z_]\w*)"),
                ("namespace", r"^\s*namespace\s+([A-Za-z_]\w*)"),
                ("fn", r"^[A-Za-z_][\w:<>\*&\s]*[\s\*&]([A-Za-z_]\w*)\s*\([^;]*$"),
                ("macro", r"^#define\s+([A-Za-z_]\w*)"),
            ])
        }),
        "markdown" => MARKDOWN.get_or_init(|| build(&[("section", r"^#{1,6}\s+(.+?)\s*$")])),
        "data" => DATA.get_or_init(|| build(&[("key", r#"^\s{0,2}(?:"([A-Za-z_][\w\-]*)"|([A-Za-z_][\w\-]*))\s*[:=]"#)])),
        _ => return &[],
    };

    set.as_slice()
}

/// A name from a match: the first non-empty group (markdown headings and data keys use their own shape).
fn name_of(captures: &regex::Captures) -> String {
    (1..captures.len()).find_map(|group| captures.get(group).map(|found| found.as_str().trim().to_string())).unwrap_or_default()
}

/// The declarations in a file's text, in line order.
pub fn symbols(path: &str, text: &str) -> Vec<Symbol> {
    let Some(family) = language(path) else {
        return Vec::new();
    };
    let patterns = decls(family);
    let mut found: Vec<Symbol> = Vec::new();

    for (index, line) in text.lines().enumerate() {
        if found.len() >= MAX_SYMBOLS {
            break;
        }

        for (kind, pattern) in patterns {
            if let Some(captures) = pattern.captures(line) {
                let name = name_of(&captures);

                if !name.is_empty() {
                    found.push(Symbol { kind: kind.to_string(), name, line: index + 1 });
                }

                break;
            }
        }
    }

    found
}

/// The declarations of `name` in a list (exact match first, then case-insensitive, then as a part of a longer name).
pub fn matching(symbols: &[Symbol], name: &str) -> Vec<Symbol> {
    let wanted = name.trim().to_lowercase();
    let exact: Vec<Symbol> = symbols.iter().filter(|symbol| symbol.name.to_lowercase() == wanted).cloned().collect();

    if !exact.is_empty() {
        return exact;
    }

    symbols.iter().filter(|symbol| symbol.name.to_lowercase().contains(&wanted)).take(40).cloned().collect()
}

/// At most this many lines of one file's map (a huge file must not eat the whole map).
const MAX_LINES_PER_FILE: usize = 24;

/// The project's map: `path` then its declarations, for the most useful files first, within `budget` tokens.
///
/// Ranking: the files this turn already changed, then files whose declarations the request names, then the files
/// with the most declarations (the biggest, most structured ones - where the work usually is).
pub fn repo_map(root: &Path, request: &str, changed: &[String], budget: usize) -> String {
    let wanted: Vec<String> = request.split(|c: char| !c.is_alphanumeric() && c != '_').map(str::to_lowercase).filter(|word| word.chars().count() >= 4).collect();
    let mut files: Vec<(i64, String, Vec<Symbol>)> = Vec::new();

    for path in crate::search::files(root, 4_000) {
        let absolute = root.join(&path);

        if intel_skipped(&path) {
            continue;
        }

        let Ok(text) = std::fs::read_to_string(&absolute) else {
            continue;
        };

        if text.len() > 2_000_000 {
            continue;
        }

        let symbols = symbols(&path, &text);

        if symbols.is_empty() {
            continue;
        }

        let named = symbols.iter().filter(|symbol| wanted.iter().any(|word| symbol.name.to_lowercase().contains(word.as_str()))).count() as i64;
        let score = if changed.iter().any(|file| file.trim_start_matches("./") == path) { 1_000 } else { 0 } + named * 50 + symbols.len().min(50) as i64;

        files.push((score, path, symbols));
    }

    files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let header = "[SDC code map (declarations only - read a file for its code):]\n";
    let mut out = header.to_string();

    for (_, path, symbols) in files {
        let mut block = format!("\n{path}\n");
        for symbol in symbols.into_iter().take(MAX_LINES_PER_FILE) {
            block.push_str(&format!("  {} {} :{}\n", symbol.kind, symbol.name, symbol.line));
        }

        if crate::context::tokens_of(&out) + crate::context::tokens_of(&block) > budget as u64 {
            break;
        }

        out.push_str(&block);
    }

    if out == header {
        return String::new();
    }

    out
}

/// Files the map leaves out: SDC's own and other tools' state, and the fixtures and translations nobody edits.
fn intel_skipped(path: &str) -> bool {
    const SKIP: &[&str] = &[".sdc/", ".git/", "node_modules/", "target/", "dist/", "build/", "vendor/", ".venv/", "__pycache__/", "coverage/"];

    SKIP.iter().any(|part| path.starts_with(part) || path.contains(&format!("/{part}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_are_named_in_every_family() {
        let rust = symbols("a.rs", "pub struct Store {}\n\nimpl Store {\n    fn open() {}\n}\npub async fn run_once() -> u64 { 0 }\nconst MAX: u64 = 4;\n");
        let kinds: Vec<(&str, &str)> = rust.iter().map(|symbol| (symbol.kind.as_str(), symbol.name.as_str())).collect();

        assert_eq!(kinds, [("struct", "Store"), ("impl", "Store"), ("fn", "open"), ("fn", "run_once"), ("const", "MAX")]);
        assert_eq!(rust[2].line, 4);

        let typescript = symbols("a.tsx", "export interface Order { id: string }\nexport function total(o: Order) { return 1; }\nconst sum = (a: number) => a + 1;\n");
        let names: Vec<&str> = typescript.iter().map(|symbol| symbol.name.as_str()).collect();

        assert_eq!(names, ["Order", "total", "sum"]);

        let python = symbols("a.py", "class Job:\n    def run(self):\n        pass\n");
        let names: Vec<&str> = python.iter().map(|symbol| symbol.name.as_str()).collect();

        assert_eq!(names, ["Job", "run"]);
        assert!(symbols("blob.bin", "whatever").is_empty(), "an unknown extension has no declarations");
    }

    #[test]
    fn a_symbol_is_found_exactly_then_loosely() {
        let symbols = vec![Symbol { kind: "fn".into(), name: "charge".into(), line: 4 }, Symbol { kind: "fn".into(), name: "charge_once".into(), line: 9 }];

        assert_eq!(matching(&symbols, "charge").len(), 1, "the exact one");
        assert_eq!(matching(&symbols, "charge_").first().unwrap().line, 9, "then a part of a name");
        assert!(matching(&symbols, "nothing").is_empty());
    }

    #[test]
    fn the_map_puts_the_changed_and_the_named_files_first_and_stays_in_budget() {
        let root = std::env::temp_dir().join(format!("sdc-intel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();

        std::fs::write(root.join("src/orders.ts"), "export function createOrder() {}\nexport function listOrders() {}\n").unwrap();
        std::fs::write(root.join("src/other.ts"), "export function nothingToDoHere() {}\n").unwrap();

        let map = repo_map(&root, "the orders list is wrong", &["src/other.ts".to_string()], 400);

        assert!(map.contains("src/other.ts"), "the changed file leads:\n{map}");
        assert!(map.find("src/other.ts").unwrap() < map.find("src/orders.ts").unwrap());
        assert!(map.contains("createOrder"), "the named file is there too:\n{map}");
        assert!(crate::context::tokens_of(&repo_map(&root, "orders", &[], 40)) <= 40, "a tiny budget is respected");
        assert_eq!(repo_map(&root, "orders", &[], 8), "", "a budget with no room says nothing");

        let _ = std::fs::remove_dir_all(&root);
    }
}
