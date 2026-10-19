//! The exact operations of `Decimal`, case by case and against brute-force references.

use super::*;
use Rounding::{
    AwayFromZero, HalfAwayFromZero, HalfEven, HalfTowardZero, TowardNegative, TowardPositive,
    TowardZero,
};

fn int(n: i64) -> BigInt {
    BigInt::from(n)
}

#[test]
fn rounding_modes_on_ties_and_beyond() {
    // n / d, then each mode in the order of `Rounding::ALL`.
    let cases: [(i64, i64, [i64; 7]); 12] = [
        (5, 2, [2, 3, 2, 2, 3, 3, 2]),           // 2.5
        (-5, 2, [-2, -3, -2, -2, -3, -2, -3]),   // -2.5
        (7, 2, [4, 4, 3, 3, 4, 4, 3]),           // 3.5
        (7, -2, [-4, -4, -3, -3, -4, -3, -4]),   // -3.5
        (-7, 2, [-4, -4, -3, -3, -4, -3, -4]),   // -3.5
        (-7, -2, [4, 4, 3, 3, 4, 4, 3]),         // 3.5
        (26, 10, [3, 3, 3, 2, 3, 3, 2]),         // 2.6
        (24, 10, [2, 2, 2, 2, 3, 3, 2]),         // 2.4
        (-24, 10, [-2, -2, -2, -2, -3, -2, -3]), // -2.4
        (6, 3, [2, 2, 2, 2, 2, 2, 2]),           // exact
        (0, 7, [0, 0, 0, 0, 0, 0, 0]),           // zero
        (0, -7, [0, 0, 0, 0, 0, 0, 0]),          // no negative zero
    ];
    for (n, d, expected) in cases {
        for (mode, want) in Rounding::ALL.iter().zip(expected) {
            assert_eq!(
                round_div(&int(n), &int(d), *mode),
                Some(int(want)),
                "{n} / {d} by {mode:?}"
            );
        }
    }
    assert_eq!(round_div(&int(1), &int(0), HalfEven), None);
}

/// The rounding of n / d by brute force over rationals: the two integers around the quotient,
/// chosen by comparing distances in exact integer arithmetic.
fn reference(n: i64, d: i64, mode: Rounding) -> i64 {
    // The quotient's sign moved onto the numerator: n2 / d2 with d2 > 0.
    let (n2, d2) = if d < 0 { (-n, -d) } else { (n, d) };
    let floor = n2.div_euclid(d2);
    let ceil = if n2.rem_euclid(d2) == 0 {
        floor
    } else {
        floor + 1
    };
    if floor == ceil {
        return floor;
    }
    // Distances to floor and ceil, times d2: n2 - floor*d2 and ceil*d2 - n2.
    let below = n2 - floor * d2;
    let above = ceil * d2 - n2;
    let positive = n2 > 0;
    let toward_zero = if positive { floor } else { ceil };
    let away = if positive { ceil } else { floor };
    match mode {
        TowardZero => toward_zero,
        AwayFromZero => away,
        TowardPositive => ceil,
        TowardNegative => floor,
        HalfEven | HalfAwayFromZero | HalfTowardZero => match below.cmp(&above) {
            std::cmp::Ordering::Less => floor,
            std::cmp::Ordering::Greater => ceil,
            std::cmp::Ordering::Equal => match mode {
                HalfEven => {
                    if floor % 2 == 0 {
                        floor
                    } else {
                        ceil
                    }
                }
                HalfAwayFromZero => away,
                _ => toward_zero,
            },
        },
    }
}

#[test]
fn rounding_agrees_with_the_rational_reference() {
    for n in -60..=60 {
        for d in (-13..=13).filter(|d| *d != 0) {
            for mode in Rounding::ALL {
                assert_eq!(
                    round_div(&int(n), &int(d), mode),
                    Some(int(reference(n, d, mode))),
                    "{n} / {d} by {mode:?}"
                );
            }
        }
    }
}

#[test]
fn rescale_widen_and_exactness() {
    assert_eq!(widen(&int(1999), 2, 4), int(199_900));
    assert_eq!(rescale(&int(12_345), 3, 1, HalfEven), int(123));
    assert_eq!(rescale(&int(12_350), 3, 1, HalfEven), int(124));
    assert_eq!(rescale(&int(12_250), 3, 1, HalfEven), int(122));
    assert_eq!(rescale(&int(-12_250), 3, 1, HalfAwayFromZero), int(-123));
    assert_eq!(rescale(&int(-12_201), 3, 1, TowardPositive), int(-122));
    assert_eq!(rescale(&int(-12_201), 3, 1, TowardNegative), int(-123));
    assert!(rescales_exactly(&int(12_300), 3, 1));
    assert!(!rescales_exactly(&int(12_301), 3, 1));
    assert!(rescales_exactly(&int(-500), 2, 0));
}

#[test]
fn divide_and_remainder() {
    // (1.00).divide(by: 3.00, scale: 2, rounding: HalfEven) is 0.33.
    assert_eq!(
        divide(&int(100), 2, &int(300), 2, 2, HalfEven),
        Some(int(33))
    );
    // 7 by -2 at scale 0, and the other signs.
    assert_eq!(divide(&int(7), 0, &int(-2), 0, 0, HalfEven), Some(int(-4)));
    assert_eq!(
        divide(&int(-7), 0, &int(2), 0, 0, TowardZero),
        Some(int(-3))
    );
    assert_eq!(
        divide(&int(-7), 0, &int(-2), 0, 0, TowardNegative),
        Some(int(3))
    );
    // 19.99 / 3 at scale 2: 6.663.. → 6.66 or 6.67.
    assert_eq!(
        divide(&int(1999), 2, &int(3), 0, 2, TowardZero),
        Some(int(666))
    );
    assert_eq!(
        divide(&int(1999), 2, &int(3), 0, 2, AwayFromZero),
        Some(int(667))
    );
    assert_eq!(divide(&int(1), 0, &int(0), 0, 2, HalfEven), None);
    assert_eq!(remainder(&int(1), 0, &int(0), 0, 2), None);
    // x == y × q + remainder, with the remainder's sign that of x, over a grid.
    for x in -40..=40 {
        for y in (-9..=9).filter(|y| *y != 0) {
            for (a, b, s) in [(0, 0, 0), (2, 0, 2), (2, 2, 1), (1, 3, 0), (3, 1, 2)] {
                let (xi, yi) = (int(x), int(y));
                let q = divide(&xi, a, &yi, b, s, TowardZero).unwrap();
                let r = remainder(&xi, a, &yi, b, s).unwrap();
                let t = remainder_scale(a, b, s);
                let rebuilt = widen(&(&yi * &q), b + s, t) + &r;
                assert_eq!(rebuilt, widen(&xi, a, t), "{x}/{y} at {a} {b} {s}");
                assert!(r.sign() == Sign::NoSign || r.sign() == xi.sign(), "{x}/{y}");
                // |remainder| < |y| × 10^-s in value: |r| × 10^(b+s) < |y| × 10^t.
                assert!(
                    widen(
                        &BigInt::from_biguint(Sign::Plus, r.magnitude().clone()),
                        t,
                        t + b + s
                    ) < widen(
                        &BigInt::from_biguint(Sign::Plus, yi.magnitude().clone()),
                        0,
                        t
                    ) * pow10(u32::from(t))
                );
            }
        }
    }
}

#[test]
fn rendering() {
    for (c, s, text) in [
        (-1250, 2, "-12.50"),
        (5, 2, "0.05"),
        (-5, 2, "-0.05"),
        (0, 2, "0.00"),
        (0, 0, "0"),
        (42, 0, "42"),
        (-42, 0, "-42"),
        (1, 5, "0.00001"),
        (123_456, 3, "123.456"),
    ] {
        assert_eq!(render(&int(c), s), text, "{c} at {s}");
    }
}

#[test]
fn parsing_and_its_failures() {
    let ok = |t: &str, s: u8| parse(t, s).unwrap();
    assert_eq!(ok("12.5", 2), int(1250));
    assert_eq!(ok("-0.05", 2), int(-5));
    assert_eq!(ok("-0", 2), int(0));
    assert_eq!(ok("7", 2), int(700));
    assert_eq!(ok("1.230000", 2), int(123));
    assert_eq!(ok("0.000", 0), int(0));
    for (t, failure) in [
        ("", ParseFailure::Empty),
        ("-", ParseFailure::NoDigits),
        ("+1", ParseFailure::NotADigit(0)),
        (" 1", ParseFailure::NotADigit(0)),
        ("01", ParseFailure::NotADigit(1)),
        ("-01", ParseFailure::NotADigit(2)),
        ("1.", ParseFailure::NotADigit(2)),
        (".5", ParseFailure::NotADigit(0)),
        ("1e5", ParseFailure::NotADigit(1)),
        ("1.5 ", ParseFailure::NotADigit(3)),
        ("1,5", ParseFailure::NotADigit(1)),
        ("--1", ParseFailure::NotADigit(1)),
        ("1.2.3", ParseFailure::NotADigit(3)),
        ("1.005", ParseFailure::NotADigit(4)),
        ("1.0001", ParseFailure::NotADigit(5)),
        ("１", ParseFailure::NotADigit(0)),
        ("nan", ParseFailure::NotADigit(0)),
    ] {
        assert_eq!(parse(t, 2), Err(failure), "{t:?}");
    }
}

#[test]
fn the_digit_cap_is_shared_and_linear() {
    let at = "9".repeat(MAX_INT_DIGITS - 2);
    assert!(parse(&format!("{at}.99"), 2).is_ok());
    let over = "9".repeat(MAX_INT_DIGITS - 1);
    assert_eq!(parse(&format!("{over}.99"), 2), Err(ParseFailure::TooLong));
    // Leading zeros of the fraction do not count; trailing zeros beyond the scale are dropped.
    let zeros = "0".repeat(10_000);
    assert_eq!(parse(&format!("0.{zeros}"), 2), Ok(int(0)));
    // A text of 10^6 digits is refused in linear time.
    let huge = "9".repeat(1_000_000);
    let start = std::time::Instant::now();
    assert_eq!(parse(&huge, 0), Err(ParseFailure::TooLong));
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn literals_at_a_scale() {
    let value = |t: &str, s: u8| match literal(t, s) {
        Literal::Value(c) => Some(c),
        _ => None,
    };
    assert_eq!(value("19.99", 2), Some(int(1999)));
    assert_eq!(value("5.0", 2), Some(int(500)));
    assert_eq!(value("1.5e3", 2), Some(int(150_000)));
    assert_eq!(value("1_000.5", 1), Some(int(10_005)));
    assert_eq!(value("1.10", 1), Some(int(11)));
    assert_eq!(value("0.0", 0), Some(int(0)));
    assert_eq!(value("125.0e-2", 2), Some(int(125)));
    assert_eq!(literal("19.995", 2), Literal::Inexact);
    assert_eq!(literal("0.005", 2), Literal::Inexact);
    assert_eq!(literal("1.5e-3", 2), Literal::Inexact);
    assert_eq!(literal("1.0e4300", 0), Literal::TooLong);
    assert_eq!(
        value("1.0e4299", 0).map(|c| c.to_string().len()),
        Some(4300)
    );
    assert_eq!(literal("1.0e1000000000", 2), Literal::TooLong);
    assert_eq!(literal("1.0e9999999999999999999999", 2), Literal::TooLong);
    assert_eq!(literal("1.0e-1000000000", 2), Literal::Inexact);
    assert_eq!(value("0.0e-1000000000", 2), Some(int(0)));
    assert_eq!(literal_scale("0.50"), Some(2));
    assert_eq!(literal_scale("0.005"), Some(3));
    assert_eq!(literal_scale("1.5e3"), Some(0));
    assert_eq!(literal_scale("1.5e-3"), Some(4));
    assert_eq!(literal_scale("1.0e-40"), None);
}

#[test]
fn to_f64_on_and_off_its_fast_path() {
    assert_eq!(to_f64(&int(1999), 2), 19.99);
    assert_eq!(to_f64(&int(-5), 2), -0.05);
    assert_eq!(to_f64(&int(0), 5).to_bits(), 0);
    assert_eq!(to_f64(&int(1), 0), 1.0);
    // The fast path equals the exact one wherever it applies.
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..20_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let c = i64::try_from(x >> 11).unwrap() * if x & 1 == 0 { 1 } else { -1 };
        let s = u8::try_from(x % 23).unwrap();
        let c = int(c);
        assert_eq!(
            to_f64(&c, s).to_bits(),
            to_f64_exact(&c, s).to_bits(),
            "{c} {s}"
        );
    }
    // Off the fast path: a coefficient of 2^60 and a large scale.
    let big = BigInt::from(1u64 << 60) + 1;
    assert_eq!(
        to_f64(&big, 30),
        "1152921504606846977e-30".parse::<f64>().unwrap()
    );
    let huge = pow10(400);
    assert_eq!(to_f64(&huge, 0), f64::INFINITY);
    assert_eq!(to_f64(&(-huge), 0), f64::NEG_INFINITY);
}

#[test]
fn from_f64_is_the_exact_binary_value_rounded_once() {
    let at = |x: f64, s: u8, mode: Rounding| from_f64_bits(x.to_bits(), s, mode);
    // 0.1 is 0.1000000000000000055511151231257827021181583404541015625.
    assert_eq!(at(0.1, 2, HalfEven), Some(int(10)));
    assert_eq!(
        at(0.1, 20, TowardZero),
        Some(BigInt::from(10_000_000_000_000_000_555_u64))
    );
    assert_eq!(
        at(0.1, 20, AwayFromZero),
        Some(BigInt::from(10_000_000_000_000_000_556_u64))
    );
    assert_eq!(
        at(0.1, 55, HalfEven),
        Some(
            BigInt::parse_bytes(
                b"1000000000000000055511151231257827021181583404541015625",
                10
            )
            .unwrap()
        )
    );
    assert_eq!(at(2.5, 0, HalfEven), Some(int(2)));
    assert_eq!(at(-2.5, 0, HalfAwayFromZero), Some(int(-3)));
    assert_eq!(at(-0.0, 2, HalfEven), Some(int(0)));
    assert_eq!(at(1.0e20, 0, HalfEven), Some(pow10(20)));
    assert_eq!(at(f64::NAN, 2, HalfEven), None);
    assert_eq!(at(f64::INFINITY, 2, HalfEven), None);
    assert_eq!(at(f64::NEG_INFINITY, 2, HalfEven), None);
    // The smallest subnormal at scale 2 is 0.00 or 0.01.
    assert_eq!(at(5.0e-324, 2, HalfEven), Some(int(0)));
    assert_eq!(at(5.0e-324, 2, AwayFromZero), Some(int(1)));
}

#[test]
fn json_numbers_at_a_scale() {
    assert_eq!(from_json_number("1.5e3", 2), Some(int(150_000)));
    assert_eq!(from_json_number("-0.25", 2), Some(int(-25)));
    assert_eq!(from_json_number("1E+2", 0), Some(int(100)));
    assert_eq!(from_json_number("0.125", 2), None);
    assert_eq!(from_json_number("1e1000000000", 2), None);
    assert_eq!(from_json_number("-0.0", 2), Some(int(0)));
}

#[test]
fn every_operation_has_a_code_that_reads_back() {
    use crate::DecimalOp;
    let ops = [
        DecimalOp::Add(2, 38),
        DecimalOp::Sub(0, 4),
        DecimalOp::Mul,
        DecimalOp::Neg,
        DecimalOp::Eq(1, 2),
        DecimalOp::Ne(3, 4),
        DecimalOp::Lt(5, 6),
        DecimalOp::Le(7, 8),
        DecimalOp::Gt(9, 10),
        DecimalOp::Ge(11, 12),
        DecimalOp::Widen(2, 4),
        DecimalOp::Rescale(4, 2),
        DecimalOp::RescaleExact(6, 0),
        DecimalOp::Divide(2, 2, 38),
        DecimalOp::Remainder(38, 0, 1),
        DecimalOp::ToInt(3),
        DecimalOp::Abs,
        DecimalOp::Coefficient,
        DecimalOp::FromInt,
        DecimalOp::FromF64(2),
        DecimalOp::ToF64(2),
        DecimalOp::ToText(9),
        DecimalOp::Parse(2),
        DecimalOp::FromJson(3),
        DecimalOp::ToJson(0),
    ];
    let mut codes = std::collections::BTreeSet::new();
    for op in ops {
        let (index, packed) = op.code();
        assert!(codes.insert(index), "{op:?}");
        assert_eq!(DecimalOp::from_code(index, packed), Some(op));
    }
    assert_eq!(DecimalOp::from_code(25, 0), None);
}

#[test]
fn rendering_digits_matches_rendering_numbers() {
    for c in [-12_345i64, -5, -1, 0, 1, 5, 99, 100, 12_345] {
        for s in [0u8, 1, 2, 5] {
            assert_eq!(
                render_digits(&c.to_string(), s),
                Some(render(&int(c), s)),
                "{c} {s}"
            );
        }
    }
    assert_eq!(render_digits("-0", 2).as_deref(), Some("0.00"));
    assert_eq!(render_digits("x", 2), None);
    assert_eq!(render_digits("", 2), None);
}

#[test]
fn the_first_failing_position_is_reported() {
    // The nonzero digit beyond the scale comes before the invalid character.
    assert_eq!(parse("1.234x", 2), Err(ParseFailure::NotADigit(4)));
    assert_eq!(parse("1.200x", 2), Err(ParseFailure::NotADigit(5)));
}

/// The least of three timings of `run`, in seconds.
fn best(mut run: impl FnMut()) -> f64 {
    (0..3)
        .map(|_| {
            let start = std::time::Instant::now();
            run();
            start.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn refusing_a_text_past_the_cap_is_linear_in_its_digits() {
    // ADR 0120, Costs: a text past the digit cap is refused in O(b), before any conversion.
    // From 10^4 to 8 × 10^4 to 6.4 × 10^5 digits, each eight times the last, the time grows
    // about eightfold, against 64 for a quadratic step; the bound, 24, leaves a margin, and a
    // floor of a millisecond keeps the clock's noise out of the ratio.
    let sizes = [10_000usize, 80_000, 640_000];
    let times: Vec<f64> = sizes
        .iter()
        .map(|&n| {
            let text = "9".repeat(n);
            best(|| assert_eq!(parse(&text, 2), Err(ParseFailure::TooLong)))
        })
        .collect();
    for pair in times.windows(2) {
        let (small, large) = (pair[0].max(1e-3), pair[1].max(1e-3));
        assert!(large / small < 24.0, "{sizes:?} took {times:?}");
    }
}

#[test]
fn parsing_under_the_cap_is_at_most_quadratic_in_its_digits() {
    // ADR 0120 rule 16: under the cap, the conversion to a binary coefficient is O(d²), so from
    // 500 to 4,000 digits, eight times as many, the time grows at most 64-fold; the bound, 192,
    // leaves a margin, and texts of 10 and 100 digits parse exactly.
    for digits in [10usize, 100, 1_000, 4_298] {
        let text = format!("{}.99", "9".repeat(digits));
        let parsed = parse(&text, 2).unwrap();
        assert_eq!(parsed.to_string().len(), digits + 2);
    }
    let time = |n: usize| {
        let text = "7".repeat(n);
        best(|| {
            parse(&text, 0).unwrap();
        })
    };
    let (small, large) = (time(500).max(1e-4), time(4_000).max(1e-4));
    assert!(large / small < 192.0, "{small} {large}");
}

#[test]
fn the_powers_of_ten_are_exact() {
    // #1090: each entry is 10^s itself, read back exactly as an integer, and the nearest binary64
    // value to the integer 10^s.
    for (s, &entry) in POW10.iter().enumerate() {
        let exact = BigInt::from(10u8).pow(u32::try_from(s).unwrap());
        assert_eq!(
            crate::float::to_int_exact(entry),
            Some(exact.clone()),
            "10^{s}"
        );
        assert_eq!(
            crate::float::from_int(&exact).to_bits(),
            entry.to_bits(),
            "10^{s}"
        );
    }
}
