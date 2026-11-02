//! The `elf_link` fuzz target (ADR 0124 rules 24 and 33, step 4b): the internal linker's ELF
//! reader and relocation engine on any bytes, as an object or an `ar` archive of objects. Every
//! relocation of every kept section is planned and applied with a synthetic layout
//! (`una_link::check_elf`). An input fails with a diagnostic or passes; it never panics, hangs or
//! reads out of bounds, and it gives the same result twice.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let first = una_link::check_elf("fuzz.o", bytes);
    assert_eq!(first, una_link::check_elf("fuzz.o", bytes));
});
