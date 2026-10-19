//! **The retrieval cascade (P1-8, switch `sme.retrieval`)**: finding the few lines that matter without a model, in two stages.
//!
//! Stage 1 costs nothing and always runs. Three signals that are already on disk are fused into one ranked list:
//!
//! * **the file's name** - a question about *billing* looks at `billing/` first, and a name is often all a file has;
//! * **the declarations** - the Code Graph (P1-1) knows which chunk *declares* a name the question uses, which is a stronger signal
//!   than a word appearing somewhere in a file;
//! * **BM25 over chunks** - chunks are the AST's own units (a function, a class, a markdown section), not fixed windows, so a hit is a
//!   whole thought and can be sent whole. BM25 is the ranking a grep cannot do: it weighs a word that is rare in this project above one
//!   that is everywhere, so `charge` in a payments project finds the payment code and `the` finds nothing.
//!
//! The stage's output is a *skim*: `path:line` and the signature, a few lines each, so the model chooses what to open.
//!
//! Stage 2 is a re-ranker on the GPU/CPU that reads the top candidates and orders them again. It is **off unless measured**:
//! [`rerank_enabled`] answers `false` unless the owner turns it on, because the plan's rule for this stage is that it ships only if it
//! lifts recall on the bench - a re-ranker that costs a model call and does not improve the order is a slower grep.
//!
//! Summaries of chunks are cached by the chunk's content hash ([`Summaries`]): the same bytes are never sent to a model twice, and a
//! re-read of the file does not invalidate the summary of a function that did not change.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use crate::graph::parse::Lang;

/// One unit of code or prose: a declaration, a markdown section, or (for a language the graph does not read) a window of lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
    pub hash: String,
}

/// A ranked candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub score: f32,
    /// Why it is here: `name`, `declares` or `text`.
    pub why: &'static str,
    pub text: String,
}

/// Lines a chunk may have when a language has no AST units SDC can read.
const WINDOW: usize = 40;
/// Files above this are not chunked.
const MAX_FILE_BYTES: u64 = 1_000_000;
/// Files a build will read.
const MAX_FILES: usize = 4_000;

/// The chunks of one file: its declarations (for a language the graph reads), its markdown sections, or windows of lines.
pub fn chunks(path: &str, text: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found: Vec<Chunk> = Vec::new();
    let mut ranges: Vec<(usize, usize)> = Vec::new();

    if let Some(lang) = Lang::of(path).filter(|lang| lang.has_symbols()) {
        if let Some(tree) = crate::graph::parse::parse(lang, text) {
            for def in crate::graph::parse::definitions(lang, &tree, text) {
                let (start, end) = (def.start_line.max(1), def.end_line.max(def.start_line).min(lines.len().max(1)));

                if start <= end {
                    ranges.push((start, end));
                }
            }
        }
    } else if matches!(Path::new(path).extension().and_then(|ext| ext.to_str()), Some("md" | "markdown" | "txt")) {
        let mut start = 1;

        for (index, line) in lines.iter().enumerate() {
            let numbered = index + 1;

            if numbered > 1 && line.starts_with('#') && numbered > start {
                ranges.push((start, numbered - 1));
                start = numbered;
            }
        }

        if start <= lines.len() {
            ranges.push((start, lines.len()));
        }
    }

    if ranges.is_empty() {
        let mut start = 1;

        while start <= lines.len() {
            let end = (start + WINDOW - 1).min(lines.len());

            ranges.push((start, end));
            start = end + 1;
        }
    }

    for (start, end) in ranges {
        let text = lines[start.saturating_sub(1)..end.min(lines.len())].join("\n");

        if text.trim().is_empty() {
            continue;
        }

        found.push(Chunk { path: path.to_string(), start_line: start, end_line: end, hash: crate::graph::parse::sha(&text), text });
    }

    found
}

/// The words a query and a chunk are compared by: lower case, `snake_case` and `camelCase` split, one letter dropped, stop words gone.
pub fn terms(text: &str) -> Vec<String> {
    let mut out = Vec::new();

    for word in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        for part in word.split('_').filter(|part| !part.is_empty()) {
            /* camelCase: `drawHeader` is `draw` and `header` */
            let mut piece = String::new();

            for (index, c) in part.char_indices() {
                if index > 0 && c.is_uppercase() && !piece.is_empty() {
                    push_term(&mut out, &piece);
                    piece.clear();
                }

                piece.push(c);
            }

            push_term(&mut out, &piece);
        }
    }

    out
}

fn push_term(out: &mut Vec<String>, piece: &str) {
    let word = piece.to_ascii_lowercase();

    if word.len() >= 2 && !STOP.contains(&word.as_str()) {
        out.push(word);
    }
}

/// Words that carry no signal in any project.
const STOP: &[&str] = &["the", "and", "for", "with", "that", "this", "you", "are", "was", "not", "but", "how", "why", "does", "did", "can", "its", "it", "to", "of", "in", "on", "is", "a", "an", "be", "as", "at", "or", "by", "from", "when", "what", "which", "there", "have", "has", "do"];

/// BM25's two constants, as the literature's defaults.
const K1: f32 = 1.2;
const B: f32 = 0.75;

/// The built index: the chunks, and the counts BM25 needs.
#[derive(Debug, Default)]
pub struct Index {
    chunks: Vec<Chunk>,
    /// How many chunks each word appears in.
    document_frequency: HashMap<String, usize>,
    /// Each chunk's length in words.
    lengths: Vec<usize>,
    /// The files that were read - what the progress tells the person.
    pub files: usize,
}

impl Index {
    /// Chunks of `(path, text)` pairs - the shape both the project reader and the tests use.
    pub fn of<'a>(sources: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
        let mut index = Index::default();

        for (path, text) in sources {
            index.files += 1;

            for chunk in chunks(path, text) {
                let words = terms(&chunk.text);
                let mut seen = HashSet::new();

                for word in &words {
                    if seen.insert(word.clone()) {
                        *index.document_frequency.entry(word.clone()).or_default() += 1;
                    }
                }

                index.lengths.push(words.len());
                index.chunks.push(chunk);
            }
        }

        index
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The chunks in order, for the tests and for a full listing.
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    fn average_length(&self) -> f32 {
        if self.lengths.is_empty() {
            0.0
        } else {
            self.lengths.iter().sum::<usize>() as f32 / self.lengths.len() as f32
        }
    }

    /// Stage 1: the best `k` chunks for a query, fused from the name, the declaration and BM25.
    pub fn search(&self, query: &str, k: usize) -> Vec<Hit> {
        let wanted = terms(query);

        if wanted.is_empty() || self.chunks.is_empty() {
            return Vec::new();
        }

        let unique: Vec<&String> = {
            let mut seen = HashSet::new();

            wanted.iter().filter(|word| seen.insert((*word).clone())).collect()
        };
        let average = self.average_length().max(1.0);
        let total = self.chunks.len() as f32;
        let mut scored: Vec<Hit> = Vec::new();

        for (position, chunk) in self.chunks.iter().enumerate() {
            let words = terms(&chunk.text);
            let mut score = 0.0f32;
            let mut matched = 0;

            for word in &unique {
                let frequency = words.iter().filter(|candidate| candidate == word).count();

                if frequency == 0 {
                    continue;
                }

                matched += 1;

                let df = *self.document_frequency.get(*word).unwrap_or(&0) as f32;
                let idf = ((total - df + 0.5) / (df + 0.5) + 1.0).ln();
                let length = self.lengths[position] as f32;

                score += idf * (frequency as f32 * (K1 + 1.0)) / (frequency as f32 + K1 * (1.0 - B + B * length / average));
            }

            if matched == 0 {
                continue;
            }

            /* a declaration of a name the question uses is a stronger signal than the word appearing in prose */
            let declares = chunk_is_declaration(&chunk.text, &unique);
            let named = name_matches(&chunk.path, &unique);
            let why = if declares {
                "declares"
            } else if named {
                "name"
            } else {
                "text"
            };

            if declares {
                score += 3.0;
            }

            if named {
                score += 1.5;
            }

            scored.push(Hit { path: chunk.path.clone(), start_line: chunk.start_line, end_line: chunk.end_line, score, why, text: chunk.text.clone() });
        }

        scored.sort_by(|left, right| right.score.total_cmp(&left.score).then_with(|| left.path.cmp(&right.path)).then_with(|| left.start_line.cmp(&right.start_line)));
        scored.truncate(k.max(1));
        scored
    }
}

/// Does this chunk's first line declare one of the words (`fn charge(`, `class Ledger`)?
fn chunk_is_declaration(text: &str, words: &[&String]) -> bool {
    let first = text.lines().next().unwrap_or_default();

    words.iter().any(|word| {
        let lower = first.to_ascii_lowercase();

        lower.contains(&format!("fn {word}")) || lower.contains(&format!("def {word}")) || lower.contains(&format!("class {word}")) || lower.contains(&format!("struct {word}")) || lower.contains(&format!("function {word}")) || lower.contains(&format!("const {word}"))
    })
}

/// Does the file's name carry one of the words?
fn name_matches(path: &str, words: &[&String]) -> bool {
    let lower = path.to_ascii_lowercase();

    words.iter().any(|word| lower.contains(word.as_str()))
}

/// Reads the project into an index. `progress(done, total)` is called as files are read, so the window can say `reading 12/40`.
pub fn build(root: &Path, mut progress: impl FnMut(usize, usize)) -> Index {
    /* the Code Graph already knows the project's files (and does not walk into `.sdc/`, `.git/` or a vendor folder); a project too big for
       it is read from its first `MAX_FILES` entries */
    let (graph, _) = crate::graph::index::refreshed(root);
    let files: Vec<String> = graph.files.keys().take(MAX_FILES).cloned().collect();
    let total = files.len();
    let contents: Vec<(String, String)> = files
        .iter()
        .enumerate()
        .filter_map(|(index, relative)| {
            progress(index + 1, total);

            let path = root.join(relative);
            let meta = std::fs::metadata(&path).ok()?;

            if meta.len() > MAX_FILE_BYTES {
                return None;
            }

            let text = std::fs::read_to_string(&path).ok()?;

            Some((relative.clone(), text))
        })
        .collect();

    Index::of(contents.iter().map(|(path, text)| (path.as_str(), text.as_str())))
}

/// Stage 1 for a folder, in one call.
pub fn retrieve(root: &Path, query: &str, k: usize) -> Vec<Hit> {
    build(root, |_, _| {}).search(query, k)
}

/// The skim the model reads: one line each - `path:start-end` and the chunk's first line - then, for the best hit only, a few of its
/// lines. Enough to choose, short enough to be worth the tokens.
pub fn skim(hits: &[Hit], lines_of_top: usize) -> String {
    if hits.is_empty() {
        return String::new();
    }

    let mut out: Vec<String> = hits
        .iter()
        .map(|hit| {
            let first = hit.text.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().trim();
            let shown: String = first.chars().take(100).collect();

            format!("{}:{}-{} [{}] {shown}", hit.path, hit.start_line, hit.end_line, hit.why)
        })
        .collect();

    let top = &hits[0];

    out.push(String::new());
    out.push(format!("-- {}:{}-{}", top.path, top.start_line, top.end_line));

    for line in top.text.lines().take(lines_of_top) {
        out.push(line.to_string());
    }

    out.join("\n")
}

/// Stage 2 is on only when the owner turned it on: the plan's rule is that it ships if it lifts recall on the bench, and nothing has
/// measured that yet. `SDC_RERANK=on` is the switch, beside the feature flag, so a bench run can compare the two orders.
pub fn rerank_enabled() -> bool {
    crate::v3::on("sme.retrieval") && std::env::var("SDC_RERANK").map(|value| value == "on").unwrap_or(false)
}

/// Content-hash-keyed summaries: the same bytes are never summarised twice.
#[derive(Default)]
pub struct Summaries {
    map: Mutex<HashMap<String, String>>,
}

impl Summaries {
    pub fn get(&self, hash: &str) -> Option<String> {
        self.map.lock().unwrap_or_else(|poison| poison.into_inner()).get(hash).cloned()
    }

    /// Stores (or replaces) a chunk's summary.
    pub fn put(&self, hash: &str, summary: &str) {
        let mut map = self.map.lock().unwrap_or_else(|poison| poison.into_inner());

        if map.len() > 2_000 {
            map.clear();
        }

        map.insert(hash.to_string(), summary.to_string());
    }

    /// A summary when one is known, otherwise asks `write` and keeps the answer.
    pub fn get_or(&self, hash: &str, write: impl FnOnce() -> Option<String>) -> Option<String> {
        if let Some(known) = self.get(hash) {
            return Some(known);
        }

        let written = write()?;

        self.put(hash, &written);
        Some(written)
    }
}

#[cfg(test)]
mod tests;
