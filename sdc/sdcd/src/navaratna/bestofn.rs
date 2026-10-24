//! **N7-1 - Best-of-N with test selection** (switch `nr.best_of_n`, off).
//!
//! A weak model's several tries, judged by tests, often beat one try - and a local try costs time, not money. So before a step goes to the
//! cloud: after the normal attempt and two sliced-error retries have failed (or for a step tagged `hard`), **two to four candidates** are made
//! at different temperatures, each from a clean copy of the tree; the project's checks run on each; the passing candidate with the smallest diff
//! is taken. If none passes, the step goes to the relay with the best candidate's error. Where there is no automatic check there is no selection,
//! so Best-of-N does not run. Only the chosen candidate ever touches the real tree.

use std::collections::BTreeMap;

pub type Tree = BTreeMap<String, String>;

pub const MIN_CANDIDATES: usize = 2;
pub const MAX_CANDIDATES: usize = 4;
/// Temperatures tried, in order; the first is the cool one.
pub const TEMPERATURES: [f32; 4] = [0.3, 0.7, 1.0, 1.2];

#[derive(Debug, Clone, Copy)]
pub struct Trigger {
    /// The normal attempt failed.
    pub normal_failed: bool,
    /// Retries with a sliced error that failed.
    pub sliced_retries_failed: u32,
    /// The step is tagged hard.
    pub hard: bool,
    /// The project has an automatic check that can say pass or fail.
    pub has_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotRun {
    SwitchOff,
    /// No verifier, no selection.
    NoCheck,
    NotYet,
}

/// Should Best-of-N run?
pub fn should_run(trigger: &Trigger) -> Result<(), NotRun> {
    if !crate::v3::on("nr.best_of_n") {
        return Err(NotRun::SwitchOff);
    }

    if !trigger.has_check {
        return Err(NotRun::NoCheck);
    }

    if trigger.hard || (trigger.normal_failed && trigger.sliced_retries_failed >= 2) {
        Ok(())
    } else {
        Err(NotRun::NotYet)
    }
}

/// What one check of a candidate said.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Check {
    pub passed: bool,
    /// How many checks failed (for choosing the best failure).
    pub failures: usize,
    /// The sliced error, for the relay.
    pub error: String,
}

pub trait Parties {
    /// A candidate's file writes, made at this temperature from the base tree. May be asked for several in a row.
    fn candidate(&mut self, base: &Tree, temperature: f32, index: usize) -> Vec<(String, String)>;
    /// Runs the project's checks on a tree.
    fn check(&mut self, tree: &Tree) -> Check;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A candidate passed: its writes, which one it was, and the size of its diff.
    Chosen { index: usize, writes: Vec<(String, String)>, diff_lines: usize, tried: usize },
    /// None passed: to the relay with the best failure's error.
    HandOver { error: String, tried: usize },
}

/// The best passing candidate so far.
struct Passing {
    index: usize,
    writes: Vec<(String, String)>,
    diff_lines: usize,
}

/// Lines added plus removed between two trees.
pub fn diff_size(base: &Tree, new: &Tree) -> usize {
    let mut total = 0;

    for (path, text) in new {
        let old = base.get(path).map_or("", String::as_str);

        total += similar::TextDiff::from_lines(old, text).iter_all_changes().filter(|change| change.tag() != similar::ChangeTag::Equal).count();
    }

    for (path, old) in base.iter().filter(|(path, _)| !new.contains_key(*path)) {
        let _ = path;
        total += old.lines().count();
    }

    total
}

/// Runs the candidates one after another (two at a time only where the memory budgeter allows two slots - not done here, sequential is always
/// correct). `n` is clamped to 2..=4.
pub fn run(base: &Tree, n: usize, parties: &mut dyn Parties) -> Outcome {
    let n = n.clamp(MIN_CANDIDATES, MAX_CANDIDATES);
    let mut best_pass: Option<Passing> = None;
    let mut best_fail: Option<(usize, usize, String)> = None;

    for (index, temperature) in TEMPERATURES.iter().take(n).enumerate() {
        let writes = parties.candidate(base, *temperature, index);
        /* each candidate lives in its own copy: the base is never touched */
        let mut tree = base.clone();

        for (path, content) in &writes {
            tree.insert(path.clone(), content.clone());
        }

        let check = parties.check(&tree);
        let size = diff_size(base, &tree);

        if check.passed {
            if best_pass.as_ref().is_none_or(|best| size < best.diff_lines) {
                best_pass = Some(Passing { index, writes, diff_lines: size });
            }
        } else if best_fail.as_ref().is_none_or(|(failures, best_size, _)| (check.failures, size) < (*failures, *best_size)) {
            best_fail = Some((check.failures, size, check.error));
        }
    }

    match (best_pass, best_fail) {
        (Some(Passing { index, writes, diff_lines }), _) => Outcome::Chosen { index, writes, diff_lines, tried: n },
        (None, fail) => Outcome::HandOver { error: fail.map(|(_, _, error)| error).unwrap_or_default(), tried: n },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Tree {
        Tree::from([("a.py".to_string(), "def f(x):\n    return 0\n".to_string())])
    }

    /// One scripted candidate: (writes, passes, failures, error).
    type Candidate = (Vec<(String, String)>, bool, usize, &'static str);

    struct Script {
        candidates: Vec<Candidate>,
        asked: Vec<f32>,
        checked: usize,
    }

    impl Parties for Script {
        fn candidate(&mut self, _base: &Tree, temperature: f32, index: usize) -> Vec<(String, String)> {
            self.asked.push(temperature);
            self.candidates[index].0.clone()
        }

        fn check(&mut self, tree: &Tree) -> Check {
            let index = self.checked;

            self.checked += 1;

            let (writes, passed, failures, error) = &self.candidates[index];
            let applied = writes.iter().all(|(path, content)| tree.get(path) == Some(content));

            Check { passed: *passed && applied, failures: *failures, error: error.to_string() }
        }
    }

    fn write(text: &str) -> Vec<(String, String)> {
        vec![("a.py".to_string(), text.to_string())]
    }

    #[test]
    fn the_good_candidate_is_always_chosen_and_the_bad_ones_leave_no_trace() {
        crate::v3::forcing(&["nr.best_of_n"], || {
            let base = base();
            let mut script = Script {
                candidates: vec![
                    (write("def f(x):\n    return x +\n"), false, 3, "SyntaxError"),
                    (write("def f(x):\n    return x * 2\n"), true, 0, ""),
                    (write("def f(x):\n    return -1\n"), false, 1, "assert failed"),
                ],
                asked: vec![],
                checked: 0,
            };
            let outcome = run(&base, 3, &mut script);

            assert!(matches!(&outcome, Outcome::Chosen { index: 1, writes, tried: 3, .. } if writes[0].1.contains("x * 2")), "{outcome:?}");
            assert_eq!(base["a.py"], "def f(x):\n    return 0\n", "the base tree is untouched");
            assert_eq!(script.asked, vec![0.3, 0.7, 1.0], "varied temperatures");
        });
    }

    #[test]
    fn among_passing_candidates_the_smallest_diff_wins() {
        crate::v3::forcing(&["nr.best_of_n"], || {
            let mut script = Script {
                candidates: vec![
                    (write("def f(x):\n    y = x\n    z = y\n    w = z\n    return w * 2\n"), true, 0, ""),
                    (write("def f(x):\n    return x * 2\n"), true, 0, ""),
                ],
                asked: vec![],
                checked: 0,
            };
            let Outcome::Chosen { index, diff_lines, .. } = run(&base(), 2, &mut script) else { panic!() };

            assert_eq!(index, 1);
            assert_eq!(diff_lines, 2, "one line out, one in");
        });
    }

    #[test]
    fn if_none_passes_the_best_failure_goes_to_the_relay() {
        crate::v3::forcing(&["nr.best_of_n"], || {
            let mut script = Script {
                candidates: vec![(write("a"), false, 5, "five failures"), (write("b"), false, 1, "one failure"), (write("c"), false, 2, "two failures")],
                asked: vec![],
                checked: 0,
            };

            assert_eq!(run(&base(), 3, &mut script), Outcome::HandOver { error: "one failure".into(), tried: 3 });
        });
    }

    #[test]
    fn n_is_clamped_to_two_through_four() {
        crate::v3::forcing(&["nr.best_of_n"], || {
            let make = || Script { candidates: (0..4).map(|_| (write("x"), false, 1, "e")).collect(), asked: vec![], checked: 0 };
            let (mut one, mut nine) = (make(), make());

            assert!(matches!(run(&base(), 1, &mut one), Outcome::HandOver { tried: 2, .. }));
            assert!(matches!(run(&base(), 9, &mut nine), Outcome::HandOver { tried: 4, .. }));
        });
    }

    #[test]
    fn it_runs_only_after_the_normal_attempt_and_two_sliced_retries_or_for_a_hard_step_and_never_without_a_check() {
        let t = |normal_failed, sliced_retries_failed, hard, has_check| Trigger { normal_failed, sliced_retries_failed, hard, has_check };

        assert_eq!(should_run(&t(true, 2, false, true)), Err(NotRun::SwitchOff), "off by default");

        crate::v3::forcing(&["nr.best_of_n"], || {
            assert_eq!(should_run(&t(true, 2, false, true)), Ok(()));
            assert_eq!(should_run(&t(true, 1, false, true)), Err(NotRun::NotYet));
            assert_eq!(should_run(&t(false, 0, false, true)), Err(NotRun::NotYet));
            assert_eq!(should_run(&t(false, 0, true, true)), Ok(()));
            assert_eq!(should_run(&t(true, 5, true, false)), Err(NotRun::NoCheck), "no verifier, no selection");
        });
    }
}
