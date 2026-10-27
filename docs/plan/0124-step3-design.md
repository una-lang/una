# ADR 0124 step 3: the internal linker, macOS arm64 (design note)

- Issue: #1208 (part of #891 and of #1104; ADR 0124 rules 12, 22 to 29, 31, 33, 34, Implementation plan step 3)
- Base: step 2 (#1207): the runtime embedded in `una` with its digest; step 1's build record and release checks.
- Scope: no construct of the language changes. On macOS arm64, `una build` stops running `cc`: `una-link` writes the executable itself. Linux keeps `cc` until step 4, and development builds keep an external link for the `sanitize` gate (rule 19).

This note fixes how step 3 is built, and lists the decisions it takes where the record leaves a choice. Each decision takes the most secure option the record allows, and every structure the linker writes is one the loader of macOS 13 reads; where a detail of the format is pinned from Apple's `dyld` and `ld64` sources rather than from a document, the note says so, and a conformance test checks it on the real system.

## 1. What step 3 delivers

1. The crate `una-link`, safe Rust (`unsafe_code = "forbid"`), no dependency on a crate of the repository but `una-diag` (for diagnostics) and, for reading, the `object` crate already in `Cargo.lock` (rule 22); its writer is its own.
2. `una build` on `aarch64-apple-darwin` links the program's object and the embedded runtime into a Mach-O executable for macOS 13 and later, bound to `/usr/lib/libSystem.B.dylib` alone, with chained fixups, `__unwind_info`, an ad-hoc signature with the hardened runtime, and `--debuggable` (rules 22 to 29).
3. The symbol index of the embedded runtime, computed at `una`'s build (rule 12, deferred from step 2), so that a member is found by the symbol it defines without reading the others.
4. The relocation scan (`scripts/scan-relocations`), the verifier that reads every output back (rule 34) with its canaries, the oracle tests against Apple's `ld` and the fuzzing of the readers and the whole link (rule 33), the external link of the `sanitize` gate (`dev-external-link`), the `macos-oldest` and `link-oracle` gates, U7015, and `spec/16`'s section "Linking" (rule 31, Diagnostics).

Not in step 3: ELF output (step 4), range-extension thunks (rule 24: no program comes within a factor of 50 of ±128 MiB), debug information (rule 26), `una build --release` (ADR 0106, withdrawn by #1099).

## 2. The crate and its modules

```text
crates/una-link/src/
  lib.rs        the entry: link(inputs, options) -> Result<Vec<u8>, Problem>, and verify(bytes, inputs)
  input.rs      inputs: an object's bytes, an archive's bytes with its symbol index; reading through `object`
  resolve.rs    symbol resolution: the fixed input order, member loading to a fixed point, duplicates, weak, COMDAT, the linker-defined symbols, the libSystem export list
  gc.rs         dead-code removal over sections and atoms, from the roots, with the FDE/LSDA/personality rules
  layout.rs     the output's segments, sections and addresses; the fixed order of inputs, sections and symbols
  reloc.rs      the arm64 Mach-O relocation types, their position-independence and bounds rules, their application
  fixups.rs     LC_DYLD_CHAINED_FIXUPS: the rebase and bind chains, the imports table
  unwind.rs     __unwind_info from __compact_unwind and the __eh_frame it keeps
  commands.rs   the load commands: segments, symtab, dysymtab, dylinker, uuid, build version, main, dylib, function starts, data in code, exports trie, code signature
  signature.rs  the ad-hoc signature: SuperBlob, CodeDirectory 0x20500, entitlements with --debuggable
  verify.rs     rule 34: the reader of the output and its checks
  problem.rs    the diagnostics (U8001, U7015) with type, input, section and symbol
```

**Decision D1: `object` reads, `una-link` writes.** `object` 0.40 (through `cranelift-object`) reads `ar`, ELF and Mach-O, including Mach-O relocations and `.subsections_via_symbols` atoms; nothing in the repository writes Mach-O, and no crate covers chained fixups and a signature, so the writer is new code (ADR 0124 Alternative E). Everything the linker writes is a byte buffer assembled in a fixed order; nothing is written to disk but the executable, by `una build`'s existing private placement (#1160).

**Decision D2: one pass per phase, fixed orders.** Resolution, dead-code removal, layout, relocation application, fixups, unwind, commands, signature, each a pass over vectors in input order, never a hash map's order (rule 29). Symbol tables are `BTreeMap`s. The whole link is O(N + S log S), N the loaded bytes and S the sections (Costs), and the scaling tests of rule 33 (1,000 to 8,000 synthetic functions) run on it.

## 3. Inputs, the symbol index and resolution

- **Inputs** (rule 22): the program's object from Cranelift (`object` writes it today, `crates/una-codegen`), and the embedded runtime archive (`libuna_rt.a`, step 2). Never a file a package or the user names.
- **The symbol index** (rule 12): at `una`'s build, `una-cli`'s build script (step 2) reads the archive's members through `object` and writes, beside the embedded bytes, a table `symbol -> member index` for every defined global symbol, sorted by symbol, as a compact binary (`u32` member, `u32` name offset, names concatenated), embedded with `include_bytes!`. `una-link` looks a symbol up in O(log n) and reads only the members it loads. The table is checked at `una`'s build against the archive (every member's definitions, no duplicate strong definition), so the index can never name a member that does not define the symbol.
- **Resolution** (rule 23): the program's object first, then the runtime's members as undefined symbols demand them, repeated to a fixed point; among members that define one symbol, the first in archive order wins; two strong definitions in loaded inputs are U8001 naming both inputs; weak yields to strong; COMDAT groups and Mach-O weak definitions keep the first.
- **Linker-defined symbols:** `__mh_execute_header` only, on Mach-O; an input that defines it is U8001.
- **libSystem's exports** (rule 12, Question 5). **Decision D3: a plain list, generated once from the SDK and pinned.** `scripts/libsystem-exports.sh` reads `libSystem.tbd` and the `.tbd` files it re-exports from the macOS 13 SDK (`MacOSX13.3.sdk`), with the SDK's path given and its `.tbd` digests pinned in `supply-chain/libsystem.lock` (the SDK version, each `.tbd`'s SHA-256 and the list's SHA-256), and writes `supply-chain/libsystem/exports-arm64.txt`: one symbol per line, sorted, with the `current_version` of libSystem and the SDK version at the top. `una-cli`'s build script embeds the list and its digest into `una`; the build record gains `libsystem_exports_sha256`. The `.tbd` format is a YAML text with `exports:`/`reexports:` sections per target; the script parses only the lines it needs (`- '_symbol'` under `symbols:`) and fails on anything it does not understand, so a format change is a loud failure, not a shorter list. A symbol that stays undefined after resolution and is in the list is bound to libSystem (ordinal 1, two-level namespace); one outside the list is U8001 naming the symbol, so the runtime cannot come to need another library or a newer macOS unnoticed. The list is regenerated only when the pinned SDK changes.
- **Dead code** (rule 23): roots are `_main`'s atom, every initializer (`__mod_init_func` / `__init_offsets` entries), and the exported symbols (none for an executable but the header); reachability follows relocations; an `__eh_frame` FDE is kept only when its function is, and its LSDA and personality are kept through the FDE; `__compact_unwind` entries likewise. This keeps `-dead_strip`'s effect: a program without `Http` carries no `rustls`.

## 4. Relocations (rule 24)

- **The set**, from the scan: `ARM64_RELOC_UNSIGNED` (8 bytes, with `ADDEND` through the pair form in `object`), `SUBTRACTOR` (paired with `UNSIGNED`), `BRANCH26`, `PAGE21`, `PAGEOFF12`, `GOT_LOAD_PAGE21`, `GOT_LOAD_PAGEOFF12`, `POINTER_TO_GOT`, `TLVP_LOAD_PAGE21`, `TLVP_LOAD_PAGEOFF12`, `ADDEND`. `scripts/scan-relocations` lists, for the repository's programs' objects, the runtime's members, the Rust 1.98.1 `aarch64-apple-darwin` standard library and `ring` 0.17's assembly, every (type, section kind, symbol kind) and fails on a type outside the set; it runs at every bump of the toolchain, Cranelift or a crate with assembly, and a new type is added with its oracle test or the bump waits.
- **Position independence:** an `UNSIGNED` 8-byte pointer only in `__DATA` or `__DATA_CONST` at an 8-byte-aligned place (it becomes a chained rebase or bind); a `SUBTRACTOR`/`UNSIGNED` pair is resolved at link time anywhere (it is a distance); an `UNSIGNED` in `__TEXT` is U8001. A `GOT_LOAD_*` against a local symbol is relaxed to `ADR_PREL_PG_HI21`/`ADD` as `ld64` does, so the GOT holds only imports and the symbols the inputs take the address of through the GOT.
- **Bounds:** every relocation's place, with its width, lies in its own section; every symbol's `value + size ≤ section size`; input sections never overlap in the output; a violation is U8001. Application computes the place from the section's output offset, never from an offset into the whole buffer.
- **Range:** a `BRANCH26` whose target is beyond ±128 MiB is U7015.
- **Ignored:** relocations in `__DWARF` and `__LD,__compact_unwind`'s debug parts, which the output does not contain.
- **Thread-local variables:** `__thread_vars` entries are bound to `__tlv_bootstrap` of libSystem, with `__thread_data` and `__thread_bss` following; `TLVP_LOAD_*` reaches the descriptor.

## 5. Layout (rule 25)

Segments, 16 KiB aligned, in this order:

1. `__PAGEZERO`, 4 GiB, no protection.
2. `__TEXT` (r-x): `__text`, `__stubs` (none: every import is bound at load, no lazy binding, so no stub table; a call to an import goes through the GOT with `GOT_LOAD_*`, as Rust's objects already do), `__const`, `__cstring`, `__unwind_info`, `__eh_frame`, plus the header and load commands at the start.
3. `__DATA_CONST` (rw-, `SG_READ_ONLY`): `__got`, `__const` of the inputs' `__DATA_CONST`, `__mod_init_func`; read-only after the fixups, which dyld applies before it protects the segment.
4. `__DATA` (rw-): `__data`, `__thread_vars`, `__thread_data`, `__thread_bss`, `__bss`, `__common`.
5. `__LINKEDIT` (r--): the chained fixups, the exports trie, function starts, data in code, the symbol table, the string table, the code signature, in this order, each 8-aligned, the signature 16-aligned.

Input sections of one output section are placed in input order, each at its own alignment; the layout records every input section's output offset. Atoms (`.subsections_via_symbols`) keep their order within their section.

## 6. Load commands (rule 25)

`LC_SEGMENT_64` × 5; `LC_DYLD_CHAINED_FIXUPS`; `LC_DYLD_EXPORTS_TRIE` (an empty trie: an executable exports nothing; the header symbol is not exported); `LC_SYMTAB` (the function symbols, rule 26, and the undefined imports, with their string table); `LC_DYSYMTAB` (the local, external and undefined ranges, no indirect table since there are no stubs); `LC_LOAD_DYLINKER` (`/usr/lib/dyld`); `LC_UUID` (rule 27's digest); `LC_BUILD_VERSION` (platform 1, minos 13.0.0, sdk from the export list's SDK version, one tool entry naming `una` with its version packed); `LC_MAIN` (the file offset of `_main`); `LC_LOAD_DYLIB` (`/usr/lib/libSystem.B.dylib`, timestamp 2, current version from the export list, compatibility version 1.0.0); `LC_FUNCTION_STARTS` (ULEB128 deltas of every function's offset, from the symbol table's functions); `LC_DATA_IN_CODE` (empty); `LC_CODE_SIGNATURE`. The flags: `MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL | MH_PIE`.

**Decision D4: no `LC_SOURCE_VERSION`, no `LC_VERSION_MIN_MACOSX`, no `LC_RPATH`:** nothing of the building machine, and no search path, is written.

## 7. Chained fixups (rule 25)

The format of `dyld`'s `mach-o/fixup-chains.h`, pointer format `DYLD_CHAINED_PTR_64` (format 2), page size 4 KiB:

- `dyld_chained_fixups_header` (version 0, offsets of the starts, imports and symbols, `imports_count`, `imports_format` 1 `DYLD_CHAINED_IMPORT`, `symbols_format` 0 uncompressed);
- `dyld_chained_starts_in_image` with one `dyld_chained_starts_in_segment` per segment that holds a fixup (`__DATA_CONST` and `__DATA`), each with `page_size` 0x1000, `pointer_format` 2, `segment_offset`, `max_valid_pointer` 0, and a `page_start` per page: the offset of the page's first fixup, or `DYLD_CHAINED_PTR_START_NONE` (0xFFFF);
- in the segment's bytes, each fixup is a 64-bit word: a **rebase** (`bind` bit 0: `target` 36 bits = the unslid address's low bits, `high8`, `next` 12 bits = the distance to the next fixup in 4-byte units, `bind` 0) or a **bind** (`bind` bit 1: `ordinal` 24 bits = the import's index, `addend` 8 bits, `next`); the chain of a page ends with `next` 0;
- the imports table: one `DYLD_CHAINED_IMPORT` (32 bits: `lib_ordinal` 8 = 1 for libSystem, `weak_import` 1 = 0, `name_offset` 23) per bound symbol, and the symbol names concatenated.

Every pointer the layout placed in `__DATA_CONST` or `__DATA` (an `UNSIGNED` against a defined symbol becomes a rebase; against an import, a bind; an addend beyond 8 bits for a bind is U8001, which the scan shows never occurs) goes into its page's chain in address order; a page with more than one fixup links them by `next`. The GOT entries are such pointers. A fixup's place must be 8-aligned and inside its page (rule 24). With every import bound at load (no `DYLD_CHAINED_PTR_64_OFFSET`, no lazy binding), there is no writable stub table (rule 25).

**Pinned from sources, checked on the system:** the bit layout above is `ld64`'s and `dyld`'s; the conformance test runs an executable with a rebase and a bind in each data segment under macOS 13 (the `macos-oldest` gate) and on the gate machine, and `llvm-readobj --macho-dyld-info` (rule 33) reads the chains back.

## 8. Unwind information (rule 25)

The runtime unwinds (`catch_unwind` at its boundary), so an executable needs its unwind tables. `__unwind_info`'s format is `mach-o/compact_unwind_encoding.h`:

- from the inputs' `__LD,__compact_unwind` entries (one per function: function address, length, encoding, personality, LSDA), kept only for live functions; an entry whose encoding needs DWARF (`UNWIND_ARM64_MODE_DWARF`) points at its `__eh_frame` FDE, which is kept with its CIE, LSDA and personality;
- the output: `unwind_info_section_header` (version 1), the common encodings array, the personality array (GOT addresses of the personalities), the index of first-level entries (one per page of up to 4 KiB of functions, `UNWIND_SECOND_LEVEL_COMPRESSED` where the encodings fit, `REGULAR` otherwise), the LSDA index, and the second-level pages, in function-address order;
- `__eh_frame` keeps the FDEs that `__unwind_info` refers to, with their relocations applied (the `SUBTRACTOR` pairs of `__eh_frame` are resolved at link time, rule 24).

The oracle test links the runtime's panic path with `ld` and with `una-link` and runs it: a panic inside the runtime is caught at its boundary and gives U8001 as today (Conformance: "a panic inside the runtime is caught at its boundary").

## 9. The signature and `--debuggable` (rule 27)

The last part of `__LINKEDIT`, 16-aligned, named by `LC_CODE_SIGNATURE`:

- a SuperBlob (magic `0xfade0cc0`, length, count) with one `CodeDirectory` (slot 0) and, with `--debuggable`, an entitlements blob (slot 5, magic `0xfade7171`, the plist `com.apple.security.get-task-allow` = true);
- the CodeDirectory (magic `0xfade0c02`, version `0x20500`): flags `CS_ADHOC` (0x2) | `CS_LINKER_SIGNED` (0x20000) | `CS_RUNTIME` (0x10000); `hashType` 2 (SHA-256), `hashSize` 32, `pageSize` 12 (4 KiB); `identOffset` naming the output file's name (no directory); `nSpecialSlots` 0, or 5 with the entitlements (slot -5 holding the SHA-256 of the entitlements blob, the others zero); `nCodeSlots` = ⌈codeLimit / 4096⌉; `codeLimit` = the offset of the signature; `execSegBase` 0, `execSegLimit` = `__TEXT`'s file size, `execSegFlags` `CS_EXECSEG_MAIN_BINARY` (0x1) | `CS_EXECSEG_ALLOW_UNSIGNED` never; `runtime` = the packed version 13.0.0; no `teamID`, no requirements slot;
- the page hashes: SHA-256 of each 4 KiB page of the file from offset 0 to `codeLimit`, the last page shorter; computed in parallel over pages (rayon, allowed), each written at its fixed slot (rule 29);
- `LC_UUID`: the first 16 bytes of the SHA-256 of the whole file with the UUID and the signature bytes zeroed, with the version-4 bits set.

**Decision D5: the layout of rule 27 exactly, and the fallback it names.** Probe P3 showed `linker-signed` with `runtime` enforced, with a requirements slot present; this layout has none. The conformance test is `codesign --verify --strict` on every output, `DYLD_INSERT_LIBRARIES` refused on an executable without `--debuggable`, `lldb` attaching only with it; if macOS 13 or the gate machine refuses the layout without the slot, the signature gains an empty requirements slot (magic `0xfade0c01`, count 0) in the same record, never dropping the hardened runtime. The user may replace the signature (`codesign --force --sign`); notarization is theirs.

## 10. Symbols and determinism (rules 26 and 29)

- `LC_SYMTAB` holds the defined functions (`N_SECT`, external or private as the input had them) and the undefined imports (`N_UNDF | N_EXT`, library ordinal 1); no debug symbols (`N_STAB`), no object file names, no paths. Data symbols are kept too, so `nm` names what the file holds; nothing a profiler or a crash report needs is lost.
- The executable is a function of the inputs' bytes, the `una` version, the output file's name and `--debuggable`: the two-builds test of rule 29 runs from two directories as two users, and the two-names test differs only in the identifier, the page hashes of the page that holds it, and the UUID.
- `una.build/1` keeps its keys; the report's `native` section gains `linker` (`"una <version>"`) and `debuggable` (rule 28); `runtime_sha256` came with step 2.

## 11. The verifier (rule 34)

`verify(bytes, inputs)` reads the output back with `object` and `una-link`'s own readers of the chains and the signature, and refuses it (U8001) unless: no segment is both writable and executable; `__DATA_CONST` is `SG_READ_ONLY`; the stack is not executable (no `MH_ALLOW_STACK_EXECUTION`); every relocation was applied inside its own section (the layout's record of places); every kept input section is placed once, without overlap; every chained fixup's place is 8-aligned, inside its page, and its target a defined symbol's address or an import of the list; every import is in the libSystem list; the signature's page hashes match the file; the `codeLimit` is the signature's offset; the UUID is the file's digest. The canaries, each a test input that must make the verifier or the gate fail: a writable and executable section; an `R_X86_64_IRELATIVE` (on the ELF reader, step 4) and, on Mach-O, a relocation of an unsupported type; an undefined symbol outside the list; a signature with one wrong page hash; an executable with the stack flag; a relocation whose place lies 8 bytes past its section; a symbol past its section's end; two input sections placed overlapping. The verifier runs on every `una build` (its cost is one read of the output, rule 34), and the fuzzing of rule 33 uses it as its oracle.

## 12. Tests and gates (rules 31 and 33)

- **Per relocation type:** `crates/una-link/tests/fixtures/*.s`, one assembly file per type, assembled on the gate machine by `clang -c` (an assembler is a tool of the test, not of `una`), linked by `ld` and by `una-link`; the test decodes the instruction or datum at each place and maps the address it reaches back to a symbol and an offset; both outputs must agree at every place. The must-reject cases (`UNSIGNED` in `__TEXT,__const`, an unaligned one) refused by `una-link`.
- **Per program:** every runnable program of the repository, linked by both, runs with the same output, failure and trace; the `native` gate's equivalence holds on `una-link`'s output.
- **Generated programs:** the hunt's generators' programs, linked by both and run.
- **Structure:** `llvm-readobj --macho-segment --macho-dyld-info --macho-unwind-info` and `codesign --verify --strict` read every output; `vtool -show-build` shows minos 13.0.
- **Fuzzing** (`sanitize` gate): the `ar`, ELF and Mach-O readers and the whole link, on mutated objects and archives, with the verifier as the oracle: an input either fails with a diagnostic or yields an executable the verifier accepts.
- **Scaling:** synthetic programs of 1,000 to 8,000 functions, medians of 9 runs, each doubling at most ×2.3; the same on relocations per function.
- **Gates:** `link-oracle` (the above, on each target); `macos-oldest` (the `native` gate's corpus built on the gate machine, run on a macOS 13 virtual machine with the interpreter's outputs; the virtual machine is a resource of the gate machine, named in the record); `toolchain-free` gains the build phase (`una build` of `examples/hello` and of a program with an `Http` request over TLS to a local test server, under the step 1 sandbox) and the run phase.
- **The external link for `sanitize`** (rule 19): the Cargo feature `dev-external-link` of `una-cli`, absent from release builds and refused by `scripts/check-release.sh` by a marker as the test hooks are, enables `una build --external-linker <cc>`, which links through that program as `build.rs` does today, so the gate can link the AddressSanitizer runtime (a dynamic library on macOS); a release `una` has no such flag (usage error).

## 13. Records amended

- `spec/16`: the new section "Linking" (the inputs, the relocation lists per target, what an executable contains and links, U7015), `§1` rules 1 and 2 (the internal linker; the executables' targets), `§2` rule 3 (a failure to link is U8001 on Una's own inputs, a usage error only for the file), `§3` (unchanged for macOS; RELRO is step 4).
- `spec/13 §4` (U7015), `§5` (`--debuggable`, `--external-linker` in development builds), `§6` (`native.linker`, `native.debuggable`).
- ADR 0055's amendment (NB-1: the internal linker), appended dated.
- `docs/compiler/architecture.md §7` (`una-link`), `docs/security/tcb.md` §3.5 (the linker replaces the unpinned `cc`; gap 7 closes on macOS), `docs/release-gates.md` §2.

## 14. Slices (the issue's, refined)

- [ ] 3a. **Readers, the symbol index, resolution, dead code.** `una-link` with `input.rs`, `resolve.rs`, `gc.rs`; the index written by `una-cli`'s build script and embedded; `scripts/libsystem-exports.sh`, `supply-chain/libsystem.lock` and the pinned list; tests over the repository's real objects and the runtime (what each link loads, every undefined symbol in the list); no output written.
- [ ] 3b. **Relocations and layout.** `reloc.rs`, `layout.rs`; the arm64 set with the position-independence and bounds rules; `scripts/scan-relocations` and the lists in `spec/16` "Linking"; U7015; unit tests per rule with hand-made sections.
- [ ] 3c. **Load commands, fixups and unwind.** `commands.rs`, `fixups.rs`, `unwind.rs`; a first executable of `examples/hello` that dyld runs (no signature yet: an unsigned arm64 executable is refused by the kernel, so this slice's test runs `codesign -s - ` on the output as a temporary measure, marked as such).
- [ ] 3d. **The signature and `--debuggable`.** `signature.rs`, `LC_UUID`; `codesign --verify --strict`; `DYLD_INSERT_LIBRARIES` refused; `lldb` attaches only with the flag; the report's `native.linker` and `native.debuggable`; `una build` switches to `una-link` on macOS, `cc` stays for Linux.
- [ ] 3e. **The verifier and its canaries.** `verify.rs`, run on every build; the eight canaries.
- [ ] 3f. **Oracle, fuzzing, external link.** The fixtures per type and the oracle tests, the per-program and generated-program comparisons, the fuzz targets in `fuzz/` with the verifier as oracle, the scaling tests, `dev-external-link` and `--external-linker` for the `sanitize` gate with its marker in `check-release.sh`.
- [ ] 3g. **Gates, spec and docs.** `link-oracle`, `macos-oldest`, the `toolchain-free` build and run phases on macOS; `spec/16` "Linking" and the amended sections; `spec/13`; ADR 0055's amendment; `architecture.md`, `tcb.md`, `release-gates.md`; changelog fragment.
