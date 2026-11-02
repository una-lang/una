//! The scan of the ELF inputs of a Linux executable (ADR 0124 rules 12, 23 and 24, step 4a,
//! `docs/plan/0124-step4-design.md` §2 to §4): musl's `libc.a`, its start-up objects,
//! `libunwind.a` and the Rust standard library of the musl targets, read through the `object`
//! crate before the ELF linker of step 4 links any of them.
//!
//! It counts every relocation by type, by the kind of section it is in and by its referent, and
//! refuses, as U8001 naming the input, the section and the symbol:
//!
//! - an `R_*_IRELATIVE` relocation or an indirect function (`STT_GNU_IFUNC`) anywhere (rule 24,
//!   "never supported"; design decision D2);
//! - in a section the executable loads (`SHF_ALLOC`), a relocation type outside rule 24's list
//!   for the object's architecture, a narrow absolute relocation against a symbol that is not
//!   absolute, and a 64-bit absolute one in a section that is neither writable nor thread-local
//!   data (a text relocation). Relocations in sections that are not loaded (`.debug_*`,
//!   `.comment`) are counted and never refused.
//!
//! It lists the common symbols, which the standard library must not define (design note §3), and
//! reads musl's version from `__libc_version`, which decides whether its start-up applies
//! `DT_RELR` (rule 25, probe P2). It links nothing and writes nothing.

use std::collections::{BTreeMap, BTreeSet};

use object::read::{Object, ObjectSection, ObjectSymbol, RelocationTarget};
use object::{
    Architecture, BinaryFormat, ObjectKind, RelocationFlags, SectionFlags, SymbolFlags,
    SymbolSection,
};

use crate::input::Archive;
use crate::problem::{Problem, Result, shown};

/// What the scan found in its inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElfScan {
    /// Every relocation, by `TYPE loaded|unloaded KIND REFERENT`, with its count.
    pub counts: BTreeMap<String, u64>,
    /// Each common symbol, as `input: symbol`.
    pub common: BTreeSet<String>,
    /// musl's version, from the `__libc_version` of `libc.a`, when an input defines it.
    pub musl_version: Option<String>,
    /// How many entries of the archives' symbol tables were checked against their members.
    pub indexed: u64,
}

impl ElfScan {
    /// Adds what `other` found to this scan.
    pub fn merge(&mut self, other: ElfScan) {
        for (key, count) in other.counts {
            *self.counts.entry(key).or_insert(0) += count;
        }
        self.common.extend(other.common);
        if self.musl_version.is_none() {
            self.musl_version = other.musl_version;
        }
        self.indexed += other.indexed;
    }
}

/// Scans `bytes`, an ELF relocatable object for x86-64 or arm64 or an `ar` archive of them,
/// named `name` in messages. An archive's `lib.rmeta` member (an rlib's metadata) is skipped.
/// O(size of the input).
///
/// # Errors
///
/// U8001 when an input cannot be read or holds what the module's rules refuse.
pub fn scan_elf(name: &str, bytes: &[u8]) -> Result<ElfScan> {
    let mut scan = ElfScan::default();
    if bytes.starts_with(b"!<arch>\n") {
        let archive = Archive::read(name, bytes)?;
        for (_, member, data) in archive.members()? {
            if member.ends_with("(lib.rmeta)") {
                continue;
            }
            scan.merge(scan_object(&member, data)?);
        }
        scan.indexed = check_index(&archive)?;
    } else {
        scan.merge(scan_object(name, bytes)?);
    }
    Ok(scan)
}

/// Checks the archive's symbol table, the index by which the linker finds the member that
/// defines a symbol without reading the others (rule 12): every entry must name a member that
/// defines that symbol (as a global, weak or common symbol), and returns how many entries there
/// are. Each member named is parsed once. O(entries × log + size of the members named).
fn check_index(archive: &Archive<'_>) -> Result<u64> {
    let mut defined: BTreeMap<u64, (String, BTreeSet<Vec<u8>>)> = BTreeMap::new();
    let mut count = 0u64;
    for (symbol, offset) in archive.index() {
        let (member, names) = match defined.entry(offset) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let (member, data) = archive.member(offset)?;
                let file = object::File::parse(data).map_err(|error| {
                    Problem::internal(format!("cannot read `{member}`: {error}"))
                })?;
                let names = file
                    .symbols()
                    .filter(|symbol| !symbol.is_undefined() && !symbol.is_local())
                    .filter_map(|symbol| symbol.name_bytes().ok().map(<[u8]>::to_vec))
                    .collect();
                entry.insert((member, names))
            }
        };
        if !names.contains(symbol) {
            return Err(Problem::internal(format!(
                "the symbol table of `{}` names `{member}` for `{}`, which that member does not define",
                archive.name,
                shown(symbol)
            )));
        }
        count += 1;
    }
    Ok(count)
}

/// The architectures of the musl targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arch {
    X86_64,
    Aarch64,
}

/// What a relocation type is, for the rules of rule 24.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// Supported, with no restriction of its own.
    Plain,
    /// A narrow absolute type: only against an absolute symbol or outside loaded sections.
    Narrow,
    /// A 64-bit absolute type: only in a writable section, where it becomes a relative fixup.
    Wide,
}

/// Rule 24's list for x86-64: (type, name, class).
const X86_64: &[(u32, &str, Class)] = &[
    (1, "R_X86_64_64", Class::Wide),
    (2, "R_X86_64_PC32", Class::Plain),
    (4, "R_X86_64_PLT32", Class::Plain),
    (9, "R_X86_64_GOTPCREL", Class::Plain),
    (10, "R_X86_64_32", Class::Narrow),
    (11, "R_X86_64_32S", Class::Narrow),
    (19, "R_X86_64_TLSGD", Class::Plain),
    (20, "R_X86_64_TLSLD", Class::Plain),
    (21, "R_X86_64_DTPOFF32", Class::Plain),
    (22, "R_X86_64_GOTTPOFF", Class::Plain),
    (23, "R_X86_64_TPOFF32", Class::Plain),
    (24, "R_X86_64_PC64", Class::Plain),
    (41, "R_X86_64_GOTPCRELX", Class::Plain),
    (42, "R_X86_64_REX_GOTPCRELX", Class::Plain),
];

/// Rule 24's list for arm64: (type, name, class). `TLSLE_*`, `TLSIE_*` and `TLSDESC_*` are the
/// types of those families that the ABI defines for the local-exec model and its relaxations.
const AARCH64: &[(u32, &str, Class)] = &[
    (257, "R_AARCH64_ABS64", Class::Wide),
    (258, "R_AARCH64_ABS32", Class::Narrow),
    (260, "R_AARCH64_PREL64", Class::Plain),
    (261, "R_AARCH64_PREL32", Class::Plain),
    (263, "R_AARCH64_MOVW_UABS_G0", Class::Narrow),
    (264, "R_AARCH64_MOVW_UABS_G0_NC", Class::Narrow),
    (265, "R_AARCH64_MOVW_UABS_G1", Class::Narrow),
    (266, "R_AARCH64_MOVW_UABS_G1_NC", Class::Narrow),
    (267, "R_AARCH64_MOVW_UABS_G2", Class::Narrow),
    (268, "R_AARCH64_MOVW_UABS_G2_NC", Class::Narrow),
    (269, "R_AARCH64_MOVW_UABS_G3", Class::Narrow),
    (274, "R_AARCH64_ADR_PREL_LO21", Class::Plain),
    (275, "R_AARCH64_ADR_PREL_PG_HI21", Class::Plain),
    (277, "R_AARCH64_ADD_ABS_LO12_NC", Class::Plain),
    (278, "R_AARCH64_LDST8_ABS_LO12_NC", Class::Plain),
    (279, "R_AARCH64_TSTBR14", Class::Plain),
    (280, "R_AARCH64_CONDBR19", Class::Plain),
    (282, "R_AARCH64_JUMP26", Class::Plain),
    (283, "R_AARCH64_CALL26", Class::Plain),
    (284, "R_AARCH64_LDST16_ABS_LO12_NC", Class::Plain),
    (285, "R_AARCH64_LDST32_ABS_LO12_NC", Class::Plain),
    (286, "R_AARCH64_LDST64_ABS_LO12_NC", Class::Plain),
    (299, "R_AARCH64_LDST128_ABS_LO12_NC", Class::Plain),
    (311, "R_AARCH64_ADR_GOT_PAGE", Class::Plain),
    (312, "R_AARCH64_LD64_GOT_LO12_NC", Class::Plain),
    (541, "R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21", Class::Plain),
    (542, "R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC", Class::Plain),
    (544, "R_AARCH64_TLSLE_MOVW_TPREL_G2", Class::Plain),
    (545, "R_AARCH64_TLSLE_MOVW_TPREL_G1", Class::Plain),
    (546, "R_AARCH64_TLSLE_MOVW_TPREL_G1_NC", Class::Plain),
    (547, "R_AARCH64_TLSLE_MOVW_TPREL_G0", Class::Plain),
    (548, "R_AARCH64_TLSLE_MOVW_TPREL_G0_NC", Class::Plain),
    (549, "R_AARCH64_TLSLE_ADD_TPREL_HI12", Class::Plain),
    (550, "R_AARCH64_TLSLE_ADD_TPREL_LO12", Class::Plain),
    (551, "R_AARCH64_TLSLE_ADD_TPREL_LO12_NC", Class::Plain),
    (552, "R_AARCH64_TLSLE_LDST8_TPREL_LO12", Class::Plain),
    (553, "R_AARCH64_TLSLE_LDST8_TPREL_LO12_NC", Class::Plain),
    (554, "R_AARCH64_TLSLE_LDST16_TPREL_LO12", Class::Plain),
    (555, "R_AARCH64_TLSLE_LDST16_TPREL_LO12_NC", Class::Plain),
    (556, "R_AARCH64_TLSLE_LDST32_TPREL_LO12", Class::Plain),
    (557, "R_AARCH64_TLSLE_LDST32_TPREL_LO12_NC", Class::Plain),
    (558, "R_AARCH64_TLSLE_LDST64_TPREL_LO12", Class::Plain),
    (559, "R_AARCH64_TLSLE_LDST64_TPREL_LO12_NC", Class::Plain),
    (560, "R_AARCH64_TLSDESC_LD_PREL19", Class::Plain),
    (561, "R_AARCH64_TLSDESC_ADR_PREL21", Class::Plain),
    (562, "R_AARCH64_TLSDESC_ADR_PAGE21", Class::Plain),
    (563, "R_AARCH64_TLSDESC_LD64_LO12", Class::Plain),
    (564, "R_AARCH64_TLSDESC_ADD_LO12", Class::Plain),
    (569, "R_AARCH64_TLSDESC_CALL", Class::Plain),
    (570, "R_AARCH64_TLSLE_LDST128_TPREL_LO12", Class::Plain),
    (571, "R_AARCH64_TLSLE_LDST128_TPREL_LO12_NC", Class::Plain),
];

/// `R_X86_64_IRELATIVE`.
const X86_64_IRELATIVE: u32 = 37;
/// `R_AARCH64_IRELATIVE`.
const AARCH64_IRELATIVE: u32 = 1032;
/// `STT_GNU_IFUNC`.
const STT_GNU_IFUNC: u8 = 10;
/// `SHF_WRITE`, `SHF_ALLOC`, `SHF_EXECINSTR` and `SHF_TLS`.
const SHF_WRITE: u64 = 0x1;
const SHF_ALLOC: u64 = 0x2;
const SHF_EXECINSTR: u64 = 0x4;
const SHF_TLS: u64 = 0x400;

/// The entry of `r_type` in `arch`'s list.
fn entry(arch: Arch, r_type: u32) -> Option<(&'static str, Class)> {
    let list = match arch {
        Arch::X86_64 => X86_64,
        Arch::Aarch64 => AARCH64,
    };
    list.iter()
        .find(|(value, _, _)| *value == r_type)
        .map(|(_, name, class)| (*name, *class))
}

/// The name of `r_type` for messages and counts.
fn type_name(arch: Arch, r_type: u32) -> String {
    match (entry(arch, r_type), arch) {
        (Some((name, _)), _) => name.to_owned(),
        (None, Arch::X86_64) if r_type == X86_64_IRELATIVE => "R_X86_64_IRELATIVE".to_owned(),
        (None, Arch::Aarch64) if r_type == AARCH64_IRELATIVE => "R_AARCH64_IRELATIVE".to_owned(),
        (None, Arch::X86_64) => format!("x86-64 relocation type {r_type}"),
        (None, Arch::Aarch64) => format!("arm64 relocation type {r_type}"),
    }
}

/// Scans one ELF relocatable object.
fn scan_object(name: &str, data: &[u8]) -> Result<ElfScan> {
    let bad = |what: String| Problem::internal(format!("cannot read `{name}`: {what}"));
    let file = object::File::parse(data).map_err(|error| bad(error.to_string()))?;
    if file.format() != BinaryFormat::Elf || !file.is_64() || !file.is_little_endian() {
        return Err(bad("not a 64-bit little-endian ELF object".to_owned()));
    }
    if file.kind() != ObjectKind::Relocatable {
        return Err(bad("not a relocatable object".to_owned()));
    }
    let arch = match file.architecture() {
        Architecture::X86_64 => Arch::X86_64,
        Architecture::Aarch64 => Arch::Aarch64,
        other => return Err(bad(format!("an object for {other:?}"))),
    };
    let mut scan = ElfScan::default();
    for symbol in file.symbols() {
        let symbol_name = shown(symbol.name_bytes().unwrap_or_default());
        if let SymbolFlags::Elf { st_info, .. } = symbol.flags()
            && st_info.st_type().0 == STT_GNU_IFUNC
        {
            return Err(Problem::internal(format!(
                "`{name}` defines `{symbol_name}` as an indirect function (STT_GNU_IFUNC), which a static executable cannot have"
            )));
        }
        if symbol.section() == SymbolSection::Common {
            scan.common.insert(format!("{name}: {symbol_name}"));
        }
        if symbol_name == "__libc_version" && !symbol.is_undefined() {
            scan.musl_version = Some(version(name, &file, &symbol)?);
        }
    }
    for section in file.sections() {
        let section_name = section.name().unwrap_or("?").to_owned();
        let flags = match section.flags() {
            SectionFlags::Elf { sh_flags, .. } => sh_flags.0,
            _ => 0,
        };
        let loaded = flags & SHF_ALLOC != 0;
        let kind = if !loaded {
            "unloaded"
        } else if flags & SHF_TLS != 0 {
            "tls"
        } else if flags & SHF_EXECINSTR != 0 {
            "text"
        } else if flags & SHF_WRITE != 0 {
            "data"
        } else {
            "rodata"
        };
        for (_, relocation) in section.relocations() {
            let RelocationFlags::Elf { r_type } = relocation.flags() else {
                return Err(bad("a relocation that is not ELF's".to_owned()));
            };
            let r_type = r_type.0;
            let shown_type = type_name(arch, r_type);
            let (referent, absolute, symbol_name) = match relocation.target() {
                RelocationTarget::Symbol(index) => match file.symbol_by_index(index) {
                    Ok(symbol) => {
                        let symbol_name = shown(symbol.name_bytes().unwrap_or_default());
                        let referent = if symbol.is_undefined() {
                            "undefined"
                        } else if symbol.section() == SymbolSection::Absolute {
                            "absolute"
                        } else if symbol.is_local() {
                            "local"
                        } else {
                            "global"
                        };
                        (
                            referent,
                            symbol.section() == SymbolSection::Absolute,
                            symbol_name,
                        )
                    }
                    Err(error) => return Err(bad(error.to_string())),
                },
                RelocationTarget::Section(_) => ("section", false, String::new()),
                RelocationTarget::Absolute => ("none", true, String::new()),
                _ => ("other", false, String::new()),
            };
            let refused = |why: &str| {
                Problem::internal(format!(
                    "`{name}` has {shown_type} in `{section_name}` against `{symbol_name}`: {why}"
                ))
            };
            if r_type == X86_64_IRELATIVE && arch == Arch::X86_64
                || r_type == AARCH64_IRELATIVE && arch == Arch::Aarch64
            {
                return Err(refused(
                    "indirect functions are never supported (ADR 0124 rule 24)",
                ));
            }
            if loaded {
                match entry(arch, r_type) {
                    None => {
                        return Err(refused(
                            "the type is outside the supported set (ADR 0124 rule 24)",
                        ));
                    }
                    Some((_, Class::Narrow)) if !absolute => {
                        return Err(refused(
                            "a narrow absolute relocation against a symbol that is not absolute, which no load-time fixup can express",
                        ));
                    }
                    Some((_, Class::Wide)) if !absolute && flags & SHF_WRITE == 0 => {
                        return Err(refused(
                            "a 64-bit absolute relocation in a section that is not writable, a text relocation",
                        ));
                    }
                    Some(_) => {}
                }
            }
            *scan
                .counts
                .entry(format!("{shown_type} {kind} {referent}"))
                .or_insert(0) += 1;
        }
    }
    Ok(scan)
}

/// musl's version: the text that `__libc_version` holds, `MAJOR.MINOR.PATCH`.
fn version<'data>(
    name: &str,
    file: &object::File<'data>,
    symbol: &object::Symbol<'data, '_>,
) -> Result<String> {
    let bad = || {
        Problem::internal(format!(
            "`{name}` defines `__libc_version`, but not as a version `MAJOR.MINOR.PATCH`"
        ))
    };
    let index = symbol.section_index().ok_or_else(bad)?;
    let section = file.section_by_index(index).map_err(|_| bad())?;
    let data = section.data().map_err(|_| bad())?;
    let start = usize::try_from(symbol.address()).map_err(|_| bad())?;
    let size = usize::try_from(symbol.size()).map_err(|_| bad())?;
    let end = start.checked_add(size).ok_or_else(bad)?;
    let bytes = data.get(start..end).ok_or_else(bad)?;
    let text = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    let text = std::str::from_utf8(text).map_err(|_| bad())?;
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        Ok(text.to_owned())
    } else {
        Err(bad())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers of the lists are the ABI's, as the `object` crate names them.
    #[test]
    fn the_lists_use_the_abi_s_numbers() {
        use object::elf;
        let pairs = [
            (elf::R_X86_64_64, "R_X86_64_64"),
            (elf::R_X86_64_GOTPCRELX, "R_X86_64_GOTPCRELX"),
            (elf::R_X86_64_REX_GOTPCRELX, "R_X86_64_REX_GOTPCRELX"),
            (elf::R_X86_64_TPOFF32, "R_X86_64_TPOFF32"),
            (elf::R_X86_64_PC64, "R_X86_64_PC64"),
        ];
        for (value, name) in pairs {
            assert_eq!(type_name(Arch::X86_64, value.0), name);
        }
        let pairs = [
            (elf::R_AARCH64_ABS64, "R_AARCH64_ABS64"),
            (
                elf::R_AARCH64_LDST128_ABS_LO12_NC,
                "R_AARCH64_LDST128_ABS_LO12_NC",
            ),
            (
                elf::R_AARCH64_LD64_GOT_LO12_NC,
                "R_AARCH64_LD64_GOT_LO12_NC",
            ),
            (elf::R_AARCH64_TLSDESC_CALL, "R_AARCH64_TLSDESC_CALL"),
            (
                elf::R_AARCH64_TLSLE_LDST128_TPREL_LO12_NC,
                "R_AARCH64_TLSLE_LDST128_TPREL_LO12_NC",
            ),
            (
                elf::R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC,
                "R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC",
            ),
            (elf::R_AARCH64_MOVW_UABS_G3, "R_AARCH64_MOVW_UABS_G3"),
        ];
        for (value, name) in pairs {
            assert_eq!(type_name(Arch::Aarch64, value.0), name);
        }
        assert_eq!(elf::R_X86_64_IRELATIVE.0, X86_64_IRELATIVE);
        assert_eq!(elf::R_AARCH64_IRELATIVE.0, AARCH64_IRELATIVE);
        assert_eq!(elf::STT_GNU_IFUNC.0, STT_GNU_IFUNC);
        assert_eq!(elf::SHF_TLS.0, SHF_TLS);
    }
}
