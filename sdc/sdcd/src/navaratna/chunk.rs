//! **N4-4 - output chunking and continuation** (switch `nr.out_chunk`).
//!
//! A model stops writing after a couple of thousand words, silently. A long document is therefore written **section by section**; every
//! section ends with the marker `<<SECTION n END>>`. A section without its marker is incomplete: a continuation call is made with the last
//! thirty lines as context. Sections are kept as separate files and joined by a manifest - and a document with a section that never
//! completed is reported, never joined half-done.

use std::path::{Path, PathBuf};

/// Lines of the unfinished section the continuation call is shown.
pub const CONTEXT_LINES: usize = 30;
/// Continuations tried for one section before it is reported incomplete.
pub const MAX_CONTINUATIONS: usize = 3;

pub fn marker(n: usize) -> String {
    format!("<<SECTION {n} END>>")
}

/// Is the section finished: does its last non-empty line carry its marker?
pub fn finished(text: &str, n: usize) -> bool {
    text.trim_end().ends_with(&marker(n))
}

/// The section text without its marker.
pub fn strip_marker(text: &str, n: usize) -> String {
    text.trim_end().trim_end_matches(&marker(n)).trim_end().to_string()
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();

    lines[lines.len().saturating_sub(CONTEXT_LINES)..].join("\n")
}

/// The model's side of the work: given a prompt, the text it wrote.
pub trait Writer {
    fn write(&mut self, prompt: &str) -> String;
}

impl<F: FnMut(&str) -> String> Writer for F {
    fn write(&mut self, prompt: &str) -> String {
        self(prompt)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub n: usize,
    pub title: String,
    pub file: String,
    pub complete: bool,
    pub continuations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub name: String,
    pub sections: Vec<Entry>,
}

impl Manifest {
    pub fn complete(&self) -> bool {
        !self.sections.is_empty() && self.sections.iter().all(|entry| entry.complete)
    }

    pub fn incomplete(&self) -> Vec<usize> {
        self.sections.iter().filter(|entry| !entry.complete).map(|entry| entry.n).collect()
    }
}

fn dir_of(root: &Path, name: &str) -> PathBuf {
    root.join(".sdc/docs").join(name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>())
}

/// Writes a document of `titles`, one section at a time, and stores the sections and the manifest. Answers the manifest; whether the
/// document is whole is `manifest.complete()`.
pub fn write_document(root: &Path, name: &str, goal: &str, titles: &[&str], writer: &mut dyn Writer) -> Result<Manifest, String> {
    let dir = dir_of(root, name);

    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;

    let mut manifest = Manifest { name: name.to_string(), sections: Vec::new() };

    for (index, title) in titles.iter().enumerate() {
        let n = index + 1;
        let mut text = writer.write(&format!(
            "Goal: {goal}\nWrite section {n} of {}: \"{title}\". Write only this section, and end it with the line {} exactly.",
            titles.len(),
            marker(n)
        ));
        let mut continuations = 0;

        while !finished(&text, n) && continuations < MAX_CONTINUATIONS {
            continuations += 1;

            let more = writer.write(&format!(
                "Goal: {goal}\nSection {n} (\"{title}\") stopped before its end. The last lines written are:\n---\n{}\n---\nContinue exactly where it stopped, without repeating, and end with the line {}.",
                tail(&text),
                marker(n)
            ));

            text.push_str(if text.ends_with('\n') { "" } else { "\n" });
            text.push_str(&more);
        }

        let complete = finished(&text, n);
        let file = format!("section-{n:03}.md");

        std::fs::write(dir.join(&file), if complete { strip_marker(&text, n) } else { text.clone() }).map_err(|error| error.to_string())?;
        manifest.sections.push(Entry { n, title: title.to_string(), file, complete, continuations });
    }

    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    Ok(manifest)
}

/// The whole document, joined from its sections in manifest order - or the list of sections that never completed.
pub fn join(root: &Path, name: &str) -> Result<String, Vec<usize>> {
    let dir = dir_of(root, name);
    let manifest: Manifest = std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|text| serde_json::from_str(&text).ok()).ok_or_else(Vec::new)?;

    if !manifest.complete() {
        return Err(manifest.incomplete());
    }

    let parts: Vec<String> = manifest.sections.iter().filter_map(|entry| std::fs::read_to_string(dir.join(&entry.file)).ok()).collect();

    if parts.len() != manifest.sections.len() {
        return Err(Vec::new());
    }

    Ok(parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake model that stops mid-section the first time it writes section 2 and again once more.
    struct Stopper {
        cuts_left: usize,
        calls: Vec<String>,
    }

    impl Writer for Stopper {
        fn write(&mut self, prompt: &str) -> String {
            self.calls.push(prompt.to_string());

            if prompt.contains("stopped before its end") {
                return if self.cuts_left > 0 && prompt.contains("Section 3") { self.cuts_left -= 1; "still going".to_string() } else { "and the end.\n".to_string() + &marker(if prompt.contains("Section 2") { 2 } else { 3 }) };
            }

            if prompt.contains("section 2 of 3") {
                return (1..=40).map(|n| format!("line {n}\n")).collect::<String>();
            }

            let n = if prompt.contains("section 1 of 3") { 1 } else { 3 };

            if n == 3 && self.cuts_left > 0 {
                return "half a sentence".to_string();
            }

            format!("text of section {n}\n{}", marker(n))
        }
    }

    #[test]
    fn a_section_that_stops_is_continued_from_its_last_thirty_lines_and_joined_whole() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = Stopper { cuts_left: 0, calls: Vec::new() };
        let manifest = write_document(dir.path(), "guide", "a guide", &["one", "two", "three"], &mut writer).unwrap();

        assert!(manifest.complete(), "{manifest:?}");
        assert_eq!(manifest.sections[1].continuations, 1);

        let continuation = writer.calls.iter().find(|call| call.contains("stopped before its end")).unwrap();

        assert!(continuation.contains("line 40") && continuation.contains("line 11") && !continuation.contains("line 10\n"), "the last 30 lines only: {continuation}");

        let joined = join(dir.path(), "guide").unwrap();

        assert!(joined.contains("text of section 1") && joined.contains("line 40\nand the end.") && joined.contains("text of section 3"));
        assert!(!joined.contains("<<SECTION"), "markers are stripped");
    }

    #[test]
    fn a_section_that_never_completes_is_reported_and_the_document_is_not_joined() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = Stopper { cuts_left: 99, calls: Vec::new() };
        let manifest = write_document(dir.path(), "guide", "a guide", &["one", "two", "three"], &mut writer).unwrap();

        assert!(!manifest.complete());
        assert_eq!(manifest.incomplete(), vec![3]);
        assert_eq!(manifest.sections[2].continuations, MAX_CONTINUATIONS);
        assert_eq!(join(dir.path(), "guide"), Err(vec![3]), "never joined half-done");
    }

    #[test]
    fn the_marker_must_be_the_end_of_the_text() {
        assert!(finished("abc\n<<SECTION 4 END>>\n", 4));
        assert!(!finished("abc <<SECTION 4 END>> and more", 4));
        assert!(!finished("abc\n<<SECTION 5 END>>", 4));
    }
}
