//! **N3-3 - the Error Slicer** (switch `nr.error_slice`).
//!
//! A build or test failure is 200 lines of noise around one real error. The slicer finds the first error of a rustc/cargo, tsc, eslint,
//! pytest, jest or go log - its message, its `file:line` and the code ten lines either side - and keeps the whole log in the vault, where
//! `spec_read` can fetch any part of it. It never hides that there were more errors: the count is appended.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Rustc,
    Tsc,
    Eslint,
    Pytest,
    Jest,
    Go,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Rustc => "rustc",
            Tool::Tsc => "tsc",
            Tool::Eslint => "eslint",
            Tool::Pytest => "pytest",
            Tool::Jest => "jest",
            Tool::Go => "go",
        }
    }
}

/// One failure, found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub tool: Tool,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    /// How many failures the log held in all.
    pub total: usize,
}

/// What the model is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    pub found: Found,
    pub text: String,
    /// The vault id of the whole log.
    pub log_id: Option<String>,
}

/// Which tool wrote a log, from its own markers.
pub fn detect(log: &str) -> Option<Tool> {
    if log.contains("error[E") || (log.contains("error: ") && log.contains(" --> ")) || log.contains("could not compile") {
        return Some(Tool::Rustc);
    }

    if log.contains("error TS") {
        return Some(Tool::Tsc);
    }

    if log.contains("● ") && (log.contains("Expected:") || log.contains("expect(") || log.contains("Test Suites:")) {
        return Some(Tool::Jest);
    }

    if log.contains("=== FAILURES ===") || log.contains("short test summary info") || log.lines().any(|l| l.starts_with("E   ") || l.starts_with("E       ")) {
        return Some(Tool::Pytest);
    }

    if log.lines().any(|l| l.contains("problems (") || (l.split_whitespace().nth(1).is_some_and(|w| w == "error" || w == "warning") && l.contains(':'))) && log.lines().any(|l| l.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit()) && l.contains("error")) {
        return Some(Tool::Eslint);
    }

    if log.contains("--- FAIL:") || log.lines().any(|l| l.contains(".go:") && !l.contains("--> ")) {
        return Some(Tool::Go);
    }

    None
}

fn split_location(text: &str) -> Option<(String, usize)> {
    /* `path:line:col` or `path:line` */
    let parts: Vec<&str> = text.trim().rsplitn(3, ':').collect();
    let numeric = |s: &str| s.trim().parse::<usize>().ok();

    if parts.len() == 3 {
        if let (Some(_), Some(line)) = (numeric(parts[0]), numeric(parts[1])) {
            return Some((parts[2].to_string(), line));
        }
    }

    if parts.len() >= 2 {
        let line = numeric(parts[0])?;
        let file = text.trim().strip_suffix(&format!(":{}", parts[0]))?;

        return Some((file.to_string(), line));
    }

    None
}

fn rustc(log: &str) -> Option<Found> {
    let lines: Vec<&str> = log.lines().collect();
    let is_error = |l: &str| l.starts_with("error") && !l.starts_with("error: aborting") && !l.starts_with("error: could not compile") && !l.starts_with("error: Recipe") && !l.starts_with("error: test failed");
    let total = lines.iter().filter(|l| is_error(l)).count();
    let at = lines.iter().position(|l| is_error(l))?;
    let message = lines[at].trim().to_string();
    let (file, line) = lines[at + 1..].iter().take(6).find_map(|l| l.trim_start().strip_prefix("--> ").and_then(split_location)).map_or((None, None), |(f, n)| (Some(f), Some(n)));

    Some(Found { tool: Tool::Rustc, message, file, line, expected: None, actual: None, total: total.max(1) })
}

fn tsc(log: &str) -> Option<Found> {
    let errors: Vec<&str> = log.lines().filter(|l| l.contains("error TS")).collect();
    let first = errors.first()?;
    /* `src/a.ts(10,5): error TS2322: msg`  or  `src/a.ts:10:5 - error TS2322: msg` */
    let (location, message) = first.split_once(": error ").or_else(|| first.split_once(" - error "))?;
    let (file, line) = if let Some((file, rest)) = location.split_once('(') {
        (file.to_string(), rest.split(',').next()?.parse().ok()?)
    } else {
        split_location(location)?
    };

    Some(Found { tool: Tool::Tsc, message: format!("error {message}"), file: Some(file), line: Some(line), expected: None, actual: None, total: errors.len() })
}

fn eslint(log: &str) -> Option<Found> {
    let mut file = None;
    let mut found: Vec<(String, usize, String)> = Vec::new();

    for line in log.lines() {
        let trimmed = line.trim_start();

        if !line.starts_with(' ') && !trimmed.is_empty() && (trimmed.contains('/') || trimmed.contains('.')) && !trimmed.contains("problems") {
            file = Some(trimmed.to_string());
            continue;
        }

        let mut words = trimmed.split_whitespace();
        let position = words.next().unwrap_or_default();
        let severity = words.next().unwrap_or_default();

        if severity == "error" {
            if let (Some(file), Some(n)) = (file.clone(), position.split(':').next().and_then(|n| n.parse().ok())) {
                found.push((file, n, words.collect::<Vec<_>>().join(" ")));
            }
        }
    }

    let (file, line, message) = found.first()?.clone();

    Some(Found { tool: Tool::Eslint, message, file: Some(file), line: Some(line), expected: None, actual: None, total: found.len() })
}

fn pytest(log: &str) -> Option<Found> {
    let lines: Vec<&str> = log.lines().collect();
    let failed = lines.iter().filter(|l| l.starts_with("FAILED ") || l.starts_with("ERROR ")).count();
    let first_e = lines.iter().position(|l| l.starts_with("E  "))?;
    let mut message_lines = Vec::new();

    for line in &lines[first_e..] {
        if let Some(rest) = line.strip_prefix('E') {
            message_lines.push(rest.trim().to_string());
        } else {
            break;
        }
    }

    let message = message_lines.join(" | ");
    /* the location line right after: `tests/test_a.py:12: AssertionError` */
    let (file, line) = lines[first_e..].iter().take(message_lines.len() + 3).find_map(|l| {
        let head = l.split_whitespace().next()?;

        head.strip_suffix(':').and_then(|h| h.rsplit_once(':')).and_then(|(f, n)| Some((f.to_string(), n.parse::<usize>().ok()?)))
    }).map_or((None, None), |(f, n)| (Some(f), Some(n)));
    /* `assert 3 == 4` or `AssertionError: assert 3 == 4` */
    let (expected, actual) = message_lines.iter().find_map(|m| {
        let tail = m.split("assert ").nth(1)?;
        let (left, right) = tail.split_once(" == ")?;

        Some((right.trim().to_string(), left.trim().to_string()))
    }).map_or((None, None), |(e, a)| (Some(e), Some(a)));

    Some(Found { tool: Tool::Pytest, message, file, line, expected, actual, total: failed.max(1) })
}

fn jest(log: &str) -> Option<Found> {
    let lines: Vec<&str> = log.lines().collect();
    let titles = lines.iter().filter(|l| l.trim_start().starts_with("● ") && !l.contains("Console")).count();
    let first = lines.iter().position(|l| l.trim_start().starts_with("● ") && !l.contains("Console"))?;
    let title = lines[first].trim().trim_start_matches("● ").to_string();
    let block = &lines[first..lines.len().min(first + 40)];
    let expected = block.iter().find_map(|l| l.trim().strip_prefix("Expected:").map(|v| v.trim().to_string()));
    let actual = block.iter().find_map(|l| l.trim().strip_prefix("Received:").map(|v| v.trim().to_string()));
    let assertion = block.iter().find(|l| l.trim().starts_with("expect(")).map(|l| l.trim().to_string());
    let (file, line) = block.iter().find_map(|l| {
        let inner = l.trim().strip_prefix("at ")?;
        let inner = inner.rsplit_once('(').map_or(inner, |(_, rest)| rest).trim_end_matches(')');

        split_location(inner)
    }).map_or((None, None), |(f, n)| (Some(f), Some(n)));

    Some(Found { tool: Tool::Jest, message: format!("{title}{}", assertion.map(|a| format!(": {a}")).unwrap_or_default()), file, line, expected, actual, total: titles.max(1) })
}

fn go(log: &str) -> Option<Found> {
    let lines: Vec<&str> = log.lines().collect();
    let fails = lines.iter().filter(|l| l.trim_start().starts_with("--- FAIL:")).count();
    let compile: Vec<&str> = lines.iter().copied().filter(|l| l.contains(".go:") && l.split_whitespace().next().is_some_and(|w| w.ends_with(':') || w.contains(".go:"))).collect();
    let first = compile.first()?;
    let first = first.trim();
    let (location, message) = first.split_once(": ")?;
    let (file, line) = split_location(location)?;
    /* a test line: `main_test.go:12: got 1, want 2` */
    let (expected, actual) = message.split_once("want ").map_or((None, None), |(got, want)| (Some(want.trim().to_string()), got.rsplit_once("got ").map(|(_, g)| g.trim().trim_end_matches(',').to_string())));

    Some(Found { tool: Tool::Go, message: message.to_string(), file: Some(file.trim_start_matches("./").to_string()), line: Some(line), expected, actual, total: compile.len().max(fails).max(1) })
}

/// The first failure of a log of `tool` (`None` when the log holds none that this reader knows).
pub fn find(tool: Tool, log: &str) -> Option<Found> {
    match tool {
        Tool::Rustc => rustc(log),
        Tool::Tsc => tsc(log),
        Tool::Eslint => eslint(log),
        Tool::Pytest => pytest(log),
        Tool::Jest => jest(log),
        Tool::Go => go(log),
    }
}

/// `file:line` and `radius` lines of code each side, numbered; the line itself is marked with `>`.
pub fn code_around(source: &str, line: usize, radius: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let from = line.saturating_sub(radius).max(1);
    let to = (line + radius).min(lines.len());

    (from..=to).map(|n| format!("{}{:>4} | {}", if n == line { ">" } else { " " }, n, lines.get(n - 1).copied().unwrap_or_default())).collect::<Vec<_>>().join("\n")
}

/// The slice a model is shown for a log: message, location, code around it, expected against actual, and a note of how many more there were
/// and where the whole log is. `cap_tokens` bounds it; the code is trimmed first, never the location.
pub fn slice(root: &Path, log: &str, tool: Option<Tool>, cap_tokens: u64) -> Option<Slice> {
    let tool = tool.or_else(|| detect(log))?;
    let found = find(tool, log)?;
    let log_id = super::vault::put(root, "log", log).ok();
    let mut head = format!("{} error: {}", tool.name(), found.message.trim());

    if let (Some(file), Some(line)) = (&found.file, found.line) {
        head.push_str(&format!("\n at {file}:{line}"));
    }

    if let (Some(expected), Some(actual)) = (&found.expected, &found.actual) {
        head.push_str(&format!("\n expected: {expected}\n actual: {actual}"));
    }

    let mut tail = String::new();

    if found.total > 1 {
        tail.push_str(&format!("\n(+{} more error{}", found.total - 1, if found.total == 2 { "" } else { "s" }));
    } else {
        tail.push_str("\n(");
    }

    tail.push_str(&match &log_id {
        Some(id) => format!("; the whole log is in the vault: spec_read log {id})"),
        None => ")".to_string(),
    });
    tail = tail.replacen("(;", "(", 1);

    let source = found.file.as_ref().and_then(|file| std::fs::read_to_string(root.join(file)).ok());
    let mut radius = 10usize;

    loop {
        let code = match (&source, found.line) {
            (Some(source), Some(line)) => format!("\n{}", code_around(source, line, radius)),
            _ => String::new(),
        };
        let text = format!("{head}{code}{tail}");

        if radius == 0 || crate::tokens::estimate(&text) <= cap_tokens {
            return Some(Slice { found, text, log_id });
        }

        radius = if radius > 2 { radius / 2 } else { 0 };
    }
}

#[cfg(test)]
mod tests;
