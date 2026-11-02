# ADR 0124 probe P2: RELRO and DT_RELR under the toolchain's musl (results)

- Issue: #1204 (part of #891; ADR 0124 rules 25 and 36, Implementation plan step 4; `docs/plan/0124-step4-design.md` §3 and §6)
- Measured: 2026-11-02, read-only for the repository. The probe program, its build script and every build product stayed outside the tree; this note records the results only.
- Question: does the musl of Rust 1.98.1's self-contained targets protect `PT_GNU_RELRO` in a static-pie executable, or only apply the relative relocations? Does its start-up apply `DT_RELR`? Does a later `mprotect` of the range from `main` succeed?

## Answer

| Item | aarch64 (native) | x86-64 (Rosetta) |
|---|---|---|
| 1. Static-pie executable with the pinned `rcrt1.o`, `crti.o`, `crtn.o`, `libc.a` | linked and runs, relocations applied | linked and runs, relocations applied |
| 2. RELRO pages (`.got`, `.data.rel.ro`, `.init_array`, `.fini_array`, `.dynamic`) after start-up | **`rw-p`, writable**: a write to them succeeds | **`rw-p`, writable**: a write to them succeeds |
| 3. Start-up applies `DT_RELA` | yes | yes |
| 3. Start-up applies `DT_RELR` | **yes** (an executable with only `DT_RELR`, no `DT_RELA`, starts with every pointer correct) | **yes** (same) |
| 4. `mprotect(PROT_READ)` of the RELRO range from `main` | **succeeds** (0); pages become `r--p`; a write then faults with `SIGSEGV`; the adjacent `.data` stays writable | **succeeds** (0); same outcome |

musl 1.2.5's static-pie start-up only applies the relative relocations: it does not protect `PT_GNU_RELRO`. The protection rule 25 prescribes works.

## Decision it feeds

1. **`ffi.rs` protects the range; it is not only a check.** On both architectures musl leaves the RELRO pages writable, so the runtime's start-up call (design note §6, decision D3) is the protection itself: `mprotect(PROT_READ)` of `PT_GNU_RELRO`'s range as the first thing `main` does, U8001 if it fails. The claim "full RELRO" still needs the run-time conformance test that reads `/proc/self/maps` (rule 25).
2. **`DT_RELR` may be written.** The embedded musl applies it on both architectures (measured, item 3), which is the condition rule 25 sets. Rule 25 asks that a gate show it: if step 4 writes `DT_RELR`, the release gates carry a run-time test that a `DT_RELR`-only executable built by `una` starts with its relocations applied, rerun whenever `supply-chain/musl.lock` changes. Until that test exists, `DT_RELA` stays the format (design note §3). The format is fixed at `una`'s build by the pinned musl, never chosen at run time.
3. **Notes for the implementation of §6:**
   - The load base comes from `AT_PHDR` minus `PT_PHDR`'s `p_vaddr`. GNU ld's static-pie output (2.42) has no `PT_PHDR`; LLD's has one. `una-link` writes `PT_PHDR` (rule 25), so `ffi.rs` can rely on it and fail closed (U8001) when it is missing rather than guess.
   - Rounding: the range's start is rounded down to the page size, its end rounded down too (as glibc does), so that a page shared with writable data is never made read-only. With rule 25's padding of the end to the segment alignment the two roundings agree; `ffi.rs` should still round the end down, never up. The page size comes from the kernel at run time (`AT_PAGESZ` or `sysconf`): only 4 KiB kernels were measured here, not 16 or 64 KiB ones.
   - Before `main`, musl's start-up path (`rcrt1.o`, `__libc_start_main`, `__init_tls`) references no `mprotect`; in `libc.a` only `malloc` and `pthread_create` do.

## Environment

| | aarch64 | x86-64 |
|---|---|---|
| Rust toolchain | 1.98.1 (48a229cea 2026-09-01), rust-std of `aarch64-unknown-linux-musl` | 1.98.1, rust-std of `x86_64-unknown-linux-musl` |
| musl | 1.2.5 (`__libc_version` in `libc.a`) | 1.2.5 |
| Input digests | `libc.a`, `rcrt1.o`, `crti.o`, `crtn.o`, `crtbeginS.o`, `crtendS.o` equal to `supply-chain/musl.lock` | equal to `supply-chain/musl.lock` |
| Kernel | Linux 7.0.14-linuxkit (Docker Desktop 29.8.2 VM, Apple silicon host) | same kernel; the x86-64 code runs under Rosetta for Linux (`/run/rosetta/rosetta` in the maps, CPU "VirtualApple") |
| Page size | 4096 (`AT_PAGESZ`, `getconf PAGESIZE`) | 4096 |
| Container | `una-local-ci:1.98.1-arm64` (`linux/arm64`) | `una-local-ci:1.98.1` (`linux/amd64`) |
| Linkers | GNU ld 2.42; LLD 22.1.8 (`rust-lld` of the container's 1.98.1 toolchain) | same |
| Compiler of the probe | gcc 13.3.0, `-fPIE -nostdinc -ffreestanding`, no libc headers | same, plus `-Wa,-mrelax-relocations=no` |

## Method

The probe is a small measurement program, a host tool outside the tree (not part of Una or its toolchain), compiled inside the container and linked statically with no dynamic linker: `rcrt1.o crti.o crtbeginS.o probe.o libc.a libgcc.a crtendS.o crtn.o`, `-static -pie --no-dynamic-linker -z relro -z now -z noexecstack`. Three executables per architecture:

- `bfd-rela`: GNU ld, relative relocations as `DT_RELA` (33 `R_AARCH64_RELATIVE`, 30 `R_X86_64_RELATIVE`);
- `lld-rela`: LLD, `DT_RELA` (same counts);
- `lld-relr`: LLD with `-z pack-relative-relocs`, which writes only `DT_RELR` (5 words, 40 bytes) and no `DT_RELA`. GNU ld 2.42 ignores the option on aarch64, so `DT_RELR` was measured with LLD on both.

Each `PT_GNU_RELRO` covers `.init_array`, `.fini_array`, `.data.rel.ro`, `.dynamic` and `.got` (and `.got.plt` with LLD on x86-64), checked with `readelf`. At run time, `main`:

1. finds the program headers through `getauxval(AT_PHDR)` and `AT_PHNUM`, and the `PT_GNU_RELRO` range;
2. checks that the relocations were applied: a constant table of function pointers in `.data.rel.ro`, a table of string pointers and a writable table in `.data` hold the addresses the code computes PC-relatively, and calling through them works;
3. reads `/proc/self/maps` and reports the mapping and permissions of the first and last byte of the RELRO range and of the table, and in a forked child writes a word back to the table's page;
4. calls `mprotect(PROT_READ)` on the range (start rounded down, end rounded down to the page), rereads the maps, and in forked children writes to the table, to the last word of the range (the `.got` page) and to `.data`.

Results were identical for the three executables of each architecture. aarch64, `lld-relr`: the range `0x22d98`..`0x23000` (load base added) mapped `rw-p` after start-up; `mprotect` returned 0, the page became `r--p`, writes to it faulted with signal 11, the write to `.data` succeeded. x86-64, `lld-relr`: the range `0x45c8`..`0x5000`, same outcome. With GNU ld the RELRO page and the first `.data` page share one mapping at start-up; `mprotect` split it and left `.data` writable.

The archive agrees with the measurement: `rcrt1.o` imports only `__libc_start_main` and `main` (and `_GLOBAL_OFFSET_TABLE_` on x86-64), and on aarch64 its code holds no system-call instruction, so it cannot change a page's protection; it relocates and calls `__libc_start_main`. This matches musl's design as documented upstream (the static-pie start-up, `ldso/dlstart.c`, relocates; RELRO is protected by the dynamic linker, `ldso/dynlink.c`, which a static executable does not run; `DT_RELR` support arrived in musl 1.2.4), which this probe did not re-read: the run-time measurement is the evidence.

A corroborating run on aarch64: a Rust program built by the container's rustc 1.98.1 for `aarch64-unknown-linux-musl` with its default settings came out as a static non-PIE executable (`ET_EXEC`), with its `PT_GNU_RELRO` pages also mapped `rw-p` at run time.

## Limits

- x86-64 was measured under Rosetta, not on an x86-64 kernel. What musl's start-up does (no `mprotect`) does not depend on the CPU, and Rosetta enforced the protection once requested; a native x86-64 Linux leg (a release-candidate Linux leg) would confirm the run-time behavior on real hardware. Under Rosetta the executables loaded at the same base on every run (`0x7ffffffea000`), so address randomization was not exercised there; that is Rosetta's, not musl's.
- Only 4 KiB page kernels were measured; arm64 kernels with 16 or 64 KiB pages were not available.
- The probe was linked with GNU ld and LLD, not with `una-link`, which does not write ELF executables yet. The conformance test of rule 25 repeats the maps check on `una`'s own output.
- The Rust corroboration was not run on x86-64: the `linux/amd64` local-ci image predates the musl targets' installation (the Dockerfile change of #1209 slice 4a) and has no `x86_64-unknown-linux-musl` standard library.
