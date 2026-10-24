use super::*;

/// The scenarios run once for all the tests: the edit gate's "same text again" memory is process-wide, so two runs side by side would see each other.
fn all_rows() -> &'static [Row] {
    static ROWS: std::sync::OnceLock<Vec<Row>> = std::sync::OnceLock::new();

    ROWS.get_or_init(run_all)
}

fn row<'a>(rows: &'a [Row], switch: &str) -> &'a Row {
    rows.iter().find(|row| row.switch == switch).unwrap_or_else(|| panic!("no row for {switch}"))
}

#[test]
fn the_bench_runs_end_to_end_and_every_navaratna_switch_has_a_row() {
    let rows = all_rows();
    let mut expected: Vec<&str> = crate::v3::FEATURES.iter().filter(|f| f.name.starts_with("nr.")).map(|f| f.name).collect();
    let mut got: Vec<&str> = rows.iter().map(|r| r.switch).collect();

    expected.sort();
    got.sort();

    assert_eq!(got, expected, "one scenario per nr.* switch");
}

#[test]
fn the_planted_traps_are_caught_with_the_switch_on_and_not_with_it_off() {
    let rows = all_rows();

    for (switch, off, on) in [
        ("nr.sentence_cover", 1.0, 0.0),
        ("nr.registry", 1.0, 0.0),
        ("nr.out_chunk", 1.0, 0.0),
        ("nr.edit_gate", 5.0, 0.0),
        ("nr.symbol_patch", 3.0, 0.0),
        ("nr.test_first", 1.0, 0.0),
        ("nr.exist_gate", 2.0, 0.0),
        ("nr.lang_spec", 3.0, 0.0),
        ("nr.ctx_lock", 2.0, 0.0),
        ("nr.contract_lock", 1.0, 0.0),
        ("nr.git_shadow", 2.0, 0.0),
        ("nr.tool_envelope", 4.0, 0.0),
        ("nr.role_rotation", 1.0, 0.0),
        ("nr.mem_budget", 1.0, 0.0),
    ] {
        let row = row(rows, switch);

        assert_eq!((row.off, row.on), (off, on), "{switch}: {row:?}");
        assert_eq!(row.verdict(), "helps", "{switch}");
    }
}

#[test]
fn the_token_scenarios_shrink_and_the_rates_rise() {
    let rows = all_rows();

    for switch in ["nr.fresh_step", "nr.skeleton_pack", "nr.error_slice", "nr.escalation_packet"] {
        let row = row(rows, switch);

        assert!(row.on < row.off, "{switch}: {row:?}");
    }

    let golden = row(rows, "nr.golden_trace");

    assert_eq!((golden.off, golden.on), (0.0, 0.5));

    let share = row(rows, "nr.best_of_n");

    assert!((share.off - 0.7).abs() < 1e-9 && (share.on - 0.9).abs() < 1e-9, "{share:?}");
    assert!(row(rows, "nr.think_budget").on < row(rows, "nr.think_budget").off);
    assert_eq!(row(rows, "nr.blueprint").on, 0.0, "no consent, no cloud call, switch on or off");
    assert_eq!(row(rows, "nr.quant_advice").verdict(), "info");
    assert_eq!((row(rows, "nr.symbol_index").off, row(rows, "nr.symbol_index").on), (0.0, 2.0));
    assert_eq!((row(rows, "nr.tag_route").off, row(rows, "nr.tag_route").on), (0.0, 2.0));
}

#[test]
fn no_switch_makes_its_own_metric_worse() {
    for row in all_rows() {
        assert_ne!(row.verdict(), "worse", "{row:?}");
    }
}

#[test]
fn the_report_is_a_table_with_one_line_per_switch_and_says_it_is_scripted() {
    let rows = all_rows();
    let text = render(rows);

    assert!(text.contains("SCRIPTED run on a fake model") && text.contains("| switch | metric | off | on | verdict | ms |"));
    assert_eq!(text.lines().filter(|line| line.starts_with("| nr.")).count(), rows.len());
    assert!(text.contains("switches measured:"));
}

#[test]
fn live_mode_is_honestly_owner_later() {
    let error = command(&["--live".to_string()]).unwrap_err();

    assert!(error.contains("OWNER-LATER") && error.contains("Ollama"));
}

#[test]
fn the_bench_leaves_the_switches_as_it_found_them() {
    let before = crate::v3::on("nr.edit_gate");

    all_rows();
    assert_eq!(crate::v3::on("nr.edit_gate"), before);
}
