//! The code steps of R-FIX: reproduce the failing tests, find where they point, apply a SEARCH/REPLACE patch only
//! if it matches exactly, and keep it only if the project's whole test run is green (P3-5).

use super::codefns::{run_capture, Data, Env};
use std::path::{Path, PathBuf};

const TEST_SECONDS: u64 = 300;
const TAIL_LINES: usize = 60;
const TAIL_CHARS: usize = 4_000;
const WINDOW: usize = 40;
const MAX_SUSPECTS: usize = 3;
const CODE_EXTENSIONS: [&str; 12] = ["rs", "py", "js", "mjs", "cjs", "jsx", "ts", "tsx", "go", "java", "c", "cpp"];

struct TestCommand {
    label: String,
    program: PathBuf,
    args: Vec<&'static str>,
}

fn npm() -> PathBuf {
    PathBuf::from(if cfg!(windows) { "npm.cmd" } else { "npm" })
}

fn python() -> PathBuf {
    PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
}

/// How this project runs its tests, from the files it has.
fn test_command(root: &Path) -> Option<TestCommand> {
    if root.join("Cargo.toml").is_file() {
        return Some(TestCommand { label: "cargo test".to_string(), program: PathBuf::from("cargo"), args: vec!["test", "--quiet"] });
    }

    if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
        let script = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|json| json["scripts"]["test"].as_str().map(str::to_string));

        if script.is_some_and(|script| !script.contains("no test specified")) {
            return Some(TestCommand { label: "npm test".to_string(), program: npm(), args: vec!["test", "--silent"] });
        }
    }

    let has_pytest = ["pytest.ini", "conftest.py", "tox.ini"].iter().any(|name| root.join(name).is_file()) || std::fs::read_to_string(root.join("pyproject.toml")).is_ok_and(|text| text.contains("pytest"));

    if has_pytest || root.join("tests").is_dir() && root.join("tests").read_dir().is_ok_and(|mut entries| entries.any(|entry| entry.is_ok_and(|entry| entry.file_name().to_string_lossy().starts_with("test_")))) {
        return Some(TestCommand { label: "pytest".to_string(), program: python(), args: vec!["-m", "pytest", "-x", "-q"] });
    }

    None
}

fn tail_of(output: &str) -> String {
    let lines: Vec<&str> = output.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
    let skip = tail.chars().count().saturating_sub(TAIL_CHARS);

    tail.chars().skip(skip).collect()
}

/// Runs the project's tests: (passed, the tail of what they printed).
fn run_tests_now(env: &Env, command: &TestCommand) -> Result<(bool, String), String> {
    run_capture(&env.root, &command.program, &command.args, TEST_SECONDS).map(|(passed, output)| (passed, tail_of(&output))).map_err(|reason| format!("{} could not be run: {reason}", command.label))
}

/// R-FIX step 1: the failing test output. Fails (with the reason) when there is no test command or the tests already pass.
pub fn run_tests(env: &Env, _unit: Option<&str>, _data: &Data) -> Result<Vec<String>, String> {
    let command = test_command(&env.root).ok_or("no test command was found (no Cargo.toml, no package.json test script, no pytest) - tell me how to run the tests")?;
    let (passed, tail) = run_tests_now(env, &command)?;

    if passed {
        return Err(format!("{} passes already, so there is nothing to reproduce - tell me what fails", command.label));
    }

    let mut lines = vec![format!("#ran {}", command.label)];

    lines.extend(tail.lines().map(str::to_string));

    Ok(lines)
}

fn is_test_path(path: &str) -> bool {
    let lower = path.to_lowercase();

    lower.contains("test") || lower.contains("spec") || lower.contains("__tests__")
}

fn code_file(root: &Path, relative: &str) -> bool {
    Path::new(relative).extension().is_some_and(|extension| CODE_EXTENSIONS.contains(&extension.to_string_lossy().to_lowercase().as_str())) && !relative.contains("node_modules") && root.join(relative).is_file()
}

/// (relative path, line) pairs a failure output points at, in order of appearance.
fn pointed_at(env: &Env, failures: &[String]) -> Vec<(String, usize)> {
    let root = env.root.to_string_lossy().replace('\\', "/");
    let real = std::fs::canonicalize(&env.root).map(|path| path.to_string_lossy().replace('\\', "/").trim_start_matches("//?/").to_string()).unwrap_or_default();
    let mut found: Vec<(String, usize)> = Vec::new();

    for raw in failures {
        let mut line = raw.replace('\\', "/").replace(&format!("{root}/"), "");

        if !real.is_empty() {
            line = line.replace(&format!("{real}/"), "");
        }

        if let Some(at) = line.find("File \"") {
            let rest = &line[at + 6..];

            if let (Some(end), Some(row)) = (rest.find('"'), rest.split("line ").nth(1).and_then(|tail| tail.split(|c: char| !c.is_ascii_digit()).next()).and_then(|digits| digits.parse::<usize>().ok())) {
                if code_file(&env.root, &rest[..end]) {
                    found.push((rest[..end].to_string(), row));
                }
            }
        }

        for token in line.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '\'' | '"' | '<' | '>' | ',')) {
            let mut path = token.trim_end_matches(':');
            let mut row = 0;

            for _ in 0..2 {
                if let Some((head, tail)) = path.rsplit_once(':') {
                    if let Ok(number) = tail.parse::<usize>() {
                        row = number;
                        path = head;
                    }
                }
            }

            let path = path.trim_start_matches("./");

            if row > 0 && code_file(&env.root, path) && !found.iter().any(|(known, _)| known == path) {
                found.push((path.to_string(), row));
            }
        }
    }

    found
}

/// The files a test file imports (`./x`, `../x`, python `from x import`), as relative paths that exist.
pub fn imports_of(env: &Env, test_file: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(env.root.join(test_file)) else {
        return Vec::new();
    };
    let dir = Path::new(test_file).parent().map(Path::to_path_buf).unwrap_or_default();
    let mut names: Vec<String> = Vec::new();

    for line in text.lines() {
        for quote in ['\'', '"'] {
            for piece in line.split(quote).skip(1).step_by(2) {
                if piece.starts_with("./") || piece.starts_with("../") {
                    names.push(piece.to_string());
                }
            }
        }

        let words: Vec<&str> = line.split_whitespace().collect();

        if let (Some("from" | "import"), Some(module)) = (words.first().copied(), words.get(1)) {
            if !module.starts_with(['\'', '"']) && module.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.')) {
                names.push(format!("./{}", module.replace('.', "/")));
            }
        }
    }

    let mut files = Vec::new();

    for name in names {
        let joined = dir.join(&name);
        let base = joined.components().fold(PathBuf::new(), |mut path, part| {
            match part {
                std::path::Component::ParentDir => {
                    path.pop();
                }
                std::path::Component::CurDir => {}
                other => path.push(other),
            }

            path
        });
        let base_text = base.to_string_lossy().replace('\\', "/");
        let mut candidates = vec![base_text.clone()];

        candidates.extend(CODE_EXTENSIONS.iter().map(|extension| format!("{base_text}.{extension}")));

        if let Some(found) = candidates.into_iter().find(|candidate| code_file(&env.root, candidate)) {
            if !files.contains(&found) {
                files.push(found);
            }
        }
    }

    files
}

fn window_of(env: &Env, path: &str, row: usize) -> Option<String> {
    let text = std::fs::read_to_string(env.root.join(path)).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let start = row.saturating_sub(WINDOW + 1).min(lines.len());
    let end = (row + WINDOW).min(lines.len());
    let body: Vec<String> = lines[start..end].iter().enumerate().map(|(offset, line)| format!("{}| {line}", start + offset + 1)).collect();

    Some(format!("File: {path} (lines {}-{})\n{}", start + 1, end, body.join("\n")))
}

/// R-FIX step 2: where the failure points - the source files first, then what a failing test imports - each with the code around the line.
pub fn localize(env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let failures = data.get("failures").cloned().unwrap_or_default();
    let pointed = pointed_at(env, &failures);
    let mut suspects: Vec<(String, usize)> = pointed.iter().filter(|(path, _)| !is_test_path(path)).cloned().collect();

    for (test_file, _) in pointed.iter().filter(|(path, _)| is_test_path(path)) {
        for file in imports_of(env, test_file) {
            if !is_test_path(&file) && !suspects.iter().any(|(known, _)| *known == file) {
                suspects.push((file, 1));
            }
        }
    }

    suspects.truncate(MAX_SUSPECTS);

    if suspects.is_empty() {
        return Err("the failure output does not point at a source file I can find".to_string());
    }

    let mut context: Vec<String> = suspects.iter().filter_map(|(path, row)| window_of(env, path, *row)).collect();

    for (test_file, row) in pointed.iter().filter(|(path, _)| is_test_path(path)).take(1) {
        context.extend(window_of(env, test_file, *row));
    }

    Ok(vec![context.join("\n\n")])
}

#[derive(Debug, PartialEq)]
pub struct Block {
    pub path: String,
    pub search: String,
    pub replace: String,
    /// Set by the symbol form (P4-1, `nr.symbol_patch`): replace the whole function of this name; `replace` is its new text and `search` is empty.
    pub symbol: Option<String>,
}

fn path_of(line: &str) -> Option<String> {
    let line = line.trim().trim_start_matches(['#', '*', '`', ' ']).trim_end_matches(['*', '`', ':', ' ']);
    let line = line.strip_prefix("FILE:").or_else(|| line.strip_prefix("File:")).or_else(|| line.strip_prefix("FILE")).unwrap_or(line).trim().trim_matches(['`', '"', '\'', '*']);

    (!line.is_empty() && line.len() <= 120 && !line.contains(' ') && line.contains('.')).then(|| line.trim_start_matches("./").replace('\\', "/"))
}

/// The SEARCH/REPLACE blocks of an answer.
pub fn parse_blocks(answer: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut path = String::new();
    let mut part = 0;
    let mut search: Vec<&str> = Vec::new();
    let mut replace: Vec<&str> = Vec::new();
    let mut symbol = String::new();
    let symbol_form = crate::v3::on("nr.symbol_patch");

    for line in answer.lines() {
        let trimmed = line.trim();

        match part {
            0 if symbol_form && trimmed.starts_with("SYMBOL:") => {
                symbol = trimmed.trim_start_matches("SYMBOL:").trim().trim_matches(['`', '"', '\'', '*']).to_string();
                replace.clear();
                part = 3;
            }
            3 if trimmed.starts_with("```") => part = 4,
            3 => {}
            4 if trimmed.starts_with("```") => {
                if !path.is_empty() && !symbol.is_empty() {
                    blocks.push(Block { path: path.clone(), search: String::new(), replace: replace.join("
"), symbol: Some(symbol.clone()) });
                }

                part = 0;
            }
            4 => replace.push(line),
            0 if trimmed.starts_with("<<<<<<<") => {
                part = 1;
                search.clear();
                replace.clear();
            }
            0 => {
                if let Some(found) = path_of(line).filter(|_| !trimmed.starts_with("```") || trimmed.len() > 3) {
                    path = found;
                }
            }
            1 if trimmed.starts_with("=======") => part = 2,
            1 => search.push(line),
            _ if trimmed.starts_with(">>>>>>>") => {
                if !path.is_empty() {
                    blocks.push(Block { path: path.clone(), search: search.join("\n"), replace: replace.join("\n"), symbol: None });
                }

                part = 0;
            }
            _ => replace.push(line),
        }
    }

    blocks
}

fn safe_relative(root: &Path, path: &str) -> bool {
    !path.is_empty() && !path.contains(':') && !path.starts_with('/') && Path::new(path).components().all(|part| matches!(part, std::path::Component::Normal(_))) && root.join(path).is_file()
}

/// Applies the blocks in memory. Every SEARCH must be in its file exactly once.
fn apply(env: &Env, blocks: &[Block]) -> Result<Vec<(String, String, String)>, String> {
    let mut files: Vec<(String, String, String)> = Vec::new();

    for (number, block) in blocks.iter().enumerate() {
        if !safe_relative(&env.root, &block.path) {
            return Err(format!("block {}: {} is not an existing file of the project", number + 1, block.path));
        }

        if !files.iter().any(|(path, _, _)| *path == block.path) {
            let text = std::fs::read_to_string(env.root.join(&block.path)).map_err(|error| format!("block {}: could not read {}: {error}", number + 1, block.path))?;

            files.push((block.path.clone(), text.clone(), text));
        }

        let entry = files.iter_mut().find(|(path, _, _)| *path == block.path).expect("just added");

        if let Some(symbol) = &block.symbol {
            entry.2 = crate::navaratna::patcher::replace_symbol(&block.path, &entry.2, symbol, &block.replace).map_err(|refused| format!("block {}: the function {symbol} in {} was not replaced: {refused}", number + 1, block.path))?;

            super::engines::note("symbol_patch", format!("{symbol} in {}", block.path));

            continue;
        }

        let crlf = entry.1.contains("\r\n");
        let (search, replace) = if crlf { (block.search.replace('\n', "\r\n"), block.replace.replace('\n', "\r\n")) } else { (block.search.clone(), block.replace.clone()) };

        if search.trim().is_empty() {
            return Err(format!("block {}: the SEARCH text is empty", number + 1));
        }

        match entry.2.matches(&search).count() {
            1 => entry.2 = entry.2.replacen(&search, &replace, 1),
            0 => return Err(format!("block {}: the SEARCH text is not in {} - copy it exactly as it is in the file, with its indentation", number + 1, block.path)),
            count => return Err(format!("block {}: the SEARCH text is in {} {count} times - include more lines around it so it is unique", number + 1, block.path)),
        }
    }

    Ok(files)
}

/// The patch check of the `patch` step: the blocks must match exactly once each, and the project's whole test run must pass with them.
/// If it does not, every file is put back as it was and the reason (with the test output) goes to the next try.
pub fn patch_fixes(env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let answer = data.get("output").and_then(|lines| lines.first()).map(String::as_str).unwrap_or("");
    let blocks = parse_blocks(answer);

    if blocks.is_empty() {
        return Err("no SEARCH/REPLACE block was found - answer with FILE: path, then <<<<<<< SEARCH, the old lines, =======, the new lines, >>>>>>> REPLACE".to_string());
    }

    let command = test_command(&env.root).ok_or("no test command was found")?;

    /* P4-1 (N5-1): a step tagged test_first treats the tests as anchors - a patch that edits one is refused before anything is written. */
    if super::engines::test_first(data) {
        super::engines::refuse_test_edits(&blocks.iter().map(|block| block.path.clone()).collect::<Vec<_>>())?;
        super::engines::note("test_first", "the tests were the anchors: no test file may change");
    }

    let files = apply(env, &blocks)?;
    let paths: Vec<String> = files.iter().map(|(path, _, _)| path.clone()).collect();

    super::engines::before_create(env, &paths);

    let mut write_and_test = || -> Result<Vec<String>, String> {
        for (path, _, new) in &files {
            std::fs::write(env.root.join(path), new).map_err(|error| format!("could not write {path}: {error}"))?;
        }

        let put_back = |files: &[(String, String, String)]| {
            for (path, old, _) in files {
                let _ = std::fs::write(env.root.join(path), old);
            }
        };

        match run_tests_now(env, &command) {
            Ok((true, _)) => Ok(paths.clone()),
            Ok((false, tail)) => {
                put_back(&files);

                Err(format!("with that change {} still fails (the change was undone):
{tail}", command.label))
            }
            Err(reason) => {
                put_back(&files);

                Err(reason)
            }
        }
    };
    let kept = super::engines::shadowed(env, data, &mut write_and_test)?;

    super::engines::register(env, data, &kept, &format!("{} passes", command.label));

    Ok(kept)
}

/// The short report of a fix.
pub fn fix_report(_env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let label = data.get("failures").and_then(|lines| lines.first()).and_then(|line| line.strip_prefix("#ran ")).unwrap_or("the tests");
    let patch = data.get("patch").and_then(|lines| lines.first()).map(String::as_str).unwrap_or("");
    let mut lines = vec!["# Fix".to_string(), String::new(), format!("`{label}` failed before the change and passes after it (run again with the change in place).")];

    lines.push(String::new());
    lines.push("Changed:".to_string());

    for block in parse_blocks(patch) {
        lines.push(format!("- `{}`: `{}` -> `{}`", block.path, one_line(&block.search), one_line(&block.replace)));
    }

    Ok(vec![lines.join("\n")])
}

fn one_line(text: &str) -> String {
    let joined = text.lines().map(str::trim).collect::<Vec<_>>().join(" ");

    if joined.chars().count() > 90 { format!("{}...", joined.chars().take(90).collect::<String>()) } else { joined }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("sdc-fix-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        std::fs::create_dir_all(root.join("src")).unwrap();

        Env::new(root)
    }

    #[test]
    fn search_replace_blocks_are_read_from_an_answer() {
        let answer = "Here is the fix.\n\nFILE: src/math.js\n<<<<<<< SEARCH\nreturn a - b;\n=======\nreturn a + b;\n>>>>>>> REPLACE\n\n```\nsrc/other.js\n<<<<<<< SEARCH\nx\n=======\ny\n>>>>>>> REPLACE\n```";
        let blocks = parse_blocks(answer);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], Block { path: "src/math.js".into(), search: "return a - b;".into(), replace: "return a + b;".into(), symbol: None });
        assert_eq!(blocks[1].path, "src/other.js");
    }

    #[test]
    fn a_search_must_match_exactly_once_and_nothing_else_is_touched() {
        let env = project("apply");

        std::fs::write(env.root.join("src/a.js"), "one();\ntwo();\ntwo();\n").unwrap();

        let block = |search: &str, path: &str| Block { path: path.into(), search: search.into(), replace: "x();".into(), symbol: None };

        assert!(apply(&env, &[block("one();", "src/a.js")]).is_ok());
        assert!(apply(&env, &[block("two();", "src/a.js")]).unwrap_err().contains("2 times"));
        assert!(apply(&env, &[block("three();", "src/a.js")]).unwrap_err().contains("not in src/a.js"));
        assert!(apply(&env, &[block("one();", "../a.js")]).unwrap_err().contains("not an existing file"));
        assert!(apply(&env, &[block("one();", "src/none.js")]).is_err());
        assert_eq!(std::fs::read_to_string(env.root.join("src/a.js")).unwrap(), "one();\ntwo();\ntwo();\n", "applying in memory leaves the file alone");

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn the_failure_points_at_source_files_and_at_what_the_test_imports() {
        let env = project("locate");

        std::fs::write(env.root.join("src/math.js"), "exports.add = (a, b) => a - b;\n").unwrap();
        std::fs::write(env.root.join("test.js"), "const { add } = require('./src/math');\nassert.strictEqual(add(1, 2), 3);\n").unwrap();

        let failures = vec!["AssertionError: 1 !== 3".to_string(), format!("    at Object.<anonymous> ({}\\test.js:2:8)", env.root.display())];
        let mut data = Data::new();

        data.insert("failures".to_string(), failures);

        let context = localize(&env, None, &data).unwrap().join("\n");

        assert!(context.contains("File: src/math.js") && context.contains("1| exports.add = (a, b) => a - b;"), "{context}");
        assert!(context.contains("File: test.js"), "{context}");

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn with_no_test_command_or_passing_tests_there_is_nothing_to_reproduce() {
        let env = project("none");

        assert!(run_tests(&env, None, &Data::new()).unwrap_err().contains("no test command"));

        let _ = std::fs::remove_dir_all(&env.root);
    }
}
