//! The relocation engine of rule 24 for the ELF targets: which relocation types are supported,
//! what each needs (a GOT slot, a relative fixup, a relaxation), and the patching of one input
//! section's bytes with the addresses of a layout.
//!
//! A link uses it in two passes over each kept input section. [`plan`] runs before addresses
//! exist: from the kind of each relocation's referent (absolute, an address in the image, a
//! thread-local variable, an undefined weak symbol), it decides the action at every place,
//! refuses what rule 24 refuses, and lists the symbols that need a GOT slot, so the layout can
//! allocate them. [`apply`] runs after the layout: it plans again with the layout's referents
//! (the same decision, since only the kind decides), then writes each place.
//!
//! Rules, the same on both targets:
//! - a type outside the target's list is U8001, naming the type, the input, the section and the
//!   symbol;
//! - a narrow absolute type only against an absolute symbol (`SHN_ABS`); a 64-bit absolute
//!   type against an address only in a writable section, where it becomes a relative fixup
//!   (`R_*_RELATIVE`), and against an undefined weak symbol (the absolute 0) only there too,
//!   otherwise U8001 (a text relocation), as the scan of 4a decides;
//! - a PC-relative type only against an address in the image: against an absolute symbol, an
//!   undefined weak one or a thread-local variable it would be wrong once the executable loads
//!   at a random address, so it is U8001, but for a call (`PLT32` on a `call` or `jmp`,
//!   `CALL26`, `JUMP26`) to an undefined weak function, resolved as `ld.lld` does;
//! - thread-local types only against thread-local variables, and the other types never against
//!   them; every thread-local access is relaxed to the local-exec model (one module, a static
//!   executable), so no GOT slot ever holds a thread-local offset;
//! - the bytes a relocation (or a relaxed sequence) patches lie inside its own section, and the
//!   ranges of two relocations never overlap; a place in a zero-fill section is U8001;
//! - a value that does not fit its field is U8001 when the input alone decides it (an absolute
//!   value, an alignment), U7015 when the layout's distances do (a branch beyond its reach, a
//!   PC-relative displacement beyond ±2 GiB).
//!
//! O(R log R) per section of R relocations (they are sorted by the reader; each is decided and
//! written in O(1)).

use std::collections::BTreeSet;

use crate::elf::input::{ElfObject, Rela, Section};
use crate::elf::{Arch, aarch64, x86_64};
use crate::problem::{Code, Problem, Result};

/// What a relocation's symbol resolves to, as resolution and the layout give it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Referent {
    /// An absolute value (`SHN_ABS`), which the load does not move.
    Absolute(u64),
    /// An address in the image, at its link-time value (the image's first byte is at 0; the
    /// executable loads at a random base, which relative fixups add).
    Address(u64),
    /// A thread-local variable, at this offset from the start of the TLS template (`PT_TLS`).
    Tls(u64),
    /// An undefined weak symbol: the absolute 0.
    UndefinedWeak,
}

impl Referent {
    /// The referent's kind in messages.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Referent::Absolute(_) => "an absolute symbol",
            Referent::Address(_) => "an address in the image",
            Referent::Tls(_) => "a thread-local variable",
            Referent::UndefinedWeak => "an undefined weak symbol",
        }
    }
}

/// The TLS template of the executable: `PT_TLS`'s memory size and alignment.
///
/// Precondition for the layout (slice 4c): `PT_TLS` is placed at an address that is a multiple
/// of its alignment. The x86-64 offsets of [`tp_offset`] (variant II, `offset - align_up(size,
/// align)`) are those musl's start-up and `ld.lld` compute only then; the template's address is
/// not an input of the engine, so the layout must guarantee it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tls {
    /// `p_memsz`.
    pub(crate) size: u64,
    /// `p_align`, a power of two (1 for 0).
    pub(crate) align: u64,
}

/// What a layout gives the engine.
pub(crate) trait Layout {
    /// What symbol `symbol` of the object resolves to.
    ///
    /// # Errors
    ///
    /// U8001 when it resolves to nothing the executable has.
    fn referent(&self, symbol: u32) -> Result<Referent>;
    /// The address of the GOT slot of symbol `symbol` (one that [`plan`] listed).
    ///
    /// # Errors
    ///
    /// U8001 when the layout allocated none.
    fn got(&self, symbol: u32) -> Result<u64>;
    /// The TLS template.
    ///
    /// # Errors
    ///
    /// U8001 when the executable has none.
    fn tls(&self) -> Result<Tls>;
}

/// The class of a relocation type for the position-independence rules of rule 24.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Supported, with no restriction of the class.
    Plain,
    /// A narrow absolute type: only against an absolute symbol, or outside loaded sections.
    Narrow,
    /// A 64-bit absolute type: against an address only in a writable section.
    Wide,
}

/// One supported relocation type of a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Type {
    /// Its number, as the ABI gives it.
    pub(crate) value: u32,
    /// Its name.
    pub(crate) name: &'static str,
    /// Its class.
    pub(crate) class: Class,
}

/// The supported list of `arch` (rule 24, `spec/16` "Linking").
pub(crate) fn types(arch: Arch) -> &'static [Type] {
    match arch {
        Arch::X86_64 => x86_64::TYPES,
        Arch::Aarch64 => aarch64::TYPES,
    }
}

/// The entry of type `value` in `arch`'s list.
pub(crate) fn find(arch: Arch, value: u32) -> Option<&'static Type> {
    types(arch).iter().find(|t| t.value == value)
}

/// The name of type `value` of `arch` for messages.
pub(crate) fn type_name(arch: Arch, value: u32) -> String {
    if let Some(t) = find(arch, value) {
        return t.name.to_owned();
    }
    match (arch, value) {
        (Arch::X86_64, x86_64::R_IRELATIVE) => "R_X86_64_IRELATIVE".to_owned(),
        (Arch::Aarch64, aarch64::R_IRELATIVE) => "R_AARCH64_IRELATIVE".to_owned(),
        (Arch::X86_64, _) => format!("x86-64 relocation type {value}"),
        (Arch::Aarch64, _) => format!("arm64 relocation type {value}"),
    }
}

/// What the engine does at one relocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Action {
    /// Computes the type's value from the referent and writes it to the type's field.
    Direct,
    /// Computes the value from the referent's GOT slot.
    Got,
    /// A load from the GOT relaxed into an address computation (x86-64 `mov` to `lea`, and the
    /// indirect `call` and `jmp` to direct ones), so the GOT holds no slot for it.
    RelaxGot,
    /// A 64-bit pointer to an address in the image: written, and recorded as a relative fixup.
    Relative,
    /// A thread-local access relaxed to the local-exec model.
    RelaxTls,
    /// Patched by the relaxation of the relocation before it (the call to `__tls_get_addr`).
    Consumed,
}

impl Action {
    /// The action's name in counts and messages.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Action::Direct => "direct",
            Action::Got => "got",
            Action::RelaxGot => "relaxed-got",
            Action::Relative => "relative",
            Action::RelaxTls => "relaxed-tls",
            Action::Consumed => "consumed",
        }
    }
}

/// The decision at one relocation: its action and the bytes of the section it patches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    /// The relocation's index in its section's (sorted) list.
    pub(crate) index: usize,
    /// What is done.
    pub(crate) action: Action,
    /// The first byte patched, as an offset into the section.
    pub(crate) start: u64,
    /// One past the last byte patched.
    pub(crate) end: u64,
}

/// The plan of one section: a step per relocation, and the symbols that need a GOT slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    /// One step per relocation, in the section's order.
    pub(crate) steps: Vec<Step>,
    /// The symbols (indices into the object's table) that need a GOT slot.
    pub(crate) got: BTreeSet<u32>,
}

/// A relative fixup: at `place` (an address in the image), the load adds the image's base to
/// `addend` (`R_X86_64_RELATIVE`, `R_AARCH64_RELATIVE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Fixup {
    /// The address of the 8 bytes fixed.
    pub(crate) place: u64,
    /// The link-time address they hold.
    pub(crate) addend: u64,
}

/// What a GOT slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GotEntry {
    /// The link-time value of the slot.
    pub(crate) value: u64,
    /// Whether the slot needs a relative fixup (it holds an address in the image).
    pub(crate) relative: bool,
}

/// What the GOT slot of `referent` holds.
///
/// # Errors
///
/// U8001 for a thread-local variable, whose accesses are all relaxed (no GOT slot holds one).
pub(crate) fn got_entry(referent: Referent) -> Result<GotEntry> {
    match referent {
        Referent::Address(value) => Ok(GotEntry {
            value,
            relative: true,
        }),
        Referent::Absolute(value) => Ok(GotEntry {
            value,
            relative: false,
        }),
        Referent::UndefinedWeak => Ok(GotEntry {
            value: 0,
            relative: false,
        }),
        Referent::Tls(_) => Err(Problem::internal(
            "a GOT slot for a thread-local variable, which every access relaxes away",
        )),
    }
}

/// The context of one relocation, for the per-target modules and their messages.
pub(crate) struct Site<'o, 'a> {
    /// The object.
    pub(crate) object: &'o ElfObject<'a>,
    /// The section's index.
    pub(crate) section: usize,
    /// The section's relocations.
    pub(crate) relocs: &'o [Rela],
    /// This relocation's index.
    pub(crate) index: usize,
    /// This relocation.
    pub(crate) rela: Rela,
}

impl Site<'_, '_> {
    /// The section.
    pub(crate) fn section(&self) -> Option<&Section<'_>> {
        self.object.sections.get(self.section)
    }

    /// The section's bytes (empty for zero-fill).
    pub(crate) fn data(&self) -> &[u8] {
        self.section().map_or(&[][..], |s| s.data)
    }

    /// A problem of `code` at this relocation: `why` after the type, the input, the section, the
    /// offset and the symbol.
    pub(crate) fn problem(&self, code: Code, why: &str) -> Problem {
        let message = format!(
            "`{}` has {} in `{}` at {:#x} against `{}`: {why}",
            self.object.name,
            type_name(self.object.arch, self.rela.kind),
            self.object.section_name(self.section),
            self.rela.offset,
            self.object.symbol_name(self.rela.symbol)
        );
        Problem { code, message }
    }

    /// A U8001 problem at this relocation.
    pub(crate) fn refused(&self, why: &str) -> Problem {
        self.problem(Code::U8001, why)
    }

    /// The bytes `[start, end)` of the section as offsets, checked to lie inside it; `start` is
    /// the place plus `before` (which may be negative).
    pub(crate) fn range(&self, before: i64, width: u64) -> Result<(u64, u64)> {
        let start = i128::from(self.rela.offset) + i128::from(before);
        let size = self.section().map_or(0, |s| s.size);
        let end = start + i128::from(width);
        if start < 0 || end > i128::from(size) {
            return Err(self.refused(&format!(
                "the {width} bytes it patches lie outside the section ({size:#x} bytes)"
            )));
        }
        Ok((
            u64::try_from(start).unwrap_or(u64::MAX),
            u64::try_from(end).unwrap_or(u64::MAX),
        ))
    }

    /// The byte at `before` from the place (inside the section, which [`Site::range`] checked).
    pub(crate) fn byte(&self, before: i64) -> Result<u8> {
        let at = i128::from(self.rela.offset) + i128::from(before);
        usize::try_from(at)
            .ok()
            .and_then(|at| self.data().get(at).copied())
            .ok_or_else(|| self.refused("the instruction it patches lies outside the section"))
    }

    /// The next relocation of the section, if any.
    pub(crate) fn next(&self) -> Option<&Rela> {
        self.relocs.get(self.index.saturating_add(1))
    }
}

/// Plans the relocations of section `section` of `object`, with `referent` giving what each
/// symbol resolves to (only its kind is used). A section the executable does not contain gets
/// an empty plan: its relocations are ignored (rule 24).
///
/// # Errors
///
/// U8001 on a relocation that rule 24 refuses; U7015 never (no address is known yet).
pub(crate) fn plan(
    object: &ElfObject<'_>,
    section: usize,
    referent: &dyn Fn(u32) -> Result<Referent>,
) -> Result<Plan> {
    let this = object.section(section)?;
    let mut out = Plan::default();
    if !this.kept() {
        return Ok(out);
    }
    let relocs = &this.relocs;
    let mut consumed_until: Option<usize> = None;
    let mut last_end: u64 = 0;
    for (index, rela) in relocs.iter().enumerate() {
        let site = Site {
            object,
            section,
            relocs,
            index,
            rela: *rela,
        };
        if consumed_until.is_some_and(|until| index <= until) {
            out.steps.push(Step {
                index,
                action: Action::Consumed,
                start: rela.offset,
                end: rela.offset,
            });
            continue;
        }
        if this.zerofill() {
            return Err(site.refused("a relocation of a zero-fill section, which has no bytes"));
        }
        if find(object.arch, rela.kind).is_none() {
            return Err(site.refused("the type is outside the supported set (ADR 0124 rule 24)"));
        }
        let referent = referent(rela.symbol)?;
        let decision = match object.arch {
            Arch::X86_64 => x86_64::decide(&site, referent)?,
            Arch::Aarch64 => aarch64::decide(&site, referent)?,
        };
        if decision.start < last_end && index > 0 {
            return Err(
                site.refused("the bytes it patches overlap those of the relocation before it")
            );
        }
        last_end = decision.end;
        if decision.consumes > 0 {
            consumed_until = Some(index.saturating_add(decision.consumes));
        }
        if decision.action == Action::Got {
            out.got.insert(rela.symbol);
        }
        out.steps.push(Step {
            index,
            action: decision.action,
            start: decision.start,
            end: decision.end,
        });
    }
    Ok(out)
}

/// A per-target decision: the action, the bytes patched, and how many of the following
/// relocations the relaxed sequence consumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Decision {
    /// What is done.
    pub(crate) action: Action,
    /// The first byte patched (an offset into the section).
    pub(crate) start: u64,
    /// One past the last.
    pub(crate) end: u64,
    /// How many relocations after this one the sequence consumes.
    pub(crate) consumes: usize,
}

/// Applies the relocations of section `section` of `object` to `bytes`, that section's own bytes
/// in the output, which the layout placed at `address`. Returns the relative fixups the section
/// needs, in place order.
///
/// # Errors
///
/// U8001 on a relocation that rule 24 refuses or a value that does not fit its field; U7015 on a
/// distance beyond a field's reach.
pub(crate) fn apply(
    object: &ElfObject<'_>,
    section: usize,
    bytes: &mut [u8],
    address: u64,
    layout: &dyn Layout,
) -> Result<Vec<Fixup>> {
    let this = object.section(section)?;
    let plan = plan(object, section, &|symbol| layout.referent(symbol))?;
    if !plan.steps.is_empty() && u64::try_from(bytes.len()).ok() != Some(this.size) {
        return Err(Problem::internal(format!(
            "`{}` section `{}`: the output holds {} bytes for a section of {:#x}",
            object.name,
            object.section_name(section),
            bytes.len(),
            this.size
        )));
    }
    let mut fixups = Vec::new();
    for step in &plan.steps {
        if step.action == Action::Consumed {
            continue;
        }
        let Some(rela) = this.relocs.get(step.index) else {
            continue;
        };
        let site = Site {
            object,
            section,
            relocs: &this.relocs,
            index: step.index,
            rela: *rela,
        };
        let place = address
            .checked_add(rela.offset)
            .ok_or_else(|| site.problem(Code::U7015, "its place lies beyond the address space"))?;
        let referent = layout.referent(rela.symbol)?;
        let fixup = match object.arch {
            Arch::X86_64 => x86_64::write(&site, step, bytes, place, referent, layout)?,
            Arch::Aarch64 => aarch64::write(&site, step, bytes, place, referent, layout)?,
        };
        fixups.extend(fixup);
    }
    Ok(fixups)
}

/// `S + A` as a 64-bit field: two's complement of a value in `[i64::MIN, u64::MAX]`.
pub(crate) fn wide(site: &Site<'_, '_>, value: i128) -> Result<u64> {
    if value < i128::from(i64::MIN) || value > i128::from(u64::MAX) {
        return Err(site.refused("the value does not fit 64 bits"));
    }
    Ok(u64::try_from(value.rem_euclid(1i128 << 64)).unwrap_or(0))
}

/// `value` reduced to 64 bits and read as a signed number: ELF's arithmetic on absolute values
/// is modulo 2^64, so `-8` written as an absolute symbol's `st_value` is `0xffff_ffff_ffff_fff8`,
/// which fits a signed 32-bit field.
pub(crate) fn as_signed(value: i128) -> i128 {
    let low = u64::try_from(value.rem_euclid(1i128 << 64)).unwrap_or(0);
    i128::from(i64::from_le_bytes(low.to_le_bytes()))
}

/// `value` reduced to 64 bits and read as an unsigned number.
pub(crate) fn as_unsigned(value: i128) -> i128 {
    value.rem_euclid(1i128 << 64)
}

/// `value` in a signed field of `bits` bits, or `code` naming the range.
pub(crate) fn signed(site: &Site<'_, '_>, value: i128, bits: u32, code: Code) -> Result<u64> {
    let half = 1i128 << (bits - 1);
    if value < -half || value >= half {
        return Err(site.problem(
            code,
            &format!("the value {value:#x} does not fit a signed field of {bits} bits"),
        ));
    }
    Ok(u64::try_from(value.rem_euclid(1i128 << bits)).unwrap_or(0))
}

/// `value` in an unsigned field of `bits` bits.
pub(crate) fn unsigned(site: &Site<'_, '_>, value: i128, bits: u32, code: Code) -> Result<u64> {
    if value < 0 || value >= 1i128 << bits {
        return Err(site.problem(
            code,
            &format!("the value {value:#x} does not fit an unsigned field of {bits} bits"),
        ));
    }
    Ok(u64::try_from(value).unwrap_or(0))
}

/// `value` in a field of `bits` bits that holds either a signed or an unsigned value (the ABI's
/// `-2^(bits-1) <= X < 2^bits`).
pub(crate) fn either(site: &Site<'_, '_>, value: i128, bits: u32, code: Code) -> Result<u64> {
    if value < -(1i128 << (bits - 1)) || value >= 1i128 << bits {
        return Err(site.problem(
            code,
            &format!("the value {value:#x} does not fit a field of {bits} bits"),
        ));
    }
    Ok(u64::try_from(value.rem_euclid(1i128 << bits)).unwrap_or(0))
}

/// The thread-pointer offset of the variable at `offset` in the TLS template, for `arch`:
/// variant II on x86-64 (the template ends at the thread pointer, its size rounded up to its
/// alignment), variant I on arm64 (two words of thread control block, rounded up to the
/// template's alignment, then the template).
pub(crate) fn tp_offset(site: &Site<'_, '_>, arch: Arch, offset: u64, tls: Tls) -> Result<i128> {
    let align = tls.align.max(1);
    if !align.is_power_of_two() {
        return Err(site.refused("the TLS template's alignment is not a power of two"));
    }
    let round = |value: u64| -> Result<i128> {
        value
            .checked_next_multiple_of(align)
            .map(i128::from)
            .ok_or_else(|| site.problem(Code::U7015, "the TLS template is too large"))
    };
    if offset > tls.size {
        return Err(site.refused("the thread-local variable lies outside the TLS template"));
    }
    match arch {
        Arch::X86_64 => Ok(i128::from(offset) - round(tls.size)?),
        Arch::Aarch64 => Ok(round(16)? + i128::from(offset)),
    }
}

/// Reads the little-endian `u32` at offset `at` of `bytes`.
pub(crate) fn read32(site: &Site<'_, '_>, bytes: &[u8], at: u64) -> Result<u32> {
    crate::bytes::u32_at(bytes, at).map_err(|_| site.refused("its place lies outside the section"))
}

/// Writes `value` at offset `at` of `bytes`.
pub(crate) fn put(site: &Site<'_, '_>, bytes: &mut [u8], at: u64, value: &[u8]) -> Result<()> {
    crate::bytes::put(bytes, at, value)
        .map_err(|_| site.refused("its place lies outside the section"))
}
