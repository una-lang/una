//! Unwind information (ADR 0124 rule 25, `docs/plan/0124-step3-design.md` §8): `__eh_frame`
//! with the records of live functions, and `__unwind_info` built from the inputs'
//! `__compact_unwind` entries, in the format of `mach-o/compact_unwind_encoding.h` (version 1,
//! regular second-level pages). Every function of `__text` has an entry, with encoding 0 when it
//! has no unwind information, so a lookup never borrows its neighbor's.

use std::collections::{BTreeMap, BTreeSet};

use crate::bytes::{self, Writer};
use crate::gc::{Graph, Target};
use crate::problem::{Problem, Result};
use crate::sections::Out;

/// `UNWIND_ARM64_MODE_MASK`.
const MODE_MASK: u32 = 0x0f00_0000;
/// `UNWIND_ARM64_MODE_DWARF`.
const MODE_DWARF: u32 = 0x0300_0000;
/// `UNWIND_PERSONALITY_MASK`.
const PERSONALITY_MASK: u32 = 0x3000_0000;
/// `UNWIND_HAS_LSDA`.
const HAS_LSDA: u32 = 0x4000_0000;
/// Entries of a regular second-level page: (4096 - 8) / 8.
const PAGE_ENTRIES: usize = 511;

/// Where an unwind entry starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Start {
    /// An offset into a live atom of `__text`.
    Code(usize, i64),
    /// The start of `__stubs`, which has no unwind information.
    Stubs,
}

/// One entry of `__unwind_info`.
#[derive(Clone, Debug)]
pub(crate) struct Entry<'a> {
    /// Where the function starts.
    pub(crate) start: Start,
    /// Its encoding, complete but for nothing: the DWARF offset and personality index included.
    pub(crate) encoding: u32,
    /// Its LSDA, when the encoding has `UNWIND_HAS_LSDA`.
    pub(crate) lsda: Option<Target<'a>>,
}

/// The unwind plan, made before addresses are known: the records of `__eh_frame` with their
/// offsets, and the entries of `__unwind_info` with its size.
pub(crate) struct Plan<'a> {
    /// The kept records of `__eh_frame`, each with its offset in the output section.
    pub(crate) records: Vec<(usize, u64)>,
    /// The size of `__eh_frame`.
    pub(crate) eh_size: u64,
    /// The entries, in address order.
    pub(crate) entries: Vec<Entry<'a>>,
    /// The personality routines, each through a GOT slot.
    pub(crate) personalities: Vec<Target<'a>>,
    /// The size of `__unwind_info`.
    pub(crate) size: u64,
}

/// Plans `__eh_frame` and `__unwind_info` for the live functions of `graph`, with `stubs`
/// telling whether `__stubs` follows `__text`.
pub(crate) fn plan<'a>(graph: &Graph<'a>, stubs: bool) -> Result<Plan<'a>> {
    // The records of __eh_frame: every live FDE and the CIEs they use, in input order.
    let mut records = Vec::new();
    let mut offsets: BTreeMap<usize, u64> = BTreeMap::new();
    let mut cursor = 0u64;
    for (index, record) in graph.eh_records.iter().enumerate() {
        if !record.live {
            continue;
        }
        if let Some(cie) = record.cie
            && !offsets.contains_key(&cie)
        {
            return Err(Problem::internal(
                "an FDE of __eh_frame whose CIE is not kept before it",
            ));
        }
        offsets.insert(index, cursor);
        records.push((index, cursor));
        cursor = cursor.saturating_add(record.end.saturating_sub(record.start));
    }
    let eh_size = cursor;
    if eh_size > 0x00ff_ffff {
        return Err(Problem::layout(
            "the __eh_frame section is larger than the 16 MiB that compact unwind encodings reach",
        ));
    }
    // The starts: every live atom of __text, and every live entry or FDE inside one.
    let mut compact: BTreeMap<(usize, i64), usize> = BTreeMap::new();
    for (index, entry) in graph.compact.iter().enumerate() {
        if let Target::Atom { atom, offset } = entry.function
            && graph.atom(atom)?.live
        {
            compact.entry((atom, offset)).or_insert(index);
        }
    }
    let mut fdes: BTreeMap<(usize, i64), usize> = BTreeMap::new();
    for (index, record) in graph.eh_records.iter().enumerate() {
        if record.live
            && let Some(Target::Atom { atom, offset }) = record.function
        {
            fdes.entry((atom, offset)).or_insert(index);
        }
    }
    let mut starts: BTreeSet<(usize, i64)> = BTreeSet::new();
    for (index, atom) in graph.atoms.iter().enumerate() {
        if atom.live && atom.out == Out::Text && atom.end > atom.start {
            starts.insert((index, 0));
        }
    }
    for key in compact.keys().chain(fdes.keys()) {
        let atom = graph.atom(key.0)?;
        let size = i64::try_from(atom.end.saturating_sub(atom.start)).unwrap_or(i64::MAX);
        if atom.out == Out::Text && (0..size).contains(&key.1) {
            starts.insert(*key);
        }
    }
    let mut personalities: Vec<Target<'a>> = Vec::new();
    let mut entries = Vec::with_capacity(starts.len().saturating_add(1));
    for key in starts {
        let fde_encoding = |record: usize| -> Result<u32> {
            let offset = offsets.get(&record).copied().unwrap_or(0);
            Ok(MODE_DWARF
                | u32::try_from(offset).map_err(|_| Problem::layout("an FDE offset too large"))?)
        };
        let entry = if let Some(cu) = compact.get(&key).and_then(|i| graph.compact.get(*i)) {
            if cu.encoding & MODE_MASK == MODE_DWARF {
                let record = fdes.get(&key).copied().ok_or_else(|| {
                    Problem::internal(
                        "a compact unwind entry in DWARF mode for a function with no FDE",
                    )
                })?;
                Entry {
                    start: Start::Code(key.0, key.1),
                    encoding: fde_encoding(record)?,
                    lsda: None,
                }
            } else {
                let mut encoding = cu.encoding & !(PERSONALITY_MASK | HAS_LSDA);
                if let Some(personality) = cu.personality {
                    let index = match personalities.iter().position(|p| *p == personality) {
                        Some(index) => index,
                        None => {
                            personalities.push(personality);
                            personalities.len().saturating_sub(1)
                        }
                    };
                    if index >= 3 {
                        return Err(Problem::internal(
                            "more than three personality routines in the unwind information",
                        ));
                    }
                    encoding |= u32::try_from(index.saturating_add(1)).unwrap_or(0) << 28;
                }
                if cu.lsda.is_some() {
                    encoding |= HAS_LSDA;
                }
                Entry {
                    start: Start::Code(key.0, key.1),
                    encoding,
                    lsda: cu.lsda,
                }
            }
        } else if let Some(record) = fdes.get(&key) {
            Entry {
                start: Start::Code(key.0, key.1),
                encoding: fde_encoding(*record)?,
                lsda: None,
            }
        } else {
            Entry {
                start: Start::Code(key.0, key.1),
                encoding: 0,
                lsda: None,
            }
        };
        entries.push(entry);
    }
    if stubs {
        entries.push(Entry {
            start: Start::Stubs,
            encoding: 0,
            lsda: None,
        });
    }
    let count = u64::try_from(entries.len()).unwrap_or(u64::MAX);
    let pages = count.div_ceil(PAGE_ENTRIES as u64);
    let lsdas = u64::try_from(entries.iter().filter(|e| e.lsda.is_some()).count()).unwrap_or(0);
    let people = u64::try_from(personalities.len()).unwrap_or(0);
    let size = 28 + 4 * people + 12 * (pages + 1) + 8 * lsdas + 8 * pages + 8 * count;
    Ok(Plan {
        records,
        eh_size,
        entries,
        personalities,
        size,
    })
}

/// The addresses the writers need, from the layout.
pub(crate) trait Addresses<'a> {
    /// The address of a target.
    fn address(&self, target: Target<'a>) -> Result<u64>;
    /// The address of the GOT slot of a target.
    fn got(&self, target: Target<'a>) -> Result<u64>;
    /// The address of `__stubs`.
    fn stubs(&self) -> u64;
}

/// The bytes of `__eh_frame` at `base`: the kept records, their CIE pointers recomputed and
/// their relocations applied.
pub(crate) fn write_eh_frame<'a>(
    graph: &Graph<'a>,
    plan: &Plan<'a>,
    addresses: &dyn Addresses<'a>,
    base: u64,
) -> Result<Vec<u8>> {
    let mut out = vec![0u8; usize::try_from(plan.eh_size).unwrap_or(0)];
    let offsets: BTreeMap<usize, u64> = plan.records.iter().copied().collect();
    for &(index, offset) in &plan.records {
        let record = graph
            .eh_records
            .get(index)
            .ok_or_else(|| Problem::internal("an eh_frame record index out of range"))?;
        let section = graph.section(record.object, record.section)?;
        let bytes_in = section
            .data
            .get(
                usize::try_from(record.start).unwrap_or(usize::MAX)
                    ..usize::try_from(record.end).unwrap_or(usize::MAX),
            )
            .ok_or_else(|| Problem::internal("an eh_frame record outside its section"))?;
        bytes::put(&mut out, offset, bytes_in)?;
        if let Some(cie) = record.cie {
            let cie_offset = offsets
                .get(&cie)
                .copied()
                .ok_or_else(|| Problem::internal("an FDE whose CIE is not kept"))?;
            let pointer = (offset + 4)
                .checked_sub(cie_offset)
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| Problem::internal("an FDE placed before its CIE"))?;
            bytes::put_u32(&mut out, offset + 4, pointer)?;
        }
        for field in &record.fields {
            let at = offset + (field.offset - record.start);
            let place = base.saturating_add(at);
            let to = if field.got {
                addresses.got(field.target)?
            } else {
                addresses.address(field.target)?
            };
            let value = i128::from(to) - i128::from(place);
            write_signed(&mut out, at, field.width, value)?;
        }
    }
    Ok(out)
}

/// Writes `value` into a field of `width` bytes, refusing a value that does not fit.
pub(crate) fn write_signed(out: &mut [u8], at: u64, width: u64, value: i128) -> Result<()> {
    match width {
        4 => {
            let value = i32::try_from(value).map_err(|_| {
                Problem::internal(format!("a 32-bit field at {at:#x} cannot hold {value:#x}"))
            })?;
            bytes::put(out, at, &value.to_le_bytes())
        }
        8 => {
            let value = i64::try_from(value).map_err(|_| {
                Problem::internal(format!("a 64-bit field at {at:#x} cannot hold {value:#x}"))
            })?;
            bytes::put(out, at, &value.to_le_bytes())
        }
        _ => Err(Problem::internal(format!(
            "a field of {width} bytes at {at:#x}"
        ))),
    }
}

/// The bytes of `__unwind_info`, for functions whose last ends at `end`.
pub(crate) fn write_unwind_info<'a>(
    graph: &Graph<'a>,
    plan: &Plan<'a>,
    addresses: &dyn Addresses<'a>,
    image_base: u64,
    end: u64,
) -> Result<Vec<u8>> {
    let offset_of = |address: u64| -> Result<u32> {
        address
            .checked_sub(image_base)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| Problem::layout("an address of the unwind information beyond 4 GiB"))
    };
    let start_of = |start: Start| -> Result<u64> {
        match start {
            Start::Code(atom, offset) => addresses.address(Target::Atom { atom, offset }),
            Start::Stubs => Ok(addresses.stubs()),
        }
    };
    let _ = graph;
    let people = u32::try_from(plan.personalities.len()).unwrap_or(0);
    let pages: Vec<&[Entry<'a>]> = plan.entries.chunks(PAGE_ENTRIES).collect();
    let page_count = u32::try_from(pages.len()).unwrap_or(0);
    let mut w = Writer::default();
    let personality_offset = 28u32;
    let index_offset = personality_offset + 4 * people;
    let lsda_offset = index_offset + 12 * (page_count + 1);
    let lsda_count =
        u32::try_from(plan.entries.iter().filter(|e| e.lsda.is_some()).count()).unwrap_or(0);
    let pages_offset = lsda_offset + 8 * lsda_count;
    w.u32(1);
    w.u32(personality_offset);
    w.u32(0);
    w.u32(personality_offset);
    w.u32(people);
    w.u32(index_offset);
    w.u32(page_count + 1);
    for personality in &plan.personalities {
        w.u32(offset_of(addresses.got(*personality)?)?);
    }
    // The first-level index: one entry per page, then the sentinel at the end of the code.
    let mut page_at = pages_offset;
    let mut lsda_at = lsda_offset;
    for page in &pages {
        let first = page.first().map_or(Ok(end), |e| start_of(e.start))?;
        w.u32(offset_of(first)?);
        w.u32(page_at);
        w.u32(lsda_at);
        page_at += 8 + 8 * u32::try_from(page.len()).unwrap_or(0);
        lsda_at += 8 * u32::try_from(page.iter().filter(|e| e.lsda.is_some()).count()).unwrap_or(0);
    }
    w.u32(offset_of(end)?);
    w.u32(0);
    w.u32(lsda_at);
    // The LSDA index, in function order.
    for entry in &plan.entries {
        if let Some(lsda) = entry.lsda {
            w.u32(offset_of(start_of(entry.start)?)?);
            w.u32(offset_of(addresses.address(lsda)?)?);
        }
    }
    // The regular second-level pages.
    for page in &pages {
        w.u32(2);
        w.u16(8);
        w.u16(u16::try_from(page.len()).unwrap_or(0));
        for entry in *page {
            w.u32(offset_of(start_of(entry.start)?)?);
            w.u32(entry.encoding);
        }
    }
    if w.len() != plan.size {
        return Err(Problem::internal(
            "the unwind information does not have the size it was planned with",
        ));
    }
    Ok(w.bytes)
}
