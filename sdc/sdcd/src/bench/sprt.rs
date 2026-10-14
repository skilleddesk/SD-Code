//! **Sequential testing for the bench** (v3 Q4): a comparison run costs money, and "we have 20 pairs, look at the
//! numbers" either stops too early (a lucky run reads as an improvement) or runs far past the point where the
//! answer was already clear. The SPRT decides as the pairs arrive: keep going, the candidate is better, or the
//! candidate is not better - with the error rates the test asks for. The CUSUM watches a model's pass rate over
//! time and says when it has drifted away from the rate it was chosen for.
//!
//! Both work on what the bench already records: pairs where one side passed and the other did not (a pair where
//! both passed, or both failed, says nothing about the difference).

/// What the sequential test says about the pairs so far.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum Decision {
    /// Nothing can be said yet: take more pairs.
    KeepSampling { log_likelihood: f64 },
    /// The candidate is better than the baseline by at least the effect being tested for.
    Accept { log_likelihood: f64 },
    /// The candidate is not better.
    Reject { log_likelihood: f64 },
}

/// The sequential test on paired wins: `candidate_wins` pairs where only the candidate passed, `baseline_wins`
/// where only the baseline passed. `p1` is the improvement worth detecting (0.05 = five points on pairs).
///
/// A pair where one side wins is a fair coin under "no better" (p0 = 0.5) and `0.5 + p1 / 2` under "better"; the
/// log-likelihood ratio between the two is compared with the boundaries the person's error rates give.
pub fn sprt(candidate_wins: u32, baseline_wins: u32, p1: f64, alpha: f64, beta: f64) -> Decision {
    let p0 = 0.5;
    let p1 = (0.5 + p1 / 2.0).clamp(p0 + 0.001, 0.999);

    let logarithm = |p: f64, wins: u32, losses: u32| wins as f64 * p.ln() + losses as f64 * (1.0 - p).ln();
    let ratio = logarithm(p1, candidate_wins, baseline_wins) - logarithm(p0, candidate_wins, baseline_wins);
    let upper = ((1.0 - beta) / alpha).ln();
    let lower = (beta / (1.0 - alpha)).ln();

    if ratio >= upper {
        Decision::Accept { log_likelihood: ratio }
    } else if ratio <= lower {
        Decision::Reject { log_likelihood: ratio }
    } else {
        Decision::KeepSampling { log_likelihood: ratio }
    }
}

/// The pairs the test takes: `(candidate passed, baseline passed)`.
pub fn from_pairs(pairs: &[(bool, bool)]) -> (u32, u32) {
    let candidate_wins = pairs.iter().filter(|(candidate, baseline)| *candidate && !*baseline).count() as u32;
    let baseline_wins = pairs.iter().filter(|(candidate, baseline)| !*candidate && *baseline).count() as u32;

    (candidate_wins, baseline_wins)
}

/// A drift alarm (CUSUM): given a model's pass rates in order, `Some(index)` when the rate has slipped away from
/// `target` by more than `alarm` in total - the model changed under the profile, or its provider did.
pub fn drift(rates: &[f64], target: f64, slack: f64, alarm: f64) -> Option<usize> {
    let mut sum = 0.0;

    for (index, rate) in rates.iter().enumerate() {
        sum = (sum + (target - slack) - rate).max(0.0);

        if sum > alarm {
            return Some(index);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_stops_when_the_answer_is_clear_and_waits_when_it_is_not() {
        /* A five-point effect is small: 120 pairs one way and 40 the other is where it becomes clear. */
        assert!(matches!(sprt(120, 40, 0.05, 0.05, 0.05), Decision::Accept { .. }));
        assert!(matches!(sprt(30, 5, 0.15, 0.05, 0.05), Decision::Accept { .. }), "a fifteen-point effect shows in 35 pairs");
        assert!(matches!(sprt(30, 5, 0.05, 0.05, 0.05), Decision::KeepSampling { .. }), "a 30-5 run is not yet enough for a five-point effect");

        /* A clear regression. */
        assert!(matches!(sprt(40, 120, 0.05, 0.05, 0.05), Decision::Reject { .. }));

        /* Nothing like enough evidence either way. */
        assert!(matches!(sprt(6, 5, 0.05, 0.05, 0.05), Decision::KeepSampling { .. }));
        assert!(matches!(sprt(0, 0, 0.05, 0.05, 0.05), Decision::KeepSampling { .. }));

        let pairs: Vec<(bool, bool)> = (0..30).map(|index| (index % 6 != 0, false)).collect();

        assert_eq!(from_pairs(&pairs), (25, 0), "pairs both passed or both failed say nothing");
    }

    #[test]
    fn a_model_that_slips_away_from_its_rate_raises_the_alarm() {
        let steady = [0.8, 0.78, 0.82, 0.79, 0.81, 0.8];

        assert_eq!(drift(&steady, 0.8, 0.1, 0.5), None);

        let slipped = [0.8, 0.8, 0.5, 0.5, 0.45, 0.4];

        assert_eq!(drift(&slipped, 0.8, 0.1, 0.5), Some(4), "the alarm comes two runs after the slip starts");
    }
}
