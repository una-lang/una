//! The relocation engine run over real inputs before the ELF layout exists ([`check_elf`]): each
//! object is laid out on its own by a simple synthetic layout (its loaded sections one after the
//! other, its thread-local sections in a TLS template, a GOT slot per symbol that needs one, and
//! each undefined symbol, and each symbol the linker defines, at an address of its own past the
//! sections), and every relocation of every
//! kept section is planned and applied. It shows that the engine accepts every relocation the
//! inputs hold and that the reader accepts every object, without a link: the layout of slice 4c
//! replaces the synthetic one.

use std::collections::{BTreeMap, BTreeSet};

use crate::elf::input::{self, ElfObject, Place};
use crate::elf::reloc::{self, Layout, Referent, Tls};
use crate::input::Archive;
use crate::problem::{Problem, Result};

/// What [`check_elf`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElfCheck {
    /// How many objects were read.
    pub objects: u64,
    /// Every relocation applied, by `TYPE action`, with its count.
    pub relocations: BTreeMap<String, u64>,
    /// How many GOT slots the synthetic layouts needed.
    pub got: u64,
    /// How many relative fixups the relocations produced.
    pub fixups: u64,
    /// How many COMDAT groups the objects hold.
    pub groups: u64,
}

impl ElfCheck {
    fn merge(&mut self, other: ElfCheck) {
        self.objects += other.objects;
        for (key, count) in other.relocations {
            *self.relocations.entry(key).or_insert(0) += count;
        }
        self.got += other.got;
        self.fixups += other.fixups;
        self.groups += other.groups;
    }
}

/// Reads `bytes`, an ELF relocatable object for x86-64 or arm64 or an `ar` archive of them
/// (whose `lib.rmeta` member, an rlib's metadata, is skipped), and plans and applies every
/// relocation of every section the executable would contain, with a synthetic layout of each
/// object. O(size of the input + R log R) for R relocations.
///
/// # Errors
///
/// U8001 when an object cannot be read or holds a relocation that ADR 0124 rule 24 refuses;
/// U7015 when an object cannot be laid out (a section too large, a field beyond its reach).
pub fn check_elf(name: &str, bytes: &[u8]) -> Result<ElfCheck> {
    let mut check = ElfCheck::default();
    if bytes.starts_with(b"!<arch>\n") {
        let archive = Archive::read(name, bytes)?;
        for (_, member, data) in archive.members()? {
            if member.ends_with("(lib.rmeta)") {
                continue;
            }
            check.merge(check_object(&member, data)?);
        }
    } else {
        check.merge(check_object(name, bytes)?);
    }
    Ok(check)
}

/// The synthetic layout of one object.
struct Synthetic<'o, 'a> {
    /// The object, for the messages.
    object: &'o ElfObject<'a>,
    /// The address of each kept section that is not thread-local, by index.
    address: BTreeMap<usize, u64>,
    /// The template offset of each thread-local section, by index.
    tls_offset: BTreeMap<usize, u64>,
    /// The template.
    tls: Tls,
    /// The referent of each symbol, by index; `None` for a symbol of a section the executable
    /// does not contain, whose problem is formatted only when a relocation reaches it (a message
    /// copies the name, and an object may hold many such symbols).
    referents: Vec<Option<Referent>>,
    /// The GOT slot of each symbol that needs one.
    got: BTreeMap<u32, u64>,
}

impl Layout for Synthetic<'_, '_> {
    fn referent(&self, symbol: u32) -> Result<Referent> {
        let found = usize::try_from(symbol)
            .ok()
            .and_then(|index| self.referents.get(index))
            .ok_or_else(|| Problem::internal(format!("no symbol {symbol}")))?;
        found.ok_or_else(|| {
            let section = match self.object.symbol(symbol).map(|s| s.place) {
                Ok(Place::Section(index)) => self.object.section_name(index),
                _ => String::new(),
            };
            Problem::internal(format!(
                "`{}`: `{}` is in `{section}`, a section the executable does not contain",
                self.object.name,
                self.object.symbol_name(symbol)
            ))
        })
    }

    fn got(&self, symbol: u32) -> Result<u64> {
        self.got
            .get(&symbol)
            .copied()
            .ok_or_else(|| Problem::internal(format!("no GOT slot for symbol {symbol}")))
    }

    fn tls(&self) -> Result<Tls> {
        Ok(self.tls)
    }
}

/// Where the synthetic layout starts: past the headers of an executable.
const BASE: u64 = 0x1_0000;

fn grow(object: &ElfObject<'_>, at: u64, align: u64, size: u64) -> Result<(u64, u64)> {
    let start = at
        .checked_next_multiple_of(align.max(1))
        .ok_or_else(|| Problem::layout(format!("`{}` is too large to lay out", object.name)))?;
    let end = start
        .checked_add(size)
        .filter(|end| *end < 1 << 40)
        .ok_or_else(|| Problem::layout(format!("`{}` is too large to lay out", object.name)))?;
    Ok((start, end))
}

fn check_object(name: &str, data: &[u8]) -> Result<ElfCheck> {
    let object = input::read(name, data)?;
    let mut layout = Synthetic {
        object: &object,
        address: BTreeMap::new(),
        tls_offset: BTreeMap::new(),
        tls: Tls { size: 0, align: 1 },
        referents: Vec::new(),
        got: BTreeMap::new(),
    };
    let mut at = BASE;
    let mut tls_at = 0;
    // The TLS template: initialized data first, then zero-fill, as `PT_TLS` holds them.
    for zerofill in [false, true] {
        for (index, section) in object.sections.iter().enumerate() {
            if section.kept() && section.tls() && section.zerofill() == zerofill {
                let (start, end) = grow(&object, tls_at, section.align, section.size)?;
                layout.tls_offset.insert(index, start);
                layout.tls.align = layout.tls.align.max(section.align);
                tls_at = end;
            }
        }
    }
    layout.tls.size = tls_at;
    for (index, section) in object.sections.iter().enumerate() {
        if section.kept() && !section.tls() {
            let (start, end) = grow(&object, at, section.align, section.size)?;
            layout.address.insert(index, start);
            at = end;
        }
    }
    // Undefined symbols, each at an address of its own past the sections, then the common
    // symbols, then the GOT.
    // Keyed by the symbol's index, not its name: comparing names would cost their lengths.
    let mut undefined: BTreeMap<usize, u64> = BTreeMap::new();
    for (index, symbol) in object.symbols.iter().enumerate() {
        if symbol.place == Place::Undefined
            && (!symbol.weak() || crate::elf::linker_defined(symbol.name))
            && !symbol.tls()
        {
            let next = u64::try_from(undefined.len()).unwrap_or(u64::MAX);
            undefined.insert(index, next);
        }
    }
    let count = u64::try_from(undefined.len()).unwrap_or(u64::MAX);
    let (at, _) = grow(&object, at, 16, 0)?;
    let (mut commons, _) = grow(&object, at, 16, count.saturating_add(1).saturating_mul(16))?;
    commons = commons.saturating_add(count.saturating_add(1).saturating_mul(16));
    let mut referents = Vec::with_capacity(object.symbols.len());
    for (position, symbol) in object.symbols.iter().enumerate() {
        let referent = match symbol.place {
            Place::Absolute => Some(Referent::Absolute(symbol.value)),
            Place::Undefined if symbol.weak() && !crate::elf::linker_defined(symbol.name) => {
                Some(Referent::UndefinedWeak)
            }
            Place::Undefined if symbol.tls() => Some(Referent::Tls(0)),
            Place::Undefined => {
                let slot = undefined.get(&position).copied().unwrap_or(0);
                Some(Referent::Address(
                    at.saturating_add(slot.saturating_mul(16)),
                ))
            }
            Place::Common { align } => {
                let (start, end) = grow(&object, commons, align, symbol.size)?;
                commons = end;
                Some(Referent::Address(start))
            }
            Place::Section(index) => {
                if let Some(offset) = layout.tls_offset.get(&index) {
                    Some(Referent::Tls(offset.saturating_add(symbol.value)))
                } else {
                    layout
                        .address
                        .get(&index)
                        .map(|address| Referent::Address(address.saturating_add(symbol.value)))
                }
            }
        };
        referents.push(referent);
    }
    layout.referents = referents;
    let mut needed: BTreeSet<u32> = BTreeSet::new();
    for index in 0..object.sections.len() {
        let plan = reloc::plan(&object, index, &|symbol| layout.referent(symbol))?;
        needed.extend(plan.got);
    }
    let (mut slot, _) = grow(&object, commons, 8, 0)?;
    let mut check = ElfCheck {
        objects: 1,
        groups: u64::try_from(object.groups.len()).unwrap_or(u64::MAX),
        ..ElfCheck::default()
    };
    for symbol in needed {
        reloc::got_entry(layout.referent(symbol)?)?;
        layout.got.insert(symbol, slot);
        slot = slot.saturating_add(8);
        check.got += 1;
    }
    for (index, section) in object.sections.iter().enumerate() {
        if !section.kept() || section.relocs.is_empty() {
            continue;
        }
        let plan = reloc::plan(&object, index, &|symbol| layout.referent(symbol))?;
        for step in &plan.steps {
            let kind = section.relocs.get(step.index).map_or(0, |r| r.kind);
            let key = format!(
                "{} {}",
                reloc::type_name(object.arch, kind),
                step.action.name()
            );
            *check.relocations.entry(key).or_insert(0) += 1;
        }
        let address = layout
            .address
            .get(&index)
            .or_else(|| layout.tls_offset.get(&index))
            .copied()
            .unwrap_or(BASE);
        let mut bytes = section.data.to_vec();
        let fixups = reloc::apply(&object, index, &mut bytes, address, &layout)?;
        check.fixups += u64::try_from(fixups.len()).unwrap_or(u64::MAX);
    }
    Ok(check)
}
