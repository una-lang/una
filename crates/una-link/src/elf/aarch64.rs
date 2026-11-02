//! The arm64 relocations of rule 24 and their relaxations, as the ELF for the Arm 64-bit
//! Architecture (AAELF64, sections 5.7 and 5.7.11 to 5.7.12) and `ld.lld` give them for a static
//! position-independent executable:
//!
//! - data: `ABS64` (a relative fixup against an address, in a writable section only), `PREL64`,
//!   `PREL32`, and `ABS32` and `MOVW_UABS_G0` to `G3` (absolute symbols only);
//! - code: `CALL26`, `JUMP26`, `CONDBR19`, `TSTBR14` (no thunks: beyond the reach is U7015),
//!   `ADR_PREL_LO21`, `ADR_PREL_PG_HI21`, `ADD_ABS_LO12_NC`, `LDST8` to `LDST128_ABS_LO12_NC`
//!   (the access size of the instruction must be the type's), `ADR_GOT_PAGE` and
//!   `LD64_GOT_LO12_NC` through a GOT slot (with an addend of 0: a slot holds a symbol's address);
//! - thread-local data in the local-exec model: the `TLSLE_*` types that `ld.lld` applies (the
//!   `MOVW_TPREL` group, `ADD_TPREL_HI12`, and the unchecked `_NC` forms of `ADD_TPREL_LO12`
//!   and `LDST8` to `LDST128_TPREL_LO12`; the checked forms of the last two, which no input uses
//!   and `ld.lld` does not know, are not supported, so no type lacks its oracle); the
//!   initial-exec pair
//!   `TLSIE_ADR_GOTTPREL_PAGE21`/`TLSIE_LD64_GOTTPREL_LO12_NC` relaxed to `movz`/`movk` of the
//!   offset; the descriptor sequence `TLSDESC_ADR_PAGE21`/`LD64_LO12`/`ADD_LO12`/`CALL` relaxed
//!   to `movz x0`/`movk x0`/`nop`/`nop` (AAELF64's relaxation table). The tiny-model descriptor
//!   types (`TLSDESC_LD_PREL19`, `TLSDESC_ADR_PREL21`) are not supported: no input uses them
//!   and `ld.lld` does not relax them.
//!
//! Every type checks that its instruction is the class the ABI patches (an `ADRP` for a page,
//! a load or store of the type's size for an offset, a branch of the type's form), and refuses
//! (U8001) any other, rather than writing a field into an instruction that has none.

use crate::elf::reloc::{self, Action, Class, Decision, Fixup, Layout, Referent, Site, Step, Type};
use crate::problem::{Code, Result};

pub(crate) const R_ABS64: u32 = 257;
pub(crate) const R_ABS32: u32 = 258;
pub(crate) const R_PREL64: u32 = 260;
pub(crate) const R_PREL32: u32 = 261;
pub(crate) const R_MOVW_UABS_G0: u32 = 263;
pub(crate) const R_MOVW_UABS_G0_NC: u32 = 264;
pub(crate) const R_MOVW_UABS_G1: u32 = 265;
pub(crate) const R_MOVW_UABS_G1_NC: u32 = 266;
pub(crate) const R_MOVW_UABS_G2: u32 = 267;
pub(crate) const R_MOVW_UABS_G2_NC: u32 = 268;
pub(crate) const R_MOVW_UABS_G3: u32 = 269;
pub(crate) const R_ADR_PREL_LO21: u32 = 274;
pub(crate) const R_ADR_PREL_PG_HI21: u32 = 275;
pub(crate) const R_ADD_ABS_LO12_NC: u32 = 277;
pub(crate) const R_LDST8_ABS_LO12_NC: u32 = 278;
pub(crate) const R_TSTBR14: u32 = 279;
pub(crate) const R_CONDBR19: u32 = 280;
pub(crate) const R_JUMP26: u32 = 282;
pub(crate) const R_CALL26: u32 = 283;
pub(crate) const R_LDST16_ABS_LO12_NC: u32 = 284;
pub(crate) const R_LDST32_ABS_LO12_NC: u32 = 285;
pub(crate) const R_LDST64_ABS_LO12_NC: u32 = 286;
pub(crate) const R_LDST128_ABS_LO12_NC: u32 = 299;
pub(crate) const R_ADR_GOT_PAGE: u32 = 311;
pub(crate) const R_LD64_GOT_LO12_NC: u32 = 312;
pub(crate) const R_TLSIE_ADR_GOTTPREL_PAGE21: u32 = 541;
pub(crate) const R_TLSIE_LD64_GOTTPREL_LO12_NC: u32 = 542;
pub(crate) const R_TLSLE_MOVW_TPREL_G2: u32 = 544;
pub(crate) const R_TLSLE_MOVW_TPREL_G1: u32 = 545;
pub(crate) const R_TLSLE_MOVW_TPREL_G1_NC: u32 = 546;
pub(crate) const R_TLSLE_MOVW_TPREL_G0: u32 = 547;
pub(crate) const R_TLSLE_MOVW_TPREL_G0_NC: u32 = 548;
pub(crate) const R_TLSLE_ADD_TPREL_HI12: u32 = 549;
pub(crate) const R_TLSLE_ADD_TPREL_LO12_NC: u32 = 551;
pub(crate) const R_TLSLE_LDST8_TPREL_LO12_NC: u32 = 553;
pub(crate) const R_TLSLE_LDST16_TPREL_LO12_NC: u32 = 555;
pub(crate) const R_TLSLE_LDST32_TPREL_LO12_NC: u32 = 557;
pub(crate) const R_TLSLE_LDST64_TPREL_LO12_NC: u32 = 559;
pub(crate) const R_TLSDESC_ADR_PAGE21: u32 = 562;
pub(crate) const R_TLSDESC_LD64_LO12: u32 = 563;
pub(crate) const R_TLSDESC_ADD_LO12: u32 = 564;
pub(crate) const R_TLSDESC_CALL: u32 = 569;
pub(crate) const R_TLSLE_LDST128_TPREL_LO12_NC: u32 = 571;
pub(crate) const R_IRELATIVE: u32 = 1032;

macro_rules! types {
    ($($value:ident, $name:literal, $class:ident;)*) => {
        /// Rule 24's list for arm64.
        pub(crate) const TYPES: &[Type] = &[
            $(Type { value: $value, name: $name, class: Class::$class },)*
        ];
    };
}

types! {
    R_ABS64, "R_AARCH64_ABS64", Wide;
    R_ABS32, "R_AARCH64_ABS32", Narrow;
    R_PREL64, "R_AARCH64_PREL64", Plain;
    R_PREL32, "R_AARCH64_PREL32", Plain;
    R_MOVW_UABS_G0, "R_AARCH64_MOVW_UABS_G0", Narrow;
    R_MOVW_UABS_G0_NC, "R_AARCH64_MOVW_UABS_G0_NC", Narrow;
    R_MOVW_UABS_G1, "R_AARCH64_MOVW_UABS_G1", Narrow;
    R_MOVW_UABS_G1_NC, "R_AARCH64_MOVW_UABS_G1_NC", Narrow;
    R_MOVW_UABS_G2, "R_AARCH64_MOVW_UABS_G2", Narrow;
    R_MOVW_UABS_G2_NC, "R_AARCH64_MOVW_UABS_G2_NC", Narrow;
    R_MOVW_UABS_G3, "R_AARCH64_MOVW_UABS_G3", Narrow;
    R_ADR_PREL_LO21, "R_AARCH64_ADR_PREL_LO21", Plain;
    R_ADR_PREL_PG_HI21, "R_AARCH64_ADR_PREL_PG_HI21", Plain;
    R_ADD_ABS_LO12_NC, "R_AARCH64_ADD_ABS_LO12_NC", Plain;
    R_LDST8_ABS_LO12_NC, "R_AARCH64_LDST8_ABS_LO12_NC", Plain;
    R_TSTBR14, "R_AARCH64_TSTBR14", Plain;
    R_CONDBR19, "R_AARCH64_CONDBR19", Plain;
    R_JUMP26, "R_AARCH64_JUMP26", Plain;
    R_CALL26, "R_AARCH64_CALL26", Plain;
    R_LDST16_ABS_LO12_NC, "R_AARCH64_LDST16_ABS_LO12_NC", Plain;
    R_LDST32_ABS_LO12_NC, "R_AARCH64_LDST32_ABS_LO12_NC", Plain;
    R_LDST64_ABS_LO12_NC, "R_AARCH64_LDST64_ABS_LO12_NC", Plain;
    R_LDST128_ABS_LO12_NC, "R_AARCH64_LDST128_ABS_LO12_NC", Plain;
    R_ADR_GOT_PAGE, "R_AARCH64_ADR_GOT_PAGE", Plain;
    R_LD64_GOT_LO12_NC, "R_AARCH64_LD64_GOT_LO12_NC", Plain;
    R_TLSIE_ADR_GOTTPREL_PAGE21, "R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21", Plain;
    R_TLSIE_LD64_GOTTPREL_LO12_NC, "R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC", Plain;
    R_TLSLE_MOVW_TPREL_G2, "R_AARCH64_TLSLE_MOVW_TPREL_G2", Plain;
    R_TLSLE_MOVW_TPREL_G1, "R_AARCH64_TLSLE_MOVW_TPREL_G1", Plain;
    R_TLSLE_MOVW_TPREL_G1_NC, "R_AARCH64_TLSLE_MOVW_TPREL_G1_NC", Plain;
    R_TLSLE_MOVW_TPREL_G0, "R_AARCH64_TLSLE_MOVW_TPREL_G0", Plain;
    R_TLSLE_MOVW_TPREL_G0_NC, "R_AARCH64_TLSLE_MOVW_TPREL_G0_NC", Plain;
    R_TLSLE_ADD_TPREL_HI12, "R_AARCH64_TLSLE_ADD_TPREL_HI12", Plain;
    R_TLSLE_ADD_TPREL_LO12_NC, "R_AARCH64_TLSLE_ADD_TPREL_LO12_NC", Plain;
    R_TLSLE_LDST8_TPREL_LO12_NC, "R_AARCH64_TLSLE_LDST8_TPREL_LO12_NC", Plain;
    R_TLSLE_LDST16_TPREL_LO12_NC, "R_AARCH64_TLSLE_LDST16_TPREL_LO12_NC", Plain;
    R_TLSLE_LDST32_TPREL_LO12_NC, "R_AARCH64_TLSLE_LDST32_TPREL_LO12_NC", Plain;
    R_TLSLE_LDST64_TPREL_LO12_NC, "R_AARCH64_TLSLE_LDST64_TPREL_LO12_NC", Plain;
    R_TLSLE_LDST128_TPREL_LO12_NC, "R_AARCH64_TLSLE_LDST128_TPREL_LO12_NC", Plain;
    R_TLSDESC_ADR_PAGE21, "R_AARCH64_TLSDESC_ADR_PAGE21", Plain;
    R_TLSDESC_LD64_LO12, "R_AARCH64_TLSDESC_LD64_LO12", Plain;
    R_TLSDESC_ADD_LO12, "R_AARCH64_TLSDESC_ADD_LO12", Plain;
    R_TLSDESC_CALL, "R_AARCH64_TLSDESC_CALL", Plain;
}

/// `NOP`.
const NOP: u32 = 0xd503_201f;

/// The instruction classes the relocations patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Insn {
    /// `ADRP`.
    Adrp,
    /// `ADR`.
    Adr,
    /// `ADD` or `ADDS` (immediate), unshifted (`lsl #0`).
    AddLo,
    /// `ADD` or `ADDS` (immediate), shifted by 12.
    AddHi,
    /// A load or store with an unsigned 12-bit offset of `1 << scale` bytes.
    LoadStore(u32),
    /// `MOVZ` or `MOVK`.
    MovUnsigned,
    /// `MOVZ` or `MOVN`.
    MovSigned,
    /// `MOVK` or `MOVZ` (the `_NC` parts of a sequence).
    MovPart,
    /// `B` or `BL`.
    Branch26,
    /// `B.cond`, `CBZ` or `CBNZ`.
    Branch19,
    /// `TBZ` or `TBNZ`.
    Branch14,
    /// `BLR`.
    Blr,
}

/// Whether `insn` is of class `class`.
fn is(insn: u32, class: Insn) -> bool {
    match class {
        Insn::Adrp => insn & 0x9f00_0000 == 0x9000_0000,
        Insn::Adr => insn & 0x9f00_0000 == 0x1000_0000,
        Insn::AddLo => insn & 0x5fc0_0000 == 0x1100_0000,
        Insn::AddHi => insn & 0x5fc0_0000 == 0x1140_0000,
        Insn::LoadStore(scale) => insn & 0x3b00_0000 == 0x3900_0000 && access(insn) == scale,
        Insn::MovUnsigned => matches!(insn & 0x7f80_0000, 0x5280_0000 | 0x7280_0000),
        Insn::MovSigned => matches!(insn & 0x7f80_0000, 0x5280_0000 | 0x1280_0000),
        Insn::MovPart => matches!(insn & 0x7f80_0000, 0x7280_0000 | 0x5280_0000),
        Insn::Branch26 => insn & 0x7c00_0000 == 0x1400_0000,
        Insn::Branch19 => insn & 0xff00_0010 == 0x5400_0000 || insn & 0x7e00_0000 == 0x3400_0000,
        Insn::Branch14 => insn & 0x7e00_0000 == 0x3600_0000,
        Insn::Blr => insn & 0xffff_fc1f == 0xd63f_0000,
    }
}

/// The access size of a load or store with an unsigned offset, as a power of two.
fn access(insn: u32) -> u32 {
    let size = insn >> 30;
    let vector = (insn >> 26) & 1;
    let opc = (insn >> 22) & 3;
    if vector == 1 && size == 0 && opc & 2 != 0 {
        4
    } else {
        size
    }
}

/// The instruction class and, for offsets, the scale that each code type patches; `None` for a
/// data type.
fn class(kind: u32) -> Option<Insn> {
    Some(match kind {
        R_ADR_PREL_PG_HI21
        | R_ADR_GOT_PAGE
        | R_TLSIE_ADR_GOTTPREL_PAGE21
        | R_TLSDESC_ADR_PAGE21 => Insn::Adrp,
        R_ADR_PREL_LO21 => Insn::Adr,
        R_ADD_ABS_LO12_NC | R_TLSLE_ADD_TPREL_LO12_NC | R_TLSDESC_ADD_LO12 => Insn::AddLo,
        R_TLSLE_ADD_TPREL_HI12 => Insn::AddHi,
        R_LDST8_ABS_LO12_NC | R_TLSLE_LDST8_TPREL_LO12_NC => Insn::LoadStore(0),
        R_LDST16_ABS_LO12_NC | R_TLSLE_LDST16_TPREL_LO12_NC => Insn::LoadStore(1),
        R_LDST32_ABS_LO12_NC | R_TLSLE_LDST32_TPREL_LO12_NC => Insn::LoadStore(2),
        R_LDST64_ABS_LO12_NC
        | R_LD64_GOT_LO12_NC
        | R_TLSIE_LD64_GOTTPREL_LO12_NC
        | R_TLSDESC_LD64_LO12
        | R_TLSLE_LDST64_TPREL_LO12_NC => Insn::LoadStore(3),
        R_LDST128_ABS_LO12_NC | R_TLSLE_LDST128_TPREL_LO12_NC => Insn::LoadStore(4),
        R_MOVW_UABS_G0 | R_MOVW_UABS_G1 | R_MOVW_UABS_G2 | R_MOVW_UABS_G3 => Insn::MovUnsigned,
        R_MOVW_UABS_G0_NC | R_MOVW_UABS_G1_NC | R_MOVW_UABS_G2_NC => Insn::MovUnsigned,
        R_TLSLE_MOVW_TPREL_G2 | R_TLSLE_MOVW_TPREL_G1 | R_TLSLE_MOVW_TPREL_G0 => Insn::MovSigned,
        R_TLSLE_MOVW_TPREL_G1_NC | R_TLSLE_MOVW_TPREL_G0_NC => Insn::MovPart,
        R_JUMP26 | R_CALL26 => Insn::Branch26,
        R_CONDBR19 => Insn::Branch19,
        R_TSTBR14 => Insn::Branch14,
        R_TLSDESC_CALL => Insn::Blr,
        _ => return None,
    })
}

/// Whether `kind` is a thread-local type.
fn thread_local(kind: u32) -> bool {
    (R_TLSIE_ADR_GOTTPREL_PAGE21..=R_TLSLE_LDST128_TPREL_LO12_NC).contains(&kind)
}

/// Whether `kind` is PC-relative (its value depends on where the executable loads).
fn pc_relative(kind: u32) -> bool {
    matches!(
        kind,
        R_PREL64
            | R_PREL32
            | R_ADR_PREL_LO21
            | R_ADR_PREL_PG_HI21
            | R_TSTBR14
            | R_CONDBR19
            | R_JUMP26
            | R_CALL26
    )
}

/// The instruction at the place.
fn instruction(site: &Site<'_, '_>) -> Result<u32> {
    if !site.rela.offset.is_multiple_of(4) {
        return Err(site.refused("an instruction relocation at a place that is not 4-byte aligned"));
    }
    reloc::read32(site, site.data(), site.rela.offset)
}

/// Decides the action at one relocation, from its type and its referent's kind.
pub(crate) fn decide(site: &Site<'_, '_>, referent: Referent) -> Result<Decision> {
    let rela = site.rela;
    let kind = rela.kind;
    let width = if matches!(kind, R_ABS64 | R_PREL64) {
        8
    } else {
        4
    };
    let (start, end) = site.range(0, width)?;
    let decision = |action| Decision {
        action,
        start,
        end,
        consumes: 0,
    };
    if let Some(class) = class(kind) {
        let insn = instruction(site)?;
        if !is(insn, class) {
            return Err(site.refused(&format!(
                "the instruction it patches ({insn:#010x}) is not of the class the type patches"
            )));
        }
        check_registers(site, kind, insn)?;
    }
    let is_tls = matches!(referent, Referent::Tls(_));
    if thread_local(kind) != is_tls {
        return Err(site.refused(&if is_tls {
            "a thread-local variable through a type that is not thread-local".to_owned()
        } else {
            format!("a thread-local type against {}", referent.describe())
        }));
    }
    if pc_relative(kind)
        && !matches!(referent, Referent::Address(_))
        && !(referent == Referent::UndefinedWeak && matches!(kind, R_CALL26 | R_JUMP26))
    {
        return Err(site.refused(&format!(
            "a PC-relative type against {}, which a position-independent executable cannot reach",
            referent.describe()
        )));
    }
    let class = reloc::find(crate::elf::Arch::Aarch64, kind).map(|t| t.class);
    match (class, referent) {
        // Rule 24: a narrow absolute type only against an absolute symbol (`SHN_ABS`), and a
        // 64-bit one against anything else only in a writable section, as the scan of 4a says.
        (Some(reloc::Class::Narrow), Referent::Address(_) | Referent::UndefinedWeak) => {
            return Err(site.refused(
                "a narrow absolute relocation against a symbol that is not absolute, which no load-time fixup can express",
            ));
        }
        (Some(reloc::Class::Wide), Referent::Address(_) | Referent::UndefinedWeak) => {
            if site.section().is_some_and(|s| s.writable()) {
                return Ok(decision(if referent == Referent::UndefinedWeak {
                    // The absolute 0: no fixup.
                    Action::Direct
                } else {
                    Action::Relative
                }));
            }
            return Err(site.refused(
                "a 64-bit absolute relocation in a section that is not writable, a text relocation",
            ));
        }
        _ => {}
    }
    Ok(match kind {
        R_ADR_GOT_PAGE | R_LD64_GOT_LO12_NC => {
            if rela.addend != 0 {
                return Err(site.refused("a GOT relocation with an addend, which no slot holds"));
            }
            decision(Action::Got)
        }
        R_TLSIE_ADR_GOTTPREL_PAGE21
        | R_TLSIE_LD64_GOTTPREL_LO12_NC
        | R_TLSDESC_ADR_PAGE21
        | R_TLSDESC_LD64_LO12
        | R_TLSDESC_ADD_LO12
        | R_TLSDESC_CALL => decision(Action::RelaxTls),
        _ => decision(Action::Direct),
    })
}

/// The register checks of the relaxed sequences: the descriptor sequence works on `x0`
/// (AAELF64's form, which the relaxation keeps), and the initial-exec load's destination must be
/// its base, so that the `movk` completes the `movz` the page became.
fn check_registers(site: &Site<'_, '_>, kind: u32, insn: u32) -> Result<()> {
    let rd = insn & 0x1f;
    let rn = (insn >> 5) & 0x1f;
    let ok = match kind {
        R_TLSDESC_ADR_PAGE21 => rd == 0,
        R_TLSDESC_LD64_LO12 => rn == 0 && insn & 0xffc0_0000 == 0xf940_0000,
        R_TLSDESC_ADD_LO12 => rd == 0 && rn == 0 && insn >> 31 == 1,
        R_TLSIE_LD64_GOTTPREL_LO12_NC => rd == rn && insn & 0xffc0_0000 == 0xf940_0000,
        R_LD64_GOT_LO12_NC => insn & 0xffc0_0000 == 0xf940_0000,
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(site.refused(&format!(
            "the instruction ({insn:#010x}) does not use the registers of the sequence the relaxation rewrites"
        )))
    }
}

/// The page of `value` (4 KiB).
fn page(value: i128) -> i128 {
    value & !0xfff
}

/// Writes one relocation planned as `step`, at `place` (its address).
pub(crate) fn write(
    site: &Site<'_, '_>,
    step: &Step,
    bytes: &mut [u8],
    place: u64,
    referent: Referent,
    layout: &dyn Layout,
) -> Result<Option<Fixup>> {
    let rela = site.rela;
    let kind = rela.kind;
    let at = rela.offset;
    let a = i128::from(rela.addend);
    let p = i128::from(place);
    let changed = || site.refused("the referent changed kind between the plan and the layout");
    // S + A: the symbol's value (an address, an absolute value, or 0 for an undefined weak
    // symbol), or the thread-pointer offset of a thread-local variable.
    let x = match referent {
        // A call to an undefined weak function, which its caller guards with a test of the
        // function's address: a branch to the next instruction (AAELF64 5.7.10, as `ld.lld`).
        Referent::UndefinedWeak if matches!(kind, R_CALL26 | R_JUMP26) => p + 4,
        Referent::Address(value) | Referent::Absolute(value) => i128::from(value) + a,
        Referent::UndefinedWeak => a,
        Referent::Tls(offset) => {
            reloc::tp_offset(site, crate::elf::Arch::Aarch64, offset, layout.tls()?)? + a
        }
    };
    if kind == R_ABS64 || kind == R_PREL64 {
        let value = reloc::wide(site, if kind == R_PREL64 { x - p } else { x })?;
        reloc::put(site, bytes, at, &value.to_le_bytes())?;
        return Ok((step.action == Action::Relative).then_some(Fixup {
            place,
            addend: value,
        }));
    }
    if kind == R_ABS32 || kind == R_PREL32 {
        let (value, code) = if kind == R_PREL32 {
            (x - p, Code::U7015)
        } else {
            (reloc::as_signed(x), Code::U8001)
        };
        let field = reloc::either(site, value, 32, code)?;
        reloc::put(
            site,
            bytes,
            at,
            &u32::try_from(field).unwrap_or(0).to_le_bytes(),
        )?;
        return Ok(None);
    }
    let insn = reloc::read32(site, bytes, at)?;
    let imm12 = |insn: u32, field: u64| {
        (insn & !(0xfff << 10)) | (u32::try_from(field & 0xfff).unwrap_or(0) << 10)
    };
    let imm16 = |insn: u32, field: i128| {
        (insn & !(0xffff << 5)) | (u32::try_from(field.rem_euclid(1 << 16)).unwrap_or(0) << 5)
    };
    let lo12 = |value: i128, scale: u32| -> Result<u64> {
        let low = u64::try_from(value.rem_euclid(1 << 12)).unwrap_or(0);
        if low & ((1u64 << scale) - 1) != 0 {
            return Err(site.refused(&format!(
                "the offset {low:#x} is not aligned to the access of {} bytes",
                1u64 << scale
            )));
        }
        Ok(low >> scale)
    };
    let out = match (kind, step.action) {
        (
            R_MOVW_UABS_G0 | R_MOVW_UABS_G0_NC | R_MOVW_UABS_G1 | R_MOVW_UABS_G1_NC
            | R_MOVW_UABS_G2 | R_MOVW_UABS_G2_NC | R_MOVW_UABS_G3,
            Action::Direct,
        ) => {
            let (shift, limit) = match kind {
                R_MOVW_UABS_G0 => (0, Some(16)),
                R_MOVW_UABS_G0_NC => (0, None),
                R_MOVW_UABS_G1 => (16, Some(32)),
                R_MOVW_UABS_G1_NC => (16, None),
                R_MOVW_UABS_G2 => (32, Some(48)),
                R_MOVW_UABS_G2_NC => (32, None),
                _ => (48, None),
            };
            let value = reloc::as_unsigned(x);
            if let Some(bits) = limit {
                reloc::unsigned(site, value, bits, Code::U8001)?;
            }
            imm16(insn, value >> shift)
        }
        (R_TLSLE_MOVW_TPREL_G2 | R_TLSLE_MOVW_TPREL_G1 | R_TLSLE_MOVW_TPREL_G0, Action::Direct) => {
            let (shift, bits) = match kind {
                R_TLSLE_MOVW_TPREL_G2 => (32, 48),
                R_TLSLE_MOVW_TPREL_G1 => (16, 32),
                _ => (0, 16),
            };
            reloc::signed(site, x, bits, Code::U7015)?;
            if x < 0 {
                // MOVN of the complement.
                imm16(insn & !(1 << 30), (!x) >> shift)
            } else {
                imm16(insn | (1 << 30), x >> shift)
            }
        }
        (R_TLSLE_MOVW_TPREL_G1_NC, Action::Direct) => imm16(insn, x >> 16),
        (R_TLSLE_MOVW_TPREL_G0_NC, Action::Direct) => imm16(insn, x),
        (R_ADR_PREL_LO21, Action::Direct) => {
            let value = reloc::signed(site, x - p, 21, Code::U7015)?;
            adr(insn, value)
        }
        (R_ADR_PREL_PG_HI21, Action::Direct) => {
            let pages = reloc::signed(site, (page(x) - page(p)) >> 12, 21, Code::U7015)?;
            adr(insn, pages)
        }
        (R_ADR_GOT_PAGE, Action::Got) => {
            let g = i128::from(layout.got(rela.symbol)?);
            let pages = reloc::signed(site, (page(g) - page(p)) >> 12, 21, Code::U7015)?;
            adr(insn, pages)
        }
        (R_LD64_GOT_LO12_NC, Action::Got) => {
            let g = i128::from(layout.got(rela.symbol)?);
            imm12(insn, lo12(g, 3)?)
        }
        (R_ADD_ABS_LO12_NC, Action::Direct) => imm12(insn, lo12(x, 0)?),
        (R_TLSLE_ADD_TPREL_LO12_NC, Action::Direct) => imm12(insn, lo12(x, 0)?),
        (R_TLSLE_ADD_TPREL_HI12, Action::Direct) => {
            let value = reloc::unsigned(site, x, 24, Code::U7015)?;
            imm12(insn, value >> 12)
        }
        (
            R_LDST8_ABS_LO12_NC
            | R_LDST16_ABS_LO12_NC
            | R_LDST32_ABS_LO12_NC
            | R_LDST64_ABS_LO12_NC
            | R_LDST128_ABS_LO12_NC
            | R_TLSLE_LDST8_TPREL_LO12_NC
            | R_TLSLE_LDST16_TPREL_LO12_NC
            | R_TLSLE_LDST32_TPREL_LO12_NC
            | R_TLSLE_LDST64_TPREL_LO12_NC
            | R_TLSLE_LDST128_TPREL_LO12_NC,
            Action::Direct,
        ) => imm12(insn, lo12(x, access(insn))?),
        (R_JUMP26 | R_CALL26 | R_CONDBR19 | R_TSTBR14, Action::Direct) => {
            let delta = x - p;
            if delta % 4 != 0 {
                return Err(site.refused("a branch to a target that is not 4-byte aligned"));
            }
            let (bits, shift, mask) = match kind {
                R_TSTBR14 => (14, 5, 0x3fffu32),
                R_CONDBR19 => (19, 5, 0x7_ffff),
                _ => (26, 0, 0x3ff_ffff),
            };
            let words = reloc::signed(site, delta / 4, bits, Code::U7015)
                .map_err(|_| site.problem(Code::U7015, &format!(
                    "the branch target is {delta:#x} bytes away, beyond its reach (no range-extension thunks, ADR 0124 rule 24)"
                )))?;
            (insn & !(mask << shift)) | ((u32::try_from(words).unwrap_or(0) & mask) << shift)
        }
        (R_TLSIE_ADR_GOTTPREL_PAGE21 | R_TLSDESC_ADR_PAGE21, Action::RelaxTls) => {
            let value = reloc::unsigned(site, x, 32, Code::U7015)?;
            let rd = if kind == R_TLSDESC_ADR_PAGE21 {
                0
            } else {
                insn & 0x1f
            };
            0xd2a0_0000 | rd | (u32::try_from((value >> 16) & 0xffff).unwrap_or(0) << 5)
        }
        (R_TLSIE_LD64_GOTTPREL_LO12_NC | R_TLSDESC_LD64_LO12, Action::RelaxTls) => {
            let value = reloc::unsigned(site, x, 32, Code::U7015)?;
            let rt = if kind == R_TLSDESC_LD64_LO12 {
                0
            } else {
                insn & 0x1f
            };
            0xf280_0000 | rt | (u32::try_from(value & 0xffff).unwrap_or(0) << 5)
        }
        (R_TLSDESC_ADD_LO12 | R_TLSDESC_CALL, Action::RelaxTls) => {
            reloc::unsigned(site, x, 32, Code::U7015)?;
            NOP
        }
        _ => return Err(changed()),
    };
    reloc::put(site, bytes, at, &out.to_le_bytes())?;
    Ok(None)
}

/// `ADR` or `ADRP` with its 21-bit immediate set to `value` (already reduced to 21 bits).
fn adr(insn: u32, value: u64) -> u32 {
    let value = u32::try_from(value & 0x1f_ffff).unwrap_or(0);
    (insn & 0x9f00_001f) | ((value & 3) << 29) | (((value >> 2) & 0x7_ffff) << 5)
}
