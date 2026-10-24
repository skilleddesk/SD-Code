//! **Does the API the edit just used exist? (M4-1, switch `verify.api_exists`, on, a warning only)**: a model that writes `from requests import
//! fetch_json` or `import { useFoo } from 'react-query'` may be inventing a name. After an edit, the imports of the new text are looked up in the
//! dependencies that are actually installed - `node_modules`, a virtual environment's `site-packages`, a Rust crate's source in `vendor/` or the
//! cargo registry - with no model and no network.
//!
//! The answer is one of three things, and the third is the honest one:
//!
//! * the module and the names are there - nothing is said;
//! * the module or a name is **not** there - a warning with the nearest real names ("did you mean ..."), returned with the edit's answer so the
//!   model can fix it. Nothing is blocked: the check reads files, it does not run the code;
//! * the dependencies are not installed, or their source is not on this machine - "could not be checked", never "fine".

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use regex::Regex;

/// How many files of one package are read looking for a name, and how many bytes in all.
const MAX_FILES: usize = 400;
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// One imported module and the names taken from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub module: String,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Found,
    /// No such package in the installed dependencies; the nearest names.
    ModuleMissing { module: String, near: Vec<String> },
    /// The package is there and does not declare this name; the nearest names it does.
    NameMissing { module: String, name: String, near: Vec<String> },
    /// There was nothing to look in.
    Unchecked { module: String, why: String },
}

fn language(path: &str) -> Option<&'static str> {
    match path.rsplit('.').next()?.to_lowercase().as_str() {
        "py" => Some("python"),
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" => Some("js"),
        "rs" => Some("rust"),
        _ => None,
    }
}

/// The third-party-looking imports of a text. Relative paths, aliases and the language's own standard library are left to [`check`].
pub fn imports_of(path: &str, text: &str) -> Vec<Import> {
    match language(path) {
        Some("python") => python_imports(text),
        Some("js") => js_imports(text),
        Some("rust") => rust_imports(text),
        _ => Vec::new(),
    }
}

fn names_of(list: &str) -> Vec<String> {
    list.split(',')
        .map(|part| {
            let mut words = part.split_whitespace();
            let first = words.next().unwrap_or_default();

            /* `type FC` imports the name FC. */
            (if first == "type" { words.next().unwrap_or_default() } else { first }).to_string()
        })
        .map(|name| name.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '$').to_string())
        .filter(|name| !name.is_empty() && name != "*")
        .collect()
}

fn python_imports(text: &str) -> Vec<Import> {
    let from = Regex::new(r"(?m)^\s*from\s+([A-Za-z_][\w.]*)\s+import\s+\(?([^)\n#]+)").expect("a valid pattern");
    let plain = Regex::new(r"(?m)^\s*import\s+([A-Za-z_][\w.]*)").expect("a valid pattern");
    let mut found = Vec::new();

    for capture in from.captures_iter(text) {
        let names: Vec<String> = capture[2].split(',').map(|part| part.split_whitespace().next().unwrap_or_default().to_string()).filter(|name| !name.is_empty() && name != "*").collect();

        found.push(Import { module: capture[1].to_string(), names });
    }

    for capture in plain.captures_iter(text) {
        found.push(Import { module: capture[1].to_string(), names: Vec::new() });
    }

    found
}

fn js_imports(text: &str) -> Vec<Import> {
    let named = Regex::new(r#"import\s+(?:[\w$]+\s*,\s*)?\{([^}]*)\}\s*from\s*['"]([^'"]+)['"]"#).expect("a valid pattern");
    let bare = Regex::new(r#"(?:import\s+(?:[\w$*\s]+\s+from\s+)?|require\(\s*)['"]([^'"]+)['"]"#).expect("a valid pattern");
    let mut found = Vec::new();

    for capture in named.captures_iter(text) {
        found.push(Import { module: capture[2].to_string(), names: names_of(&capture[1]) });
    }

    for capture in bare.captures_iter(text) {
        if !found.iter().any(|import: &Import| import.module == capture[1]) {
            found.push(Import { module: capture[1].to_string(), names: Vec::new() });
        }
    }

    found
}

fn rust_imports(text: &str) -> Vec<Import> {
    let uses = Regex::new(r"(?m)^\s*(?:pub\s+)?use\s+([a-z_][a-z0-9_]*)::([^;]+);").expect("a valid pattern");
    let mut found = Vec::new();

    for capture in uses.captures_iter(text) {
        if matches!(&capture[1], "std" | "core" | "alloc" | "crate" | "self" | "super") {
            continue;
        }

        let tail = capture[2].trim();
        let names: Vec<String> = match tail.split_once('{') {
            Some((_, inside)) => names_of(inside.trim_end_matches('}')),
            None => tail.rsplit("::").next().map(|last| vec![last.split_whitespace().next().unwrap_or_default().to_string()]).unwrap_or_default(),
        };

        found.push(Import { module: capture[1].to_string(), names: names.into_iter().filter(|name| name != "self" && name != "*").collect() });
    }

    found
}

// ---------------------------------------------------------------------------------------------------- near names

fn distance(left: &str, right: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (left.chars().collect(), right.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();

    for (i, x) in a.iter().enumerate() {
        let mut previous = row[0];

        row[0] = i + 1;

        for (j, y) in b.iter().enumerate() {
            let next = (previous + usize::from(x != y)).min(row[j] + 1).min(row[j + 1] + 1);

            previous = row[j + 1];
            row[j + 1] = next;
        }
    }

    row[b.len()]
}

/// The candidates within two edits of `wanted` (or containing it), nearest first, at most three.
pub fn nearest(wanted: &str, candidates: &BTreeSet<String>) -> Vec<String> {
    let mut near: Vec<(usize, &String)> = candidates
        .iter()
        .filter(|candidate| candidate.as_str() != wanted)
        .map(|candidate| (distance(&wanted.to_lowercase(), &candidate.to_lowercase()), candidate))
        .filter(|(edits, candidate)| *edits <= 2 || (wanted.len() >= 4 && candidate.to_lowercase().contains(&wanted.to_lowercase())))
        .collect();

    near.sort();
    near.into_iter().take(3).map(|(_, name)| name.clone()).collect()
}

// ---------------------------------------------------------------------------------------------------- where the code is

/// Source files of a package folder worth reading for declarations, within the caps.
fn package_files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0usize;

    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.') || name == "node_modules" || name == "tests" || name == "test" || name == "__pycache__" || name == "target" {
                continue;
            }

            if path.is_dir() {
                stack.push(path);
            } else if extensions.iter().any(|extension| name.ends_with(extension)) {
                files.push(path);
                seen += 1;

                if seen >= MAX_FILES {
                    return files;
                }
            }
        }
    }

    files
}

/// Every identifier a package declares, by the declaration forms of its language.
fn declared(files: &[PathBuf], pattern: &Regex) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut bytes = 0usize;

    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };

        bytes += text.len();

        for capture in pattern.captures_iter(&text) {
            for group in capture.iter().skip(1).flatten() {
                names.insert(group.as_str().to_string());
            }
        }

        if bytes > MAX_BYTES {
            break;
        }
    }

    names
}

fn js_package(module: &str) -> Option<String> {
    if module.starts_with('.') || module.starts_with('/') || module.starts_with("node:") || module.starts_with("@/") || module.starts_with('~') || module.starts_with('#') {
        return None;
    }

    let mut parts = module.split('/');
    let first = parts.next()?;

    Some(if first.starts_with('@') { format!("{first}/{}", parts.next()?) } else { first.to_string() })
}

const NODE_BUILTINS: &[&str] = &[
    "assert", "buffer", "child_process", "cluster", "console", "crypto", "dgram", "dns", "events", "fs", "http", "http2", "https", "net", "os", "path", "perf_hooks", "process", "querystring", "readline", "stream", "string_decoder", "timers", "tls", "tty", "url", "util", "v8", "vm", "worker_threads", "zlib",
];

fn node_modules_above(from: &Path) -> Option<PathBuf> {
    from.ancestors().map(|dir| dir.join("node_modules")).find(|dir| dir.is_dir())
}

fn check_js(root: &Path, file: &Path, import: &Import) -> Verdict {
    let Some(package) = js_package(&import.module) else {
        return Verdict::Found;
    };

    if NODE_BUILTINS.contains(&package.as_str()) {
        return Verdict::Found;
    }

    let start = file.parent().unwrap_or(root);
    let Some(modules) = node_modules_above(start).or_else(|| node_modules_above(root)) else {
        return Verdict::Unchecked { module: import.module.clone(), why: "node_modules is not installed here".into() };
    };
    let folder = modules.join(&package);

    if !folder.is_dir() {
        /* A workspace package or a path alias resolves elsewhere: only a name close to an installed one is worth a warning. */
        let installed: BTreeSet<String> = std::fs::read_dir(&modules).map(|dir| dir.flatten().filter_map(|entry| entry.file_name().to_str().map(str::to_string)).filter(|name| !name.starts_with('.') && !name.starts_with('@')).collect()).unwrap_or_default();
        let near = nearest(&package, &installed);

        return if near.is_empty() { Verdict::Unchecked { module: import.module.clone(), why: format!("`{package}` is not in node_modules (it may be a workspace package or an alias)") } } else { Verdict::ModuleMissing { module: import.module.clone(), near } };
    }

    if import.names.is_empty() {
        return Verdict::Found;
    }

    let files = package_files(&folder, &[".d.ts", ".js", ".mjs", ".cjs", ".ts"]);
    let pattern = Regex::new(r"(?:export\s+(?:declare\s+)?(?:default\s+)?(?:async\s+)?(?:function\*?|class|const|let|var|type|interface|enum|namespace)\s+([A-Za-z_$][\w$]*)|exports\.([A-Za-z_$][\w$]*)\s*=|export\s*\{([^}]*)\})").expect("a valid pattern");
    let mut names = declared(&files, &pattern);

    /* `export { a, b as c }` lists: split them. */
    let lists = Regex::new(r"export\s*\{([^}]*)\}").expect("a valid pattern");

    for file in &files {
        if let Ok(text) = std::fs::read_to_string(file) {
            for capture in lists.captures_iter(&text) {
                for part in capture[1].split(',') {
                    if let Some(name) = part.rsplit(" as ").next().map(str::trim).filter(|name| !name.is_empty()) {
                        names.insert(name.to_string());
                    }
                }
            }
        }
    }

    if names.is_empty() {
        return Verdict::Unchecked { module: import.module.clone(), why: "no declarations could be read from the package".into() };
    }

    match import.names.iter().find(|name| !names.contains(*name)) {
        Some(missing) => Verdict::NameMissing { module: import.module.clone(), name: missing.clone(), near: nearest(missing, &names) },
        None => Verdict::Found,
    }
}

const PYTHON_STDLIB: &[&str] = &[
    "abc", "argparse", "ast", "asyncio", "base64", "bisect", "builtins", "calendar", "collections", "concurrent", "configparser", "contextlib", "copy", "csv", "ctypes", "dataclasses", "datetime", "decimal", "difflib", "email", "enum", "errno", "fnmatch", "fractions", "functools", "gc", "getpass", "glob", "gzip", "hashlib", "heapq", "hmac", "html", "http", "importlib", "inspect", "io", "ipaddress", "itertools", "json", "logging", "math", "mimetypes", "multiprocessing", "numbers", "operator", "os", "pathlib", "pickle", "platform", "pprint", "queue", "random", "re", "secrets", "select", "shlex", "shutil", "signal", "socket", "sqlite3", "ssl", "stat", "statistics", "string", "struct", "subprocess", "sys", "tempfile", "textwrap", "threading", "time", "timeit", "tkinter", "traceback", "types", "typing", "unicodedata", "unittest", "urllib", "uuid", "warnings", "weakref", "xml", "zipfile", "zlib", "zoneinfo", "__future__",
];

fn site_packages(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();

    for venv in [".venv", "venv", "env"] {
        let lib = root.join(venv).join("lib");

        if let Ok(entries) = std::fs::read_dir(&lib) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("site-packages");

                if candidate.is_dir() {
                    found.push(candidate);
                }
            }
        }

        let windows = root.join(venv).join("Lib").join("site-packages");

        if windows.is_dir() {
            found.push(windows);
        }
    }

    found
}

fn check_python(root: &Path, file: &Path, import: &Import) -> Verdict {
    let top = import.module.split('.').next().unwrap_or_default();

    if top.is_empty() || PYTHON_STDLIB.contains(&top) {
        return Verdict::Found;
    }

    /* The project's own modules: next to the file, or at the root. */
    let local = |base: &Path| base.join(top).is_dir() || base.join(format!("{top}.py")).is_file();

    if local(file.parent().unwrap_or(root)) || local(root) || local(&root.join("src")) {
        return Verdict::Found;
    }

    let places = site_packages(root);

    if places.is_empty() {
        return Verdict::Unchecked { module: import.module.clone(), why: "no virtual environment (.venv, venv) with installed packages was found".into() };
    }

    let Some(base) = places.iter().find(|place| place.join(top).is_dir() || place.join(format!("{top}.py")).is_file()) else {
        let installed: BTreeSet<String> = places.iter().flat_map(|place| std::fs::read_dir(place).into_iter().flatten().flatten()).filter_map(|entry| entry.file_name().to_str().map(str::to_string)).map(|name| name.trim_end_matches(".py").to_string()).filter(|name| !name.contains('-') && !name.starts_with('_')).collect();

        return Verdict::ModuleMissing { module: import.module.clone(), near: nearest(top, &installed) };
    };

    if import.names.is_empty() {
        return Verdict::Found;
    }

    let target = base.join(top);
    let files = if target.is_dir() { package_files(&target, &[".py", ".pyi"]) } else { vec![base.join(format!("{top}.py"))] };
    let pattern = Regex::new(r"(?m)^\s*(?:async\s+)?(?:def|class)\s+([A-Za-z_]\w*)|^([A-Za-z_]\w*)\s*(?::[^=\n]+)?=|^from\s+\S+\s+import\s+\(?([^)\n]+)|^__all__\s*=\s*\[([^\]]*)\]").expect("a valid pattern");
    let mut names = declared(&files, &pattern);

    /* `from .x import a, b` and `__all__` lists name things the package re-exports. */
    let split: BTreeSet<String> = names.iter().flat_map(|raw| raw.split(',').map(|part| part.trim().trim_matches(['\'', '"']).split_whitespace().last().unwrap_or_default().to_string()).collect::<Vec<_>>()).filter(|name| !name.is_empty()).collect();

    names.extend(split);

    /* A submodule is a name too: `from pkg import submodule`. */
    if let Ok(entries) = std::fs::read_dir(&target) {
        for entry in entries.flatten() {
            names.insert(entry.file_name().to_string_lossy().trim_end_matches(".py").to_string());
        }
    }

    match import.names.iter().find(|name| !names.contains(*name)) {
        Some(missing) => Verdict::NameMissing { module: import.module.clone(), name: missing.clone(), near: nearest(missing, &names) },
        None => Verdict::Found,
    }
}

/// The crate names a `Cargo.toml` lists as dependencies (`-` as `_`, which is how the code spells them).
fn cargo_dependencies(root: &Path) -> BTreeSet<String> {
    let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return BTreeSet::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return BTreeSet::new();
    };
    let mut names = BTreeSet::new();

    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(deps) = value.get(table).and_then(toml::Value::as_table) {
            names.extend(deps.keys().map(|name| name.replace('-', "_")));
        }
    }

    names
}

fn crate_source(root: &Path, krate: &str) -> Option<PathBuf> {
    let dashed = krate.replace('_', "-");
    let mut roots = vec![root.join("vendor")];

    if let Some(home) = dirs::home_dir() {
        let registry = home.join(".cargo").join("registry").join("src");

        if let Ok(indexes) = std::fs::read_dir(&registry) {
            roots.extend(indexes.flatten().map(|entry| entry.path()));
        }
    }

    roots.iter().find_map(|base| {
        std::fs::read_dir(base).ok()?.flatten().map(|entry| entry.path()).find(|path| {
            path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name == krate || name == dashed || name.starts_with(&format!("{krate}-")) || name.starts_with(&format!("{dashed}-")))
        })
    })
}

fn check_rust(root: &Path, import: &Import) -> Verdict {
    let dependencies = cargo_dependencies(root);

    if !dependencies.contains(&import.module) {
        /* A module of the crate itself, or a workspace crate: not a third-party name to check. */
        return Verdict::Found;
    }

    let Some(source) = crate_source(root, &import.module) else {
        return Verdict::Unchecked { module: import.module.clone(), why: "the crate's source is not downloaded (no vendor/ and nothing in the cargo registry)".into() };
    };

    if import.names.is_empty() {
        return Verdict::Found;
    }

    let files = package_files(&source, &[".rs"]);
    let pattern = Regex::new(r"(?m)\bpub(?:\([a-z]+\))?\s+(?:unsafe\s+)?(?:async\s+)?(?:const\s+)?(?:fn|struct|enum|trait|type|const|static|mod|union)\s+([A-Za-z_]\w*)|\bmacro_rules!\s+([A-Za-z_]\w*)|\bpub\s+use\s+[^;{]*::([A-Za-z_]\w*)\s*;|\bpub\s+use\s+[^;]*\{([^}]*)\}").expect("a valid pattern");
    let mut names = declared(&files, &pattern);
    let split: BTreeSet<String> = names.iter().flat_map(|raw| raw.split(',').map(|part| part.trim().rsplit(" as ").next().unwrap_or_default().trim().to_string()).collect::<Vec<_>>()).filter(|name| !name.is_empty()).collect();

    names.extend(split);

    if names.is_empty() {
        return Verdict::Unchecked { module: import.module.clone(), why: "no declarations could be read from the crate".into() };
    }

    match import.names.iter().find(|name| !names.contains(*name)) {
        Some(missing) => Verdict::NameMissing { module: import.module.clone(), name: missing.clone(), near: nearest(missing, &names) },
        None => Verdict::Found,
    }
}

/// Checks every import of an edited file against the installed dependencies. `path` is relative to `root`.
pub fn check(root: &Path, path: &str, text: &str) -> Vec<Verdict> {
    let file = root.join(path);

    imports_of(path, text)
        .iter()
        .map(|import| match language(path) {
            Some("js") => check_js(root, &file, import),
            Some("python") => check_python(root, &file, import),
            Some("rust") => check_rust(root, import),
            _ => Verdict::Found,
        })
        .collect()
}

/// The note for the model: what does not exist (with the nearest real names), and what could not be checked. `None` when all is well.
pub fn warning(verdicts: &[Verdict]) -> Option<String> {
    let mut lines = Vec::new();
    let mut unchecked: BTreeSet<String> = BTreeSet::new();

    for verdict in verdicts {
        match verdict {
            Verdict::Found => {}
            Verdict::ModuleMissing { module, near } => lines.push(format!("`{module}` is not among the installed dependencies{}.", if near.is_empty() { String::new() } else { format!(" - did you mean {}?", near.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")) })),
            Verdict::NameMissing { module, name, near } => lines.push(format!("`{name}` is not declared by `{module}`{}.", if near.is_empty() { String::new() } else { format!(" - did you mean {}?", near.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")) })),
            Verdict::Unchecked { why, .. } => {
                unchecked.insert(why.clone());
            }
        }
    }

    if lines.is_empty() && unchecked.is_empty() {
        return None;
    }

    let mut text = String::new();

    if !lines.is_empty() {
        text.push_str(&format!("[SDC check - a warning, nothing was blocked] This edit uses names the installed dependencies do not have:\n- {}\nCheck the package's own documentation or source before relying on them.", lines.join("\n- ")));
    }

    if !unchecked.is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }

        text.push_str(&format!("[SDC check] Some imports could not be verified: {}.", unchecked.into_iter().collect::<Vec<_>>().join("; ")));
    }

    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-api-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        root
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_imports_of_three_languages_are_read() {
        assert_eq!(
            imports_of("a.py", "import os\nfrom requests import get, Session as S\nimport numpy.linalg\n"),
            [
                Import { module: "requests".into(), names: vec!["get".into(), "Session".into()] },
                Import { module: "os".into(), names: vec![] },
                Import { module: "numpy.linalg".into(), names: vec![] },
            ]
        );
        assert_eq!(
            imports_of("a.tsx", "import React, { useState, type FC } from 'react';\nimport x from \"./local\";\nconst y = require('lodash');\n"),
            [Import { module: "react".into(), names: vec!["useState".into(), "FC".into()] }, Import { module: "./local".into(), names: vec![] }, Import { module: "lodash".into(), names: vec![] }]
        );
        assert_eq!(imports_of("a.rs", "use std::fs;\nuse serde::{Deserialize, Serialize};\nuse regex::Regex;\nuse crate::x::Y;\n"), [Import { module: "serde".into(), names: vec!["Deserialize".into(), "Serialize".into()] }, Import { module: "regex".into(), names: vec!["Regex".into()] }]);
        assert!(imports_of("notes.txt", "import x").is_empty());
    }

    #[test]
    fn a_real_js_export_is_found_and_an_invented_one_is_named_with_the_closest_real_one() {
        let root = project("js");

        write(&root.join("node_modules/react-query/package.json"), "{}");
        write(&root.join("node_modules/react-query/index.d.ts"), "export declare function useQuery(): void;\nexport declare function useMutation(): void;\nexport { QueryClient, QueryCache as Cache } from './core';\n");

        let good = check(&root, "src/a.ts", "import { useQuery, QueryClient } from 'react-query';");
        let bad = check(&root, "src/a.ts", "import { useQuerry, QueryClient } from 'react-query';");

        assert_eq!(good, [Verdict::Found]);
        assert_eq!(bad, [Verdict::NameMissing { module: "react-query".into(), name: "useQuerry".into(), near: vec!["useQuery".into()] }]);

        let note = warning(&bad).unwrap();

        assert!(note.contains("`useQuerry` is not declared by `react-query` - did you mean `useQuery`?") && note.contains("nothing was blocked"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_misspelled_package_is_named_with_the_installed_one_and_builtins_and_relative_paths_are_fine() {
        let root = project("js-module");

        write(&root.join("node_modules/express/package.json"), "{}");
        write(&root.join("node_modules/express/index.js"), "exports.Router = 1;");

        assert_eq!(check(&root, "a.js", "const a = require('expresss');"), [Verdict::ModuleMissing { module: "expresss".into(), near: vec!["express".into()] }]);
        assert_eq!(check(&root, "a.js", "import fs from 'fs'; import b from './b'; import c from 'node:path'; import d from '@/alias';"), vec![Verdict::Found; 4]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_node_modules_it_says_it_could_not_check_and_never_says_fine() {
        let root = project("js-none");
        let verdicts = check(&root, "a.ts", "import { x } from 'left-pad';");

        assert!(matches!(&verdicts[0], Verdict::Unchecked { why, .. } if why.contains("not installed")));
        assert!(warning(&verdicts).unwrap().contains("could not be verified"));
        assert_eq!(warning(&[Verdict::Found]), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn python_names_are_looked_up_in_the_virtual_environment() {
        let root = project("py");
        let packages = root.join(".venv/lib/python3.12/site-packages");

        write(&packages.join("requests/__init__.py"), "from .api import get, post\nclass Session: pass\n");
        write(&packages.join("requests/api.py"), "def get(url): pass\ndef post(url): pass\n");
        write(&root.join("shop/__init__.py"), "");

        assert_eq!(check(&root, "app.py", "from requests import get, Session\nimport os\nfrom shop import x\n"), [Verdict::Found, Verdict::Found, Verdict::Found]);
        assert_eq!(check(&root, "app.py", "from requests import gett"), [Verdict::NameMissing { module: "requests".into(), name: "gett".into(), near: vec!["get".into()] }]);
        assert!(matches!(check(&root, "app.py", "import requestz")[0], Verdict::ModuleMissing { .. }));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn python_without_a_virtual_environment_is_unchecked_not_fine() {
        let root = project("py-none");

        assert!(matches!(&check(&root, "app.py", "import requests")[0], Verdict::Unchecked { .. }));
        assert_eq!(check(&root, "app.py", "import json"), [Verdict::Found], "the standard library needs no environment");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_rust_crate_is_read_from_vendor_and_a_missing_source_is_said() {
        let root = project("rs");

        write(&root.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n[dependencies]\nserde-json = \"1\"\nregex = \"1\"\n");
        write(&root.join("vendor/serde-json-1.0.0/src/lib.rs"), "pub fn to_string() {}\npub struct Value;\npub use self::map::Map;\n");

        assert_eq!(check(&root, "src/main.rs", "use serde_json::{to_string, Value};"), [Verdict::Found]);
        assert_eq!(check(&root, "src/main.rs", "use serde_json::to_strng;"), [Verdict::NameMissing { module: "serde_json".into(), name: "to_strng".into(), near: vec!["to_string".into()] }]);
        assert!(matches!(&check(&root, "src/main.rs", "use regex::Regexx;")[0], Verdict::Unchecked { .. } | Verdict::NameMissing { .. }), "regex's source may or may not be in this machine's registry");
        assert_eq!(check(&root, "src/main.rs", "use other_crate::Thing;"), [Verdict::Found], "not a listed dependency: not ours to judge");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_nearest_names_are_within_two_edits_nearest_first() {
        let names: BTreeSet<String> = ["useState", "useEffect", "useStates", "render"].iter().map(|name| name.to_string()).collect();

        assert_eq!(nearest("useStat", &names), ["useState", "useStates"]);
        assert!(nearest("zzzzzz", &names).is_empty());
    }
}
