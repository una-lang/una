//! The layout (ADR 0124 rule 25, `docs/plan/0124-step3-design.md` §5): what goes into the GOT
//! and the stubs, the addresses of every live atom, the bytes of every output section with the
//! relocations applied (rule 24), and the pointers that become chained fixups.

use std::collections::{BTreeMap, BTreeSet};

use crate::bytes::{self, align_up};
use crate::gc::{Graph, Target, relocs_in};
use crate::input::{self, Referent, Reloc};
use crate::problem::{Problem, Result, shown};
use crate::reloc;
use crate::sections::{ALL, Out, Seg};
use crate::unwind::{self, Addresses};

/// The address of the Mach-O header: `__TEXT` follows the 4 GiB `__PAGEZERO`.
pub(crate) const BASE: u64 = 0x1_0000_0000;
/// The alignment of segments (16 KiB, the page size of arm64 macOS).
pub(crate) const SEGMENT_ALIGN: u64 = 0x4000;
/// The size of a stub.
const STUB_SIZE: u64 = 12;

/// A pointer that dyld writes at load: a rebase to an address of the image, or a bind to an
/// import with an addend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fixup<'a> {
    /// The unslid address the pointer holds.
    Rebase(u64),
    /// The import and the addend.
    Bind(&'a [u8], i64),
}

/// An output section with its address and bytes.
#[derive(Clone, Debug)]
pub(crate) struct OutSection {
    /// Which section.
    pub(crate) out: Out,
    /// Its address.
    pub(crate) addr: u64,
    /// Its size.
    pub(crate) size: u64,
    /// Its alignment, as a power of two.
    pub(crate) align: u32,
    /// Its bytes, empty for a zero-fill section.
    pub(crate) bytes: Vec<u8>,
    /// `reserved1`: the first index into the indirect symbol table, for `__got` and `__stubs`.
    pub(crate) reserved1: u32,
    /// `reserved2`: the stub size, for `__stubs`.
    pub(crate) reserved2: u32,
}

/// A symbol of the output's symbol table.
#[derive(Clone, Debug)]
pub(crate) struct OutSymbol<'a> {
    /// Its name.
    pub(crate) name: &'a [u8],
    /// `n_type`.
    pub(crate) n_type: u8,
    /// The output section's ordinal (1-based), 0 for an import.
    pub(crate) n_sect: u8,
    /// `n_desc`.
    pub(crate) n_desc: u16,
    /// Its address.
    pub(crate) value: u64,
}

/// Everything the writer of the file needs.
pub(crate) struct Image<'a> {
    /// The present output sections, in layout order.
    pub(crate) sections: Vec<OutSection>,
    /// The pointers to fix up at load, by address.
    pub(crate) fixups: BTreeMap<u64, Fixup<'a>>,
    /// The imports, sorted by name, with whether each is a weak reference.
    pub(crate) imports: Vec<(&'a [u8], bool)>,
    /// The GOT slots, in order: what each holds.
    pub(crate) got: Vec<Target<'a>>,
    /// The stubs, in order: the import each calls.
    pub(crate) stubs: Vec<&'a [u8]>,
    /// The symbols: locals, then defined externals sorted by name, then imports sorted by name.
    pub(crate) symbols: Vec<OutSymbol<'a>>,
    /// How many of [`Image::symbols`] are local, and how many are defined externals.
    pub(crate) symbol_counts: (u32, u32),
    /// The start of each function, sorted.
    pub(crate) function_starts: Vec<u64>,
    /// The address of the entry point.
    pub(crate) entry: u64,
    /// Whether the image has thread-local variables.
    pub(crate) tlv: bool,
    /// The size of the header with its load commands.
    pub(crate) header_size: u64,
}

/// What the layout keeps while it assigns addresses.
struct Placing<'a> {
    atom_addr: Vec<u64>,
    got_index: BTreeMap<Target<'a>, usize>,
    got_addr: u64,
    stubs_addr: u64,
    stub_index: BTreeMap<&'a [u8], usize>,
}

impl<'a> Placing<'a> {
    /// The address of the stub that calls `name`.
    fn stub(&self, name: &'a [u8]) -> Result<u64> {
        let index = self
            .stub_index
            .get(name)
            .copied()
            .ok_or_else(|| Problem::internal(format!("no stub for `{}`", shown(name))))?;
        Ok(self.stubs_addr + STUB_SIZE * u64::try_from(index).unwrap_or(0))
    }
}

impl<'a> Addresses<'a> for Placing<'a> {
    fn address(&self, target: Target<'a>) -> Result<u64> {
        match target {
            Target::Atom { atom, offset } => {
                let base = self
                    .atom_addr
                    .get(atom)
                    .copied()
                    .filter(|a| *a != u64::MAX)
                    .ok_or_else(|| {
                        Problem::internal("a reference to code or data that was removed")
                    })?;
                u64::try_from(i128::from(base) + i128::from(offset))
                    .map_err(|_| Problem::internal("an address out of range"))
            }
            Target::Header => Ok(BASE),
            Target::Import(name) => Err(Problem::internal(format!(
                "a direct reference to `{}`, which libSystem defines, needs a stub or a GOT slot",
                shown(name)
            ))),
        }
    }

    fn got(&self, target: Target<'a>) -> Result<u64> {
        let index = self
            .got_index
            .get(&target)
            .copied()
            .ok_or_else(|| Problem::internal("a GOT slot that was not planned"))?;
        Ok(self.got_addr + 8 * u64::try_from(index).unwrap_or(0))
    }

    fn stubs(&self) -> u64 {
        self.stubs_addr
    }
}

/// The size of the header and of the load commands that [`crate::commands`] writes, for `counts`
/// sections in `__TEXT`, `__DATA_CONST` and `__DATA`.
pub(crate) fn header_size(counts: [u64; 3]) -> u64 {
    let segments: u64 = counts.iter().filter(|n| **n > 0).map(|n| 72 + 80 * n).sum();
    // __PAGEZERO and __LINKEDIT, then chained fixups, exports trie, symtab, dysymtab, dylinker,
    // uuid, build version, main, dylib, function starts, data in code, code signature.
    32 + 72 + segments + 72 + 16 + 16 + 24 + 80 + 32 + 24 + 24 + 24 + 56 + 16 + 16 + 16
}

/// Lays out the live atoms of `graph` and applies their relocations.
///
/// # Errors
///
/// U8001 on an import outside `exports` or a relocation that rule 24 refuses; U7015 when a
/// branch or a section exceeds its reach.
pub(crate) fn lay_out<'a>(
    graph: &Graph<'a>,
    entry: &[u8],
    exports: &dyn Fn(&[u8]) -> bool,
) -> Result<Image<'a>> {
    // 1. The imports, the GOT and the stubs.
    let mut imports: BTreeMap<&'a [u8], bool> = BTreeMap::new();
    let mut got: BTreeSet<Target<'a>> = BTreeSet::new();
    let mut stubs: BTreeSet<&'a [u8]> = BTreeSet::new();
    let need_import = |target: Target<'a>, imports: &mut BTreeMap<&'a [u8], bool>| {
        if let Target::Import(name) = target {
            imports.entry(name).or_insert(false);
        }
    };
    for (index, atom) in graph.atoms.iter().enumerate() {
        if !atom.live {
            continue;
        }
        for reloc in graph.atom_relocs(index)? {
            let target = graph.reloc_target(atom.object, atom.section, reloc)?;
            need_import(target, &mut imports);
            match (reloc.kind, target) {
                (input::BRANCH26, Target::Import(name)) => {
                    stubs.insert(name);
                    got.insert(target);
                }
                (input::GOT_LOAD_PAGE21 | input::GOT_LOAD_PAGEOFF12, Target::Import(_))
                | (input::POINTER_TO_GOT, _) => {
                    got.insert(target);
                }
                _ => {}
            }
        }
    }
    for record in graph.eh_records.iter().filter(|r| r.live) {
        for field in &record.fields {
            if field.got {
                need_import(field.target, &mut imports);
                got.insert(field.target);
            } else if let Target::Import(name) = field.target {
                return Err(Problem::internal(format!(
                    "an __eh_frame record points at `{}`, which no input defines",
                    shown(name)
                )));
            }
        }
    }
    let unwind_plan = unwind::plan(graph, !stubs.is_empty())?;
    for personality in &unwind_plan.personalities {
        need_import(*personality, &mut imports);
        got.insert(*personality);
    }
    // Weak references among the imports: one pass over the loaded objects' symbols.
    let weak_references: BTreeSet<&'a [u8]> = graph
        .resolved
        .objects
        .iter()
        .flat_map(|object| object.symbols.iter())
        .filter(|s| s.undefined() && s.n_desc & input::nlist::N_WEAK_REF != 0)
        .map(|s| s.name)
        .collect();
    for (name, weak) in &mut imports {
        *weak = weak_references.contains(name);
    }
    for name in imports.keys() {
        if !exports(name) {
            return Err(Problem::internal(format!(
                "the symbol `{}` is defined by no input and is not in the export list of libSystem built into `una` (from the macOS 12.1 SDK)",
                shown(name)
            )));
        }
    }
    let got: Vec<Target<'a>> = got.into_iter().collect();
    let stubs: Vec<&'a [u8]> = stubs.into_iter().collect();

    // 2. Which sections are present, and the header's size.
    let mut present: BTreeSet<Out> = BTreeSet::new();
    for atom in graph.atoms.iter().filter(|a| a.live) {
        present.insert(atom.out);
    }
    if !stubs.is_empty() {
        present.insert(Out::Stubs);
    }
    if !got.is_empty() {
        present.insert(Out::Got);
    }
    if unwind_plan.size > 0 && present.contains(&Out::Text) {
        present.insert(Out::UnwindInfo);
    }
    if unwind_plan.eh_size > 0 {
        present.insert(Out::EhFrame);
    }
    let mut counts = [0u64; 3];
    for out in &present {
        let slot = match out.segment() {
            Seg::Text => 0,
            Seg::DataConst => 1,
            Seg::Data => 2,
        };
        if let Some(count) = counts.get_mut(slot) {
            *count += 1;
        }
    }
    let header = header_size(counts);

    // 3. Addresses.
    let mut placing = Placing {
        atom_addr: vec![u64::MAX; graph.atoms.len()],
        got_index: got.iter().enumerate().map(|(i, t)| (*t, i)).collect(),
        got_addr: 0,
        stubs_addr: 0,
        stub_index: stubs.iter().enumerate().map(|(i, n)| (*n, i)).collect(),
    };
    let mut by_out: BTreeMap<Out, Vec<usize>> = BTreeMap::new();
    for (index, atom) in graph.atoms.iter().enumerate() {
        if atom.live {
            by_out.entry(atom.out).or_default().push(index);
        }
    }
    let mut sections: Vec<OutSection> = Vec::new();
    let mut cursor = BASE + header;
    let mut segment = Seg::Text;
    for out in ALL {
        if !present.contains(&out) {
            continue;
        }
        if out.segment() != segment {
            segment = out.segment();
            cursor = align_up(cursor, SEGMENT_ALIGN);
        }
        let (align, size) = match out {
            Out::Stubs => (2u32, STUB_SIZE * u64::try_from(stubs.len()).unwrap_or(0)),
            Out::Got => (3, 8 * u64::try_from(got.len()).unwrap_or(0)),
            Out::UnwindInfo => (2, unwind_plan.size),
            Out::EhFrame => (3, unwind_plan.eh_size),
            _ => {
                let atoms = by_out.get(&out).map_or(&[][..], Vec::as_slice);
                let mut align = 0u32;
                for index in atoms {
                    let atom = graph.atom(*index)?;
                    align = align.max(graph.section(atom.object, atom.section)?.align);
                }
                (align, 0)
            }
        };
        // Every address is checked against the 4 GiB image as it is made, so no size an input
        // states can wrap the arithmetic (U7015).
        let too_large =
            || Problem::layout("the executable is larger than the 4 GiB this layout supports");
        let limit = BASE + (1u64 << 32);
        let within = |value: Option<u64>| value.filter(|v| *v < limit).ok_or_else(too_large);
        let start = within(Some(align_up(cursor, 1u64 << align)))?;
        let mut end = within(start.checked_add(size))?;
        if let Some(atoms) = by_out.get(&out) {
            let mut at = start;
            for index in atoms {
                let atom = graph.atom(*index)?;
                let section = graph.section(atom.object, atom.section)?;
                let modulus = 1u64 << section.align;
                let wanted = section.addr.checked_add(atom.start).ok_or_else(too_large)? % modulus;
                let here = at % modulus;
                at = within(at.checked_add((wanted + modulus - here) % modulus))?;
                if let Some(slot) = placing.atom_addr.get_mut(*index) {
                    *slot = at;
                }
                let length = atom.end.checked_sub(atom.start).ok_or_else(too_large)?;
                at = within(at.checked_add(length))?;
            }
            end = end.max(at);
        }
        match out {
            Out::Got => placing.got_addr = start,
            Out::Stubs => placing.stubs_addr = start,
            _ => {}
        }
        sections.push(OutSection {
            out,
            addr: start,
            size: end - start,
            align,
            bytes: if out.zerofill() {
                Vec::new()
            } else {
                vec![0; usize::try_from(end - start).unwrap_or(0)]
            },
            reserved1: 0,
            reserved2: 0,
        });
        cursor = end;
    }
    let find = |sections: &[OutSection], out: Out| sections.iter().position(|s| s.out == out);
    // The thread-local template starts at the first of __thread_data and __thread_bss.
    let tlv_start = sections
        .iter()
        .find(|s| matches!(s.out, Out::ThreadData | Out::ThreadBss))
        .map(|s| s.addr);

    // 4. The atoms' bytes and their relocations.
    let mut fixups: BTreeMap<u64, Fixup<'a>> = BTreeMap::new();
    for (index, atom) in graph.atoms.iter().enumerate() {
        if !atom.live {
            continue;
        }
        let input = graph.section(atom.object, atom.section)?;
        let position = find(&sections, atom.out)
            .ok_or_else(|| Problem::internal("an atom of a section that is not laid out"))?;
        let Some(out_section) = sections.get_mut(position) else {
            continue;
        };
        let addr = placing.atom_addr.get(index).copied().unwrap_or(0);
        let offset = addr - out_section.addr;
        if !atom.out.zerofill() {
            let data = input
                .data
                .get(
                    usize::try_from(atom.start).unwrap_or(usize::MAX)
                        ..usize::try_from(atom.end).unwrap_or(usize::MAX),
                )
                .ok_or_else(|| Problem::internal("an atom outside its section's bytes"))?;
            bytes::put(&mut out_section.bytes, offset, data)?;
        } else if !input.relocs.is_empty() {
            return Err(Problem::internal(format!(
                "`{}`: a zero-fill section {},{} with relocations",
                graph.object(atom.object)?.name,
                input.segment,
                input.name
            )));
        }
        for reloc in relocs_in(&input.relocs, atom.start, atom.end) {
            let at = offset + (reloc.offset - atom.start);
            let place = out_section.addr + at;
            apply(
                graph,
                &placing,
                atom.object,
                atom.section,
                atom.out,
                reloc,
                &mut out_section.bytes,
                at,
                place,
                tlv_start,
                &mut fixups,
            )
            .map_err(|problem| context(graph, atom.object, input, reloc, problem))?;
        }
    }

    // 5. The GOT, the stubs, __eh_frame and __unwind_info.
    if let Some(position) = find(&sections, Out::Got)
        && let Some(section) = sections.get_mut(position)
    {
        for (slot, target) in got.iter().enumerate() {
            let address = section.addr + 8 * u64::try_from(slot).unwrap_or(0);
            let fixup = match target {
                Target::Import(name) => Fixup::Bind(name, 0),
                other => Fixup::Rebase(placing.address(*other)?),
            };
            fixups.insert(address, fixup);
        }
        section.reserved1 = u32::try_from(stubs.len()).unwrap_or(0);
    }
    if let Some(position) = find(&sections, Out::Stubs)
        && let Some(section) = sections.get_mut(position)
    {
        for (index, name) in stubs.iter().enumerate() {
            let place = section.addr + STUB_SIZE * u64::try_from(index).unwrap_or(0);
            let slot = placing.got(Target::Import(name))?;
            for (word, instruction) in reloc::stub(place, slot)?.iter().enumerate() {
                bytes::put_u32(
                    &mut section.bytes,
                    place - section.addr + 4 * u64::try_from(word).unwrap_or(0),
                    *instruction,
                )?;
            }
        }
        section.reserved1 = 0;
        section.reserved2 = u32::try_from(STUB_SIZE).unwrap_or(0);
    }
    if let Some(position) = find(&sections, Out::EhFrame) {
        let base = sections.get(position).map_or(0, |s| s.addr);
        let bytes_out = unwind::write_eh_frame(graph, &unwind_plan, &placing, base)?;
        if let Some(section) = sections.get_mut(position) {
            section.bytes = bytes_out;
        }
    }
    let code_end = sections
        .iter()
        .filter(|s| s.out.code())
        .map(|s| s.addr + s.size)
        .max()
        .unwrap_or(BASE);
    if let Some(position) = find(&sections, Out::UnwindInfo) {
        let bytes_out = unwind::write_unwind_info(graph, &unwind_plan, &placing, BASE, code_end)?;
        if let Some(section) = sections.get_mut(position) {
            section.bytes = bytes_out;
        }
    }

    // 6. The entry, the symbols and the function starts.
    let entry_address = match graph.resolved.globals.get(entry) {
        Some(crate::resolve::Global::Defined { object, symbol, .. }) => {
            let target =
                graph.symbol_target(*object, u32::try_from(*symbol).unwrap_or(u32::MAX))?;
            placing.address(target)?
        }
        _ => {
            return Err(Problem::internal(format!(
                "the entry point `{}` is not defined",
                shown(entry)
            )));
        }
    };
    let ordinal_of = |out: Out| {
        sections
            .iter()
            .position(|s| s.out == out)
            .and_then(|p| u8::try_from(p.saturating_add(1)).ok())
            .unwrap_or(0)
    };
    let mut locals = Vec::new();
    let mut externals: BTreeMap<&'a [u8], OutSymbol<'a>> = BTreeMap::new();
    for &object in &graph.resolved.order {
        let input_object = graph.object(object)?;
        for (position, symbol) in input_object.symbols.iter().enumerate() {
            if !symbol.in_section() || symbol.name.is_empty() {
                continue;
            }
            // Assembler-local names (`l…`, `L…`) are not kept, as ld64 does not keep them.
            if matches!(symbol.name.first(), Some(b'l' | b'L')) {
                continue;
            }
            let section = usize::from(symbol.n_sect).saturating_sub(1);
            if !matches!(
                graph.classes.get(object).and_then(|c| c.get(section)),
                Some(crate::sections::Class::Out(_))
            ) {
                continue;
            }
            let target =
                graph.symbol_target(object, u32::try_from(position).unwrap_or(u32::MAX))?;
            let Target::Atom { atom, .. } = target else {
                continue;
            };
            let atom_info = graph.atom(atom)?;
            if !atom_info.live {
                continue;
            }
            if symbol.external() {
                // A definition that lost to another is not listed.
                let wins = matches!(
                    graph.resolved.globals.get(symbol.name),
                    Some(crate::resolve::Global::Defined { object: o, symbol: s, .. }) if *o == object && *s == position
                );
                if !wins {
                    continue;
                }
            }
            // A private extern is local to the executable, as ld64 writes it: `N_PEXT` without
            // `N_EXT`.
            let private = symbol.n_type & input::nlist::N_PEXT != 0;
            let n_type = if private {
                input::nlist::N_SECT | input::nlist::N_PEXT
            } else {
                symbol.n_type & (input::nlist::N_TYPE | input::nlist::N_EXT)
            };
            let out_symbol = OutSymbol {
                name: symbol.name,
                n_type,
                n_sect: ordinal_of(atom_info.out),
                n_desc: symbol.n_desc & input::nlist::N_WEAK_DEF,
                value: placing.address(target)?,
            };
            if symbol.external() && !private {
                externals.insert(symbol.name, out_symbol);
            } else {
                locals.push(out_symbol);
            }
        }
    }
    if matches!(
        graph.resolved.globals.get(crate::resolve::HEADER_SYMBOL),
        Some(crate::resolve::Global::Header)
    ) {
        externals.insert(
            crate::resolve::HEADER_SYMBOL,
            OutSymbol {
                name: crate::resolve::HEADER_SYMBOL,
                n_type: input::nlist::N_SECT | input::nlist::N_EXT,
                n_sect: 1,
                n_desc: 0x10,
                value: BASE,
            },
        );
    }
    let local_count = u32::try_from(locals.len()).unwrap_or(u32::MAX);
    let external_count = u32::try_from(externals.len()).unwrap_or(u32::MAX);
    let mut symbols = locals;
    symbols.extend(externals.into_values());
    for (name, weak) in &imports {
        symbols.push(OutSymbol {
            name,
            n_type: input::nlist::N_UNDF | input::nlist::N_EXT,
            n_sect: 0,
            // Library ordinal 1 (libSystem) in the high byte, two-level namespace.
            n_desc: 0x100 | if *weak { input::nlist::N_WEAK_REF } else { 0 },
            value: 0,
        });
    }
    let mut function_starts: Vec<u64> = graph
        .atoms
        .iter()
        .enumerate()
        .filter(|(_, a)| a.live && a.out == Out::Text && a.end > a.start)
        .filter_map(|(i, _)| placing.atom_addr.get(i).copied())
        .collect();
    function_starts.sort_unstable();
    function_starts.dedup();

    Ok(Image {
        sections,
        fixups,
        imports: imports.into_iter().collect(),
        got,
        stubs,
        symbols,
        symbol_counts: (local_count, external_count),
        function_starts,
        entry: entry_address,
        tlv: tlv_start.is_some(),
        header_size: header,
    })
}

/// A relocation's problem with its input, section and symbol (rule 24, Errors).
fn context(
    graph: &Graph<'_>,
    object: usize,
    section: &input::InSection<'_>,
    reloc: &Reloc,
    problem: Problem,
) -> Problem {
    let symbol = match reloc.referent {
        Referent::Symbol(index) => graph
            .object(object)
            .ok()
            .and_then(|o| o.symbols.get(usize::try_from(index).unwrap_or(usize::MAX)))
            .map_or_else(String::new, |s| format!(" against `{}`", shown(s.name))),
        Referent::Section(ordinal) => format!(" against section {ordinal}"),
    };
    let name = graph
        .object(object)
        .map(|o| o.name.clone())
        .unwrap_or_default();
    Problem {
        code: problem.code,
        message: format!(
            "`{name}`, section {},{}: {} at {:#x}{symbol}: {}",
            section.segment,
            section.name,
            input::reloc_name(reloc.kind),
            reloc.offset,
            problem.message
        ),
    }
}

/// Applies one relocation at `at` of `bytes_out`, whose address is `place`.
#[allow(clippy::too_many_arguments)]
fn apply<'a>(
    graph: &Graph<'a>,
    placing: &Placing<'a>,
    object: usize,
    section: usize,
    out: Out,
    reloc: &Reloc,
    bytes_out: &mut [u8],
    at: u64,
    place: u64,
    tlv_start: Option<u64>,
    fixups: &mut BTreeMap<u64, Fixup<'a>>,
) -> Result<()> {
    let target = graph.reloc_target(object, section, reloc)?;
    let instruction = || bytes::u32_at(bytes_out, at);
    let local = !matches!(target, Target::Import(_));
    match reloc.kind {
        input::BRANCH26 => {
            let to = match target {
                Target::Import(name) => placing.stub(name)?,
                other => placing.address(other)?,
            };
            let patched = reloc::branch26(instruction()?, place, to)?;
            bytes::put_u32(bytes_out, at, patched)
        }
        input::PAGE21 => {
            let to = placing.address(target)?;
            bytes::put_u32(bytes_out, at, reloc::page21(instruction()?, place, to)?)
        }
        input::PAGEOFF12 => {
            let to = placing.address(target)?;
            bytes::put_u32(bytes_out, at, reloc::pageoff12(instruction()?, place, to)?)
        }
        input::GOT_LOAD_PAGE21 => {
            let to = if local {
                placing.address(target)?
            } else {
                placing.got(target)?
            };
            bytes::put_u32(bytes_out, at, reloc::page21(instruction()?, place, to)?)
        }
        input::GOT_LOAD_PAGEOFF12 => {
            let patched = if local {
                reloc::relax_ldr(instruction()?, place, placing.address(target)?)?
            } else {
                reloc::pageoff12(instruction()?, place, placing.got(target)?)?
            };
            bytes::put_u32(bytes_out, at, patched)
        }
        input::TLVP_LOAD_PAGE21 | input::TLVP_LOAD_PAGEOFF12 => {
            if !local {
                return Err(Problem::internal(
                    "a thread-local variable that no input defines",
                ));
            }
            if let Target::Atom { atom, .. } = target
                && graph.atom(atom)?.out != Out::ThreadVars
            {
                return Err(Problem::internal(
                    "a thread-local load whose target is not a thread-local variable",
                ));
            }
            let to = placing.address(target)?;
            let patched = if reloc.kind == input::TLVP_LOAD_PAGE21 {
                reloc::page21(instruction()?, place, to)?
            } else {
                reloc::relax_ldr(instruction()?, place, to)?
            };
            bytes::put_u32(bytes_out, at, patched)
        }
        input::POINTER_TO_GOT => {
            if !(reloc.pcrel && reloc.width == 4) {
                return Err(Problem::internal(
                    "a pointer to a GOT slot that is not 32-bit place-relative",
                ));
            }
            let value = i128::from(placing.got(target)?) - i128::from(place);
            unwind::write_signed(bytes_out, at, 4, value)
        }
        input::UNSIGNED => {
            let content = graph.content_addend(object, section, reloc)?;
            if let Some(minus) = reloc.minus {
                // A distance between two symbols: resolved here, wherever it is.
                let Referent::Symbol(minus) = minus else {
                    return Err(Problem::internal("a subtractor that names no symbol"));
                };
                let minus = graph.symbol_target(object, minus)?;
                let value = i128::from(placing.address(target)?) + i128::from(content)
                    - i128::from(placing.address(minus)?);
                return unwind::write_signed(bytes_out, at, reloc.width, value);
            }
            if reloc.width != 8 || reloc.pcrel {
                return Err(Problem::internal(
                    "a 32-bit or place-relative absolute pointer, which no load-time fixup can express",
                ));
            }
            if out.segment() == Seg::Text {
                return Err(Problem::internal(
                    "an absolute pointer in __TEXT, which is not writable at load",
                ));
            }
            if out == Out::ThreadVars {
                match reloc.offset % 24 {
                    0 => {}
                    16 => {
                        // The offset of the variable's initial value in the thread-local template.
                        let start = tlv_start.ok_or_else(|| {
                            Problem::internal(
                                "a thread-local variable with no thread-local template",
                            )
                        })?;
                        let address = i128::from(placing.address(crate::gc::add(target, content))?);
                        let value = address - i128::from(start);
                        return unwind::write_signed(bytes_out, at, 8, value);
                    }
                    _ => {
                        return Err(Problem::internal(
                            "a relocation in the key field of a thread-local variable",
                        ));
                    }
                }
            }
            if !place.is_multiple_of(8) {
                return Err(Problem::internal(
                    "an absolute pointer at an address that is not 8-aligned, which chained fixups cannot express",
                ));
            }
            let fixup = match target {
                Target::Import(name) => Fixup::Bind(name, content),
                other => Fixup::Rebase(placing.address(crate::gc::add(other, content))?),
            };
            bytes::put_u64(bytes_out, at, 0)?;
            fixups.insert(place, fixup);
            Ok(())
        }
        other => Err(Problem::internal(format!(
            "{} is not supported by the internal linker",
            input::reloc_name(other)
        ))),
    }
}
