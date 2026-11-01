//! Constant-time helpers (ADR 0133 rule 5).

use core::hint::black_box;

/// Compares two byte strings of public length in time that does not depend on where they
/// differ. Strings of different length are unequal at once: lengths are public.
#[must_use]
pub fn eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    // The barrier keeps the optimizer from turning the fold into an early exit.
    black_box(diff) == 0
}

/// All ones when `bit` is 1, all zeros when it is 0. `bit` must be 0 or 1.
#[must_use]
pub(crate) fn mask64(bit: u64) -> u64 {
    black_box(0u64.wrapping_sub(bit & 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_unequal_and_lengths() {
        assert!(eq(b"", b""));
        assert!(eq(b"abc", b"abc"));
        assert!(!eq(b"abc", b"abd"));
        assert!(!eq(b"abc", b"ab"));
        assert_eq!(mask64(1), u64::MAX);
        assert_eq!(mask64(0), 0);
    }
}
