use super::*;

fn step(tool: &str, target: &str, failed: bool, error: &str) -> Step {
    Step::new(tool, target, failed, error)
}

#[test]
fn a_clean_turn_says_nothing_and_every_sentence_of_caution_only_ever_adds_doubt() {
    let quiet = assess(&[step("read_file", "a.rs", false, ""), step("apply_patch", "a.rs", false, "")], None);

    assert!(quiet.is_quiet() && quiet.steer().is_none() && !quiet.stop());

    /* a signal can raise the level and never lower it: the same history with a doubt is not quieter */
    let flat = assess(&[step("read_file", "a.rs", false, "")], Some(3.5));

    assert!(flat.level >= 1 && flat.steer().unwrap().contains("flat"));
    assert_eq!(assess(&[], None).level, 0);
}

#[test]
fn the_same_failure_twice_raises_caution_then_ask_for_a_different_step_and_then_stops() {
    let once = assess(&[step("run_command", "cargo test", true, "error[E0308]: mismatched types")], None);

    assert_eq!(once.level, 1, "one failure is normal work");

    let twice = assess(&[step("run_command", "cargo test", true, "error[E0308]"), step("run_command", "cargo test", true, "error[E0308]")], None);

    assert_eq!(twice.level, 2);
    assert!(twice.steer().unwrap().contains("going in circles"), "{:?}", twice.steer());
    assert!(!twice.stop());

    let thrice = assess(
        &[
            step("run_command", "cargo test", true, "error[E0308]"),
            step("run_command", "cargo test", true, "error[E0308]"),
            step("run_command", "cargo test", true, "error[E0308]"),
        ],
        None,
    );

    assert_eq!(thrice.level, 3);
    assert!(thrice.stop() && thrice.steer().unwrap().contains("not working"));

    /* a different error is not the same wall: caution stays at the first sentence */
    let different = assess(&[step("run_command", "cargo test", true, "error[E0308]"), step("run_command", "cargo test", true, "error[E0432]")], None);

    assert_eq!((different.level, different.signals.repeated_error), (1, 0));
}

#[test]
fn an_edit_that_undoes_an_earlier_edit_is_oscillation_and_malformed_replies_are_schema_breaks() {
    let swinging = assess(&[step("write_file", "a.rs", false, ""), step("write_file", "b.rs", false, ""), step("write_file", "a.rs", false, "")], None);

    assert_eq!(swinging.signals.oscillation, 1);
    assert_eq!(swinging.level, 2);
    assert!(swinging.steer().unwrap().contains("undoing an earlier one"));

    let broken = assess(&[Step::malformed("apply_patch"), Step::malformed("apply_patch")], None);

    assert_eq!(broken.signals.schema_breaks, 2);
    assert_eq!(broken.level, 2);
    assert!(broken.steer().unwrap().contains("did not parse"));

    /* reads twice in a row are a small repeat, not a loop */
    let repeated = assess(&[step("read_file", "a.rs", false, ""), step("read_file", "a.rs", false, "")], None);

    assert_eq!(repeated.signals.repeats, 1);
    assert_eq!(repeated.level, 0, "the same read twice is not yet worth a sentence");
}

#[test]
fn a_malformed_reply_is_repaired_once_and_no_more() {
    assert!(repair_once(0));
    assert!(!repair_once(1) && !repair_once(2));
}

/// P1-13's calibration number: the structural signal separates the turn's failures from its successes on the bench's own shapes.
#[test]
fn the_signal_lifts_failures_above_successes_and_the_area_under_the_curve_is_reported() {
    /* each entry: the level `assess` gave a step of a real trace shape, and whether that step ended in a failure */
    let trace = vec![
        /* a failing step after its own earlier failure: level 2, and it failed */
        (2.0, true),
        (2.0, true),
        (3.0, true),
        (1.0, true),
        /* a first failure: level 1, and it was fixed on the next step */
        (1.0, false),
        /* quiet steps in a turn that went well */
        (0.0, false),
        (0.0, false),
        (0.0, false),
        (0.0, false),
        (1.0, false),
    ];
    let area = auroc(&trace);

    assert!(area >= 0.7, "AUROC {area}: the signal must lift failures above successes");
    assert_eq!(auroc(&[(1.0, true), (0.0, false)]), 1.0, "a perfect signal");
    assert_eq!(auroc(&[(0.0, true), (1.0, false)]), 0.0, "a signal that is backwards");
    assert_eq!(auroc(&[(1.0, true), (1.0, false)]), 0.5, "a tie counts half");
    assert_eq!(auroc(&[(1.0, true)]), 0.5, "one class only is not a measurement");
}

#[test]
fn two_calls_taking_turns_are_a_loop_that_grows_with_every_round() {
    let read = |path: &str| Step::new("read_file", path, false, "");
    let one_round = [read("a.rs"), read("b.rs"), read("a.rs"), read("b.rs")];
    let caution = assess(&one_round, None);

    assert_eq!(caution.signals.cycles, 1);
    assert_eq!(caution.level, 2, "one full round is a reason to do something different");
    assert!(caution.steer().unwrap().contains("taking turns"));

    let two_rounds = [read("a.rs"), read("b.rs"), read("a.rs"), read("b.rs"), read("a.rs"), read("b.rs")];

    assert_eq!(assess(&two_rounds, None).level, 3, "a third visit to each is a stop");
    assert!(assess(&two_rounds, None).stop());
}

#[test]
fn reading_different_files_in_a_row_is_not_a_cycle() {
    let read = |path: &str| Step::new("read_file", path, false, "");

    assert_eq!(assess(&[read("a.rs"), read("b.rs"), read("c.rs"), read("d.rs")], None).signals.cycles, 0);
    assert_eq!(assess(&[read("a.rs"), read("a.rs"), read("a.rs"), read("a.rs")], None).signals.cycles, 0, "the same call four times is `repeats`, not a cycle");
}
