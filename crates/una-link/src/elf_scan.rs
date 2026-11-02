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
//!   for the object's architecture (the list of the relocation engine, [`crate::elf::reloc`]), a narrow absolute relocation against a symbol that is not
//!   absolute, and a 64-bit absolute one in a section that is neither writable nor thread-local
//!   data (a text relocation). Relocations in sections that are not loaded (`.debug_*`,
//!   `.comment`) are counted and never refused.
//!
//! It lists the common symbols, which the standard library must not define (design note §3), and
//! reads musl's version from `__libc_version`, which decides whether its start-up applies
//! `DT_RELR` (rule 25, probe P2). It links nothing and writes nothing.

use std::collections::{BTreeMap, BTreeSet};

use object::read::{Object, ObjectSection, RelocationTarget};
use object::{Architecture, BinaryFormat, ObjectKind, RelocationFlags, SectionFlags};

use crate::elf::Arch;
use crate::elf::input::{self, ElfObject, Place, Symbol};
use crate::elf::reloc::{self, Class};
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
/// defines a symbol without reading the others (rule 12), both ways and over every entry,
/// duplicates included: each entry must name the header of a member that defines that symbol
/// (as a global, weak or common symbol), and each such definition of each member must have its
/// entry, so the resolver can neither be sent to the wrong member nor miss a definition.
/// Returns how many entries there are. O((entries + symbols) × log).
fn check_index(archive: &Archive<'_>) -> Result<u64> {
    let entries = archive.table()?;
    let listed: BTreeSet<(&[u8], u64)> = entries.iter().copied().collect();
    let mut defined: BTreeMap<u64, (String, BTreeSet<&[u8]>)> = BTreeMap::new();
    for (start, member, data) in archive.members()? {
        if member.ends_with("(lib.rmeta)") {
            continue;
        }
        // A GNU archive's member header is the 60 bytes before its data.
        let header = start.checked_sub(60).ok_or_else(|| {
            Problem::internal(format!("cannot read `{member}`: a member without a header"))
        })?;
        // The linker's reader, whose names are borrowed from the member and bounded in total.
        let object = input::read(&member, data)?;
        let mut names = BTreeSet::new();
        for symbol in &object.symbols {
            if symbol.place == Place::Undefined || symbol.bind == crate::elf::STB_LOCAL {
                continue;
            }
            let name = symbol.name;
            if !listed.contains(&(name, header)) {
                return Err(Problem::internal(format!(
                    "`{member}` defines `{}`, which the symbol table of `{}` does not list for it",
                    shown(name),
                    archive.name
                )));
            }
            names.insert(name);
        }
        defined.insert(header, (member, names));
    }
    for (symbol, offset) in &entries {
        let Some((member, names)) = defined.get(offset) else {
            return Err(Problem::internal(format!(
                "the symbol table of `{}` names no member for `{}`",
                archive.name,
                shown(symbol)
            )));
        };
        if !names.contains(*symbol) {
            return Err(Problem::internal(format!(
                "the symbol table of `{}` names `{member}` for `{}`, which that member does not define",
                archive.name,
                shown(symbol)
            )));
        }
    }
    u64::try_from(entries.len()).map_err(|_| Problem::internal("too many symbols"))
}

/// `R_X86_64_IRELATIVE` and `R_AARCH64_IRELATIVE`.
const X86_64_IRELATIVE: u32 = crate::elf::x86_64::R_IRELATIVE;
const AARCH64_IRELATIVE: u32 = crate::elf::aarch64::R_IRELATIVE;
/// `SHF_WRITE`, `SHF_ALLOC`, `SHF_EXECINSTR` and `SHF_TLS`.
const SHF_WRITE: u64 = crate::elf::SHF_WRITE;
const SHF_ALLOC: u64 = crate::elf::SHF_ALLOC;
const SHF_EXECINSTR: u64 = crate::elf::SHF_EXECINSTR;
const SHF_TLS: u64 = crate::elf::SHF_TLS;

/// The entry of `r_type` in `arch`'s list: the relocation engine's list (rule 24), so that the
/// scan accepts exactly what the linker applies.
fn entry(arch: Arch, r_type: u32) -> Option<(&'static str, Class)> {
    reloc::find(arch, r_type).map(|t| (t.name, t.class))
}

/// The name of `r_type` for messages and counts.
fn type_name(arch: Arch, r_type: u32) -> String {
    reloc::type_name(arch, r_type)
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
    check_relocation_sections(name, data)?;
    // The linker's reader (step 4b): it refuses indirect functions and what else the linker
    // would, and reads every name in O(log) within a budget linear in the input, so that the
    // scan's work stays linear however the names share the string table (no name is copied
    // but into a reported message or a listed common symbol, which is cut to 256 bytes).
    let object = input::read(name, data)?;
    let mut scan = ElfScan::default();
    for symbol in &object.symbols {
        if let Place::Common { .. } = symbol.place {
            let cut = symbol.name.get(..256).unwrap_or(symbol.name);
            scan.common.insert(format!("{name}: {}", shown(cut)));
        }
        if symbol.name == b"__libc_version" && !matches!(symbol.place, Place::Undefined) {
            scan.musl_version = Some(version(name, &object, symbol)?);
        }
    }
    for section in file.sections() {
        let section_name = || object.section_name(section.index().0);
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
            let (referent, absolute, symbol) = match relocation.target() {
                RelocationTarget::Symbol(index) => {
                    let position = u32::try_from(index.0).unwrap_or(u32::MAX);
                    let symbol = object.symbol(position)?;
                    let referent = match symbol.place {
                        Place::Undefined => "undefined",
                        Place::Absolute => "absolute",
                        _ if symbol.bind == crate::elf::STB_LOCAL => "local",
                        _ => "global",
                    };
                    (referent, symbol.place == Place::Absolute, Some(position))
                }
                RelocationTarget::Section(_) => ("section", false, None),
                RelocationTarget::Absolute => ("none", true, None),
                _ => ("other", false, None),
            };
            // Built only on an error path: a message copies the names.
            let refused = |why: &str| {
                Problem::internal(format!(
                    "`{name}` has {shown_type} in `{}` against `{}`: {why}",
                    section_name(),
                    symbol.map_or_else(String::new, |index| object.symbol_name(index))
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

/// `SHT_SYMTAB`, `SHT_RELA`, `SHT_REL` and `SHT_CREL`.
const SHT_SYMTAB: u32 = 2;
const SHT_RELA: u32 = 4;
const SHT_REL: u32 = 9;
const SHT_CREL: u32 = 0x4000_0014;

/// Refuses a relocation section that the `object` crate's reader would pass over without a word:
/// one whose `sh_link` is not the object's symbol table, or whose `sh_info` names no section
/// (0, itself, or past the table). Its relocations would be neither scanned nor, by a linker on
/// the same reader, applied. O(sections).
pub(crate) fn check_relocation_sections(name: &str, data: &[u8]) -> Result<()> {
    use object::read::elf::{ElfFile64, SectionHeader};
    let bad = |what: String| Problem::internal(format!("cannot read `{name}`: {what}"));
    let file = ElfFile64::<object::LittleEndian>::parse(data).map_err(|e| bad(e.to_string()))?;
    let endian = file.endian();
    let sections = file.elf_section_table();
    let count = sections.len();
    let symtab = sections
        .iter()
        .position(|section| section.sh_type(endian).0 == SHT_SYMTAB);
    for (index, section) in sections.iter().enumerate() {
        let kind = section.sh_type(endian).0;
        if kind != SHT_RELA && kind != SHT_REL && kind != SHT_CREL {
            continue;
        }
        // Built only on an error path: names may share one long string.
        let section_name = || {
            sections
                .section_name(endian, section)
                .map_or_else(|_| "?".to_owned(), shown)
        };
        let link = usize::try_from(section.sh_link(endian)).unwrap_or(usize::MAX);
        if Some(link) != symtab {
            return Err(bad(format!(
                "the relocation section `{}` is not linked to the symbol table",
                section_name()
            )));
        }
        let info = usize::try_from(section.sh_info(endian)).unwrap_or(usize::MAX);
        if info == 0 || info == index || info >= count {
            return Err(bad(format!(
                "the relocation section `{}` applies to no section",
                section_name()
            )));
        }
    }
    Ok(())
}

/// musl's version: the text that `__libc_version` holds, `MAJOR.MINOR.PATCH`.
fn version(name: &str, object: &ElfObject<'_>, symbol: &Symbol<'_>) -> Result<String> {
    let bad = || {
        Problem::internal(format!(
            "`{name}` defines `__libc_version`, but not as a version `MAJOR.MINOR.PATCH`"
        ))
    };
    let Place::Section(index) = symbol.place else {
        return Err(bad());
    };
    let data = object.section(index).map_err(|_| bad())?.data;
    let start = usize::try_from(symbol.value).map_err(|_| bad())?;
    let size = usize::try_from(symbol.size).map_err(|_| bad())?;
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
        assert_eq!(elf::STT_GNU_IFUNC.0, crate::elf::STT_GNU_IFUNC);
        assert_eq!(elf::SHF_TLS.0, SHF_TLS);
    }
}
