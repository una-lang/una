//! The inputs (ADR 0124 rule 22): Mach-O relocatable objects for arm64, read through the
//! `object` crate, and `ar` archives of them with their symbol table, the index of which member
//! defines which symbol (rule 12).
//!
//! Reading checks the bounds of rule 24: every section's bytes lie in the file, every relocation's
//! place with its width lies inside its own section, and every defined symbol lies inside its
//! section. A relocation of a type outside the supported set is U8001 naming it.

use std::collections::BTreeMap;

use object::LittleEndian as LE;
use object::macho;
use object::read::archive::ArchiveFile;
use object::read::macho::{LoadCommandVariant, MachHeader, Nlist, Section, Segment};

use crate::problem::{Problem, Result, shown};

/// `ARM64_RELOC_UNSIGNED`.
pub(crate) const UNSIGNED: u8 = 0;
/// `ARM64_RELOC_SUBTRACTOR`.
pub(crate) const SUBTRACTOR: u8 = 1;
/// `ARM64_RELOC_BRANCH26`.
pub(crate) const BRANCH26: u8 = 2;
/// `ARM64_RELOC_PAGE21`.
pub(crate) const PAGE21: u8 = 3;
/// `ARM64_RELOC_PAGEOFF12`.
pub(crate) const PAGEOFF12: u8 = 4;
/// `ARM64_RELOC_GOT_LOAD_PAGE21`.
pub(crate) const GOT_LOAD_PAGE21: u8 = 5;
/// `ARM64_RELOC_GOT_LOAD_PAGEOFF12`.
pub(crate) const GOT_LOAD_PAGEOFF12: u8 = 6;
/// `ARM64_RELOC_POINTER_TO_GOT`.
pub(crate) const POINTER_TO_GOT: u8 = 7;
/// `ARM64_RELOC_TLVP_LOAD_PAGE21`.
pub(crate) const TLVP_LOAD_PAGE21: u8 = 8;
/// `ARM64_RELOC_TLVP_LOAD_PAGEOFF12`.
pub(crate) const TLVP_LOAD_PAGEOFF12: u8 = 9;
/// `ARM64_RELOC_ADDEND`.
pub(crate) const ADDEND: u8 = 10;

/// The name of a relocation type, for messages.
pub(crate) fn reloc_name(kind: u8) -> String {
    let name = match kind {
        UNSIGNED => "ARM64_RELOC_UNSIGNED",
        SUBTRACTOR => "ARM64_RELOC_SUBTRACTOR",
        BRANCH26 => "ARM64_RELOC_BRANCH26",
        PAGE21 => "ARM64_RELOC_PAGE21",
        PAGEOFF12 => "ARM64_RELOC_PAGEOFF12",
        GOT_LOAD_PAGE21 => "ARM64_RELOC_GOT_LOAD_PAGE21",
        GOT_LOAD_PAGEOFF12 => "ARM64_RELOC_GOT_LOAD_PAGEOFF12",
        POINTER_TO_GOT => "ARM64_RELOC_POINTER_TO_GOT",
        TLVP_LOAD_PAGE21 => "ARM64_RELOC_TLVP_LOAD_PAGE21",
        TLVP_LOAD_PAGEOFF12 => "ARM64_RELOC_TLVP_LOAD_PAGEOFF12",
        ADDEND => "ARM64_RELOC_ADDEND",
        11 => "ARM64_RELOC_AUTHENTICATED_POINTER",
        other => return format!("relocation type {other}"),
    };
    name.to_owned()
}

/// Section types and attributes of `mach-o/loader.h`.
pub(crate) mod flags {
    pub(crate) const SECTION_TYPE: u32 = 0xff;
    pub(crate) const S_REGULAR: u32 = 0x0;
    pub(crate) const S_ZEROFILL: u32 = 0x1;
    pub(crate) const S_CSTRING_LITERALS: u32 = 0x2;
    pub(crate) const S_4BYTE_LITERALS: u32 = 0x3;
    pub(crate) const S_8BYTE_LITERALS: u32 = 0x4;
    pub(crate) const S_NON_LAZY_SYMBOL_POINTERS: u32 = 0x6;
    pub(crate) const S_SYMBOL_STUBS: u32 = 0x8;
    pub(crate) const S_MOD_INIT_FUNC_POINTERS: u32 = 0x9;
    pub(crate) const S_COALESCED: u32 = 0xb;
    pub(crate) const S_GB_ZEROFILL: u32 = 0xc;
    pub(crate) const S_16BYTE_LITERALS: u32 = 0xe;
    pub(crate) const S_THREAD_LOCAL_REGULAR: u32 = 0x11;
    pub(crate) const S_THREAD_LOCAL_ZEROFILL: u32 = 0x12;
    pub(crate) const S_THREAD_LOCAL_VARIABLES: u32 = 0x13;
    pub(crate) const S_ATTR_PURE_INSTRUCTIONS: u32 = 0x8000_0000;
    pub(crate) const S_ATTR_NO_TOC_EH: u32 = 0x4000_0000;
    pub(crate) const S_ATTR_STRIP_STATIC_SYMS: u32 = 0x2000_0000;
    pub(crate) const S_ATTR_NO_DEAD_STRIP: u32 = 0x1000_0000;
    pub(crate) const S_ATTR_LIVE_SUPPORT: u32 = 0x0800_0000;
    pub(crate) const S_ATTR_DEBUG: u32 = 0x0200_0000;
    pub(crate) const S_ATTR_SOME_INSTRUCTIONS: u32 = 0x400;
}

/// Symbol types and descriptions of `mach-o/nlist.h`.
pub(crate) mod nlist {
    pub(crate) const N_STAB: u8 = 0xe0;
    pub(crate) const N_PEXT: u8 = 0x10;
    pub(crate) const N_TYPE: u8 = 0x0e;
    pub(crate) const N_EXT: u8 = 0x01;
    pub(crate) const N_UNDF: u8 = 0x0;
    pub(crate) const N_SECT: u8 = 0xe;
    pub(crate) const N_NO_DEAD_STRIP: u16 = 0x20;
    pub(crate) const N_WEAK_REF: u16 = 0x40;
    pub(crate) const N_WEAK_DEF: u16 = 0x80;
    pub(crate) const N_ALT_ENTRY: u16 = 0x200;
}

/// What a relocation refers to, as its input writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Referent {
    /// `r_extern` 1: an index into the object's symbol table.
    Symbol(u32),
    /// `r_extern` 0: a section ordinal (1-based); the address is in the place's bytes.
    Section(u8),
}

/// One relocation of an input section, with its `ARM64_RELOC_ADDEND` and the
/// `ARM64_RELOC_SUBTRACTOR` of a pair folded in.
#[derive(Clone, Debug)]
pub(crate) struct Reloc {
    /// The place: an offset into its section.
    pub(crate) offset: u64,
    /// The `ARM64_RELOC_*` type.
    pub(crate) kind: u8,
    /// `r_pcrel`.
    pub(crate) pcrel: bool,
    /// The width of the place in bytes (1 << `r_length`).
    pub(crate) width: u64,
    /// What it refers to.
    pub(crate) referent: Referent,
    /// The addend of a preceding `ARM64_RELOC_ADDEND`, else 0.
    pub(crate) addend: i64,
    /// For an `ARM64_RELOC_UNSIGNED` paired with an `ARM64_RELOC_SUBTRACTOR`, what is subtracted.
    pub(crate) minus: Option<Referent>,
}

/// An input section.
#[derive(Clone, Debug)]
pub(crate) struct InSection<'a> {
    /// The segment name.
    pub(crate) segment: String,
    /// The section name.
    pub(crate) name: String,
    /// The section's address in the object.
    pub(crate) addr: u64,
    /// Its size.
    pub(crate) size: u64,
    /// Its alignment, as a power of two.
    pub(crate) align: u32,
    /// Its type and attributes.
    pub(crate) flags: u32,
    /// Its bytes; empty for a zero-fill section.
    pub(crate) data: &'a [u8],
    /// Its relocations, in the order of the input.
    pub(crate) relocs: Vec<Reloc>,
}

impl InSection<'_> {
    /// The section type.
    pub(crate) fn kind(&self) -> u32 {
        self.flags & flags::SECTION_TYPE
    }

    /// Whether the section has no bytes in the file.
    pub(crate) fn zerofill(&self) -> bool {
        matches!(
            self.kind(),
            flags::S_ZEROFILL | flags::S_GB_ZEROFILL | flags::S_THREAD_LOCAL_ZEROFILL
        )
    }

    /// Whether the section is debug information, which the output does not contain (rule 26).
    pub(crate) fn debug(&self) -> bool {
        self.flags & flags::S_ATTR_DEBUG != 0 && self.segment != "__LD" || self.segment == "__DWARF"
    }
}

/// An input symbol.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InSymbol<'a> {
    /// Its name.
    pub(crate) name: &'a [u8],
    /// `n_type`.
    pub(crate) n_type: u8,
    /// `n_sect` (1-based; 0 for none).
    pub(crate) n_sect: u8,
    /// `n_desc`.
    pub(crate) n_desc: u16,
    /// `n_value`: an address in the object.
    pub(crate) n_value: u64,
}

impl InSymbol<'_> {
    /// Whether it is a debugging symbol.
    pub(crate) fn stab(&self) -> bool {
        self.n_type & nlist::N_STAB != 0
    }
    /// Whether it is external.
    pub(crate) fn external(&self) -> bool {
        !self.stab() && self.n_type & nlist::N_EXT != 0
    }
    /// Whether it is defined in a section of its object.
    pub(crate) fn in_section(&self) -> bool {
        !self.stab() && self.n_type & nlist::N_TYPE == nlist::N_SECT
    }
    /// Whether it is undefined.
    pub(crate) fn undefined(&self) -> bool {
        !self.stab() && self.n_type & nlist::N_TYPE == nlist::N_UNDF
    }
    /// Whether it is a weak definition.
    pub(crate) fn weak(&self) -> bool {
        self.n_desc & nlist::N_WEAK_DEF != 0
    }
}

/// A relocatable arm64 Mach-O object.
#[derive(Clone, Debug)]
pub(crate) struct Object<'a> {
    /// Its name in messages: `the program's object` or `libuna_rt.a(<member>)`.
    pub(crate) name: String,
    /// Whether it may be split at its symbols (`MH_SUBSECTIONS_VIA_SYMBOLS`).
    pub(crate) subsections: bool,
    /// Its sections; section ordinal `n` is index `n - 1`.
    pub(crate) sections: Vec<InSection<'a>>,
    /// Its symbol table.
    pub(crate) symbols: Vec<InSymbol<'a>>,
}

const MH_SUBSECTIONS_VIA_SYMBOLS: u32 = 0x2000;

/// Reads a relocatable arm64 Mach-O object.
///
/// # Errors
///
/// U8001 when the bytes are not such an object, a section or its relocations lie outside the
/// file, a relocation's place lies outside its section, a symbol lies outside its section, or a
/// relocation is of a type this linker does not support.
pub(crate) fn read_object<'a>(name: &str, data: &'a [u8]) -> Result<Object<'a>> {
    let bad = |what: &str| Problem::internal(format!("cannot read `{name}`: {what}"));
    let header = macho::MachHeader64::<LE>::parse(data, 0)
        .map_err(|error| bad(&format!("not a 64-bit Mach-O object ({error})")))?;
    let endian = LE;
    if !header.is_type_64() || !header.is_little_endian() {
        return Err(bad("not a little-endian 64-bit Mach-O object"));
    }
    if header.cputype(endian) != macho::CPU_TYPE_ARM64 {
        return Err(bad("not an arm64 object"));
    }
    if header.filetype(endian) != macho::MH_OBJECT {
        return Err(bad("not a relocatable object (MH_OBJECT)"));
    }
    let subsections = header.flags(endian).0 & MH_SUBSECTIONS_VIA_SYMBOLS != 0;
    let mut sections = Vec::new();
    let mut symbols = Vec::new();
    let mut commands = header
        .load_commands(endian, data, 0)
        .map_err(|error| bad(&format!("its load commands ({error})")))?;
    while let Some(command) = commands
        .next()
        .map_err(|error| bad(&format!("its load commands ({error})")))?
    {
        match command
            .variant()
            .map_err(|error| bad(&format!("a load command ({error})")))?
        {
            LoadCommandVariant::Segment64(segment, section_data) => {
                let raw = segment
                    .sections(endian, section_data)
                    .map_err(|error| bad(&format!("its sections ({error})")))?;
                for section in raw {
                    sections.push(read_section(name, data, section)?);
                }
            }
            LoadCommandVariant::Symtab(symtab) => {
                let table = symtab
                    .symbols::<macho::MachHeader64<LE>, _>(endian, data)
                    .map_err(|error| bad(&format!("its symbol table ({error})")))?;
                let strings = table.strings();
                for symbol in table.iter() {
                    let symbol_name = symbol
                        .name(endian, strings)
                        .map_err(|error| bad(&format!("a symbol's name ({error})")))?;
                    symbols.push(InSymbol {
                        name: symbol_name,
                        n_type: symbol.n_type().0,
                        n_sect: symbol.n_sect(),
                        n_desc: symbol.n_desc(endian).0,
                        n_value: symbol.n_value(endian),
                    });
                }
            }
            _ => {}
        }
    }
    let object = Object {
        name: name.to_owned(),
        subsections,
        sections,
        symbols,
    };
    check_object(&object)?;
    Ok(object)
}

fn read_section<'a>(
    object: &str,
    data: &'a [u8],
    section: &macho::Section64<LE>,
) -> Result<InSection<'a>> {
    let endian = LE;
    let segment = shown(section.segment_name());
    let name = shown(section.name());
    let flags = section.flags(endian).0;
    let size = section.size(endian);
    let kind = flags & flags::SECTION_TYPE;
    let zerofill = matches!(
        kind,
        flags::S_ZEROFILL | flags::S_GB_ZEROFILL | flags::S_THREAD_LOCAL_ZEROFILL
    );
    let bad =
        |what: String| Problem::internal(format!("`{object}`, section {segment},{name}: {what}"));
    // No input section of Una's own inputs comes near 4 GiB; a larger size, which a zero-fill
    // section could state without bytes, is refused before any address is computed from it.
    if size > u64::from(u32::MAX) {
        return Err(bad(format!(
            "a size of {size:#x}, beyond the 4 GiB the linker accepts"
        )));
    }
    let section_addr = section.addr(endian);
    if section_addr
        .checked_add(size)
        .is_none_or(|end| end > 1u64 << 48)
    {
        return Err(bad(format!(
            "an address range at {section_addr:#x} beyond the linker's reach"
        )));
    }
    let bytes = if zerofill {
        &[][..]
    } else {
        let start = u64::from(section.offset(endian));
        let end = start
            .checked_add(size)
            .ok_or_else(|| bad("its size overflows".to_owned()))?;
        data.get(
            usize::try_from(start).unwrap_or(usize::MAX)
                ..usize::try_from(end).unwrap_or(usize::MAX),
        )
        .ok_or_else(|| bad("its bytes lie outside the file".to_owned()))?
    };
    let raw = section
        .relocations(endian, data)
        .map_err(|error| bad(format!("its relocations ({error})")))?;
    let mut relocs = Vec::with_capacity(raw.len());
    let mut addend: Option<(u64, i64)> = None;
    let mut minus: Option<(u64, u64, Referent)> = None;
    for relocation in raw {
        if relocation.r_scattered(endian, macho::CPU_TYPE_ARM64) {
            return Err(bad(
                "a scattered relocation, which arm64 does not use".to_owned()
            ));
        }
        let info = relocation.info(endian);
        let r_type: u8 = info.r_type.0;
        let offset = u64::from(info.r_address);
        let width = 1u64 << info.r_length;
        if r_type == ADDEND {
            // A 24-bit signed addend in r_symbolnum, for the next relocation at its place.
            let raw = i64::from(info.r_symbolnum);
            let value = if raw & 0x80_0000 != 0 {
                raw - 0x100_0000
            } else {
                raw
            };
            addend = Some((offset, value));
            continue;
        }
        let referent = if info.r_extern {
            Referent::Symbol(info.r_symbolnum)
        } else {
            Referent::Section(u8::try_from(info.r_symbolnum).map_err(|_| {
                bad(format!(
                    "a relocation at {offset:#x} names section {}",
                    info.r_symbolnum
                ))
            })?)
        };
        match r_type {
            SUBTRACTOR => {
                if !info.r_extern {
                    return Err(bad(format!(
                        "an ARM64_RELOC_SUBTRACTOR at {offset:#x} that names no symbol"
                    )));
                }
                minus = Some((offset, width, referent));
                continue;
            }
            UNSIGNED | BRANCH26 | PAGE21 | PAGEOFF12 | GOT_LOAD_PAGE21 | GOT_LOAD_PAGEOFF12
            | POINTER_TO_GOT | TLVP_LOAD_PAGE21 | TLVP_LOAD_PAGEOFF12 => {}
            other => {
                return Err(bad(format!(
                    "{} at {offset:#x} is not supported by the internal linker",
                    reloc_name(other)
                )));
            }
        }
        let taken_addend = match addend.take() {
            Some((at, value)) if at == offset => value,
            Some(_) => {
                return Err(bad(format!(
                    "an ARM64_RELOC_ADDEND that does not precede a relocation at its place, before {offset:#x}"
                )));
            }
            None => 0,
        };
        let taken_minus = match minus.take() {
            Some((at, minus_width, referent)) => {
                if at != offset || minus_width != width || r_type != UNSIGNED {
                    return Err(bad(format!(
                        "an ARM64_RELOC_SUBTRACTOR at {at:#x} not followed by an ARM64_RELOC_UNSIGNED of its place and width"
                    )));
                }
                Some(referent)
            }
            None => None,
        };
        relocs.push(Reloc {
            offset,
            kind: r_type,
            pcrel: info.r_pcrel,
            width,
            referent,
            addend: taken_addend,
            minus: taken_minus,
        });
    }
    if addend.is_some() || minus.is_some() {
        return Err(bad(
            "a relocation list ends with an unpaired ARM64_RELOC_ADDEND or ARM64_RELOC_SUBTRACTOR"
                .to_owned(),
        ));
    }
    // Relocations are applied and looked up by place: sorted by offset, stably, so the input's
    // order (usually descending) never matters.
    relocs.sort_by_key(|reloc| reloc.offset);
    // Two relocations at one place would patch it twice: refused, as no assembler writes them.
    if let Some(pair) = relocs
        .windows(2)
        .find(|pair| matches!(pair, [a, b] if a.offset == b.offset))
    {
        let at = pair.first().map_or(0, |r| r.offset);
        return Err(bad(format!("two relocations at the place {at:#x}")));
    }
    Ok(InSection {
        segment,
        name,
        addr: section.addr(endian),
        size,
        align: section.align(endian),
        flags,
        data: bytes,
        relocs,
    })
}

/// The bounds of rule 24 on one object.
fn check_object(object: &Object<'_>) -> Result<()> {
    let count = object.sections.len();
    for section in &object.sections {
        if section.align > 15 {
            return Err(Problem::internal(format!(
                "`{}`, section {},{}: an alignment of 2^{} is larger than the linker allows",
                object.name, section.segment, section.name, section.align
            )));
        }
        for reloc in &section.relocs {
            let end = reloc.offset.checked_add(reloc.width);
            if end.is_none_or(|end| end > section.size) {
                return Err(Problem::internal(format!(
                    "`{}`, section {},{}: the place of {} at {:#x} lies outside its section",
                    object.name,
                    section.segment,
                    section.name,
                    reloc_name(reloc.kind),
                    reloc.offset
                )));
            }
            for referent in std::iter::once(reloc.referent).chain(reloc.minus) {
                let fine = match referent {
                    Referent::Symbol(index) => {
                        usize::try_from(index).is_ok_and(|i| i < object.symbols.len())
                    }
                    Referent::Section(ordinal) => ordinal >= 1 && usize::from(ordinal) <= count,
                };
                if !fine {
                    return Err(Problem::internal(format!(
                        "`{}`, section {},{}: {} at {:#x} refers to no symbol or section",
                        object.name,
                        section.segment,
                        section.name,
                        reloc_name(reloc.kind),
                        reloc.offset
                    )));
                }
            }
        }
    }
    // Input sections never overlap in the object's addresses (rule 24): a target found by its
    // address must belong to one section.
    let mut spans: Vec<(u64, u64, &InSection<'_>)> = object
        .sections
        .iter()
        .filter(|s| s.size > 0)
        .map(|s| (s.addr, s.addr.saturating_add(s.size), s))
        .collect();
    spans.sort_by_key(|span| span.0);
    for pair in spans.windows(2) {
        if let [(_, end, first), (start, _, second)] = pair
            && start < end
        {
            return Err(Problem::internal(format!(
                "`{}`: the sections {},{} and {},{} overlap",
                object.name, first.segment, first.name, second.segment, second.name
            )));
        }
    }
    for symbol in &object.symbols {
        if !symbol.in_section() {
            continue;
        }
        let section = usize::from(symbol.n_sect)
            .checked_sub(1)
            .and_then(|index| object.sections.get(index))
            .ok_or_else(|| {
                Problem::internal(format!(
                    "`{}`: the symbol `{}` names section {}, which does not exist",
                    object.name,
                    shown(symbol.name),
                    symbol.n_sect
                ))
            })?;
        let end = section.addr.saturating_add(section.size);
        if symbol.n_value < section.addr || symbol.n_value > end {
            return Err(Problem::internal(format!(
                "`{}`: the symbol `{}` lies outside its section {},{}",
                object.name,
                shown(symbol.name),
                section.segment,
                section.name
            )));
        }
    }
    Ok(())
}

/// An `ar` archive of objects with its symbol table (rule 12: a member is found by the symbol it
/// defines, without reading the others).
pub(crate) struct Archive<'a> {
    /// The archive's name in messages.
    pub(crate) name: String,
    data: &'a [u8],
    file: ArchiveFile<'a>,
    /// Each symbol of the archive's symbol table and the member offset of its first definition,
    /// in archive order.
    index: BTreeMap<&'a [u8], u64>,
}

impl<'a> Archive<'a> {
    /// Reads an archive and its symbol table.
    ///
    /// # Errors
    ///
    /// U8001 when it is not an archive, is a thin archive, or has no symbol table.
    pub(crate) fn read(name: &str, data: &'a [u8]) -> Result<Archive<'a>> {
        let bad = |what: String| Problem::internal(format!("cannot read `{name}`: {what}"));
        let file =
            ArchiveFile::parse(data).map_err(|error| bad(format!("not an archive ({error})")))?;
        if file.is_thin() {
            return Err(bad("a thin archive".to_owned()));
        }
        let symbols = file
            .symbols()
            .map_err(|error| bad(format!("its symbol table ({error})")))?
            .ok_or_else(|| bad("it has no symbol table".to_owned()))?;
        let mut index: BTreeMap<&'a [u8], u64> = BTreeMap::new();
        for symbol in symbols {
            let symbol = symbol.map_err(|error| bad(format!("its symbol table ({error})")))?;
            let offset = symbol.offset().0;
            index
                .entry(symbol.name())
                .and_modify(|first| *first = (*first).min(offset))
                .or_insert(offset);
        }
        Ok(Archive {
            name: name.to_owned(),
            data,
            file,
            index,
        })
    }

    /// Each symbol of the archive's symbol table and the member offset of its first definition,
    /// in the symbols' byte order.
    pub(crate) fn index(&self) -> impl Iterator<Item = (&'a [u8], u64)> + '_ {
        self.index.iter().map(|(name, offset)| (*name, *offset))
    }

    /// The member offset that the symbol table names for `symbol`.
    pub(crate) fn member_of(&self, symbol: &[u8]) -> Option<u64> {
        self.index.get(symbol).copied()
    }

    /// The member at `offset`: its name and bytes.
    ///
    /// # Errors
    ///
    /// U8001 when the offset names no member.
    pub(crate) fn member(&self, offset: u64) -> Result<(String, &'a [u8])> {
        let member = self
            .file
            .member(object::read::archive::ArchiveOffset(offset))
            .map_err(|error| {
                Problem::internal(format!("cannot read a member of `{}`: {error}", self.name))
            })?;
        let bytes = member.data(self.data).map_err(|error| {
            Problem::internal(format!("cannot read a member of `{}`: {error}", self.name))
        })?;
        Ok((format!("{}({})", self.name, shown(member.name())), bytes))
    }

    /// Every member's offset, name and bytes, in archive order.
    ///
    /// # Errors
    ///
    /// U8001 when a member cannot be read.
    pub(crate) fn members(&self) -> Result<Vec<(u64, String, &'a [u8])>> {
        let mut out = Vec::new();
        for member in self.file.members() {
            let member = member.map_err(|error| {
                Problem::internal(format!("cannot read a member of `{}`: {error}", self.name))
            })?;
            let bytes = member.data(self.data).map_err(|error| {
                Problem::internal(format!("cannot read a member of `{}`: {error}", self.name))
            })?;
            let (start, _) = member.file_range();
            out.push((
                start,
                format!("{}({})", self.name, shown(member.name())),
                bytes,
            ));
        }
        Ok(out)
    }
}
