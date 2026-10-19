//! **The quote verifier (P1-9, switch `sme.quote_verify`)**: a note about a file is worth what its quote is worth, and a small model
//! produces quotations that *look* right and are not in the file - a line reworded from memory, a signature with one parameter missing,
//! a variable renamed to the one it expected. Every quote a tool is given is checked against the file before the note is kept.
//!
//! A quote is accepted when it is in the file **exactly**, or when it differs from it **only in whitespace** (indentation, line breaks,
//! the spaces after a comma) - which is how a model's copy of a line usually differs and is harmless. It is refused when a token
//! differs, when it spans lines that do not run together in that order, or when the file does not have it at all: then the answer is
//! "read it again", and nothing is remembered.
//!
//! The rate this watches is the **fabricated-quote rate**: the share of notes whose quote is not in the file it names. The test below
//! measures it on mutations of real lines (a word dropped, a token swapped, a line reordered) and requires it to be exactly zero, because
//! a quote that cannot be trusted is worse than no quote: it carries the authority of the source into a sentence nobody wrote.

use std::path::Path;

/// The most a quote may be, and the most of a file that is searched.
const MAX_QUOTE: usize = 400;
const MAX_FILE: u64 = 2_000_000;

/// Whitespace as a comparison ignores it: runs collapse to one space, and the ends are trimmed.
pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the quote is in the text (byte offset of the first line it starts on), or `None`. Exact first, then whitespace-insensitive.
pub fn find(text: &str, quote: &str) -> Option<usize> {
    let quote = quote.trim();

    if quote.is_empty() {
        return None;
    }

    if let Some(at) = text.find(quote) {
        return Some(at);
    }

    let wanted = normalize(quote);

    if wanted.is_empty() {
        return None;
    }

    /* a window of lines at a time: a quote may span lines, and the comparison ignores how they were wrapped */
    let lines: Vec<&str> = text.lines().collect();
    let mut offset = 0;

    for (start, line) in lines.iter().enumerate() {
        let mut candidate = String::new();

        for line in &lines[start..] {
            if !candidate.is_empty() {
                candidate.push(' ');
            }

            candidate.push_str(line);

            let normalized = normalize(&candidate);

            if normalized.len() >= wanted.len() {
                if normalized.contains(&wanted) {
                    return Some(offset);
                }

                break;
            }
        }

        offset += line.len() + 1;
    }

    None
}

/// Checks a quote against a file: `Ok(())` when it is there, `Err(the sentence for the model)` when it is not.
pub fn verify(root: &Path, source: &str, quote: &str) -> Result<(), String> {
    let quote = quote.trim();

    if source.trim().is_empty() {
        return Err("a quote needs a `source` file: quote the file the line is in, or leave both out".into());
    }

    if quote.is_empty() {
        return Err("a `quote` is empty: quote the line itself, or leave it out".into());
    }

    if quote.len() > MAX_QUOTE {
        return Err(format!("the quote is {} characters: quote one line (under {MAX_QUOTE}), not the file", quote.len()));
    }

    let path = root.join(source.trim_start_matches("./"));

    if path.metadata().map(|meta| meta.len() > MAX_FILE).unwrap_or(true) {
        return Err(format!("{source} is not a file SDC can read here, so the quote cannot be checked. Read it again and quote a line from what you read."));
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Err(format!("{source} could not be read as text, so the quote cannot be checked. Read it again.")),
    };

    if find(&text, quote).is_some() {
        return Ok(());
    }

    /* the most useful sentence names the line that is nearly it: where the quote's own first token appears */
    let first = quote.split_whitespace().next().unwrap_or_default();
    let near = text
        .lines()
        .enumerate()
        .find(|(_, line)| !first.is_empty() && line.contains(first))
        .map(|(index, line)| format!(" The nearest line is {}: `{}`", index + 1, line.trim().chars().take(120).collect::<String>()))
        .unwrap_or_default();

    Err(format!("the quote is not in {source}, so the note was not kept.{near} Read the file again and quote it exactly."))
}

#[cfg(test)]
mod tests;
