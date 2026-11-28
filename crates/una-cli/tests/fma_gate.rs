//! The FMA disassembly gate, `scripts/check-fma.sh` (#1216; ADR 0114 Appendix C rules 7 and 8,
//! as ADR 0120 Appendix A amends them), which `scripts/check-release.sh` runs on the shipped
//! `una` and `libuna_rt.a`: an FMA instruction planted in a symbol outside the allowlist fails
//! it, and the same instruction in a symbol of an allowlisted variant passes.
//!
//! The planted objects are x86-64 ELF relocatable files that this test writes byte by byte, so
//! it needs no compiler, no assembler and no installed target: only a disassembler that reads
//! x86-64, which the gate needs too. Where none does (GNU `objdump` of another architecture),
//! the test prints that it is skipped; the x86-64 leg, `scripts/local-ci.sh --linux`, runs it.
//!
//! The same canaries in Mach-O (#1356): an x86-64 Mach-O object and an archive of it, which
//! the `objdump -f` of LLVM refuses and the gate reads with `otool -hv`, fail it; an arm64
//! Mach-O object, as the macOS `una` and `libuna_rt.a` are, is not applicable; a universal
//! file fails. They run on macOS, where a stock Xcode toolchain has `objdump` and `otool`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// `vfmadd231sd %xmm2, %xmm1, %xmm0` (VEX.LIG.66.0F38.W1 B9 /r), then `ret`.
const FMA: &[u8] = &[0xC4, 0xE2, 0xF1, 0xB9, 0xC2, 0xC3];

/// `addsd %xmm1, %xmm0`, then `ret`: no FMA.
const ADD: &[u8] = &[0xF2, 0x0F, 0x58, 0xC1, 0xC3];

/// The legacy-mangled name of `una_core::dense::variants::x86_64_v3::lu`, as rustc writes it.
const VARIANT: &str = "_ZN8una_core5dense8variants9x86_64_v32lu17h0123456789abcdefE";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn u16le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn u64le(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn pad(out: &mut Vec<u8>, align: usize) {
    while !out.len().is_multiple_of(align) {
        out.push(0);
    }
}

/// An x86-64 ELF relocatable object whose `.text` holds each function of `functions`, a name
/// and its code, as a global function symbol.
fn object(functions: &[(&str, &[u8])]) -> Vec<u8> {
    let mut text = Vec::new();
    let mut strtab = vec![0_u8];
    let mut symbols = Vec::new();
    for (name, code) in functions {
        pad(&mut text, 16);
        let name_offset = u32::try_from(strtab.len()).unwrap();
        strtab.extend_from_slice(name.as_bytes());
        strtab.push(0);
        symbols.push((name_offset, text.len() as u64, code.len() as u64));
        text.extend_from_slice(code);
    }
    let shstrtab = b"\0.text\0.symtab\0.strtab\0.shstrtab\0";
    let mut out = vec![0_u8; 64];
    let text_offset = out.len();
    out.extend_from_slice(&text);
    pad(&mut out, 8);
    let symtab_offset = out.len();
    out.extend_from_slice(&[0; 24]);
    for (name, value, size) in &symbols {
        u32le(&mut out, *name);
        out.push(0x12); // STB_GLOBAL, STT_FUNC
        out.push(0);
        u16le(&mut out, 1); // .text
        u64le(&mut out, *value);
        u64le(&mut out, *size);
    }
    let symtab_size = out.len() - symtab_offset;
    let strtab_offset = out.len();
    out.extend_from_slice(&strtab);
    let shstrtab_offset = out.len();
    out.extend_from_slice(shstrtab);
    pad(&mut out, 8);
    let section_headers = out.len();
    let mut section = |name: u32,
                       kind: u32,
                       flags: u64,
                       offset: usize,
                       size: usize,
                       link: u32,
                       info: u32,
                       align: u64,
                       entry: u64| {
        u32le(&mut out, name);
        u32le(&mut out, kind);
        u64le(&mut out, flags);
        u64le(&mut out, 0);
        u64le(&mut out, offset as u64);
        u64le(&mut out, size as u64);
        u32le(&mut out, link);
        u32le(&mut out, info);
        u64le(&mut out, align);
        u64le(&mut out, entry);
    };
    section(0, 0, 0, 0, 0, 0, 0, 0, 0);
    section(1, 1, 6, text_offset, text.len(), 0, 0, 16, 0); // .text, ALLOC | EXECINSTR
    section(7, 2, 0, symtab_offset, symtab_size, 3, 1, 8, 24); // .symtab, link .strtab
    section(15, 3, 0, strtab_offset, strtab.len(), 0, 0, 1, 0); // .strtab
    section(23, 3, 0, shstrtab_offset, shstrtab.len(), 0, 0, 1, 0); // .shstrtab
    let mut header = Vec::new();
    header.extend_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 0]);
    header.extend_from_slice(&[0; 8]);
    u16le(&mut header, 1); // ET_REL
    u16le(&mut header, 62); // EM_X86_64
    u32le(&mut header, 1);
    u64le(&mut header, 0);
    u64le(&mut header, 0);
    u64le(&mut header, section_headers as u64);
    u32le(&mut header, 0);
    u16le(&mut header, 64);
    u16le(&mut header, 0);
    u16le(&mut header, 0);
    u16le(&mut header, 64);
    u16le(&mut header, 5);
    u16le(&mut header, 4);
    out[..64].copy_from_slice(&header);
    out
}

/// The CPU type of x86-64 in a Mach-O header (`CPU_TYPE_X86_64`).
const MACHO_X86_64: u32 = 0x0100_0007;

/// The CPU type of arm64 in a Mach-O header (`CPU_TYPE_ARM64`).
const MACHO_ARM64: u32 = 0x0100_000C;

/// `ret` on arm64.
const ARM64_RET: &[u8] = &[0xC0, 0x03, 0x5F, 0xD6];

/// A Mach-O relocatable object (`MH_OBJECT`) for `cpu` whose `__TEXT,__text` holds each
/// function of `functions`, a name and its code, as an external symbol of that section.
fn macho_object(cpu: u32, functions: &[(&str, &[u8])]) -> Vec<u8> {
    let mut text = Vec::new();
    let mut strtab = vec![0_u8];
    let mut symbols = Vec::new();
    for (name, code) in functions {
        pad(&mut text, 16);
        let name_offset = u32::try_from(strtab.len()).unwrap();
        strtab.extend_from_slice(name.as_bytes());
        strtab.push(0);
        symbols.push((name_offset, text.len() as u64));
        text.extend_from_slice(code);
    }
    pad(&mut strtab, 8);
    let segment_size = 72 + 80;
    let symtab_size = 24;
    let commands = segment_size + symtab_size;
    let text_offset = 32 + commands;
    let symbols_offset = text_offset + text.len().next_multiple_of(8);
    let strings_offset = symbols_offset + 16 * symbols.len();
    let fixed = |name: &str| {
        let mut field = [0_u8; 16];
        field[..name.len()].copy_from_slice(name.as_bytes());
        field
    };
    let mut out = Vec::new();
    u32le(&mut out, 0xFEED_FACF); // MH_MAGIC_64
    u32le(&mut out, cpu);
    u32le(&mut out, if cpu == MACHO_X86_64 { 3 } else { 0 }); // the "all" subtype
    u32le(&mut out, 1); // MH_OBJECT
    u32le(&mut out, 2);
    u32le(&mut out, u32::try_from(commands).unwrap());
    u32le(&mut out, 0);
    u32le(&mut out, 0);
    // LC_SEGMENT_64 with one section, __TEXT,__text.
    u32le(&mut out, 0x19);
    u32le(&mut out, u32::try_from(segment_size).unwrap());
    out.extend_from_slice(&fixed(""));
    u64le(&mut out, 0);
    u64le(&mut out, text.len() as u64);
    u64le(&mut out, text_offset as u64);
    u64le(&mut out, text.len() as u64);
    u32le(&mut out, 7);
    u32le(&mut out, 7);
    u32le(&mut out, 1);
    u32le(&mut out, 0);
    out.extend_from_slice(&fixed("__text"));
    out.extend_from_slice(&fixed("__TEXT"));
    u64le(&mut out, 0);
    u64le(&mut out, text.len() as u64);
    u32le(&mut out, u32::try_from(text_offset).unwrap());
    u32le(&mut out, 4); // aligned to 2^4
    u32le(&mut out, 0);
    u32le(&mut out, 0);
    u32le(&mut out, 0x8000_0400); // S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS
    u32le(&mut out, 0);
    u32le(&mut out, 0);
    u32le(&mut out, 0);
    // LC_SYMTAB.
    u32le(&mut out, 0x2);
    u32le(&mut out, u32::try_from(symtab_size).unwrap());
    u32le(&mut out, u32::try_from(symbols_offset).unwrap());
    u32le(&mut out, u32::try_from(symbols.len()).unwrap());
    u32le(&mut out, u32::try_from(strings_offset).unwrap());
    u32le(&mut out, u32::try_from(strtab.len()).unwrap());
    assert_eq!(out.len(), text_offset);
    out.extend_from_slice(&text);
    pad(&mut out, 8);
    for (name, value) in &symbols {
        u32le(&mut out, *name);
        out.push(0x0F); // N_SECT | N_EXT
        out.push(1);
        u16le(&mut out, 0);
        u64le(&mut out, *value);
    }
    out.extend_from_slice(&strtab);
    out
}

/// A Unix `ar` archive holding `member`, named `name` (at most 15 bytes), as `libuna_rt.a`
/// holds its objects.
fn archive(name: &str, member: &[u8]) -> Vec<u8> {
    let mut out = b"!<arch>\n".to_vec();
    let header = format!(
        "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
        0,
        0,
        0,
        644,
        member.len()
    );
    assert_eq!(header.len(), 60);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(member);
    pad(&mut out, 2);
    out
}

/// A universal (fat) Mach-O file holding `slices`, each a CPU type and its object.
fn universal(slices: &[(u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0xCAFE_BABE_u32.to_be_bytes());
    out.extend_from_slice(&u32::try_from(slices.len()).unwrap().to_be_bytes());
    let mut offset = (8 + 20 * slices.len()).next_multiple_of(16);
    for (cpu, object) in slices {
        let subtype: u32 = if *cpu == MACHO_X86_64 { 3 } else { 0 };
        for field in [
            *cpu,
            subtype,
            u32::try_from(offset).unwrap(),
            u32::try_from(object.len()).unwrap(),
            4,
        ] {
            out.extend_from_slice(&field.to_be_bytes());
        }
        offset = (offset + object.len()).next_multiple_of(16);
    }
    for (_, object) in slices {
        pad(&mut out, 16);
        out.extend_from_slice(object);
    }
    out
}

/// Whether the Mach-O canaries run here: on macOS, where the gate reads Mach-O with `otool`.
#[allow(clippy::print_stderr)]
fn reads_macho() -> bool {
    if cfg!(target_os = "macos") {
        return true;
    }
    eprintln!("skipped: the Mach-O canaries run on macOS, where the gate reads Mach-O with otool");
    false
}

/// Writes `bytes` to a fresh file named `name` under the test's directory.
fn write(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fma-gate");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The disassembler the gate picks, when it reads x86-64 here; otherwise the test is skipped,
/// and says so.
#[allow(clippy::print_stderr)]
fn disassembles(path: &Path) -> bool {
    for tool in ["objdump", "llvm-objdump"] {
        let Ok(output) = Command::new(tool).arg("-d").arg(path).output() else {
            continue;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && text.contains("vfmadd231sd") {
            return true;
        }
        eprintln!(
            "skipped: {tool} does not disassemble x86-64 here ({}); the x86-64 leg, scripts/local-ci.sh --linux, runs this test",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        // The gate takes the first disassembler that exists, as this test does.
        return false;
    }
    eprintln!("skipped: no objdump or llvm-objdump here");
    false
}

fn gate(args: &[&str], artifacts: &[&Path]) -> Output {
    Command::new("sh")
        .arg(root().join("scripts/check-fma.sh"))
        .args(args)
        .args(artifacts)
        .output()
        .unwrap()
}

#[test]
fn an_fma_instruction_outside_the_allowlist_fails_the_gate() {
    let planted = write(
        "planted.o",
        &object(&[("planted_outside_the_allowlist", FMA), ("clean", ADD)]),
    );
    if !disassembles(&planted) {
        return;
    }
    let output = gate(&[], &[&planted]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("FMA instructions outside the allowlist"),
        "{stderr}"
    );
    assert!(stderr.contains("planted_outside_the_allowlist"), "{stderr}");
    assert!(!stderr.contains("clean\t"), "{stderr}");
}

#[test]
fn an_fma_instruction_in_an_allowlisted_variant_passes_the_gate() {
    let variant = write("variant.o", &object(&[(VARIANT, FMA), ("clean", ADD)]));
    if !disassembles(&variant) {
        return;
    }
    let output = gate(&[], &[&variant]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("no FMA instruction outside the allowlist")
    );
    // The allowlist the report gives replaces the default: without the variant's prefix, the
    // same object fails.
    let output = gate(
        &["--allow", "una_core::dense::variants::x86_64_v4"],
        &[&variant],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_prefix_allows_only_its_own_module() {
    // `x86_64_v3x` is not inside `x86_64_v3`.
    let near = write(
        "near.o",
        &object(&[("una_core::dense::variants::x86_64_v3x::lu", FMA)]),
    );
    if !disassembles(&near) {
        return;
    }
    let output = gate(&[], &[&near]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_default_allowlist_is_the_kernels_allowlist() {
    let script = std::fs::read_to_string(root().join("scripts/check-fma.sh")).unwrap();
    for prefix in una_core::dense::variants::ALLOWLIST {
        assert!(script.contains(&format!("{prefix}\n")), "{prefix}");
    }
}

#[test]
fn a_macho_fma_instruction_outside_the_allowlist_fails_the_gate() {
    // #1356: the `objdump -f` of LLVM refuses Mach-O; the gate must still read the
    // architecture, from `otool -hv`, and scan an x86-64 Mach-O object and an archive of one.
    if !reads_macho() {
        return;
    }
    let planted = macho_object(
        MACHO_X86_64,
        &[("_planted_outside_the_allowlist", FMA), ("_clean", ADD)],
    );
    let object = write("planted-macho.o", &planted);
    let library = write("libplanted.a", &archive("planted.o", &planted));
    for artifact in [&object, &library] {
        let output = gate(&[], &[artifact]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{artifact:?}: {stderr}");
        assert!(
            stderr.contains("FMA instructions outside the allowlist"),
            "{artifact:?}: {stderr}"
        );
        assert!(
            stderr.contains("planted_outside_the_allowlist"),
            "{artifact:?}: {stderr}"
        );
    }
    // The same instruction in an allowlisted variant's symbol passes.
    let variant = write(
        "variant-macho.o",
        &macho_object(
            MACHO_X86_64,
            &[(&format!("_{VARIANT}"), FMA), ("_clean", ADD)],
        ),
    );
    let output = gate(&[], &[&variant]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn an_arm64_macho_artifact_is_not_applicable_and_a_universal_one_fails() {
    // #1356: the macOS `una` and `libuna_rt.a` are arm64 Mach-O, a file and an archive; the
    // gate reports them not applicable instead of failing on the format. A universal file,
    // whose x86-64 slice `objdump -f` does not name, is refused rather than passed.
    if !reads_macho() {
        return;
    }
    let arm = macho_object(MACHO_ARM64, &[("_main", ARM64_RET)]);
    let object = write("arm64-macho.o", &arm);
    let library = write("libarm64.a", &archive("arm64.o", &arm));
    for artifact in [&object, &library] {
        let output = gate(&[], &[artifact]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{artifact:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("not applicable"), "{artifact:?}: {stdout}");
    }
    let x86 = macho_object(MACHO_X86_64, &[("_planted_outside_the_allowlist", FMA)]);
    let fat = write(
        "universal-macho",
        &universal(&[(MACHO_X86_64, &x86), (MACHO_ARM64, &arm)]),
    );
    let output = gate(&[], &[&fat]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("universal"), "{stderr}");
}

/// The four exact names of ADR 0120's amendment of 2026-11-27 (#2106): the hardware paths of
/// the pinned toolchain's IEEE `fma` and `fmaf` in `compiler_builtins`.
const TOOLCHAIN_FMA: [&str; 4] = [
    "compiler_builtins::math::libm_math::arch::x86::fma::fma_with_fma",
    "compiler_builtins::math::libm_math::arch::x86::fma::fma_with_fma4",
    "compiler_builtins::math::libm_math::arch::x86::fma::fmaf_with_fma",
    "compiler_builtins::math::libm_math::arch::x86::fma::fmaf_with_fma4",
];

/// `fma_with_fma` as rustc mangles it (v0), with the crate disambiguator that the demangler
/// prints in brackets and the gate drops, as in the x86-64 `una` of 0.5.2's tree (#2106).
const TOOLCHAIN_FMA_V0: &str =
    "_RNvNtNtNtNtNtCseo7sFwwH0bC_17compiler_builtins4math9libm_math4arch3x863fma12fma_with_fma";

/// Writes `bytes` as `name` in a directory of its own, `case`, so that files named `una` or
/// `libuna_rt.a` of different tests never meet.
fn write_as(case: &str, name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("fma-gate-toolchain")
        .join(case);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn the_toolchain_fma_is_allowed_by_its_four_exact_names_in_una_and_the_runtime_only() {
    // ADR 0120's amendment of 2026-11-27 (#2106): with --toolchain-fma, an FMA instruction in
    // each of the four exact names passes, in a file named `una` or `libuna_rt.a`.
    let mut functions: Vec<(&str, &[u8])> = TOOLCHAIN_FMA.iter().map(|name| (*name, FMA)).collect();
    functions.push((TOOLCHAIN_FMA_V0, FMA));
    functions.push(("clean", ADD));
    let bytes = object(&functions);
    let una = write_as("allowed", "una", &bytes);
    if !disassembles(&una) {
        return;
    }
    let runtime = write_as("allowed", "libuna_rt.a", &bytes);
    let output = gate(&["--toolchain-fma"], &[&una, &runtime]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Without the option, the same file fails on each of them: the default allowlist is
    // unchanged.
    let output = gate(&[], &[&una]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    for name in TOOLCHAIN_FMA {
        assert!(stderr.contains(&format!("{name}\t")), "{name}: {stderr}");
    }
    // A fifth name of the same module, a name inside an allowed one (no prefix match), a near
    // name and a name of another crate still fail with the option.
    for (case, name) in [
        (
            "fifth",
            "compiler_builtins::math::libm_math::arch::x86::fma::fma_fallback",
        ),
        (
            "inside",
            "compiler_builtins::math::libm_math::arch::x86::fma::fma_with_fma::inner",
        ),
        (
            "near",
            "compiler_builtins::math::libm_math::arch::x86::fma::fma_with_fma5",
        ),
        (
            "module",
            "compiler_builtins::math::libm_math::arch::x86::fma",
        ),
        (
            "other",
            "my_crate::math::libm_math::arch::x86::fma::fma_with_fma",
        ),
    ] {
        let una = write_as(case, "una", &object(&[(name, FMA)]));
        let output = gate(&["--toolchain-fma"], &[&una]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{case}: {stderr}");
        assert!(stderr.contains(&format!("{name}\t")), "{case}: {stderr}");
    }
    // A program that `una build` writes keeps the allowlist alone: the option is refused for any
    // file but `una` and `libuna_rt.a`, and without it the names fail there.
    let program = write_as("program", "hello", &bytes);
    let output = gate(&["--toolchain-fma"], &[&una, &program]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("--toolchain-fma is for una and libuna_rt.a only"),
        "{stderr}"
    );
    let output = gate(&[], &[&program]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_release_check_gives_the_toolchain_names_to_una_and_the_runtime_alone() {
    // scripts/check-release.sh passes --toolchain-fma on its one scan of the shipped `una` and
    // `libuna_rt.a`; scripts/fma-canary.sh, which scans the programs it builds, does not.
    let release = std::fs::read_to_string(root().join("scripts/check-release.sh")).unwrap();
    let scans: Vec<&str> = release
        .lines()
        .filter(|line| line.trim_start().starts_with("scripts/check-fma.sh"))
        .collect();
    assert_eq!(
        scans,
        ["scripts/check-fma.sh \"$@\" --toolchain-fma \"$una\" \"$runtime\""]
    );
    let canary = std::fs::read_to_string(root().join("scripts/fma-canary.sh")).unwrap();
    assert!(!canary.contains("--toolchain-fma"));
    let script = std::fs::read_to_string(root().join("scripts/check-fma.sh")).unwrap();
    for name in TOOLCHAIN_FMA {
        assert!(script.contains(&format!("{name}\n")), "{name}");
    }
}
