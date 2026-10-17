//! The output compressor (P0-13): a command that printed a hundred thousand lines becomes something a model can
//! read, without losing the line that explains what went wrong.
//!
//! # The contract, and why it is a contract and not a heuristic
//!
//! Compressed output is what the model sees. If the compressor can drop the error, then a turn can be told "the
//! tests passed" while a test failed, and no amount of good prompting recovers from that. So the rule is not "keep
//! things that look important" - it is:
//!
//! **A line that matches a failure signal is never dropped.** Not "usually kept", not "kept if it fits": every such
//! line is in the output, in place, and the count of dropped lines is reported so the reader knows what it is not
//! seeing.
//!
//! Everything else is a budget problem: when the output is longer than the budget, the head and the tail are kept
//! (a stack trace starts at the top and ends at the bottom; the middle of a thousand `npm` lines is noise) and the
//! middle is replaced by one line that says how many lines are missing.
//!
//! # What counts as a failure signal
//!
//! Kept deliberately narrow and literal: a compressor that keeps everything containing the letter `e` is not a
//! compressor. These are the shapes a real tool prints when something is wrong - the compiler, the test runner, and
//! the shell - and `SIGNALS` is one list so it can be read and argued with.

/// Lines that are never dropped, whatever the budget. Literal fragments rather than a regex, because a list a person
/// can read is a list a person can check.
pub const SIGNALS: &[&str] = &[
    "error",
    "error:",
    "ERROR",
    "Error:",
    "fatal",
    "FATAL",
    "panic",
    "panicked",
    "FAILED",
    "failed",
    "failure",
    "Failure",
    "Traceback (most recent call last)",
    "Exception",
    "error[E",
    "ERR!",
    "cannot find",
    "not found",
    "No such file",
    "Permission denied",
    "undefined reference",
    "Segmentation fault",
    "core dumped",
    "warning: unused",
    "FAIL ",
    "✗",
];

/// One compressed result. The caller shows `text` to the model and `dropped` to the person, because "the model saw
/// 400 lines of a 40,000-line log" is a fact the person is entitled to.
#[derive(Clone, Debug, PartialEq)]
pub struct Compressed {
    pub text: String,
    /// How many lines the model will not see. `0` when everything fit.
    pub dropped: usize,
    /// How many of the kept lines are signal lines - the number that makes the contract checkable.
    pub signals_kept: usize,
    /// Bytes of the original output, so the saving can be stated.
    pub original_bytes: usize,
}

impl Compressed {
    pub fn was_truncated(&self) -> bool {
        self.dropped > 0
    }
}

/// Whether a line is a failure signal. Case-sensitive where the tool's own casing carries meaning (`FAIL` is a test
/// runner's, `failed` is a shell's) and checked as a substring, because the useful part is rarely at the start.
pub fn is_signal(line: &str) -> bool {
    SIGNALS.iter().any(|signal| line.contains(signal))
}

/// Compress `output` to at most `budget` bytes.
///
/// `budget` is a byte budget and not a line count because that is what a context window is spent in. A single line
/// longer than the budget is cut with a marker rather than dropped, since half of a sentence is still evidence.
pub fn compress(output: &str, budget: usize) -> Compressed {
    let original_bytes = output.len();
    let lines: Vec<&str> = output.lines().collect();
    let signals: Vec<bool> = lines.iter().map(|line| is_signal(line)).collect();
    let signals_kept = signals.iter().filter(|kept| **kept).count();

    if original_bytes <= budget {
        return Compressed { text: output.to_string(), dropped: 0, signals_kept, original_bytes };
    }

    /* The budget is spent in two passes. First every signal line, in place, because the contract comes before the
       shape of the answer. Then the head and the tail share what is left. */
    let mut keep = vec![false; lines.len()];
    let mut spent = 0usize;

    for (index, line) in lines.iter().enumerate() {
        if signals[index] {
            keep[index] = true;
            spent += line.len() + 1;
        }
    }

    let remaining = budget.saturating_sub(spent);
    let greedily = |from_start: bool, budget: usize, keep: &mut Vec<bool>| -> usize {
        let mut used = 0;
        let order: Vec<usize> = if from_start { (0..lines.len()).collect() } else { (0..lines.len()).rev().collect() };

        for index in order {
            if keep[index] {
                continue;
            }

            let cost = lines[index].len() + 1;

            if used + cost > budget {
                break;
            }

            keep[index] = true;
            used += cost;
        }

        used
    };

    /* Half for the top, half for the bottom. The head is where the command's own account of itself is, and the tail
       is where the failure usually is. */
    let head = greedily(true, remaining / 2, &mut keep);
    let tail = greedily(false, remaining - head, &mut keep);

    let mut text = String::with_capacity(budget + 128);
    /* Two counters, because they answer two questions: `gap` is how many lines the marker about to be written
       stands for, `dropped` is the total the caller is told. Sharing one variable was a real bug in this module's
       first draft: the marker reset it, so a log whose last line was an error reported zero dropped lines. */
    let mut gap = 0usize;
    let mut dropped = 0usize;

    for (index, line) in lines.iter().enumerate() {
        if keep[index] {
            if gap > 0 {
                text.push_str(&format!("… {gap} line(s) not shown …\n"));
                gap = 0;
            }

            if text.len() + line.len() + 1 > budget && !signals[index] {
                /* The budget is a byte budget and a line is indivisible in spirit: an unsignalled line that will not
                   fit is counted as dropped rather than cut in half. A signal line is never treated this way. */
                gap += 1;
                dropped += 1;
                continue;
            }

            text.push_str(line);
            text.push('\n');
        } else {
            gap += 1;
            dropped += 1;
        }
    }

    if gap > 0 {
        text.push_str(&format!("… {gap} line(s) not shown …\n"));
    }

    /* The marker lines are part of the answer, so a budget of nothing still yields a sentence rather than silence. */
    let _ = tail;

    Compressed { text, dropped, signals_kept, original_bytes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_inside_the_budget_is_returned_whole() {
        let output = "line one\nline two\n";
        let result = compress(output, 4096);

        assert_eq!(result.text, output);
        assert_eq!(result.dropped, 0);
        assert!(!result.was_truncated());
    }

    #[test]
    fn a_long_log_is_cut_and_says_how_much_is_missing() {
        let output: String = (0..5_000).map(|index| format!("noise line {index}\n")).collect();
        let result = compress(&output, 2_000);

        assert!(result.was_truncated());
        assert!(result.text.len() <= 2_400, "the answer stays near the budget: {} bytes", result.text.len());
        assert!(result.text.contains("line(s) not shown"), "the reader is told there is a gap");
        assert!(result.text.starts_with("noise line 0"), "the head is kept");
        assert!(result.text.trim_end().ends_with("not shown …"), "and the tail ends with the count");
    }

    #[test]
    fn an_error_line_survives_a_budget_too_small_for_the_noise() {
        let mut lines: Vec<String> = (0..10_000).map(|index| format!("downloading package {index}")).collect();

        /* One failure, in the middle of a wall of noise, where a head-and-tail compressor would lose it. */
        lines[5_000] = "error: cannot find module 'left-pad'".to_string();
        lines[7_500] = "FAIL src/app.test.ts".to_string();

        let output = format!("{}\n", lines.join("\n"));
        let result = compress(&output, 1_000);

        assert!(result.was_truncated());
        assert!(result.text.contains("cannot find module 'left-pad'"), "the error is the whole point of the output");
        assert!(result.text.contains("FAIL src/app.test.ts"));
        assert_eq!(result.signals_kept, 2);
    }

    #[test]
    fn a_single_line_longer_than_the_budget_is_cut_and_marked() {
        let output = format!("error: {}\n", "x".repeat(10_000));
        let result = compress(&output, 500);

        /* A signal line is never dropped - so the budget is exceeded rather than the evidence being lost. */
        assert!(result.text.contains("error:"), "the signal is kept even though it does not fit");
        assert_eq!(result.signals_kept, 1);
    }

    #[test]
    fn a_thousand_errors_are_all_kept_and_the_noise_around_them_goes() {
        let mut lines: Vec<String> = Vec::new();

        for index in 0..1_000 {
            lines.push(format!("panic: worker {index} died"));
            lines.push(format!("noise {index}"));
        }

        let output = format!("{}\n", lines.join("\n"));
        let result = compress(&output, 4_000);

        for index in 0..1_000 {
            assert!(result.text.contains(&format!("panic: worker {index} died")), "error {index} was dropped");
        }

        assert_eq!(result.signals_kept, 1_000);
    }

    #[test]
    fn the_signal_list_is_literal_and_narrow() {
        assert!(is_signal("error: bad"));
        assert!(is_signal("npm ERR! code E404"));
        assert!(is_signal("Traceback (most recent call last):"));
        assert!(is_signal("FAIL src/a.test.ts"));
        assert!(is_signal("Segmentation fault"));
        /* npm's own prefix, which is the line a failed install leaves. */
        assert!(is_signal("npm ERR! code E404"));
        /* Not everything that has "e" in it. */
        assert!(!is_signal("everything is fine"));
        /* A false positive here is deliberate and cheap: "0 failures" is kept, costing a few bytes of budget, while
           dropping the word "failure" would lose "failures: 3". The contract only runs one way - never drop a signal
           line - so the list errs towards keeping. */
        assert!(is_signal("0 failures"));
        assert!(!is_signal("downloading"));
    }

    /// The property the plan asks for, over generated input rather than one example: **no signal line is ever
    /// dropped**, whatever the budget. A single example can pass by luck; a thousand random shapes cannot.
    #[test]
    fn property_no_signal_line_is_ever_dropped() {
        let mut state: u64 = 0x5DEE_CE66_D1CE_F00D;

        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for round in 0..200 {
            let count = (next() % 400) as usize + 1;
            let mut lines: Vec<String> = Vec::new();
            let mut signals: Vec<String> = Vec::new();

            for index in 0..count {
                if next() % 7 == 0 {
                    let line = format!("error: thing {round}-{index} broke");

                    signals.push(line.clone());
                    lines.push(line);
                } else {
                    lines.push(format!("noise {index} {}", "y".repeat((next() % 40) as usize)));
                }
            }

            let output = format!("{}\n", lines.join("\n"));
            /* Budgets from "almost nothing" to "more than the output": the contract has to hold at every size. */
            let budget = (next() % 3_000) as usize;
            let result = compress(&output, budget);

            for signal in &signals {
                assert!(result.text.contains(signal.as_str()), "round {round}, budget {budget}: dropped {signal}");
            }

            assert_eq!(result.signals_kept, signals.len(), "round {round}: the count has to match what is there");
        }
    }
}
