//! `sdcd bench search` (v3 W1, A-04/A-05): the agent's own `glob` and `grep` timed on a folder, first call and
//! repeats, p50/p95 - and `sdcd bench gen-tree`, the deterministic 50k-file tree the targets are set on.
//!
//! "First" is the first call in this process (with the v3 engine: the file index built, the content index still
//! building in the background); p50/p95 are the repeats after the background indexing finished.
//! The first call's OS file cache state: the operating system's file cache may still be warm from an
//! earlier run, so a truly cold number needs the cache dropped first (`sudo purge` on macOS, `echo 3 >
//! /proc/sys/vm/drop_caches` on Linux). The report says which it was.

use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use super::stats;

#[derive(Debug, Serialize)]
pub struct Timing {
    pub kind: String,
    pub query: String,
    pub results: usize,
    pub first_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
}

/// The default queries: two filename globs, three content searches of growing cost, and one that matches nothing
/// (every file is read - the worst case).
pub const GLOBS: [&str; 2] = ["**/*.rs", "**/module_7*/**/*.ts"];
pub const GREPS: [&str; 4] = ["fn compute_total", "TODO\\(perf\\)", "[A-Z][a-z]+Service::new", "no_such_symbol_anywhere"];

pub fn measure(root: &Path, repeats: usize) -> Result<Vec<Timing>, String> {
    let root_text = root.display().to_string();
    let mut timings = Vec::new();
    let mut time = |kind: &str, query: &str, call: &dyn Fn() -> Result<usize, String>| -> Result<(), String> {
        let mut samples = Vec::new();
        let mut results = 0;

        for round in 0..repeats.max(2) {
            let started = Instant::now();

            results = call()?;
            samples.push(started.elapsed().as_secs_f64() * 1_000.0);

            /* The steady state is measured once the engine's background indexing is done (v3 W3). */
            if round == 0 && crate::v3::on("search") {
                crate::search::wait_ready(root, std::time::Duration::from_secs(120));
            }
        }

        let (p50, p95) = stats::p50_p95(&samples[1..]).unwrap_or((samples[0], samples[0]));

        timings.push(Timing { kind: kind.into(), query: query.into(), results, first_ms: samples[0], p50_ms: p50, p95_ms: p95 });

        Ok(())
    };

    for pattern in GLOBS {
        time("filename", pattern, &|| crate::agent::search::glob(&root_text, None, "", pattern, 100).map(|(hits, _)| hits.len()).map_err(|error| error.message.clone()))?;
    }

    for pattern in GREPS {
        time("text", pattern, &|| crate::agent::search::grep(&root_text, None, "", pattern, None, false, 100).map(|hits| hits.len()).map_err(|error| error.message.clone()))?;
    }

    Ok(timings)
}

/// A deterministic tree: `files` source files in nested modules, with Bengali file names, a `node_modules`
/// and a `.gitignore`, so later search work can be checked on ignore rules and Unicode names too.
pub fn generate_tree(out: &Path, files: usize) -> Result<usize, String> {
    let write = |path: &Path, text: &str| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        std::fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
    };
    let extensions = ["rs", "ts", "py", "md", "tsx", "json"];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    write(&out.join(".gitignore"), "node_modules/\ntarget/\n*.log\n")?;

    for index in 0..files {
        let module = index % 97;
        let depth = (index / 97) % 5;
        let ext = extensions[index % extensions.len()];
        let mut dir = out.join(format!("module_{module}"));

        for level in 0..depth {
            dir = dir.join(format!("sub_{level}_{}", (index / 7) % 11));
        }

        let name = if index % 500 == 0 { format!("হিসাব_{index}.{ext}") } else { format!("file_{index}.{ext}") };
        let service = ["Order", "Ledger", "Invoice", "User", "Cart"][(next() % 5) as usize];
        let mut text = format!("// file {index}\n");

        for line in 0..(20 + next() % 60) {
            match next() % 9 {
                0 => text.push_str(&format!("fn compute_total_{line}(items: &[u64]) -> u64 {{ items.iter().sum() }}\n")),
                1 => text.push_str(&format!("let svc = {service}Service::new({line});\n")),
                2 if index % 50 == 0 => text.push_str("// TODO(perf): cache this lookup\n"),
                _ => text.push_str(&format!("const VALUE_{line}: u64 = {};\n", next() % 100_000)),
            }
        }

        write(&dir.join(name), &text)?;
    }

    for index in 0..(files / 20).max(1) {
        write(&out.join("node_modules").join(format!("pkg_{}", index % 40)).join(format!("index_{index}.js")), "module.exports = function compute_total() {};\n")?;
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_tree_is_deterministic_and_searchable() {
        let base = std::env::temp_dir().join(format!("sdc-bench-tree-{}", std::process::id()));
        let (one, two) = (base.join("one"), base.join("two"));
        let _ = std::fs::remove_dir_all(&base);

        generate_tree(&one, 600).unwrap();
        generate_tree(&two, 600).unwrap();

        let a = std::fs::read_to_string(one.join("module_5").join("file_5.json")).unwrap();
        let b = std::fs::read_to_string(two.join("module_5").join("file_5.json")).unwrap();

        assert_eq!(a, b, "same seed, same tree");
        assert!(one.join("module_0").join("হিসাব_0.rs").exists(), "a Bengali file name");

        let timings = measure(&one, 3).unwrap();

        assert_eq!(timings.len(), GLOBS.len() + GREPS.len());
        assert!(timings.iter().find(|timing| timing.query == "**/*.rs").unwrap().results > 0);
        assert!(timings.iter().all(|timing| timing.p95_ms >= timing.p50_ms));

        let _ = std::fs::remove_dir_all(&base);
    }
}
