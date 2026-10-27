//! The FMA disassembly gate, `scripts/check-fma.sh` (#1216; ADR 0114 Appendix C rules 7 and 8,
//! as ADR 0120 Appendix A amends them), which `scripts/check-release.sh` runs on the shipped
//! `una` and `libuna_rt.a`: an FMA instruction planted in a symbol outside the allowlist fails
//! it, and the same instruction in a symbol of an allowlisted variant passes.
//!
//! The planted objects are x86-64 ELF relocatable files that this test writes byte by byte, so
//! it needs no compiler, no assembler and no installed target: only a disassembler that reads
//! x86-64, which the gate needs too. Where none does (GNU `objdump` of another architecture),
//! the test prints that it is skipped; the x86-64 leg, `scripts/local-ci.sh --linux`, runs it.

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
