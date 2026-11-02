//! The per-type oracle of rule 33 for the ELF relocation engine, before the ELF layout exists
//! ([`elf_oracle`]): a fixture object, written in assembly, is linked by a system linker
//! (`ld.lld` or GNU `ld`) into a static-pie executable; the engine is then run on the fixture's
//! sections with the addresses that linker chose (each input section at its output section, each
//! symbol where the output's symbol table puts it, the TLS template of its `PT_TLS`, and the GOT
//! slots of its `.got`), and at every place a relocation patched, the engine's bytes must be the
//! linker's. A 64-bit absolute relocation that became a relative fixup is compared with the
//! linker's `R_*_RELATIVE` entry of `.rela.dyn` (place and addend) instead of its bytes, which a
//! linker may leave as zero.
//!
//! Where the linker made a different choice with the same meaning, the place is counted as
//! equivalent and listed, so a reader sees each one: GNU `ld` relaxes an x86-64
//! `mov foo@GOTPCREL(%rip), %reg` into `lea foo(%rip), %reg` even for the plain `GOTPCREL` type,
//! which the engine (as `ld.lld`) keeps; the engine's load then goes through a GOT slot of its
//! own past the linker's image, and the two must load the same address. And GNU `ld` turns a
//! call to an undefined weak function on arm64 into a `nop`, where the engine (as `ld.lld`)
//! writes a branch to the next instruction, and on x86-64 into a call through a `.plt.got` stub
//! of a null GOT slot, where the engine (as `ld.lld`) calls the image's first byte (neither is
//! ever reached: the caller tests the function's address first). Any other difference is a
//! mismatch.
//!
//! The fixture's discipline: one input section per output section name (`.text`, `.data`,
//! `.rodata`, `.data.rel.ro`, `.tdata`, `.tbss`), no `.eh_frame`, and every defined symbol named,
//! so that each input section's address is its output section's.

use std::collections::BTreeMap;

use object::read::elf::{ElfFile64, ProgramHeader as _, Rela as _};
use object::read::{Object, ObjectSection, ObjectSymbol};
use object::{LittleEndian, SymbolKind};

use crate::elf::Arch;
use crate::elf::input::{self, Place};
use crate::elf::reloc::{self, Action, Layout, Referent, Tls};
use crate::problem::{Problem, Result, shown};

/// What [`elf_oracle`] compared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElfOracle {
    /// The places compared and found equal, by `TYPE action`.
    pub equal: BTreeMap<String, u64>,
    /// Each place where the linker made a different choice with the same meaning, described.
    pub equivalent: Vec<String>,
    /// Each place where the engine and the linker differ, described.
    pub mismatches: Vec<String>,
}

/// The kind of a symbol of the linker's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// In a section that is not thread-local.
    Address,
    /// Thread-local.
    Tls,
    /// Absolute (`SHN_ABS`).
    Absolute,
}

/// The output of the system linker, as the oracle reads it.
struct Linked<'a> {
    /// Each output section by name: (address, file offset, size, zero-fill).
    sections: BTreeMap<String, (u64, u64, u64, bool)>,
    /// Each defined symbol by name: its value and kind.
    symbols: BTreeMap<Vec<u8>, (u64, Kind)>,
    /// `PT_TLS`: address, memory size, alignment.
    tls: Option<(u64, u64, u64)>,
    /// `.rela.dyn`: place → (type, addend).
    dynamic: BTreeMap<u64, (u32, i64)>,
    /// The GOT's slots: address → what the slot holds once relocated.
    got: BTreeMap<u64, u64>,
    /// The file.
    data: &'a [u8],
}

fn read_linked(data: &[u8]) -> Result<Linked<'_>> {
    let bad = |what: String| Problem::internal(format!("cannot read the oracle's output: {what}"));
    let file = ElfFile64::<LittleEndian>::parse(data).map_err(|e| bad(e.to_string()))?;
    let endian = file.endian();
    let mut linked = Linked {
        sections: BTreeMap::new(),
        symbols: BTreeMap::new(),
        tls: None,
        dynamic: BTreeMap::new(),
        got: BTreeMap::new(),
        data,
    };
    for section in file.sections() {
        let name = section.name().map_err(|e| bad(e.to_string()))?.to_owned();
        let zerofill = section.file_range().is_none();
        let offset = section.file_range().map_or(0, |(offset, _)| offset);
        linked
            .sections
            .insert(name, (section.address(), offset, section.size(), zerofill));
    }
    for symbol in file.symbols() {
        if symbol.is_undefined() {
            continue;
        }
        let name = symbol.name_bytes().map_err(|e| bad(e.to_string()))?;
        if name.is_empty() {
            continue;
        }
        let kind = if symbol.kind() == SymbolKind::Tls {
            Kind::Tls
        } else if symbol.section() == object::SymbolSection::Absolute {
            Kind::Absolute
        } else {
            Kind::Address
        };
        linked
            .symbols
            .insert(name.to_vec(), (symbol.address(), kind));
    }
    for header in file.elf_program_headers() {
        if header.p_type(endian) == object::elf::PT_TLS {
            linked.tls = Some((
                header.p_vaddr(endian),
                header.p_memsz(endian),
                header.p_align(endian),
            ));
        }
    }
    if let Some(section) = file.section_by_name(".rela.dyn") {
        let bytes = section.data().map_err(|e| bad(e.to_string()))?;
        let (entries, _) = object::pod::slice_from_bytes::<object::elf::Rela64<LittleEndian>>(
            bytes,
            bytes.len() / 24,
        )
        .map_err(|()| bad("its `.rela.dyn`".to_owned()))?;
        for entry in entries {
            linked.dynamic.insert(
                entry.r_offset(endian),
                (entry.r_type(endian, false).0, entry.r_addend(endian)),
            );
        }
    }
    if let Some(&(address, offset, size, false)) = linked.sections.get(".got") {
        let mut slot: u64 = 0;
        while slot.saturating_add(8) <= size {
            let place = address.wrapping_add(slot);
            let literal = crate::bytes::u64_at(data, offset.wrapping_add(slot))?;
            let value = match linked.dynamic.get(&place) {
                Some(&(_, addend)) => u64::from_le_bytes(addend.to_le_bytes()),
                None => literal,
            };
            linked.got.insert(place, value);
            slot = slot.saturating_add(8);
        }
    }
    Ok(linked)
}

/// The layout the oracle's output implies for the fixture.
struct Oracle {
    referents: Vec<Result<Referent>>,
    tls: Option<Tls>,
    got: BTreeMap<u32, u64>,
}

impl Layout for Oracle {
    fn referent(&self, symbol: u32) -> Result<Referent> {
        usize::try_from(symbol)
            .ok()
            .and_then(|index| self.referents.get(index))
            .cloned()
            .unwrap_or_else(|| Err(Problem::internal(format!("no symbol {symbol}"))))
    }

    fn got(&self, symbol: u32) -> Result<u64> {
        self.got.get(&symbol).copied().ok_or_else(|| {
            Problem::internal(format!(
                "the engine needs a GOT slot for symbol {symbol} that the oracle's output does not have"
            ))
        })
    }

    fn tls(&self) -> Result<Tls> {
        self.tls
            .ok_or_else(|| Problem::internal("the oracle's output has no `PT_TLS`"))
    }
}

/// Compares the engine with a system linker on the fixture `object` and the linker's static-pie
/// output `linked` (see the module's description).
///
/// # Errors
///
/// U8001 when either input cannot be read, the fixture does not follow the oracle's discipline,
/// or the engine refuses a relocation of the fixture.
pub fn elf_oracle(object: &[u8], linked: &[u8]) -> Result<ElfOracle> {
    // The oracle is a test of the engine: its own arithmetic on the linker's addresses wraps
    // rather than panics, and any wrong address shows as a mismatch.
    let fixture = input::read("the fixture", object)?;
    let out = read_linked(linked)?;
    let arch = fixture.arch;
    let relative = match arch {
        Arch::X86_64 => object::elf::R_X86_64_RELATIVE.0,
        Arch::Aarch64 => object::elf::R_AARCH64_RELATIVE.0,
    };
    let tls_base = out.tls.map_or(0, |(address, _, _)| address);
    let mut address: BTreeMap<usize, u64> = BTreeMap::new();
    for (index, section) in fixture.sections.iter().enumerate() {
        if !section.kept() {
            continue;
        }
        let name = shown(section.name);
        let Some(&(at, _, size, _)) = out.sections.get(&name) else {
            if section.size == 0 {
                continue;
            }
            return Err(Problem::internal(format!(
                "the oracle's output has no section `{name}` for the fixture's"
            )));
        };
        if size < section.size {
            return Err(Problem::internal(format!(
                "the oracle's `{name}` is smaller than the fixture's: it holds more than one input"
            )));
        }
        address.insert(index, at);
    }
    let mut referents = Vec::with_capacity(fixture.symbols.len());
    let mut problems = Vec::new();
    // A problem stored per symbol names it by index: a name is copied only into a message that
    // is reported.
    for (position, symbol) in fixture.symbols.iter().enumerate() {
        let referent = match symbol.place {
            Place::Absolute => Ok(Referent::Absolute(symbol.value)),
            Place::Common { .. } => Err(Problem::internal("a common symbol in the fixture")),
            Place::Undefined => match out.symbols.get(symbol.name) {
                Some(&(value, Kind::Tls)) => Ok(Referent::Tls(value)),
                Some(&(value, Kind::Absolute)) => Ok(Referent::Absolute(value)),
                Some(&(value, Kind::Address)) => Ok(Referent::Address(value)),
                None if symbol.weak() => Ok(Referent::UndefinedWeak),
                None => Err(Problem::internal(format!(
                    "the fixture's symbol {position} is undefined in the oracle's output too"
                ))),
            },
            Place::Section(index) => {
                let at = address.get(&index).copied();
                let tls = fixture.sections.get(index).is_some_and(|s| s.tls());
                match at {
                    Some(at) if tls => Ok(Referent::Tls(
                        at.wrapping_add(symbol.value).wrapping_sub(tls_base),
                    )),
                    Some(at) => {
                        let value = at.wrapping_add(symbol.value);
                        if let Some(&(linked, Kind::Address)) = out.symbols.get(symbol.name)
                            && linked != value
                        {
                            problems.push(format!(
                                "the oracle placed `{}` at {linked:#x}, not at {value:#x}",
                                shown(symbol.name)
                            ));
                        }
                        Ok(Referent::Address(value))
                    }
                    None => Err(Problem::internal(format!(
                        "the fixture's symbol {position} is in a section the executable does not contain"
                    ))),
                }
            }
        };
        referents.push(referent);
    }
    let mut oracle = Oracle {
        referents,
        tls: out.tls.map(|(_, size, align)| Tls { size, align }),
        got: BTreeMap::new(),
    };
    // The engine's GOT slots are the oracle's slots that hold the same value, or slots of its
    // own past the oracle's image where the oracle has none.
    let mut own: BTreeMap<u64, u64> = BTreeMap::new();
    let end = out
        .sections
        .values()
        .map(|&(address, _, size, _)| address.wrapping_add(size))
        .max()
        .unwrap_or(0);
    let mut next = end.wrapping_add(0x1000) & !7;
    for index in 0..fixture.sections.len() {
        let plan = reloc::plan(&fixture, index, &|symbol| oracle.referent(symbol))?;
        for symbol in plan.got {
            let entry = reloc::got_entry(oracle.referent(symbol)?)?;
            if oracle.got.contains_key(&symbol) {
                continue;
            }
            if let Some((&slot, _)) = out.got.iter().find(|(_, value)| **value == entry.value) {
                oracle.got.insert(symbol, slot);
            } else {
                oracle.got.insert(symbol, next);
                own.insert(next, entry.value);
                next = next.wrapping_add(8);
            }
        }
    }
    let mut report = ElfOracle {
        mismatches: problems,
        ..ElfOracle::default()
    };
    for (index, section) in fixture.sections.iter().enumerate() {
        let Some(&at) = address.get(&index) else {
            continue;
        };
        if section.relocs.is_empty() {
            continue;
        }
        let name = shown(section.name);
        let Some(&(_, offset, _, _)) = out.sections.get(&name) else {
            continue;
        };
        let plan = reloc::plan(&fixture, index, &|symbol| oracle.referent(symbol))?;
        let mut bytes = section.data.to_vec();
        let fixups = match reloc::apply(&fixture, index, &mut bytes, at, &oracle) {
            Ok(fixups) => fixups,
            Err(problem) => {
                report.mismatches.push(format!("`{name}`: {problem}"));
                continue;
            }
        };
        for step in plan.steps {
            let Some(rela) = section.relocs.get(step.index) else {
                continue;
            };
            let key = format!(
                "{} {}",
                reloc::type_name(arch, rela.kind),
                step.action.name()
            );
            let place = at.wrapping_add(rela.offset);
            let equal = match step.action {
                Action::Consumed => true,
                Action::Relative => {
                    let ours = fixups.iter().find(|f| f.place == place);
                    let theirs = out.dynamic.get(&place);
                    match (ours, theirs) {
                        (Some(fixup), Some(&(kind, addend)))
                            if kind == relative
                                && u64::from_le_bytes(addend.to_le_bytes()) == fixup.addend =>
                        {
                            true
                        }
                        _ => {
                            report.mismatches.push(format!(
                                "{key} at `{name}`+{:#x}: the engine's fixup {ours:?}, the oracle's {theirs:?}",
                                rela.offset
                            ));
                            false
                        }
                    }
                }
                _ => {
                    let range = usize::try_from(step.start).unwrap_or(usize::MAX)
                        ..usize::try_from(step.end).unwrap_or(usize::MAX);
                    let file = usize::try_from(offset).unwrap_or(usize::MAX);
                    let theirs = out
                        .data
                        .get(file.saturating_add(range.start)..file.saturating_add(range.end));
                    let ours = bytes.get(range);
                    let at =
                        file.saturating_add(usize::try_from(rela.offset).unwrap_or(usize::MAX));
                    if ours.is_some() && ours == theirs {
                        true
                    } else if let Some(why) = equivalent(
                        arch,
                        (step.action, rela),
                        (section.data, &out, at),
                        (place, oracle.referent(rela.symbol)?, &oracle.got, &own),
                    ) {
                        report
                            .equivalent
                            .push(format!("{key} at `{name}`+{:#x}: {why}", rela.offset));
                        true
                    } else {
                        report.mismatches.push(format!(
                            "{key} at `{name}`+{:#x}: the engine wrote {ours:02x?}, the oracle {theirs:02x?}",
                            rela.offset
                        ));
                        false
                    }
                }
            };
            if equal {
                *report.equal.entry(key).or_insert(0) += 1;
            }
        }
    }
    Ok(report)
}

/// Why a place where the bytes differ means the same, if it is one of the known choices of
/// the module's description.
fn equivalent(
    arch: Arch,
    (action, rela): (Action, &input::Rela),
    (input, out, at): (&[u8], &Linked<'_>, usize),
    (place, referent, got, own): (u64, Referent, &BTreeMap<u32, u64>, &BTreeMap<u64, u64>),
) -> Option<&'static str> {
    let linked = out.data;
    let offset = usize::try_from(rela.offset).ok()?;
    match (arch, action, referent) {
        (Arch::X86_64, Action::Got, Referent::Address(target)) => {
            // Ours: `mov` from our own slot holding the target; theirs: `lea` of the target.
            let slot = got.get(&rela.symbol)?;
            let held = own.get(slot)?;
            let ours = (
                input.get(offset.checked_sub(2)?)?,
                input.get(offset.checked_sub(1)?)?,
            );
            let theirs = (
                linked.get(at.checked_sub(2)?)?,
                linked.get(at.checked_sub(1)?)?,
            );
            let disp = i32::from_le_bytes(linked.get(at..at.checked_add(4)?)?.try_into().ok()?);
            let reached = place.wrapping_add(4).wrapping_add_signed(i64::from(disp));
            (rela.addend == -4
                && *ours.0 == 0x8b
                && *theirs.0 == 0x8d
                && ours.1 == theirs.1
                && *held == target
                && reached == target)
                .then_some("GNU ld relaxed the GOT load to `lea` of the same address")
        }
        (Arch::X86_64, Action::Direct, Referent::UndefinedWeak) => {
            // Ours (as `ld.lld`): a call to the image's first byte. Theirs: a call through a
            // `.plt.got` stub whose GOT slot holds 0. Neither is reached: the caller tests the
            // function's address first, and either would fault.
            let disp = i32::from_le_bytes(linked.get(at..at.checked_add(4)?)?.try_into().ok()?);
            let reached = place.wrapping_add(4).wrapping_add_signed(i64::from(disp));
            let stub = [".plt", ".plt.got"].iter().any(|name| {
                out.sections
                    .get(*name)
                    .is_some_and(|&(address, offset, size, _)| {
                        reached >= address
                            && reached < address.wrapping_add(size)
                            && null_slot(
                                out,
                                reached.wrapping_sub(address).wrapping_add(offset),
                                reached,
                            )
                    })
            });
            (rela.kind == crate::elf::x86_64::R_PLT32 && stub).then_some(
                "GNU ld called the undefined weak function through a `.plt.got` stub whose GOT slot holds 0",
            )
        }
        (Arch::Aarch64, Action::Direct, Referent::UndefinedWeak) => {
            let theirs = u32::from_le_bytes(linked.get(at..at.checked_add(4)?)?.try_into().ok()?);
            (theirs == 0xd503_201f).then_some(
                "GNU ld wrote a `nop` for the call to the undefined weak function, a branch to the next instruction",
            )
        }
        _ => None,
    }
}

/// Whether the stub at `address` (at `file` in the linker's output) jumps through a GOT slot that
/// holds 0 and has no dynamic relocation: `jmp *slot(%rip)` (`ff 25`), possibly after an
/// `endbr64` and a `bnd` prefix, within the stub's first 8 bytes.
fn null_slot(out: &Linked<'_>, file: u64, address: u64) -> bool {
    let Ok(file) = usize::try_from(file) else {
        return false;
    };
    let Some(stub) = out.data.get(file..file.saturating_add(16)) else {
        return false;
    };
    (0..=8).any(|at| {
        let Some(([0xff, 0x25], disp)) = stub
            .get(at..at + 2)
            .zip(stub.get(at + 2..at + 6))
            .map(|(op, disp)| (<[u8; 2]>::try_from(op).unwrap_or_default(), disp))
        else {
            return false;
        };
        let disp = i32::from_le_bytes(<[u8; 4]>::try_from(disp).unwrap_or_default());
        let slot = address
            .wrapping_add(u64::try_from(at + 6).unwrap_or(0))
            .wrapping_add_signed(i64::from(disp));
        out.got.get(&slot) == Some(&0) && !out.dynamic.contains_key(&slot)
    })
}
