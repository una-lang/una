//! Conversion of natural numbers to and from digits in a radix from 2 to 36 (#1093).
//!
//! A power-of-two radix reads and writes the bits directly, O(n). Any other radix works on
//! chunks of digits, `radix^d`. A small value is written by repeated division by the largest
//! chunk below 2^32, two hardware divisions per limb, and read by Horner's rule on the
//! largest chunk in a limb, both O(n²). From [`TO_TEXT_DIVIDE`] limbs, or
//! [`FROM_TEXT_DIVIDE`] chunks of digits, a conversion divides and conquers over the powers
//! `radix^(d·2^i)`, splitting the value (or the digits) in two halves at each level:
//! O(M(n) log n), M(n) the cost of a multiplication, since each level costs one division
//! (Burnikel-Ziegler) or one multiplication of its size.

use super::nat;

/// From this many limbs, writing digits divides and conquers.
pub(super) const TO_TEXT_DIVIDE: usize = 32;

/// Below this many limbs, a part of a divided value is written by repeated division.
const TO_TEXT_BASE: usize = 8;

/// From this many chunks of a limb's worth of digits, reading divides and conquers.
pub(super) const FROM_TEXT_DIVIDE: usize = 384;

/// The digits per chunk and the chunk `radix^d`, the largest power of `radix` in a limb.
fn chunk(radix: u32) -> (usize, u64) {
    largest_power(radix, u64::MAX)
}

/// The digits per chunk and the chunk `radix^d`, the largest power of `radix` below 2^32,
/// which divides a limb in two 64-bit hardware divisions.
fn half_chunk(radix: u32) -> (usize, u64) {
    largest_power(radix, u64::from(u32::MAX))
}

/// The largest `radix^d <= limit`, with `d`.
fn largest_power(radix: u32, limit: u64) -> (usize, u64) {
    let radix = u64::from(radix.clamp(2, 36));
    let mut digits = 1;
    let mut base = radix;
    while let Some(next) = base.checked_mul(radix).filter(|n| *n <= limit) {
        base = next;
        digits += 1;
    }
    (digits, base)
}

/// The ASCII digit of `d < 36`, lowercase.
fn ascii(d: u8) -> u8 {
    if d < 10 { b'0' + d } else { b'a' + (d - 10) }
}

/// The digits of `a` in `radix`, most significant first, lowercase, `0` for zero.
pub(super) fn to_text(a: &[u64], radix: u32) -> String {
    let a = nat::trim(a);
    let radix = radix.clamp(2, 36);
    if a.is_empty() {
        return "0".to_owned();
    }
    if let [lo, hi @ ..] = a
        && hi.len() <= 1
        && radix == 10
    {
        let hi = hi.first().copied().unwrap_or(0);
        return (u128::from(*lo) | (u128::from(hi) << 64)).to_string();
    }
    let digits = if radix.is_power_of_two() {
        power_of_two_digits(a, radix.trailing_zeros())
    } else if a.len() < TO_TEXT_DIVIDE {
        small_digits(a, radix)
    } else {
        let (d, base) = chunk(radix);
        let mut powers = vec![vec![base]];
        while let Some(last) = powers.last() {
            if 2 * last.len() > a.len() {
                break;
            }
            let next = nat::mul(last, last);
            powers.push(next);
        }
        let mut out = Vec::with_capacity(a.len() * 20);
        emit(a, None, radix, d, &powers, &mut out);
        out
    };
    // Every byte is an ASCII digit, so the conversion never fails.
    String::from_utf8(digits).unwrap_or_default()
}

/// The digits of a power-of-two radix of `bits` bits each, most significant first. O(n).
fn power_of_two_digits(a: &[u64], bits: u32) -> Vec<u8> {
    let total = nat::bits(a);
    let count = total.div_ceil(u64::from(bits));
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for i in (0..count).rev() {
        let at = i * u64::from(bits);
        let limb = usize::try_from(at / 64).unwrap_or(usize::MAX);
        let offset = (at % 64) as u32;
        let low = nat::part(a, limb, limb + 1).first().copied().unwrap_or(0) >> offset;
        let high = if offset + bits > 64 {
            nat::part(a, limb + 1, limb + 2)
                .first()
                .copied()
                .unwrap_or(0)
                << (64 - offset)
        } else {
            0
        };
        out.push(ascii(((low | high) & mask) as u8));
    }
    out
}

/// Appends the digits of `a` to `out`: exactly `width` digits, zeros first, when it is given
/// (`a < radix^width`), else without leading zeros (nothing for zero).
fn emit(
    a: &[u64],
    width: Option<usize>,
    radix: u32,
    d: usize,
    powers: &[Vec<u64>],
    out: &mut Vec<u8>,
) {
    let a = nat::trim(a);
    // The largest power of at most half of `a`'s limbs, so both halves shrink.
    let level = powers
        .iter()
        .rposition(|p| 2 * p.len() <= a.len() + 1)
        .filter(|_| a.len() >= TO_TEXT_BASE);
    let Some(level) = level else {
        let digits = small_digits(a, radix);
        if let Some(width) = width {
            out.extend(std::iter::repeat_n(
                b'0',
                width.saturating_sub(digits.len()),
            ));
        }
        out.extend(digits);
        return;
    };
    let Some(power) = powers.get(level) else {
        return;
    };
    let low = d << level;
    let (q, r) = nat::divrem(a, power).unwrap_or_default();
    emit(
        &q,
        width.map(|w| w.saturating_sub(low)),
        radix,
        d,
        powers,
        out,
    );
    emit(&r, Some(low), radix, d, powers, out);
}

/// The digits of `a` without leading zeros, by repeated division by the half chunk. O(n²).
fn small_digits(a: &[u64], radix: u32) -> Vec<u8> {
    // Radix 10 gets its own copy, where the divisions by constants become multiplications.
    if radix == 10 {
        small_digits_in(a, 10)
    } else {
        small_digits_in(a, radix)
    }
}

#[inline(always)]
fn small_digits_in(a: &[u64], radix: u32) -> Vec<u8> {
    let (d, base) = half_chunk(radix);
    let radix = u64::from(radix);
    let mut rest = nat::trim(a).to_vec();
    // Least significant digit first, reversed at the end.
    let mut out = Vec::with_capacity(rest.len() * 20);
    while !rest.is_empty() {
        nat::count(rest.len());
        let mut r = 0u64;
        for limb in rest.iter_mut().rev() {
            // r < base < 2^32, so each step divides a value below 2^64.
            let high = (r << 32) | (*limb >> 32);
            let (q_high, r_high) = (high / base, high % base);
            let low = (r_high << 32) | (*limb & 0xFFFF_FFFF);
            let (q_low, r_low) = (low / base, low % base);
            *limb = (q_high << 32) | q_low;
            r = r_low;
        }
        while rest.last() == Some(&0) {
            rest.pop();
        }
        let start = out.len();
        while r > 0 {
            out.push(ascii((r % radix) as u8));
            r /= radix;
        }
        if !rest.is_empty() {
            out.resize(start + d, b'0');
        }
    }
    out.reverse();
    out
}

/// The natural number of `digits`, values below `radix`, most significant first.
pub(super) fn from_digits(digits: &[u8], radix: u32) -> Vec<u64> {
    let radix = radix.clamp(2, 36);
    if radix.is_power_of_two() {
        return from_power_of_two(digits, radix.trailing_zeros());
    }
    let (d, base) = chunk(radix);
    if digits.len() <= FROM_TEXT_DIVIDE * d {
        return horner(digits, radix, d);
    }
    let mut powers = vec![vec![base]];
    while let Some(last) = powers.last() {
        if (d << powers.len()) >= digits.len() {
            break;
        }
        let next = nat::mul(last, last);
        powers.push(next);
    }
    read(digits, radix, d, &powers)
}

/// The bits of digits of a power-of-two radix. O(n).
fn from_power_of_two(digits: &[u8], bits: u32) -> Vec<u64> {
    let mut out = Vec::with_capacity(digits.len() * bits as usize / 64 + 1);
    let mut acc = 0u64;
    let mut filled = 0u32;
    for &digit in digits.iter().rev() {
        let digit = u64::from(digit);
        acc |= digit << filled;
        filled += bits;
        if filled >= 64 {
            out.push(acc);
            filled -= 64;
            acc = if filled == 0 {
                0
            } else {
                digit >> (bits - filled)
            };
        }
    }
    if filled > 0 {
        out.push(acc);
    }
    nat::normalized(out)
}

/// The value of `digits`: Horner's rule on chunks below the threshold, O(n²), else the high
/// part times `radix^(d·2^i)` plus the low `d·2^i` digits, for the largest such power below
/// the length.
fn read(digits: &[u8], radix: u32, d: usize, powers: &[Vec<u64>]) -> Vec<u64> {
    let level = (0..powers.len())
        .rev()
        .find(|&i| (d << i) < digits.len())
        .filter(|_| digits.len() > FROM_TEXT_DIVIDE * d);
    let Some((level, power)) = level.and_then(|i| powers.get(i).map(|p| (i, p))) else {
        return horner(digits, radix, d);
    };
    let split = digits.len() - (d << level);
    let (high, low) = digits.split_at_checked(split).unwrap_or((digits, &[]));
    let high = read(high, radix, d, powers);
    let low = read(low, radix, d, powers);
    nat::add(&nat::mul(&high, power), &low)
}

/// Horner's rule over chunks of `d` digits. O(n²).
fn horner(digits: &[u8], radix: u32, d: usize) -> Vec<u64> {
    // Radix 10 gets its own copy, where the multiplications by constants fold.
    if radix == 10 {
        horner_in(digits, 10, d)
    } else {
        horner_in(digits, radix, d)
    }
}

#[inline(always)]
fn horner_in(digits: &[u8], radix: u32, d: usize) -> Vec<u64> {
    let (_, base) = chunk(radix);
    let radix = u64::from(radix);
    // A group of at most d digits below `radix` is below radix^d, which fits a limb.
    let value = |group: &[u8]| {
        group
            .iter()
            .fold(0u64, |v, &digit| v * radix + u64::from(digit))
    };
    let mut out: Vec<u64> = Vec::with_capacity(digits.len() / d + 2);
    let (head, tail) = digits
        .split_at_checked(digits.len() % d)
        .unwrap_or((&[], digits));
    let first = value(head);
    if first != 0 {
        out.push(first);
    }
    for group in tail.chunks(d) {
        // out = out × radix^d + the group's value.
        let mut carry = value(group);
        nat::count(out.len());
        for limb in out.iter_mut() {
            let t = u128::from(*limb) * u128::from(base) + u128::from(carry);
            *limb = t as u64;
            carry = (t >> 64) as u64;
        }
        if carry != 0 {
            out.push(carry);
        }
    }
    out
}
