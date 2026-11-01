//! Atoms, the targets of relocations, and dead-code removal (ADR 0124 rule 23).
//!
//! An input section is split into atoms at its symbols when its object allows it
//! (`MH_SUBSECTIONS_VIA_SYMBOLS`), and is one atom otherwise. Atoms are live when they are
//! reachable from the roots through relocations: `_main`, the initializers, and what an input
//! marks as never stripped. An `__eh_frame` FDE and a `__compact_unwind` entry are kept only for
//! a live function, and keep its LSDA and personality alive (live support). This keeps the
//! effect of `-dead_strip`: an executable that never uses `Http` carries no TLS stack.

use std::collections::BTreeMap;

use crate::bytes;
use crate::input::{self, InSection, Object, Referent, Reloc, flags, nlist};
use crate::problem::{Problem, Result, shown};
use crate::resolve::{Global, Resolved};
use crate::sections::{Class, Out, classify};

/// A contiguous piece of an input section, the unit of dead-code removal and placement.
#[derive(Clone, Debug)]
pub(crate) struct Atom {
    /// Its object.
    pub(crate) object: usize,
    /// Its section's index in the object.
    pub(crate) section: usize,
    /// Its start, an offset into the section.
    pub(crate) start: u64,
    /// Its end, exclusive.
    pub(crate) end: u64,
    /// Its output section.
    pub(crate) out: Out,
    /// Whether it is reachable from the roots.
    pub(crate) live: bool,
}

/// What a relocation reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Target<'a> {
    /// A place in an atom: the atom's index and an offset from its start.
    Atom {
        /// The atom.
        atom: usize,
        /// The offset from the atom's start (an addend may lead outside it).
        offset: i64,
    },
    /// A symbol that no input defines, bound to libSystem.
    Import(&'a [u8]),
    /// `__mh_execute_header`.
    Header,
}

/// A `__compact_unwind` entry.
#[derive(Clone, Debug)]
pub(crate) struct CompactEntry<'a> {
    /// The function.
    pub(crate) function: Target<'a>,
    /// The compact encoding.
    pub(crate) encoding: u32,
    /// The personality routine.
    pub(crate) personality: Option<Target<'a>>,
    /// The language-specific data area.
    pub(crate) lsda: Option<Target<'a>>,
}

/// A record of an input `__eh_frame`.
#[derive(Clone, Debug)]
pub(crate) struct EhRecord<'a> {
    /// Its object.
    pub(crate) object: usize,
    /// Its section in the object.
    pub(crate) section: usize,
    /// Its start in the section.
    pub(crate) start: u64,
    /// Its end, exclusive.
    pub(crate) end: u64,
    /// For an FDE, the index of its CIE in [`Graph::eh_records`]; `None` for a CIE.
    pub(crate) cie: Option<usize>,
    /// For an FDE, the function it describes.
    pub(crate) function: Option<Target<'a>>,
    /// Every other target of the record's relocations (LSDA, personality).
    pub(crate) others: Vec<Target<'a>>,
    /// Whether the record is kept.
    pub(crate) live: bool,
}

/// The atoms of every loaded object, with the tables that find them.
pub(crate) struct Graph<'a> {
    /// The resolution.
    pub(crate) resolved: Resolved<'a>,
    /// Every atom, in placement order of objects, then sections, then offsets.
    pub(crate) atoms: Vec<Atom>,
    /// For each object and section, the range of its atoms in [`Graph::atoms`].
    pub(crate) section_atoms: Vec<Vec<std::ops::Range<usize>>>,
    /// The class of each object's sections.
    pub(crate) classes: Vec<Vec<Class>>,
    /// Every `__compact_unwind` entry, in placement order.
    pub(crate) compact: Vec<CompactEntry<'a>>,
    /// Every `__eh_frame` record, in placement order.
    pub(crate) eh_records: Vec<EhRecord<'a>>,
}

impl<'a> Graph<'a> {
    /// The object at `index`.
    pub(crate) fn object(&self, index: usize) -> Result<&Object<'a>> {
        self.resolved
            .objects
            .get(index)
            .ok_or_else(|| Problem::internal("an object index out of range"))
    }

    /// The section `section` of `object`.
    pub(crate) fn section(&self, object: usize, section: usize) -> Result<&InSection<'a>> {
        self.object(object)?
            .sections
            .get(section)
            .ok_or_else(|| Problem::internal("a section index out of range"))
    }

    /// The atom at `index`.
    pub(crate) fn atom(&self, index: usize) -> Result<&Atom> {
        self.atoms
            .get(index)
            .ok_or_else(|| Problem::internal("an atom index out of range"))
    }

    /// The relocations of an atom: those of its section whose place lies in it.
    pub(crate) fn atom_relocs(&self, index: usize) -> Result<&[Reloc]> {
        let atom = self.atom(index)?;
        let section = self.section(atom.object, atom.section)?;
        Ok(relocs_in(&section.relocs, atom.start, atom.end))
    }

    /// The atom of `section` of `object` that holds `offset`, and the offset from its start.
    fn atom_at(&self, object: usize, section: usize, offset: u64) -> Result<Target<'a>> {
        let range = self
            .section_atoms
            .get(object)
            .and_then(|sections| sections.get(section))
            .cloned()
            .unwrap_or(0..0);
        let atoms = self.atoms.get(range.clone()).unwrap_or(&[]);
        let section_size = self.section(object, section)?.size;
        if atoms.is_empty() || offset > section_size {
            let input = self.section(object, section)?;
            return Err(Problem::internal(format!(
                "`{}`: a reference into {},{} at {offset:#x} reaches no part of the output",
                self.object(object)?.name,
                input.segment,
                input.name
            )));
        }
        let position = atoms.partition_point(|atom| atom.start <= offset);
        let local = position.saturating_sub(1);
        let atom = range.start.saturating_add(local);
        let start = atoms.get(local).map_or(0, |a| a.start);
        Ok(Target::Atom {
            atom,
            offset: signed(offset.saturating_sub(start))?,
        })
    }

    /// What the symbol `symbol` of `object` reaches.
    pub(crate) fn symbol_target(&self, object: usize, symbol: u32) -> Result<Target<'a>> {
        let input = self.object(object)?;
        let sym = input
            .symbols
            .get(usize::try_from(symbol).unwrap_or(usize::MAX))
            .ok_or_else(|| Problem::internal("a symbol index out of range"))?;
        if sym.external() || sym.undefined() {
            return match self.resolved.globals.get(sym.name) {
                Some(Global::Defined {
                    object: other,
                    symbol: index,
                    ..
                }) => self.defined_target(*other, *index),
                Some(Global::Undefined) => Ok(Target::Import(sym.name)),
                Some(Global::Header) => Ok(Target::Header),
                None => Err(Problem::internal(format!(
                    "`{}`: the symbol `{}` was not resolved",
                    input.name,
                    shown(sym.name)
                ))),
            };
        }
        if sym.in_section() {
            return self.defined_target(object, usize::try_from(symbol).unwrap_or(usize::MAX));
        }
        Err(Problem::internal(format!(
            "`{}`: a relocation refers to `{}`, a symbol of a type ({:#x}) the internal linker does not support",
            input.name,
            shown(sym.name),
            sym.n_type
        )))
    }

    fn defined_target(&self, object: usize, symbol: usize) -> Result<Target<'a>> {
        let input = self.object(object)?;
        let sym = input
            .symbols
            .get(symbol)
            .ok_or_else(|| Problem::internal("a symbol index out of range"))?;
        let section = usize::from(sym.n_sect).saturating_sub(1);
        let base = self.section(object, section)?.addr;
        self.atom_at(object, section, sym.n_value.saturating_sub(base))
    }

    /// What the relocation `reloc` of `section` of `object` reaches, with its addend, but
    /// without the addend that its place's bytes hold (which [`Graph::content_addend`] reads).
    /// A section-relative relocation reads the address from its place.
    pub(crate) fn reloc_target(
        &self,
        object: usize,
        section: usize,
        reloc: &Reloc,
    ) -> Result<Target<'a>> {
        match reloc.referent {
            Referent::Symbol(symbol) => Ok(add(self.symbol_target(object, symbol)?, reloc.addend)),
            Referent::Section(ordinal) => {
                if reloc.kind != input::UNSIGNED || reloc.width != 8 || reloc.minus.is_some() {
                    return Err(Problem::internal(format!(
                        "`{}`: a section-relative {} at {:#x}, which the internal linker does not support",
                        self.object(object)?.name,
                        input::reloc_name(reloc.kind),
                        reloc.offset
                    )));
                }
                let data = self.section(object, section)?.data;
                let address = bytes::u64_at(data, reloc.offset)?;
                let target_section = usize::from(ordinal).saturating_sub(1);
                let base = self.section(object, target_section)?.addr;
                let offset = address.checked_sub(base).ok_or_else(|| {
                    Problem::internal(format!(
                        "`{}`: a section-relative relocation at {:#x} points before its section",
                        self.object(object)
                            .map(|o| o.name.clone())
                            .unwrap_or_default(),
                        reloc.offset
                    ))
                })?;
                self.atom_at(object, target_section, offset)
            }
        }
    }

    /// The addend that the place of `reloc` holds: none for a section-relative relocation, whose
    /// bytes are the address itself, and for an instruction, whose addend is in its
    /// `ARM64_RELOC_ADDEND`; the signed contents for an `ARM64_RELOC_UNSIGNED` against a symbol.
    pub(crate) fn content_addend(
        &self,
        object: usize,
        section: usize,
        reloc: &Reloc,
    ) -> Result<i64> {
        if reloc.kind != input::UNSIGNED || matches!(reloc.referent, Referent::Section(_)) {
            return Ok(0);
        }
        let data = self.section(object, section)?.data;
        match reloc.width {
            8 => Ok(i64::from_le_bytes(bytes::array::<8>(data, reloc.offset)?)),
            4 => Ok(i64::from(i32::from_le_bytes(bytes::array::<4>(
                data,
                reloc.offset,
            )?))),
            _ => Err(Problem::internal(format!(
                "`{}`: an ARM64_RELOC_UNSIGNED of {} bytes at {:#x}, which the internal linker does not support",
                self.object(object)?.name,
                reloc.width,
                reloc.offset
            ))),
        }
    }
}

/// `target` moved by `addend`.
pub(crate) fn add(target: Target<'_>, addend: i64) -> Target<'_> {
    match target {
        Target::Atom { atom, offset } => Target::Atom {
            atom,
            offset: offset.saturating_add(addend),
        },
        other => other,
    }
}

fn signed(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Problem::internal("an offset too large"))
}

/// The relocations of a sorted list whose place lies in `[start, end)`.
pub(crate) fn relocs_in(relocs: &[Reloc], start: u64, end: u64) -> &[Reloc] {
    let first = relocs.partition_point(|reloc| reloc.offset < start);
    let last = relocs.partition_point(|reloc| reloc.offset < end);
    relocs.get(first..last).unwrap_or(&[])
}

/// Splits every loaded object into atoms, reads its unwind entries, and marks what is live from
/// the roots.
///
/// # Errors
///
/// U8001 on an unsupported section, an unreadable unwind entry or record, or a relocation that
/// reaches no part of the output.
pub(crate) fn build<'a>(resolved: Resolved<'a>, entry: &[u8]) -> Result<Graph<'a>> {
    let mut graph = Graph {
        resolved,
        atoms: Vec::new(),
        section_atoms: Vec::new(),
        classes: Vec::new(),
        compact: Vec::new(),
        eh_records: Vec::new(),
    };
    let count = graph.resolved.objects.len();
    graph.section_atoms = vec![Vec::new(); count];
    graph.classes = vec![Vec::new(); count];
    let order = graph.resolved.order.clone();
    for &object in &order {
        let input = graph.object(object)?;
        let mut classes = Vec::with_capacity(input.sections.len());
        let mut ranges = Vec::with_capacity(input.sections.len());
        let mut atoms = Vec::new();
        for (index, section) in input.sections.iter().enumerate() {
            let class = classify(&input.name, section)?;
            classes.push(class);
            let first = graph.atoms.len().saturating_add(atoms.len());
            if let Class::Out(out) = class {
                for (start, end) in split(input, index, section) {
                    atoms.push(Atom {
                        object,
                        section: index,
                        start,
                        end,
                        out,
                        live: false,
                    });
                }
            }
            let last = graph.atoms.len().saturating_add(atoms.len());
            ranges.push(first..last);
        }
        graph.atoms.extend(atoms);
        if let (Some(slot), Some(class_slot)) = (
            graph.section_atoms.get_mut(object),
            graph.classes.get_mut(object),
        ) {
            *slot = ranges;
            *class_slot = classes;
        }
    }
    for &object in &order {
        let sections = graph.classes.get(object).cloned().unwrap_or_default();
        for (index, class) in sections.into_iter().enumerate() {
            match class {
                Class::CompactUnwind => read_compact(&mut graph, object, index)?,
                Class::EhFrame => read_eh_frame(&mut graph, object, index)?,
                Class::Drop | Class::Out(_) => {}
            }
        }
    }
    mark(&mut graph, entry)?;
    Ok(graph)
}

/// The atoms of a section: split at its symbols, except alternate entries, when the object
/// allows it.
fn split(object: &Object<'_>, index: usize, section: &InSection<'_>) -> Vec<(u64, u64)> {
    let mut starts: Vec<u64> = vec![0];
    if object.subsections {
        let ordinal = u8::try_from(index.saturating_add(1)).unwrap_or(0);
        for symbol in &object.symbols {
            if symbol.in_section()
                && symbol.n_sect == ordinal
                && symbol.n_desc & nlist::N_ALT_ENTRY == 0
            {
                let offset = symbol.n_value.saturating_sub(section.addr);
                if offset < section.size {
                    starts.push(offset);
                }
            }
        }
    }
    starts.sort_unstable();
    starts.dedup();
    let mut out = Vec::with_capacity(starts.len());
    for (position, start) in starts.iter().enumerate() {
        let end = starts
            .get(position.saturating_add(1))
            .copied()
            .unwrap_or(section.size);
        out.push((*start, end));
    }
    out
}

/// Reads the 32-byte entries of a `__compact_unwind` section.
fn read_compact<'a>(graph: &mut Graph<'a>, object: usize, index: usize) -> Result<()> {
    let section = graph.section(object, index)?.clone();
    let name = graph.object(object)?.name.clone();
    if section.size % 32 != 0 {
        return Err(Problem::internal(format!(
            "`{name}`: a __compact_unwind section whose size is not a multiple of 32"
        )));
    }
    let mut entry = 0u64;
    while entry < section.size {
        let field = |at: u64| -> Result<Option<Target<'a>>> {
            let relocs = relocs_in(&section.relocs, entry + at, entry + at + 8);
            match relocs {
                [] => {
                    if bytes::u64_at(section.data, entry + at)? != 0 {
                        return Err(Problem::internal(format!(
                            "`{name}`: a __compact_unwind entry at {entry:#x} has an address without a relocation"
                        )));
                    }
                    Ok(None)
                }
                [reloc]
                    if reloc.kind == input::UNSIGNED
                        && reloc.width == 8
                        && reloc.offset == entry + at =>
                {
                    let target = graph.reloc_target(object, index, reloc)?;
                    let addend = graph.content_addend(object, index, reloc)?;
                    Ok(Some(add(target, addend)))
                }
                _ => Err(Problem::internal(format!(
                    "`{name}`: a __compact_unwind entry at {entry:#x} has a relocation the linker does not expect"
                ))),
            }
        };
        let function = field(0)?.ok_or_else(|| {
            Problem::internal(format!(
                "`{name}`: a __compact_unwind entry at {entry:#x} names no function"
            ))
        })?;
        let personality = field(16)?;
        let lsda = field(24)?;
        let encoding = bytes::u32_at(section.data, entry + 12)?;
        graph.compact.push(CompactEntry {
            function,
            encoding,
            personality,
            lsda,
        });
        entry += 32;
    }
    Ok(())
}

/// Reads the CIEs and FDEs of an `__eh_frame` section. The address an FDE describes is the
/// target of the pair of relocations at its `pc_begin`, read as the place-relative pointer it
/// encodes (`docs/plan/0124-step3-design.md` §8).
fn read_eh_frame<'a>(graph: &mut Graph<'a>, object: usize, index: usize) -> Result<()> {
    let section = graph.section(object, index)?.clone();
    let name = graph.object(object)?.name.clone();
    let bad = |what: String| Problem::internal(format!("`{name}`, __TEXT,__eh_frame: {what}"));
    let mut cies: BTreeMap<u64, usize> = BTreeMap::new();
    let mut at = 0u64;
    while at < section.size {
        let length = u64::from(bytes::u32_at(section.data, at)?);
        if length == 0 {
            break;
        }
        if length == 0xffff_ffff {
            return Err(bad(format!("a 64-bit record at {at:#x}")));
        }
        let end = at
            .checked_add(4)
            .and_then(|v| v.checked_add(length))
            .filter(|end| *end <= section.size)
            .ok_or_else(|| bad(format!("a record at {at:#x} runs past the section")))?;
        let id = u64::from(bytes::u32_at(section.data, at + 4)?);
        let relocs = relocs_in(&section.relocs, at, end).to_vec();
        let record = graph.eh_records.len();
        if id == 0 {
            let mut others = Vec::new();
            for reloc in &relocs {
                others.push(eh_target(graph, object, index, &section, reloc)?);
            }
            cies.insert(at, record);
            graph.eh_records.push(EhRecord {
                object,
                section: index,
                start: at,
                end,
                cie: None,
                function: None,
                others,
                live: false,
            });
        } else {
            let cie_at = (at + 4)
                .checked_sub(id)
                .ok_or_else(|| bad(format!("an FDE at {at:#x} points before the section")))?;
            let cie = *cies
                .get(&cie_at)
                .ok_or_else(|| bad(format!("an FDE at {at:#x} names no CIE before it")))?;
            let mut function = None;
            let mut others = Vec::new();
            for reloc in &relocs {
                let target = eh_target(graph, object, index, &section, reloc)?;
                if reloc.offset == at + 8 {
                    function = Some(target);
                } else {
                    others.push(target);
                }
            }
            let function = function.ok_or_else(|| {
                bad(format!(
                    "an FDE at {at:#x} has no relocation at its pc_begin"
                ))
            })?;
            if !matches!(function, Target::Atom { .. }) {
                return Err(bad(format!(
                    "an FDE at {at:#x} describes no function of the inputs"
                )));
            }
            graph.eh_records.push(EhRecord {
                object,
                section: index,
                start: at,
                end,
                cie: Some(cie),
                function: Some(function),
                others,
                live: false,
            });
        }
        at = end;
    }
    Ok(())
}

/// What a relocation of `__eh_frame` reaches. A pair is a place-relative pointer: its target is
/// the place plus the distance the input encodes, so the record may move.
pub(crate) fn eh_target<'a>(
    graph: &Graph<'a>,
    object: usize,
    index: usize,
    section: &InSection<'_>,
    reloc: &Reloc,
) -> Result<Target<'a>> {
    let name = || {
        graph
            .object(object)
            .map(|o| o.name.clone())
            .unwrap_or_default()
    };
    match (reloc.kind, reloc.minus) {
        (input::UNSIGNED, Some(Referent::Symbol(minus))) => {
            let target = graph.reloc_target(object, index, reloc)?;
            let content = graph.content_addend(object, index, reloc)?;
            let input = graph.object(object)?;
            let minus_symbol = input
                .symbols
                .get(usize::try_from(minus).unwrap_or(usize::MAX))
                .filter(|s| s.in_section() && !s.external())
                .ok_or_else(|| {
                    Problem::internal(format!(
                        "`{}`, __TEXT,__eh_frame: a pair at {:#x} subtracts a symbol that is not local to the object",
                        name(),
                        reloc.offset
                    ))
                })?;
            let place = section.addr.saturating_add(reloc.offset);
            let distance =
                i128::from(content) + i128::from(place) - i128::from(minus_symbol.n_value);
            let distance = i64::try_from(distance)
                .map_err(|_| Problem::internal("a pointer of __eh_frame out of range"))?;
            Ok(add(target, distance))
        }
        (input::POINTER_TO_GOT, None) if reloc.pcrel && reloc.width == 4 => {
            graph.reloc_target(object, index, reloc)
        }
        _ => Err(Problem::internal(format!(
            "`{}`, __TEXT,__eh_frame: {} at {:#x} is not supported there",
            name(),
            input::reloc_name(reloc.kind),
            reloc.offset
        ))),
    }
}

/// Marks the live atoms, unwind entries and records, from the roots.
fn mark<'a>(graph: &mut Graph<'a>, entry: &[u8]) -> Result<()> {
    let mut work: Vec<usize> = Vec::new();
    match graph.resolved.globals.get(entry) {
        Some(Global::Defined { object, symbol, .. }) => {
            if let Target::Atom { atom, .. } = graph.defined_target(*object, *symbol)? {
                work.push(atom);
            }
        }
        _ => {
            return Err(Problem::internal(format!(
                "the entry point `{}` is not defined",
                shown(entry)
            )));
        }
    }
    // Initializers, and what an input marks as never stripped.
    for (index, atom) in graph.atoms.iter().enumerate() {
        let section = graph.section(atom.object, atom.section)?;
        if section.flags & flags::S_ATTR_NO_DEAD_STRIP != 0
            || section.kind() == flags::S_MOD_INIT_FUNC_POINTERS
        {
            work.push(index);
        }
    }
    for &object in &graph.resolved.order {
        let input = graph.object(object)?;
        for (position, symbol) in input.symbols.iter().enumerate() {
            if symbol.in_section() && symbol.n_desc & nlist::N_NO_DEAD_STRIP != 0 {
                let section = usize::from(symbol.n_sect).saturating_sub(1);
                if matches!(
                    graph.classes.get(object).and_then(|c| c.get(section)),
                    Some(Class::Out(_))
                ) && let Target::Atom { atom, .. } = graph.defined_target(object, position)?
                {
                    work.push(atom);
                }
            }
        }
    }
    // The unwind entries and records of each function atom.
    let mut compact_of: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, entry) in graph.compact.iter().enumerate() {
        if let Target::Atom { atom, .. } = entry.function {
            compact_of.entry(atom).or_default().push(index);
        }
    }
    let mut fdes_of: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, record) in graph.eh_records.iter().enumerate() {
        if let Some(Target::Atom { atom, .. }) = record.function {
            fdes_of.entry(atom).or_default().push(index);
        }
    }
    let mut targets: Vec<Target<'a>> = Vec::new();
    while let Some(index) = work.pop() {
        let Some(atom) = graph.atoms.get_mut(index) else {
            continue;
        };
        if atom.live {
            continue;
        }
        atom.live = true;
        let (object, section) = (atom.object, atom.section);
        targets.clear();
        for reloc in graph.atom_relocs(index)? {
            targets.push(graph.reloc_target(object, section, reloc)?);
            if let Some(Referent::Symbol(minus)) = reloc.minus {
                targets.push(graph.symbol_target(object, minus)?);
            }
        }
        for entry in compact_of.get(&index).into_iter().flatten() {
            if let Some(entry) = graph.compact.get(*entry) {
                targets.extend(entry.personality);
                targets.extend(entry.lsda);
            }
        }
        for record in fdes_of.get(&index).into_iter().flatten() {
            let mut chain = vec![*record];
            if let Some(cie) = graph.eh_records.get(*record).and_then(|r| r.cie) {
                chain.push(cie);
            }
            for position in chain {
                if let Some(record) = graph.eh_records.get_mut(position) {
                    record.live = true;
                    targets.extend(record.others.iter().copied());
                }
            }
        }
        for target in targets.iter().copied() {
            if let Target::Atom { atom, .. } = target {
                work.push(atom);
            }
        }
    }
    Ok(())
}
