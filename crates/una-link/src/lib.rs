//! `una-link`, the internal linker of `una build` (ADR 0124 rules 22 to 29 and 34,
//! `docs/plan/0124-step3-design.md`).
//!
//! It links the program's object from Cranelift and the runtime archive built into `una` into an
//! executable, in safe Rust, with no other program run. The target is `aarch64-apple-darwin`: a
//! Mach-O executable for macOS 13 and later that links `/usr/lib/libSystem.B.dylib` alone, with
//! chained fixups, `__unwind_info`, and an ad-hoc signature with the hardened runtime.
//!
//! For Linux (ADR 0124 step 4), it so far scans the ELF inputs that `una` carries for its musl
//! target ([`scan_elf`]), and reads ELF objects and applies their relocations by rule 24 (the
//! engine of step 4b, which [`check_elf`] runs over real inputs and [`elf_oracle`] compares
//! with a system linker); it does not link them yet.
//!
//! Inputs are read through the `object` crate; everything written is this crate's own. The
//! output is a function of the inputs' bytes and the [`Options`]: no path, user name, time or
//! random value is written (rules 26 and 29).

mod bytes;
mod commands;
mod elf;
mod elf_scan;
mod fixups;
mod gc;
mod input;
mod layout;
mod libsystem;
mod problem;
mod reloc;
mod resolve;
mod sections;
mod signature;
mod unwind;
mod verify;

pub use elf::check::{ElfCheck, check_elf};
pub use elf::oracle::{ElfOracle, elf_oracle};
pub use elf_scan::{ElfScan, scan_elf};
pub use problem::{Code, Problem, Result};

use std::collections::BTreeMap;

/// What a link takes besides its inputs.
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    /// The output file's name, without its directory: the signature's identifier (rule 27).
    pub identifier: &'a str,
    /// `--debuggable`: the signature's entitlements let a debugger attach (Question 7).
    pub debuggable: bool,
}

/// Links the program's object `program` and the runtime archive `runtime` into a Mach-O
/// executable for macOS 13 and later on arm64, and returns its bytes.
///
/// The bytes are a function of the inputs and `options` alone (rule 29).
///
/// # Errors
///
/// U8001 on an input the linker cannot read or refuses (rules 23 and 24); U7015 when the
/// executable cannot be laid out.
pub fn link(program: &[u8], runtime: &[u8], options: &Options<'_>) -> Result<Vec<u8>> {
    let program = input::read_object("the program's object", program)?;
    let archive = input::Archive::read("libuna_rt.a", runtime)?;
    let resolved = resolve::resolve(program, &archive)?;
    let graph = gc::build(resolved, ENTRY)?;
    let exports = libsystem::exports();
    let image = layout::lay_out(&graph, ENTRY, &|name| exports.contains(name))?;
    let signed = signature::Signed {
        identifier: options.identifier,
        debuggable: options.debuggable,
        text: (0, 0),
    };
    let target = commands::Target13 {
        libsystem_version: libsystem::current_version(),
    };
    let bytes = commands::write(&image, &target, &signed)?;
    verify::verify(&bytes, options.debuggable, &|name| exports.contains(name))?;
    Ok(bytes)
}

/// Reads an executable back and refuses it unless it holds the hardening of ADR 0124 rule 34:
/// what [`link`] runs on every output, for the tests and the canaries of the gate. `debuggable`
/// says whether the executable was built with `--debuggable`: without it, an entitlements blob
/// (a `get-task-allow`) is refused.
///
/// # Errors
///
/// U8001 naming the first check that fails.
pub fn verify(executable: &[u8], debuggable: bool) -> Result<()> {
    let exports = libsystem::exports();
    verify::verify(executable, debuggable, &|name| exports.contains(name))
}

/// The relocation types the ELF linker applies, by musl target (ADR 0124 rule 24, the lists of
/// `spec/16` "Linking"), in the ABI's numbering order.
#[must_use]
pub fn elf_relocation_types() -> BTreeMap<&'static str, Vec<&'static str>> {
    [
        ("x86_64-unknown-linux-musl", elf::Arch::X86_64),
        ("aarch64-unknown-linux-musl", elf::Arch::Aarch64),
    ]
    .into_iter()
    .map(|(target, arch)| {
        (
            target,
            elf::reloc::types(arch).iter().map(|t| t.name).collect(),
        )
    })
    .collect()
}

/// The entry point: the `main` that the program's object defines (`spec/16 §3`).
const ENTRY: &[u8] = b"_main";

/// Counts every relocation of `bytes`, an object or an archive of objects, by type, section and
/// kind of referent: the scan of rule 24 (`scripts/scan-relocations`).
///
/// # Errors
///
/// U8001 when an input cannot be read, or holds a relocation type outside the supported set.
pub fn scan_relocations(name: &str, bytes: &[u8]) -> Result<BTreeMap<String, u64>> {
    let mut counts = BTreeMap::new();
    let objects: Vec<(String, &[u8])> = if bytes.starts_with(b"!<arch>\n") {
        let archive = input::Archive::read(name, bytes)?;
        archive
            .members()?
            .into_iter()
            .filter(|(_, member, _)| {
                !member.ends_with("__.SYMDEF)") && !member.ends_with("__.SYMDEF SORTED)")
            })
            .map(|(_, member, data)| (member, data))
            .collect()
    } else {
        vec![(name.to_owned(), bytes)]
    };
    for (member, data) in objects {
        let object = input::read_object(&member, data)?;
        for section in &object.sections {
            if section.segment == "__LD" && section.name == "__compact_unwind" {
                for entry in section.data.chunks(32) {
                    let encoding = entry
                        .get(12..16)
                        .and_then(|b| <[u8; 4]>::try_from(b).ok())
                        .map_or(0, u32::from_le_bytes);
                    let key = format!("compact unwind mode {:#x}", encoding & 0x0f00_0000);
                    *counts.entry(key).or_insert(0) += 1;
                }
            }
            *counts
                .entry(format!(
                    "section {},{} type {:#x}",
                    section.segment, section.name, section.flags
                ))
                .or_insert(0) += 1;
            for reloc in &section.relocs {
                let referent = match reloc.referent {
                    input::Referent::Section(_) => "section".to_owned(),
                    input::Referent::Symbol(index) => {
                        let symbol = object
                            .symbols
                            .get(usize::try_from(index).unwrap_or(usize::MAX));
                        match symbol {
                            Some(s) if s.undefined() => "undefined".to_owned(),
                            Some(s) if s.external() => "external".to_owned(),
                            Some(_) => "local".to_owned(),
                            None => "none".to_owned(),
                        }
                    }
                };
                let key = format!(
                    "{} {},{} {}{}{} width={} pcrel={}",
                    input::reloc_name(reloc.kind),
                    section.segment,
                    section.name,
                    referent,
                    if reloc.minus.is_some() { " (pair)" } else { "" },
                    if reloc.addend != 0 { " (addend)" } else { "" },
                    reloc.width,
                    reloc.pcrel
                );
                *counts.entry(key).or_insert(0) += 1;
            }
        }
    }
    Ok(counts)
}
