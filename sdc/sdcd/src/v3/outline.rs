//! **Skeleton-first reads** (v3 I2, switch `v3.skeleton`, a token technique - off until it passes Q3): a whole read
//! of a file over 200 lines answers its outline - the imports summed up, every declaration with its line number -
//! and the agent reads the part it needs with `offset`/`limit`.
//!
//! The outline is made by line patterns per language (declarations start lines in every language SDC's users write);
//! `W4` replaces it with tree-sitter symbols. When the outline would not save at least 40%, the file is sent whole.

use std::sync::OnceLock;

use regex::Regex;

/// Files at or under this many lines are always read whole.
pub const MIN_LINES: usize = 200;

/// At most this many outline lines.
const MAX_ENTRIES: usize = 160;

fn patterns(extension: &str) -> Option<&'static [Regex]> {
    static RUST: OnceLock<Vec<Regex>> = OnceLock::new();
    static TS: OnceLock<Vec<Regex>> = OnceLock::new();
    static PY: OnceLock<Vec<Regex>> = OnceLock::new();
    static GO: OnceLock<Vec<Regex>> = OnceLock::new();
    static JVM: OnceLock<Vec<Regex>> = OnceLock::new();
    static RUBY: OnceLock<Vec<Regex>> = OnceLock::new();
    static PHP: OnceLock<Vec<Regex>> = OnceLock::new();
    static C: OnceLock<Vec<Regex>> = OnceLock::new();
    static MD: OnceLock<Vec<Regex>> = OnceLock::new();
    static DATA: OnceLock<Vec<Regex>> = OnceLock::new();

    let build = |list: &[&str]| list.iter().map(|pattern| Regex::new(pattern).expect("a valid outline pattern")).collect::<Vec<_>>();
    let set = match extension {
        "rs" => RUST.get_or_init(|| build(&[r"^\s*(pub(\([^)]*\))?\s+)?(async\s+)?(unsafe\s+)?(const\s+)?(fn|struct|enum|trait|impl|mod|type|union|macro_rules!)\b", r"^\s*#\[(test|derive|cfg)"])),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "svelte" => TS.get_or_init(|| {
            build(&[
                r"^\s*(export\s+)?(default\s+)?(declare\s+)?(abstract\s+)?(async\s+)?(function\*?|class|interface|type|enum|namespace)\b",
                r"^\s*(export\s+)?(const|let|var)\s+\w+\s*(:[^=]+)?=\s*(async\s*)?(\([^)]*\)|\w+)\s*(:[^=]+)?=>",
                r"^\s*(public|private|protected|static|readonly|async|get|set|\s)*\s*[A-Za-z_$][\w$]*\s*\([^)]*\)\s*(:\s*[^{]+)?\{\s*$",
                r"^\s*(describe|it|test)\(",
            ])
        }),
        "py" => PY.get_or_init(|| build(&[r"^\s*(async\s+)?(def|class)\s", r"^@\w"])),
        "go" => GO.get_or_init(|| build(&[r"^(func|type)\s", r"^\s+\w+\s+(struct|interface)\s*\{"])),
        "java" | "kt" | "kts" | "cs" | "scala" | "swift" | "dart" => JVM.get_or_init(|| {
            build(&[
                r"^\s*((public|private|protected|internal|static|final|abstract|sealed|open|override|data|async|suspend|virtual|partial)\s+)*(class|interface|enum|record|struct|object|protocol|extension|fun|func|def)\b",
                r"^\s*((public|private|protected|internal|static|final|abstract|override|async|virtual)\s+)+[\w<>\[\],? ]+\s+\w+\s*\([^;]*$",
            ])
        }),
        "rb" => RUBY.get_or_init(|| build(&[r"^\s*(def|class|module)\s"])),
        "php" => PHP.get_or_init(|| build(&[r"^\s*((public|private|protected|static|abstract|final)\s+)*(function|class|interface|trait|enum)\b"])),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" => C.get_or_init(|| build(&[r"^(struct|class|enum|union|namespace|template|typedef)\b", r"^[A-Za-z_][\w:<>\*&\s,]*\s[\*&]?[A-Za-z_][\w:]*\s*\([^;]*$", r"^#define\s"])),
        "md" | "mdx" | "rst" => MD.get_or_init(|| build(&[r"^#{1,6}\s"])),
        "json" | "yaml" | "yml" | "toml" | "ini" => DATA.get_or_init(|| build(&[r#"^[A-Za-z_"\[][^\s]*"#, r#"^  ["A-Za-z_][\w"\-]*\s*[:=]"#])),
        _ => return None,
    };

    Some(set.as_slice())
}

fn import_line(line: &str) -> bool {
    let trimmed = line.trim_start();

    ["use ", "import ", "from ", "#include", "require(", "using ", "package "].iter().any(|start| trimmed.starts_with(start)) || trimmed.starts_with("const ") && trimmed.contains("require(")
}

/// The outline of a file's text, or `None` when there is no outline worth sending instead of the file.
pub fn outline(path: &str, text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();

    if lines.len() <= MIN_LINES {
        return None;
    }

    let extension = path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    let patterns = patterns(&extension)?;
    let imports: Vec<usize> = lines.iter().enumerate().filter(|(_, line)| import_line(line)).map(|(index, _)| index + 1).collect();
    let mut entries: Vec<String> = Vec::new();

    if let (Some(first), Some(last)) = (imports.first(), imports.last()) {
        entries.push(format!("{first:>5}\t[{} import line(s), {first}-{last}]", imports.len()));
    }

    for (index, line) in lines.iter().enumerate() {
        if imports.contains(&(index + 1)) {
            continue;
        }

        if patterns.iter().any(|pattern| pattern.is_match(line)) {
            let shown: String = line.trim_end().chars().take(160).collect();

            entries.push(format!("{:>5}\t{shown}", index + 1));

            if entries.len() >= MAX_ENTRIES {
                entries.push("     \t[… more declarations - read a range to see them]".to_string());
                break;
            }
        }
    }

    let body = format!(
        "[{path}: {} lines - this is its outline (SDC skeleton read). Read the part you need with read_file and offset/limit; the numbers are line numbers.]\n{}",
        lines.len(),
        entries.join("\n")
    );

    /* Worth it only when it saves at least 40% of the file. */
    (body.len() * 10 <= text.len() * 6 && entries.len() > 1).then_some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_file() -> String {
        let mut text = String::from("use std::fmt;\nuse std::io;\n\n");

        for index in 0..40 {
            text.push_str(&format!("/// Doc {index}\npub fn handler_{index}(input: &str) -> String {{\n    let value = input.trim();\n    format!(\"{{value}}\")\n}}\n\n"));
        }

        text.push_str("pub struct Ledger {\n    entries: Vec<u64>,\n}\n\nimpl Ledger {\n    pub fn total(&self) -> u64 {\n        self.entries.iter().sum()\n    }\n}\n");
        text
    }

    #[test]
    fn a_long_file_becomes_its_declarations_with_line_numbers() {
        let text = rust_file();
        let outline = outline("src/handlers.rs", &text).expect("a 250-line file has an outline");

        assert!(outline.contains("[2 import line(s), 1-2]"));
        assert!(outline.contains("pub fn handler_0(input: &str) -> String {"));
        assert!(outline.contains("pub struct Ledger") && outline.contains("impl Ledger") && outline.contains("pub fn total"));
        assert!(outline.len() * 10 <= text.len() * 6, "it saves at least 40%");

        let line = text.lines().position(|line| line.starts_with("pub struct Ledger")).unwrap() + 1;

        assert!(outline.contains(&format!("{line:>5}\tpub struct Ledger")), "the numbers are the file's");
    }

    #[test]
    fn short_files_unknown_kinds_and_flat_files_are_read_whole() {
        assert!(outline("a.rs", "fn main() {}\n").is_none());
        assert!(outline("notes.xyz", &"line\n".repeat(400)).is_none());
        assert!(outline("data.rs", &"    1, 2, 3,\n".repeat(400)).is_none(), "nothing to outline");
    }

    #[test]
    fn other_languages_have_outlines_too() {
        let python: String = (0..60).map(|index| format!("def step_{index}(x):\n    y = x + 1\n    return y\n\n")).collect();
        let typescript: String = (0..60)
            .map(|index| format!("export function step{index}(x: number): number {{\n  const y = x + 1;\n  const z = y * 2;\n  if (z > 10) {{\n    console.log(z);\n  }}\n  return y + z;\n}}\n"))
            .collect();

        assert!(outline("tasks.py", &python).unwrap().contains("def step_59(x):"));
        assert!(outline("tasks.ts", &typescript).unwrap().contains("export function step0(x: number): number {"));
    }
}
