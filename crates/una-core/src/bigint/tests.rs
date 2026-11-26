//! Tests of the big integers (#1093): known values, a slow reference on random operands from
//! a fixed seed, the identities of division, shifts and text, edge cases, and scaling tests
//! on the deterministic count of limb steps, which a quadratic algorithm above its threshold
//! would fail.

use super::nat::{self, BURNIKEL_ZIEGLER, KARATSUBA, TOOM3};
use super::*;

/// A xorshift generator with a fixed seed: the same operands on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }

    /// `n` limbs in one of several shapes: random, all ones, sparse, a power of two.
    fn limbs(&mut self, n: usize) -> Vec<u64> {
        let shape = self.below(6);
        (0..n)
            .map(|i| match shape {
                0 => u64::MAX,
                1 => u64::from(i + 1 == n),
                2 if self.below(8) != 0 => 0,
                3 => {
                    if self.below(2) == 0 {
                        u64::MAX
                    } else {
                        0
                    }
                }
                _ => self.next(),
            })
            .collect()
    }

    fn int(&mut self, n: usize) -> BigInt {
        let negative = self.below(2) == 0;
        BigInt::signed(negative, self.limbs(n))
    }

    /// A size near a threshold or anywhere below `max`.
    fn size(&mut self, max: usize) -> usize {
        let near = |r: &mut Rng, t: usize| t - 3 + r.below(7) as usize;
        match self.below(6) {
            0 => self.below(4) as usize,
            1 => near(self, KARATSUBA),
            2 => near(self, BURNIKEL_ZIEGLER),
            3 => near(self, TOOM3),
            _ => self.below(max as u64) as usize,
        }
    }
}

// The slow reference: textbook algorithms on limbs, written independently of `nat`.

fn ref_trim(mut v: Vec<u64>) -> Vec<u64> {
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

fn ref_cmp(a: &[u64], b: &[u64]) -> Ordering {
    let (a, b) = (ref_trim(a.to_vec()), ref_trim(b.to_vec()));
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn ref_add(a: &[u64], b: &[u64]) -> Vec<u64> {
    let n = a.len().max(b.len());
    let mut out = Vec::new();
    let mut carry = 0u128;
    for i in 0..n {
        let t = u128::from(*a.get(i).unwrap_or(&0)) + u128::from(*b.get(i).unwrap_or(&0)) + carry;
        out.push(t as u64);
        carry = t >> 64;
    }
    out.push(carry as u64);
    ref_trim(out)
}

fn ref_sub(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut borrow = 0i128;
    for (i, &x) in a.iter().enumerate() {
        let mut t = i128::from(x) - i128::from(*b.get(i).unwrap_or(&0)) - borrow;
        borrow = 0;
        if t < 0 {
            t += 1 << 64;
            borrow = 1;
        }
        out.push(t as u64);
    }
    assert_eq!(borrow, 0);
    ref_trim(out)
}

fn ref_mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut out = vec![0u64; a.len() + b.len() + 1];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u128;
        for (j, &y) in b.iter().enumerate() {
            let t = u128::from(out[i + j]) + u128::from(x) * u128::from(y) + carry;
            out[i + j] = t as u64;
            carry = t >> 64;
        }
        let mut k = i + b.len();
        while carry != 0 {
            let t = u128::from(out[k]) + carry;
            out[k] = t as u64;
            carry = t >> 64;
            k += 1;
        }
    }
    ref_trim(out)
}

/// Long division one bit at a time.
fn ref_divrem(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    let bits = a.len() * 64;
    let mut q = vec![0u64; a.len()];
    let mut r: Vec<u64> = Vec::new();
    for i in (0..bits).rev() {
        r = ref_add(&r, &r);
        if (a[i / 64] >> (i % 64)) & 1 == 1 {
            r = ref_add(&r, &[1]);
        }
        if ref_cmp(&r, b) != Ordering::Less {
            r = ref_sub(&r, b);
            q[i / 64] |= 1 << (i % 64);
        }
    }
    (ref_trim(q), r)
}

/// Decimal digits by repeated division by ten.
fn ref_decimal(a: &[u64]) -> String {
    let mut rest = ref_trim(a.to_vec());
    if rest.is_empty() {
        return "0".to_owned();
    }
    let mut digits = Vec::new();
    while !rest.is_empty() {
        let mut r = 0u128;
        for limb in rest.iter_mut().rev() {
            let n = (r << 64) | u128::from(*limb);
            *limb = (n / 10) as u64;
            r = n % 10;
        }
        digits.push(b'0' + r as u8);
        rest = ref_trim(rest);
    }
    digits.reverse();
    String::from_utf8(digits).unwrap()
}

fn int(n: i128) -> BigInt {
    BigInt::from(n)
}

fn pow2(k: u64) -> BigInt {
    BigInt::from(1u8) << k
}

/// The Euclidean quotient and remainder, as both engines build `div` and `mod` (ADR 0004).
fn euclid(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    let (q, r) = (a / b, a % b);
    if r.sign() == Sign::Minus {
        if b.sign() == Sign::Plus {
            (q - 1, r + b)
        } else {
            (q + 1, r - b)
        }
    } else {
        (q, r)
    }
}

#[test]
fn known_values() {
    let factorial: BigInt = (1..=100).map(BigInt::from).product();
    assert_eq!(
        factorial.to_string(),
        "93326215443944152681699238856266700490715968264381621468592963895217599993229915608941463976156518286253697920827223758251185210916864000000000000000000000000"
    );
    assert_eq!(
        pow2(1000).to_string(),
        "10715086071862673209484250490600018105614048117055336074437503883703510511249361224931983788156958581275946729175531468251871452856923140435984577574698574803934567774824230985421074605062371141877954182153046474983581941267398767559165543946077062914571196477686542167660429831652624386837205668069376"
    );
    assert_eq!(
        (-BigInt::from(3u8).pow(200)).to_string(),
        "-265613988875874769338781322035779626829233452653394495974574961739092490901302182994384699044001"
    );
    let f60: BigInt = (1..=60).map(BigInt::from).product();
    assert_eq!(
        f60.to_str_radix(16),
        "118b5727f009f525dcfe0d58e8653c742de9d889c2efe3c5516f88700000000000000"
    );
    assert_eq!(&factorial / &f60 * &f60, factorial);
    assert_eq!(BigInt::from(-7) / BigInt::from(2), int(-3));
    assert_eq!(BigInt::from(-7) % BigInt::from(2), int(-1));
    assert_eq!(BigInt::from(7) % BigInt::from(-2), int(1));
    assert_eq!(euclid(&int(-7), &int(2)), (int(-4), int(1)));
    assert_eq!(euclid(&int(-7), &int(-2)), (int(4), int(1)));
    assert_eq!(euclid(&int(7), &int(-2)), (int(-3), int(1)));
    assert_eq!(BigInt::from(-5) >> 1, int(-3));
    assert_eq!(BigInt::from(-4) >> 1, int(-2));
    assert_eq!(BigInt::from(5) >> 1, int(2));
    assert_eq!(BigInt::from(-6) & BigInt::from(5), int(-6 & 5));
    assert_eq!(BigInt::from(-6) | BigInt::from(5), int(-6 | 5));
    assert_eq!(BigInt::from(-6) ^ BigInt::from(-5), int(-6 ^ -5));
    assert_eq!(!BigInt::from(5), int(!5));
    assert_eq!(!BigInt::from(-1), int(0));
    assert_eq!(BigInt::from(0).pow(0), int(1));
    assert_eq!(BigInt::from(-2).pow(3), int(-8));
    assert_eq!(BigInt::from(-2).pow(64), pow2(64));
}

#[test]
fn zero_has_one_representation() {
    for zero in [
        BigInt::from(0),
        "-0".parse::<BigInt>().unwrap(),
        BigInt::from(5) - BigInt::from(5),
        -BigInt::from(0),
        BigInt::from(-3) * BigInt::from(0),
        BigInt::from(-3) % BigInt::from(3),
        BigInt::from(-1) >> 0u8 & BigInt::from(0),
        BigInt::from_biguint(Sign::Minus, BigUint::ZERO),
        BigInt::from_biguint(Sign::NoSign, BigUint::from(7u8)),
        pow2(200) - pow2(200),
    ] {
        assert_eq!(zero.sign(), Sign::NoSign);
        assert_eq!(zero, BigInt::ZERO);
        assert_eq!(zero.to_string(), "0");
        assert_eq!(zero.to_bytes_le(), (Sign::NoSign, vec![0]));
        assert_eq!(zero.iter_u64_digits().count(), 0);
        assert_eq!(zero.bits(), 0);
        assert_eq!(zero.trailing_zeros(), None);
    }
}

#[test]
fn limb_boundaries_and_fixed_widths() {
    let max = BigInt::from(u64::MAX);
    assert_eq!((&max + 1u8).iter_u64_digits().collect::<Vec<u64>>(), [0, 1]);
    assert_eq!(((&max + 1) - 1u8), max);
    assert_eq!(pow2(64).bits(), 65);
    assert_eq!(pow2(64).trailing_zeros(), Some(64));
    assert_eq!(
        pow2(128).to_string(),
        "340282366920938463463374607431768211456"
    );
    assert_eq!(i64::try_from(&int(i128::from(i64::MIN))), Ok(i64::MIN));
    assert_eq!(
        i64::try_from(&int(i128::from(i64::MIN) - 1)),
        Err(TryFromBigIntError)
    );
    assert_eq!(
        i64::try_from(&int(i128::from(i64::MAX) + 1)),
        Err(TryFromBigIntError)
    );
    assert_eq!(u64::try_from(&int(-1)), Err(TryFromBigIntError));
    assert_eq!(u64::try_from(&max), Ok(u64::MAX));
    assert_eq!(i128::try_from(&int(i128::MIN)), Ok(i128::MIN));
    assert_eq!(
        i128::try_from(&(int(i128::MIN) - 1)),
        Err(TryFromBigIntError)
    );
    assert_eq!(u128::try_from(&BigInt::from(u128::MAX)), Ok(u128::MAX));
    assert_eq!(
        u128::try_from(&(BigInt::from(u128::MAX) + 1)),
        Err(TryFromBigIntError)
    );
    assert_eq!(usize::try_from(BigInt::from(7)), Ok(7));
    assert_eq!(
        u8::try_from(&BigUint::from(256u32)),
        Err(TryFromBigIntError)
    );
    assert_eq!(u64::try_from(&BigUint::from(u64::MAX)), Ok(u64::MAX));
    for v in [i128::MIN, i128::MIN + 1, -1, 0, 1, i128::MAX] {
        assert_eq!(i128::try_from(BigInt::from(v)), Ok(v));
        assert_eq!(BigInt::from(v).to_string(), v.to_string());
    }
    assert_eq!(
        int(-258).to_bytes_le(),
        (Sign::Minus, vec![2, 1]),
        "the hash encoding's bytes"
    );
    assert_eq!(
        pow2(64).to_bytes_le(),
        (Sign::Plus, vec![0, 0, 0, 0, 0, 0, 0, 0, 1])
    );
}

#[test]
fn text_grammar_is_that_of_num_bigint() {
    let ok = |t: &str, v: i128| assert_eq!(t.parse::<BigInt>(), Ok(int(v)), "{t:?}");
    let invalid = ParseBigIntError { empty: false };
    let empty = ParseBigIntError { empty: true };
    ok("0", 0);
    ok("-0", 0);
    ok("+0", 0);
    ok("+12", 12);
    ok("-12", -12);
    ok("00012", 12);
    ok("1_000", 1000);
    ok("1_", 1);
    ok("1__2", 12);
    ok("-1_0", -10);
    for t in [
        "_1", "-_1", "+_1", "+-1", "-+1", "--1", "++1", "1a", " 1", "1 ", "_", "0x10",
    ] {
        assert_eq!(t.parse::<BigInt>(), Err(invalid.clone()), "{t:?}");
    }
    for t in ["", "-", "+"] {
        assert_eq!(t.parse::<BigInt>(), Err(empty.clone()), "{t:?}");
    }
    assert_eq!("-1".parse::<BigUint>(), Err(invalid.clone()));
    assert_eq!(BigInt::parse_bytes(b"ff", 16), Some(int(255)));
    assert_eq!(BigInt::parse_bytes(b"FF", 16), Some(int(255)));
    assert_eq!(BigInt::parse_bytes(b"zz", 36), Some(int(36 * 36 - 1)));
    assert_eq!(BigInt::parse_bytes(b"12", 2), None);
    assert_eq!(BigInt::parse_bytes(&[0xff], 10), None);
    assert_eq!(BigInt::from_str_radix("1", 37), Err(invalid.clone()));
    assert_eq!(BigInt::from_str_radix("1", 1), Err(invalid));
    assert_eq!(empty.to_string(), "cannot parse integer from empty string");
    // Formatting honors the flags as integers do.
    assert_eq!(
        format!(
            "{:>6}|{:+}|{:<4}|{:05}",
            int(-42),
            int(42),
            int(7),
            int(-42)
        ),
        "   -42|+42|7   |-0042"
    );
    assert_eq!(format!("{:?}", int(-42)), "-42");
}

#[test]
fn arithmetic_agrees_with_the_slow_reference() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..400 {
        let (n, m) = (rng.size(320), rng.size(320));
        let (a, b) = (rng.limbs(n), rng.limbs(m));
        let (x, y) = (
            BigUint::from_limbs(a.clone()),
            BigUint::from_limbs(b.clone()),
        );
        assert_eq!((&x + &y).limbs(), ref_add(&a, &b));
        assert_eq!((&x * &y).limbs(), ref_mul(&a, &b));
        assert_eq!((&x * &x).limbs(), ref_mul(&a, &a));
        assert_eq!(x.cmp(&y), ref_cmp(&a, &b));
        if ref_cmp(&a, &b) != Ordering::Less {
            assert_eq!((&x - &y).limbs(), ref_sub(&ref_trim(a.clone()), &b));
        }
    }
}

#[test]
fn division_agrees_with_the_slow_reference() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for round in 0..60 {
        // Every few rounds, a divisor and a quotient past the recursive division's threshold.
        let (m, n) = if round % 3 == 0 {
            (
                BURNIKEL_ZIEGLER + rng.below(80) as usize,
                BURNIKEL_ZIEGLER + rng.below(80) as usize,
            )
        } else {
            (rng.size(90), rng.size(90))
        };
        let b = rng.limbs(m.max(1));
        if ref_trim(b.clone()).is_empty() {
            continue;
        }
        let a = rng.limbs(m + n);
        let (q, r) = nat::divrem(&a, &b).unwrap();
        assert_eq!(
            (q, r),
            ref_divrem(&a, &b),
            "{} by {} limbs",
            a.len(),
            b.len()
        );
    }
}

#[test]
fn division_identity_with_the_signs_of_una() {
    // The top limb of each operand is forced nonzero, so every length is exactly the one
    // named, whatever shape the generator draws; all-ones operands are checked too.
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let exact = |rng: &mut Rng, n: usize| {
        let mut limbs = rng.limbs(n);
        if let Some(top) = limbs.last_mut() {
            *top |= 1 << 63;
        }
        limbs
    };
    for &b in &[TOOM3 * 3 / 2, TOOM3 * 3 / 2 + 1, 500, 640] {
        let k = b.div_ceil(3);
        for a in [
            9 * b / 16,
            9 * b / 16 + 1,
            2 * k - 1,
            2 * k,
            2 * k + 1,
            b / 2 + 1,
        ] {
            let ones = (vec![u64::MAX; a], vec![u64::MAX; b]);
            for (x, y) in [(exact(&mut rng, a), exact(&mut rng, b)), ones] {
                assert_eq!((x.len(), y.len()), (a, b));
                let (p, q) = (
                    BigUint::from_limbs(x.clone()),
                    BigUint::from_limbs(y.clone()),
                );
                assert_eq!((&p * &q).limbs(), ref_mul(&x, &y), "{a} by {b} limbs");
                assert_eq!((&q * &p).limbs(), ref_mul(&x, &y), "{b} by {a} limbs");
            }
        }
    }
}

#[test]
fn shifts_and_bits() {
    let mut rng = Rng(0x94D0_49BB_1331_11EB);
    for _ in 0..300 {
        let n = rng.size(100);
        let a = rng.int(n);
        let k = rng.below(400);
        assert_eq!(&(&a << k) >> k, a);
        assert_eq!(&a << k, &a * pow2(k));
        // `>>` is the floor of the quotient by 2^k.
        assert_eq!(&a >> k, euclid(&a, &pow2(k)).0);
        assert_eq!(&a >> -i64::try_from(k).unwrap(), &a << k);
        assert_eq!(!!a.clone(), a);
        let m = rng.size(100);
        let b = rng.int(m);
        assert_eq!(&a ^ &b ^ &b, a);
        assert_eq!((&a & &b) + (&a | &b), &a + &b);
        assert_eq!(
            a.bits(),
            a.magnitude().to_str_radix(2).len() as u64 * u64::from(!a.is_zero())
        );
    }
    assert_eq!(BigInt::from(-1) >> u64::MAX, int(-1));
    assert_eq!(BigInt::from(5) >> u64::MAX, int(0));
    assert_eq!(BigInt::from(-5) << -1, int(-3));
    assert_eq!(BigUint::from(5u8) >> 300u32, BigUint::ZERO);
    assert!(pow2(130).magnitude().bit(130));
    assert!(!pow2(130).magnitude().bit(129));
    assert!(!pow2(130).magnitude().bit(u64::MAX));
}

#[test]
fn text_round_trips_in_every_radix() {
    let mut rng = Rng(0xBF58_476D_1CE4_E5B9);
    for round in 0..200 {
        let n = if round % 20 == 0 {
            200 + rng.below(400) as usize
        } else {
            rng.size(60)
        };
        let a = rng.int(n);
        let radix = 2 + (round % 35) as u32;
        let text = a.to_str_radix(radix);
        assert_eq!(
            BigInt::from_str_radix(&text, radix),
            Ok(a.clone()),
            "radix {radix}"
        );
        assert_eq!(
            BigInt::from_str_radix(&text.to_uppercase(), radix),
            Ok(a.clone())
        );
        let decimal = a.to_string();
        if n <= 40 {
            let reference = ref_decimal(a.magnitude().limbs());
            let sign = if a.sign() == Sign::Minus { "-" } else { "" };
            assert_eq!(decimal, format!("{sign}{reference}"));
        }
        assert_eq!(decimal.parse::<BigInt>(), Ok(a));
    }
    // Past the divide-and-conquer thresholds both ways.
    let big: BigInt = BigInt::from(7u8).pow(40_000) - 1;
    let text = big.to_string();
    assert_eq!(text.len(), 33_804);
    assert!(text.starts_with("834834849946771739398189689082"));
    assert!(
        text.ends_with("000000"),
        "7^40000 = 2401^10000 ends in ...000001"
    );
    let digit_sum: u64 = text.bytes().map(|b| u64::from(b - b'0')).sum();
    assert_eq!(digit_sum, 151_737);
    assert_eq!(text.parse::<BigInt>(), Ok(big));
}

#[test]
fn large_operands() {
    // About 6.4 million bits: Toom-3 at depth, exact divisions both ways, and a square.
    let a = pow2(6_400_000) - 1;
    let b = BigInt::from(3u8).pow(1_000_000) + 7;
    let p = &a * &b;
    assert_eq!(&p / &b, a);
    assert_eq!(&p % &b, BigInt::ZERO);
    assert_eq!(&p % &a, BigInt::ZERO);
    assert_eq!((&a * &a), (&a << 6_400_000u64) - &a);
}

#[test]
fn products_at_the_toom3_ratio_agree_with_the_slow_reference() {
    // #1368: Toom-3 takes balanced products from 9/16 of the longer operand's limbs, so the
    // shorter operand's top third can be empty. Every ratio near the switch and near 2/3.
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for &b in &[TOOM3 * 3 / 2, TOOM3 * 3 / 2 + 1, 500, 640] {
        let k = b.div_ceil(3);
        for a in [
            9 * b / 16,
            9 * b / 16 + 1,
            2 * k - 1,
            2 * k,
            2 * k + 1,
            b / 2 + 1,
        ] {
            let (x, y) = (rng.limbs(a), rng.limbs(b));
            let (p, q) = (
                BigUint::from_limbs(x.clone()),
                BigUint::from_limbs(y.clone()),
            );
            assert_eq!((&p * &q).limbs(), ref_mul(&x, &y), "{a} by {b} limbs");
            assert_eq!((&q * &p).limbs(), ref_mul(&x, &y), "{b} by {a} limbs");
        }
    }
}

#[test]
fn two_limb_division_agrees_with_the_slow_reference() {
    // #1368: operands of at most two limbs divide in 128-bit arithmetic.
    let mut rng = Rng(0x94D0_49BB_1331_11EB);
    let edge = [0, 1, 2, u64::MAX - 1, u64::MAX];
    for round in 0..2_000 {
        let pick = |rng: &mut Rng| {
            if rng.below(3) == 0 {
                edge[rng.below(5) as usize]
            } else {
                rng.next()
            }
        };
        let a = ref_trim(vec![pick(&mut rng), pick(&mut rng)]);
        let b = ref_trim(match round % 3 {
            0 => vec![pick(&mut rng)],
            _ => vec![pick(&mut rng), pick(&mut rng)],
        });
        if b.is_empty() {
            continue;
        }
        let (q, r) = ref_divrem(&a, &b);
        let (x, y) = (
            BigUint::from_limbs(a.clone()),
            BigUint::from_limbs(b.clone()),
        );
        assert_eq!((&x / &y).limbs(), q, "{a:?} / {b:?}");
        assert_eq!((&x % &y).limbs(), r, "{a:?} % {b:?}");
        for (sa, sb) in [(false, false), (false, true), (true, false), (true, true)] {
            let (i, j) = (BigInt::signed(sa, a.clone()), BigInt::signed(sb, b.clone()));
            assert_eq!(&i / &j, BigInt::signed(sa != sb, q.clone()));
            assert_eq!(&i % &j, BigInt::signed(sa, r.clone()));
        }
    }
}

#[test]
fn text_of_values_with_zero_low_limbs_agrees_with_the_slow_reference() {
    // #1368: writing divides by radix powers without their zero low limbs; powers of ten and
    // values with zero low limbs, written divided and conquered.
    let mut rng = Rng(0xBF58_476D_1CE4_E5B9);
    for digits in [600u32, 617, 618, 1_234, 2_500, 5_000, 9_999] {
        let text = BigUint::from(10u8).pow(digits).to_string();
        assert_eq!(text.len(), digits as usize + 1);
        assert!(text.starts_with('1') && text.bytes().skip(1).all(|c| c == b'0'));
        let below = (BigUint::from(10u8).pow(digits) - BigUint::from(1u8)).to_string();
        assert!(below.len() == digits as usize && below.bytes().all(|c| c == b'9'));
    }
    for round in 0..24 {
        let n = 32 + rng.below(200) as usize;
        let mut limbs = rng.limbs(n);
        for limb in limbs.iter_mut().take(rng.below(n as u64 / 2) as usize) {
            *limb = 0;
        }
        let x = BigUint::from_limbs(limbs.clone());
        assert_eq!(x.to_string(), ref_decimal(&limbs), "round {round}");
        for radix in [6, 12, 36] {
            let text = x.to_str_radix(radix);
            assert_eq!(BigUint::from_str_radix(&text, radix), Ok(x.clone()));
        }
    }
}

#[test]
fn text_reads_agree_at_the_divide_threshold() {
    // #1368: reading divides and conquers from 768 chunks of 19 digits (14,592 digits).
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for digits in [14_591usize, 14_592, 14_593, 14_611, 20_000] {
        let text: String = (0..digits)
            .map(|i| char::from(b'0' + (rng.below(10) as u8).max(u8::from(i == 0))))
            .collect();
        let x: BigUint = text.parse().unwrap();
        assert_eq!(x.to_string(), text);
        // Horner's rule one digit at a time, the reference.
        let mut reference = BigUint::ZERO;
        for c in text.bytes() {
            reference = reference * BigUint::from(10u8) + BigUint::from(c - b'0');
        }
        assert_eq!(x, reference);
    }
}

#[test]
fn no_operation_panics_at_its_edges() {
    let a = int(12345);
    assert_eq!(&a / 0, BigInt::ZERO);
    assert_eq!(&a % 0, a);
    assert_eq!(BigUint::from(3u8) - BigUint::from(5u8), BigUint::ZERO);
    assert_eq!(BigUint::from(3u8) / 0u8, BigUint::ZERO);
    assert_eq!(BigUint::from(3u8) % 0u8, BigUint::from(3u8));
    assert_eq!(a.to_str_radix(1), a.to_str_radix(2));
    assert_eq!(a.to_str_radix(99), a.to_str_radix(36));
    assert_eq!(BigInt::ZERO.pow(5), BigInt::ZERO);
    assert_eq!(BigInt::from(1u8).pow(u32::MAX), int(1));
    assert_eq!(BigInt::from(-1).pow(u32::MAX), int(-1));
    assert_eq!(nat::divrem(&[1, 2], &[]), None);
}

/// The limb steps of `f`, counted on this thread.
fn steps(f: impl FnOnce()) -> u64 {
    nat::take_steps();
    f();
    nat::take_steps()
}

/// Asserts that the steps grow by less than `bound` each time the size doubles.
fn assert_doubling(name: &str, sizes: &[usize], bound: f64, mut f: impl FnMut(usize) -> u64) {
    let counts: Vec<u64> = sizes.iter().map(|&n| f(n)).collect();
    for (w, n) in counts.windows(2).zip(sizes.windows(2)) {
        let ratio = w[1] as f64 / w[0] as f64;
        assert!(
            ratio < bound,
            "{name}: {} to {} limbs grew {ratio:.2}x (bound {bound}): {counts:?}",
            n[0],
            n[1]
        );
    }
}

#[test]
fn multiplication_is_subquadratic() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let sizes = [512, 1024, 2048, 4096, 8192];
    let operands: Vec<(Vec<u64>, Vec<u64>)> = sizes
        .iter()
        .map(|&n| {
            let a: Vec<u64> = (0..n).map(|_| rng.next() | 1).collect();
            let b: Vec<u64> = (0..n).map(|_| rng.next() | 1).collect();
            (a, b)
        })
        .collect();
    // Schoolbook would grow 4x per doubling; Karatsuba grows 3x, Toom-3 about 2.8x.
    let mut i = 0;
    assert_doubling("multiplication", &sizes, 3.3, |_| {
        let (a, b) = &operands[i];
        i += 1;
        steps(|| {
            nat::mul(a, b);
        })
    });
    let mut i = 0;
    assert_doubling("squaring", &sizes, 3.3, |_| {
        let (a, _) = &operands[i];
        i += 1;
        steps(|| {
            nat::mul(a, a);
        })
    });
    // An unbalanced product grows linearly in the longer operand.
    assert_doubling("unbalanced", &[4096, 8192, 16384], 2.2, |n| {
        let a: Vec<u64> = vec![u64::MAX; 200];
        let b: Vec<u64> = vec![u64::MAX; n];
        steps(|| {
            nat::mul(&a, &b);
        })
    });
}

#[test]
fn division_is_subquadratic() {
    let mut rng = Rng(0x0F1E_2D3C_4B5A_6978);
    // A 2n by n division: Knuth's algorithm D would grow 4x per doubling, Burnikel-Ziegler
    // grows as M(n) log n, about 3.3x here.
    assert_doubling("division", &[512, 1024, 2048, 4096], 3.6, |n| {
        let a: Vec<u64> = (0..2 * n).map(|_| rng.next()).collect();
        let b: Vec<u64> = (0..n).map(|_| rng.next() | 1 << 63).collect();
        steps(|| {
            nat::divrem(&a, &b);
        })
    });
}

#[test]
fn text_conversion_is_subquadratic() {
    let mut rng = Rng(0x7777_1111_3333_5555);
    assert_doubling("to text", &[1024, 2048, 4096, 8192], 3.6, |n| {
        let a: Vec<u64> = (0..n).map(|_| rng.next()).collect();
        steps(|| {
            radix::to_text(&a, 10);
        })
    });
    assert_doubling("from text", &[20_000, 40_000, 80_000, 160_000], 3.6, |n| {
        let digits: Vec<u8> = (0..n).map(|_| (rng.next() % 10) as u8).collect();
        steps(|| {
            radix::from_digits(&digits, 10);
        })
    });
}

#[test]
fn writing_an_int_past_the_bound_of_parsing_is_subquadratic() {
    // #1223: an `Int` of any size is written as text (`spec/11 §7`), through `Display`, as
    // both engines write it. From 256 limbs, about 4,900 digits, just past the 4,300 that
    // parsing reads, to 16,384 limbs, about 315,000 digits: repeated division would grow 4x per
    // doubling, the divide and conquer grows as M(n) log n, about 3.3x here.
    let mut rng = Rng(0x2468_ACE0_1357_9BDF);
    let sizes = [256, 512, 1024, 2048, 4096, 8192, 16384];
    let values: Vec<BigInt> = sizes
        .iter()
        .map(|&n| {
            let limbs: Vec<u64> = (0..n).map(|_| rng.next()).collect();
            BigInt::from_biguint(Sign::Minus, BigUint::from_limbs(limbs))
        })
        .collect();
    let mut i = 0;
    assert_doubling("writing", &sizes, 3.6, |n| {
        let value = &values[i];
        i += 1;
        let mut text = String::new();
        let counted = steps(|| text = value.to_string());
        // The sign and every digit: 19.27 digits per limb, the top limb nearly full.
        let digits = text.len() - 1;
        assert!(text.starts_with('-'), "{n} limbs");
        assert!(
            digits as f64 > (n as f64 - 1.0) * 19.26 && digits as f64 <= n as f64 * 19.27,
            "{n} limbs: {digits} digits"
        );
        counted
    });
}
