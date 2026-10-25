//! **The code steps a recipe can name (P3-1)**: plain Rust functions a recipe calls by name - no model, no guessing. A code step gets the project, the one
//! unit it works on (for a fan-out step) and only the data it declared as inputs; it returns lines, or the reason it could not.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Where a recipe runs.
#[derive(Debug, Clone)]
pub struct Env {
    pub root: PathBuf,
    /// The most characters of a file one review unit holds (half the model's window, in characters).
    pub unit_chars: usize,
    /// "bn" or "same as the request": the language the person is answered in.
    pub language_out: String,
    /// What the person asked for, in English when it was put into English.
    pub goal: String,
    /// The model that answers the recipe's steps (written into the registry beside what a step made).
    pub model: String,
}

impl Env {
    pub fn new(root: PathBuf) -> Env {
        Env { root, unit_chars: 12_000, language_out: "same as the request".to_string(), goal: String::new(), model: String::new() }
    }
}

/// The named lists the steps of a recipe pass to each other.
pub type Data = BTreeMap<String, Vec<String>>;

/// A code step or a gate: the project, the unit (if the step fans out), the declared inputs. A gate returns its evidence; a code step its output lines.
pub type CodeFn = fn(&Env, Option<&str>, &Data) -> Result<Vec<String>, String>;

/// The most files an inventory lists.
pub const MAX_FILES: usize = 20_000;

/// Folders that are never reviewed or counted.
pub const SKIPPED: &[&str] = &[".git", "node_modules", "target", "dist", ".sdc"];

/// Every file of the project, relative to its root with forward slashes, sorted: `.gitignore` is respected and `node_modules`, `target`, `dist` are skipped.
pub fn list_files(env: &Env, _unit: Option<&str>, _data: &Data) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let walk = ignore::WalkBuilder::new(&env.root).hidden(false).filter_entry(|entry| !SKIPPED.contains(&entry.file_name().to_string_lossy().as_ref())).build();

    for entry in walk.flatten() {
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            if let Ok(relative) = entry.path().strip_prefix(&env.root) {
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }

        if files.len() >= MAX_FILES {
            break;
        }
    }

    files.sort();

    if files.is_empty() {
        return Err(format!("no files under {}", env.root.display()));
    }

    Ok(files)
}

/// `path: N lines` for the file named by the unit.
pub fn count_lines(env: &Env, unit: Option<&str>, _data: &Data) -> Result<Vec<String>, String> {
    let path = unit.ok_or("count_lines works on one file at a time")?;
    let bytes = std::fs::read(Path::new(&env.root).join(path)).map_err(|error| format!("could not read {path}: {error}"))?;
    let lines = bytes.iter().filter(|byte| **byte == b'\n').count() + usize::from(!bytes.is_empty() && bytes.last() != Some(&b'\n'));

    Ok(vec![format!("{path}: {lines} lines")])
}

/// A gate: every declared input holds at least one non-blank line.
pub fn outputs_present(_env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let mut proof = Vec::new();

    for (name, lines) in data.iter().filter(|(name, _)| !name.starts_with('_')) {
        let count = lines.iter().filter(|line| !line.trim().is_empty()).count();

        if count == 0 {
            return Err(format!("{name} is empty"));
        }

        proof.push(format!("{name}: {count} items"));
    }

    Ok(proof)
}

/// Extensions of files that are not text worth reviewing (binary, generated, or a lock file).
const NOT_REVIEWED: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "bmp", "webp", "pdf", "zip", "gz", "tar", "7z", "exe", "dll", "so", "dylib", "bin", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "mov", "wav", "ogg", "wasm", "class", "jar", "o",
    "a", "lib", "pdb", "db", "sqlite", "svg", "lock", "map",
];

/// Folders that a project's tests and docs live in: they are reviewed after the code.
const LOW_RISK: &[&str] = &["test", "tests", "spec", "fixtures", "docs", "doc", "examples", "example"];

/// Words in a path that say the code guards something.
const RISKY: &[&str] = &["auth", "login", "password", "secret", "token", "crypt", "payment", "sql", "shell", "exec", "upload", "session", "admin", "permission", "sandbox"];

const MAX_FILE_BYTES: u64 = 1_000_000;
const MAX_LINE_CHARS: usize = 4_000;
const MAX_DIAGNOSTICS: usize = 150;
const CHECKER_SECONDS: u64 = 180;

fn reviewable_name(path: &str) -> bool {
    let lower = path.to_lowercase();
    let extension = lower.rsplit('.').next().unwrap_or("");

    !(NOT_REVIEWED.contains(&extension) || lower.ends_with(".min.js") || lower.ends_with(".min.css") || lower.ends_with("package-lock.json") || lower.ends_with("pnpm-lock.yaml"))
}

/// The text of a file worth reviewing, or `None` (too big, not text, or a minified line).
fn read_text(env: &Env, path: &str) -> Option<String> {
    let full = env.root.join(path);

    if std::fs::metadata(&full).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }

    let text = String::from_utf8(std::fs::read(full).ok()?).ok()?;

    (!text.lines().any(|line| line.chars().count() > MAX_LINE_CHARS)).then_some(text)
}

/// Splits lines into parts that each fit `limit` characters once numbered; half-open `(start, end)` line ranges.
fn parts(lines: &[&str], limit: usize) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = 0;
    let mut size = 0;

    for (index, line) in lines.iter().enumerate() {
        let cost = line.len() + 8;

        if index > start && size + cost > limit {
            found.push((start, index));
            start = index;
            size = 0;
        }

        size += cost;
    }

    if start < lines.len() {
        found.push((start, lines.len()));
    }

    found
}

/// `path` or `path#2/5` (the second of five parts of a long file).
pub fn split_unit(unit: &str) -> (&str, Option<(usize, usize)>) {
    if let Some((path, tail)) = unit.rsplit_once('#') {
        if let Some((part, total)) = tail.split_once('/') {
            if let (Ok(part), Ok(total)) = (part.parse::<usize>(), total.parse::<usize>()) {
                return (path, Some((part, total)));
            }
        }
    }

    (unit, None)
}

/// The numbered text of one review unit, with a header that says which part of the file it is.
pub fn read_unit(env: &Env, unit: &str) -> Result<String, String> {
    let (path, part) = split_unit(unit);
    let text = read_text(env, path).ok_or_else(|| format!("could not read {path} as text"))?;
    let lines: Vec<&str> = text.lines().collect();
    let all = parts(&lines, env.unit_chars);
    let (number, _) = part.unwrap_or((1, 1));
    let &(start, end) = all.get(number.saturating_sub(1)).ok_or_else(|| format!("{path} has no part {number}"))?;
    let body: Vec<String> = lines[start..end].iter().enumerate().map(|(offset, line)| format!("{}| {line}", start + offset + 1)).collect();

    Ok(format!("File: {path} (part {number} of {}, lines {}-{})\n{}", all.len(), start + 1, end, body.join("\n")))
}

/// `path:line: message` -> (path, line, message). Understands `path:12:5: m`, `path:12: m` and tsc's `path(12,5): m`.
fn locate(line: &str) -> Option<(String, usize, String)> {
    if let Some(open) = line.find('(') {
        let rest = &line[open + 1..];

        if let Some(close) = rest.find("): ") {
            if let Some((row, column)) = rest[..close].split_once(',') {
                if let (Ok(row), Ok(_)) = (row.trim().parse::<usize>(), column.trim().parse::<usize>()) {
                    return Some((line[..open].to_string(), row, rest[close + 3..].to_string()));
                }
            }
        }
    }

    for (at, _) in line.match_indices(':') {
        let after = &line[at + 1..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();

        if at > 0 && !digits.is_empty() && after[digits.len()..].starts_with(':') {
            let mut message = &after[digits.len() + 1..];
            let column: String = message.chars().take_while(|c| c.is_ascii_digit()).collect();

            if !column.is_empty() && message[column.len()..].starts_with(':') {
                message = &message[column.len() + 1..];
            }

            return Some((line[..at].to_string(), digits.parse().ok()?, message.trim().to_string()));
        }
    }

    None
}

/// The Error Slicer: from a checker's raw output only the diagnostics, each as `path:line: message` relative to the project root, never more than 150.
pub fn slice_errors(output: &str, root: &Path, dir: &Path) -> Vec<String> {
    let root_text = root.to_string_lossy().replace('\\', "/").to_lowercase();
    let mut found = Vec::new();

    for raw in output.lines() {
        let Some((path, row, message)) = locate(raw.trim()) else {
            continue;
        };
        let lower = message.to_lowercase();

        if !(lower.contains("error") || lower.contains("warning")) {
            continue;
        }

        let mut path = path.trim().replace('\\', "/");

        if path.to_lowercase().starts_with(&root_text) {
            path = path[root_text.len()..].trim_start_matches('/').to_string();
        } else if !path.starts_with('/') && !path.contains(':') {
            let base = dir.strip_prefix(root).unwrap_or(Path::new("")).to_string_lossy().replace('\\', "/");

            path = if base.is_empty() { path.trim_start_matches("./").to_string() } else { format!("{base}/{}", path.trim_start_matches("./")) };
        }

        found.push(format!("{path}:{row}: {}", message.chars().take(200).collect::<String>()));

        if found.len() >= MAX_DIAGNOSTICS {
            break;
        }
    }

    found
}

fn checker_output(dir: &Path, program: &Path, args: &[&str]) -> Result<String, String> {
    run_capture(dir, program, args, CHECKER_SECONDS).map(|(_, output)| output)
}

/// Runs a program in `dir` without a window, with its output collected and a time limit; whether it exited with success, and everything it printed.
pub fn run_capture(dir: &Path, program: &Path, args: &[&str], seconds: u64) -> Result<(bool, String), String> {
    use std::io::Read;
    use std::process::Stdio;

    let mut command = std::process::Command::new(program);

    command.args(args).current_dir(dir).env("CARGO_NET_OFFLINE", "true").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(0x0800_0000);
    }

    let mut child = command.spawn().map_err(|error| format!("could not start {}: {error}", program.display()))?;
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();

            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut bytes);
            }

            String::from_utf8_lossy(&bytes).to_string()
        })
    };
    let first = drain(child.stdout.take().map(|stream| Box::new(stream) as Box<dyn Read + Send>));
    let second = drain(child.stderr.take().map(|stream| Box::new(stream) as Box<dyn Read + Send>));
    let started = std::time::Instant::now();

    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed().as_secs() >= seconds => {
                let _ = child.kill();
                let _ = child.wait();

                return Err(format!("timed out after {seconds}s"));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(error) => return Err(error.to_string()),
        }
    };

    Ok((success, format!("{}\n{}", first.join().unwrap_or_default(), second.join().unwrap_or_default())))
}

/// A tool installed in the project (`node_modules/.bin`) in `dir` or above it, up to the project root.
fn local_tool(root: &Path, dir: &Path, name: &str) -> Option<PathBuf> {
    let mut at = Some(dir);

    while let Some(here) = at {
        for candidate in [format!("{name}.cmd"), name.to_string()] {
            let path = here.join("node_modules").join(".bin").join(candidate);

            if path.is_file() {
                return Some(path);
            }
        }

        at = if here == root { None } else { here.parent() };
    }

    None
}

/// The project's own checkers (`cargo check`, `tsc --noEmit`, `eslint`) run where the inventory finds their config. Their real diagnostics, sliced;
/// a checker that is missing or fails to run is named on a `#` line, never silently dropped. Needs the `files` input.
pub fn collect_errors(env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let files = data.get("files").cloned().unwrap_or_default();
    let mut lines = Vec::new();
    let mut ran = 0;

    for file in &files {
        let name = file.rsplit('/').next().unwrap_or(file);
        let dir_name = file.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        let dir = if dir_name.is_empty() { env.root.clone() } else { env.root.join(dir_name) };
        let shown = if dir_name.is_empty() { ".".to_string() } else { dir_name.to_string() };

        if ran >= 6 {
            break;
        }

        let (label, program, args): (&str, Option<PathBuf>, Vec<&str>) = match name {
            "Cargo.toml" => ("cargo check", Some(PathBuf::from("cargo")), vec!["check", "--message-format", "short", "--quiet"]),
            "tsconfig.json" => ("tsc", local_tool(&env.root, &dir, "tsc"), vec!["--noEmit", "--pretty", "false"]),
            "eslint.config.js" | "eslint.config.mjs" | ".eslintrc.json" | ".eslintrc.js" | ".eslintrc.cjs" => ("eslint", local_tool(&env.root, &dir, "eslint"), vec![".", "--format", "unix"]),
            _ => continue,
        };

        let Some(program) = program else {
            lines.push(format!("#skipped {label} in {shown}: not installed in the project"));

            continue;
        };

        ran += 1;

        match checker_output(&dir, &program, &args) {
            Ok(output) => {
                let found = slice_errors(&output, &env.root, &dir);

                lines.push(format!("#ran {label} in {shown}: {} diagnostics", found.len()));
                lines.extend(found);
            }
            Err(reason) => lines.push(format!("#skipped {label} in {shown}: {reason}")),
        }
    }

    Ok(lines)
}

/// Puts the files in the order a reviewer should read them (files a checker already complained about first, then code that guards something, then the
/// rest, tests and docs last) and cuts long files into parts that each fit a small model's window. Skips what is not text. Needs `files` and `errors`.
pub fn plan_units(env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let files = data.get("files").cloned().unwrap_or_default();
    let errors = data.get("errors").cloned().unwrap_or_default();
    let mut scored: Vec<(i64, String, usize)> = Vec::new();

    for file in files.iter().filter(|file| reviewable_name(file)) {
        let Some(text) = read_text(env, file) else {
            continue;
        };

        if text.trim().is_empty() {
            continue;
        }

        let lower = file.to_lowercase();
        let prefix = format!("{file}:");
        let mut score = errors.iter().filter(|line| line.starts_with(&prefix)).count() as i64 * 100;

        score += RISKY.iter().filter(|word| lower.contains(*word)).count() as i64 * 30;
        score += (text.lines().count() / 100).min(20) as i64;

        if ["rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "php", "c", "cpp", "cs", "rb", "kt", "swift"].contains(&lower.rsplit('.').next().unwrap_or("")) {
            score += 10;
        }

        if lower.split('/').any(|part| LOW_RISK.contains(&part)) {
            score -= 20;
        }

        let lines: Vec<&str> = text.lines().collect();

        scored.push((score, file.clone(), parts(&lines, env.unit_chars).len()));
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let units: Vec<String> = scored.into_iter().flat_map(|(_, file, count)| if count <= 1 { vec![file] } else { (1..=count).map(|part| format!("{file}#{part}/{count}")).collect() }).collect();

    if units.is_empty() {
        return Err("no source text to review".into());
    }

    Ok(units)
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_line_number(evidence: &str) -> &str {
    let digits = evidence.chars().take_while(|c| c.is_ascii_digit()).count();

    if digits > 0 {
        if let Some(after) = evidence[digits..].trim_start().strip_prefix('|') {
            return after;
        }
    }

    evidence
}

/// One finding line cut into `[location, severity, category, problem, evidence]`, or `None` when it does not have the five parts.
fn fields(line: &str) -> Option<[String; 5]> {
    let line = line.trim().trim_start_matches(['-', '*', '`']).trim();
    let line = match line.split_once(". ") {
        Some((number, rest)) if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) => rest,
        _ => line,
    };
    let separator = if line.contains(" · ") { " · " } else { " | " };
    let mut parts: Vec<&str> = line.split(separator).map(str::trim).collect();

    if parts.len() < 5 {
        return None;
    }

    let evidence = parts.split_off(4).join(separator);

    Some([parts[0].to_string(), parts[1].to_lowercase(), parts[2].to_string(), parts[3].to_string(), evidence])
}

/// The evidence gate: a finding stays only if its file exists in the project, its line is inside the file, and the evidence it quotes is in that file word
/// for word (whitespace aside). Everything else is dropped and counted on a `#dropped` line. Needs the model's `reviews`.
pub fn verify_findings(env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let mut kept = Vec::new();
    let mut unmatched = 0;
    let mut malformed = 0;
    let mut texts: BTreeMap<String, Option<(String, usize)>> = BTreeMap::new();

    for review in data.get("reviews").into_iter().flatten() {
        for line in review.lines().map(str::trim).filter(|line| !line.is_empty() && !line.to_uppercase().starts_with("NO FINDINGS")) {
            let Some([location, severity, category, problem, evidence]) = fields(line) else {
                malformed += 1;

                continue;
            };
            let Some((path, row)) = location.trim_matches('`').rsplit_once(':') else {
                malformed += 1;

                continue;
            };
            let path = path.trim().replace('\\', "/").trim_start_matches("./").to_string();
            let row: usize = row.split(['-', ' ']).next().unwrap_or("").parse().unwrap_or(0);
            let severity = if severity == "critical" { "high".to_string() } else { severity };

            if row == 0 || !["high", "medium", "low"].contains(&severity.as_str()) || path.split('/').any(|part| part == "..") {
                malformed += 1;

                continue;
            }

            let known = texts.entry(path.clone()).or_insert_with(|| read_text(env, &path).map(|text| (squash(&text), text.lines().count())));
            let quote = squash(strip_line_number(evidence.trim().trim_matches(|c| c == '`' || c == '"' || c == '\'')));

            match known {
                Some((text, count)) if row <= *count && quote.chars().count() >= 6 && text.contains(&quote) => kept.push(format!("{path}:{row} · {severity} · {category} · {problem} · {quote}")),
                _ => unmatched += 1,
            }
        }
    }

    kept.push(format!("#dropped {unmatched} whose evidence is not in the file, {malformed} not in the finding format"));

    Ok(kept)
}

/// Removes repeats (same file, line and category) and orders the findings high, medium, low, then by file and line. `#` lines are kept at the end.
pub fn rank_findings(_env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let lines = data.get("verified").cloned().unwrap_or_default();
    let weight = |line: &str| match line.split(" · ").nth(1) {
        Some("high") => 0,
        Some("medium") => 1,
        _ => 2,
    };
    let place = |line: &str| -> (String, usize) {
        let place = line.split(" · ").next().unwrap_or("");
        let (path, row) = place.rsplit_once(':').unwrap_or((place, "0"));

        (path.to_string(), row.parse().unwrap_or(0))
    };
    let mut findings: Vec<&String> = lines.iter().filter(|line| !line.starts_with('#')).collect();

    findings.sort_by(|a, b| weight(a).cmp(&weight(b)).then(place(a).cmp(&place(b))));

    let mut seen = BTreeSet::new();
    let mut ranked: Vec<String> = findings
        .into_iter()
        .filter(|line| {
            let mut field = line.split(" · ");
            let key = (field.next().unwrap_or("").to_string(), field.nth(1).unwrap_or("").to_lowercase());

            seen.insert(key)
        })
        .cloned()
        .collect();

    ranked.extend(lines.iter().filter(|line| line.starts_with('#')).cloned());

    Ok(ranked)
}

/// The report the person reads: counts, the findings by severity with their evidence, what was not reviewed and why, and which checkers ran.
/// Needs `ranked`, `errors`, `files`, `units` and `_unreviewed`. Built from the evidence-checked list only, so it cannot say more than was proved.
pub fn build_report(_env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let get = |name: &str| data.get(name).cloned().unwrap_or_default();
    let ranked = get("ranked");
    let findings: Vec<&String> = ranked.iter().filter(|line| !line.starts_with('#')).collect();
    let units = get("units");
    let unreviewed = get("_unreviewed");
    let planned: BTreeSet<&str> = units.iter().map(|unit| split_unit(unit).0).collect();
    let skipped = get("files").iter().filter(|file| !planned.contains(file.as_str())).count();
    let severity = |wanted: &str| findings.iter().filter(|line| line.split(" · ").nth(1) == Some(wanted)).count();
    let mut report = vec!["# Code review".to_string(), String::new()];

    report.push(format!(
        "{} of {} review units done, {} verified findings ({} high, {} medium, {} low). {skipped} files were not source text (binary, generated, lock files) and were skipped.",
        units.len().saturating_sub(unreviewed.len()),
        units.len(),
        findings.len(),
        severity("high"),
        severity("medium"),
        severity("low")
    ));

    if let Some(dropped) = ranked.iter().find(|line| line.starts_with("#dropped")) {
        report.push(format!("Dropped: {}.", dropped.trim_start_matches("#dropped ")));
    }

    for (title, wanted) in [("High", "high"), ("Medium", "medium"), ("Low", "low")] {
        let group: Vec<&&String> = findings.iter().filter(|line| line.split(" · ").nth(1) == Some(wanted)).collect();

        if group.is_empty() {
            continue;
        }

        report.push(String::new());
        report.push(format!("## {title}"));

        for (number, line) in group.iter().enumerate() {
            let mut field = line.split(" · ");
            let place = field.next().unwrap_or("");

            field.next();

            let category = field.next().unwrap_or("");
            let problem = field.next().unwrap_or("");
            let evidence = field.collect::<Vec<_>>().join(" · ");

            report.push(format!("{}. `{place}` [{category}] {problem}", number + 1));
            report.push(format!("   evidence: `{evidence}`"));
        }
    }

    if findings.is_empty() {
        report.push(String::new());
        report.push("No verified findings.".to_string());
    }

    if !unreviewed.is_empty() {
        report.push(String::new());
        report.push("## Not reviewed".to_string());
        report.extend(unreviewed.iter().map(|line| format!("- {line}")));
    }

    let checkers: Vec<String> = get("errors").iter().filter(|line| line.starts_with('#')).map(|line| format!("- {}", line.trim_start_matches('#'))).collect();

    if !checkers.is_empty() {
        report.push(String::new());
        report.push("## Checkers".to_string());
        report.extend(checkers);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("sdc-codefns-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src dir")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(root.join("src dir/a.rs"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(root.join("b.txt"), "no newline at the end").unwrap();
        std::fs::write(root.join("node_modules/x/i.js"), "x").unwrap();
        std::fs::write(root.join("target/t.o"), "x").unwrap();
        std::fs::write(root.join("dist/d.js"), "x").unwrap();
        std::fs::write(root.join("ignored.log"), "x").unwrap();
        std::fs::write(root.join(".ignore"), "ignored.log\n").unwrap();

        Env::new(root)
    }

    #[test]
    fn the_inventory_skips_what_is_never_reviewed_and_handles_a_space_in_a_path() {
        let env = project("list");
        let files = list_files(&env, None, &Data::new()).unwrap();

        assert!(files.contains(&"src dir/a.rs".to_string()) && files.contains(&"b.txt".to_string()), "{files:?}");
        assert!(!files.iter().any(|file| file.starts_with("node_modules") || file.starts_with("target") || file.starts_with("dist") || file == "ignored.log"), "{files:?}");

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn lines_are_counted_and_a_missing_file_says_so() {
        let env = project("count");

        assert_eq!(count_lines(&env, Some("src dir/a.rs"), &Data::new()).unwrap(), vec!["src dir/a.rs: 3 lines"]);
        assert_eq!(count_lines(&env, Some("b.txt"), &Data::new()).unwrap(), vec!["b.txt: 1 lines"]);
        assert!(count_lines(&env, Some("nope.txt"), &Data::new()).unwrap_err().contains("could not read nope.txt"));
        assert!(count_lines(&env, None, &Data::new()).is_err());

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn the_present_gate_names_the_empty_input() {
        let env = Env::new(PathBuf::new());
        let mut data = Data::new();

        data.insert("files".into(), vec!["a".into()]);
        assert_eq!(outputs_present(&env, None, &data).unwrap(), vec!["files: 1 items"]);

        data.insert("counts".into(), vec!["  ".into()]);
        assert_eq!(outputs_present(&env, None, &data).unwrap_err(), "counts is empty");
    }
}
