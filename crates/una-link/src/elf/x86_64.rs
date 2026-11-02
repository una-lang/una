//! The x86-64 relocations of rule 24 and their relaxations, as the x86-64 psABI (sections 4.4
//! and B.2, and the TLS document's chapter 11) and `ld.lld` give them for a static
//! position-independent executable:
//!
//! - `R_X86_64_64` (a relative fixup against an address, in a writable section only), `PC32`,
//!   `PC64`, `PLT32` (resolved directly: there is no PLT), `32` and `32S` (absolute symbols
//!   only);
//! - `GOTPCREL` through a GOT slot; `GOTPCRELX` and `REX_GOTPCRELX` with an addend of -4
//!   against an address relaxed as `ld.lld` does in a PIE: `mov foo@GOTPCREL(%rip), %reg`
//!   becomes `lea foo(%rip), %reg`, `call *foo@GOTPCREL(%rip)` becomes `addr32 call foo` and
//!   `jmp *foo@GOTPCREL(%rip)` becomes `jmp foo; nop`; any other form keeps its GOT slot;
//! - the thread-local types relaxed to the local-exec model: the general-dynamic sequence
//!   (`TLSGD` with the call to `__tls_get_addr` through `PLT32` or through the GOT) becomes
//!   `mov %fs:0, %rax; lea x@tpoff(%rax), %rax`; the local-dynamic one (`TLSLD`, both forms)
//!   becomes `mov %fs:0, %rax` with padding, and its `DTPOFF32` offsets become thread-pointer
//!   offsets; the initial-exec `GOTTPOFF` load (`movq` or `addq`) becomes an immediate; and
//!   `TPOFF32` is the offset itself.
//!
//! Each relaxation checks the bytes of the sequence it rewrites, and refuses (U8001) one that is
//! not the psABI's form, rather than rewriting instructions it does not know.

use crate::elf::reloc::{self, Action, Class, Decision, Fixup, Layout, Referent, Site, Step, Type};
use crate::problem::{Code, Result};

pub(crate) const R_64: u32 = 1;
pub(crate) const R_PC32: u32 = 2;
pub(crate) const R_PLT32: u32 = 4;
pub(crate) const R_GOTPCREL: u32 = 9;
pub(crate) const R_32: u32 = 10;
pub(crate) const R_32S: u32 = 11;
pub(crate) const R_TLSGD: u32 = 19;
pub(crate) const R_TLSLD: u32 = 20;
pub(crate) const R_DTPOFF32: u32 = 21;
pub(crate) const R_GOTTPOFF: u32 = 22;
pub(crate) const R_TPOFF32: u32 = 23;
pub(crate) const R_PC64: u32 = 24;
pub(crate) const R_IRELATIVE: u32 = 37;
pub(crate) const R_GOTPCRELX: u32 = 41;
pub(crate) const R_REX_GOTPCRELX: u32 = 42;

/// Rule 24's list for x86-64.
pub(crate) const TYPES: &[Type] = &[
    Type {
        value: R_64,
        name: "R_X86_64_64",
        class: Class::Wide,
    },
    Type {
        value: R_PC32,
        name: "R_X86_64_PC32",
        class: Class::Plain,
    },
    Type {
        value: R_PLT32,
        name: "R_X86_64_PLT32",
        class: Class::Plain,
    },
    Type {
        value: R_GOTPCREL,
        name: "R_X86_64_GOTPCREL",
        class: Class::Plain,
    },
    Type {
        value: R_32,
        name: "R_X86_64_32",
        class: Class::Narrow,
    },
    Type {
        value: R_32S,
        name: "R_X86_64_32S",
        class: Class::Narrow,
    },
    Type {
        value: R_TLSGD,
        name: "R_X86_64_TLSGD",
        class: Class::Plain,
    },
    Type {
        value: R_TLSLD,
        name: "R_X86_64_TLSLD",
        class: Class::Plain,
    },
    Type {
        value: R_DTPOFF32,
        name: "R_X86_64_DTPOFF32",
        class: Class::Plain,
    },
    Type {
        value: R_GOTTPOFF,
        name: "R_X86_64_GOTTPOFF",
        class: Class::Plain,
    },
    Type {
        value: R_TPOFF32,
        name: "R_X86_64_TPOFF32",
        class: Class::Plain,
    },
    Type {
        value: R_PC64,
        name: "R_X86_64_PC64",
        class: Class::Plain,
    },
    Type {
        value: R_GOTPCRELX,
        name: "R_X86_64_GOTPCRELX",
        class: Class::Plain,
    },
    Type {
        value: R_REX_GOTPCRELX,
        name: "R_X86_64_REX_GOTPCRELX",
        class: Class::Plain,
    },
];

/// The general-dynamic sequence's prefix before the `TLSGD` place: `data16 lea x@tlsgd(%rip),
/// %rdi`.
const GD_LEA: [u8; 4] = [0x66, 0x48, 0x8d, 0x3d];
/// The call after it, through `PLT32`: `data16 data16 rex64 call __tls_get_addr@PLT`.
const GD_CALL_PLT: [u8; 4] = [0x66, 0x66, 0x48, 0xe8];
/// The call after it, through the GOT: `data16 rex64 call *__tls_get_addr@GOTPCREL(%rip)`.
const GD_CALL_GOT: [u8; 4] = [0x66, 0x48, 0xff, 0x15];
/// What the general-dynamic sequence becomes: `mov %fs:0, %rax; lea x@tpoff(%rax), %rax`.
const GD_TO_LE: [u8; 12] = [0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0x48, 0x8d, 0x80];
/// The local-dynamic sequence's prefix before the `TLSLD` place: `lea x@tlsld(%rip), %rdi`.
const LD_LEA: [u8; 3] = [0x48, 0x8d, 0x3d];
/// What the local-dynamic sequence with a `PLT32` call becomes (12 bytes): `data16 data16 data16
/// mov %fs:0, %rax`.
const LD_TO_LE_PLT: [u8; 12] = [0x66, 0x66, 0x66, 0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0];
/// What it becomes with a call through the GOT (13 bytes, the psABI's table 11.9).
const LD_TO_LE_GOT: [u8; 13] = [
    0x66, 0x66, 0x66, 0x66, 0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0,
];

/// The calls to `__tls_get_addr` a relaxed sequence consumes.
const TLS_GET_ADDR: &[u8] = b"__tls_get_addr";

/// Decides the action at one relocation, from its type and its referent's kind.
pub(crate) fn decide(site: &Site<'_, '_>, referent: Referent) -> Result<Decision> {
    let rela = site.rela;
    let writable = site.section().is_some_and(|s| s.writable());
    let direct = |(start, end): (u64, u64)| Decision {
        action: Action::Direct,
        start,
        end,
        consumes: 0,
    };
    let not_tls = || -> Result<()> {
        if matches!(referent, Referent::Tls(_)) {
            Err(site.refused("a thread-local variable through a type that is not thread-local"))
        } else {
            Ok(())
        }
    };
    let tls = || -> Result<()> {
        if matches!(referent, Referent::Tls(_)) {
            Ok(())
        } else {
            Err(site.refused(&format!(
                "a thread-local type against {}",
                referent.describe()
            )))
        }
    };
    let address = || -> Result<()> {
        if matches!(referent, Referent::Address(_)) {
            Ok(())
        } else {
            Err(site.refused(&format!(
                "a PC-relative type against {}, which a position-independent executable cannot reach",
                referent.describe()
            )))
        }
    };
    match rela.kind {
        R_64 => {
            let range = site.range(0, 8)?;
            not_tls()?;
            match referent {
                Referent::Address(_) if writable => Ok(Decision {
                    action: Action::Relative,
                    ..direct(range)
                }),
                Referent::Address(_) | Referent::UndefinedWeak if !writable => Err(site.refused(
                    "a 64-bit absolute relocation in a section that is not writable, a text relocation",
                )),
                _ => Ok(direct(range)),
            }
        }
        R_PLT32 if referent == Referent::UndefinedWeak => {
            // A call (or a tail jump) to an undefined weak function, which its caller guards
            // with a test of the function's address (crtbeginS.o's `__cxa_finalize`): resolved
            // as `ld.lld` does, with the symbol's value 0, so the call, if it ever ran, would
            // reach the image's first byte, the headers, which are not executable. Only a
            // `call rel32` or `jmp rel32`: anything else is a PC-relative reference.
            let range = site.range(0, 4)?;
            if rela.offset == 0 || !matches!(site.byte(-1)?, 0xe8 | 0xe9) {
                return Err(site.refused(
                    "a PC-relative type against an undefined weak symbol that is not a call",
                ));
            }
            Ok(direct(range))
        }
        R_PC32 | R_PLT32 => {
            let range = site.range(0, 4)?;
            address()?;
            Ok(direct(range))
        }
        R_PC64 => {
            let range = site.range(0, 8)?;
            address()?;
            Ok(direct(range))
        }
        R_32 | R_32S => {
            let range = site.range(0, 4)?;
            not_tls()?;
            // Rule 24: only against an absolute symbol (`SHN_ABS`), as the scan of 4a says.
            if !matches!(referent, Referent::Absolute(_)) {
                return Err(site.refused(
                    "a narrow absolute relocation against a symbol that is not absolute, which no load-time fixup can express",
                ));
            }
            Ok(direct(range))
        }
        R_GOTPCREL => {
            let range = site.range(0, 4)?;
            not_tls()?;
            Ok(Decision {
                action: Action::Got,
                ..direct(range)
            })
        }
        R_GOTPCRELX | R_REX_GOTPCRELX => {
            let range = site.range(0, 4)?;
            not_tls()?;
            if matches!(referent, Referent::Address(_))
                && rela.addend == -4
                && relaxable(site)?.is_some()
            {
                let (start, _) = site.range(-2, 6)?;
                Ok(Decision {
                    action: Action::RelaxGot,
                    start,
                    end: range.1,
                    consumes: 0,
                })
            } else {
                Ok(Decision {
                    action: Action::Got,
                    ..direct(range)
                })
            }
        }
        R_TLSGD => {
            tls()?;
            let (start, end) = site.range(-4, 16)?;
            expect(site, -4, &GD_LEA, "the general-dynamic sequence's `lea`")?;
            let call = following_call(site, 8)?;
            let form = if call == R_PLT32 || call == R_PC32 {
                GD_CALL_PLT
            } else {
                GD_CALL_GOT
            };
            expect(site, 4, &form, "the general-dynamic sequence's call")?;
            Ok(Decision {
                action: Action::RelaxTls,
                start,
                end,
                consumes: 1,
            })
        }
        R_TLSLD => {
            tls()?;
            site.range(-3, 7)?;
            expect(site, -3, &LD_LEA, "the local-dynamic sequence's `lea`")?;
            let width = if site.byte(4)? == 0xe8 {
                if !matches!(following_call(site, 5)?, R_PLT32 | R_PC32) {
                    return Err(site.refused("the local-dynamic sequence's call is not its form"));
                }
                12
            } else {
                expect(site, 4, &[0xff, 0x15], "the local-dynamic sequence's call")?;
                if matches!(following_call(site, 6)?, R_PLT32 | R_PC32) {
                    return Err(site.refused("the local-dynamic sequence's call is not its form"));
                }
                13
            };
            let (start, end) = site.range(-3, width)?;
            Ok(Decision {
                action: Action::RelaxTls,
                start,
                end,
                consumes: 1,
            })
        }
        R_DTPOFF32 | R_TPOFF32 => {
            let range = site.range(0, 4)?;
            tls()?;
            Ok(direct(range))
        }
        R_GOTTPOFF => {
            tls()?;
            let (start, end) = site.range(-3, 7)?;
            initial_exec(site)?;
            Ok(Decision {
                action: Action::RelaxTls,
                start,
                end,
                consumes: 0,
            })
        }
        _ => Err(site.refused("the type is outside the supported set (ADR 0124 rule 24)")),
    }
}

/// The instruction of a `GOTPCRELX` load that the relaxation rewrites, if it is one of the forms
/// it rewrites: `Some(0x8b)` for `mov`, `Some(0x15)` for `call *`, `Some(0x25)` for `jmp *`.
fn relaxable(site: &Site<'_, '_>) -> Result<Option<u8>> {
    if site.rela.offset < 2 {
        return Ok(None);
    }
    let op = site.byte(-2)?;
    let modrm = site.byte(-1)?;
    if site.rela.kind == R_REX_GOTPCRELX {
        // A REX prefix before the opcode: `mov` only (the 64-bit loads).
        if site.rela.offset < 3 || site.byte(-3)? & 0xf0 != 0x40 {
            return Ok(None);
        }
        return Ok((op == 0x8b && modrm & 0xc7 == 0x05).then_some(0x8b));
    }
    Ok(match (op, modrm) {
        (0x8b, m) if m & 0xc7 == 0x05 => Some(0x8b),
        (0xff, 0x15) => Some(0x15),
        (0xff, 0x25) => Some(0x25),
        _ => None,
    })
}

/// Checks that the bytes at `before` from the place are `expected`.
fn expect(site: &Site<'_, '_>, before: i64, expected: &[u8], what: &str) -> Result<()> {
    for (at, want) in (before..).zip(expected) {
        if site.byte(at)? != *want {
            return Err(site.refused(&format!(
                "{what} is not the psABI's form, so the sequence cannot be relaxed"
            )));
        }
    }
    Ok(())
}

/// The type of the relocation after a `TLSGD` or `TLSLD`, which must be the call to
/// `__tls_get_addr` at `at` bytes from the place, through `PLT32`/`PC32` or through the GOT.
fn following_call(site: &Site<'_, '_>, at: u64) -> Result<u32> {
    let wrong = || {
        site.refused("a thread-local sequence that is not followed by its call to `__tls_get_addr`")
    };
    let next = site.next().ok_or_else(wrong)?;
    let place = site.rela.offset.checked_add(at).ok_or_else(wrong)?;
    let name = site
        .object
        .symbol(next.symbol)
        .map(|s| s.name)
        .unwrap_or_default();
    if next.offset != place
        || name != TLS_GET_ADDR
        || !matches!(
            next.kind,
            R_PLT32 | R_PC32 | R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX
        )
    {
        return Err(wrong());
    }
    Ok(next.kind)
}

/// The initial-exec load before a `GOTTPOFF` place: `(rex, opcode, modrm)` of a `movq` or
/// `addq` from the GOT, RIP-relative.
fn initial_exec(site: &Site<'_, '_>) -> Result<(u8, u8, u8)> {
    let rex = site.byte(-3)?;
    let op = site.byte(-2)?;
    let modrm = site.byte(-1)?;
    if (rex == 0x48 || rex == 0x4c) && (op == 0x8b || op == 0x03) && modrm & 0xc7 == 0x05 {
        Ok((rex, op, modrm))
    } else {
        Err(site.refused(
            "an initial-exec access that is not a `movq` or an `addq` from the GOT, so it cannot be relaxed",
        ))
    }
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
    let at = rela.offset;
    let a = i128::from(rela.addend);
    let p = i128::from(place);
    let pcrel = |value: i128| reloc::signed(site, value, 32, Code::U7015);
    let s = || -> Result<i128> {
        match referent {
            Referent::Address(value) => Ok(i128::from(value)),
            Referent::UndefinedWeak if rela.kind == R_PLT32 => Ok(0),
            _ => Err(site.refused("the referent changed kind between the plan and the layout")),
        }
    };
    let tp = || -> Result<i128> {
        match referent {
            Referent::Tls(offset) => {
                reloc::tp_offset(site, crate::elf::Arch::X86_64, offset, layout.tls()?)
            }
            _ => Err(site.refused("the referent changed kind between the plan and the layout")),
        }
    };
    let absolute = || -> Result<i128> {
        match referent {
            Referent::Absolute(value) => Ok(i128::from(value)),
            Referent::UndefinedWeak => Ok(0),
            _ => Err(site.refused("the referent changed kind between the plan and the layout")),
        }
    };
    let put32 = |bytes: &mut [u8], at: u64, field: u64| {
        reloc::put(
            site,
            bytes,
            at,
            &u32::try_from(field).unwrap_or(0).to_le_bytes(),
        )
    };
    match (rela.kind, step.action) {
        (R_64, Action::Relative) => {
            let value = reloc::wide(site, s()? + a)?;
            reloc::put(site, bytes, at, &value.to_le_bytes())?;
            return Ok(Some(Fixup {
                place,
                addend: value,
            }));
        }
        (R_64, Action::Direct) => {
            let value = reloc::wide(site, absolute()? + a)?;
            reloc::put(site, bytes, at, &value.to_le_bytes())?;
        }
        (R_PC32 | R_PLT32, Action::Direct) => put32(bytes, at, pcrel(s()? + a - p)?)?,
        (R_PC64, Action::Direct) => {
            let value = reloc::wide(site, s()? + a - p)?;
            reloc::put(site, bytes, at, &value.to_le_bytes())?;
        }
        (R_32, Action::Direct) => {
            let value = reloc::as_unsigned(absolute()? + a);
            put32(bytes, at, reloc::unsigned(site, value, 32, Code::U8001)?)?;
        }
        (R_32S, Action::Direct) => {
            let value = reloc::as_signed(absolute()? + a);
            put32(bytes, at, reloc::signed(site, value, 32, Code::U8001)?)?;
        }
        (R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX, Action::Got) => {
            let g = i128::from(layout.got(rela.symbol)?);
            put32(bytes, at, pcrel(g + a - p)?)?;
        }
        (R_GOTPCRELX | R_REX_GOTPCRELX, Action::RelaxGot) => {
            let value = s()? + a - p;
            let start = at.saturating_sub(2);
            match relaxable(site)? {
                Some(0x8b) => {
                    reloc::put(site, bytes, start, &[0x8d])?;
                    put32(bytes, at, pcrel(value)?)?;
                }
                Some(0x15) => {
                    reloc::put(site, bytes, start, &[0x67, 0xe8])?;
                    put32(bytes, at, pcrel(value)?)?;
                }
                Some(0x25) => {
                    reloc::put(site, bytes, start, &[0xe9])?;
                    put32(bytes, at.saturating_sub(1), pcrel(value + 1)?)?;
                    reloc::put(site, bytes, at.saturating_add(3), &[0x90])?;
                }
                _ => {
                    return Err(
                        site.refused("the GOT load changed between the plan and the layout")
                    );
                }
            }
        }
        (R_TLSGD, Action::RelaxTls) => {
            let field = reloc::signed(site, tp()? + a + 4, 32, Code::U7015)?;
            reloc::put(site, bytes, step.start, &GD_TO_LE)?;
            put32(bytes, at.saturating_add(8), field)?;
        }
        (R_TLSLD, Action::RelaxTls) => {
            tp()?;
            if step.end.saturating_sub(step.start) == 12 {
                reloc::put(site, bytes, step.start, &LD_TO_LE_PLT)?;
            } else {
                reloc::put(site, bytes, step.start, &LD_TO_LE_GOT)?;
            }
        }
        (R_DTPOFF32 | R_TPOFF32, Action::Direct) => {
            put32(bytes, at, reloc::signed(site, tp()? + a, 32, Code::U7015)?)?;
        }
        (R_GOTTPOFF, Action::RelaxTls) => {
            let (rex, op, modrm) = initial_exec(site)?;
            let reg = (modrm >> 3) & 7;
            let high = rex == 0x4c;
            let rewritten = if op == 0x8b {
                [if high { 0x49 } else { 0x48 }, 0xc7, 0xc0 | reg]
            } else if reg == 4 {
                [if high { 0x49 } else { 0x48 }, 0x81, 0xc0 | reg]
            } else {
                [
                    if high { 0x4d } else { 0x48 },
                    0x8d,
                    0x80 | (reg << 3) | reg,
                ]
            };
            let field = reloc::signed(site, tp()? + a + 4, 32, Code::U7015)?;
            reloc::put(site, bytes, step.start, &rewritten)?;
            put32(bytes, at, field)?;
        }
        _ => return Err(site.refused("the plan has no action of this kind for the type")),
    }
    Ok(None)
}
