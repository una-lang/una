//! Unit tests of the ELF reader and relocation engine: a vector per type and form, the bounds and
//! ranges of each field, the cases rule 24 refuses, objects the reader refuses, and every
//! mutation of small objects, which must fail with a diagnostic or pass, never panic.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeMap;

use super::input;
use super::reloc::{self, Action, Fixup, Layout, Referent, Tls};
use super::testobj::*;
use super::{aarch64 as a64, x86_64 as x64};
use crate::problem::{Code, Problem, Result};

/// A layout of fixed referents (index 0 is the null symbol), GOT slots from 0x3000, and a TLS
/// template of 0x20 bytes aligned to 16.
struct Fixed {
    referents: Vec<Referent>,
    got: BTreeMap<u32, u64>,
    tls: Tls,
}

impl Layout for Fixed {
    fn referent(&self, symbol: u32) -> Result<Referent> {
        self.referents
            .get(symbol as usize)
            .copied()
            .ok_or_else(|| Problem::internal("no such symbol"))
    }
    fn got(&self, symbol: u32) -> Result<u64> {
        self.got
            .get(&symbol)
            .copied()
            .ok_or_else(|| Problem::internal("no GOT slot"))
    }
    fn tls(&self) -> Result<Tls> {
        Ok(self.tls)
    }
}

const TEXT: u64 = 0x1000;
const GOT: u64 = 0x3000;
const TLS_TEMPLATE: Tls = Tls {
    size: 0x20,
    align: 16,
};

/// Applies the relocations of `section` (the object's section 1), placed at `address`, with
/// `symbols` resolving to `referents`. Returns the bytes, the fixups and the plan's GOT symbols.
fn apply(
    machine: u16,
    section: Sec,
    symbols: &[Sym],
    referents: &[Referent],
    address: u64,
) -> Result<(Vec<u8>, Vec<Fixup>, Vec<u32>)> {
    let bytes = object(machine, &[section], symbols);
    let object = input::read("test.o", &bytes)?;
    let mut layout = Fixed {
        referents: std::iter::once(Referent::Absolute(0))
            .chain(referents.iter().copied())
            .collect(),
        got: BTreeMap::new(),
        tls: TLS_TEMPLATE,
    };
    let plan = reloc::plan(&object, 1, &|symbol| layout.referent(symbol))?;
    for (slot, symbol) in plan.got.iter().enumerate() {
        layout.got.insert(*symbol, GOT + 8 * slot as u64);
    }
    let mut data = object.sections[1].data.to_vec();
    let fixups = reloc::apply(&object, 1, &mut data, address, &layout)?;
    Ok((data, fixups, plan.got.into_iter().collect()))
}

/// One relocation of `kind` at `offset` of a section of `data` against symbol 1, defined as a
/// global function of section 1 (its value is the referent's, not the object's).
fn one(
    machine: u16,
    flags: u64,
    data: Vec<u8>,
    (offset, kind, addend): (u64, u32, i64),
    referent: Referent,
) -> Result<Vec<u8>> {
    let mut section = sec(".text", flags, data);
    section.relocs = vec![(offset, 1, kind, addend)];
    let symbols = [sym("target", GLOBAL_NOTYPE, ABS, 0)];
    apply(machine, section, &symbols, &[referent], TEXT).map(|(bytes, _, _)| bytes)
}

fn refused(result: Result<impl std::fmt::Debug>, code: Code, words: &str) {
    match result {
        Err(problem) => {
            assert_eq!(problem.code, code, "{problem}");
            assert!(problem.message.contains(words), "{problem}");
        }
        Ok(value) => panic!("accepted: {value:?}, expected {words}"),
    }
}

fn le(insn: u32) -> Vec<u8> {
    insn.to_le_bytes().to_vec()
}

#[test]
fn x86_64_pc_relative_and_absolute_vectors() {
    let addr = Referent::Address(0x2000);
    // lea 0x2000-4-0x1003(%rip), %rax
    let lea = vec![0x48, 0x8d, 0x05, 0, 0, 0, 0];
    assert_eq!(
        one(X86_64, EXEC, lea.clone(), (3, x64::R_PC32, -4), addr).unwrap(),
        [0x48, 0x8d, 0x05, 0xf9, 0x0f, 0, 0]
    );
    assert_eq!(
        one(
            X86_64,
            EXEC,
            vec![0xe8, 0, 0, 0, 0],
            (1, x64::R_PLT32, -4),
            addr
        )
        .unwrap(),
        [0xe8, 0xfb, 0x0f, 0, 0]
    );
    // PC64 and 64 against an absolute symbol (no fixup), 32 and 32S modulo 2^64.
    assert_eq!(
        one(X86_64, WRITE, vec![0; 8], (0, x64::R_PC64, 8), addr).unwrap(),
        (0x2000u64 + 8 - 0x1000).to_le_bytes()
    );
    assert_eq!(
        one(
            X86_64,
            0,
            vec![0; 8],
            (0, x64::R_64, 1),
            Referent::Absolute(0x1234)
        )
        .unwrap(),
        0x1235u64.to_le_bytes()
    );
    assert_eq!(
        one(
            X86_64,
            EXEC,
            vec![0; 4],
            (0, x64::R_32S, 0),
            Referent::Absolute(u64::MAX - 7)
        )
        .unwrap(),
        (-8i32).to_le_bytes()
    );
    refused(
        one(
            X86_64,
            EXEC,
            vec![0; 4],
            (0, x64::R_32, 0),
            Referent::Absolute(1 << 32),
        ),
        Code::U8001,
        "does not fit an unsigned field of 32 bits",
    );
    refused(
        one(
            X86_64,
            EXEC,
            vec![0; 4],
            (0, x64::R_32S, 0),
            Referent::Absolute(1 << 31),
        ),
        Code::U8001,
        "signed field of 32 bits",
    );
    // A displacement beyond ±2 GiB is the layout's (U7015).
    refused(
        one(
            X86_64,
            EXEC,
            lea.clone(),
            (3, x64::R_PC32, -4),
            Referent::Address(0x9000_0000),
        ),
        Code::U7015,
        "signed field of 32 bits",
    );
    assert!(
        one(
            X86_64,
            EXEC,
            lea,
            (3, x64::R_PC32, -4),
            Referent::Address(0x8000_1002)
        )
        .is_ok()
    );
}

#[test]
fn a_64_bit_pointer_to_an_address_is_a_relative_fixup_in_a_writable_section_only() {
    let mut section = sec(".data.rel.ro", WRITE, vec![0; 16]);
    section.relocs = vec![(8, 1, x64::R_64, 0x10)];
    let symbols = [sym("f", GLOBAL_FUNC, ABS, 0)];
    let (bytes, fixups, _) = apply(
        X86_64,
        section,
        &symbols,
        &[Referent::Address(0x2000)],
        0x5000,
    )
    .unwrap();
    assert_eq!(&bytes[8..], 0x2010u64.to_le_bytes());
    assert_eq!(
        fixups,
        [Fixup {
            place: 0x5008,
            addend: 0x2010
        }]
    );
    for (machine, kind) in [(X86_64, x64::R_64), (AARCH64, a64::R_ABS64)] {
        refused(
            one(
                machine,
                0,
                vec![0; 8],
                (0, kind, 0),
                Referent::Address(0x2000),
            ),
            Code::U8001,
            "text relocation",
        );
        // An undefined weak symbol is the absolute 0: no fixup, anywhere.
        assert_eq!(
            one(
                machine,
                0,
                vec![0; 8],
                (0, kind, 0),
                Referent::UndefinedWeak
            )
            .unwrap(),
            [0; 8]
        );
    }
}

#[test]
fn narrow_absolute_and_pc_relative_types_need_their_kind_of_referent() {
    let addr = Referent::Address(0x2000);
    for (machine, kind) in [
        (X86_64, x64::R_32),
        (X86_64, x64::R_32S),
        (AARCH64, a64::R_ABS32),
    ] {
        refused(
            one(machine, WRITE, vec![0; 4], (0, kind, 0), addr),
            Code::U8001,
            "narrow absolute",
        );
    }
    refused(
        one(
            AARCH64,
            EXEC,
            le(0xd280_0000),
            (0, a64::R_MOVW_UABS_G0, 0),
            addr,
        ),
        Code::U8001,
        "narrow absolute",
    );
    for referent in [
        Referent::Absolute(5),
        Referent::UndefinedWeak,
        Referent::Tls(0),
    ] {
        refused(
            one(X86_64, EXEC, vec![0; 4], (0, x64::R_PC32, -4), referent),
            Code::U8001,
            if referent == Referent::Tls(0) {
                "thread-local"
            } else {
                "PC-relative"
            },
        );
        refused(
            one(
                AARCH64,
                EXEC,
                le(0x9000_0000),
                (0, a64::R_ADR_PREL_PG_HI21, 0),
                referent,
            ),
            Code::U8001,
            if referent == Referent::Tls(0) {
                "thread-local"
            } else {
                "PC-relative"
            },
        );
    }
    // Thread-local types never against an address.
    refused(
        one(X86_64, EXEC, vec![0; 4], (0, x64::R_TPOFF32, 0), addr),
        Code::U8001,
        "a thread-local type against an address",
    );
    refused(
        one(
            AARCH64,
            EXEC,
            le(0xd280_0000),
            (0, a64::R_TLSLE_MOVW_TPREL_G0, 0),
            addr,
        ),
        Code::U8001,
        "a thread-local type",
    );
}

#[test]
fn a_guarded_call_to_an_undefined_weak_function_resolves_as_ld_lld_does() {
    // x86-64: the value 0, so the call reaches the image's first byte (not executable).
    assert_eq!(
        one(
            X86_64,
            EXEC,
            vec![0xe8, 0, 0, 0, 0],
            (1, x64::R_PLT32, -4),
            Referent::UndefinedWeak
        )
        .unwrap(),
        [0xe8]
            .iter()
            .copied()
            .chain((-4i32 - 0x1001).to_le_bytes())
            .collect::<Vec<_>>()
    );
    // arm64: a branch to the next instruction.
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0x9400_0000),
            (0, a64::R_CALL26, 0),
            Referent::UndefinedWeak
        )
        .unwrap(),
        le(0x9400_0001)
    );
}

#[test]
fn x86_64_got_loads_relax_to_address_computations_or_keep_their_slot() {
    let addr = Referent::Address(0x2000);
    // mov data@GOTPCREL(%rip), %rax -> lea
    assert_eq!(
        one(
            X86_64,
            EXEC,
            vec![0x48, 0x8b, 0x05, 0, 0, 0, 0],
            (3, x64::R_REX_GOTPCRELX, -4),
            addr
        )
        .unwrap(),
        [0x48, 0x8d, 0x05, 0xf9, 0x0f, 0, 0]
    );
    // call *f@GOTPCREL(%rip) -> addr32 call f
    assert_eq!(
        one(
            X86_64,
            EXEC,
            vec![0xff, 0x15, 0, 0, 0, 0],
            (2, x64::R_GOTPCRELX, -4),
            addr
        )
        .unwrap(),
        [0x67, 0xe8, 0xfa, 0x0f, 0, 0]
    );
    // jmp *f@GOTPCREL(%rip) -> jmp f; nop
    assert_eq!(
        one(
            X86_64,
            EXEC,
            vec![0xff, 0x25, 0, 0, 0, 0],
            (2, x64::R_GOTPCRELX, -4),
            addr
        )
        .unwrap(),
        [0xe9, 0xfb, 0x0f, 0, 0, 0x90]
    );
    // cmp %rax, data@GOTPCREL(%rip): not relaxed, through the GOT slot at 0x3000.
    let mut section = sec(".text", EXEC, vec![0x48, 0x3b, 0x05, 0, 0, 0, 0]);
    section.relocs = vec![(3, 1, x64::R_REX_GOTPCRELX, -4)];
    let (bytes, _, got) = apply(
        X86_64,
        section,
        &[sym("d", GLOBAL_OBJECT, ABS, 0)],
        &[addr],
        TEXT,
    )
    .unwrap();
    assert_eq!(bytes, [0x48, 0x3b, 0x05, 0xf9, 0x1f, 0, 0]);
    assert_eq!(got, [1]);
    // Against an absolute symbol, or with another addend, the slot is kept too.
    for (referent, addend) in [
        (Referent::Absolute(5), -4),
        (addr, -5),
        (Referent::UndefinedWeak, -4),
    ] {
        let mut section = sec(".text", EXEC, vec![0x48, 0x8b, 0x05, 0, 0, 0, 0]);
        section.relocs = vec![(3, 1, x64::R_REX_GOTPCRELX, addend)];
        let (_, _, got) = apply(
            X86_64,
            section,
            &[sym("d", GLOBAL_OBJECT, ABS, 0)],
            &[referent],
            TEXT,
        )
        .unwrap();
        assert_eq!(got, [1], "{referent:?} {addend}");
    }
    // A GOT slot of a thread-local variable never exists.
    assert!(reloc::got_entry(Referent::Tls(0)).is_err());
    assert_eq!(
        reloc::got_entry(Referent::Address(8)).unwrap(),
        reloc::GotEntry {
            value: 8,
            relative: true
        }
    );
}

/// The general-dynamic sequence with its call, and the relocation of the call.
fn general_dynamic(call: [u8; 4], call_kind: u32) -> (Sec, [Sym; 2]) {
    let mut data = vec![0x66, 0x48, 0x8d, 0x3d, 0, 0, 0, 0];
    data.extend_from_slice(&call);
    data.extend_from_slice(&[0; 4]);
    let mut section = sec(".text", EXEC, data);
    section.relocs = vec![(4, 1, x64::R_TLSGD, -4), (12, 2, call_kind, -4)];
    (
        section,
        [
            sym("v", GLOBAL_TLS, ABS, 0),
            sym("__tls_get_addr", GLOBAL_FUNC, ABS, 0),
        ],
    )
}

#[test]
fn x86_64_thread_local_sequences_relax_to_local_exec() {
    // The variable at 8 in a template of 0x20: %fs:-0x18.
    let referents = [Referent::Tls(8), Referent::Address(0x4000)];
    let le_bytes: [u8; 16] = [
        0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0x48, 0x8d, 0x80, 0xe8, 0xff, 0xff, 0xff,
    ];
    for (call, kind) in [
        ([0x66, 0x66, 0x48, 0xe8], x64::R_PLT32),
        ([0x66, 0x48, 0xff, 0x15], x64::R_GOTPCRELX),
        ([0x66, 0x48, 0xff, 0x15], x64::R_GOTPCREL),
    ] {
        let (section, symbols) = general_dynamic(call, kind);
        let (bytes, _, got) = apply(X86_64, section, &symbols, &referents, TEXT).unwrap();
        assert_eq!(bytes, le_bytes);
        assert!(
            got.is_empty(),
            "the call through the GOT needs no slot once relaxed"
        );
    }
    // A sequence that is not the psABI's, or a call to something else, is refused.
    let (mut section, symbols) = general_dynamic([0x66, 0x66, 0x48, 0xe8], x64::R_PLT32);
    section.data[0] = 0x90;
    refused(
        apply(X86_64, section, &symbols, &referents, TEXT),
        Code::U8001,
        "psABI's form",
    );
    let (mut section, mut symbols) = general_dynamic([0x66, 0x66, 0x48, 0xe8], x64::R_PLT32);
    symbols[1].name = "memcpy";
    section.relocs[1].0 = 12;
    refused(
        apply(X86_64, section, &symbols, &referents, TEXT),
        Code::U8001,
        "__tls_get_addr",
    );
    let (mut section, symbols) = general_dynamic([0x66, 0x66, 0x48, 0xe8], x64::R_PLT32);
    section.relocs.pop();
    refused(
        apply(X86_64, section, &symbols, &referents, TEXT),
        Code::U8001,
        "__tls_get_addr",
    );
    // Local-dynamic, both forms.
    for (call, kind, expected) in [
        (
            &[0xe8][..],
            x64::R_PLT32,
            &[0x66, 0x66, 0x66, 0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0][..],
        ),
        (
            &[0xff, 0x15][..],
            x64::R_GOTPCRELX,
            &[
                0x66, 0x66, 0x66, 0x66, 0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0,
            ][..],
        ),
    ] {
        let mut data = vec![0x48, 0x8d, 0x3d, 0, 0, 0, 0];
        data.extend_from_slice(call);
        data.extend_from_slice(&[0; 4]);
        let mut section = sec(".text", EXEC, data);
        let at = 3 + 4 + call.len() as u64;
        section.relocs = vec![(3, 1, x64::R_TLSLD, -4), (at, 2, kind, -4)];
        let symbols = [
            sym("v", GLOBAL_TLS, ABS, 0),
            sym("__tls_get_addr", GLOBAL_FUNC, ABS, 0),
        ];
        let (bytes, _, _) = apply(X86_64, section, &symbols, &referents, TEXT).unwrap();
        assert_eq!(bytes, expected);
    }
    // Initial-exec: movq to %r9, addq to %rcx and to %r12.
    for (insn, expected) in [
        ([0x4c, 0x8b, 0x0d], [0x49, 0xc7, 0xc1]),
        ([0x48, 0x03, 0x0d], [0x48, 0x8d, 0x89]),
        ([0x4c, 0x03, 0x25], [0x49, 0x81, 0xc4]),
        ([0x48, 0x8b, 0x05], [0x48, 0xc7, 0xc0]),
    ] {
        let mut data = insn.to_vec();
        data.extend_from_slice(&[0; 4]);
        let out = one(
            X86_64,
            EXEC,
            data,
            (3, x64::R_GOTTPOFF, -4),
            Referent::Tls(8),
        )
        .unwrap();
        assert_eq!(out[..3], expected);
        assert_eq!(out[3..], (-0x18i32).to_le_bytes());
    }
    refused(
        one(
            X86_64,
            EXEC,
            vec![0x48, 0x8d, 0x05, 0, 0, 0, 0],
            (3, x64::R_GOTTPOFF, -4),
            Referent::Tls(8),
        ),
        Code::U8001,
        "initial-exec",
    );
    assert_eq!(
        one(
            X86_64,
            EXEC,
            vec![0; 4],
            (0, x64::R_TPOFF32, 4),
            Referent::Tls(8)
        )
        .unwrap(),
        (-0x14i32).to_le_bytes()
    );
    refused(
        one(
            X86_64,
            EXEC,
            vec![0; 4],
            (0, x64::R_TPOFF32, 0),
            Referent::Tls(0x21),
        ),
        Code::U8001,
        "outside the TLS template",
    );
}

#[test]
fn arm64_page_offset_and_branch_vectors() {
    let s = Referent::Address(0x2_3456);
    // adrp x0 at 0x10000 to 0x23456: 0x13 pages.
    let mut section = sec(".text", EXEC, le(0x9000_0000));
    section.relocs = vec![(0, 1, a64::R_ADR_PREL_PG_HI21, 0)];
    let (bytes, _, _) = apply(
        AARCH64,
        section,
        &[sym("t", GLOBAL_NOTYPE, ABS, 0)],
        &[s],
        0x1_0000,
    )
    .unwrap();
    assert_eq!(bytes, le(0x9000_0000 | (3 << 29) | (4 << 5)));
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0x9100_0000),
            (0, a64::R_ADD_ABS_LO12_NC, 0),
            s
        )
        .unwrap(),
        le(0x9100_0000 | (0x456 << 10))
    );
    // ldr x (8 bytes) at an offset not a multiple of 8 is refused; a 32-bit type on a 64-bit
    // load is refused.
    refused(
        one(
            AARCH64,
            EXEC,
            le(0xf940_0000),
            (0, a64::R_LDST64_ABS_LO12_NC, 0),
            Referent::Address(0x2004),
        ),
        Code::U8001,
        "not aligned",
    );
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0xf940_0000),
            (0, a64::R_LDST64_ABS_LO12_NC, 0),
            Referent::Address(0x2008)
        )
        .unwrap(),
        le(0xf940_0000 | (1 << 10))
    );
    refused(
        one(
            AARCH64,
            EXEC,
            le(0xf940_0000),
            (0, a64::R_LDST32_ABS_LO12_NC, 0),
            Referent::Address(0x2008),
        ),
        Code::U8001,
        "not of the class",
    );
    // ldr q (16 bytes)
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0x3dc0_0000),
            (0, a64::R_LDST128_ABS_LO12_NC, 0),
            Referent::Address(0x2010)
        )
        .unwrap(),
        le(0x3dc0_0000 | (1 << 10))
    );
    // Branches: each reach, its boundary, and the class of its instruction.
    let base: i64 = 0x1000_0000;
    let branch = |insn: u32, kind: u32, delta: i64| {
        let mut section = sec(".text", EXEC, le(insn));
        section.relocs = vec![(0, 1, kind, 0)];
        let target = Referent::Address(u64::try_from(base + delta).unwrap());
        apply(
            AARCH64,
            section,
            &[sym("t", GLOBAL_NOTYPE, ABS, 0)],
            &[target],
            base as u64,
        )
        .map(|(bytes, _, _)| bytes)
    };
    assert_eq!(
        branch(0x9400_0000, a64::R_CALL26, 0x1000).unwrap(),
        le(0x9400_0400)
    );
    assert!(branch(0x9400_0000, a64::R_CALL26, (128 << 20) - 4).is_ok());
    assert!(branch(0x1400_0000, a64::R_JUMP26, -(128 << 20)).is_ok());
    refused(
        branch(0x1400_0000, a64::R_JUMP26, -(128 << 20) - 4),
        Code::U7015,
        "beyond its reach",
    );
    refused(
        branch(0x9400_0000, a64::R_CALL26, 128 << 20),
        Code::U7015,
        "beyond its reach",
    );
    refused(
        branch(0x9400_0000, a64::R_CALL26, 6),
        Code::U8001,
        "not 4-byte aligned",
    );
    refused(
        branch(0xd503_201f, a64::R_CALL26, 4),
        Code::U8001,
        "not of the class",
    );
    assert_eq!(
        branch(0x5400_0000, a64::R_CONDBR19, 0x100).unwrap(),
        le(0x5400_0800)
    );
    assert!(branch(0xb400_0000, a64::R_CONDBR19, (1 << 20) - 4).is_ok());
    refused(
        branch(0xb400_0000, a64::R_CONDBR19, 1 << 20),
        Code::U7015,
        "beyond its reach",
    );
    assert_eq!(
        branch(0x3718_0000, a64::R_TSTBR14, 8).unwrap(),
        le(0x3718_0040)
    );
    assert!(branch(0x3718_0000, a64::R_TSTBR14, -(1 << 15)).is_ok());
    refused(
        branch(0x3718_0000, a64::R_TSTBR14, 1 << 15),
        Code::U7015,
        "beyond its reach",
    );
    // ADR reaches ±1 MiB; ADRP ±4 GiB.
    assert!(branch(0x1000_0000, a64::R_ADR_PREL_LO21, (1 << 20) - 1).is_ok());
    refused(
        branch(0x1000_0000, a64::R_ADR_PREL_LO21, 1 << 20),
        Code::U7015,
        "signed field of 21 bits",
    );
    refused(
        branch(0x9000_0000, a64::R_ADR_PREL_PG_HI21, 1 << 32),
        Code::U7015,
        "signed field of 21 bits",
    );
    // A relocation of an instruction at a place that is not 4-byte aligned.
    refused(
        one(
            AARCH64,
            EXEC,
            vec![0; 8],
            (2, a64::R_CALL26, 0),
            Referent::Address(0x2000),
        ),
        Code::U8001,
        "4-byte aligned",
    );
}

#[test]
fn arm64_absolute_and_got_vectors() {
    let big = Referent::Absolute(0x1234_5678_9abc);
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0xd2e0_0004),
            (0, a64::R_MOVW_UABS_G3, 0),
            big
        )
        .unwrap(),
        le(0xd2e0_0004)
    );
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0xf2c0_0004),
            (0, a64::R_MOVW_UABS_G2_NC, 0),
            big
        )
        .unwrap(),
        le(0xf2c0_0004 | (0x1234 << 5))
    );
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0xf280_0004),
            (0, a64::R_MOVW_UABS_G0_NC, 0),
            big
        )
        .unwrap(),
        le(0xf280_0004 | (0x9abc << 5))
    );
    refused(
        one(
            AARCH64,
            EXEC,
            le(0xd280_0005),
            (0, a64::R_MOVW_UABS_G0, 0),
            Referent::Absolute(0x1_0000),
        ),
        Code::U8001,
        "unsigned field of 16 bits",
    );
    refused(
        one(
            AARCH64,
            EXEC,
            le(0x9100_0000),
            (0, a64::R_MOVW_UABS_G0, 0),
            Referent::Absolute(1),
        ),
        Code::U8001,
        "not of the class",
    );
    assert_eq!(
        one(
            AARCH64,
            WRITE,
            vec![0; 4],
            (0, a64::R_ABS32, 0),
            Referent::Absolute(u64::MAX - 7)
        )
        .unwrap(),
        (-8i32).to_le_bytes()
    );
    refused(
        one(
            AARCH64,
            WRITE,
            vec![0; 4],
            (0, a64::R_ABS32, 0),
            Referent::Absolute(1 << 32),
        ),
        Code::U8001,
        "field of 32 bits",
    );
    // The GOT: adrp and ldr of the slot at 0x3000; an addend is refused.
    let mut section = sec(".text", EXEC, [le(0x9000_0003), le(0xf940_0063)].concat());
    section.relocs = vec![
        (0, 1, a64::R_ADR_GOT_PAGE, 0),
        (4, 1, a64::R_LD64_GOT_LO12_NC, 0),
    ];
    let (bytes, _, got) = apply(
        AARCH64,
        section,
        &[sym("d", GLOBAL_OBJECT, ABS, 0)],
        &[Referent::Address(0x2000)],
        TEXT,
    )
    .unwrap();
    assert_eq!(got, [1]);
    assert_eq!(
        bytes,
        [le(0x9000_0003 | (2 << 29)), le(0xf940_0063)].concat()
    );
    refused(
        one(
            AARCH64,
            EXEC,
            le(0x9000_0003),
            (0, a64::R_ADR_GOT_PAGE, 8),
            Referent::Address(0x2000),
        ),
        Code::U8001,
        "addend",
    );
    refused(
        one(
            AARCH64,
            EXEC,
            le(0xb940_0063),
            (0, a64::R_LD64_GOT_LO12_NC, 0),
            Referent::Address(0x2000),
        ),
        Code::U8001,
        "not of the class",
    );
}

#[test]
fn arm64_thread_local_sequences_relax_to_local_exec() {
    // The variable at 0x10 of a template aligned to 16: thread-pointer offset 16 + 0x10.
    let v = Referent::Tls(0x10);
    let desc = [
        le(0x9000_0000),
        le(0xf940_0001),
        le(0x9100_0000),
        le(0xd63f_0020),
    ]
    .concat();
    let mut section = sec(".text", EXEC, desc.clone());
    section.relocs = vec![
        (0, 1, a64::R_TLSDESC_ADR_PAGE21, 0),
        (4, 1, a64::R_TLSDESC_LD64_LO12, 0),
        (8, 1, a64::R_TLSDESC_ADD_LO12, 0),
        (12, 1, a64::R_TLSDESC_CALL, 0),
    ];
    let symbols = [sym("v", GLOBAL_TLS, ABS, 0)];
    let (bytes, _, got) = apply(AARCH64, section.clone(), &symbols, &[v], TEXT).unwrap();
    assert_eq!(
        bytes,
        [
            le(0xd2a0_0000),
            le(0xf280_0000 | (0x20 << 5)),
            le(0xd503_201f),
            le(0xd503_201f)
        ]
        .concat()
    );
    assert!(got.is_empty());
    // The sequence on another register than x0 is refused.
    section.data[0] = 0x01;
    refused(
        apply(AARCH64, section, &symbols, &[v], TEXT),
        Code::U8001,
        "registers",
    );
    // Initial-exec on x6.
    let mut section = sec(".text", EXEC, [le(0x9000_0006), le(0xf940_00c6)].concat());
    section.relocs = vec![
        (0, 1, a64::R_TLSIE_ADR_GOTTPREL_PAGE21, 0),
        (4, 1, a64::R_TLSIE_LD64_GOTTPREL_LO12_NC, 0),
    ];
    let (bytes, _, _) = apply(AARCH64, section, &symbols, &[v], TEXT).unwrap();
    assert_eq!(bytes, [le(0xd2a0_0006), le(0xf280_0406)].concat());
    // A load into another register than its base cannot be relaxed.
    refused(
        one(
            AARCH64,
            EXEC,
            le(0xf940_00c7),
            (0, a64::R_TLSIE_LD64_GOTTPREL_LO12_NC, 0),
            v,
        ),
        Code::U8001,
        "registers",
    );
    // Local-exec: a negative offset turns MOVZ into MOVN of the complement.
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0xd280_0007),
            (0, a64::R_TLSLE_MOVW_TPREL_G0, -0x40),
            v
        )
        .unwrap(),
        le(0x9280_0007 | (0x1f << 5))
    );
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0x9100_00e7),
            (0, a64::R_TLSLE_ADD_TPREL_LO12_NC, 0),
            v
        )
        .unwrap(),
        le(0x9100_00e7 | (0x20 << 10))
    );
    assert_eq!(
        one(
            AARCH64,
            EXEC,
            le(0x9140_00e7),
            (0, a64::R_TLSLE_ADD_TPREL_HI12, 0x1000),
            v
        )
        .unwrap(),
        le(0x9140_00e7 | (1 << 10))
    );
    refused(
        one(
            AARCH64,
            EXEC,
            le(0x9100_00e7),
            (0, a64::R_TLSLE_ADD_TPREL_HI12, 0),
            v,
        ),
        Code::U8001,
        "not of the class",
    );
    refused(
        one(
            AARCH64,
            EXEC,
            le(0x9140_00e7),
            (0, a64::R_TLSLE_ADD_TPREL_HI12, 1 << 24),
            v,
        ),
        Code::U7015,
        "unsigned field of 24 bits",
    );
}

#[test]
fn relocations_outside_their_section_or_overlapping_are_refused() {
    let addr = Referent::Address(0x2000);
    refused(
        one(X86_64, EXEC, vec![0; 4], (1, x64::R_PC32, 0), addr),
        Code::U8001,
        "lie outside the section",
    );
    refused(
        one(
            X86_64,
            EXEC,
            vec![0; 4],
            (u64::MAX - 1, x64::R_PC32, 0),
            addr,
        ),
        Code::U8001,
        "lie outside the section",
    );
    refused(
        one(
            AARCH64,
            WRITE,
            vec![0; 8],
            (4, a64::R_ABS64, 0),
            Referent::Absolute(1),
        ),
        Code::U8001,
        "lie outside the section",
    );
    // A relaxed sequence needs its prefix inside the section.
    refused(
        one(
            X86_64,
            EXEC,
            vec![0; 16],
            (2, x64::R_TLSGD, -4),
            Referent::Tls(0),
        ),
        Code::U8001,
        "lie outside the section",
    );
    let mut section = sec(".text", EXEC, vec![0; 8]);
    section.relocs = vec![(0, 1, x64::R_PC32, 0), (2, 1, x64::R_PC32, 0)];
    refused(
        apply(
            X86_64,
            section,
            &[sym("t", GLOBAL_NOTYPE, ABS, 0)],
            &[addr],
            TEXT,
        ),
        Code::U8001,
        "overlap",
    );
    // A relocation of a zero-fill section.
    let mut bss = sec(".bss", WRITE, Vec::new());
    bss.kind = 8;
    bss.size = 16;
    bss.relocs = vec![(0, 1, x64::R_64, 0)];
    refused(
        apply(
            X86_64,
            bss,
            &[sym("t", GLOBAL_NOTYPE, ABS, 0)],
            &[addr],
            TEXT,
        ),
        Code::U8001,
        "zero-fill",
    );
}

#[test]
fn types_outside_the_list_are_refused_in_loaded_sections_and_ignored_elsewhere() {
    // IRELATIVE, R_X86_64_GOT32, the checked TLSLE LO12 forms and the tiny TLSDESC forms of arm64.
    for (machine, kind) in [
        (X86_64, x64::R_IRELATIVE),
        (X86_64, 3),
        (AARCH64, a64::R_IRELATIVE),
        (AARCH64, 550),
        (AARCH64, 552),
        (AARCH64, 560),
        (AARCH64, 561),
    ] {
        refused(
            one(
                machine,
                WRITE,
                vec![0; 8],
                (0, kind, 0),
                Referent::Address(0x2000),
            ),
            Code::U8001,
            "outside the supported set",
        );
        let mut debug = sec(".debug_info", 0, vec![0; 8]);
        debug.flags = 0;
        debug.relocs = vec![(0, 1, kind, 0)];
        let bytes = object(machine, &[debug], &[sym("t", GLOBAL_NOTYPE, ABS, 0)]);
        let object = input::read("test.o", &bytes).unwrap();
        assert!(object.sections[1].relocs.is_empty());
        let plan = reloc::plan(&object, 1, &|_| Ok(Referent::Address(0))).unwrap();
        assert!(plan.steps.is_empty());
    }
    // The scan accepts exactly the engine's list.
    for (arch, list) in [
        (super::Arch::X86_64, x64::TYPES),
        (super::Arch::Aarch64, a64::TYPES),
    ] {
        for t in list {
            assert_eq!(reloc::find(arch, t.value), Some(t));
        }
    }
    assert_eq!(x64::TYPES.len(), 14);
    assert_eq!(a64::TYPES.len(), 43);
}

/// The section header fields of `bytes` at `index`, `field` bytes from its start, replaced.
fn patch(mut bytes: Vec<u8>, at: usize, value: &[u8]) -> Vec<u8> {
    bytes[at..at + value.len()].copy_from_slice(value);
    bytes
}

fn read_refused(bytes: &[u8], code: Code, words: &str) {
    refused(input::read("bad.o", bytes).map(|_| ()), code, words);
}

#[test]
fn the_reader_refuses_malformed_objects() {
    let text = || sec(".text", EXEC, vec![0xc3; 8]);
    let good = object(X86_64, &[text()], &[sym("f", GLOBAL_FUNC, 1, 0)]);
    assert!(input::read("good.o", &good).is_ok());
    for length in [0, 4, 16, 63, 64, good.len() - 1] {
        assert!(input::read("cut.o", &good[..length]).is_err(), "{length}");
    }
    read_refused(
        &patch(good.clone(), 16, &2u16.to_le_bytes()),
        Code::U8001,
        "not a relocatable",
    );
    read_refused(
        &patch(good.clone(), 18, &3u16.to_le_bytes()),
        Code::U8001,
        "machine 3",
    );
    // Symbol past the end of its section; in a section the object does not have; reserved.
    let mut far = sym("f", GLOBAL_FUNC, 1, 4);
    far.size = 5;
    read_refused(
        &object(X86_64, &[text()], &[far]),
        Code::U8001,
        "past the end",
    );
    let mut huge = sym("f", GLOBAL_FUNC, 1, u64::MAX);
    huge.size = 2;
    read_refused(
        &object(X86_64, &[text()], &[huge]),
        Code::U8001,
        "past the end",
    );
    read_refused(
        &object(X86_64, &[text()], &[sym("f", GLOBAL_FUNC, 40, 0)]),
        Code::U8001,
        "",
    );
    read_refused(
        &object(X86_64, &[text()], &[sym("f", GLOBAL_FUNC, 0xff00, 0)]),
        Code::U8001,
        "reserved",
    );
    // Bindings: unknown, local after global, undefined local; an indirect function.
    read_refused(
        &object(X86_64, &[text()], &[sym("f", 0x32, 1, 0)]),
        Code::U8001,
        "binding 3",
    );
    read_refused(
        &object(
            X86_64,
            &[text()],
            &[sym("g", GLOBAL_FUNC, 1, 0), sym("l", LOCAL, 1, 0)],
        ),
        Code::U8001,
        "out of place",
    );
    read_refused(
        &object(X86_64, &[text()], &[sym("l", LOCAL, 0, 0)]),
        Code::U8001,
        "local and undefined",
    );
    read_refused(
        &object(X86_64, &[text()], &[sym("f", 0x1a, 1, 0)]),
        Code::U8001,
        "STT_GNU_IFUNC",
    );
    // Thread-local symbols only in thread-local sections, and the reverse.
    read_refused(
        &object(X86_64, &[text()], &[sym("v", GLOBAL_TLS, 1, 0)]),
        Code::U8001,
        "thread-local data only",
    );
    let tdata = sec(".tdata", WRITE | TLS, vec![0; 8]);
    read_refused(
        &object(
            X86_64,
            std::slice::from_ref(&tdata),
            &[sym("v", GLOBAL_OBJECT, 1, 0)],
        ),
        Code::U8001,
        "thread-local data only",
    );
    assert!(
        input::read(
            "ok.o",
            &object(X86_64, &[tdata], &[sym("v", GLOBAL_TLS, 1, 0)])
        )
        .is_ok()
    );
    read_refused(
        &object(X86_64, &[text()], &[sym("c", GLOBAL_TLS, COMMON, 8)]),
        Code::U8001,
        "thread-local common",
    );
    read_refused(
        &object(X86_64, &[text()], &[sym("c", GLOBAL_OBJECT, COMMON, 3)]),
        Code::U8001,
        "power of two",
    );
    // Sections: writable and executable, compressed, unknown type, alignment.
    read_refused(
        &object(X86_64, &[sec(".wx", WRITE | EXEC, vec![0; 4])], &[]),
        Code::U8001,
        "writable and executable",
    );
    let mut compressed = text();
    compressed.flags |= 0x800;
    read_refused(
        &object(X86_64, &[compressed], &[]),
        Code::U8001,
        "compressed",
    );
    let mut unknown = text();
    unknown.kind = 0x6fff_fff0;
    read_refused(
        &object(X86_64, &[unknown], &[]),
        Code::U8001,
        "does not know",
    );
    let mut odd = text();
    odd.align = 12;
    read_refused(&object(X86_64, &[odd], &[]), Code::U8001, "power of two");
    let mut wide = text();
    wide.align = 0x2_0000;
    read_refused(
        &object(X86_64, &[wide], &[]),
        Code::U7015,
        "beyond the 64 KiB",
    );
    let mut stack = sec(".note.GNU-stack", EXEC, Vec::new());
    stack.flags = EXEC;
    read_refused(
        &object(X86_64, &[stack], &[]),
        Code::U8001,
        "executable stack",
    );
    // Relocations: a symbol index past the table, the null symbol, a REL section, a wrong
    // entry size, a section relocated twice.
    let mut relocated = text();
    relocated.relocs = vec![(0, 9, x64::R_PC32, 0)];
    read_refused(
        &object(X86_64, &[relocated.clone()], &[sym("f", GLOBAL_FUNC, 1, 0)]),
        Code::U8001,
        "symbol 9",
    );
    relocated.relocs = vec![(0, 0, x64::R_PC32, 0)];
    read_refused(
        &object(X86_64, &[relocated.clone()], &[sym("f", GLOBAL_FUNC, 1, 0)]),
        Code::U8001,
        "symbol 0",
    );
    relocated.relocs = vec![(0, 1, x64::R_PC32, 0)];
    let ok = object(X86_64, &[relocated], &[sym("f", GLOBAL_FUNC, 1, 0)]);
    assert!(input::read("ok.o", &ok).is_ok());
    // The `.rela.text` header is 2: its type (offset 4), its entry size (offset 56).
    let rela = header(&ok, 2);
    read_refused(
        &patch(ok.clone(), rela + 4, &9u32.to_le_bytes()),
        Code::U8001,
        "",
    );
    read_refused(
        &patch(ok.clone(), rela + 56, &16u64.to_le_bytes()),
        Code::U8001,
        "Elf64_Rela",
    );
    // The section's bytes or the relocations past the end of the input.
    let text_header = header(&ok, 1);
    read_refused(
        &patch(ok.clone(), text_header + 24, &(1u64 << 40).to_le_bytes()),
        Code::U8001,
        "outside the input",
    );
    read_refused(
        &patch(ok.clone(), rela + 32, &(1u64 << 40).to_le_bytes()),
        Code::U8001,
        "",
    );
}

/// The objects whose every mutation the robustness test tries: relocations of every family on
/// both targets, thread-local data and a GOT.
fn seeds() -> Vec<Vec<u8>> {
    let mut x86 = sec(
        ".text",
        EXEC,
        [
            &[
                0x66, 0x48, 0x8d, 0x3d, 0, 0, 0, 0, 0x66, 0x66, 0x48, 0xe8, 0, 0, 0, 0,
            ][..],
            &[
                0x48, 0x8b, 0x05, 0, 0, 0, 0, 0xff, 0x15, 0, 0, 0, 0, 0x4c, 0x03, 0x25, 0, 0, 0, 0,
            ][..],
            &[0xe8, 0, 0, 0, 0, 0xc3][..],
        ]
        .concat(),
    );
    x86.relocs = vec![
        (4, 3, x64::R_TLSGD, -4),
        (12, 4, x64::R_PLT32, -4),
        (19, 1, x64::R_REX_GOTPCRELX, -4),
        (25, 2, x64::R_GOTPCRELX, -4),
        (32, 3, x64::R_GOTTPOFF, -4),
        (37, 2, x64::R_PLT32, -4),
    ];
    let mut data = sec(".data", WRITE, vec![0; 16]);
    data.relocs = vec![(0, 2, x64::R_64, 0), (8, 1, x64::R_PC32, 0)];
    let tdata = sec(".tdata", WRITE | TLS, vec![0; 8]);
    let symbols = [
        sym("d", GLOBAL_OBJECT, 2, 0),
        sym("f", GLOBAL_FUNC, 1, 0),
        sym("v", GLOBAL_TLS, 3, 0),
        sym("__tls_get_addr", GLOBAL_FUNC, 0, 0),
    ];
    let x86 = object(X86_64, &[x86, data.clone(), tdata.clone()], &symbols);
    let mut arm = sec(
        ".text",
        EXEC,
        [
            le(0x9000_0000),
            le(0xf940_0001),
            le(0x9100_0000),
            le(0xd63f_0020),
            le(0x9400_0000),
            le(0x9000_0003),
            le(0xf940_0063),
            le(0x3dc0_0000),
        ]
        .concat(),
    );
    arm.relocs = vec![
        (0, 3, a64::R_TLSDESC_ADR_PAGE21, 0),
        (4, 3, a64::R_TLSDESC_LD64_LO12, 0),
        (8, 3, a64::R_TLSDESC_ADD_LO12, 0),
        (12, 3, a64::R_TLSDESC_CALL, 0),
        (16, 4, a64::R_CALL26, 0),
        (20, 1, a64::R_ADR_GOT_PAGE, 0),
        (24, 1, a64::R_LD64_GOT_LO12_NC, 0),
        (28, 1, a64::R_LDST128_ABS_LO12_NC, 0),
    ];
    data.relocs = vec![(0, 2, a64::R_ABS64, 0), (8, 1, a64::R_PREL32, 0)];
    let arm = object(AARCH64, &[arm, data, tdata], &symbols);
    vec![x86, arm]
}

#[test]
fn every_mutation_of_an_object_fails_or_passes_without_panicking() {
    for seed in seeds() {
        let check = super::check::check_elf("seed.o", &seed).unwrap();
        assert!(check.relocations.len() >= 6, "{:?}", check.relocations);
        for length in 0..seed.len() {
            let _ = super::check::check_elf("cut.o", &seed[..length]);
        }
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for at in 0..seed.len() {
            for value in [
                0x00,
                0xff,
                0x80,
                0x7f,
                seed[at] ^ 1,
                seed[at].wrapping_add(8),
            ] {
                let mut bytes = seed.clone();
                bytes[at] = value;
                let _ = super::check::check_elf("mutated.o", &bytes);
            }
            // A random 8-byte word, so counts and offsets take large values too.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mut bytes = seed.clone();
            let end = (at + 8).min(bytes.len());
            bytes[at..end].copy_from_slice(&state.to_le_bytes()[..end - at]);
            let _ = super::check::check_elf("mutated.o", &bytes);
        }
    }
}

#[test]
fn the_plan_and_the_writes_agree_on_every_action() {
    for seed in seeds() {
        let object = input::read("seed.o", &seed).unwrap();
        for index in 0..object.sections.len() {
            let plan = reloc::plan(&object, index, &|symbol| {
                Ok(match object.symbol(symbol)?.kind {
                    super::STT_TLS => Referent::Tls(0),
                    _ => Referent::Address(0x2000),
                })
            })
            .unwrap();
            for step in &plan.steps {
                assert!(step.start <= step.end);
                assert!(step.end <= object.sections[index].size);
                if step.action == Action::Got {
                    assert!(
                        plan.got
                            .contains(&object.sections[index].relocs[step.index].symbol)
                    );
                }
            }
        }
    }
}
