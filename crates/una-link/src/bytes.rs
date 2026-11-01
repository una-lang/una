//! Bounds-checked reads and writes of little-endian integers in byte buffers. Every access of
//! the linker to an input or to the output goes through these, so an offset outside a buffer is
//! a [`Problem`], never a panic.

use crate::problem::{Problem, Result};

/// The `N` bytes at `offset` of `data`.
pub(crate) fn array<const N: usize>(data: &[u8], offset: u64) -> Result<[u8; N]> {
    let start = usize::try_from(offset).map_err(|_| out_of_range(offset, N))?;
    let end = start
        .checked_add(N)
        .ok_or_else(|| out_of_range(offset, N))?;
    let slice = data
        .get(start..end)
        .ok_or_else(|| out_of_range(offset, N))?;
    let mut out = [0u8; N];
    out.copy_from_slice(slice);
    Ok(out)
}

/// The little-endian `u32` at `offset`.
pub(crate) fn u32_at(data: &[u8], offset: u64) -> Result<u32> {
    array::<4>(data, offset).map(u32::from_le_bytes)
}

/// The little-endian `u64` at `offset`.
pub(crate) fn u64_at(data: &[u8], offset: u64) -> Result<u64> {
    array::<8>(data, offset).map(u64::from_le_bytes)
}

/// Writes `bytes` at `offset` of `data`.
pub(crate) fn put(data: &mut [u8], offset: u64, bytes: &[u8]) -> Result<()> {
    let start = usize::try_from(offset).map_err(|_| out_of_range(offset, bytes.len()))?;
    let end = start
        .checked_add(bytes.len())
        .ok_or_else(|| out_of_range(offset, bytes.len()))?;
    let slice = data
        .get_mut(start..end)
        .ok_or_else(|| out_of_range(offset, bytes.len()))?;
    slice.copy_from_slice(bytes);
    Ok(())
}

/// Writes a little-endian `u32` at `offset`.
pub(crate) fn put_u32(data: &mut [u8], offset: u64, value: u32) -> Result<()> {
    put(data, offset, &value.to_le_bytes())
}

/// Writes a little-endian `u64` at `offset`.
pub(crate) fn put_u64(data: &mut [u8], offset: u64, value: u64) -> Result<()> {
    put(data, offset, &value.to_le_bytes())
}

/// `value` rounded up to a multiple of `align`, a power of two (1 when 0).
pub(crate) fn align_up(value: u64, align: u64) -> u64 {
    let align = align.max(1);
    value.div_ceil(align).saturating_mul(align)
}

/// A growing output buffer of little-endian fields.
#[derive(Default)]
pub(crate) struct Writer {
    /// The bytes written so far.
    pub(crate) bytes: Vec<u8>,
}

impl Writer {
    /// Appends a `u8`.
    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }
    /// Appends a little-endian `u16`.
    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    /// Appends a little-endian `u32`.
    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    /// Appends a little-endian `u64`.
    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    /// Appends raw bytes.
    pub(crate) fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
    /// Appends a name padded with zeros to 16 bytes (a segment or section name).
    pub(crate) fn name16(&mut self, name: &str) {
        let mut field = [0u8; 16];
        for (slot, byte) in field.iter_mut().zip(name.bytes()) {
            *slot = byte;
        }
        self.raw(&field);
    }
    /// Appends an unsigned LEB128 number.
    pub(crate) fn uleb(&mut self, mut value: u64) {
        loop {
            let byte = u8::try_from(value & 0x7f).unwrap_or(0);
            value >>= 7;
            if value == 0 {
                self.u8(byte);
                break;
            }
            self.u8(byte | 0x80);
        }
    }
    /// Pads with zeros to a multiple of `align`.
    pub(crate) fn pad_to(&mut self, align: u64) {
        let len = self.len();
        let target = align_up(len, align);
        self.bytes
            .resize(usize::try_from(target).unwrap_or(self.bytes.len()), 0);
    }
    /// The length written so far.
    pub(crate) fn len(&self) -> u64 {
        u64::try_from(self.bytes.len()).unwrap_or(u64::MAX)
    }
}

fn out_of_range(offset: u64, width: usize) -> Problem {
    Problem::internal(format!(
        "an access of {width} bytes at offset {offset:#x} lies outside its buffer"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_are_bounds_checked() {
        let mut data = [0u8; 8];
        assert!(put_u32(&mut data, 4, 0xdead_beef).is_ok());
        assert_eq!(u32_at(&data, 4), Ok(0xdead_beef));
        assert!(put_u32(&mut data, 5, 1).is_err());
        assert!(u64_at(&data, 1).is_err());
        assert!(u32_at(&data, u64::MAX).is_err());
    }

    #[test]
    fn uleb128_encodes_as_dwarf_does() {
        let mut w = Writer::default();
        w.uleb(0);
        w.uleb(127);
        w.uleb(128);
        w.uleb(624_485);
        assert_eq!(w.bytes, vec![0, 0x7f, 0x80, 0x01, 0xe5, 0x8e, 0x26]);
    }

    #[test]
    fn alignment_rounds_up() {
        assert_eq!(align_up(0, 16), 0);
        assert_eq!(align_up(1, 16), 16);
        assert_eq!(align_up(16, 16), 16);
        assert_eq!(align_up(17, 0), 17);
    }
}
