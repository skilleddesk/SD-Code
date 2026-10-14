//! **Memory** (v3 L0, switch `v3.memory`, a token technique - off until it passes Q3): v1 sends the project's whole
//! `.sdc/memory.md` in front of every request. A project that has been used for months carries hundreds of facts
//! then, nearly all of them nothing to do with the question. With this switch on, only the facts the request
//! touches travel - and when none of them does, the newest few, so the shape of the project still shows.
//!
//! `consolidate` is the writing half: the same fact written twice (in different words) is kept once, newest
//! wording, so a long-lived memory file stays readable and cheap.

use std::collections::HashSet;

/// At most this many facts travel with one request.
pub const MAX_FACTS: usize = 24;

/// A memory file's facts: `- ...` lines, in the order they were written.
fn facts(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|line| line.starts_with("- ") || line.starts_with("* ")).map(|line| line.trim_start_matches(['-', '*']).trim().to_string()).filter(|fact| !fact.is_empty()).collect()
}

/// The words of a fact or request that can match: 4+ letters, lowercase, numbers kept.
fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '/' && c != '-')
        .map(str::to_lowercase)
        .filter(|word| word.chars().count() >= 4)
        .collect()
}

/// The fewest facts sent, however little the request touches: enough for the shape of the project to show.
const MIN_FACTS: usize = 5;

/// The facts to send for a request: the ones it shares words with, best match first - and, when fewer than
/// [`MIN_FACTS`] match, the newest facts until that many do.
pub fn select(text: &str, request: &str, max: usize) -> String {
    let facts = facts(text);

    if facts.len() <= MIN_FACTS {
        return text.trim().to_string();
    }

    let wanted = words(request);
    let mut scored: Vec<(usize, usize)> = facts
        .iter()
        .enumerate()
        .map(|(index, fact)| (words(fact).intersection(&wanted).count(), index))
        .filter(|(shared, _)| *shared > 0)
        .collect();

    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

    let mut chosen: Vec<usize> = scored.into_iter().take(max).map(|(_, index)| index).collect();

    for index in (0..facts.len()).rev() {
        if chosen.len() >= MIN_FACTS {
            break;
        }

        if !chosen.contains(&index) {
            chosen.push(index);
        }
    }

    chosen.sort();

    let kept: Vec<String> = chosen.iter().map(|index| format!("- {}", facts[*index])).collect();

    format!("{}\n[SDC: {} of {} facts kept - the ones this request touches, and the newest of the rest.]", kept.join("\n"), kept.len(), facts.len())
}

/// The same fact twice (in different words) is kept once: the newest wording wins, the order is kept.
pub fn consolidate(text: &str) -> String {
    let facts = facts(text);
    let mut seen: Vec<HashSet<String>> = Vec::new();
    let mut kept: Vec<String> = Vec::new();

    for fact in facts.iter().rev() {
        let theirs = words(fact);

        if seen.iter().any(|theirs| similar(theirs, &words(fact))) {
            continue;
        }

        seen.push(theirs);
        kept.push(fact.clone());
    }

    kept.reverse();

    if kept.len() == facts.len() {
        return text.trim_end().to_string();
    }

    format!("{}\n", kept.iter().map(|fact| format!("- {fact}")).collect::<Vec<_>>().join("\n"))
}

/// Two facts are one fact when they share most of their words (and neither has far more words than the other).
fn similar(left: &HashSet<String>, right: &HashSet<String>) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }

    let shared = left.intersection(right).count();

    shared * 5 >= left.len().min(right.len()) * 4 && left.len() <= right.len() * 3 && right.len() <= left.len() * 3
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMORY: &str = "- Use pnpm, never npm\n- The orders list lives in src/orders.ts\n- Answer the person in Bangla when they write Banglish\n- Never commit .env\n- Use pnpm in CI too\n- Theme colours come from tokens.css\n";

    #[test]
    fn only_the_facts_the_request_touches_travel() {
        let selected = select(MEMORY, "the orders list shows the wrong total in src/orders.ts", 24);

        assert!(selected.contains("src/orders.ts"), "{selected}");
        assert!(selected.contains("Never commit .env"), "the newest facts fill the rest");
        assert!(selected.contains("facts kept"), "and it says what it left out");
    }

    #[test]
    fn a_short_memory_is_sent_as_it_is() {
        assert_eq!(select("- one\n- two\n", "anything", 24), "- one\n- two");
    }

    #[test]
    fn the_same_fact_twice_is_kept_once_with_the_newest_wording() {
        let consolidated = consolidate(MEMORY);

        assert!(consolidated.contains("Use pnpm in CI too"), "the newest wording stays:\n{consolidated}");
        assert!(!consolidated.contains("Use pnpm, never npm"), "the older one goes:\n{consolidated}");
        assert_eq!(consolidated.lines().count(), 5);
        assert_eq!(consolidate("").trim(), "");
    }
}
