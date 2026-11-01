//! Una's own arbitrary-precision integers (#1093): the values of `Int` (ADR 0004), of the
//! coefficients of `Decimal` and of the exact steps of floating-point conversions, in both
//! engines. Safe Rust with no dependency, deterministic, and no panic on any input.
//!
//! [`BigUint`] is a natural number in little-endian 64-bit limbs without high zero limbs, one
//! limb stored inline, so every value has one representation and zero has no limbs.
//! [`BigInt`] is a sign and a magnitude; zero has [`Sign::NoSign`] only, so there is no
//! negative zero.
//!
//! The interface is the subset of `num-bigint` 0.4 that the workspace used, with the same
//! results bit for bit: truncating `/` and `%` (the engines build Euclidean `div` and `mod` on
//! them), `>>` rounding toward negative infinity, two's-complement `&`, `|`, `^` and `!`, the
//! text grammar of `from_str_radix` (an optional sign, digits and `_` separators), and
//! `to_bytes_le`, which the hash encoding reads (`spec/28 §1` rule 2). Where `num-bigint`
//! panicked, on a zero divisor, a natural subtraction below zero or a negative shift, these
//! types return a value instead, documented at each operator: no caller reaches those cases.
//!
//! Complexity, n the limbs of the larger operand and M(n) the cost of a product: addition,
//! subtraction, comparison, shifts and bit operations O(n); multiplication schoolbook O(n²)
//! below 40 limbs, Karatsuba O(n^1.585), Toom-3 O(n^1.465) from 256 limbs; division Knuth's
//! algorithm D O(m·n) for a divisor or a quotient below 64 limbs, Burnikel-Ziegler
//! O(M(n) log n) above; `pow` by squaring O(M(size of the result)); text in a radix by
//! divide and conquer O(M(n) log n), O(n) for a power-of-two radix. Operands of one limb take
//! a 128-bit path and never allocate. Every allocation goes through the global allocator, so
//! the memory limit of ADR 0061 counts it, and one the machine cannot make stops the process
//! as any allocation does (ADR 0061 rule 3).

mod nat;
mod radix;
#[cfg(test)]
mod tests;

use std::cmp::Ordering;
use std::fmt;
use std::iter::{Product, Sum};
use std::ops::{
    Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Div,
    DivAssign, Mul, MulAssign, Neg, Not, Rem, RemAssign, Shl, ShlAssign, Shr, ShrAssign, Sub,
    SubAssign,
};
use std::str::FromStr;

/// The sign of a [`BigInt`]: `NoSign` exactly for zero. Ordered `Minus < NoSign < Plus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sign {
    /// Below zero.
    Minus,
    /// Zero.
    NoSign,
    /// Above zero.
    Plus,
}

/// A natural number of any size.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct BigUint {
    digits: Digits,
}

/// The limbs of a [`BigUint`]: a value below 2^64 inline, so the common small integer never
/// allocates, else at least two little-endian limbs with a nonzero top one. Each value has
/// exactly one representation, so the derived equality and hash are those of the value.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Digits {
    Inline(u64),
    Heap(Vec<u64>),
}

impl Default for Digits {
    fn default() -> Self {
        Digits::Inline(0)
    }
}

/// An integer of any size: a sign and a magnitude, zero never negative.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BigInt {
    sign: Sign,
    magnitude: BigUint,
}

/// A text that is not an integer in the radix asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseBigIntError {
    empty: bool,
}

impl fmt::Display for ParseBigIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.empty {
            "cannot parse integer from empty string"
        } else {
            "invalid digit found in string"
        })
    }
}

impl std::error::Error for ParseBigIntError {}

/// A conversion to a fixed-width integer of a value outside its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TryFromBigIntError;

impl fmt::Display for TryFromBigIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("out of range conversion regarding big integer attempted")
    }
}

impl std::error::Error for TryFromBigIntError {}

impl BigUint {
    /// Zero.
    pub const ZERO: BigUint = BigUint {
        digits: Digits::Inline(0),
    };

    fn from_limbs(limbs: Vec<u64>) -> Self {
        let limbs = nat::normalized(limbs);
        let digits = match limbs.as_slice() {
            [] => Digits::Inline(0),
            [x] => Digits::Inline(*x),
            _ => Digits::Heap(limbs),
        };
        BigUint { digits }
    }

    /// The little-endian limbs, with no high zero limb: none for zero.
    fn limbs(&self) -> &[u64] {
        match &self.digits {
            Digits::Inline(0) => &[],
            Digits::Inline(x) => std::slice::from_ref(x),
            Digits::Heap(v) => v,
        }
    }

    /// Whether the value is zero.
    pub fn is_zero(&self) -> bool {
        self.limbs().is_empty()
    }

    /// The number of significant bits, 0 for zero. O(1).
    pub fn bits(&self) -> u64 {
        nat::bits(&self.limbs())
    }

    /// Whether bit `i` is set, bit 0 the least significant. O(1).
    pub fn bit(&self, i: u64) -> bool {
        let limb = usize::try_from(i / 64).unwrap_or(usize::MAX);
        self.limbs()
            .get(limb)
            .is_some_and(|l| (l >> (i % 64)) & 1 == 1)
    }

    /// The number of zero bits below the lowest set bit, `None` for zero. O(n).
    pub fn trailing_zeros(&self) -> Option<u64> {
        let (i, limb) = self.limbs().iter().enumerate().find(|(_, l)| **l != 0)?;
        Some(i as u64 * 64 + u64::from(limb.trailing_zeros()))
    }

    /// `self^e`, 1 for `e = 0`, by squaring from the top bit of `e`: O(M(n·e)), the last
    /// squaring dominating; a power of two is a shift.
    pub fn pow(&self, e: u32) -> BigUint {
        if e == 0 {
            return BigUint::from(1u8);
        }
        if let Some(z) = self.trailing_zeros()
            && self.bits() == z + 1
        {
            return BigUint::from_limbs(nat::shl(&[1], z.saturating_mul(u64::from(e))));
        }
        let mut out = self.limbs().to_vec();
        for i in (0..31 - e.leading_zeros()).rev() {
            out = nat::mul(&out, &out);
            if (e >> i) & 1 == 1 {
                out = nat::mul(&out, &self.limbs());
            }
        }
        BigUint::from_limbs(out)
    }

    /// The digits in `radix`, lowercase, most significant first; the radix is clamped to 2 to
    /// 36. O(M(n) log n), O(n) for a power of two.
    pub fn to_str_radix(&self, radix: u32) -> String {
        radix::to_text(&self.limbs(), radix)
    }

    /// The natural number of `s` in `radix` (2 to 36): an optional `+`, then digits, with `_`
    /// allowed between and after them but not first. O(M(n) log n).
    pub fn from_str_radix(s: &str, radix: u32) -> Result<BigUint, ParseBigIntError> {
        if !(2..=36).contains(&radix) {
            return Err(ParseBigIntError { empty: false });
        }
        let mut s = s;
        if let Some(tail) = s.strip_prefix('+')
            && !tail.starts_with('+')
        {
            s = tail;
        }
        if s.is_empty() {
            return Err(ParseBigIntError { empty: true });
        }
        if s.starts_with('_') {
            return Err(ParseBigIntError { empty: false });
        }
        let mut digits = Vec::with_capacity(s.len());
        for b in s.bytes() {
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'z' => b - b'a' + 10,
                b'A'..=b'Z' => b - b'A' + 10,
                b'_' => continue,
                _ => u8::MAX,
            };
            if u32::from(d) >= radix {
                return Err(ParseBigIntError { empty: false });
            }
            digits.push(d);
        }
        Ok(BigUint::from_limbs(radix::from_digits(&digits, radix)))
    }

    /// [`BigUint::from_str_radix`] of the UTF-8 text `buf`, `None` when it is not one.
    pub fn parse_bytes(buf: &[u8], radix: u32) -> Option<BigUint> {
        let s = std::str::from_utf8(buf).ok()?;
        BigUint::from_str_radix(s, radix).ok()
    }

    /// The bytes of the value, least significant first, without high zero bytes; `[0]` for
    /// zero.
    pub fn to_bytes_le(&self) -> Vec<u8> {
        if self.is_zero() {
            return vec![0];
        }
        let mut out: Vec<u8> = self.limbs().iter().flat_map(|l| l.to_le_bytes()).collect();
        while out.last() == Some(&0) {
            out.pop();
        }
        out
    }

    /// The 64-bit limbs of the value, least significant first, none for zero.
    pub fn iter_u64_digits(&self) -> std::iter::Copied<std::slice::Iter<'_, u64>> {
        self.limbs().iter().copied()
    }
}

impl BigInt {
    /// Zero.
    pub const ZERO: BigInt = BigInt {
        sign: Sign::NoSign,
        magnitude: BigUint::ZERO,
    };

    /// The integer of `sign` and `magnitude`: zero when either is zero.
    pub fn from_biguint(sign: Sign, magnitude: BigUint) -> BigInt {
        if sign == Sign::NoSign || magnitude.is_zero() {
            return BigInt::ZERO;
        }
        BigInt { sign, magnitude }
    }

    fn signed(negative: bool, limbs: Vec<u64>) -> BigInt {
        BigInt::with_sign(negative, BigUint::from_limbs(limbs))
    }

    fn with_sign(negative: bool, magnitude: BigUint) -> BigInt {
        let sign = if negative { Sign::Minus } else { Sign::Plus };
        BigInt::from_biguint(sign, magnitude)
    }

    fn negative(&self) -> bool {
        self.sign == Sign::Minus
    }

    /// The sign.
    pub fn sign(&self) -> Sign {
        self.sign
    }

    /// The magnitude, the absolute value.
    pub fn magnitude(&self) -> &BigUint {
        &self.magnitude
    }

    /// The sign and the magnitude.
    pub fn into_parts(self) -> (Sign, BigUint) {
        (self.sign, self.magnitude)
    }

    /// Whether the value is zero.
    pub fn is_zero(&self) -> bool {
        self.sign == Sign::NoSign
    }

    /// The number of significant bits of the magnitude, 0 for zero. O(1).
    pub fn bits(&self) -> u64 {
        self.magnitude.bits()
    }

    /// The number of zero bits below the lowest set bit, `None` for zero. O(n).
    pub fn trailing_zeros(&self) -> Option<u64> {
        self.magnitude.trailing_zeros()
    }

    /// `self^e`, 1 for `e = 0`, negative for a negative base and an odd `e`. O(M(n·e)).
    pub fn pow(&self, e: u32) -> BigInt {
        BigInt::with_sign(self.negative() && e % 2 == 1, self.magnitude.pow(e))
    }

    /// The sign and the bytes of the magnitude, least significant first, without high zero
    /// bytes; `(NoSign, [0])` for zero.
    pub fn to_bytes_le(&self) -> (Sign, Vec<u8>) {
        (self.sign, self.magnitude.to_bytes_le())
    }

    /// The 64-bit limbs of the magnitude, least significant first, none for zero.
    pub fn iter_u64_digits(&self) -> std::iter::Copied<std::slice::Iter<'_, u64>> {
        self.magnitude.iter_u64_digits()
    }

    /// The digits in `radix` with a leading `-` when negative; the radix is clamped to 2 to
    /// 36.
    pub fn to_str_radix(&self, radix: u32) -> String {
        let digits = self.magnitude.to_str_radix(radix);
        if self.negative() {
            format!("-{digits}")
        } else {
            digits
        }
    }

    /// The integer of `s` in `radix` (2 to 36): an optional `-` (not followed by `+`), then
    /// the grammar of [`BigUint::from_str_radix`]; `-0` is zero.
    pub fn from_str_radix(s: &str, radix: u32) -> Result<BigInt, ParseBigIntError> {
        let mut s = s;
        let mut negative = false;
        if let Some(tail) = s.strip_prefix('-') {
            if !tail.starts_with('+') {
                s = tail;
            }
            negative = true;
        }
        let magnitude = BigUint::from_str_radix(s, radix)?;
        Ok(BigInt::with_sign(negative, magnitude))
    }

    /// [`BigInt::from_str_radix`] of the UTF-8 text `buf`, `None` when it is not one.
    pub fn parse_bytes(buf: &[u8], radix: u32) -> Option<BigInt> {
        let s = std::str::from_utf8(buf).ok()?;
        BigInt::from_str_radix(s, radix).ok()
    }

    /// `self << n` for `n >= 0`, `self >> -n` otherwise.
    fn shift_left(&self, n: i128) -> BigInt {
        let amount = u64::try_from(n.unsigned_abs()).unwrap_or(u64::MAX);
        if n >= 0 {
            BigInt::signed(self.negative(), nat::shl(&self.magnitude.limbs(), amount))
        } else {
            self.shift_right_floor(amount)
        }
    }

    /// `self >> n` for `n >= 0`, `self << -n` otherwise.
    fn shift_right(&self, n: i128) -> BigInt {
        let amount = u64::try_from(n.unsigned_abs()).unwrap_or(u64::MAX);
        if n >= 0 {
            self.shift_right_floor(amount)
        } else {
            BigInt::signed(self.negative(), nat::shl(&self.magnitude.limbs(), amount))
        }
    }

    /// The quotient by `2^n` rounded toward negative infinity: an arithmetic shift.
    fn shift_right_floor(&self, n: u64) -> BigInt {
        let q = nat::shr(&self.magnitude.limbs(), n);
        let inexact = self.magnitude.trailing_zeros().is_some_and(|z| z < n);
        if self.negative() && inexact {
            BigInt::signed(true, nat::add(&q, &[1]))
        } else {
            BigInt::signed(self.negative(), q)
        }
    }

    /// The limbs of the two's complement of `self` in `len` limbs.
    fn twos(&self, len: usize) -> Vec<u64> {
        let mut v = if self.negative() {
            nat::sub(&self.magnitude.limbs(), &[1])
        } else {
            self.magnitude.limbs().to_vec()
        };
        v.resize(len, 0);
        if self.negative() {
            for limb in &mut v {
                *limb = !*limb;
            }
        }
        v
    }

    /// The integer of two's-complement limbs, negative when the top bit is set.
    fn from_twos(mut v: Vec<u64>) -> BigInt {
        if v.last().is_some_and(|top| top >> 63 == 1) {
            for limb in &mut v {
                *limb = !*limb;
            }
            BigInt::signed(true, nat::add(&v, &[1]))
        } else {
            BigInt::signed(false, v)
        }
    }

    /// `f` applied to each limb of the two's complements of `self` and `other`. O(n).
    fn bitwise(&self, other: &BigInt, f: impl Fn(u64, u64) -> u64) -> BigInt {
        let len = self.magnitude.limbs().len().max(other.magnitude.limbs().len()) + 1;
        let (a, b) = (self.twos(len), other.twos(len));
        BigInt::from_twos(a.iter().zip(&b).map(|(x, y)| f(*x, *y)).collect())
    }

    fn convert<T: TryFrom<i128> + TryFrom<u128>>(&self) -> Result<T, TryFromBigIntError> {
        let m = to_u128(&self.magnitude.limbs()).ok_or(TryFromBigIntError)?;
        if self.negative() {
            let value = if m == 1 << 127 {
                i128::MIN
            } else {
                i128::try_from(m).map_err(|_| TryFromBigIntError)?.wrapping_neg()
            };
            T::try_from(value).map_err(|_| TryFromBigIntError)
        } else {
            T::try_from(m).map_err(|_| TryFromBigIntError)
        }
    }
}

impl Default for BigInt {
    fn default() -> Self {
        BigInt::ZERO
    }
}

/// The value of at most one limb.
fn to_u64(limbs: &[u64]) -> Option<u64> {
    match limbs {
        [] => Some(0),
        [x] => Some(*x),
        _ => None,
    }
}

/// The value of at most two limbs.
fn to_u128(limbs: &[u64]) -> Option<u128> {
    match limbs {
        [] => Some(0),
        [lo] => Some(u128::from(*lo)),
        [lo, hi] => Some(u128::from(*lo) | (u128::from(*hi) << 64)),
        _ => None,
    }
}

impl Ord for BigUint {
    fn cmp(&self, other: &Self) -> Ordering {
        nat::cmp(&self.limbs(), &other.limbs())
    }
}

impl PartialOrd for BigUint {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        self.sign.cmp(&other.sign).then_with(|| match self.sign {
            Sign::Plus => self.magnitude.cmp(&other.magnitude),
            Sign::Minus => other.magnitude.cmp(&self.magnitude),
            Sign::NoSign => Ordering::Equal,
        })
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for BigUint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad_integral(true, "", &self.to_str_radix(10))
    }
}

impl fmt::Debug for BigUint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad_integral(!self.negative(), "", &self.magnitude.to_str_radix(10))
    }
}

impl fmt::Debug for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for BigUint {
    type Err = ParseBigIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        BigUint::from_str_radix(s, 10)
    }
}

impl FromStr for BigInt {
    type Err = ParseBigIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        BigInt::from_str_radix(s, 10)
    }
}

impl From<BigUint> for BigInt {
    fn from(magnitude: BigUint) -> Self {
        BigInt::from_biguint(Sign::Plus, magnitude)
    }
}

impl From<u128> for BigUint {
    fn from(n: u128) -> Self {
        let (lo, hi) = (n as u64, (n >> 64) as u64);
        let digits = if hi == 0 {
            Digits::Inline(lo)
        } else {
            Digits::Heap(vec![lo, hi])
        };
        BigUint { digits }
    }
}

impl From<i128> for BigInt {
    fn from(n: i128) -> Self {
        BigInt::with_sign(n < 0, BigUint::from(n.unsigned_abs()))
    }
}

impl From<u128> for BigInt {
    fn from(n: u128) -> Self {
        BigInt::from(BigUint::from(n))
    }
}

/// `From` a narrower primitive, through the 128-bit conversions.
macro_rules! from_primitive {
    ($target:ty, $wide:ty, [$($p:ty),*]) => {$(
        impl From<$p> for $target {
            fn from(n: $p) -> Self {
                <$target>::from(<$wide>::from(n))
            }
        }
    )*};
}

from_primitive!(BigUint, u128, [u8, u16, u32, u64]);
from_primitive!(BigInt, u128, [u8, u16, u32, u64]);
from_primitive!(BigInt, i128, [i8, i16, i32, i64]);

impl From<usize> for BigUint {
    fn from(n: usize) -> Self {
        BigUint::from(n as u64)
    }
}

impl From<usize> for BigInt {
    fn from(n: usize) -> Self {
        BigInt::from(n as u64)
    }
}

impl From<isize> for BigInt {
    fn from(n: isize) -> Self {
        BigInt::from(n as i64)
    }
}

/// `TryFrom` a big integer into each fixed-width primitive, by value and by reference.
macro_rules! try_into_primitive {
    ($($p:ty),*) => {$(
        impl TryFrom<&BigInt> for $p {
            type Error = TryFromBigIntError;
            fn try_from(n: &BigInt) -> Result<$p, TryFromBigIntError> {
                n.convert()
            }
        }
        impl TryFrom<BigInt> for $p {
            type Error = TryFromBigIntError;
            fn try_from(n: BigInt) -> Result<$p, TryFromBigIntError> {
                n.convert()
            }
        }
        impl TryFrom<&BigUint> for $p {
            type Error = TryFromBigIntError;
            fn try_from(n: &BigUint) -> Result<$p, TryFromBigIntError> {
                let m = to_u128(&n.limbs()).ok_or(TryFromBigIntError)?;
                <$p>::try_from(m).map_err(|_| TryFromBigIntError)
            }
        }
        impl TryFrom<BigUint> for $p {
            type Error = TryFromBigIntError;
            fn try_from(n: BigUint) -> Result<$p, TryFromBigIntError> {
                <$p>::try_from(&n)
            }
        }
    )*};
}

try_into_primitive!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);

fn add_int(a: &BigInt, b: &BigInt) -> BigInt {
    add_parts(a.negative(), &a.magnitude.limbs(), b.negative(), &b.magnitude.limbs())
}

fn sub_int(a: &BigInt, b: &BigInt) -> BigInt {
    add_parts(a.negative(), &a.magnitude.limbs(), !b.negative(), &b.magnitude.limbs())
}

/// `±a ± b`: a sum of magnitudes when the signs agree, else a difference. O(n).
fn add_parts(a_negative: bool, a: &[u64], b_negative: bool, b: &[u64]) -> BigInt {
    // Operands of at most one limb, the common case, in 128-bit arithmetic.
    if let (Some(x), Some(y)) = (to_u64(a), to_u64(b)) {
        let (x, y) = (u128::from(x), u128::from(y));
        return if a_negative == b_negative {
            BigInt::with_sign(a_negative, BigUint::from(x + y))
        } else if x >= y {
            BigInt::with_sign(a_negative, BigUint::from(x - y))
        } else {
            BigInt::with_sign(b_negative, BigUint::from(y - x))
        };
    }
    if a_negative == b_negative {
        return BigInt::signed(a_negative, nat::add(a, b));
    }
    match nat::cmp(a, b) {
        Ordering::Less => BigInt::signed(b_negative, nat::sub(b, a)),
        _ => BigInt::signed(a_negative, nat::sub(a, b)),
    }
}

fn mul_int(a: &BigInt, b: &BigInt) -> BigInt {
    if let (Some(x), Some(y)) = (to_u64(&a.magnitude.limbs()), to_u64(&b.magnitude.limbs())) {
        return BigInt::with_sign(
            a.negative() != b.negative(),
            BigUint::from(u128::from(x) * u128::from(y)),
        );
    }
    BigInt::signed(
        a.negative() != b.negative(),
        nat::mul(&a.magnitude.limbs(), &b.magnitude.limbs()),
    )
}

/// The truncating quotient and remainder, the remainder with the sign of `a`; `(0, a)` for a
/// zero `b`, so `a = q × b + r` holds there too.
fn div_rem_int(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    if let (Some(x), Some(y)) = (to_u64(&a.magnitude.limbs()), to_u64(&b.magnitude.limbs()))
        && y != 0
    {
        return (
            BigInt::with_sign(a.negative() != b.negative(), BigUint::from(u128::from(x / y))),
            BigInt::with_sign(a.negative(), BigUint::from(u128::from(x % y))),
        );
    }
    match nat::divrem(&a.magnitude.limbs(), &b.magnitude.limbs()) {
        None => (BigInt::ZERO, a.clone()),
        Some((q, r)) => (
            BigInt::signed(a.negative() != b.negative(), q),
            BigInt::signed(a.negative(), r),
        ),
    }
}

fn div_int(a: &BigInt, b: &BigInt) -> BigInt {
    div_rem_int(a, b).0
}

fn rem_int(a: &BigInt, b: &BigInt) -> BigInt {
    div_rem_int(a, b).1
}

fn and_int(a: &BigInt, b: &BigInt) -> BigInt {
    a.bitwise(b, |x, y| x & y)
}

fn or_int(a: &BigInt, b: &BigInt) -> BigInt {
    a.bitwise(b, |x, y| x | y)
}

fn xor_int(a: &BigInt, b: &BigInt) -> BigInt {
    a.bitwise(b, |x, y| x ^ y)
}

fn add_nat(a: &BigUint, b: &BigUint) -> BigUint {
    BigUint::from_limbs(nat::add(&a.limbs(), b.limbs()))
}

/// `a - b`, 0 when `b > a`.
fn sub_nat(a: &BigUint, b: &BigUint) -> BigUint {
    BigUint::from_limbs(nat::sub(&a.limbs(), b.limbs()))
}

fn mul_nat(a: &BigUint, b: &BigUint) -> BigUint {
    BigUint::from_limbs(nat::mul(&a.limbs(), b.limbs()))
}

/// The quotient, 0 for a zero divisor.
fn div_nat(a: &BigUint, b: &BigUint) -> BigUint {
    nat::divrem(&a.limbs(), b.limbs()).map_or(BigUint::ZERO, |(q, _)| BigUint::from_limbs(q))
}

/// The remainder, `a` for a zero divisor.
fn rem_nat(a: &BigUint, b: &BigUint) -> BigUint {
    nat::divrem(&a.limbs(), b.limbs()).map_or_else(|| a.clone(), |(_, r)| BigUint::from_limbs(r))
}

fn and_nat(a: &BigUint, b: &BigUint) -> BigUint {
    BigUint::from_limbs(a.limbs().iter().zip(b.limbs()).map(|(x, y)| x & y).collect())
}

fn or_nat(a: &BigUint, b: &BigUint) -> BigUint {
    let (long, short) = if a.limbs().len() >= b.limbs().len() {
        (a, b)
    } else {
        (b, a)
    };
    let mut out = long.limbs().to_vec();
    for (x, y) in out.iter_mut().zip(short.limbs()) {
        *x |= y;
    }
    BigUint::from_limbs(out)
}

fn xor_nat(a: &BigUint, b: &BigUint) -> BigUint {
    let (long, short) = if a.limbs().len() >= b.limbs().len() {
        (a, b)
    } else {
        (b, a)
    };
    let mut out = long.limbs().to_vec();
    for (x, y) in out.iter_mut().zip(short.limbs()) {
        *x ^= y;
    }
    BigUint::from_limbs(out)
}

/// A binary operator and its assignment for every pairing of owned and borrowed operands, and
/// with each primitive on the right: dozens of identical impls written once.
macro_rules! binary {
    ($T:ident, $Trait:ident, $method:ident, $Assign:ident, $assign:ident, $core:ident,
     [$($p:ty),*]) => {
        impl $Trait<&$T> for &$T {
            type Output = $T;
            fn $method(self, other: &$T) -> $T {
                $core(self, other)
            }
        }
        impl $Trait<$T> for &$T {
            type Output = $T;
            fn $method(self, other: $T) -> $T {
                $core(self, &other)
            }
        }
        impl $Trait<&$T> for $T {
            type Output = $T;
            fn $method(self, other: &$T) -> $T {
                $core(&self, other)
            }
        }
        impl $Trait<$T> for $T {
            type Output = $T;
            fn $method(self, other: $T) -> $T {
                $core(&self, &other)
            }
        }
        impl $Assign<&$T> for $T {
            fn $assign(&mut self, other: &$T) {
                *self = $core(self, other);
            }
        }
        impl $Assign<$T> for $T {
            fn $assign(&mut self, other: $T) {
                *self = $core(self, &other);
            }
        }
        $(
            impl $Trait<$p> for &$T {
                type Output = $T;
                fn $method(self, other: $p) -> $T {
                    $core(self, &$T::from(other))
                }
            }
            impl $Trait<$p> for $T {
                type Output = $T;
                fn $method(self, other: $p) -> $T {
                    $core(&self, &$T::from(other))
                }
            }
            impl $Trait<&$T> for $p {
                type Output = $T;
                fn $method(self, other: &$T) -> $T {
                    $core(&$T::from(self), other)
                }
            }
            impl $Trait<$T> for $p {
                type Output = $T;
                fn $method(self, other: $T) -> $T {
                    $core(&$T::from(self), &other)
                }
            }
            impl $Assign<$p> for $T {
                fn $assign(&mut self, other: $p) {
                    *self = $core(self, &$T::from(other));
                }
            }
        )*
    };
}

/// Every operator of `BigInt` with every primitive on the right.
macro_rules! int_operators {
    ($($p:ty),*) => {
        binary!(BigInt, Add, add, AddAssign, add_assign, add_int, [$($p),*]);
        binary!(BigInt, Sub, sub, SubAssign, sub_assign, sub_int, [$($p),*]);
        binary!(BigInt, Mul, mul, MulAssign, mul_assign, mul_int, [$($p),*]);
        binary!(BigInt, Div, div, DivAssign, div_assign, div_int, [$($p),*]);
        binary!(BigInt, Rem, rem, RemAssign, rem_assign, rem_int, [$($p),*]);
        binary!(BigInt, BitAnd, bitand, BitAndAssign, bitand_assign, and_int, [$($p),*]);
        binary!(BigInt, BitOr, bitor, BitOrAssign, bitor_assign, or_int, [$($p),*]);
        binary!(BigInt, BitXor, bitxor, BitXorAssign, bitxor_assign, xor_int, [$($p),*]);
    };
}

int_operators!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);

/// Every operator of `BigUint` with every unsigned primitive on the right.
macro_rules! nat_operators {
    ($($p:ty),*) => {
        binary!(BigUint, Add, add, AddAssign, add_assign, add_nat, [$($p),*]);
        binary!(BigUint, Sub, sub, SubAssign, sub_assign, sub_nat, [$($p),*]);
        binary!(BigUint, Mul, mul, MulAssign, mul_assign, mul_nat, [$($p),*]);
        binary!(BigUint, Div, div, DivAssign, div_assign, div_nat, [$($p),*]);
        binary!(BigUint, Rem, rem, RemAssign, rem_assign, rem_nat, [$($p),*]);
        binary!(BigUint, BitAnd, bitand, BitAndAssign, bitand_assign, and_nat, [$($p),*]);
        binary!(BigUint, BitOr, bitor, BitOrAssign, bitor_assign, or_nat, [$($p),*]);
        binary!(BigUint, BitXor, bitxor, BitXorAssign, bitxor_assign, xor_nat, [$($p),*]);
    };
}

nat_operators!(u8, u16, u32, u64, u128, usize);

/// Shifts by every primitive amount; a negative amount shifts the other way.
macro_rules! shifts {
    ($($p:ty),*) => {$(
        impl Shl<$p> for &BigInt {
            type Output = BigInt;
            fn shl(self, n: $p) -> BigInt {
                self.shift_left(i128::try_from(n).unwrap_or(i128::MAX))
            }
        }
        impl Shl<$p> for BigInt {
            type Output = BigInt;
            fn shl(self, n: $p) -> BigInt {
                self.shift_left(i128::try_from(n).unwrap_or(i128::MAX))
            }
        }
        impl Shr<$p> for &BigInt {
            type Output = BigInt;
            fn shr(self, n: $p) -> BigInt {
                self.shift_right(i128::try_from(n).unwrap_or(i128::MAX))
            }
        }
        impl Shr<$p> for BigInt {
            type Output = BigInt;
            fn shr(self, n: $p) -> BigInt {
                self.shift_right(i128::try_from(n).unwrap_or(i128::MAX))
            }
        }
        impl ShlAssign<$p> for BigInt {
            fn shl_assign(&mut self, n: $p) {
                *self = self.shift_left(i128::try_from(n).unwrap_or(i128::MAX));
            }
        }
        impl ShrAssign<$p> for BigInt {
            fn shr_assign(&mut self, n: $p) {
                *self = self.shift_right(i128::try_from(n).unwrap_or(i128::MAX));
            }
        }
        impl Shl<$p> for &BigUint {
            type Output = BigUint;
            fn shl(self, n: $p) -> BigUint {
                self.shift(i128::try_from(n).unwrap_or(i128::MAX))
            }
        }
        impl Shl<$p> for BigUint {
            type Output = BigUint;
            fn shl(self, n: $p) -> BigUint {
                self.shift(i128::try_from(n).unwrap_or(i128::MAX))
            }
        }
        impl Shr<$p> for &BigUint {
            type Output = BigUint;
            fn shr(self, n: $p) -> BigUint {
                self.shift(i128::try_from(n).map_or(i128::MIN, |n| n.saturating_neg()))
            }
        }
        impl Shr<$p> for BigUint {
            type Output = BigUint;
            fn shr(self, n: $p) -> BigUint {
                self.shift(i128::try_from(n).map_or(i128::MIN, |n| n.saturating_neg()))
            }
        }
        impl ShlAssign<$p> for BigUint {
            fn shl_assign(&mut self, n: $p) {
                *self = self.shift(i128::try_from(n).unwrap_or(i128::MAX));
            }
        }
        impl ShrAssign<$p> for BigUint {
            fn shr_assign(&mut self, n: $p) {
                *self = self.shift(i128::try_from(n).map_or(i128::MIN, |n| n.saturating_neg()));
            }
        }
    )*};
}

shifts!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);

impl BigUint {
    /// `self × 2^n`, rounded down for a negative `n`.
    fn shift(&self, n: i128) -> BigUint {
        let amount = u64::try_from(n.unsigned_abs()).unwrap_or(u64::MAX);
        BigUint::from_limbs(if n >= 0 {
            nat::shl(&self.limbs(), amount)
        } else {
            nat::shr(&self.limbs(), amount)
        })
    }
}

impl Neg for BigInt {
    type Output = BigInt;
    fn neg(self) -> BigInt {
        -&self
    }
}

impl Neg for &BigInt {
    type Output = BigInt;
    fn neg(self) -> BigInt {
        let sign = match self.sign {
            Sign::Minus => Sign::Plus,
            Sign::NoSign => Sign::NoSign,
            Sign::Plus => Sign::Minus,
        };
        BigInt {
            sign,
            magnitude: self.magnitude.clone(),
        }
    }
}

/// `!x = -x - 1`, the complement of the two's-complement bits.
impl Not for &BigInt {
    type Output = BigInt;
    fn not(self) -> BigInt {
        sub_int(&-self, &BigInt::from(1u8))
    }
}

impl Not for BigInt {
    type Output = BigInt;
    fn not(self) -> BigInt {
        !&self
    }
}

impl Sum for BigInt {
    fn sum<I: Iterator<Item = BigInt>>(iter: I) -> BigInt {
        iter.fold(BigInt::ZERO, |acc, x| acc + x)
    }
}

impl<'a> Sum<&'a BigInt> for BigInt {
    fn sum<I: Iterator<Item = &'a BigInt>>(iter: I) -> BigInt {
        iter.fold(BigInt::ZERO, |acc, x| acc + x)
    }
}

impl Product for BigInt {
    fn product<I: Iterator<Item = BigInt>>(iter: I) -> BigInt {
        iter.fold(BigInt::from(1u8), |acc, x| acc * x)
    }
}

impl<'a> Product<&'a BigInt> for BigInt {
    fn product<I: Iterator<Item = &'a BigInt>>(iter: I) -> BigInt {
        iter.fold(BigInt::from(1u8), |acc, x| acc * x)
    }
}

impl Sum for BigUint {
    fn sum<I: Iterator<Item = BigUint>>(iter: I) -> BigUint {
        iter.fold(BigUint::ZERO, |acc, x| acc + x)
    }
}
