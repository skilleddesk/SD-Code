//! **The search engine** (v3 W3) behind the agent's `glob` and `grep` on this machine (switch `v3.search`).
//!
//! v1 walked the whole folder on every call - one `read_dir` per folder and a `stat` per file, up to 50,000
//! files, then sorted them newest first - so every search on a big project cost the same quarter second however
//! little it matched (bench `2026-10-14-search-v1`). Here:
//!
//! * the walk is the `ignore` crate's parallel one, honouring `.gitignore` / `.ignore` (even outside a git repo)
//!   on top of SDC's own skipped folders and the file guard;
//! * the result - every file with its modified time and size, newest first - is kept per folder as an index, and
//!   a `notify` watcher keeps it current: a changed path is re-read on its own, a burst of changes (a checkout, an
//!   install) rebuilds the index once. Without a watcher (a network drive), the index is rebuilt after a few seconds;
//! * `grep` reads candidate files in parallel batches but reports in index order (newest first), stops at the
//!   limit, skips binary files and files over 2 MB, and gives at most `per_file` lines from one file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use regex::Regex;

pub mod content;

/// Folders no search goes into (the same list as v1's `agent::search::SKIPPED_DIRS`).
pub use crate::agent::search::SKIPPED_DIRS;

/// A file larger than this is not searched line by line.
pub const MAX_SEARCHED: u64 = 2 * 1024 * 1024;

/// More changed paths than this since the last search, and the index is rebuilt instead of patched.
const PATCH_LIMIT: usize = 2_000;

/// Without a watcher, an index older than this is rebuilt.
const UNWATCHED_TTL: Duration = Duration::from_secs(3);

/// The most folders indexed at once (each holds a watcher).
const MAX_ROOTS: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: String,
    pub modified: SystemTime,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Default)]
struct Changes {
    paths: HashSet<PathBuf>,
    rescan: bool,
}

struct Index {
    root: PathBuf,
    entries: Vec<Entry>,
    content: Arc<Mutex<content::Content>>,
    built: Instant,
    used: Instant,
    changes: Arc<Mutex<Changes>>,
    _watcher: Option<notify::RecommendedWatcher>,
}

fn registry() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<Index>>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<Index>>>>> = OnceLock::new();

    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn skipped(name: &str) -> bool {
    SKIPPED_DIRS.contains(&name)
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root).ok().map(|relative| relative.to_string_lossy().replace('\\', "/")).filter(|relative| !relative.is_empty())
}

/// One walker thread's finds, handed over in batches and once more when the thread's visitor is dropped.
struct Batch {
    local: Vec<Entry>,
    collected: Arc<Mutex<Vec<Entry>>>,
}

impl Batch {
    fn flush(&mut self) {
        if !self.local.is_empty() {
            self.collected.lock().unwrap_or_else(|poison| poison.into_inner()).append(&mut self.local);
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Every file of a folder, in parallel, with SDC's skips, the file guard and the ignore files applied.
pub fn walk(root: &Path) -> Vec<Entry> {
    let collected: Arc<Mutex<Vec<Entry>>> = Arc::new(Mutex::new(Vec::new()));
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(12);

    ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .ignore(true)
        .parents(false)
        .follow_links(false)
        .threads(threads)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();

            !(entry.file_type().is_some_and(|kind| kind.is_dir()) && skipped(&name)) && crate::fs::blocked_reason(entry.path()).is_none()
        })
        .build_parallel()
        .run(|| {
            let root = root.to_path_buf();
            let mut batch = Batch { local: Vec::new(), collected: collected.clone() };

            Box::new(move |result| {
                if let Ok(entry) = result {
                    if entry.file_type().is_some_and(|kind| kind.is_file()) {
                        if let (Some(path), Ok(meta)) = (relative(&root, entry.path()), entry.metadata()) {
                            batch.local.push(Entry { path, modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), size: meta.len() });

                            if batch.local.len() >= 512 {
                                batch.flush();
                            }
                        }
                    }
                }

                ignore::WalkState::Continue
            })
        });

    let mut entries = std::mem::take(&mut *collected.lock().unwrap_or_else(|poison| poison.into_inner()));

    sort(&mut entries);
    entries
}

fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
}

/// Would the walk include this path? (SDC's skips and the file guard; ignore files are checked by the caller.)
fn admitted(root: &Path, path: &Path) -> bool {
    let Ok(inner) = path.strip_prefix(root) else {
        return false;
    };

    !inner.components().any(|part| skipped(&part.as_os_str().to_string_lossy())) && crate::fs::blocked_reason(path).is_none()
}

fn watch(root: &Path, changes: Arc<Mutex<Changes>>) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let mut changes = changes.lock().unwrap_or_else(|poison| poison.into_inner());

        match event {
            Ok(event) if event.need_rescan() => changes.rescan = true,
            Ok(event) => {
                if !matches!(event.kind, notify::EventKind::Access(_)) {
                    changes.paths.extend(event.paths);

                    if changes.paths.len() > PATCH_LIMIT {
                        changes.rescan = true;
                    }
                }
            }
            Err(_) => changes.rescan = true,
        }
    })
    .ok()?;

    watcher.watch(root, notify::RecursiveMode::Recursive).ok()?;

    Some(watcher)
}

impl Index {
    fn build(root: &Path) -> Self {
        let changes = Arc::new(Mutex::new(Changes::default()));
        /* The watcher first, so a change made while the walk runs is not missed. */
        let watcher = watch(root, changes.clone());

        Self { root: root.to_path_buf(), entries: walk(root), content: Arc::new(Mutex::new(content::Content::default())), built: Instant::now(), used: Instant::now(), changes, _watcher: watcher }
    }

    /// Brings the index up to date: patches the changed paths, or rebuilds.
    fn refresh(&mut self) {
        self.used = Instant::now();

        if self._watcher.is_none() {
            if self.built.elapsed() > UNWATCHED_TTL {
                self.entries = walk(&self.root);
                self.built = Instant::now();
            }

            return;
        }

        let Changes { paths, rescan } = std::mem::take(&mut *self.changes.lock().unwrap_or_else(|poison| poison.into_inner()));

        if rescan {
            self.entries = walk(&self.root);
            self.built = Instant::now();
            return;
        }

        if paths.is_empty() {
            return;
        }

        /* An ignore file changed: what is searched changed everywhere. A folder appeared: walk it. */
        if paths.iter().any(|path| path.file_name().is_some_and(|name| name == ".gitignore" || name == ".ignore")) || paths.iter().any(|path| path.is_dir()) {
            self.entries = walk(&self.root);
            self.built = Instant::now();
            return;
        }

        let mut by_path: HashMap<String, usize> = self.entries.iter().enumerate().map(|(index, entry)| (entry.path.clone(), index)).collect();
        let mut removed: HashSet<usize> = HashSet::new();

        for path in paths {
            let Some(rel) = relative(&self.root, &path) else {
                continue;
            };

            match std::fs::metadata(&path) {
                Ok(meta) if meta.is_file() && admitted(&self.root, &path) && !self.ignored(&path) => {
                    let entry = Entry { path: rel.clone(), modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), size: meta.len() };

                    match by_path.get(&rel) {
                        Some(&index) => self.entries[index] = entry,
                        None => {
                            by_path.insert(rel, self.entries.len());
                            self.entries.push(entry);
                        }
                    }
                }
                _ => {
                    if let Some(&index) = by_path.get(&rel) {
                        removed.insert(index);
                    }

                    /* A removed folder takes its files with it. */
                    let prefix = format!("{rel}/");

                    for (index, entry) in self.entries.iter().enumerate() {
                        if entry.path.starts_with(&prefix) {
                            removed.insert(index);
                        }
                    }
                }
            }
        }

        if !removed.is_empty() {
            let mut index = 0;

            self.entries.retain(|_| {
                let keep = !removed.contains(&index);

                index += 1;
                keep
            });
        }

        sort(&mut self.entries);
    }

    /// The root's own `.gitignore` decides a single new file (nested ignore files trigger a full walk instead).
    fn ignored(&self, path: &Path) -> bool {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(&self.root);

        for name in [".gitignore", ".ignore"] {
            let file = self.root.join(name);

            if file.is_file() {
                builder.add(file);
            }
        }

        builder.build().map(|matcher| matcher.matched_path_or_any_parents(path, false).is_ignore()).unwrap_or(false)
    }
}

fn index_for(root: &Path) -> Arc<Mutex<Index>> {
    let key = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut registry = registry().lock().unwrap_or_else(|poison| poison.into_inner());

    if let Some(index) = registry.get(&key) {
        return index.clone();
    }

    if registry.len() >= MAX_ROOTS {
        if let Some(oldest) = registry.iter().min_by_key(|(_, index)| index.lock().map(|index| index.used).unwrap_or_else(|_| Instant::now())).map(|(key, _)| key.clone()) {
            registry.remove(&oldest);
        }
    }

    let built = Index::build(&key);

    /* The content filters start at once, in the background: the first searches read files, the later ones skip. */
    content::fill(key.clone(), built.content.clone(), built.entries.iter().filter(|entry| entry.size <= MAX_SEARCHED).map(|entry| (entry.path.clone(), entry.modified, entry.size)).collect());

    let index = Arc::new(Mutex::new(built));

    registry.insert(key, index.clone());
    index
}

/// Waits (at most `timeout`) until a folder's content filters are built - for the bench, which measures the
/// steady state separately from the first call.
pub fn wait_ready(root: &Path, timeout: Duration) -> bool {
    let index = index_for(root);
    let content = index.lock().unwrap_or_else(|poison| poison.into_inner()).content.clone();
    let deadline = Instant::now() + timeout;

    while content.lock().map(|content| content.building).unwrap_or(false) {
        if Instant::now() > deadline {
            return false;
        }

        std::thread::sleep(Duration::from_millis(20));
    }

    true
}

/// The files of a folder, relative and newest first - at most `cap`.
pub fn files(root: &Path, cap: usize) -> Vec<String> {
    let index = index_for(root);
    let mut index = index.lock().unwrap_or_else(|poison| poison.into_inner());

    index.refresh();
    index.entries.iter().take(cap).map(|entry| entry.path.clone()).collect()
}

fn clip(line: &str) -> String {
    let trimmed = line.trim();

    if trimmed.chars().count() > 300 {
        format!("{}…", trimmed.chars().take(300).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

/// The matching lines of one file, at most `most` (none for a binary or unreadable file).
///
/// `whole` is the same pattern over bytes in multi-line mode: one pass over the whole file says whether any line
/// can match, so the many files that do not are never split into lines.
fn grep_file(full: &Path, path: &str, regex: &Regex, whole: &regex::bytes::Regex, most: usize) -> Vec<Hit> {
    let Ok(bytes) = std::fs::read(full) else {
        return Vec::new();
    };

    if bytes.iter().take(8000).any(|byte| *byte == 0) || !whole.is_match(&bytes) {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&bytes);
    let mut hits = Vec::new();

    for (number, line) in text.lines().enumerate() {
        if regex.is_match(line) {
            hits.push(Hit { path: path.to_string(), line: number + 1, text: clip(line) });

            if hits.len() >= most {
                break;
            }
        }
    }

    hits
}

/// Lines matching `regex` under `base`, in files matching `glob` (a relative-path regex), newest files first,
/// at most `limit` lines and at most `per_file` from one file.
pub fn grep(root: &Path, base: &str, pattern: &str, ignore_case: bool, glob: Option<&Regex>, limit: usize, per_file: usize) -> Result<Vec<Hit>, regex::Error> {
    let regex = regex::RegexBuilder::new(pattern).case_insensitive(ignore_case).build()?;
    /* The prefilter must never miss a line the line-by-line match finds: the same pattern and case rule, with
       `^` and `$` at every line (multi-line, CRLF-aware). */
    let whole = regex::bytes::RegexBuilder::new(pattern).case_insensitive(ignore_case).multi_line(true).crlf(true).build()?;
    let (regex, whole) = (&regex, &whole);
    let trigrams = content::required_trigrams(pattern, ignore_case);
    let (candidates, filters, root_dir) = {
        let index = index_for(root);
        let mut index = index.lock().unwrap_or_else(|poison| poison.into_inner());

        index.refresh();

        let candidates: Vec<(String, SystemTime, u64)> = index
            .entries
            .iter()
            .filter(|entry| entry.size <= MAX_SEARCHED)
            .filter(|entry| base.is_empty() || entry.path.starts_with(&format!("{base}/")))
            .filter(|entry| glob.is_none_or(|glob| glob.is_match(&entry.path)))
            .map(|entry| (entry.path.clone(), entry.modified, entry.size))
            .collect();

        (candidates, index.content.clone(), index.root.clone())
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(12);
    let mut hits = Vec::new();
    let mut missing: Vec<(String, SystemTime, u64)> = Vec::new();

    /* Batches in index order. Each batch is first narrowed by the content filters (a file without a current
       filter is kept), then read in parallel; its hits are added in order. */
    for batch in candidates.chunks(threads * 32) {
        let kept: Vec<&str> = {
            let filters_now = filters.lock().unwrap_or_else(|poison| poison.into_inner());

            batch
                .iter()
                .filter(|(path, modified, size)| match content::current(&filters_now, path, *modified, *size) {
                    Some(bloom) => trigrams.is_empty() || content::may_contain(bloom, &trigrams),
                    None => {
                        missing.push((path.clone(), *modified, *size));
                        true
                    }
                })
                .map(|(path, ..)| path.as_str())
                .collect()
        };

        if kept.is_empty() {
            continue;
        }

        let per_thread = kept.len().div_ceil(threads).max(1);
        let found: Vec<Vec<Hit>> = std::thread::scope(|scope| {
            let handles: Vec<_> = kept
                .chunks(per_thread)
                .map(|slice| scope.spawn(move || slice.iter().map(|path| grep_file(&root.join(path), path, regex, whole, per_file)).collect::<Vec<_>>()))
                .collect();

            handles.into_iter().flat_map(|handle| handle.join().unwrap_or_default()).collect()
        });

        for file_hits in found {
            for hit in file_hits {
                hits.push(hit);

                if hits.len() >= limit {
                    break;
                }
            }

            if hits.len() >= limit {
                break;
            }
        }

        if hits.len() >= limit {
            break;
        }
    }

    /* Files read without a current filter get one in the background, for the next search. */
    if !missing.is_empty() {
        content::fill(root_dir, filters, missing);
    }

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-search-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        for (path, text) in [
            ("src/main.rs", "fn main() {\n    compute_total();\n}\n"),
            ("src/lib.rs", "pub fn compute_total() -> u64 { 1 }\n// compute_total again\n// compute_total thrice\n"),
            ("docs/হিসাব.md", "compute_total in Bengali notes\n"),
            ("node_modules/pkg/index.js", "compute_total\n"),
            ("build/out.txt", "compute_total\n"),
            ("ignored/skip.rs", "compute_total\n"),
            (".env", "SECRET=compute_total\n"),
            (".github/ci.yml", "run: compute_total\n"),
        ] {
            let full = root.join(path);

            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }

        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        std::fs::write(root.join("blob.bin"), [0u8, 1, 2, b'c', b'o', b'm']).unwrap();
        root
    }

    #[test]
    fn the_walk_keeps_sdcs_skips_the_guard_and_gitignore() {
        let root = tree("walk");
        let mut found: Vec<String> = walk(&root).into_iter().map(|entry| entry.path).collect();

        found.sort();
        assert_eq!(found, [".github/ci.yml", ".gitignore", "blob.bin", "docs/হিসাব.md", "src/lib.rs", "src/main.rs"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn grep_limits_per_file_and_skips_binaries() {
        let root = tree("grep");
        let hits = grep(&root, "", "compute_total", false, None, 100, 2).unwrap();
        let in_lib = hits.iter().filter(|hit| hit.path == "src/lib.rs").count();

        assert_eq!(in_lib, 2, "at most two lines from one file");
        assert!(hits.iter().any(|hit| hit.path == "docs/হিসাব.md"));
        assert!(!hits.iter().any(|hit| hit.path.contains("node_modules") || hit.path.contains("ignored") || hit.path == ".env" || hit.path == "blob.bin"));
        assert_eq!(grep(&root, "src", "compute_total", false, None, 1, 5).unwrap().len(), 1, "the limit holds");
        assert_eq!(grep(&root, "src", "COMPUTE_TOTAL", true, None, 100, 5).unwrap().len(), 4, "case-blind when asked: one line in main.rs, three in lib.rs");
        assert!(grep(&root, "src", "COMPUTE_TOTAL", false, None, 100, 5).unwrap().is_empty());
        assert_eq!(grep(&root, "src", "^pub fn", false, None, 100, 5).unwrap().len(), 1, "^ is a line start");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_index_follows_changes_to_the_folder() {
        let root = tree("watch");

        assert!(!files(&root, 100).contains(&"src/new.rs".to_string()));
        std::fs::write(root.join("src/new.rs"), "fn compute_total_new() {}\n").unwrap();
        std::fs::remove_file(root.join("src/main.rs")).unwrap();

        /* The watcher reports within a moment; the index is patched on the next search. */
        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            let now = files(&root, 100);

            if now.contains(&"src/new.rs".to_string()) && !now.contains(&"src/main.rs".to_string()) {
                assert_eq!(now[0], "src/new.rs", "newest first");
                break;
            }

            assert!(Instant::now() < deadline, "the index did not follow the change: {now:?}");
            std::thread::sleep(Duration::from_millis(50));
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}
