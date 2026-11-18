//! The Mann-Whitney U rank test of two samples, for the timing oracles (#1145): a few lines of
//! our own, with no dependency. Values may be `f64::INFINITY`, which ranks above every finite
//! value, and ties share their mid-rank. The p-value is two-sided, from the normal
//! approximation with the tie correction and no continuity correction, which is accurate enough
//! at 20 values per sample for the thresholds the oracles use (p < 0.001).

// Each test file uses its own subset of these helpers.
#![allow(dead_code)]

/// The outcome of [`mann_whitney`].
#[derive(Clone, Copy, Debug)]
pub struct RankTest {
    /// U of the first sample: how many pairs `(a, b)` have `a > b`, ties counting one half.
    pub u: f64,
    /// The standardized U, with the tie correction; 0 when every value is tied.
    pub z: f64,
    /// The two-sided p-value of the normal approximation.
    pub p: f64,
}

/// The Mann-Whitney U test of `a` against `b`: O((n + m) log(n + m)).
pub fn mann_whitney(a: &[f64], b: &[f64]) -> RankTest {
    let mut all: Vec<(f64, bool)> = a.iter().map(|&value| (value, true)).collect();
    all.extend(b.iter().map(|&value| (value, false)));
    all.sort_by(|x, y| x.0.total_cmp(&y.0));
    let total = all.len();
    // The sum of the first sample's ranks, ties at their mid-rank, and the sum of t^3 - t over
    // the groups of t tied values.
    let (mut rank_sum, mut ties) = (0.0f64, 0.0f64);
    let mut start = 0;
    while start < total {
        let mut end = start;
        while end + 1 < total && all[end + 1].0 == all[start].0 {
            end += 1;
        }
        let rank = (start + end) as f64 / 2.0 + 1.0;
        let group = (end - start + 1) as f64;
        ties += group * group * group - group;
        rank_sum += rank * all[start..=end].iter().filter(|(_, first)| *first).count() as f64;
        start = end + 1;
    }
    let (n, m, all_n) = (a.len() as f64, b.len() as f64, total as f64);
    let u = rank_sum - n * (n + 1.0) / 2.0;
    let mean = n * m / 2.0;
    let variance = if all_n > 1.0 {
        n * m / 12.0 * ((all_n + 1.0) - ties / (all_n * (all_n - 1.0)))
    } else {
        0.0
    };
    if variance <= 0.0 {
        return RankTest { u, z: 0.0, p: 1.0 };
    }
    let z = (u - mean) / variance.sqrt();
    let p = erfc(z.abs() / std::f64::consts::SQRT_2).min(1.0);
    RankTest { u, z, p }
}

/// The complementary error function, by the Chebyshev fit of Numerical Recipes (`erfcc`), with
/// a fractional error below 1.2e-7 everywhere.
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let value = t * exp(poly);
    if x >= 0.0 { value } else { 2.0 - value }
}

/// `e^x` for the `x <= 0` that `erfc` takes, with no call into the platform C library: `x` is
/// `k ln 2 + r` with `|r| <= ln 2`, `e^r` its Taylor series to the term that no longer changes the
/// sum, and `2^k` the exponent bits. Relative error near one ulp; 0 below the normal range, which
/// no p-value here can tell from it.
fn exp(x: f64) -> f64 {
    // Truncation toward zero by conversion, an instruction, not a libm call.
    let k = (x / std::f64::consts::LN_2) as i64;
    if k < -1021 {
        return 0.0;
    }
    let r = x - k as f64 * std::f64::consts::LN_2;
    let (mut sum, mut term, mut n) = (1.0_f64, 1.0_f64, 1.0_f64);
    loop {
        term *= r / n;
        let next = sum + term;
        if next == sum {
            break;
        }
        sum = next;
        n += 1.0;
    }
    // 2^k for k in -1021..=0: a normal number, its biased exponent k + 1023.
    let scale = f64::from_bits(((k + 1023) as u64) << 52);
    sum * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `actual` is within a relative `tolerance` of `expected`.
    fn close(actual: f64, expected: f64, tolerance: f64) -> bool {
        (actual - expected).abs() <= tolerance * expected.abs()
    }

    #[test]
    fn exp_matches_known_values() {
        assert_eq!(exp(0.0), 1.0);
        assert!(close(exp(-1.0), 0.367_879_441_171_442_3, 1e-15));
        assert!(close(exp(-10.0), 4.539_992_976_248_485e-5, 1e-14));
        assert!(close(exp(-700.0), 9.859_676_543_759_77e-305, 1e-12));
        assert_eq!(exp(-800.0), 0.0);
    }

    #[test]
    fn identical_samples_give_p_of_one() {
        let sample: Vec<f64> = (0..20).map(f64::from).collect();
        let test = mann_whitney(&sample, &sample);
        assert!((test.u - 200.0).abs() < 1e-9, "{test:?}");
        assert!(test.p > 0.999, "{test:?}");
        // Every value tied: no spread to test, so p is 1.
        let flat = vec![5.0; 20];
        assert!(mann_whitney(&flat, &flat).p > 0.999);
    }

    #[test]
    fn fully_separated_samples_give_p_below_one_in_a_million() {
        let low: Vec<f64> = (0..20).map(f64::from).collect();
        let high: Vec<f64> = (20..40).map(f64::from).collect();
        let test = mann_whitney(&high, &low);
        assert!((test.u - 400.0).abs() < 1e-9, "{test:?}");
        assert!(test.p < 1e-6, "{test:?}");
        // Timeouts rank above every time: 20 timeouts against 20 times separate as fully.
        let late = vec![f64::INFINITY; 20];
        assert!(mann_whitney(&late, &low).p < 1e-6);
    }

    #[test]
    fn worked_examples_match_their_reference_values() {
        // References from the same formula with the standard library's erfc of Python 3
        // (`math.erfc`), independent of the fit above: [1, 2, 3] against [4, 5, 6], U = 0,
        // z = -1.9639610121239315, p = 0.049534613435626755; with ties, [1, 2, 2, 3, 7]
        // against [2, 3, 4, 5, 6, 8], U = 7.5, z = -1.3851370514006154,
        // p = 0.166010564370933; and 0..20 against 20..40, z = -5.410017808004594,
        // p = 6.301848221392315e-8.
        let test = mann_whitney(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]);
        assert!(test.u.abs() < 1e-9, "{test:?}");
        assert!(close(test.z, -1.963_961_012_123_931_5, 1e-12), "{test:?}");
        assert!(close(test.p, 0.049_534_613_435_626_755, 1e-6), "{test:?}");
        let tied = mann_whitney(&[1.0, 2.0, 2.0, 3.0, 7.0], &[2.0, 3.0, 4.0, 5.0, 6.0, 8.0]);
        assert!((tied.u - 7.5).abs() < 1e-9, "{tied:?}");
        assert!(close(tied.z, -1.385_137_051_400_615_4, 1e-12), "{tied:?}");
        assert!(close(tied.p, 0.166_010_564_370_933, 1e-6), "{tied:?}");
        let low: Vec<f64> = (0..20).map(f64::from).collect();
        let high: Vec<f64> = (20..40).map(f64::from).collect();
        let apart = mann_whitney(&low, &high);
        assert!(close(apart.z, -5.410_017_808_004_594, 1e-12), "{apart:?}");
        assert!(close(apart.p, 6.301_848_221_392_315e-8, 1e-6), "{apart:?}");
    }
}
