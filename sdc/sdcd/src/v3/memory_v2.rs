//! **Memory 2.0 (M3, switch `memory.v2`, off)**: what SDC remembers about a project says where each fact came from, is tidied after a turn rather
//! than during it, is offered to the model as an index rather than a pile, and stays small and lasting.
//!
//! * **Origin (M3-1).** Every fact in `.sdc/memory.md` (and every recipe and lesson line) carries `[stated]` (the person said it), `[observed]` (a tool
//!   or a test showed it) or `[inferred]` (a model guessed). An `[inferred]` fact is never given to the model as an instruction. A line with no mark -
//!   every line written before this - is read as `[observed]`, and [`migrate`] writes the mark in.
//! * **After the turn (M3-2).** A background pass, run once the turn is over and in code (the diff, the checks that passed, the person's own
//!   "remember that ..."), adds what is new, replaces what the person corrected and merges repeats. Nothing writes to memory while a turn runs.
//! * **An index (M3-3).** What travels with a request is one line per memory file - what it is for and how big - and the model reads the file it
//!   needs with `read_file`. A small memory (five facts or fewer) still travels whole.
//! * **Small and lasting (M3-4).** A file has a size limit; a fact that will not be true in a month ("the build is failing right now") is not
//!   kept; repeats are merged; when the limit is reached the weakest facts go first (`[inferred]`, then `[observed]`, oldest first) and what the
//!   person stated is never dropped.

use std::collections::HashSet;
use std::path::Path;

/// Where a fact came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// A model guessed. Never an instruction.
    Inferred,
    /// A tool, a test or the code showed it.
    Observed,
    /// The person said it.
    Stated,
}

impl Origin {
    pub fn tag(self) -> &'static str {
        match self {
            Origin::Inferred => "[inferred]",
            Origin::Observed => "[observed]",
            Origin::Stated => "[stated]",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "[inferred]" => Some(Origin::Inferred),
            "[observed]" => Some(Origin::Observed),
            "[stated]" => Some(Origin::Stated),
            _ => None,
        }
    }

    /// May the model be told this fact as something to follow?
    pub fn may_instruct(self) -> bool {
        self != Origin::Inferred
    }
}

/// One memory line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub origin: Origin,
    pub text: String,
    /// The line had no mark and was read as `[observed]`.
    pub unmarked: bool,
}

/// The facts of a text (`- ...` or `* ...` lines), oldest first.
pub fn parse(text: &str) -> Vec<Fact> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- ") || line.starts_with("* "))
        .map(|line| line[2..].trim())
        .filter(|body| !body.is_empty())
        .map(|body| {
            let (first, rest) = body.split_once(' ').unwrap_or((body, ""));

            match Origin::from_tag(first) {
                Some(origin) => Fact { origin, text: rest.trim().to_string(), unmarked: false },
                None => Fact { origin: Origin::Observed, text: body.to_string(), unmarked: true },
            }
        })
        .collect()
}

/// A fact as it is written: `- [origin] text`.
pub fn line(fact: &Fact) -> String {
    format!("- {} {}", fact.origin.tag(), fact.text)
}

/// Writes the mark into every unmarked fact line (as `[observed]`), leaving everything else - headings, the person's prose - untouched.
/// Answers the new text and how many lines it marked. Marking again marks nothing.
pub fn migrate(text: &str) -> (String, usize) {
    let mut marked = 0;
    let lines: Vec<String> = text
        .lines()
        .map(|raw| {
            let trimmed = raw.trim_start();

            if !(trimmed.starts_with("- ") || trimmed.starts_with("* ")) {
                return raw.to_string();
            }

            let body = trimmed[2..].trim();
            let first = body.split_once(' ').map_or(body, |(first, _)| first);

            if body.is_empty() || Origin::from_tag(first).is_some() {
                return raw.to_string();
            }

            marked += 1;

            format!("{}- {} {}", &raw[..raw.len() - trimmed.len()], Origin::Observed.tag(), body)
        })
        .collect();
    let mut out = lines.join("\n");

    if text.ends_with('\n') {
        out.push('\n');
    }

    (out, marked)
}

/// How many `[inferred]` lines the model is shown, as unconfirmed notes.
const UNCONFIRMED_SHOWN: usize = 3;

/// What may be given to the model as rules (`[stated]` and `[observed]`), and - apart, labelled as guesses - at most a few `[inferred]` ones.
pub fn for_context(text: &str) -> String {
    let facts = parse(text);
    let rules: Vec<String> = facts.iter().filter(|fact| fact.origin.may_instruct()).map(|fact| format!("- {} {}", fact.origin.tag(), fact.text)).collect();
    let guesses: Vec<String> = facts.iter().filter(|fact| !fact.origin.may_instruct()).rev().take(UNCONFIRMED_SHOWN).map(|fact| format!("- {}", fact.text)).collect();
    let mut out = rules.join("\n");

    if !guesses.is_empty() {
        out.push_str(&format!("\n\nUnconfirmed guesses (not instructions; check before relying on them):\n{}", guesses.join("\n")));
    }

    out.trim().to_string()
}

// ---------------------------------------------------------------------------------------------------- M3-3: the index

/// A memory file the index may list.
pub struct Entry {
    pub path: String,
    pub description: String,
    pub facts: usize,
    pub bytes: usize,
}

/// One line of description for a memory file, from what it holds.
pub fn describe(path: &str, text: &str) -> String {
    let facts = parse(text);
    let stated = facts.iter().filter(|fact| fact.origin == Origin::Stated).count();
    let purpose = if path.ends_with("memory.md") {
        "what this project is like and how to work in it"
    } else if path.ends_with("lessons.md") {
        "mistakes made here before and what worked instead"
    } else if path.contains("recipes") {
        "how a similar task was done and verified"
    } else {
        "notes about this project"
    };

    format!("{purpose} ({} fact{}{})", facts.len(), if facts.len() == 1 { "" } else { "s" }, if stated > 0 { format!(", {stated} stated by you") } else { String::new() })
}

/// The files of `.sdc` that hold memory, with a description each. Absent files are not listed.
pub fn entries(root: &Path) -> Vec<Entry> {
    let mut found = Vec::new();
    let mut consider = |relative: String| {
        if let Ok(text) = std::fs::read_to_string(root.join(&relative)) {
            if !text.trim().is_empty() {
                found.push(Entry { description: describe(&relative, &text), facts: parse(&text).len(), bytes: text.len(), path: relative });
            }
        }
    };

    consider(".sdc/memory.md".to_string());
    consider(".sdc/lessons.md".to_string());

    if let Ok(dir) = std::fs::read_dir(root.join(".sdc").join("recipes")) {
        let mut names: Vec<String> = dir.flatten().filter_map(|entry| entry.file_name().to_str().map(str::to_string)).filter(|name| name.ends_with(".md") && !name.starts_with("._")).collect();

        names.sort();

        for name in names {
            consider(format!(".sdc/recipes/{name}"));
        }
    }

    found
}

/// The index the model is sent instead of the files: one line each. Empty when there is no memory.
pub fn index_block(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let lines: Vec<String> = entries.iter().map(|entry| format!("- {}: {}", entry.path, entry.description)).collect();

    format!("What this project remembers - read a file with read_file when it bears on the task (facts marked [inferred] are guesses, not rules):\n{}", lines.join("\n"))
}

// ---------------------------------------------------------------------------------------------------- M3-4: small and lasting

/// The most facts a memory file keeps.
pub const MAX_FACTS: usize = 60;

/// Words that mark a fact as a snapshot of today, not something that will be true in a month.
const TEMPORARY: &[&str] = &["right now", "at the moment", "currently failing", "currently broken", "for now", "just now", "today", "tonight", "this morning", "this afternoon", "this bug", "todo", "wip", "work in progress", "temporarily", "for the moment", "as of now"];

/// Will this still be true in a month? A fact about the project's shape, its tools and the person's preferences is; a state of today is not.
pub fn lasts_a_month(fact: &str) -> bool {
    let lower = fact.to_lowercase();

    !TEMPORARY.iter().any(|word| lower.contains(word))
}

/// Two facts are one fact when they share most of their words, whatever their marks.
fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '/' && c != '-').map(str::to_lowercase).filter(|word| word.chars().count() >= 4).collect()
}

/// The `code` spans of a fact: what a check fact is really about. Two facts about different commands are different facts, however alike the sentence
/// around the command is.
fn spans(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

fn similar(left: &str, right: &str) -> bool {
    let (left_spans, right_spans) = (spans(left), spans(right));

    if (!left_spans.is_empty() || !right_spans.is_empty()) && left_spans != right_spans {
        return false;
    }

    let (a, b) = (words(left), words(right));

    if a.is_empty() || b.is_empty() {
        return left.trim() == right.trim();
    }

    let shared = a.intersection(&b).count();

    shared * 5 >= a.len().min(b.len()) * 4 && a.len() <= b.len() * 3 && b.len() <= a.len() * 3
}

/// Tidies a list of facts: temporary ones out (a `[stated]` one stays - the person decides what lasts), repeats merged (the newest wording wins, and the
/// stronger origin), and the size limit kept by dropping the weakest oldest first.
pub fn tidy(facts: Vec<Fact>, limit: usize) -> (Vec<Fact>, usize) {
    let before = facts.len();
    let mut kept: Vec<Fact> = Vec::new();

    for fact in facts.into_iter().rev() {
        if fact.origin != Origin::Stated && !lasts_a_month(&fact.text) {
            continue;
        }

        match kept.iter_mut().find(|known| similar(&known.text, &fact.text)) {
            Some(known) => known.origin = known.origin.max(fact.origin),
            None => kept.push(fact),
        }
    }

    kept.reverse();

    while kept.len() > limit {
        let weakest = kept.iter().enumerate().filter(|(_, fact)| fact.origin != Origin::Stated).min_by_key(|(index, fact)| (fact.origin, *index)).map(|(index, _)| index);

        match weakest {
            Some(index) => {
                kept.remove(index);
            }
            None => break,
        }
    }

    let dropped = before - kept.len();

    (kept, dropped)
}

// ---------------------------------------------------------------------------------------------------- M3-2: the pass after a turn

/// What a finished turn showed, as code can read it.
#[derive(Debug, Default, Clone)]
pub struct Turn {
    /// The person's words.
    pub request: String,
    /// Commands that passed after the turn changed something.
    pub passed_checks: Vec<String>,
}

/// What the pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub added: usize,
    pub corrected: usize,
    pub merged_or_dropped: usize,
    pub migrated: usize,
}

const LEARNED: &str = "## Learned by SDC";

/// A correction in the person's words: "actually, X", "no - X", "that is wrong: X". The corrected statement, if there is one.
pub fn correction_in(request: &str) -> Option<String> {
    let lower = request.to_lowercase();

    for marker in ["actually, ", "actually: ", "correction: ", "that is wrong: ", "that's wrong: ", "no, it is ", "no, it's ", "আসলে, ", "আসলে: ", "ভুল: "] {
        if let Some(at) = lower.find(marker) {
            let said = request[at + marker.len()..].split(['.', '।', '\n']).next().unwrap_or_default().trim();

            if said.split_whitespace().count() >= 2 {
                return Some(said.to_string());
            }
        }
    }

    None
}

/// The pass: reads `.sdc/memory.md`, marks what is unmarked, adds what the turn established, replaces what the person corrected, tidies, and
/// writes the file back (once, atomically). Nothing here calls a model.
pub fn pass(root: &Path, turn: &Turn) -> Report {
    let path = root.join(".sdc").join("memory.md");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let (text, migrated) = migrate(&text);
    let (person, learned) = match text.split_once(LEARNED) {
        Some((person, learned)) => (person.trim_end().to_string(), learned.to_string()),
        None => (text.trim_end().to_string(), String::new()),
    };
    let mut facts = parse(&person);
    let mut learned_facts = parse(&learned);
    let mut report = Report { migrated, ..Report::default() };

    /* What the person said to remember is theirs. */
    if let Some(said) = crate::sme::memory::requested_fact(&turn.request) {
        if !facts.iter().chain(&learned_facts).any(|fact| similar(&fact.text, &said)) {
            facts.push(Fact { origin: Origin::Stated, text: said, unmarked: false });
            report.added += 1;
        }
    }

    /* A correction replaces the fact it contradicts: the closest earlier fact is dropped and the corrected statement is stated. */
    if let Some(correct) = correction_in(&turn.request) {
        let close = facts.iter().chain(&learned_facts).position(|fact| words(&fact.text).intersection(&words(&correct)).count() >= 2);

        if close.is_some() {
            let drop_from = |list: &mut Vec<Fact>| {
                if let Some(at) = list.iter().position(|fact| words(&fact.text).intersection(&words(&correct)).count() >= 2) {
                    list.remove(at);

                    true
                } else {
                    false
                }
            };

            if !drop_from(&mut facts) {
                drop_from(&mut learned_facts);
            }

            facts.push(Fact { origin: Origin::Stated, text: correct, unmarked: false });
            report.corrected += 1;
        }
    }

    /* A check that passed after a change is a fact a tool showed. */
    for check in &turn.passed_checks {
        if let Some(fact) = crate::v3::memory::check_fact(check) {
            if !facts.iter().chain(&learned_facts).any(|known| spans(&known.text) == spans(&fact)) {
                learned_facts.push(Fact { origin: Origin::Observed, text: fact, unmarked: false });
                report.added += 1;
            }
        }
    }

    let (facts, dropped_person) = tidy(facts, MAX_FACTS);
    let (learned_facts, dropped_learned) = tidy(learned_facts, MAX_FACTS);

    report.merged_or_dropped = dropped_person + dropped_learned;

    if report == Report::default() && text.trim().is_empty() {
        return report;
    }

    let mut out = String::new();

    if !facts.is_empty() {
        out.push_str(&facts.iter().map(line).collect::<Vec<_>>().join("\n"));
        out.push_str("\n\n");
    }

    if !learned_facts.is_empty() {
        out.push_str(&format!("{LEARNED}\n\n{}\n", learned_facts.iter().map(line).collect::<Vec<_>>().join("\n")));
    }

    if out.trim().is_empty() || out.trim() == text.trim() {
        return report;
    }

    let partial = path.with_extension("md.partial");

    if std::fs::create_dir_all(path.parent().unwrap_or(root)).is_ok() && std::fs::write(&partial, out.trim_end().to_string() + "\n").is_ok() {
        let _ = std::fs::rename(&partial, &path);
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-memv2-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sdc")).unwrap();

        root
    }

    #[test]
    fn a_fact_has_an_origin_and_an_unmarked_one_is_read_as_observed() {
        let facts = parse("- [stated] Use pnpm, never npm\n- [inferred] The API is probably REST\n- `cargo test` passes here\nA heading line\n* [observed] Tests live in tests/\n");

        assert_eq!(facts.iter().map(|fact| fact.origin).collect::<Vec<_>>(), [Origin::Stated, Origin::Inferred, Origin::Observed, Origin::Observed]);
        assert_eq!(facts[0].text, "Use pnpm, never npm");
        assert!(facts[2].unmarked && !facts[3].unmarked);
        assert!(Origin::Stated.may_instruct() && Origin::Observed.may_instruct() && !Origin::Inferred.may_instruct());
    }

    #[test]
    fn migration_marks_old_lines_once_and_touches_nothing_else() {
        let old = "# Notes for the team\n\n- Use pnpm\n  - nested\n\nSome prose.\n\n## Learned by SDC\n\n- `cargo test` passes here\n";
        let (once, marked) = migrate(old);

        assert_eq!(marked, 3, "the three bullet lines");
        assert!(once.contains("- [observed] Use pnpm") && once.contains("# Notes for the team") && once.contains("Some prose.") && once.ends_with('\n'));
        assert!(once.contains("  - [observed] nested"), "an indented bullet keeps its indent");

        let (twice, again) = migrate(&once);

        assert_eq!(again, 0);
        assert_eq!(twice, once, "idempotent");
    }

    #[test]
    fn a_guess_is_never_given_as_a_rule() {
        let text = "- [stated] Answer in Bangla\n- [observed] Tests run with unittest\n- [inferred] Maybe the cache is Redis\n";
        let shown = for_context(text);

        assert!(shown.contains("- [stated] Answer in Bangla") && shown.contains("- [observed] Tests run with unittest"));
        assert!(shown.contains("Unconfirmed guesses (not instructions") && shown.contains("Maybe the cache is Redis"));
        assert!(!shown.contains("[inferred]"), "the guess carries no rule mark");
        assert_eq!(for_context("- [stated] One\n"), "- [stated] One");
    }

    #[test]
    fn a_fact_that_will_not_be_true_in_a_month_is_not_kept_unless_the_person_said_it() {
        assert!(lasts_a_month("Tests run with unittest"));
        assert!(!lasts_a_month("The build is failing right now"));
        assert!(!lasts_a_month("TODO fix the header"));

        let (kept, dropped) = tidy(
            vec![
                Fact { origin: Origin::Observed, text: "The build is failing right now".into(), unmarked: false },
                Fact { origin: Origin::Stated, text: "Deploy happens today at noon, always".into(), unmarked: false },
                Fact { origin: Origin::Observed, text: "Tests run with unittest".into(), unmarked: false },
            ],
            60,
        );

        assert_eq!(kept.iter().map(|fact| fact.text.as_str()).collect::<Vec<_>>(), ["Deploy happens today at noon, always", "Tests run with unittest"]);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn repeats_are_merged_keeping_the_newest_wording_and_the_strongest_origin() {
        let (kept, dropped) = tidy(
            vec![
                Fact { origin: Origin::Stated, text: "Tests run with unittest discover from the project root".into(), unmarked: false },
                Fact { origin: Origin::Inferred, text: "Tests run with unittest discover from the root of the project".into(), unmarked: false },
            ],
            60,
        );

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "Tests run with unittest discover from the root of the project", "the newest wording");
        assert_eq!(kept[0].origin, Origin::Stated, "the person's word is not weakened by a later guess");
        assert_eq!(dropped, 1);
    }

    #[test]
    fn at_the_size_limit_the_weakest_oldest_facts_go_first_and_what_the_person_said_never_goes() {
        let mut facts = vec![Fact { origin: Origin::Stated, text: "alpha beta gamma delta one".into(), unmarked: false }];

        for index in 0..4 {
            facts.push(Fact { origin: Origin::Inferred, text: format!("guess number{index} aaaa{index} bbbb{index}"), unmarked: false });
            facts.push(Fact { origin: Origin::Observed, text: format!("seen number{index} cccc{index} dddd{index}"), unmarked: false });
        }

        let (kept, _) = tidy(facts.clone(), 5);

        assert_eq!(kept.len(), 5);
        assert!(kept.iter().any(|fact| fact.origin == Origin::Stated));
        assert!(!kept.iter().any(|fact| fact.origin == Origin::Inferred), "the guesses went first");

        let (fewer, _) = tidy(facts, 3);

        assert_eq!(fewer.len(), 3);
        assert!(fewer.iter().any(|fact| fact.origin == Origin::Stated) && fewer.iter().filter(|fact| fact.origin == Origin::Observed).count() == 2);
        assert!(fewer.iter().all(|fact| !fact.text.contains("number0 cccc0") && !fact.text.contains("number1 cccc1")), "of the observed ones, the oldest go first");

        let only_stated: Vec<Fact> = (0..10).map(|n| Fact { origin: Origin::Stated, text: format!("stated fact {n} {n}{n}{n}{n}"), unmarked: false }).collect();

        assert_eq!(tidy(only_stated, 5).0.len(), 10, "what the person stated is never dropped, even over the limit");
    }

    #[test]
    fn the_index_is_one_line_a_file_and_a_fraction_of_the_files() {
        let root = temp("index");
        let facts: String = (0..40).map(|n| format!("- [observed] Fact number {n} says something about module{n} and how it is built and tested here\n")).collect();

        std::fs::write(root.join(".sdc/memory.md"), &facts).unwrap();
        std::fs::write(root.join(".sdc/lessons.md"), "- [observed] a lesson about charging\n").unwrap();
        std::fs::create_dir_all(root.join(".sdc/recipes")).unwrap();
        std::fs::write(root.join(".sdc/recipes/rename.md"), "- [observed] step one\n").unwrap();

        let listed = entries(&root);
        let block = index_block(&listed);

        assert_eq!(listed.len(), 3);
        assert!(block.contains("- .sdc/memory.md: what this project is like and how to work in it (40 facts)"), "{block}");
        assert!(block.contains(".sdc/lessons.md") && block.contains(".sdc/recipes/rename.md"));

        let whole = crate::tokens::estimate(&facts);
        let index = crate::tokens::estimate(&block);

        assert!(index * 4 < whole, "the index ({index} tokens) is a fraction of the file ({whole} tokens)");
        assert_eq!(index_block(&entries(&temp("empty"))), "");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_correction_in_the_persons_words_is_found() {
        assert_eq!(correction_in("actually, the tests run with pytest not unittest").as_deref(), Some("the tests run with pytest not unittest"));
        assert_eq!(correction_in("আসলে, টেস্ট চলে pytest দিয়ে").as_deref(), Some("টেস্ট চলে pytest দিয়ে"));
        assert_eq!(correction_in("make it faster"), None);
        assert_eq!(correction_in("actually, no"), None, "one word is not a statement");
    }

    #[test]
    fn the_pass_marks_adds_corrects_and_merges_in_code_after_the_turn() {
        let root = temp("pass");

        std::fs::write(root.join(".sdc/memory.md"), "- Tests run with unittest\n- The orders list lives in src/orders.ts\n\n## Learned by SDC\n\n- `cargo build` passes here after a change - it is how this project checks its work.\n").unwrap();

        let report = pass(&root, &Turn { request: "actually, tests run with pytest not unittest. remember that deploys go through make release".into(), passed_checks: vec!["python3 -m pytest -q 2>&1 | tail -5".into()] });
        let text = std::fs::read_to_string(root.join(".sdc/memory.md")).unwrap();

        assert_eq!(report.migrated, 3, "the three old lines were marked");
        assert_eq!(report.corrected, 1);
        assert!(report.added >= 2, "{report:?}");
        assert!(text.contains("- [stated] tests run with pytest not unittest"), "{text}");
        assert!(!text.contains("Tests run with unittest"), "the contradicted fact is gone: {text}");
        assert!(text.contains("[stated] deploys go through make release"));
        assert!(text.contains("## Learned by SDC") && text.contains("[observed] `python3 -m pytest -q` passes here"));
        assert!(text.contains("[observed] The orders list lives in src/orders.ts"));

        let again = pass(&root, &Turn::default());

        assert_eq!((again.migrated, again.added, again.corrected), (0, 0, 0), "a second pass with nothing new changes nothing");
        assert_eq!(std::fs::read_to_string(root.join(".sdc/memory.md")).unwrap(), text);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_with_no_memory_stays_without_one_until_there_is_something_to_keep() {
        let root = temp("none");

        assert_eq!(pass(&root, &Turn::default()), Report::default());
        assert!(!root.join(".sdc/memory.md").exists());

        let report = pass(&root, &Turn { request: "remember that we use tabs".into(), passed_checks: vec![] });

        assert_eq!(report.added, 1);
        assert!(std::fs::read_to_string(root.join(".sdc/memory.md")).unwrap().contains("[stated] we use tabs"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
