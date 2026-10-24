//! **Sentence-level citations (M4-2, switch `research.sentence_cite`, off)**: a `/research` answer's claim points at the exact sentences of the page that
//! say it, and SDC checks the pointer in code.
//!
//! A page is read as numbered sentences - `[2:14] The limit is 100 requests.` is page 2, sentence 14 - and the answer cites `[2:14]` or `[2:14-16]`. The
//! check that follows is words, not a second model: the content words of the claim must be found in the sentences it cites (60 %), the
//! sentences must exist, and the page must have been read, not merely listed in a search result. A claim that fails is **flagged** after the answer
//! with the reason - the answer is not rewritten.

use std::collections::BTreeSet;

use regex::Regex;

/// The most sentences kept of one page, and the longest one.
const MAX_SENTENCES: usize = 600;
const MAX_CHARS: usize = 400;

/// The share of a claim's content words that the cited sentences must contain.
const SUPPORT_PERCENT: usize = 60;

/// A page as sentences: split at `. ! ? ।` followed by a space or a line end, and at every line break (a list item is a sentence).
pub fn sentences(text: &str) -> Vec<String> {
    let mut found = Vec::new();

    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let mut current = String::new();
        let mut chars = line.chars().peekable();

        while let Some(c) = chars.next() {
            current.push(c);

            if matches!(c, '.' | '!' | '?' | '।') && chars.peek().is_none_or(|next| next.is_whitespace()) {
                found.push(std::mem::take(&mut current));
            }
        }

        found.push(current);
    }

    found
        .into_iter()
        .map(|sentence| sentence.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|sentence| sentence.split_whitespace().count() >= 3)
        .map(|sentence| sentence.chars().take(MAX_CHARS).collect::<String>())
        .take(MAX_SENTENCES)
        .collect()
}

/// A source's sentences as the model reads them: `[n:i] sentence`, one per line.
pub fn numbered(n: usize, sentences: &[String]) -> String {
    sentences.iter().enumerate().map(|(index, sentence)| format!("[{n}:{}] {sentence}", index + 1)).collect::<Vec<_>>().join("\n")
}

/// One pointer in an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    pub source: usize,
    pub from: usize,
    pub to: usize,
    /// What the answer says just before the pointer.
    pub claim: String,
}

/// The pointers of an answer, each with the claim it ends.
pub fn citations(answer: &str) -> Vec<Citation> {
    let marker = Regex::new(r"\[(\d+):(\d+)(?:\s*[-–]\s*(\d+))?\]").expect("a valid pattern");
    let mut found = Vec::new();
    let mut claim_from = 0usize;

    for capture in marker.captures_iter(answer) {
        let whole = capture.get(0).expect("a match");
        let before = &answer[claim_from..whole.start()];
        /* The claim is the last sentence before the pointer: after the previous full stop or line break. */
        let start = before.rfind(['\n', '。', '।']).map_or(0, |at| at + 1);
        let start = before.rmatches(". ").next().map_or(start, |_| before.rfind(". ").map_or(start, |at| (at + 2).max(start)));
        let claim = before[start.min(before.len())..].trim().trim_start_matches(['-', '*', ' ']).to_string();
        let number = |index: usize| capture.get(index).and_then(|part| part.as_str().parse::<usize>().ok());
        let (source, from) = (number(1).unwrap_or(0), number(2).unwrap_or(0));
        let to = number(3).unwrap_or(from);

        found.push(Citation { source, from: from.min(to), to: from.max(to), claim });
        claim_from = whole.end();
    }

    found
}

fn content_words(text: &str) -> BTreeSet<String> {
    const STOP: &[&str] = &["that", "this", "with", "from", "have", "has", "been", "which", "their", "there", "about", "into", "than", "then", "they", "them", "were", "will", "would", "also", "when", "what", "your", "such", "more", "most", "some", "only", "very"];

    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| (word.chars().count() >= 4 || (!word.is_ascii() && word.chars().count() >= 3) || word.chars().all(|c| c.is_ascii_digit())) && !STOP.contains(&word.as_str()))
        .collect()
}

/// What the check found for one pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    Supported,
    /// The claim's words are not in the cited sentences (the share found, in percent).
    Unsupported { found: usize },
    /// The pointer names sentences the page does not have.
    OutOfRange { sentences: usize },
    /// The source was not read in full (only seen in a search result), or there is no such source.
    NotRead,
}

/// Is the claim in the cited sentences? A claim with no content words has nothing to check and passes.
pub fn support(citation: &Citation, page: Option<&[String]>) -> Support {
    let Some(page) = page else {
        return Support::NotRead;
    };

    if citation.from == 0 || citation.to > page.len() {
        return Support::OutOfRange { sentences: page.len() };
    }

    let cited = page[citation.from - 1..citation.to].join(" ");
    let (claim, have) = (content_words(&citation.claim), content_words(&cited));

    if claim.is_empty() {
        return Support::Supported;
    }

    let found = claim.iter().filter(|word| have.contains(*word)).count() * 100 / claim.len();

    if found >= SUPPORT_PERCENT { Support::Supported } else { Support::Unsupported { found } }
}

/// Every pointer of an answer, checked. `page_of` answers a source's sentences when it was read in full.
pub fn check(answer: &str, page_of: &dyn Fn(usize) -> Option<Vec<String>>) -> Vec<(Citation, Support)> {
    citations(answer)
        .into_iter()
        .map(|citation| {
            let page = page_of(citation.source);
            let verdict = support(&citation, page.as_deref());

            (citation, verdict)
        })
        .collect()
}

/// The note after the answer: the claims that did not hold. `None` when every pointer held (or there was none).
pub fn report(results: &[(Citation, Support)]) -> Option<String> {
    let bad: Vec<String> = results
        .iter()
        .filter(|(_, verdict)| *verdict != Support::Supported)
        .map(|(citation, verdict)| {
            let at = if citation.from == citation.to { format!("[{}:{}]", citation.source, citation.from) } else { format!("[{}:{}-{}]", citation.source, citation.from, citation.to) };
            let why = match verdict {
                Support::Unsupported { found } => format!("only {found}% of its words are in the cited sentences"),
                Support::OutOfRange { sentences } => format!("that page has {sentences} sentence(s)"),
                Support::NotRead => "that source's page was not read in full".to_string(),
                Support::Supported => String::new(),
            };

            format!("- {at} \"{}\" - {why}", citation.claim.chars().take(120).collect::<String>())
        })
        .collect();

    (!bad.is_empty()).then(|| format!("**Citation check:** {} of {} cited claim(s) are not backed by the sentences they point at:\n{}", bad.len(), results.len(), bad.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "The service allows 100 requests per minute. Keys are rotated every 90 days! Is it free? Only paid plans get priority support.\n- Webhooks retry three times\nOK.";

    #[test]
    fn a_page_becomes_numbered_sentences() {
        let page = sentences(PAGE);

        assert_eq!(page, ["The service allows 100 requests per minute.", "Keys are rotated every 90 days!", "Is it free?", "Only paid plans get priority support.", "- Webhooks retry three times"]);
        assert_eq!(numbered(2, &page[..2]), "[2:1] The service allows 100 requests per minute.\n[2:2] Keys are rotated every 90 days!");
        assert!(sentences("").is_empty() && sentences("Hi.").is_empty(), "a fragment is not a sentence worth citing");
        assert_eq!(sentences("It costs 3.5 dollars today.").len(), 1, "a decimal point is not a sentence end");
    }

    #[test]
    fn pointers_are_found_with_the_claim_they_end() {
        let found = citations("The limit is 100 requests per minute [2:1]. Keys rotate every 90 days [2:2-3]. Plain text.\n- Webhooks retry [3:5]");

        assert_eq!(found.len(), 3);
        assert_eq!((found[0].source, found[0].from, found[0].to), (2, 1, 1));
        assert_eq!(found[0].claim, "The limit is 100 requests per minute");
        assert_eq!((found[1].from, found[1].to), (2, 3));
        assert_eq!(found[1].claim, "Keys rotate every 90 days");
        assert_eq!(found[2].claim, "Webhooks retry");
        assert_eq!(citations("[2:3-1]")[0].from, 1, "a reversed range is read the right way round");
        assert!(citations("no pointers [1] here").is_empty(), "[1] is a source number, not a sentence pointer");
    }

    #[test]
    fn a_claim_is_supported_when_most_of_its_words_are_in_the_cited_sentences() {
        let page = sentences(PAGE);
        let good = Citation { source: 2, from: 1, to: 1, claim: "The service allows 100 requests every minute".into() };
        let bad = Citation { source: 2, from: 2, to: 2, claim: "The service allows unlimited requests".into() };
        let range = Citation { source: 2, from: 9, to: 9, claim: "anything".into() };

        assert_eq!(support(&good, Some(&page)), Support::Supported);
        assert!(matches!(support(&bad, Some(&page)), Support::Unsupported { found } if found < 60));
        assert_eq!(support(&range, Some(&page)), Support::OutOfRange { sentences: 5 });
        assert_eq!(support(&good, None), Support::NotRead);
        assert_eq!(support(&Citation { source: 2, from: 1, to: 1, claim: "It is so".into() }, Some(&page)), Support::Supported, "nothing to check");
    }

    #[test]
    fn the_report_flags_only_what_did_not_hold_and_says_why() {
        let page = sentences(PAGE);
        let answer = "The service allows 100 requests per minute [2:1]. Everything is free of charge [2:3]. Webhooks retry hourly [4:1].";
        let results = check(answer, &|n| (n == 2).then(|| page.clone()));
        let note = report(&results).expect("two claims did not hold");

        assert!(note.starts_with("**Citation check:** 2 of 3 cited claim(s)"), "{note}");
        assert!(note.contains("[2:3]") && note.contains("only ") && note.contains("[4:1]") && note.contains("not read in full"));
        assert!(!note.contains("100 requests"), "the supported claim is not flagged");
        assert_eq!(report(&check("The service allows 100 requests per minute [2:1].", &|_| Some(page.clone()))), None);
        assert_eq!(report(&[]), None);
    }
}
