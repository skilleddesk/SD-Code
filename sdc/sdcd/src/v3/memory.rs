//! **Memory** (v3 L0, switch `v3.memory`, a token technique - off until it passes Q3): v1 sends the project's whole
//! `.sdc/memory.md` in front of every request. A project that has been used for months carries hundreds of facts
//! then, nearly all of them nothing to do with the question. With this switch on, only the facts the request
//! touches travel - and when none of them does, the newest few, so the shape of the project still shows.
//!
//! `consolidate` is the writing half: the same fact written twice (in different words) is kept once, newest
//! wording, so a long-lived memory file stays readable and cheap.

use std::collections::HashSet;
use std::path::Path;

pub const FILE: &str = ".sdc/memory.md";

/// The part of the memory file SDC writes itself; everything above it is the person's.
const LEARNED: &str = "## Learned by SDC";

/// At most this many facts SDC wrote itself are kept - the oldest go first.
const MAX_LEARNED: usize = 12;

/// A check command without what was piped or chained onto it this time (`2>&1 | tail -20`, `&& python3 - <<'EOF'`):
/// the same check run two ways is one fact, and a one-off probe stuck to it is not part of the check.
pub fn check_command(command: &str) -> &str {
    let command = command.trim();
    let end = [" 2>&1", " |", " &&", " ;", " <<", " >"].iter().filter_map(|cut| command.find(cut)).min().unwrap_or(command.len());

    command[..end].trim()
}

/// The fact a passing command teaches, when it is a check (tests, build, lint, types) rather than a look around.
pub fn check_fact(command: &str) -> Option<String> {
    const CHECKS: &[&str] = &["test", "build", "lint", "check", "clippy", "tsc", "pytest", "vet", "compile", "typecheck"];
    let command = check_command(command);
    let lower = command.to_lowercase();

    if command.is_empty() || command.chars().count() > 120 || command.contains('\n') || !CHECKS.iter().any(|word| lower.contains(word)) {
        return None;
    }

    Some(format!("`{command}` passes here after a change - it is how this project checks its work."))
}

/// Writes a fact SDC learned into the memory file, below the person's own, once. Answers whether it was new.
pub fn remember(root: &Path, fact: &str) -> bool {
    /* M3-1: a fact SDC learned from a passing check was shown by a tool: `[observed]` (Memory 2.0). */
    let marked;
    let fact = if crate::v3::on("memory.v2") && !fact.trim_start().starts_with('[') {
        marked = format!("{} {}", crate::v3::memory_v2::Origin::Observed.tag(), fact.trim());
        marked.as_str()
    } else {
        fact
    };
    let path = root.join(FILE);
    let text = std::fs::read_to_string(&path).unwrap_or_default();

    if facts(&text).iter().any(|known| known == fact.trim()) {
        return false;
    }

    let (person, learned) = match text.split_once(LEARNED) {
        Some((person, learned)) => (person.trim_end().to_string(), facts(learned)),
        None => (text.trim_end().to_string(), Vec::new()),
    };
    let mut learned = learned;

    learned.push(fact.trim().to_string());

    let start = learned.len().saturating_sub(MAX_LEARNED);
    let lines: Vec<String> = learned[start..].iter().map(|fact| format!("- {fact}")).collect();
    let head = if person.is_empty() { String::new() } else { format!("{person}\n\n") };

    std::fs::create_dir_all(path.parent().unwrap_or(root)).is_ok() && std::fs::write(&path, format!("{head}{LEARNED}\n\n{}\n", lines.join("\n"))).is_ok()
}

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
    fn sdc_writes_a_learned_check_below_the_persons_facts_once_and_keeps_the_newest() {
        let root = std::env::temp_dir().join(format!("sdc-memory-learn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sdc")).unwrap();
        std::fs::write(root.join(FILE), "- Use pnpm, never npm\n").unwrap();

        assert_eq!(check_fact("ls src"), None, "a look around is not a check");
        let fact = check_fact("pnpm test").expect("a test run is");

        assert_eq!(check_fact("pnpm test 2>&1 | tail -20"), Some(fact.clone()), "the same check piped another way is the same fact");
        assert_eq!(check_command("python3 -m unittest -v && python3 - <<'EOF'\nprint(1)\nEOF"), "python3 -m unittest -v");

        assert!(remember(&root, &fact));
        assert!(!remember(&root, &fact), "the same fact is written once");

        let text = std::fs::read_to_string(root.join(FILE)).unwrap();

        assert!(text.starts_with("- Use pnpm, never npm\n\n## Learned by SDC"), "the person's facts stay first:\n{text}");

        for index in 0..20 {
            remember(&root, &check_fact(&format!("cargo test -p crate{index}")).unwrap());
        }

        let text = std::fs::read_to_string(root.join(FILE)).unwrap();

        assert_eq!(facts(&text).len(), 1 + MAX_LEARNED, "only the newest learned facts are kept:\n{text}");
        assert!(text.contains("crate19") && !text.contains("pnpm test") && text.contains("Use pnpm, never npm"));

        let _ = std::fs::remove_dir_all(&root);
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
