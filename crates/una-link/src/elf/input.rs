//! The reader of ELF relocatable objects for the musl targets (ADR 0124 rules 22 to 24): a
//! 64-bit little-endian `ET_REL` object for x86-64 or arm64, read through the `object` crate's
//! ELF structures, with every field the linker uses checked here, so that what follows works on
//! sections, symbols and relocations that are known to be in bounds.
//!
//! Refused, as U8001 naming the input and the section or symbol: a header, section, symbol or
//! relocation the reader cannot read; a relocation section that the `object` crate would skip
//! (4a's rule, [`crate::elf_scan`]); `SHT_REL` or `SHT_CREL` relocations, two relocation
//! sections for one section, or a relocation of a loaded section that names no symbol; a
//! symbol outside its section (`value + size` past the section's end); a symbol of an unknown
//! binding, a local symbol after the first global, an undefined local; an indirect function
//! (`STT_GNU_IFUNC`); a thread-local symbol outside a thread-local section or the reverse; a
//! loaded section that is both writable and executable, compressed, of an unknown type, or
//! aligned beyond 64 KiB (U7015); a `.note.GNU-stack` that asks for an executable stack; a
//! COMDAT group that names a section twice or one outside the table.
//!
//! Every read is bounded by the input's bytes: the counts of sections, symbols and relocations
//! are those of tables that lie inside the input, so a forged count cannot make the reader loop
//! or allocate beyond the input's size. O(size of the input + R log R) for R relocations.

use std::collections::BTreeSet;

use object::LittleEndian;
use object::elf::FileHeader64;
use object::read::elf::{FileHeader, Rela as _, SectionHeader, SectionTable, Sym as _};

use crate::elf::{
    Arch, MAX_ALIGN, SHF_ALLOC, SHF_COMPRESSED, SHF_EXECINSTR, SHF_TLS, SHF_WRITE, SHT_CREL,
    SHT_FINI_ARRAY, SHT_GROUP, SHT_INIT_ARRAY, SHT_NOBITS, SHT_NOTE, SHT_PREINIT_ARRAY,
    SHT_PROGBITS, SHT_REL, SHT_RELA, SHT_SYMTAB, SHT_X86_64_UNWIND, STB_GLOBAL, STB_LOCAL,
    STB_WEAK, STT_GNU_IFUNC, STT_SECTION, STT_TLS,
};
use crate::problem::{Problem, Result, shown};

/// `EM_X86_64` and `EM_AARCH64`.
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;
/// `SHN_UNDEF`, `SHN_ABS` and `SHN_COMMON`.
const SHN_ABS: u16 = 0xfff1;
const SHN_COMMON: u16 = 0xfff2;
/// `GRP_COMDAT`.
const GRP_COMDAT: u32 = 1;

/// One relocation (`Elf64_Rela`), with its symbol index checked against the symbol table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rela {
    /// The place, as an offset into its section.
    pub(crate) offset: u64,
    /// `ELF64_R_TYPE`.
    pub(crate) kind: u32,
    /// `ELF64_R_SYM`, an index into [`ElfObject::symbols`] (0 for none).
    pub(crate) symbol: u32,
    /// `r_addend`.
    pub(crate) addend: i64,
}

/// One section of an object.
#[derive(Clone, Debug)]
pub(crate) struct Section<'a> {
    /// Its name.
    pub(crate) name: &'a [u8],
    /// `sh_type`.
    pub(crate) kind: u32,
    /// `sh_flags`.
    pub(crate) flags: u64,
    /// `sh_addralign`, 1 for 0; a power of two of at most 64 KiB for a loaded section.
    pub(crate) align: u64,
    /// `sh_size`.
    pub(crate) size: u64,
    /// Its bytes: empty for `SHT_NOBITS`, and for a section the executable does not contain
    /// (which is never read).
    pub(crate) data: &'a [u8],
    /// Its relocations, sorted by offset (stable); empty for a section that is not kept.
    pub(crate) relocs: Vec<Rela>,
}

impl Section<'_> {
    /// Whether the section is part of the executable's image: loaded (`SHF_ALLOC`) and not a
    /// note (rule 26: `.comment`, `.note.*` and `.debug_*` are dropped, and relocations in them
    /// are ignored, rule 24).
    pub(crate) fn kept(&self) -> bool {
        self.flags & SHF_ALLOC != 0 && self.kind != SHT_NOTE && !self.name.starts_with(b".note")
    }

    /// Whether the section is writable (`.data`, `.data.rel.ro`, `.got`, `.tdata`, ...): where a
    /// 64-bit absolute relocation may become a relative fixup.
    pub(crate) fn writable(&self) -> bool {
        self.flags & SHF_WRITE != 0
    }

    /// Whether the section holds thread-local data.
    pub(crate) fn tls(&self) -> bool {
        self.flags & SHF_TLS != 0
    }

    /// Whether the section is zero-fill (`.bss`, `.tbss`).
    pub(crate) fn zerofill(&self) -> bool {
        self.kind == SHT_NOBITS
    }
}

/// Where a symbol is defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// Undefined (`SHN_UNDEF`).
    Undefined,
    /// Absolute (`SHN_ABS`): its value does not move with the load.
    Absolute,
    /// Common (`SHN_COMMON`), with its alignment, a power of two.
    Common {
        /// `st_value`, the alignment.
        align: u64,
    },
    /// In the section of this index.
    Section(usize),
}

/// One symbol of an object.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Symbol<'a> {
    /// Its name (empty for most section symbols).
    pub(crate) name: &'a [u8],
    /// `STB_LOCAL`, `STB_GLOBAL` or `STB_WEAK`.
    pub(crate) bind: u8,
    /// `ELF64_ST_TYPE`.
    pub(crate) kind: u8,
    /// Where it is defined.
    pub(crate) place: Place,
    /// `st_value`: an offset into its section, an absolute value, or a common alignment.
    pub(crate) value: u64,
    /// `st_size`.
    pub(crate) size: u64,
}

impl Symbol<'_> {
    /// Whether the symbol holds thread-local data (`STT_TLS`, or the section symbol of a
    /// thread-local section, which [`read`] marks `STT_TLS` too).
    pub(crate) fn tls(&self) -> bool {
        self.kind == STT_TLS
    }

    /// Whether the symbol is weak.
    pub(crate) fn weak(&self) -> bool {
        self.bind == STB_WEAK
    }
}

/// A COMDAT group: its signature and its member sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Group<'a> {
    /// The name of the signature symbol.
    pub(crate) signature: &'a [u8],
    /// The indices of its member sections.
    pub(crate) members: Vec<usize>,
}

/// A relocatable object, as the linker sees it.
#[derive(Clone, Debug)]
pub(crate) struct ElfObject<'a> {
    /// Its name in messages (`libc.a(memcpy.lo)`).
    pub(crate) name: String,
    /// Its architecture.
    pub(crate) arch: Arch,
    /// Its sections, by ELF section index (index 0 is the null section).
    pub(crate) sections: Vec<Section<'a>>,
    /// Its symbols, by ELF symbol index (index 0 is the null symbol).
    pub(crate) symbols: Vec<Symbol<'a>>,
    /// Its COMDAT groups, in section order.
    pub(crate) groups: Vec<Group<'a>>,
}

impl<'a> ElfObject<'a> {
    /// The section of `index`.
    pub(crate) fn section(&self, index: usize) -> Result<&Section<'a>> {
        self.sections
            .get(index)
            .ok_or_else(|| Problem::internal(format!("`{}` has no section {index}", self.name)))
    }

    /// The symbol of `index`.
    pub(crate) fn symbol(&self, index: u32) -> Result<&Symbol<'a>> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.symbols.get(index))
            .ok_or_else(|| Problem::internal(format!("`{}` has no symbol {index}", self.name)))
    }

    /// The name of section `index` for messages.
    pub(crate) fn section_name(&self, index: usize) -> String {
        self.sections
            .get(index)
            .map_or_else(|| format!("section {index}"), |s| shown(s.name))
    }

    /// The name of symbol `index` for messages: its name, or its section's for a section symbol.
    pub(crate) fn symbol_name(&self, index: u32) -> String {
        match self.symbol(index) {
            Ok(symbol) if symbol.name.is_empty() => match symbol.place {
                Place::Section(section) => self.section_name(section),
                _ => format!("symbol {index}"),
            },
            Ok(symbol) => shown(symbol.name),
            Err(_) => format!("symbol {index}"),
        }
    }
}

type Header = FileHeader64<LittleEndian>;

/// Reads the relocatable object `data`, named `name` in messages.
///
/// # Errors
///
/// U8001 on an object the module's rules refuse; U7015 on a loaded section aligned beyond
/// 64 KiB.
pub(crate) fn read<'a>(name: &str, data: &'a [u8]) -> Result<ElfObject<'a>> {
    let bad = |what: String| Problem::internal(format!("cannot read `{name}`: {what}"));
    let header = Header::parse(data).map_err(|e| bad(e.to_string()))?;
    let endian = header.endian().map_err(|e| bad(e.to_string()))?;
    if header.e_type.get(endian) != object::elf::ET_REL {
        return Err(bad("not a relocatable object".to_owned()));
    }
    let arch = match header.e_machine.get(endian).0 {
        EM_X86_64 => Arch::X86_64,
        EM_AARCH64 => Arch::Aarch64,
        other => return Err(bad(format!("an object for machine {other}"))),
    };
    crate::elf_scan::check_relocation_sections(name, data)?;
    let table: SectionTable<'a, Header> = header
        .sections(endian, data)
        .map_err(|e| bad(e.to_string()))?;
    let mut sections = Vec::with_capacity(table.len());
    for (index, raw) in table.enumerate() {
        let section_name = if index.0 == 0 {
            &[][..]
        } else {
            table
                .section_name(endian, raw)
                .map_err(|e| bad(e.to_string()))?
        };
        let kind = raw.sh_type(endian).0;
        let flags = raw.sh_flags(endian).0;
        let size = raw.sh_size(endian);
        let align = raw.sh_addralign(endian).max(1);
        let mut section = Section {
            name: section_name,
            kind,
            flags,
            align,
            size,
            data: &[],
            relocs: Vec::new(),
        };
        if section_name == b".note.GNU-stack" && flags & SHF_EXECINSTR != 0 {
            return Err(bad(
                "`.note.GNU-stack` asks for an executable stack, which the executable never has"
                    .to_owned(),
            ));
        }
        if index.0 != 0 && flags & SHF_ALLOC != 0 {
            check_loaded(name, &section)?;
            if section.kept() && !section.zerofill() {
                section.data = raw.data(endian, data).map_err(|_| {
                    bad(format!(
                        "the bytes of `{}` lie outside the input",
                        shown(section_name)
                    ))
                })?;
                if u64::try_from(section.data.len()).ok() != Some(size) {
                    return Err(bad(format!(
                        "the size of `{}` is wrong",
                        shown(section_name)
                    )));
                }
            }
        }
        sections.push(section);
    }
    let symbols = read_symbols(name, data, endian, &table, &sections)?;
    let count = u32::try_from(symbols.len()).map_err(|_| bad("too many symbols".to_owned()))?;
    let mut relocated: BTreeSet<usize> = BTreeSet::new();
    let mut grouped: BTreeSet<usize> = BTreeSet::new();
    let mut groups = Vec::new();
    for (index, raw) in table.enumerate() {
        let kind = raw.sh_type(endian).0;
        let target = usize::try_from(raw.sh_info(endian)).unwrap_or(usize::MAX);
        if kind == SHT_GROUP {
            groups.push(read_group(
                name,
                data,
                (endian, raw, index.0),
                &symbols,
                sections.len(),
                &mut grouped,
            )?);
            continue;
        }
        if kind != SHT_RELA && kind != SHT_REL && kind != SHT_CREL {
            continue;
        }
        // `check_relocation_sections` has checked `sh_link` and `sh_info`.
        let Some(applies) = sections.get(target) else {
            return Err(bad(format!(
                "a relocation section applies to section {target}"
            )));
        };
        if !applies.kept() {
            // Relocations of sections the executable does not contain are ignored (rule 24).
            continue;
        }
        let applies_name = shown(applies.name);
        if kind != SHT_RELA {
            return Err(bad(format!(
                "`{applies_name}` has relocations that are not `SHT_RELA`, which neither target uses"
            )));
        }
        if !relocated.insert(target) {
            return Err(bad(format!("`{applies_name}` has two relocation sections")));
        }
        if raw.sh_entsize(endian) != 24 {
            return Err(bad(format!(
                "the relocations of `{applies_name}` do not have the size of `Elf64_Rela`"
            )));
        }
        let entries: &[object::elf::Rela64<LittleEndian>] =
            raw.data_as_array(endian, data).map_err(|_| {
                bad(format!(
                    "the relocations of `{applies_name}` lie outside the input"
                ))
            })?;
        let mut relocs = Vec::with_capacity(entries.len());
        for entry in entries {
            let symbol = entry.r_sym(endian, false);
            let rela = Rela {
                offset: entry.r_offset(endian),
                kind: entry.r_type(endian, false).0,
                symbol,
                addend: entry.r_addend(endian),
            };
            if symbol == 0 || symbol >= count {
                return Err(bad(format!(
                    "a relocation of `{applies_name}` at {:#x} names symbol {symbol}, which the table does not hold",
                    rela.offset
                )));
            }
            relocs.push(rela);
        }
        relocs.sort_by_key(|rela| rela.offset);
        if let Some(section) = sections.get_mut(target) {
            section.relocs = relocs;
        }
    }
    Ok(ElfObject {
        name: name.to_owned(),
        arch,
        sections,
        symbols,
        groups,
    })
}

/// The checks of a loaded section.
fn check_loaded(name: &str, section: &Section<'_>) -> Result<()> {
    let section_name = shown(section.name);
    let refused =
        |why: &str| Problem::internal(format!("`{name}` has a section `{section_name}` {why}"));
    match section.kind {
        SHT_PROGBITS | SHT_NOBITS | SHT_INIT_ARRAY | SHT_FINI_ARRAY | SHT_PREINIT_ARRAY
        | SHT_NOTE | SHT_X86_64_UNWIND => {}
        SHT_GROUP | SHT_SYMTAB | SHT_RELA | SHT_REL | SHT_CREL => {
            return Err(refused("of a type the executable cannot load"));
        }
        other => {
            return Err(refused(&format!(
                "of type {other:#x}, which the linker does not know"
            )));
        }
    }
    if section.flags & SHF_WRITE != 0 && section.flags & SHF_EXECINSTR != 0 {
        return Err(refused("that is both writable and executable"));
    }
    if section.flags & SHF_COMPRESSED != 0 {
        return Err(refused("that is compressed"));
    }
    if !section.align.is_power_of_two() {
        return Err(refused("whose alignment is not a power of two"));
    }
    if section.align > MAX_ALIGN {
        return Err(Problem::layout(format!(
            "`{name}` has a section `{section_name}` aligned to {:#x}, beyond the 64 KiB of a segment",
            section.align
        )));
    }
    Ok(())
}

/// Reads the symbol table, checked against the sections.
fn read_symbols<'a>(
    name: &str,
    data: &'a [u8],
    endian: LittleEndian,
    table: &SectionTable<'a, Header>,
    sections: &[Section<'a>],
) -> Result<Vec<Symbol<'a>>> {
    let bad = |what: String| Problem::internal(format!("cannot read `{name}`: {what}"));
    if table
        .iter()
        .filter(|s| s.sh_type(endian).0 == SHT_SYMTAB)
        .count()
        > 1
    {
        return Err(bad("two symbol tables".to_owned()));
    }
    let symtab = table
        .symbols(endian, data, object::elf::SHT_SYMTAB)
        .map_err(|e| bad(e.to_string()))?;
    let first_global = table
        .iter()
        .find(|s| s.sh_type(endian).0 == SHT_SYMTAB)
        .map_or(0, |s| {
            usize::try_from(s.sh_info(endian)).unwrap_or(usize::MAX)
        });
    let mut symbols = Vec::with_capacity(symtab.len());
    for (index, raw) in symtab.enumerate() {
        let symbol_name = symtab
            .symbol_name(endian, raw)
            .map_err(|e| bad(e.to_string()))?;
        let shown_name = shown(symbol_name);
        let refused =
            |why: &str| Problem::internal(format!("`{name}` has a symbol `{shown_name}` {why}"));
        if index.0 == 0 {
            symbols.push(Symbol {
                name: &[],
                bind: STB_LOCAL,
                kind: 0,
                place: Place::Undefined,
                value: 0,
                size: 0,
            });
            continue;
        }
        let bind = raw.st_bind().0;
        let mut kind = raw.st_type().0;
        if bind != STB_LOCAL && bind != STB_GLOBAL && bind != STB_WEAK {
            return Err(refused(&format!(
                "of binding {bind}, which the linker does not know"
            )));
        }
        if (bind == STB_LOCAL) != (index.0 < first_global) {
            return Err(refused(
                "out of place: the local symbols come first, as `sh_info` says",
            ));
        }
        if kind == STT_GNU_IFUNC {
            return Err(refused(
                "that is an indirect function (STT_GNU_IFUNC), which a static executable cannot have",
            ));
        }
        let shndx = raw.st_shndx(endian).0;
        let value = raw.st_value(endian);
        let size = raw.st_size(endian);
        let place = if shndx == SHN_ABS {
            Place::Absolute
        } else if shndx == SHN_COMMON {
            if !value.is_power_of_two() {
                return Err(refused(
                    "that is common with an alignment that is not a power of two",
                ));
            }
            if kind == STT_TLS {
                return Err(refused("that is a thread-local common symbol"));
            }
            Place::Common { align: value }
        } else {
            match symtab
                .symbol_section(endian, raw, index)
                .map_err(|e| bad(e.to_string()))?
            {
                None if shndx == 0 || shndx == object::elf::SHN_XINDEX.0 => Place::Undefined,
                None => return Err(refused(&format!("in the reserved section {shndx:#x}"))),
                Some(section) => Place::Section(section.0),
            }
        };
        if place == Place::Undefined && bind == STB_LOCAL {
            return Err(refused("that is local and undefined"));
        }
        if let Place::Section(index) = place {
            let Some(section) = sections.get(index).filter(|_| index != 0) else {
                return Err(refused(&format!(
                    "in section {index}, which the object does not have"
                )));
            };
            let end = value.checked_add(size);
            if end.is_none_or(|end| end > section.size) {
                return Err(refused(&format!(
                    "at {value:#x} of size {size:#x}, past the end of `{}` ({:#x} bytes)",
                    shown(section.name),
                    section.size
                )));
            }
            if section.flags & SHF_ALLOC != 0 {
                if kind == STT_SECTION && section.tls() {
                    kind = STT_TLS;
                }
                if (kind == STT_TLS) != section.tls() {
                    return Err(refused(&format!(
                        "whose type does not agree with its section `{}`: thread-local data only in a thread-local section",
                        shown(section.name)
                    )));
                }
            }
        }
        symbols.push(Symbol {
            name: symbol_name,
            bind,
            kind,
            place,
            value,
            size,
        });
    }
    Ok(symbols)
}

/// Reads a `SHT_GROUP` section.
fn read_group<'a>(
    name: &str,
    data: &'a [u8],
    (endian, raw, index): (
        LittleEndian,
        &object::elf::SectionHeader64<LittleEndian>,
        usize,
    ),
    symbols: &[Symbol<'a>],
    count: usize,
    grouped: &mut BTreeSet<usize>,
) -> Result<Group<'a>> {
    let bad = |what: String| Problem::internal(format!("cannot read `{name}`: {what}"));
    let (flags, members) = raw
        .group(endian, data)
        .map_err(|e| bad(e.to_string()))?
        .ok_or_else(|| bad("a group that is not one".to_owned()))?;
    if flags.0 != GRP_COMDAT {
        return Err(bad(format!("group section {index} is not a COMDAT group")));
    }
    let signature = usize::try_from(raw.sh_info(endian))
        .ok()
        .and_then(|i| symbols.get(i))
        .filter(|_| raw.sh_info(endian) != 0)
        .ok_or_else(|| bad(format!("group section {index} has no signature symbol")))?;
    let mut list = Vec::with_capacity(members.len());
    for member in members {
        let member = usize::try_from(member.get(endian)).unwrap_or(usize::MAX);
        if member == 0 || member >= count || member == index || !grouped.insert(member) {
            return Err(bad(format!(
                "group section {index} names section {member}, which is not a member it can have"
            )));
        }
        list.push(member);
    }
    Ok(Group {
        signature: signature.name,
        members: list,
    })
}
