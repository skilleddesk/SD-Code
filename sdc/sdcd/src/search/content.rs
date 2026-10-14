//! The content index (v3 W3): a trigram Bloom filter per file, so a `grep` reads only the files that can match.
//!
//! A full scan is bounded by the file system - ripgrep needs the same ~0.6 s as SDC for 52k files on the bench
//! machine - so the plan's < 200 ms needs an index. Each file gets 8 bits per distinct 3-byte window of its text
//! (ASCII lowercased, so the same filter serves case-blind searches) sets two bits. A pattern's required literals
//! (`required_trigrams`) must all be present for a file to be read.
//!
//! Correctness does not depend on the index: a file without a current filter (new, changed, still being indexed)
//! is always read, and a pattern with no required literal of three bytes - or a case-blind one with non-ASCII
//! letters - reads every file as before. Filters are built in the background and recomputed per changed file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use regex_syntax::hir::{Hir, HirKind};

/// Bits per distinct trigram: with two bits set per trigram, about 5% of absent trigrams look present - and a
/// query needs all of its trigrams, so a false read is rare. A 1.5 KB source file costs about 600 bytes.
const BITS_PER_TRIGRAM: usize = 8;

pub type Bloom = Box<[u64]>;

fn positions(trigram: [u8; 3], bits: u64) -> [u64; 2] {
    let value = u32::from_le_bytes([trigram[0], trigram[1], trigram[2], 0]) as u64;
    let one = value.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 13;
    let two = value.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) >> 17;

    [one % bits, two % bits]
}

fn lower(byte: u8) -> u8 {
    byte.to_ascii_lowercase()
}

/// The filter of a file's bytes.
pub fn bloom_of(bytes: &[u8]) -> Bloom {
    let distinct: std::collections::HashSet<[u8; 3]> = bytes.windows(3).map(|window| [lower(window[0]), lower(window[1]), lower(window[2])]).collect();
    let words = (distinct.len() * BITS_PER_TRIGRAM).div_ceil(64).max(1);
    let mut bits: Vec<u64> = vec![0; words];
    let size = (words * 64) as u64;

    for trigram in distinct {
        for bit in positions(trigram, size) {
            bits[(bit / 64) as usize] |= 1 << (bit % 64);
        }
    }

    bits.into_boxed_slice()
}

/// Could a file with this filter contain every one of these trigrams?
pub fn may_contain(bloom: &Bloom, trigrams: &[[u8; 3]]) -> bool {
    let size = (bloom.len() * 64) as u64;

    trigrams.iter().all(|trigram| positions(*trigram, size).iter().all(|bit| bloom[(*bit / 64) as usize] & (1 << (bit % 64)) != 0))
}

/// The trigrams every match of `pattern` must contain, lowercased; empty when nothing is certain.
pub fn required_trigrams(pattern: &str, ignore_case: bool) -> Vec<[u8; 3]> {
    let Ok(hir) = regex_syntax::ParserBuilder::new().case_insensitive(ignore_case).build().parse(pattern) else {
        return Vec::new();
    };
    let mut runs: Vec<Vec<u8>> = Vec::new();

    collect(&hir, &mut runs);

    let mut trigrams: Vec<[u8; 3]> = Vec::new();

    for run in runs {
        /* A case-blind non-ASCII letter has spellings the ASCII lowering does not know: no promise from it. */
        if ignore_case && run.iter().any(|byte| !byte.is_ascii()) {
            continue;
        }

        for window in run.windows(3) {
            let trigram = [lower(window[0]), lower(window[1]), lower(window[2])];

            if !trigrams.contains(&trigram) {
                trigrams.push(trigram);
            }
        }
    }

    trigrams
}

/// Literal runs a match must contain. A case-blind letter parses as a small class (`[Kk]`) and is taken as its
/// lowercase byte - the filter is lowercased too.
fn collect(hir: &Hir, runs: &mut Vec<Vec<u8>>) {
    match hir.kind() {
        HirKind::Literal(literal) => runs.push(literal.0.to_vec()),
        HirKind::Capture(capture) => collect(&capture.sub, runs),
        HirKind::Repetition(repetition) if repetition.min >= 1 => collect(&repetition.sub, runs),
        HirKind::Concat(parts) => {
            let mut current: Vec<u8> = Vec::new();

            for part in parts {
                match single_byte(part) {
                    Some(byte) => current.push(byte),
                    None => {
                        if let HirKind::Literal(literal) = part.kind() {
                            current.extend_from_slice(&literal.0);
                            continue;
                        }

                        if !current.is_empty() {
                            runs.push(std::mem::take(&mut current));
                        }

                        collect(part, runs);
                    }
                }
            }

            if !current.is_empty() {
                runs.push(current);
            }
        }
        _ => {}
    }
}

/// A class that is one ASCII letter in both cases (`[Kk]`), as its lowercase byte.
fn single_byte(hir: &Hir) -> Option<u8> {
    let HirKind::Class(regex_syntax::hir::Class::Unicode(class)) = hir.kind() else {
        return None;
    };
    let ranges = class.ranges();
    let chars: Vec<char> = ranges.iter().flat_map(|range| range.start()..=range.end()).take(3).collect();

    match chars.as_slice() {
        [one, two] if one.is_ascii_alphabetic() && one.eq_ignore_ascii_case(two) => Some(one.to_ascii_lowercase() as u8),
        _ => None,
    }
}

/// The filters of one folder, each stamped with the modified time and size it was made from.
#[derive(Default)]
pub struct Content {
    pub blooms: HashMap<String, (SystemTime, u64, Bloom)>,
    pub building: bool,
}

/// Is a file's filter current for this stamp?
pub fn current<'a>(content: &'a Content, path: &str, modified: SystemTime, size: u64) -> Option<&'a Bloom> {
    content.blooms.get(path).filter(|(at, bytes, _)| *at == modified && *bytes == size).map(|(_, _, bloom)| bloom)
}

/// Builds the missing or stale filters in the background; one builder per folder at a time.
pub fn fill(root: PathBuf, content: Arc<Mutex<Content>>, wanted: Vec<(String, SystemTime, u64)>) {
    {
        let mut guard = content.lock().unwrap_or_else(|poison| poison.into_inner());

        if guard.building {
            return;
        }

        guard.building = true;
    }

    std::thread::spawn(move || {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);

        for batch in wanted.chunks(threads * 64) {
            let per_thread = batch.len().div_ceil(threads).max(1);
            let built: Vec<(String, SystemTime, u64, Bloom)> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .chunks(per_thread)
                    .map(|slice| {
                        let root = &root;

                        scope.spawn(move || {
                            slice
                                .iter()
                                .filter_map(|(path, modified, size)| {
                                    let bytes = std::fs::read(Path::new(root).join(path)).ok()?;

                                    Some((path.clone(), *modified, *size, bloom_of(&bytes)))
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();

                handles.into_iter().flat_map(|handle| handle.join().unwrap_or_default()).collect()
            });
            let mut guard = content.lock().unwrap_or_else(|poison| poison.into_inner());

            for (path, modified, size, bloom) in built {
                guard.blooms.insert(path, (modified, size, bloom));
            }
        }

        content.lock().unwrap_or_else(|poison| poison.into_inner()).building = false;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_literals_are_found_in_common_patterns() {
        let words = |pattern: &str, blind: bool| required_trigrams(pattern, blind).len();

        assert_eq!(words("compute", false), 5);
        assert!(words("fn compute_total\\(", false) > 10);
        assert_eq!(words("[A-Z][a-z]+Service::new", false), required_trigrams("Service::new", false).len());
        assert_eq!(words("foo|barbaz", false), 0, "an alternation promises nothing");
        assert_eq!(words("ab", false), 0, "too short");
        assert_eq!(words("TODO\\(perf\\)", true), required_trigrams("todo\\(perf\\)", false).len(), "case-blind is lowered");
        assert_eq!(words("বাজার", true), 0, "case-blind non-ASCII promises nothing");
        assert!(words("বাজার", false) > 0);
    }

    #[test]
    fn a_filter_never_rejects_a_file_that_matches() {
        let text = b"fn compute_total(items: &[u64]) -> u64 { 0 }\nlet svc = LedgerService::new(3);\n";
        let bloom = bloom_of(text);

        for (pattern, blind) in [("compute_total", false), ("COMPUTE_TOTAL", true), ("[A-Z][a-z]+Service::new", false), ("let svc", false)] {
            assert!(may_contain(&bloom, &required_trigrams(pattern, blind)), "{pattern}");
        }

        assert!(!may_contain(&bloom, &required_trigrams("no_such_symbol_anywhere", false)));
        assert!(may_contain(&bloom_of(b""), &[]), "an empty file and no promise");
        assert!(!may_contain(&bloom_of(b"ab"), &required_trigrams("abc", false)));
    }
}
