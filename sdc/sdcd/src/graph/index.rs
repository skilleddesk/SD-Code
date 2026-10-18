//! The Merkle index of a project (P1-1).
//!
//! Each file's artifacts (declarations, references, imports) are keyed by the **hash of its content**. A refresh walks the project, and for
//! each file reuses the cached artifacts when the content hash is unchanged and re-parses only when it is not. A quick check on
//! (size, modification time) avoids reading a file that has plainly not changed; if either differs the file is read and hashed, so a
//! branch switch - which rewrites files with new content and new times - is always noticed, and content that comes back (switching
//! back) is recognised by its hash and not re-parsed. The directory hashes and the root hash are Merkle sums of the children, so "did
//! anything under `src/` change" is one comparison.
//!
//! The cache lives in the data directory, one file per project, and is safe to delete.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::parse::{self, Def, Lang, Reference};

const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".next", ".venv", "venv", "vendor", "__pycache__", ".sdc", ".idea", ".gradle", "Pods", "coverage"];
const MAX_FILE_BYTES: u64 = 1_000_000;
const MAX_FILES: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub hash: String,
    pub size: u64,
    pub mtime_ns: u128,
    pub lang: String,
    pub defs: Vec<Def>,
    pub refs: Vec<Reference>,
    pub imports: Vec<String>,
    /// The file does not parse cleanly (its declarations are what the parser could recover).
    pub broken: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub files: BTreeMap<String, FileEntry>,
    /// Artifacts of content that is no longer in the folder, keyed by content hash - so switching back to a branch costs no parse.
    #[serde(default)]
    pub pool: BTreeMap<String, FileEntry>,
}

/// How many remembered contents the pool keeps.
const POOL_LIMIT: usize = 4_000;

/// What a refresh did - the numbers the tests and the performance report read.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Refresh {
    pub files: usize,
    pub parsed: usize,
    pub reused: usize,
    pub removed: usize,
    pub root_hash: String,
}

fn mtime_ns(meta: &std::fs::Metadata) -> u128 {
    meta.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |duration| duration.as_nanos())
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();

    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if found.len() >= MAX_FILES {
            return;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(kind) = entry.file_type() else {
            continue;
        };

        if name.starts_with("._") || kind.is_symlink() {
            continue;
        }

        if kind.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &entry.path(), found);
            }
        } else if kind.is_file() && Lang::of(&name).is_some() {
            found.push(entry.path());
        }
    }
}

impl Index {
    /// Where a project's cache lives.
    pub fn cache_path(root: &Path) -> Option<PathBuf> {
        let real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let key = &parse::sha(&real.to_string_lossy())[..16];

        crate::paths::data_dir().ok().map(|dir| dir.join("graph").join(format!("{key}.json")))
    }

    /// The cached index of a project, or an empty one.
    pub fn load(root: &Path) -> Self {
        Self::cache_path(root).and_then(|path| std::fs::read_to_string(path).ok()).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) {
        if let Some(path) = Self::cache_path(root) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let partial = path.with_extension("json.partial");

            if std::fs::write(&partial, serde_json::to_string(self).unwrap_or_default()).is_ok() {
                let _ = std::fs::rename(partial, path);
            }
        }
    }

    /// Brings the index up to date with the folder: re-parses files whose content changed, keeps the rest, drops the deleted.
    pub fn refresh(&mut self, root: &Path) -> Refresh {
        let mut paths = Vec::new();

        walk(root, root, &mut paths);

        let mut seen = std::collections::BTreeSet::new();
        let mut stats = Refresh::default();

        for path in &paths {
            let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
            let Ok(meta) = std::fs::metadata(path) else {
                continue;
            };

            if meta.len() > MAX_FILE_BYTES {
                continue;
            }

            seen.insert(relative.clone());
            stats.files += 1;

            let (size, time) = (meta.len(), mtime_ns(&meta));

            if self.files.get(&relative).is_some_and(|entry| entry.size == size && entry.mtime_ns == time) {
                stats.reused += 1;
                continue;
            }

            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes).to_string();
            let hash = parse::sha(&text);

            /* Same content under a new time (a checkout that wrote the same bytes): the artifacts still hold. */
            if let Some(entry) = self.files.get_mut(&relative).filter(|entry| entry.hash == hash) {
                entry.size = size;
                entry.mtime_ns = time;
                stats.reused += 1;
                continue;
            }

            /* Content seen before under another path (a rename or a switch back): reuse the parse, relabel nothing - defs carry no path. */
            let same = self.files.values().find(|entry| entry.hash == hash).cloned().or_else(|| self.pool.get(&hash).cloned());
            let entry = match same {
                Some(mut entry) => {
                    entry.size = size;
                    entry.mtime_ns = time;
                    stats.reused += 1;
                    entry
                }
                None => {
                    stats.parsed += 1;
                    entry_for(&relative, &text, hash, size, time)
                }
            };

            if let Some(old) = self.files.insert(relative, entry) {
                self.pool.insert(old.hash.clone(), old);
            }
        }

        let gone: Vec<String> = self.files.keys().filter(|path| !seen.contains(*path)).cloned().collect();

        stats.removed = gone.len();

        for path in gone {
            if let Some(old) = self.files.remove(&path) {
                self.pool.insert(old.hash.clone(), old);
            }
        }

        while self.pool.len() > POOL_LIMIT {
            let Some(first) = self.pool.keys().next().cloned() else {
                break;
            };

            self.pool.remove(&first);
        }

        stats.root_hash = self.root_hash();
        stats
    }

    /// The Merkle root: a hash over every (directory, child hash) pair, bottom-up.
    pub fn root_hash(&self) -> String {
        self.dir_hashes().remove("").unwrap_or_default()
    }

    /// The hash of every directory (`""` is the root): equal hashes mean nothing under it changed.
    pub fn dir_hashes(&self) -> BTreeMap<String, String> {
        let mut children: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

        for (path, entry) in &self.files {
            let (dir, name) = path.rsplit_once('/').map_or(("", path.as_str()), |(dir, name)| (dir, name));

            children.entry(dir.to_string()).or_default().push((name.to_string(), entry.hash.clone()));

            /* make sure every ancestor exists so the sums reach the root */
            let mut parent = dir;

            while let Some((up, _)) = parent.rsplit_once('/') {
                children.entry(up.to_string()).or_default();
                parent = up;
            }

            if !dir.is_empty() {
                children.entry(String::new()).or_default();
            }
        }

        let mut hashes: BTreeMap<String, String> = BTreeMap::new();
        let mut dirs: Vec<String> = children.keys().cloned().collect();

        dirs.sort_by_key(|dir| std::cmp::Reverse(dir.matches('/').count() + usize::from(!dir.is_empty())));

        for dir in dirs {
            let mut parts: Vec<(String, String)> = children.get(&dir).cloned().unwrap_or_default();

            for (other, hash) in &hashes {
                let parent = other.rsplit_once('/').map_or("", |(parent, _)| parent);

                if parent == dir && !other.is_empty() {
                    parts.push((format!("{}/", other.rsplit('/').next().unwrap_or(other)), hash.clone()));
                }
            }

            parts.sort();
            hashes.insert(dir, parse::sha(&parts.iter().map(|(name, hash)| format!("{name}:{hash}")).collect::<Vec<_>>().join("\n")));
        }

        hashes
    }

    // ---- queries (zero model calls) -------------------------------------------------------------------------------------------

    /// Where a name is declared: `(file, def)`. Matches the plain name or the qualified one (`Ledger::total`, `Store.add`).
    pub fn where_is(&self, name: &str) -> Vec<(String, Def)> {
        let mut found: Vec<(String, Def)> = self.files.iter().flat_map(|(path, entry)| entry.defs.iter().filter(|def| def.name == name || def.qualified == name).map(move |def| (path.clone(), def.clone()))).collect();

        found.sort_by(|a, b| (a.0.as_str(), a.1.start_line).cmp(&(b.0.as_str(), b.1.start_line)));
        found
    }

    /// Where a name is called or used, without its own declarations: `(file, line, is_call)`.
    pub fn callers(&self, name: &str) -> Vec<(String, usize, bool)> {
        let short = name.rsplit([':', '.']).next().unwrap_or(name);
        let mut found = Vec::new();

        for (path, entry) in &self.files {
            let declared: std::collections::HashSet<usize> = entry.defs.iter().filter(|def| def.name == short).map(|def| def.start_line).collect();

            for reference in entry.refs.iter().filter(|reference| reference.name == short) {
                /* the declaration's own name sits on its first line */
                if !reference.call && declared.contains(&reference.line) {
                    continue;
                }

                found.push((path.clone(), reference.line, reference.call));
            }
        }

        found.sort();
        found.dedup();
        found
    }

    /// The test files that touch a source file: they import its module, or refer to a name it declares, or are named after it.
    pub fn tests_for(&self, file: &str) -> Vec<String> {
        let Some(entry) = self.files.get(file) else {
            return Vec::new();
        };
        let stem = Path::new(file).file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string();
        let declared: std::collections::HashSet<&str> = entry.defs.iter().map(|def| def.name.as_str()).filter(|name| name.len() > 2).collect();
        let mut found: Vec<String> = self
            .files
            .iter()
            .filter(|(path, _)| path.as_str() != file && is_test_path(path))
            .filter(|(path, other)| {
                let named = Path::new(path.as_str()).file_stem().and_then(|name| name.to_str()).is_some_and(|name| name.contains(&stem) && !stem.is_empty());
                let imports = other.imports.iter().any(|module| module.rsplit(['/', '.', ':']).next().is_some_and(|last| last == stem) || module.contains(&format!("{stem}::")) || module.ends_with(&stem));
                let uses = other.refs.iter().any(|reference| declared.contains(reference.name.as_str()));

                named || imports || uses
            })
            .map(|(path, _)| path.clone())
            .collect();

        found.sort();
        found
    }

    /// Declarations of one file, for `symbols` listings.
    pub fn symbols_of(&self, file: &str) -> Vec<Def> {
        self.files.get(file).map(|entry| entry.defs.clone()).unwrap_or_default()
    }
}

fn entry_for(path: &str, text: &str, hash: String, size: u64, mtime: u128) -> FileEntry {
    let lang = Lang::of(path);
    let tree = lang.and_then(|lang| parse::parse(lang, text));
    let (defs, refs, imports, broken) = match (lang, tree) {
        (Some(lang), Some(tree)) => (
            if lang.has_symbols() { parse::definitions(lang, &tree, text) } else { Vec::new() },
            if lang.has_symbols() { parse::references(&tree, text) } else { Vec::new() },
            parse::imports(lang, &tree, text),
            tree.root_node().has_error(),
        ),
        _ => (Vec::new(), Vec::new(), Vec::new(), false),
    };

    FileEntry { hash, size, mtime_ns: mtime, lang: lang.map_or("", Lang::name).to_string(), defs, refs, imports, broken }
}

/// Whether a path is a test file by the conventions of the languages the graph reads.
pub fn is_test_path(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    let name = lowered.rsplit('/').next().unwrap_or(&lowered);

    lowered.split('/').any(|part| matches!(part, "test" | "tests" | "__tests__" | "spec" | "specs" | "e2e"))
        || name.starts_with("test_")
        || name.contains("_test.")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.ends_with("test.php")
}

/// A refreshed index of a project (loaded from and saved to the cache).
pub fn refreshed(root: &Path) -> (Index, Refresh) {
    let mut index = Index::load(root);
    let stats = index.refresh(root);

    index.save(root);
    (index, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-graph-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("src/store.py"), "class Store:\n    def add(self, x):\n        return x\n\ndef helper():\n    return Store()\n").unwrap();
        std::fs::write(root.join("src/app.py"), "from src.store import Store\n\ndef main():\n    s = Store()\n    s.add(1)\n    helper()\n").unwrap();
        std::fs::write(root.join("tests/test_store.py"), "from src.store import Store\n\ndef test_add():\n    assert Store().add(2) == 2\n").unwrap();
        std::fs::write(root.join("README.md"), "not code").unwrap();
        root
    }

    #[test]
    fn where_callers_and_tests_answer_from_the_parse_with_no_model() {
        let root = project("queries");
        let mut index = Index::default();
        let stats = index.refresh(&root);

        assert_eq!((stats.files, stats.parsed), (3, 3), "the README is not a language the graph reads");

        let at = index.where_is("Store.add");

        assert_eq!(at.len(), 1);
        assert_eq!((at[0].0.as_str(), at[0].1.start_line), ("src/store.py", 2));
        assert_eq!(index.where_is("Store").len(), 1);

        let callers = index.callers("helper");

        assert!(callers.iter().any(|(file, line, call)| file == "src/app.py" && *line == 6 && *call), "{callers:?}");
        assert!(!callers.iter().any(|(file, line, _)| file == "src/store.py" && *line == 5), "the declaration is not a caller");

        assert_eq!(index.tests_for("src/store.py"), vec!["tests/test_store.py".to_string()]);
        assert!(index.tests_for("src/app.py").is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_refresh_reparses_only_the_file_that_changed() {
        let root = project("incremental");
        let mut index = Index::default();

        index.refresh(&root);

        let before = index.root_hash();
        let again = index.refresh(&root);

        assert_eq!((again.parsed, again.reused), (0, 3), "nothing changed: nothing parsed");
        assert_eq!(again.root_hash, before);

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("src/store.py"), "class Store:\n    def add(self, x):\n        return x + 1\n\n    def sub(self):\n        pass\n").unwrap();

        let changed = index.refresh(&root);

        assert_eq!((changed.parsed, changed.reused), (1, 2), "one file changed: one parse");
        assert_ne!(changed.root_hash, before, "and the Merkle root moved");
        assert_eq!(index.where_is("Store.sub").len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The branch switch: files are rewritten with other content and then rewritten back. Answers follow the content, never the past.
    #[test]
    fn a_branch_switch_never_serves_stale_answers_and_switching_back_costs_no_parse() {
        let root = project("branch");
        let original = std::fs::read_to_string(root.join("src/store.py")).unwrap();
        let mut index = Index::default();

        index.refresh(&root);

        let first_root = index.root_hash();

        /* "other branch": store.py has a different class, app.py is gone */
        std::fs::write(root.join("src/store.py"), "class Other:\n    def run(self):\n        pass\n").unwrap();
        std::fs::remove_file(root.join("src/app.py")).unwrap();

        let switched = index.refresh(&root);

        assert_eq!((switched.parsed, switched.removed), (1, 1));
        assert!(index.where_is("Store").is_empty(), "the class that is not on this branch is not found");
        assert_eq!(index.where_is("Other").len(), 1);
        assert!(index.callers("helper").is_empty());

        /* back to the first branch */
        std::fs::write(root.join("src/store.py"), &original).unwrap();
        std::fs::write(root.join("src/app.py"), "from src.store import Store\n\ndef main():\n    s = Store()\n    s.add(1)\n    helper()\n").unwrap();

        let back = index.refresh(&root);

        assert_eq!(back.parsed, 0, "both files' content was seen before: found by hash, not re-parsed");
        assert_eq!(back.root_hash, first_root, "the same tree has the same Merkle root");
        assert_eq!(index.where_is("Store").len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_hash_moves_only_when_something_under_it_does() {
        let root = project("subtree");
        let mut index = Index::default();

        index.refresh(&root);

        let before = index.dir_hashes();

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("tests/test_store.py"), "def test_other():\n    pass\n").unwrap();
        index.refresh(&root);

        let after = index.dir_hashes();

        assert_eq!(before["src"], after["src"], "src/ did not change");
        assert_ne!(before["tests"], after["tests"]);
        assert_ne!(before[""], after[""]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_cache_round_trips_and_a_big_tree_indexes_quickly() {
        let root = std::env::temp_dir().join(format!("sdc-graph-perf-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pkg")).unwrap();

        for n in 0..400 {
            let body: String = (0..8).map(|m| format!("def fn_{n}_{m}(a, b):\n    return helper_{m}(a) + b\n\n")).collect();

            std::fs::write(root.join(format!("pkg/mod_{n}.py")), body).unwrap();
        }

        let mut index = Index::default();
        let cold = std::time::Instant::now();
        let first = index.refresh(&root);
        let cold_ms = cold.elapsed().as_millis();
        let warm = std::time::Instant::now();
        let second = index.refresh(&root);
        let warm_ms = warm.elapsed().as_millis();

        println!("graph: {} files, {} defs; cold {cold_ms} ms (parsed {}), warm {warm_ms} ms (reused {})", first.files, index.files.values().map(|e| e.defs.len()).sum::<usize>(), first.parsed, second.reused);

        assert_eq!((first.files, first.parsed, second.parsed, second.reused), (400, 400, 0, 400));
        assert!(warm_ms * 3 < cold_ms.max(30), "a warm refresh is much cheaper than a cold one: {warm_ms} vs {cold_ms}");
        assert!(index.where_is("fn_399_7").len() == 1);

        let text = serde_json::to_string(&index).unwrap();
        let back: Index = serde_json::from_str(&text).unwrap();

        assert_eq!(back.root_hash(), index.root_hash());

        let _ = std::fs::remove_dir_all(&root);
    }
}
