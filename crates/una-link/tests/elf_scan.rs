//! The scan of the ELF inputs of a Linux executable (ADR 0124 rules 12 and 24, step 4a): over the
//! pinned toolchain's musl inputs and standard library for both musl targets, and over
//! hand-made objects for each case it must refuse.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use una_link::{Code, scan_elf};

/// `EM_X86_64` and `EM_AARCH64`.
const X86_64: u16 = 62;
const AARCH64: u16 = 183;
/// Section flags.
const WRITE: u64 = 0x1;
const ALLOC: u64 = 0x2;
const EXEC: u64 = 0x4;
/// `SHN_ABS` and `SHN_COMMON`.
const ABS: u16 = 0xfff1;
const COMMON: u16 = 0xfff2;

/// A section of a hand-made object: its relocations are (offset, symbol index, type).
struct Section {
    name: &'static str,
    flags: u64,
    data: Vec<u8>,
    relocs: Vec<(u64, u32, u32)>,
}

/// A symbol: `info` is `st_info`, `section` the 1-based index of its section, or `ABS`,
/// `COMMON` or 0 (undefined).
struct Symbol {
    name: &'static str,
    info: u8,
    section: u16,
    value: u64,
    size: u64,
}

const LOCAL_OBJECT: u8 = 0x01;
const GLOBAL_OBJECT: u8 = 0x11;
const GLOBAL_FUNC: u8 = 0x12;
const GLOBAL_IFUNC: u8 = 0x1a;
const GLOBAL_NOTYPE: u8 = 0x10;

fn section(name: &'static str, flags: u64, relocs: Vec<(u64, u32, u32)>) -> Section {
    Section {
        name,
        flags,
        data: vec![0; 16],
        relocs,
    }
}

fn symbol(name: &'static str, info: u8, section: u16) -> Symbol {
    Symbol {
        name,
        info,
        section,
        value: 0,
        size: 8,
    }
}

/// A 64-bit little-endian ELF relocatable object for `machine`. Symbol `i` of `symbols` has the
/// index `i + 1`; local symbols must come first.
fn object(machine: u16, sections: &[Section], symbols: &[Symbol]) -> Vec<u8> {
    let mut shstrtab = vec![0u8];
    let mut name_of = |name: &str| {
        let at = u32::try_from(shstrtab.len()).unwrap();
        shstrtab.extend_from_slice(name.as_bytes());
        shstrtab.push(0);
        at
    };
    let mut strtab = vec![0u8];
    let mut symtab = vec![0u8; 24];
    for symbol in symbols {
        let at = u32::try_from(strtab.len()).unwrap();
        strtab.extend_from_slice(symbol.name.as_bytes());
        strtab.push(0);
        symtab.extend_from_slice(&at.to_le_bytes());
        symtab.push(symbol.info);
        symtab.push(0);
        symtab.extend_from_slice(&symbol.section.to_le_bytes());
        symtab.extend_from_slice(&symbol.value.to_le_bytes());
        symtab.extend_from_slice(&symbol.size.to_le_bytes());
    }
    let locals = 1 + symbols.iter().filter(|s| s.info >> 4 == 0).count();
    // Section headers: null, the sections, a `.rela` per section with relocations, `.symtab`,
    // `.strtab`, `.shstrtab`.
    let mut headers: Vec<[u64; 8]> = vec![[0; 8]];
    let mut body: Vec<u8> = Vec::new();
    let base = 64u64;
    let place = |body: &mut Vec<u8>, bytes: &[u8]| {
        while !body.len().is_multiple_of(8) {
            body.push(0);
        }
        let at = base + u64::try_from(body.len()).unwrap();
        body.extend_from_slice(bytes);
        at
    };
    let user = u64::try_from(sections.len()).unwrap();
    let rela_count =
        u64::try_from(sections.iter().filter(|s| !s.relocs.is_empty()).count()).unwrap();
    let symtab_index = 1 + user + rela_count;
    for s in sections {
        let at = place(&mut body, &s.data);
        let name = u64::from(name_of(s.name));
        headers.push([
            name | (1 << 32),
            s.flags,
            0,
            at,
            s.data.len() as u64,
            0,
            8,
            0,
        ]);
    }
    for (index, s) in sections.iter().enumerate() {
        if s.relocs.is_empty() {
            continue;
        }
        let mut rela = Vec::new();
        for &(offset, symbol, kind) in &s.relocs {
            rela.extend_from_slice(&offset.to_le_bytes());
            rela.extend_from_slice(&((u64::from(symbol) << 32) | u64::from(kind)).to_le_bytes());
            rela.extend_from_slice(&0i64.to_le_bytes());
        }
        let at = place(&mut body, &rela);
        let name = u64::from(name_of(&format!(".rela{}", s.name)));
        let info = u64::try_from(index).unwrap() + 1;
        headers.push([
            name | (4 << 32),
            0x40,
            0,
            at,
            rela.len() as u64,
            symtab_index | (info << 32),
            8,
            24,
        ]);
    }
    let at = place(&mut body, &symtab);
    let name = u64::from(name_of(".symtab"));
    headers.push([
        name | (2 << 32),
        0,
        0,
        at,
        symtab.len() as u64,
        (symtab_index + 1) | ((locals as u64) << 32),
        8,
        24,
    ]);
    let at = place(&mut body, &strtab);
    let name = u64::from(name_of(".strtab"));
    headers.push([name | (3 << 32), 0, 0, at, strtab.len() as u64, 0, 1, 0]);
    let name = u64::from(name_of(".shstrtab"));
    let shstrtab_bytes = shstrtab.clone();
    let at = place(&mut body, &shstrtab_bytes);
    headers.push([
        name | (3 << 32),
        0,
        0,
        at,
        shstrtab_bytes.len() as u64,
        0,
        1,
        0,
    ]);
    while !body.len().is_multiple_of(8) {
        body.push(0);
    }
    let shoff = base + body.len() as u64;
    let mut out = Vec::new();
    out.extend_from_slice(b"\x7fELF\x02\x01\x01\x00");
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&1u16.to_le_bytes()); // ET_REL
    out.extend_from_slice(&machine.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes()); // entry
    out.extend_from_slice(&0u64.to_le_bytes()); // phoff
    out.extend_from_slice(&shoff.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags
    out.extend_from_slice(&64u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&64u16.to_le_bytes());
    out.extend_from_slice(&u16::try_from(headers.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&u16::try_from(headers.len() - 1).unwrap().to_le_bytes());
    out.extend_from_slice(&body);
    for [
        name_type,
        flags,
        addr,
        offset,
        size,
        link_info,
        align,
        entsize,
    ] in headers
    {
        out.extend_from_slice(
            &u32::try_from(name_type & 0xffff_ffff)
                .unwrap()
                .to_le_bytes(),
        );
        out.extend_from_slice(&u32::try_from(name_type >> 32).unwrap().to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&addr.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(link_info & 0xffff_ffff)
                .unwrap()
                .to_le_bytes(),
        );
        out.extend_from_slice(&u32::try_from(link_info >> 32).unwrap().to_le_bytes());
        out.extend_from_slice(&align.to_le_bytes());
        out.extend_from_slice(&entsize.to_le_bytes());
    }
    out
}

/// A GNU `ar` archive of `members`, whose symbol table lists `(symbol, member index)`.
fn archive(members: &[(&str, Vec<u8>)], table: &[(&str, usize)]) -> Vec<u8> {
    let names: Vec<u8> = table
        .iter()
        .flat_map(|(n, _)| n.bytes().chain([0]))
        .collect();
    let table_size = 4 + 4 * table.len() + names.len();
    let header = |name: &str, size: usize| {
        let mut h = format!(
            "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            0, 0, 0, 644, size
        );
        h.truncate(60);
        h.into_bytes()
    };
    let mut offsets = Vec::new();
    let mut at = 8 + 60 + table_size + table_size % 2;
    for (_, bytes) in members {
        offsets.push(at);
        at += 60 + bytes.len() + bytes.len() % 2;
    }
    let mut out = b"!<arch>\n".to_vec();
    out.extend(header("/", table_size));
    out.extend_from_slice(&u32::try_from(table.len()).unwrap().to_be_bytes());
    for (_, member) in table {
        out.extend_from_slice(&u32::try_from(offsets[*member]).unwrap().to_be_bytes());
    }
    out.extend_from_slice(&names);
    if table_size % 2 == 1 {
        out.push(b'\n');
    }
    for (name, bytes) in members {
        out.extend(header(&format!("{name}/"), bytes.len()));
        out.extend_from_slice(bytes);
        if bytes.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

fn refused(bytes: &[u8]) -> String {
    let problem = scan_elf("fixture.o", bytes).unwrap_err();
    assert_eq!(problem.code, Code::U8001, "{problem}");
    problem.message
}

#[test]
fn an_irelative_relocation_is_refused_on_both_targets_even_where_not_loaded() {
    for (machine, kind, name) in [
        (X86_64, 37, "R_X86_64_IRELATIVE"),
        (AARCH64, 1032, "R_AARCH64_IRELATIVE"),
    ] {
        for flags in [ALLOC | WRITE, 0] {
            let bytes = object(
                machine,
                &[section(".data", flags, vec![(0, 1, kind)])],
                &[symbol("resolver", GLOBAL_FUNC, 1)],
            );
            let message = refused(&bytes);
            assert!(message.contains(name), "{message}");
            assert!(message.contains("indirect functions"), "{message}");
        }
    }
}

#[test]
fn an_indirect_function_is_refused() {
    let bytes = object(
        X86_64,
        &[section(".text", ALLOC | EXEC, vec![])],
        &[symbol("memcpy", GLOBAL_IFUNC, 1)],
    );
    let message = refused(&bytes);
    assert!(message.contains("memcpy"), "{message}");
    assert!(message.contains("STT_GNU_IFUNC"), "{message}");
}

#[test]
fn a_type_outside_rule_24_is_refused_in_a_loaded_section_and_counted_elsewhere() {
    // R_X86_64_GOT32 (3) and R_AARCH64_P32_ABS32 (1), types of neither list.
    for (machine, kind) in [(X86_64, 3), (AARCH64, 1)] {
        let loaded = object(
            machine,
            &[section(".text", ALLOC | EXEC, vec![(0, 1, kind)])],
            &[symbol("f", GLOBAL_FUNC, 1)],
        );
        let message = refused(&loaded);
        assert!(message.contains("outside the supported set"), "{message}");
        assert!(message.contains(".text"), "{message}");
        let debug = object(
            machine,
            &[section(".debug_info", 0, vec![(0, 1, kind)])],
            &[symbol("f", GLOBAL_FUNC, 1)],
        );
        let scan = scan_elf("fixture.o", &debug).unwrap();
        assert_eq!(scan.counts.values().sum::<u64>(), 1, "{:?}", scan.counts);
    }
}

#[test]
fn a_narrow_absolute_relocation_needs_an_absolute_symbol_in_a_loaded_section() {
    // R_X86_64_32 (10), R_X86_64_32S (11), R_AARCH64_ABS32 (258), R_AARCH64_MOVW_UABS_G0 (263).
    for (machine, kind) in [(X86_64, 10), (X86_64, 11), (AARCH64, 258), (AARCH64, 263)] {
        let against_local = object(
            machine,
            &[section(".text", ALLOC | EXEC, vec![(0, 1, kind)])],
            &[symbol("table", LOCAL_OBJECT, 1)],
        );
        let message = refused(&against_local);
        assert!(message.contains("narrow absolute"), "{message}");
        let against_absolute = object(
            machine,
            &[section(".text", ALLOC | EXEC, vec![(0, 1, kind)])],
            &[symbol("size", GLOBAL_NOTYPE, ABS)],
        );
        assert!(scan_elf("fixture.o", &against_absolute).is_ok());
    }
}

#[test]
fn a_64_bit_absolute_relocation_is_allowed_only_in_a_writable_section() {
    for (machine, kind) in [(X86_64, 1), (AARCH64, 257)] {
        let rodata = object(
            machine,
            &[
                section(".rodata", ALLOC, vec![(0, 2, kind)]),
                section(".text", ALLOC | EXEC, vec![]),
            ],
            &[symbol("f", GLOBAL_FUNC, 2), symbol("g", GLOBAL_FUNC, 2)],
        );
        let message = refused(&rodata);
        assert!(message.contains("text relocation"), "{message}");
        let data = object(
            machine,
            &[
                section(".data.rel.ro", ALLOC | WRITE, vec![(0, 2, kind)]),
                section(".text", ALLOC | EXEC, vec![]),
            ],
            &[symbol("f", GLOBAL_FUNC, 2), symbol("g", GLOBAL_FUNC, 2)],
        );
        let scan = scan_elf("fixture.o", &data).unwrap();
        assert_eq!(scan.counts.len(), 1, "{:?}", scan.counts);
    }
}

#[test]
fn common_symbols_are_listed_and_musl_s_version_is_read() {
    let mut rodata = section(".rodata.__libc_version", ALLOC, vec![]);
    rodata.data = b"1.2.5\0".to_vec();
    let mut version = symbol("__libc_version", GLOBAL_OBJECT, 1);
    version.size = 6;
    let bytes = object(
        AARCH64,
        &[rodata],
        &[version, symbol("h_errno", GLOBAL_OBJECT, COMMON)],
    );
    let scan = scan_elf("version.lo", &bytes).unwrap();
    assert_eq!(scan.musl_version.as_deref(), Some("1.2.5"));
    assert_eq!(
        scan.common.iter().collect::<Vec<_>>(),
        ["version.lo: h_errno"]
    );
    let mut bad = section(".rodata.__libc_version", ALLOC, vec![]);
    bad.data = b"1.2\0".to_vec();
    let mut version = symbol("__libc_version", GLOBAL_OBJECT, 1);
    version.size = 4;
    let message = refused(&object(AARCH64, &[bad], &[version]));
    assert!(message.contains("MAJOR.MINOR.PATCH"), "{message}");
}

#[test]
fn an_archive_symbol_table_must_name_the_member_that_defines_the_symbol() {
    let defines = object(
        X86_64,
        &[section(".text", ALLOC | EXEC, vec![])],
        &[symbol("memcpy", GLOBAL_FUNC, 1)],
    );
    let other = object(
        X86_64,
        &[section(".text", ALLOC | EXEC, vec![])],
        &[symbol("memset", GLOBAL_FUNC, 1)],
    );
    let good = archive(
        &[("memcpy.lo", defines.clone()), ("memset.lo", other.clone())],
        &[("memcpy", 0), ("memset", 1)],
    );
    assert_eq!(scan_elf("libc.a", &good).unwrap().indexed, 2);
    let wrong = archive(
        &[("memcpy.lo", defines), ("memset.lo", other)],
        &[("memcpy", 1), ("memset", 1)],
    );
    let problem = scan_elf("libc.a", &wrong).unwrap_err();
    assert_eq!(problem.code, Code::U8001);
    assert!(problem.message.contains("memcpy"), "{}", problem.message);
    assert!(
        problem.message.contains("does not define"),
        "{}",
        problem.message
    );
}

/// The pinned toolchain's library directory for `target`, which rust-toolchain.toml installs.
fn toolchain_lib(target: &str) -> PathBuf {
    // The crate's directory is inside the repository, so rustup applies rust-toolchain.toml.
    let output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("rustc runs");
    let sysroot = String::from_utf8(output.stdout).unwrap();
    let lib = PathBuf::from(sysroot.trim())
        .join("lib/rustlib")
        .join(target)
        .join("lib");
    assert!(
        lib.join("self-contained/libc.a").is_file(),
        "{} is missing: rust-toolchain.toml lists {target}, so `rustup target add {target}` installs it",
        lib.display()
    );
    lib
}

#[test]
fn the_toolchain_s_musl_inputs_and_standard_library_pass_the_scan() {
    for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        let lib = toolchain_lib(target);
        let mut musl = una_link::ElfScan::default();
        for file in [
            "libc.a",
            "rcrt1.o",
            "crti.o",
            "crtn.o",
            "crtbeginS.o",
            "crtendS.o",
            "libunwind.a",
        ] {
            let path = lib.join("self-contained").join(file);
            let bytes = std::fs::read(&path).unwrap();
            let scan = scan_elf(file, &bytes).unwrap_or_else(|p| panic!("{target} {file}: {p}"));
            musl.merge(scan);
        }
        let version = musl.musl_version.clone().expect("libc.a says its version");
        assert!(version.split('.').count() == 3, "{target}: musl {version}");
        assert!(musl.indexed > 1000, "{target}: {} entries", musl.indexed);
        assert!(
            !musl.counts.keys().any(|key| key.contains("IRELATIVE")),
            "{target}"
        );
        // The standard library: no common symbol, which the linker does not allocate for it;
        // libprofiler_builtins is linked only by coverage and profiling builds.
        for entry in std::fs::read_dir(&lib).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.ends_with(".rlib") || name.starts_with("libprofiler_builtins-") {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let scan = scan_elf(&name, &bytes).unwrap_or_else(|p| panic!("{target} {name}: {p}"));
            assert!(scan.common.is_empty(), "{target} {name}: {:?}", scan.common);
        }
    }
}
