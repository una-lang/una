//! `LC_DYLD_CHAINED_FIXUPS` (`docs/plan/0124-step3-design.md` §7), in the format of dyld's
//! `mach-o/fixup-chains.h`: pointer format `DYLD_CHAINED_PTR_64`, 4 KiB pages, imports in
//! `DYLD_CHAINED_IMPORT` form, every import bound at load.

use std::collections::BTreeMap;

use crate::bytes::{self, Writer};
use crate::layout::Fixup;
use crate::problem::{Problem, Result, shown};

/// `DYLD_CHAINED_PTR_64`.
const PTR_64: u16 = 2;
/// The page size of the chains.
pub(crate) const PAGE: u64 = 0x1000;
/// `DYLD_CHAINED_PTR_START_NONE`.
const START_NONE: u16 = 0xffff;

/// A segment as the chains see it.
pub(crate) struct ChainSegment {
    /// Its address.
    pub(crate) vmaddr: u64,
    /// Its size in memory.
    pub(crate) vmsize: u64,
    /// Its offset in the file.
    pub(crate) fileoff: u64,
    /// Its size in the file.
    pub(crate) filesize: u64,
    /// Whether the segment may hold fixups (`__DATA_CONST`, `__DATA`).
    pub(crate) writable: bool,
}

/// The fixups blob for `segments` (every `LC_SEGMENT_64`, in load-command order) and the import
/// table, and writes each fixup's chained word into `file`.
///
/// # Errors
///
/// U8001 when a fixup lies outside a writable segment's file bytes, is not 8-aligned, has a bind
/// addend outside 0..=255, or a rebase target beyond 36 bits.
pub(crate) fn write(
    file: &mut [u8],
    segments: &[ChainSegment],
    fixups: &BTreeMap<u64, Fixup<'_>>,
    imports: &[(&[u8], bool)],
    base: u64,
) -> Result<Vec<u8>> {
    let ordinal: BTreeMap<&[u8], u32> = imports
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (*name, u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    // Group the fixups by segment and page.
    let mut pages: Vec<BTreeMap<u64, Vec<(u64, Fixup<'_>)>>> =
        segments.iter().map(|_| BTreeMap::new()).collect();
    for (&address, fixup) in fixups {
        if address % 8 != 0 {
            return Err(Problem::internal(format!(
                "a chained fixup at {address:#x} that is not 8-aligned"
            )));
        }
        let position = segments
            .iter()
            .position(|s| s.writable && address >= s.vmaddr && address + 8 <= s.vmaddr + s.filesize)
            .ok_or_else(|| {
                Problem::internal(format!(
                    "a chained fixup at {address:#x} outside the file bytes of a writable segment"
                ))
            })?;
        let segment = segments
            .get(position)
            .ok_or_else(|| Problem::internal("a segment index"))?;
        let page = (address - segment.vmaddr) / PAGE;
        if let Some(slot) = pages.get_mut(position) {
            slot.entry(page).or_default().push((address, *fixup));
        }
    }
    // The chained words in the file.
    for (position, segment_pages) in pages.iter().enumerate() {
        let segment = segments
            .get(position)
            .ok_or_else(|| Problem::internal("a segment index"))?;
        for list in segment_pages.values() {
            for (index, (address, fixup)) in list.iter().enumerate() {
                let next = match list.get(index + 1) {
                    Some((following, _)) => (following - address) / 4,
                    None => 0,
                };
                if next >= 1 << 12 {
                    return Err(Problem::internal("two fixups of one page too far apart"));
                }
                let word = match fixup {
                    Fixup::Rebase(target) => {
                        let low = target & ((1 << 36) - 1);
                        let high8 = target >> 56;
                        if target & !((1 << 36) - 1) & !(0xff << 56) != 0 {
                            return Err(Problem::internal(format!(
                                "a rebase to {target:#x}, beyond what DYLD_CHAINED_PTR_64 holds"
                            )));
                        }
                        low | (high8 << 36) | (next << 51)
                    }
                    Fixup::Bind(name, addend) => {
                        let ordinal = u64::from(*ordinal.get(name).ok_or_else(|| {
                            Problem::internal(format!("a bind to `{}`, not an import", shown(name)))
                        })?);
                        let addend = u64::try_from(*addend)
                            .ok()
                            .filter(|a| *a <= 0xff)
                            .ok_or_else(|| {
                                Problem::internal(format!(
                                    "a bind to `{}` with an addend of {addend}, outside 0 to 255",
                                    shown(name)
                                ))
                            })?;
                        ordinal | (addend << 24) | (next << 51) | (1 << 63)
                    }
                };
                let offset = segment.fileoff + (address - segment.vmaddr);
                bytes::put_u64(file, offset, word)?;
            }
        }
    }
    // The blob: header, starts, imports, symbols.
    let mut starts = Writer::default();
    let count = u32::try_from(segments.len()).unwrap_or(0);
    starts.u32(count);
    let table_at = starts.len();
    for _ in segments {
        starts.u32(0);
    }
    for (position, segment) in segments.iter().enumerate() {
        let Some(segment_pages) = pages.get(position) else {
            continue;
        };
        if segment_pages.is_empty() {
            continue;
        }
        starts.pad_to(8);
        let offset = u32::try_from(starts.len()).unwrap_or(0);
        bytes::put_u32(
            &mut starts.bytes,
            table_at + 4 * u64::try_from(position).unwrap_or(0),
            offset,
        )?;
        let page_count = segment.vmsize.div_ceil(PAGE);
        let page_count_u16 = u16::try_from(page_count).map_err(|_| {
            Problem::layout("a segment with more pages than chained fixups describe")
        })?;
        let size = 22 + 2 * u32::from(page_count_u16);
        starts.u32(size);
        starts.u16(u16::try_from(PAGE).unwrap_or(0));
        starts.u16(PTR_64);
        starts.u64(segment.vmaddr - base);
        starts.u32(0);
        starts.u16(page_count_u16);
        for page in 0..page_count {
            let start = match segment_pages.get(&page).and_then(|list| list.first()) {
                Some((address, _)) => {
                    u16::try_from((address - segment.vmaddr) % PAGE).unwrap_or(START_NONE)
                }
                None => START_NONE,
            };
            starts.u16(start);
        }
    }
    let mut names = Writer::default();
    let mut table = Writer::default();
    for (name, weak) in imports {
        let offset = u32::try_from(names.len()).unwrap_or(u32::MAX);
        if offset >= 1 << 23 {
            return Err(Problem::layout(
                "the names of the imports exceed what chained fixups hold",
            ));
        }
        names.raw(name);
        names.u8(0);
        table.u32(1 | (u32::from(*weak) << 8) | (offset << 9));
    }
    let mut blob = Writer::default();
    let starts_offset = 32u32;
    let imports_offset = starts_offset + u32::try_from(starts.len()).unwrap_or(0);
    let imports_offset = (imports_offset + 3) & !3;
    let symbols_offset = imports_offset + u32::try_from(table.len()).unwrap_or(0);
    blob.u32(0);
    blob.u32(starts_offset);
    blob.u32(imports_offset);
    blob.u32(symbols_offset);
    blob.u32(u32::try_from(imports.len()).unwrap_or(0));
    blob.u32(1);
    blob.u32(0);
    blob.pad_to(8);
    blob.raw(&starts.bytes);
    blob.pad_to(4);
    blob.raw(&table.bytes);
    blob.raw(&names.bytes);
    blob.pad_to(8);
    Ok(blob.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rebase_and_a_bind_chain_within_their_page() {
        let base = 0x1_0000_0000;
        let segments = [
            ChainSegment {
                vmaddr: 0,
                vmsize: base,
                fileoff: 0,
                filesize: 0,
                writable: false,
            },
            ChainSegment {
                vmaddr: base + 0x4000,
                vmsize: 0x4000,
                fileoff: 0x4000,
                filesize: 0x4000,
                writable: true,
            },
        ];
        let mut file = vec![0u8; 0x8000];
        let mut fixups = BTreeMap::new();
        fixups.insert(base + 0x4010, Fixup::Rebase(base + 0x123));
        fixups.insert(base + 0x4018, Fixup::Bind(b"_write", 4));
        fixups.insert(base + 0x5000, Fixup::Rebase(base));
        let imports: Vec<(&[u8], bool)> = vec![(b"_write", false)];
        let blob = write(&mut file, &segments, &fixups, &imports, base).unwrap();
        let rebase = bytes::u64_at(&file, 0x4010).unwrap();
        assert_eq!(rebase & ((1 << 36) - 1), base + 0x123);
        assert_eq!((rebase >> 51) & 0xfff, 2, "the next fixup is 8 bytes on");
        assert_eq!(rebase >> 63, 0);
        let bind = bytes::u64_at(&file, 0x4018).unwrap();
        assert_eq!(bind >> 63, 1);
        assert_eq!(bind & 0xff_ffff, 0);
        assert_eq!((bind >> 24) & 0xff, 4);
        assert_eq!((bind >> 51) & 0xfff, 0, "the last of its page");
        // The header names one import of format DYLD_CHAINED_IMPORT.
        assert_eq!(bytes::u32_at(&blob, 16).unwrap(), 1);
        assert_eq!(bytes::u32_at(&blob, 20).unwrap(), 1);
        // A misaligned fixup and one in a read-only segment are refused.
        let mut bad = BTreeMap::new();
        bad.insert(base + 0x4004, Fixup::Rebase(base));
        assert!(write(&mut file, &segments, &bad, &imports, base).is_err());
        let mut bad = BTreeMap::new();
        bad.insert(0x10, Fixup::Rebase(base));
        assert!(write(&mut file, &segments, &bad, &imports, base).is_err());
        let mut bad = BTreeMap::new();
        bad.insert(base + 0x4008, Fixup::Bind(b"_write", 256));
        assert!(write(&mut file, &segments, &bad, &imports, base).is_err());
    }
}
