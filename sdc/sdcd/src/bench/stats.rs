//! The statistics the bench reports with (v3 W1): medians with a bootstrap 95% interval, McNemar's exact test
//! for paired pass/fail, and the Q3 acceptance rule. Deterministic - the bootstrap uses a fixed-seed generator,
//! so the same results always print the same report.

/// The median of a sample; `None` for an empty one.
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();

    sorted.sort_by(|a, b| a.total_cmp(b));

    let middle = sorted.len() / 2;

    Some(if sorted.len() % 2 == 0 { (sorted[middle - 1] + sorted[middle]) / 2.0 } else { sorted[middle] })
}

/// A small fixed-seed generator (xorshift64*), enough for resampling.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const RESAMPLES: usize = 2_000;
const SEED: u64 = 0x5DC0_BE7C_0FFE_E001;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = p * (sorted.len() - 1) as f64;
    let (low, high) = (rank.floor() as usize, rank.ceil() as usize);

    sorted[low] + (sorted[high] - sorted[low]) * (rank - low as f64)
}

/// The percentile-bootstrap 95% interval of a statistic over one sample.
pub fn bootstrap_ci(values: &[f64], statistic: fn(&[f64]) -> Option<f64>) -> Option<(f64, f64)> {
    if values.len() < 2 {
        return None;
    }

    let mut rng = Rng(SEED);
    let mut estimates: Vec<f64> = (0..RESAMPLES)
        .filter_map(|_| {
            let sample: Vec<f64> = (0..values.len()).map(|_| values[rng.below(values.len())]).collect();

            statistic(&sample)
        })
        .collect();

    estimates.sort_by(|a, b| a.total_cmp(b));

    Some((percentile(&estimates, 0.025), percentile(&estimates, 0.975)))
}

/// The paired relative saving `1 - candidate/baseline`, per pair, as a median with its bootstrap 95% interval.
/// Pairs are the same task (and run index) under two configurations; a pair whose baseline is 0 is skipped.
pub fn paired_saving(pairs: &[(f64, f64)]) -> Option<(f64, (f64, f64))> {
    let savings: Vec<f64> = pairs.iter().filter(|(baseline, _)| *baseline > 0.0).map(|(baseline, candidate)| 1.0 - candidate / baseline).collect();
    let middle = median(&savings)?;
    let interval = bootstrap_ci(&savings, median).unwrap_or((middle, middle));

    Some((middle, interval))
}

/// McNemar's exact (binomial) two-sided p-value from the discordant pairs: `b` = baseline passed and the
/// candidate failed, `c` = the reverse.
pub fn mcnemar_p(b: u64, c: u64) -> f64 {
    let n = b + c;

    if n == 0 {
        return 1.0;
    }

    let k = b.min(c);
    /* P(X <= k) for X ~ Binomial(n, 1/2), summed in log space so a large n does not overflow. */
    let ln_half_n = n as f64 * 0.5f64.ln();
    let mut ln_choose = 0.0; // ln C(n, 0)
    let mut tail = 0.0;

    for i in 0..=k {
        if i > 0 {
            ln_choose += ((n - i + 1) as f64).ln() - (i as f64).ln();
        }

        tail += (ln_choose + ln_half_n).exp();
    }

    (2.0 * tail).min(1.0)
}

/// The Q3 decision for one (model × technique): accepted only if quality holds *and* the saving is real.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Verdict {
    pub accepted: bool,
    pub p_value: f64,
    /// Candidate pass rate minus baseline pass rate, in percentage points.
    pub pass_delta_points: f64,
    pub saving: Option<(f64, (f64, f64))>,
    pub reasons: Vec<String>,
}

/// The plan's rule: McNemar p >= 0.1, pass-rate drop <= 1 point, and a token-saving 95% CI above 0.
/// `pairs` are (baseline passed, candidate passed, baseline tokens, candidate tokens), one per paired run.
pub fn q3(pairs: &[(bool, bool, f64, f64)]) -> Verdict {
    let b = pairs.iter().filter(|(base, cand, ..)| *base && !*cand).count() as u64;
    let c = pairs.iter().filter(|(base, cand, ..)| !*base && *cand).count() as u64;
    let p_value = mcnemar_p(b, c);
    let n = pairs.len().max(1) as f64;
    let base_rate = pairs.iter().filter(|pair| pair.0).count() as f64 / n;
    let cand_rate = pairs.iter().filter(|pair| pair.1).count() as f64 / n;
    let pass_delta_points = (cand_rate - base_rate) * 100.0;
    let saving = paired_saving(&pairs.iter().map(|pair| (pair.2, pair.3)).collect::<Vec<_>>());
    let mut reasons = Vec::new();

    if pairs.is_empty() {
        reasons.push("no paired runs".to_string());
    }

    if p_value < 0.1 {
        reasons.push(format!("McNemar p = {p_value:.3} < 0.1"));
    }

    if pass_delta_points < -1.0 {
        reasons.push(format!("pass rate fell {:.1} points", -pass_delta_points));
    }

    match saving {
        Some((_, (low, _))) if low > 0.0 => {}
        Some((_, (low, _))) => reasons.push(format!("saving CI lower bound {:.1}% is not above 0", low * 100.0)),
        None => reasons.push("no token data".to_string()),
    }

    Verdict { accepted: reasons.is_empty(), p_value, pass_delta_points, saving, reasons }
}

/// Coefficient of variation (A-03): standard deviation over mean.
pub fn cv(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;

    if mean == 0.0 {
        return None;
    }

    let variance = values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;

    Some(variance.sqrt() / mean)
}

/// p50 and p95 of timings (search bench).
pub fn p50_p95(values: &[f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();

    sorted.sort_by(|a, b| a.total_cmp(b));

    Some((percentile(&sorted, 0.5), percentile(&sorted, 0.95)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medians_of_odd_and_even_samples() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn mcnemar_matches_the_binomial_table() {
        /* b = 0, c = 6: 2 * 0.5^6 = 0.03125. */
        assert!((mcnemar_p(0, 6) - 0.03125).abs() < 1e-9);
        /* b = 2, c = 8: 2 * P(X <= 2 | n = 10) = 2 * 56/1024 = 0.109375. */
        assert!((mcnemar_p(2, 8) - 0.109375).abs() < 1e-9);
        assert_eq!(mcnemar_p(0, 0), 1.0);
        assert_eq!(mcnemar_p(5, 5), 1.0);
        assert!(mcnemar_p(400, 600) < 1e-9, "large n stays finite");
    }

    #[test]
    fn the_bootstrap_interval_is_reproducible_and_holds_the_median() {
        let values: Vec<f64> = (1..=40).map(|value| value as f64).collect();
        let first = bootstrap_ci(&values, median).unwrap();

        assert_eq!(first, bootstrap_ci(&values, median).unwrap(), "fixed seed");
        assert!(first.0 < 20.5 && 20.5 < first.1);
    }

    #[test]
    fn q3_accepts_a_real_saving_that_keeps_quality_and_refuses_the_rest() {
        let good: Vec<(bool, bool, f64, f64)> = (0..20).map(|index| (index % 5 != 0, index % 5 != 0, 1_000.0 + index as f64, 600.0)).collect();
        let verdict = q3(&good);

        assert!(verdict.accepted, "{:?}", verdict.reasons);
        assert!((verdict.saving.unwrap().0 - 0.4).abs() < 0.02);

        /* Same saving, but six tasks that passed now fail. */
        let worse: Vec<(bool, bool, f64, f64)> = (0..20).map(|index| (true, index >= 6, 1_000.0, 600.0)).collect();
        let verdict = q3(&worse);

        assert!(!verdict.accepted);
        assert!(verdict.reasons.iter().any(|reason| reason.contains("McNemar")));
        assert!(verdict.reasons.iter().any(|reason| reason.contains("points")));

        /* Quality held, but no saving. */
        let flat: Vec<(bool, bool, f64, f64)> = (0..20).map(|index| (true, true, 1_000.0, 990.0 + (index % 3) as f64 * 10.0)).collect();

        assert!(!q3(&flat).accepted);
    }

    #[test]
    fn variation_and_percentiles() {
        assert!((cv(&[10.0, 10.0, 10.0]).unwrap()).abs() < 1e-12);
        assert!(cv(&[8.0, 12.0]).unwrap() > 0.2);
        assert_eq!(p50_p95(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap().0, 3.0);
    }
}
