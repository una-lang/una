//! The ELF side of the linker for the musl targets (ADR 0124 step 4,
//! `docs/plan/0124-step4-design.md`): so far the reader of relocatable objects ([`input`]) and
//! the relocation engine of rule 24 ([`reloc`], with the per-architecture fields in [`x86_64`]
//! and [`aarch64`]), slice 4b. The layout, the dynamic section and the writer of the static-pie
//! executable are slice 4c; nothing here is reached from [`crate::link`] yet, so `una build` on
//! Linux still links through `cc`.
//!
//! The engine works on one input section at a time, on that section's own bytes, with the
//! addresses a layout gives it through [`reloc::Layout`]: it cannot write outside the section,
//! and every value it computes is checked against its field (rule 24, "Bounds").

pub(crate) mod aarch64;
pub(crate) mod check;
pub(crate) mod input;
pub(crate) mod oracle;
pub(crate) mod reloc;
pub(crate) mod x86_64;

#[cfg(test)]
pub(crate) mod testobj;
#[cfg(test)]
mod tests;

/// The architectures of the musl targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Arch {
    /// `x86_64-unknown-linux-musl` (`EM_X86_64`).
    X86_64,
    /// `aarch64-unknown-linux-musl` (`EM_AARCH64`).
    Aarch64,
}

/// Section types (`sh_type`).
pub(crate) const SHT_PROGBITS: u32 = 1;
pub(crate) const SHT_SYMTAB: u32 = 2;
pub(crate) const SHT_RELA: u32 = 4;
pub(crate) const SHT_NOTE: u32 = 7;
pub(crate) const SHT_NOBITS: u32 = 8;
pub(crate) const SHT_REL: u32 = 9;
pub(crate) const SHT_INIT_ARRAY: u32 = 14;
pub(crate) const SHT_FINI_ARRAY: u32 = 15;
pub(crate) const SHT_PREINIT_ARRAY: u32 = 16;
pub(crate) const SHT_GROUP: u32 = 17;
pub(crate) const SHT_CREL: u32 = 0x4000_0014;
pub(crate) const SHT_X86_64_UNWIND: u32 = 0x7000_0001;

/// Section flags (`sh_flags`).
pub(crate) const SHF_WRITE: u64 = 0x1;
pub(crate) const SHF_ALLOC: u64 = 0x2;
pub(crate) const SHF_EXECINSTR: u64 = 0x4;
pub(crate) const SHF_TLS: u64 = 0x400;
pub(crate) const SHF_COMPRESSED: u64 = 0x800;

/// Symbol bindings and types (`st_info`).
pub(crate) const STB_LOCAL: u8 = 0;
pub(crate) const STB_GLOBAL: u8 = 1;
pub(crate) const STB_WEAK: u8 = 2;
pub(crate) const STT_SECTION: u8 = 3;
pub(crate) const STT_TLS: u8 = 6;
pub(crate) const STT_GNU_IFUNC: u8 = 10;

/// The largest alignment an input section may ask for: the segment alignment on arm64 (rule 25).
pub(crate) const MAX_ALIGN: u64 = 0x1_0000;

/// Whether the linker defines `name` (ADR 0124 rule 23, ELF): `__start_<section>` and
/// `__stop_<section>` for a section whose name is a C identifier, the bounds of the
/// initializer and finalizer arrays, `_DYNAMIC`, `_GLOBAL_OFFSET_TABLE_`, `__ehdr_start` and
/// `__dso_handle`. No input may define them; resolution (slice 4c) binds them to the layout.
pub(crate) fn linker_defined(name: &[u8]) -> bool {
    const FIXED: &[&[u8]] = &[
        b"__init_array_start",
        b"__init_array_end",
        b"__fini_array_start",
        b"__fini_array_end",
        b"__preinit_array_start",
        b"__preinit_array_end",
        b"_DYNAMIC",
        b"_GLOBAL_OFFSET_TABLE_",
        b"__ehdr_start",
        b"__dso_handle",
    ];
    if FIXED.contains(&name) {
        return true;
    }
    let section = name
        .strip_prefix(b"__start_")
        .or_else(|| name.strip_prefix(b"__stop_"));
    section.is_some_and(|section| {
        section
            .first()
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
            && section
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || *c == b'_')
    })
}
