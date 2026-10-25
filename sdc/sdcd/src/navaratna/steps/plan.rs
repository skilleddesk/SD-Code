//! The planner of the step executor: a big request becomes labelled units (the model labels, code verifies), requirement groups, and one step
//! per requirement - each with its own spec units, files and done-when.
//!
//! The model's labelling is checked by the coverage gate (N2-1); a unit it did not label is asked about once more, and what is still open
//! falls back to the deterministic labeller, which is said in the plan's notes. With `nr.lang_spec` on, each non-English unit is restated and
//! back-checked (N2-2). Nothing is ever dropped: the whole request and the map are in the vault.

use super::{Engine, Purpose, Request};
use crate::navaratna::route;
use crate::navaratna::spec::{self, Label, Map, Unit};

#[derive(Debug, Clone, PartialEq)]
pub struct PlanStep {
    pub id: String,
    pub r_ids: Vec<String>,
    pub unit_ids: Vec<String>,
    pub goal: String,
    pub done_when: String,
    pub files: Vec<String>,
    pub tag: route::Tag,
    pub why: String,
    /// A long document, written section by section (N4-4).
    pub doc: bool,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub steps: Vec<PlanStep>,
    pub map: Map,
    /// Background units, word for word: every step's pack carries them (shortened to a budget).
    pub background: Vec<String>,
    pub notes: Vec<String>,
    /// The interface contracts of the architect's Blueprint (empty signatures), when there was one.
    pub contracts: Vec<crate::navaratna::blueprint::Contract>,
}

const LABEL_BATCH: usize = 40;

fn label_prompt(units: &[&Unit]) -> String {
    let mut out = String::from(
        "Label every numbered unit of a request. For each unit answer one line, exactly `S-001: R` for a requirement (something to build, change, fix, check or write), `S-001: B` for background (context, a heading, an example that asks for nothing), or `S-001: P S-000` for a part of the requirement that unit S-000 states (a code block or table that belongs to it). Answer one line per unit and nothing else.\n\n",
    );

    for unit in units {
        let text: String = unit.text.chars().take(400).collect();

        out.push_str(&format!("{} [{:?}]: {}\n", unit.id, unit.kind, text.replace('\n', " ⏎ ")));
    }

    out
}

/// The labels in a model's answer: `(unit id, label)`. Lines it did not understand are skipped, so those units stay unlabelled.
pub fn parse_labels(answer: &str, ids: &[&str]) -> Vec<(String, Label)> {
    answer
        .lines()
        .filter_map(|line| {
            let line = line.trim().trim_start_matches(['-', '*', ' ']);
            let (id, rest) = line.split_once(':')?;
            let id = id.trim();

            if !ids.contains(&id) {
                return None;
            }

            let mut words = rest.split_whitespace();

            match words.next()?.trim_matches(['.', ',']).to_ascii_uppercase().as_str() {
                "R" | "REQUIREMENT" => Some((id.to_string(), Label::Requirement)),
                "B" | "BACKGROUND" => Some((id.to_string(), Label::Background)),
                "P" | "PART" => words.next().map(|owner| (id.to_string(), Label::PartOf(owner.trim_matches(['.', ',', ')', '(']).to_string()))),
                _ => None,
            }
        })
        .collect()
}

/// Labels the units with the model, checks the coverage and falls back for what is left.
fn label(units: Vec<Unit>, engine: &mut dyn Engine, notes: &mut Vec<String>) -> Map {
    let mut answers: std::collections::BTreeMap<String, Label> = std::collections::BTreeMap::new();

    /* two rounds: everything, then what the first round left open */
    for round in 0..2 {
        let open: Vec<&Unit> = units.iter().filter(|unit| !answers.contains_key(&unit.id)).collect();

        if open.is_empty() {
            break;
        }

        for batch in open.chunks(LABEL_BATCH) {
            let ids: Vec<&str> = batch.iter().map(|unit| unit.id.as_str()).collect();

            match engine.complete(&Request::new(Purpose::Label, label_prompt(batch))) {
                Ok(answer) => {
                    for (id, label) in parse_labels(&answer, &ids) {
                        answers.entry(id).or_insert(label);
                    }
                }
                Err(error) => {
                    notes.push(format!("labelling round {} failed: {error}", round + 1));
                    break;
                }
            }
        }
    }

    let mut fallback = Vec::new();
    let map = spec::label(units.clone(), &mut |unit, last| match answers.get(&unit.id) {
        Some(Label::PartOf(owner)) if !matches!(answers.get(owner), Some(Label::Requirement)) => {
            fallback.push(unit.id.clone());
            spec::heuristic(unit, last)
        }
        Some(label) => Some(label.clone()),
        None => {
            fallback.push(unit.id.clone());
            spec::heuristic(unit, last)
        }
    });

    if !fallback.is_empty() {
        notes.push(format!("the model left {} unit(s) unlabelled ({}); the deterministic labeller labelled them", fallback.len(), fallback.join(", ")));
    }

    map
}

fn first_sentence(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let spans = spec::sentence_spans(&flat);

    spans.first().map_or_else(|| flat.chars().take(200).collect(), |(a, b)| flat[*a..*b].chars().take(240).collect())
}

fn is_doc(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    let verb = ["write", "create", "produce", "draft", "compose", "লেখো", "লিখ"].iter().any(|word| lower.contains(word));
    let noun = ["document", "documentation", "guide", "readme", "manual", "report", "essay", "handbook", "tutorial", "chapter", "whitepaper", "ডকুমেন্ট", "গাইড"].iter().any(|word| lower.contains(word));

    verb && noun && !lower.contains("function") && !lower.contains("test")
}

/// One step per requirement of a map, with its own units, files and tag.
fn make_steps(map: &Map, rank: &dyn Fn(&str) -> Vec<String>) -> Vec<PlanStep> {
    let text_of = |id: &str| map.units.iter().find(|unit| unit.id == id).map(|unit| unit.text.clone()).unwrap_or_default();
    let mut steps = Vec::new();

    for (n, (r_id, unit_ids)) in map.requirements.iter().enumerate() {
        let body: Vec<String> = unit_ids.iter().map(|id| text_of(id)).collect();
        let joined = body.join("\n");
        let goal = first_sentence(&body[0]);
        let mut files = crate::runtime::decompose::files_in(&joined);

        for ranked in rank(&goal).into_iter().take(3) {
            if !files.contains(&ranked) {
                files.push(ranked);
            }
        }

        /* a document the request names (`docs/guide.md`) is a file of the step too */
        for token in joined.split_whitespace().map(|word| word.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '"' | '\'' | '`' | '(' | ')')).trim_end_matches('.')) {
            if (token.ends_with(".md") || token.ends_with(".txt") || token.ends_with(".rst")) && !token.starts_with('/') && !token.contains("..") && !files.iter().any(|file| file == token) {
                files.push(token.to_string());
            }
        }

        files.truncate(6);

        let (tag, why) = route::tag(
            &route::Facts {
                kind: if goal.to_lowercase().contains("test") { route::Kind::Test } else { route::Kind::Edit },
                files_touched: files.len(),
                blueprint_marker: ["folder structure", "data model", "architecture", "interface contract", "project skeleton"].iter().any(|marker| joined.to_lowercase().contains(marker)),
                failed_attempts: 0,
                text: &joined,
            },
            None,
        );

        steps.push(PlanStep { id: format!("step-{}", n + 1), r_ids: vec![r_id.clone()], unit_ids: unit_ids.clone(), done_when: goal.clone(), doc: is_doc(&joined), goal, files, tag, why });
    }

    steps
}

/// Builds the plan. `rank` gives the project files that answer a text (best first), `files_in` the paths a text names.
pub fn build(prompt: &str, engine: &mut dyn Engine, rank: &dyn Fn(&str) -> Vec<String>) -> Result<Plan, String> {
    let units = spec::parse(prompt);

    if units.is_empty() {
        return Err("the request is empty".into());
    }

    let mut notes = Vec::new();
    let mut map = label(units, engine, &mut notes);

    /* the coverage gate, on the map as the model and the fallback made it */
    spec::coverage_gate(&map).map_err(|blocked| blocked.to_string())?;

    /* N2-2: a non-English unit is restated and back-checked; a restatement that lost a number or a name is not trusted - the original stays */
    if crate::v3::on("nr.lang_spec") {
        let report = crate::navaratna::langspec::restate(&map.units, &mut |unit| engine.complete(&Request::new(Purpose::Translate, format!("Restate this request line as one precise English sentence. Keep every number, name, path and code identifier exactly as written. Do not translate code.\n\n{}", unit.text))).unwrap_or_default());

        for line in report.lines.iter().filter(|line| line.translated) {
            let Some(unit) = map.units.iter_mut().find(|unit| unit.id == line.unit) else { continue };

            if line.missing.is_empty() && !line.english.is_empty() {
                unit.text = format!("{}\n(English: {})", unit.text, line.english);
            } else {
                notes.push(format!("{}: the English restatement lost {}, so the original stands", line.unit, line.missing.join(", ")));
            }
        }
    }

    let steps = make_steps(&map, rank);

    /* N6-1: every requirement has exactly one owning step */
    let all: Vec<String> = map.requirements.keys().cloned().collect();
    let owners: Vec<(String, Vec<String>)> = steps.iter().map(|step| (step.id.clone(), step.r_ids.clone())).collect();

    crate::navaratna::registry::one_owner(&all, &owners).map_err(|problems| problems.to_string())?;

    let background: Vec<String> = map.units.iter().filter(|unit| matches!(map.labels.get(&unit.id), Some(Label::Background)) && !matches!(unit.kind, spec::Kind::Heading)).map(|unit| unit.text.clone()).collect();

    Ok(Plan { steps, map, background, notes, contracts: Vec::new() })
}

/// Adopts a validated Blueprint: its requirements and contracts become the plan.
pub fn adopt_blueprint(plan: &mut Plan, blueprint: &crate::navaratna::blueprint::Blueprint, map: Map, rank: &dyn Fn(&str) -> Vec<String>) -> Result<(), String> {
    let steps = make_steps(&map, rank);
    let all: Vec<String> = map.requirements.keys().cloned().collect();
    let owners: Vec<(String, Vec<String>)> = steps.iter().map(|step| (step.id.clone(), step.r_ids.clone())).collect();

    crate::navaratna::registry::one_owner(&all, &owners).map_err(|problems| problems.to_string())?;

    plan.background = map.units.iter().filter(|unit| matches!(map.labels.get(&unit.id), Some(Label::Background)) && !matches!(unit.kind, spec::Kind::Heading)).map(|unit| unit.text.clone()).collect();
    plan.steps = steps;
    plan.map = map;
    plan.contracts = blueprint.contracts.clone();
    plan.notes.push(format!("the architect's Blueprint is used: {} requirement(s), {} contract(s), {} folder(s)", blueprint.requirements.len(), blueprint.contracts.len(), blueprint.folders.len()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted(Vec<String>, Vec<String>);

    impl Engine for Scripted {
        fn complete(&mut self, request: &Request) -> Result<String, String> {
            self.1.push(request.prompt.clone());

            if self.0.is_empty() { Err("no more scripted answers".into()) } else { Ok(self.0.remove(0)) }
        }
    }

    const PROMPT: &str = "# Bakery\n\nThe shop is for a small bakery.\n\nAdd a function total(items) that sums the prices.\n\nWrite a user guide for the shop.\n\n```python\ndef total(items):\n    pass\n```\n";

    #[test]
    fn the_model_labels_are_read_and_a_part_attaches_to_its_requirement() {
        let units = spec::parse(PROMPT);
        let ids: Vec<&str> = units.iter().map(|u| u.id.as_str()).collect();
        let labels = parse_labels("S-001: B\nS-002: B\nS-003: R\nS-004: R.\nS-005: P S-003\nS-099: R\nnonsense", &ids);

        assert_eq!(labels.len(), 5);
        assert_eq!(labels[4], ("S-005".to_string(), Label::PartOf("S-003".into())));
    }

    #[test]
    fn a_plan_has_one_step_per_requirement_with_its_own_units_and_a_doc_step_is_recognised() {
        let mut engine = Scripted(vec!["S-001: B\nS-002: B\nS-003: R\nS-004: R\nS-005: P S-003".into()], vec![]);
        let plan = build(PROMPT, &mut engine, &|_| vec!["src/shop.py".to_string()]).unwrap();

        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].r_ids, vec!["R-001".to_string()]);
        assert_eq!(plan.steps[0].unit_ids, vec!["S-003".to_string(), "S-005".to_string()], "the code block is part of the first requirement");
        assert!(plan.steps[0].files.contains(&"src/shop.py".to_string()));
        assert!(!plan.steps[0].doc && plan.steps[1].doc);
        assert_eq!(plan.background, vec!["The shop is for a small bakery.".to_string()]);
        assert!(plan.notes.is_empty(), "{:?}", plan.notes);
        assert_eq!(engine.1.len(), 1, "one labelling call");
    }

    #[test]
    fn units_the_model_skips_are_asked_again_and_then_labelled_by_code_and_it_is_said() {
        let mut engine = Scripted(vec!["S-001: B\nS-003: R".into(), "S-004: R".into()], vec![]);
        let plan = build(PROMPT, &mut engine, &|_| vec![]).unwrap();

        assert_eq!(engine.1.len(), 2);
        assert!(engine.1[1].contains("S-002 [") && !engine.1[1].contains("S-001 ["), "{}", engine.1[1]);
        assert!(plan.map.unmapped().is_empty());
        assert!(plan.notes.iter().any(|note| note.contains("unlabelled")), "{:?}", plan.notes);
    }

    #[test]
    fn a_dead_model_still_gives_a_complete_plan_through_the_fallback() {
        let mut engine = Scripted(vec![], vec![]);
        let plan = build(PROMPT, &mut engine, &|_| vec![]).unwrap();

        assert!(plan.map.unmapped().is_empty() && !plan.steps.is_empty());
        assert!(plan.notes.iter().any(|note| note.contains("failed")));
    }

    #[test]
    fn a_part_of_a_background_unit_is_not_trusted() {
        let mut engine = Scripted(vec!["S-001: B\nS-002: B\nS-003: B\nS-004: R\nS-005: P S-003".into()], vec![]);
        let plan = build(PROMPT, &mut engine, &|_| vec![]).unwrap();

        assert!(plan.map.unmapped().is_empty());
        assert!(plan.notes.iter().any(|note| note.contains("S-005")), "{:?}", plan.notes);
    }

    #[test]
    fn the_english_restatement_is_added_only_when_the_back_check_passes() {
        crate::v3::forcing(&["nr.lang_spec"], || {
            let bangla = "shop.py ফাইলে total() ফাংশনটা ঠিক করো, ৩.৫ শতাংশ কর ধরো।";
            let mut good = Scripted(vec!["S-001: R".into(), "In shop.py fix the function total(), using a tax of 3.5 percent.".into()], vec![]);
            let plan = build(bangla, &mut good, &|_| vec![]).unwrap();

            assert!(plan.map.units[0].text.contains("(English: In shop.py fix the function total()"), "{}", plan.map.units[0].text);

            let mut lossy = Scripted(vec!["S-001: R".into(), "Fix the tax function.".into()], vec![]);
            let plan = build(bangla, &mut lossy, &|_| vec![]).unwrap();

            assert!(!plan.map.units[0].text.contains("(English:") && plan.notes.iter().any(|note| note.contains("lost")), "{:?}", plan.notes);
        });
    }
}
