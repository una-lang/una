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
    /// The pointers the record holds (`pc_begin`, the LSDA, the personality), each rewritten
    /// for the output.
    pub(crate) fields: Vec<EhField<'a>>,
    /// Whether the record is kept.
    pub(crate) live: bool,
}

/// A place-relative pointer of an `__eh_frame` record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EhField<'a> {
    /// Its offset in the input section.
    pub(crate) offset: u64,
    /// Its width in bytes.
    pub(crate) width: u64,
    /// Whether it points at the GOT slot of its target (an indirect personality).
    pub(crate) got: bool,
    /// What it reaches.
    pub(crate) target: Target<'a>,
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

    /// The atom that holds `address`, an address in the object `object`.
    pub(crate) fn address_target(&self, object: usize, address: u64) -> Result<Target<'a>> {
        let input = self.object(object)?;
        for (index, section) in input.sections.iter().enumerate() {
            let placed = matches!(
                self.classes.get(object).and_then(|c| c.get(index)),
                Some(Class::Out(_))
            );
            if placed
                && address >= section.addr
                && address < section.addr.saturating_add(section.size)
            {
                return self.atom_at(object, index, address - section.addr);
            }
        }
        Err(Problem::internal(format!(
            "`{}`: an address {address:#x} that lies in no section of the output",
            input.name
        )))
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

/// Reads the CIEs and FDEs of an `__eh_frame` section (`docs/plan/0124-step3-design.md` §8).
/// Each pointer field that the augmentation of a CIE declares (`pc_begin`, the LSDA, the
/// personality) is read either from its relocation (a pair, read as the place-relative pointer it
/// encodes, or an `ARM64_RELOC_POINTER_TO_GOT`), or, where the assembler resolved it inside the
/// object, from its encoded value; so a record may be dropped or moved. A relocation that is not
/// at a pointer field, and a pointer encoding that is not place-relative, are U8001.
fn read_eh_frame<'a>(graph: &mut Graph<'a>, object: usize, index: usize) -> Result<()> {
    let section = graph.section(object, index)?.clone();
    let name = graph.object(object)?.name.clone();
    let bad = |what: String| Problem::internal(format!("`{name}`, __TEXT,__eh_frame: {what}"));
    // Each CIE's pointer encodings: (record index, FDE encoding, LSDA encoding, has `z`).
    let mut cies: BTreeMap<u64, (usize, u8, u8, bool)> = BTreeMap::new();
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
        let mut fields: Vec<EhField<'a>> = Vec::new();
        let mut cursor = Cursor {
            data: section.data,
            at: at + 8,
            end,
        };
        let take = |graph: &Graph<'a>,
                    cursor: &mut Cursor<'_>,
                    encoding: u8,
                    fields: &mut Vec<EhField<'a>>|
         -> Result<()> {
            let offset = cursor.at;
            let width = pointer_width(encoding).ok_or_else(|| {
                bad(format!("a pointer encoding {encoding:#x} at {offset:#x} that the linker does not support"))
            })?;
            let value = cursor.signed(width)?;
            let reloc = relocs.iter().find(|r| r.offset == offset);
            let field = if let Some(reloc) = reloc {
                if reloc.width != width {
                    return Err(bad(format!(
                        "a relocation at {offset:#x} whose width is not its pointer's"
                    )));
                }
                EhField {
                    offset,
                    width,
                    got: reloc.kind == input::POINTER_TO_GOT,
                    target: eh_target(graph, object, index, &section, reloc)?,
                }
            } else {
                if encoding & 0x70 != 0x10 || encoding & 0x80 != 0 {
                    return Err(bad(format!(
                        "a pointer at {offset:#x} with encoding {encoding:#x} and no relocation, which is not place-relative"
                    )));
                }
                let address = i128::from(section.addr) + i128::from(offset) + i128::from(value);
                let address = u64::try_from(address).map_err(|_| {
                    bad(format!(
                        "a pointer at {offset:#x} that leads outside the object"
                    ))
                })?;
                EhField {
                    offset,
                    width,
                    got: false,
                    target: graph.address_target(object, address)?,
                }
            };
            fields.push(field);
            Ok(())
        };
        if id == 0 {
            let version = cursor.u8()?;
            let augmentation = cursor.string()?;
            cursor.uleb()?;
            cursor.sleb()?;
            if version == 1 {
                cursor.u8()?;
            } else {
                cursor.uleb()?;
            }
            let (mut fde_encoding, mut lsda_encoding) = (0u8, 0xffu8);
            let z = augmentation.first() == Some(&b'z');
            if z {
                let length = cursor.uleb()?;
                let data_end = cursor.at.saturating_add(length);
                for letter in augmentation.iter().skip(1) {
                    match letter {
                        b'P' => {
                            let encoding = cursor.u8()?;
                            if encoding & 0x80 != 0 {
                                // An indirect personality: a pointer to its GOT slot.
                                let offset = cursor.at;
                                let width = pointer_width(encoding).unwrap_or(0);
                                let reloc = relocs.iter().find(|r| r.offset == offset).ok_or_else(|| {
                                    bad(format!("an indirect personality at {offset:#x} with no relocation"))
                                })?;
                                if reloc.kind != input::POINTER_TO_GOT || reloc.width != width {
                                    return Err(bad(format!(
                                        "an indirect personality at {offset:#x} that is not ARM64_RELOC_POINTER_TO_GOT"
                                    )));
                                }
                                cursor.signed(width)?;
                                fields.push(EhField {
                                    offset,
                                    width,
                                    got: true,
                                    target: graph.reloc_target(object, index, reloc)?,
                                });
                            } else {
                                take(graph, &mut cursor, encoding, &mut fields)?;
                            }
                        }
                        b'L' => lsda_encoding = cursor.u8()?,
                        b'R' => fde_encoding = cursor.u8()?,
                        b'S' | b'B' => {}
                        other => {
                            return Err(bad(format!(
                                "a CIE at {at:#x} with the augmentation `{}` the linker does not know",
                                char::from(*other)
                            )));
                        }
                    }
                }
                if cursor.at > data_end {
                    return Err(bad(format!(
                        "a CIE at {at:#x} whose augmentation overruns its data"
                    )));
                }
            }
            check_fields(&relocs, &fields)
                .map_err(|what| bad(format!("{what} in the CIE at {at:#x}")))?;
            cies.insert(at, (record, fde_encoding, lsda_encoding, z));
            graph.eh_records.push(EhRecord {
                object,
                section: index,
                start: at,
                end,
                cie: None,
                function: None,
                fields,
                live: false,
            });
        } else {
            // The CIE pointer: the distance back from its field to the CIE, encoded, or, as
            // `ld -r` writes it, a pair of two symbols of this section (#1833).
            let field = at + 4;
            let distance = match relocs.iter().find(|r| r.offset == field) {
                Some(reloc) => cie_distance(graph, object, index, reloc)
                    .map_err(|what| bad(format!("{what} at {field:#x}")))?,
                None => i128::from(id),
            };
            let cie_at = i128::from(field)
                .checked_sub(distance)
                .and_then(|v| u64::try_from(v).ok())
                .filter(|v| *v < at)
                .ok_or_else(|| bad(format!("an FDE at {at:#x} points before the section")))?;
            let (cie, fde_encoding, lsda_encoding, z) = *cies
                .get(&cie_at)
                .ok_or_else(|| bad(format!("an FDE at {at:#x} names no CIE before it")))?;
            take(graph, &mut cursor, fde_encoding, &mut fields)?;
            let function = fields.first().map(|f| f.target);
            // pc_range: a length, not a pointer.
            cursor.signed(pointer_width(fde_encoding & 0x0f).unwrap_or(8))?;
            if z {
                cursor.uleb()?;
                if lsda_encoding != 0xff {
                    let offset = cursor.at;
                    let width = pointer_width(lsda_encoding).unwrap_or(8);
                    let has_reloc = relocs.iter().any(|r| r.offset == offset);
                    let value = Cursor {
                        data: section.data,
                        at: offset,
                        end,
                    }
                    .signed(width)?;
                    if has_reloc || value != 0 {
                        take(graph, &mut cursor, lsda_encoding, &mut fields)?;
                    }
                }
            }
            let others: Vec<Reloc> = relocs
                .iter()
                .filter(|r| r.offset != field)
                .cloned()
                .collect();
            check_fields(&others, &fields)
                .map_err(|what| bad(format!("{what} in the FDE at {at:#x}")))?;
            let function =
                function.ok_or_else(|| bad(format!("an FDE at {at:#x} names no function")))?;
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
                fields,
                live: false,
            });
        }
        at = end;
    }
    Ok(())
}

/// The value of an FDE's CIE pointer that `reloc` encodes: an `ARM64_RELOC_UNSIGNED` of 4 bytes
/// paired with an `ARM64_RELOC_SUBTRACTOR`, both naming local symbols of this `__eh_frame`, so
/// the value is fixed by the input's own addresses: target - subtracted + the place's content.
/// The linker writes the pointer anew for the CIE it finds (`unwind::write_eh_frame`).
fn cie_distance(
    graph: &Graph<'_>,
    object: usize,
    index: usize,
    reloc: &Reloc,
) -> std::result::Result<i128, String> {
    let input = graph.object(object).map_err(|_| "no object".to_owned())?;
    let local = |referent: Option<Referent>| match referent {
        Some(Referent::Symbol(symbol)) => input
            .symbols
            .get(usize::try_from(symbol).unwrap_or(usize::MAX))
            .filter(|s| {
                s.in_section() && !s.external() && usize::from(s.n_sect) == index.saturating_add(1)
            })
            .map(|s| s.n_value),
        _ => None,
    };
    let pair = (local(Some(reloc.referent)), local(reloc.minus));
    let (Some(target), Some(minus)) = pair else {
        return Err(format!(
            "a CIE pointer {} that is not a pair of local symbols of the section",
            input::reloc_name(reloc.kind)
        ));
    };
    if reloc.kind != input::UNSIGNED || reloc.width != 4 || reloc.pcrel || reloc.addend != 0 {
        return Err("a CIE pointer relocation that is not a 4-byte pair".to_owned());
    }
    let content = graph
        .content_addend(object, index, reloc)
        .map_err(|_| "a CIE pointer outside its section".to_owned())?;
    Ok(i128::from(target) - i128::from(minus) + i128::from(content))
}

/// Every relocation of a record must be at one of its pointer fields.
fn check_fields(relocs: &[Reloc], fields: &[EhField<'_>]) -> std::result::Result<(), String> {
    for reloc in relocs {
        if !fields.iter().any(|f| f.offset == reloc.offset) {
            return Err(format!(
                "{} at {:#x}, which is not at a pointer field",
                input::reloc_name(reloc.kind),
                reloc.offset
            ));
        }
    }
    Ok(())
}

/// The width of a DWARF pointer encoding's format, for the formats the linker supports.
fn pointer_width(encoding: u8) -> Option<u64> {
    match encoding & 0x0f {
        0x00 | 0x04 | 0x0c => Some(8),
        0x03 | 0x0b => Some(4),
        _ => None,
    }
}

/// The bits of one LEB128 group at `shift`, refusing a number wider than 64 bits.
fn leb_part(bits: u64, shift: u32) -> Result<u64> {
    let too_wide = || Problem::internal("an LEB128 number of __eh_frame wider than 64 bits");
    if shift >= 64 {
        return if bits == 0 && shift < 70 {
            Ok(0)
        } else {
            Err(too_wide())
        };
    }
    let part = bits.checked_shl(shift).ok_or_else(too_wide)?;
    if part >> shift != bits {
        return Err(too_wide());
    }
    Ok(part)
}

/// A reader over a record's bytes.
struct Cursor<'d> {
    data: &'d [u8],
    at: u64,
    end: u64,
}

impl Cursor<'_> {
    fn u8(&mut self) -> Result<u8> {
        if self.at >= self.end {
            return Err(Problem::internal("an __eh_frame record ends early"));
        }
        let [byte] = bytes::array::<1>(self.data, self.at)?;
        self.at += 1;
        Ok(byte)
    }

    fn string(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let byte = self.u8()?;
            if byte == 0 {
                return Ok(out);
            }
            out.push(byte);
        }
    }

    fn uleb(&mut self) -> Result<u64> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            value |= leb_part(u64::from(byte & 0x7f), shift)?;
            shift += 7;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
    }

    fn sleb(&mut self) -> Result<i64> {
        let mut value = 0i64;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            value |= leb_part(u64::from(byte & 0x7f), shift)?.cast_signed();
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    value |= (-1i64).checked_shl(shift).unwrap_or(0);
                }
                return Ok(value);
            }
        }
    }

    fn signed(&mut self, width: u64) -> Result<i64> {
        if self.at.saturating_add(width) > self.end {
            return Err(Problem::internal("an __eh_frame record ends early"));
        }
        let value = match width {
            4 => i64::from(i32::from_le_bytes(bytes::array::<4>(self.data, self.at)?)),
            8 => i64::from_le_bytes(bytes::array::<8>(self.data, self.at)?),
            _ => {
                return Err(Problem::internal(
                    "a pointer width the linker does not support",
                ));
            }
        };
        self.at += width;
        Ok(value)
    }
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
                    targets.extend(record.fields.iter().map(|f| f.target));
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
