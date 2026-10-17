//! `~/.sdc/cache/` (P0-5): content already paid for, kept so it is not paid for twice.
//!
//! Content is stored under the digest of the content, so two paths with the same bytes are one entry and the same
//! bytes never cost a model call twice. The store has a **size cap** and evicts by **least recently used**.
//!
//! # Why the use order is SDC's own and not the file system's
//!
//! The obvious way to know when an entry was last used is the file's own time - and it does not work. A person's
//! file system is usually mounted `relatime` (Linux) or with access times switched off entirely, and reading a file
//! then does not move its time at all; the answer would be "everything was used at the same moment", and the
//! eviction would be arbitrary. So SDC keeps its own order in `index.json`: a counter that goes up on every use, and
//! a number per key. Reading a file moves its number, which is what makes LRU mean what it says.
//!
//! The counter and not a clock: two uses in the same millisecond are ordinary, and a clock would make them the same
//! moment and the order arbitrary again.
//!
//! # Why the cap is enforced on write and not by a timer
//!
//! A background cleaner is a thread that can fail at a bad moment, and its failure mode is a disk that fills up
//! anyway - the thing the cap exists to prevent. Enforcing the cap inside `put` means the store is never larger than
//! the cap for longer than one entry, and the person sees the eviction in the same call that caused it.
//!
//! # The key
//!
//! The plan asks for BLAKE3. BLAKE3 is not a dependency of this build and cannot be added while the build is
//! offline, so the key is SHA-256 - the digest `v3::cas` and `anywhere::crypto` already use. `key` is the only
//! function that decides, so moving to BLAKE3 later is one function and one kind of filename.
//!
//! # Where this sits next to `v3::cas`
//!
//! `v3::cas` keeps a project's captured artefacts under `<project>/.sdc/objects` and never evicts: those are part of
//! a project's history. This cache is the person's, lives in the home folder, and **does** evict, because its
//! contents can be re-read from the file they came from. Two stores with different lifetimes should not be one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How much of a person's disk the cache may use by default: 512 MiB - big enough that a working session does not
/// evict itself, small enough that nobody has to think about it.
pub const DEFAULT_CAP: u64 = 512 * 1024 * 1024;

/// The use-order file, beside the entries.
const INDEX: &str = "index.json";

pub fn dir(home: &Path) -> PathBuf {
    home.join("cache")
}

/// The key for some content: lowercase hex SHA-256.
pub fn key(content: &[u8]) -> String {
    hex::encode(Sha256::digest(content))
}

pub fn entry_path(home: &Path, key: &str) -> PathBuf {
    dir(home).join(key)
}

/// The use order: a counter that only goes up, and the number each key was last used at.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Index {
    seq: u64,
    used: BTreeMap<String, u64>,
}

impl Index {
    fn read(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(INDEX)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    fn write(&self, dir: &Path) -> Result<(), String> {
        let file = dir.join(INDEX);
        let text = serde_json::to_string(self).map_err(|error| error.to_string())?;

        std::fs::write(&file, text).map_err(|error| format!("{}: {error}", file.display()))
    }

    /// Mark a key as used now, and answer the number it got.
    fn touch(&mut self, key: &str) -> u64 {
        self.seq += 1;
        self.used.insert(key.to_string(), self.seq);

        self.seq
    }
}

/// What the store reports about itself. Returned rather than printed, so the caller decides whether a person hears.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Stats {
    pub entries: usize,
    pub bytes: u64,
    pub cap: u64,
    /// The keys evicted by the last `put`, least recently used first.
    pub evicted: Vec<String>,
}

/// A content-addressed cache in the home folder.
#[derive(Clone, Debug)]
pub struct Cache {
    home: PathBuf,
    cap: u64,
}

impl Cache {
    pub fn new(home: &Path) -> Self {
        Self { home: home.to_path_buf(), cap: DEFAULT_CAP }
    }

    /// A cache with a chosen cap. `0` means no cap, which a caller can ask for on purpose; it is not the default,
    /// because an uncapped cache is a full disk waiting for a date.
    pub fn with_cap(home: &Path, cap: u64) -> Self {
        Self { home: home.to_path_buf(), cap }
    }

    pub fn cap(&self) -> u64 {
        self.cap
    }

    pub fn dir(&self) -> PathBuf {
        dir(&self.home)
    }

    /// The stored content for a key, or `None`. A read that succeeds touches the entry, because a read is a use.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let bytes = std::fs::read(entry_path(&self.home, key)).ok()?;

        if let Ok(mut index) = self.index() {
            index.touch(key);
            let _ = index.write(&self.dir());
        }

        Some(bytes)
    }

    /// Store content under its own key and enforce the cap. Answers the key and what happened.
    pub fn put(&self, content: &[u8]) -> Result<(String, Stats), String> {
        let key = key(content);
        let dir = self.dir();

        std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        std::fs::write(entry_path(&self.home, &key), content).map_err(|error| format!("{}: {error}", entry_path(&self.home, &key).display()))?;

        let mut index = self.index()?;

        index.touch(&key);
        index.write(&dir)?;

        Ok((key, self.enforce()?))
    }

    fn index(&self) -> Result<Index, String> {
        Ok(Index::read(&self.dir()))
    }

    /// Every entry with its size and its last use, **least recently used first**. This is the eviction order.
    fn entries(&self) -> Vec<(String, u64, u64)> {
        let index = Index::read(&self.dir());

        let Ok(listing) = std::fs::read_dir(self.dir()) else {
            return Vec::new();
        };

        let mut entries: Vec<(String, u64, u64)> = listing
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();

                if name == INDEX {
                    return None;
                }

                let metadata = entry.metadata().ok()?;

                if !metadata.is_file() {
                    return None;
                }

                /* A file with no number was never touched through this cache; it is the oldest there is (`0`), which
                   is the safe reading: an entry nobody has a record of using is the first one to let go. */
                let used = index.used.get(&name).copied().unwrap_or(0);

                Some((name, metadata.len(), used))
            })
            .collect();

        entries.sort_by_key(|(_, _, used)| *used);

        entries
    }

    pub fn stats(&self) -> Stats {
        let entries = self.entries();

        Stats { entries: entries.len(), bytes: entries.iter().map(|(_, size, _)| size).sum(), cap: self.cap, evicted: Vec::new() }
    }

    /// Drop the least recently used entries until the store is inside its cap.
    fn enforce(&self) -> Result<Stats, String> {
        let dir = self.dir();
        let mut entries = self.entries();
        let mut bytes: u64 = entries.iter().map(|(_, size, _)| size).sum();
        let mut evicted = Vec::new();
        let mut index = Index::read(&dir);

        if self.cap > 0 {
            for (name, size, _) in entries.drain(..) {
                if bytes <= self.cap {
                    break;
                }

                if std::fs::remove_file(dir.join(&name)).is_ok() {
                    bytes -= size;
                    index.used.remove(&name);
                    evicted.push(name);
                }
            }

            if !evicted.is_empty() {
                index.write(&dir)?;
            }
        }

        Ok(Stats { entries: self.entries().len(), bytes: self.entries().iter().map(|(_, size, _)| size).sum(), cap: self.cap, evicted })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_addressed_means_the_same_bytes_are_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());

        let (first, _) = cache.put(b"fn main() {}").unwrap();
        let (second, stats) = cache.put(b"fn main() {}").unwrap();

        assert_eq!(first, second);
        assert_eq!(stats.entries, 1, "the same bytes never cost a second entry");
        assert_eq!(cache.get(&first).unwrap(), b"fn main() {}");
    }

    #[test]
    fn the_key_is_a_fingerprint_of_the_content() {
        assert_eq!(key(b"abc").len(), 64);
        assert_eq!(key(b"abc"), key(b"abc"));
        assert_ne!(key(b"abc"), key(b"abd"));
    }

    #[test]
    fn a_read_of_something_that_was_never_stored_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(Cache::new(dir.path()).get(&key(b"never wrote this")), None);
    }

    #[test]
    fn the_cap_is_enforced_by_evicting_the_least_recently_used_first() {
        let dir = tempfile::tempdir().unwrap();
        /* Three entries of ten bytes fit in thirty-five; the fourth has to push one out. */
        let cache = Cache::with_cap(dir.path(), 35);

        let (oldest, _) = cache.put(&[b'a'; 10]).unwrap();
        let (middle, _) = cache.put(&[b'b'; 10]).unwrap();
        let (newest, _) = cache.put(&[b'c'; 10]).unwrap();

        assert_eq!(cache.stats().entries, 3, "all three fit before the cap is reached");

        /* Use the oldest, which is what "recently used" in LRU means: it is now the most recent of all. */
        assert!(cache.get(&oldest).is_some());

        let (_, stats) = cache.put(&[b'd'; 10]).unwrap();

        assert!(stats.bytes <= 35, "the store is inside its cap after the write: {} bytes", stats.bytes);
        assert_eq!(stats.evicted, vec![middle.clone()], "the one that was not used went, not the one that was");
        assert!(cache.get(&oldest).is_some(), "the entry that was just read stays");
        assert!(cache.get(&newest).is_some(), "and so does the newest");
        assert_eq!(cache.get(&middle), None);
    }

    #[test]
    fn an_entry_no_one_has_a_record_of_using_goes_first() {
        let dir = tempfile::tempdir().unwrap();
        /* A cap of twenty-five holds two entries of ten bytes: the stray plus one of the two real ones. */
        let cache = Cache::with_cap(dir.path(), 25);

        let (known, _) = cache.put(&[b'a'; 10]).unwrap();

        /* A file that appeared in the folder without going through `put` - an older build's entry, or a person's
           stray file. It has no number, so it is the first to let go. */
        std::fs::write(entry_path(dir.path(), "stray"), [b's'; 10]).unwrap();

        let (_, stats) = cache.put(&[b'b'; 10]).unwrap();

        assert_eq!(stats.evicted, vec!["stray".to_string()]);
        assert!(cache.get(&known).is_some(), "the entry with a use behind it stays");
    }

    #[test]
    fn a_cap_of_zero_evicts_nothing_and_means_no_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::with_cap(dir.path(), 0);

        for byte in *b"abc" {
            cache.put(&[byte; 100]).unwrap();
        }

        let stats = cache.stats();

        assert_eq!(stats.entries, 3);
        assert!(stats.evicted.is_empty());
    }

    #[test]
    fn a_cap_smaller_than_one_entry_leaves_the_store_empty_rather_than_over() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::with_cap(dir.path(), 5);

        let (key, stats) = cache.put(&[b'x'; 100]).unwrap();

        assert_eq!(stats.bytes, 0, "nothing is kept when even one entry is too big");
        assert_eq!(stats.evicted, vec![key.clone()]);
        assert_eq!(cache.get(&key), None);
    }

    #[test]
    fn the_default_is_a_cap_and_the_order_survives_a_restart() {
        let cache = Cache::new(Path::new("/tmp"));

        assert_eq!(cache.cap(), DEFAULT_CAP);
        assert!(cache.cap() > 0, "an uncapped cache is a full disk with a date on it");

        /* The order is in a file, not in memory: a second `Cache` on the same folder reads the same answer. */
        let dir = tempfile::tempdir().unwrap();
        let first = Cache::with_cap(dir.path(), 35);

        first.put(&[b'a'; 10]).unwrap();
        let (b, _) = first.put(&[b'b'; 10]).unwrap();
        first.put(&[b'c'; 10]).unwrap();
        first.get(&b);

        let reopened = Cache::with_cap(dir.path(), 35);

        reopened.put(&[b'd'; 10]).unwrap();

        assert!(reopened.get(&b).is_some(), "the use recorded before the restart still counts after it");
    }

    #[test]
    fn the_cache_lives_under_the_home_folder_and_the_index_is_not_an_entry() {
        assert_eq!(dir(Path::new("/tmp/home")), PathBuf::from("/tmp/home/cache"));

        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());

        cache.put(b"one").unwrap();
        cache.put(b"two").unwrap();

        assert_eq!(cache.stats().entries, 2, "index.json is the order, not content, so it is not counted");
    }
}
