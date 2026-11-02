//! The names of an ELF object, read in time linear in the object's size.
//!
//! A string table may share bytes between names: `st_name` (or `sh_name`) is any offset, and a
//! name runs to the next NUL. Finding that NUL by scanning from each offset costs the name's
//! length per symbol, so N symbols that all name one long string cost N times its length. Here
//! the NULs of a table are indexed once, in O(size of the table), and each name is found in
//! O(log) by a binary search. Names that overlap still have a total length larger than the
//! table, and work that reads a name (a comparison, a copy for a message) costs its length; so
//! [`Budget`] bounds the total length of the names of an object to a multiple of its size, and
//! refuses an object past it (U8001): every pass over the names is then linear in the input.

use crate::problem::{Problem, Result};

/// A string table with its NULs indexed.
pub(crate) struct Strings<'a> {
    data: &'a [u8],
    /// The offset of each NUL, in increasing order.
    nuls: Vec<u32>,
}

impl<'a> Strings<'a> {
    /// Indexes `data`, a string table. O(len).
    ///
    /// # Errors
    ///
    /// U8001 when the table is larger than 4 GiB, which no `u32` offset can address whole.
    pub(crate) fn new(data: &'a [u8]) -> Result<Strings<'a>> {
        let mut nuls = Vec::new();
        for (at, byte) in data.iter().enumerate() {
            if *byte == 0 {
                nuls.push(
                    u32::try_from(at)
                        .map_err(|_| Problem::internal("a string table larger than 4 GiB"))?,
                );
            }
        }
        Ok(Strings { data, nuls })
    }

    /// The name at `offset`: the bytes up to the next NUL, or `None` when the offset is past the
    /// table or no NUL follows it. O(log).
    pub(crate) fn get(&self, offset: u32) -> Option<&'a [u8]> {
        let index = self.nuls.partition_point(|nul| *nul < offset);
        let end = usize::try_from(*self.nuls.get(index)?).ok()?;
        let start = usize::try_from(offset).ok()?;
        self.data.get(start..end)
    }
}

/// How many bytes of names an object may hold, counted with their overlaps, per byte of the
/// object: a compiler's string table shares at most a name's suffix with another name, so real
/// objects stay well below it (the musl inputs and the standard library of both targets are
/// checked by `tests/elf_reloc.rs`).
pub(crate) const NAMES_PER_BYTE: u64 = 4;

/// The bound of [`NAMES_PER_BYTE`] on the names of one object.
pub(crate) struct Budget {
    left: u64,
}

impl Budget {
    /// The budget of an object of `size` bytes: [`NAMES_PER_BYTE`] times it, and at least 64 KiB.
    pub(crate) fn new(size: usize) -> Budget {
        let size = u64::try_from(size).unwrap_or(u64::MAX);
        Budget {
            left: size.saturating_mul(NAMES_PER_BYTE).max(1 << 16),
        }
    }

    /// Takes `name`'s length from the budget.
    ///
    /// # Errors
    ///
    /// U8001 naming the object `object` when the budget is exhausted.
    pub(crate) fn take(&mut self, object: &str, name: &[u8]) -> Result<()> {
        let length = u64::try_from(name.len()).unwrap_or(u64::MAX);
        self.left = self.left.checked_sub(length).ok_or_else(|| {
            Problem::internal(format!(
                "cannot read `{object}`: its names, counted with their overlaps in the string table, total more than {NAMES_PER_BYTE} times its size"
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_found_by_their_offset_and_overlaps_are_counted() {
        let strings = Strings::new(b"\0main\0foo\0").unwrap();
        assert_eq!(strings.get(0), Some(&b""[..]));
        assert_eq!(strings.get(1), Some(&b"main"[..]));
        assert_eq!(strings.get(3), Some(&b"in"[..]));
        assert_eq!(strings.get(6), Some(&b"foo"[..]));
        assert_eq!(strings.get(10), None);
        assert_eq!(Strings::new(b"\0abc").unwrap().get(1), None);
        let mut budget = Budget::new(0);
        assert!(budget.take("x.o", &[0; 1 << 16]).is_ok());
        assert!(budget.take("x.o", b"a").is_err());
    }
}
