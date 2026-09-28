//! The agent's two finding tools (0.13): `grep` - a regular expression over the files' lines - and
//! `glob` - files by name pattern (`src/**/*.tsx`). Claude Code's Grep and Glob, on both machines.
//!
//! Until 0.13 the agent had `search`, a literal string, so "where is this function defined" or "every
//! file under components" took a dozen `list_dir` calls - each a step, each a round trip to the model.
//!
//! Both skip what is never the project (`.git`, `node_modules`, `target`, `dist`, `vendor`, build output)
//! and what the file guard hides (keys, `.env`), on this machine by walking the folder and on a host with
//! `grep -E` / `find` - the host's own tools, one round trip each.

use std::path::Path;
use std::time::Duration;

use regex::Regex;

use crate::sdcp::envelope::ErrorObject;
use crate::ssh::Ssh;

/// Folders no search goes into.
pub const SKIPPED_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", "vendor", ".next", ".nuxt", ".svelte-kit", ".pnpm-store", "__pycache__", ".venv", "venv", ".cache", "coverage", ".turbo"];

/// A file larger than this is not searched line by line (a bundle, a dump, a lock file).
const MAX_SEARCHED: u64 = 2 * 1024 * 1024;

/// A glob as a regular expression over a `/`-separated relative path.
///
/// `**` crosses folders, `*` and `?` do not, `{a,b}` is either. A pattern with no `/` matches the name
/// at any depth - `*.ts` is every TypeScript file, the way people write it.
pub fn glob_regex(pattern: &str) -> Result<Regex, ErrorObject> {
    let pattern = pattern.trim().trim_start_matches("./");
    let anywhere = !pattern.contains('/');
    let mut out = String::from(if anywhere { "^(?:.*/)?" } else { "^" });
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0;
    let mut in_braces = false;

    while index < chars.len() {
        let c = chars[index];

        match c {
            '*' if chars.get(index + 1) == Some(&'*') => {
                if chars.get(index + 2) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    index += 3;
                } else {
                    out.push_str(".*");
                    index += 2;
                }

                continue;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '{' => {
                in_braces = true;
                out.push_str("(?:");
            }
            '}' if in_braces => {
                in_braces = false;
                out.push(')');
            }
            ',' if in_braces => out.push('|'),
            other => out.push_str(&regex::escape(&other.to_string())),
        }

        index += 1;
    }

    out.push('$');

    Regex::new(&out).map_err(|error| ErrorObject::bad_request(format!("`{pattern}` is not a pattern I can read: {error}")))
}

/// Every file under `root` (relative, `/`-separated) with its modified time, skipped folders left out.
fn walk_files(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, std::time::SystemTime)>, cap: usize) {
    if depth == 0 || out.len() >= cap {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if out.len() >= cap {
            return;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if crate::fs::blocked_reason(&path).is_some() {
            continue;
        }

        let Ok(meta) = entry.metadata() else {
            continue;
        };

        if meta.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                walk_files(root, &path, depth - 1, out, cap);
            }
        } else if let Ok(relative) = path.strip_prefix(root) {
            out.push((relative.to_string_lossy().replace('\\', "/"), meta.modified().unwrap_or(std::time::UNIX_EPOCH)));
        }
    }
}

/// The files of a folder, on its machine, relative - at most `cap`, newest first.
fn files_of(root: &str, remote: Option<&Ssh>, cap: usize) -> Result<Vec<String>, ErrorObject> {
    match remote {
        Some(ssh) => {
            let expr = crate::ssh::ops::remote_expr(root)?;
            let prune = SKIPPED_DIRS.iter().map(|dir| format!("-name {}", crate::ssh::sh_quote(dir))).collect::<Vec<_>>().join(" -o ");
            /* `%T@ %P`: seconds and the path relative to the root - sorted newest first on the host. */
            let line = format!("cd {expr} && find . -mindepth 1 \\( {prune} \\) -prune -o -type f -printf '%T@ %P\\n' 2>/dev/null | sort -rn | head -n {cap}");
            let output = ssh.run(&line, Duration::from_secs(60))?;

            Ok(output
                .stdout
                .lines()
                .filter_map(|line| line.split_once(' ').map(|(_, path)| path.to_string()))
                .filter(|path| crate::fs::blocked_reason(Path::new(path)).is_none())
                .collect())
        }
        None => {
            let mut files = Vec::new();

            walk_files(Path::new(root), Path::new(root), 24, &mut files, cap);
            files.sort_by_key(|file| std::cmp::Reverse(file.1));

            Ok(files.into_iter().map(|(path, _)| path).collect())
        }
    }
}

/// `glob`: the files under `base` (relative to `root`) matching `pattern`, newest first, at most `limit`.
pub fn glob(root: &str, remote: Option<&Ssh>, base: &str, pattern: &str, limit: usize) -> Result<(Vec<String>, usize), ErrorObject> {
    let matcher = glob_regex(pattern)?;
    let base = base.trim().trim_start_matches("./").trim_end_matches('/');
    let base = if base == "." { "" } else { base };
    let files = files_of(root, remote, 50_000)?;
    let matching: Vec<String> = files
        .into_iter()
        .filter(|path| base.is_empty() || path.starts_with(&format!("{base}/")))
        .filter(|path| {
            let inner = if base.is_empty() { path.as_str() } else { &path[base.len() + 1..] };

            matcher.is_match(inner)
        })
        .collect();
    let total = matching.len();

    Ok((matching.into_iter().take(limit).collect(), total))
}

/// One `grep` hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// `grep`: lines matching a regular expression, under `base`, in files matching `glob`.
pub fn grep(root: &str, remote: Option<&Ssh>, base: &str, pattern: &str, glob: Option<&str>, ignore_case: bool, limit: usize) -> Result<Vec<Hit>, ErrorObject> {
    let regex = regex::RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|error| ErrorObject::bad_request(format!("`{pattern}` is not a valid regular expression: {error}")))?;
    let base = base.trim().trim_start_matches("./").trim_end_matches('/');
    let base = if base == "." { "" } else { base };

    match remote {
        Some(ssh) => {
            let dir = if base.is_empty() { root.to_string() } else { format!("{}/{base}", root.trim_end_matches('/')) };
            let expr = crate::ssh::ops::remote_expr(&dir)?;
            let excludes = SKIPPED_DIRS.iter().map(|dir| format!("--exclude-dir={}", crate::ssh::sh_quote(dir))).collect::<Vec<_>>().join(" ");
            let include = glob.map(|glob| format!(" --include={}", crate::ssh::sh_quote(glob.rsplit('/').next().unwrap_or(glob)))).unwrap_or_default();
            let line = format!(
                "cd {expr} && grep -rnIE{} {excludes}{include} -- {} . 2>/dev/null | head -n {limit}",
                if ignore_case { "i" } else { "" },
                crate::ssh::sh_quote(pattern)
            );
            let output = ssh.run(&line, Duration::from_secs(90))?;

            Ok(output
                .stdout
                .lines()
                .filter_map(|line| {
                    let line = line.strip_prefix("./").unwrap_or(line);
                    let (path, rest) = line.split_once(':')?;
                    let (number, text) = rest.split_once(':')?;
                    let path = if base.is_empty() { path.to_string() } else { format!("{base}/{path}") };

                    (crate::fs::blocked_reason(Path::new(&path)).is_none()).then(|| Hit { path, line: number.parse().unwrap_or(0), text: clip(text) })
                })
                .collect())
        }
        None => {
            let matcher = glob.map(glob_regex).transpose()?;
            let mut hits = Vec::new();

            for path in files_of(root, None, 50_000)? {
                if hits.len() >= limit {
                    break;
                }

                if !base.is_empty() && !path.starts_with(&format!("{base}/")) {
                    continue;
                }

                if matcher.as_ref().is_some_and(|matcher| !matcher.is_match(&path)) {
                    continue;
                }

                let full = Path::new(root).join(&path);

                if std::fs::metadata(&full).map(|meta| meta.len() > MAX_SEARCHED).unwrap_or(true) {
                    continue;
                }

                let Ok(bytes) = std::fs::read(&full) else {
                    continue;
                };

                /* A NUL in the first kilobytes is a binary file - an image, a font, a build artifact. */
                if bytes.iter().take(8000).any(|byte| *byte == 0) {
                    continue;
                }

                let text = String::from_utf8_lossy(&bytes);

                for (number, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        hits.push(Hit { path: path.clone(), line: number + 1, text: clip(line) });

                        if hits.len() >= limit {
                            break;
                        }
                    }
                }
            }

            Ok(hits)
        }
    }
}

/// A matching line, trimmed and cut: a minified bundle's one line is not a search result.
fn clip(line: &str) -> String {
    let trimmed = line.trim();

    if trimmed.chars().count() > 300 {
        format!("{}…", trimmed.chars().take(300).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_read_the_way_people_write_them() {
        let ts = glob_regex("*.ts").unwrap();

        assert!(ts.is_match("a.ts"));
        assert!(ts.is_match("src/deep/a.ts"));
        assert!(!ts.is_match("a.tsx"));

        let deep = glob_regex("src/**/*.{tsx,ts}").unwrap();

        assert!(deep.is_match("src/a.tsx"));
        assert!(deep.is_match("src/x/y/b.ts"));
        assert!(!deep.is_match("lib/a.ts"));

        let one = glob_regex("src/*.rs").unwrap();

        assert!(one.is_match("src/main.rs"));
        assert!(!one.is_match("src/agent/mod.rs"));
    }

    #[test]
    fn grep_and_glob_find_the_project_and_skip_the_rest() {
        let root = std::env::temp_dir().join(format!("sdc-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/api")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("src/api/pay.ts"), "export function chargeCard(amount) {\n  return amount;\n}\n").unwrap();
        std::fs::write(root.join("src/index.ts"), "import { chargeCard } from './api/pay';\n").unwrap();
        std::fs::write(root.join("node_modules/x/index.ts"), "function chargeCard() {}\n").unwrap();
        std::fs::write(root.join(".env"), "KEY=function chargeCard\n").unwrap();

        let base = root.to_str().unwrap();
        let hits = grep(base, None, ".", r"function\s+charge\w+", None, false, 50).unwrap();

        assert_eq!(hits, vec![Hit { path: "src/api/pay.ts".into(), line: 1, text: "export function chargeCard(amount) {".into() }]);

        let (files, total) = glob(base, None, ".", "**/*.ts", 10).unwrap();

        assert_eq!(total, 2);
        assert!(files.contains(&"src/index.ts".to_string()) && files.contains(&"src/api/pay.ts".to_string()));

        let (files, _) = glob(base, None, "src/api", "*.ts", 10).unwrap();

        assert_eq!(files, vec!["src/api/pay.ts".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }
}
