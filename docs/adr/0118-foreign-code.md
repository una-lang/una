# 0118. Foreign code: foreign capabilities over the C ABI, gated to `systems`

- Status: Proposed
- Date: 2026-10-18
- Deciders:
- Resolves: issue #23 (the design of foreign code; issue #33 implements it)
- Related:
  - ADR 0109 (profiles), whose rule 5 gates foreign code to `systems` packages and to `[trusted]` modules, and states what this record MUST decide;
  - ADR 0058 (`unsafe` at the runtime's C ABI boundary), and its amendment by ADRs 0116 and 0117, which creates the crate `una-sys`;
  - ADR 0112 (eliding the checks the verifier proves), whose rule that a wrong proof costs a wrong value, never memory, this record keeps at the boundary;
  - ADR 0114 (`F64`), whose floating-point environment a foreign call must leave as it found it;
  - ADR 0116 (long-running programs) and ADR 0117 (concurrency for servers), whose workers, stacks and signal handlers a foreign call meets;
  - ADR 0093 (authority at module boundaries) and ADR 0092 (conditions and locks on `uses`);
  - `docs/security/tcb.md`, the risk profile of the trusted base, which names foreign code as the largest addition to come (§5);
  - issue #65 (a TCB risk profile beyond line counts) and issue #769 (the report's `tcb` section is incomplete).
- Target: the first edition that admits a construct that widens what a program may do (Question 5), in packages whose profile is `systems` (ADR 0109). Editions `0.1` to `0.3`, and every `app` package, do not change.
- Amends:
  - **Lexical and grammar:** `spec/01 §5` (contextual words), `spec/02 §4` (the foreign capability);
  - **Capabilities:** `spec/05 §2`, `§7` (foreign capabilities enter as host capabilities do), `§9` rule 5 (examples);
  - **Information flow:** `spec/07 §5` and `§6` (policies and trusted modules), `§7` (the labels of foreign operations);
  - **Core:** `spec/09` (a `foreign` field on a capability declaration);
  - **Diagnostics and project files:** `spec/13 §4` (codes), `§5` (the grant spec `foreign:<library>`, `una test --native`), `§6` (report), `§7` (the `[foreign]` table), `§8` (Core export);
  - **Native backend:** `spec/16 §2`, `§3` rule 3 (the stack), `§4` rule 8, `§5`;
  - **Charter:** `docs/00-charter.md §3`, the Trusted zone's row;
  - **Accepted records:** ADRs 0058, 0093, 0114 and 0116, by the dated amendments of Appendix A.

  The rules would live in a new chapter of the specification, `spec/NN-foreign-code.md`, numbered when it lands.

> **What is trusted, in plain words.**
> - **What a package can now do.** A `systems` package can call fast C code that its own authors built, for pure computations on numbers and byte buffers, such as compression or checksums. Every function that can reach that code says so in its header, and every run must grant it.
> - **What Una checks.** The C code's inputs and outputs, its labels, its declared preconditions and postconditions at every call, and, in the archive, the absence of the most common ways C code reaches outside: imports, writable globals, start-up code and linker tricks. The C code runs on its own stack and buffers, apart from Una's memory, with every register it does not need cleared. Every package also has a Una model of the same computation, which the interpreter and the tests run and which is compared with the C code.
> - **What Una cannot check.** **A foreign archive is trusted as the runtime is.** The audit and the boundary checks detect common defects of honest code. They do not confine code written to evade them. Only a sandbox would (Question 1).
> - **Building runs the C code.** Building a package with foreign code runs that code once, in a test harness that compares it with its model. The harness runs confined by the operating system, with no network, no file access outside a temporary directory, and a time limit, and the build log shows where a comparison failed, never the data (rule 33).

## Questions for the decision

Each question states, in plain words, what is being decided, with the recommendation this draft follows and an example.

1. **Should C code run inside the Una process?** Recommended: **yes, for the first version, inside a compartment** (rules 25 to 30): its own stack and its own buffers, with guard pages, apart from Una's memory, with every register it does not need cleared before each call and the Una state saved where it cannot reach. A bug in honest C code then lands in memory that only that C code uses. A malicious archive is still fully trusted. The alternative, a WebAssembly sandbox (Alternative E), would confine even a malicious archive, at the cost of a large engine in the trusted base and slower calls.
   - *Example.* A vendored compressor that, by a bug, writes one byte past its output buffer hits a canary, and the run stops with U6008. The same compressor written on purpose to read the process's memory, or to call `open` through an address it computes, is not stopped.
   - **This is an explicit exception** to the project's rule to choose the most secure option, taken because the purpose of `systems` code is native performance, and ADR 0109 rule 5.1 already admits foreign code to the trusted base for that purpose.
   - **The sandbox is evaluated before v1.0, as a blocker,** on defined workloads: a compressor and its decompressor, a checksum (CRC-32C), a cryptographic hash (SHA-256), and a microbenchmark of small calls, each at 64 B, 64 KiB and 16 MiB of input. Each runs compiled to WebAssembly in a pinned engine and natively in the compartment, measuring time per call, peak memory and start-up time. Beside the timings, the evaluation reports the engine's size in the trusted base: lines of `unsafe` code and lines of its code generator.
   - **The proposed threshold:** if the sandbox's slowdown is at most 1.5 times in geometric mean and at most 2 times on every workload and size, it becomes the default boundary, by a later record; otherwise that record states why the exception stands. These thresholds are the project's choice to make, and they are looser than the targets the project sets for native Una code against Rust (at most 1.1 times in geometric mean, no program above 1.5 times): a sandbox would be accepted while slower than Una code itself is allowed to be, in exchange for confinement.
   - **Until archives are built from source by Una** (Alternative K), `una.toml` names the directory that holds each archive's source and recipe (rule 7), so a reader can rebuild the archive and compare. The toolchain does not check that the archive came from it.
   - **If the sandbox later becomes the default,** existing packages keep their `una.toml`, their models and their C source with its recipe, and are rebuilt for the sandbox from that source; the later record says whether the in-process boundary stays available behind an explicit choice or is retired.
2. **Must every foreign capability come with a Una model, and be compared with it when the package is built?** Recommended: **yes**. The model **must** give the same results as the C code (rule 31). `una run` and the tests run the model. `una test --native` compares the model with the C code, and `una build` runs that comparison first and refuses to write an executable until it passes (rule 33).
   - **This means building runs the C code.** Building a package with foreign code runs that code once, in a test harness, on the arguments the package's tests used. Until now, building never ran a package's code. The harness runs as a separate process confined by the operating system: no network, no file access outside a temporary directory, no new processes, no inherited files, and a time limit. When a comparison fails, the build log names the operation, the index of the call and the first byte offset where the results differ, never the bytes themselves, so a hostile archive cannot use the log to carry data out. For pure code this confinement costs nothing, and it covers building an untrusted package, such as a proposed change in continuous integration.
   - **The skip.** `una build --skip-model-check` builds without running the comparison, for code one does not want to run at all. The report then says `"model_compared": null`, "model not compared", and a release build (`--release`) refuses the flag.
   - *Example.* A compressor whose Una model writes a different header byte than its C code fails `una test --native` on the first compressed test input, and `una build` prints `compress`, the index of that call and the offset of the first differing byte.
3. **May C code allocate memory?** Recommended: **no** in the first version. C code that needs working memory takes a scratch buffer as an argument (rule 9). A later record may route `malloc` to Una's memory budget.
   - *Example.* A library that calls `malloc` for its internal tables is rejected by the audit (U7006). Its shim takes a `scratch: List[U8]` argument of the size the library documents, and passes that memory to the library instead.
4. **May a declassification policy use C code?** Recommended: **no**. A policy's `via` code skips label checks, and C code skips every check, so the release would rest on nothing verified. The ban is checked at each policy call, following everything the call can run (rule 19).
   - *Example.* A policy that lowers a phone number from `secret` to `internal` by hashing it with a C implementation of HMAC is rejected (U3031), also when the C code is reached through a handler or a lambda the policy call is given. The same policy written with Una's own `Keys.hmac` is accepted, as today.
5. **Which edition admits foreign code?** Today, an edition that is still a preview may only make programs stricter; foreign code lets programs do more, so the rule must change or foreign code must wait. Edition `0.3` is a preview under that rule (`spec/13 §7`), and ADR 0116 opens `0.4` under the same terms, so neither can hold foreign code as things stand. Two options:
   - **(a) Recommended:** amend `spec/13 §7`, through ADR 0056's amendment process, so that a preview edition MAY add a construct that widens what a program may do when the construct is a syntax error in every earlier edition, is enabled only by its own opt-in in `una.toml`, and is listed in the build report. A program that does not opt in is unchanged. **An opted-in construct carries no stability promise while its edition is a preview:** a later release may change or narrow it, and a package that opts in accepts that.
   - **(b)** Wait for the first edition that is not a preview.
   - *Example.* Under (a), a package would write `edition = "0.4"`, `profile = "systems"` and an opt-in such as `preview = ["foreign"]` in `[package]`; the key's exact name belongs to the amendment. Without the opt-in, `foreign capability` stays a syntax error.

   This record does not decide edition terms. It is written against "the first edition that admits it", and takes effect in that edition. ADR 0116's `for ever` widens a preview edition in the same way; that is raised on ADR 0116's own issue, and this record only cites the outcome.

## Glossary

- **Foreign code.** Machine code that no Una check covers, linked into a native executable from a static archive.
- **Foreign capability.** A capability whose operations are implemented by foreign code instead of a handler or the host (rule 4).
- **Foreign operation.** An operation of a foreign capability; each names one C symbol.
- **Archive.** A static library (`.a`) holding one relocatable object for one target, vendored in the package and pinned by its SHA-256 (rules 7 to 9).
- **Import allowlist.** The undefined symbols that an archive may reference (rule 9).
- **Compartment.** The stack and buffer arena where foreign code of one capability runs on one worker, mapped apart from the Una heap (rule 26).
- **Model.** A Una handler of a foreign capability, verified like any handler, that gives the same results as the foreign code (rule 31).
- **Boundary checks.** What the runtime checks before and after each foreign call (rule 28). They are run-time checks, never obligations, and never elided.
- **Premise.** A property of foreign code that a theorem of the specification takes as an explicit assumption (ADR 0109 rule 5.3). The premises of this record are FP1 to FP6 (rule 24).
- **Worker.** An operating-system thread that runs Una tasks (ADR 0117 rule 2). Before ADR 0117 is implemented, each thread of a run is its own worker.
- **`ℓ*(T)`.** Every label anywhere in the type `T`, joined (`spec/07 §7` rule 3).
- **T6.** The noninterference theorem of `spec/10 §10`.

## Context

- **No foreign code exists today.** Every byte of a native executable is generated by `una-codegen` or comes from `una-rt` and its pinned crates (`spec/16`). `unsafe` Rust is confined to `crates/una-rt/src/ffi.rs` (ADR 0058) and, once ADRs 0116 and 0117 land, to `una-sys`. ADR 0114 relies on this: "Neither `una-rt` nor a host capability changes the environment afterwards; Una has no foreign function interface."
- **The gate is decided.** ADR 0109 rule 5 reserves foreign code for `systems` packages and for modules listed in `[trusted]`, makes it the only entry that a gated construct may add to the trusted base (rule 5.1), and requires every theorem to take the listed foreign entries as a premise (rule 5.3). Its record MUST state what a foreign call may assume and must guarantee, how its effects enter `uses`, how its labels are declared, and how the report counts it.
- **No ambient authority.** The only way to act outside the program is an operation on a capability in scope (`spec/05 §1`). Host capabilities enter only through `main`'s `uses`, and a run grants each of them explicitly (`spec/05 §7`). A capability's `ensures` is an assumption about the outside world, listed in the Assumed zone (`spec/05 §2`).
- **Trusted modules.** A module is trusted when its header says `trusted module` and `una.toml` lists it (U7005). Today it may hold policies, whose `via` functions skip label checks (`spec/07 §5`, `§6`) and run on their caller's bindings (ADR 0092, second amendment). ADR 0109 Alternative E warns against mixing a second trusted base into `[trusted]`.
- **Labels.** Every capability argument is a sink bounded by the operation's `accepts`, `public` by default (`spec/07 §4`).
- **The risk profile.** `docs/security/tcb.md` names foreign code as "the largest addition to come": code that no Una check covers, and memory-safety risk at a new boundary (§5). The memory-safety tooling of the runtime boundary is its first open gap (§6).
- **The proof rule for elision.** A run-time check is removed only where a false proof costs a wrong value, never an access out of bounds (ADR 0106 RB-3a, ADR 0112 §2). A foreign function's preconditions are usually what its memory safety rests on.
- **Stacks.** A native executable "never overflows its stack" (`spec/16 §3` rule 3): Una code checks its depth. Under ADR 0117, task stacks come from a pool, are reused, and are sized from a depth of 1,000 by default; whatever an earlier frame left below the stack pointer stays there.
- **Byte buffers.** `List[U8]` is the language's byte sequence (`spec/18 §1` rule 3), and `bytes(t)` turns a `Text` into one (`spec/22`).
- **Static linking.** In a static link, a global symbol that an archive defines can take the place of the same name in the C library or in another library, and the object formats carry linker instructions of their own (autolinking, dependent libraries, link-time optimization). An audit that does not rule these out does not bound what an archive changes.

The design aims at the most restrictive form that keeps the purpose of foreign code in `systems` packages: fast, existing implementations of pure computations, such as compression, codecs and checksums. Foreign code that performs input or output, keeps state between calls, allocates, uses the C math library, or calls back into Una is out of the first version.

## Decision

In one paragraph: a `systems` package, in an edition that admits it, may declare a **foreign capability** in a trusted module. Its operations name C symbols of a static archive that `una.toml` pins by digest per target, built by the package's authors from source kept in the package. Foreign code is reached only through that capability, which enters a run as a host capability does, by a grant, and never from a declassification. Arguments and results are fixed-width scalars, `Bool`, `F64` and byte buffers, which the runtime copies into a compartment apart from Una's memory, where the call runs on its own stack with the other registers cleared. Every result is labeled at least as high as everything the capability accepts. `requires` and `ensures` are proven or assumed as for any capability, and the runtime also checks both at every call. `una build` audits a private copy of each archive by section types, symbols, imports and load commands, links it with hardened flags, and verifies the linked executable. `una run` and the tests run a verified Una model of each capability, and `una test --native`, which `una build` runs first, compares it with the archive in a harness confined by the operating system: so building such a package runs its archive once. **A foreign archive is trusted as the runtime is: the audit and the boundary checks detect common defects of honest code, and do not confine code written to evade them.**

### A. The gate

1. **Edition and profile.** A foreign capability is legal only in a package of an edition that admits it (Question 5), with that edition's opt-in if it needs one, and whose `[package] profile` is `"systems"` (ADR 0109 rule 1). In an `app` package it is U3030, and a `[foreign]` table is U7002. In editions `0.1` to `0.3`, `foreign` is not a word of the language, and the declaration is a syntax error (U0101).
2. **The module.** A foreign capability is declared only in a module that is trusted (`spec/07 §6`) and that `una.toml` names as the module of its library (rule 7). Otherwise it is U3030. A module that declares a foreign capability declares no `policy` and no other foreign capability (U3030), so each trusted module belongs to one of the two trusted bases, and one library to one capability.
3. **Nothing else changes.** A package that declares no foreign capability is typed, checked and run exactly as before (ADR 0109 rule 5.2).

### B. Declaration

4. **Syntax.** These productions extend `spec/02 §4`:

   ```ebnf
   decl             = ... | foreign_cap_decl ;
   foreign_cap_decl = [ "pub" ] "foreign" "capability" upper_ident NEWLINE
                      INDENT "library" text_lit NEWLINE
                      foreign_op { foreign_op } DEDENT ;
   foreign_op       = "op" lower_ident "(" [ params ] ")" "->" type NEWLINE
                      INDENT "symbol" text_lit NEWLINE
                      [ "capacity" add_expr NEWLINE ]
                      { op_clause } DEDENT ;
   ```

   - **Words.** `foreign` is a keyword only at the start of a top-level declaration, after an optional `pub` and before `capability`. `library`, `symbol` and `capacity` are keywords only at the start of a line inside a foreign capability. Elsewhere, all four are identifiers, as `measure` is (`spec/01 §5`).
   - **Canonical form** (`spec/15`): `library` first; in each operation, `symbol`, then `capacity`, then the clauses of `op_clause` in the order of `spec/02 §4`.

   ```una
   trusted module codec.zstd

   pub kind ZstdError = Corrupt | TooLarge

   pub foreign capability Zstd
     library "zstd"
     op bound(length: U64) -> internal U64
       symbol "zstd_shim_bound"
       requires length <= 1073741824
       accepts internal
     op compress(source: List[U8], level: I32 in 1..19) -> internal List[U8]
       symbol "zstd_shim_compress"
       capacity source.len + source.len / 128 + 512
       requires source.len <= 67108864
       fails ZstdError
       accepts internal
   ```

   Every result of `Zstd` is labeled `internal`, since `compress` accepts `internal` (rule 21). A helper that should return `public` beside it needs a library and an archive of its own.
5. **Symbols.** A `symbol` is a C identifier, `[A-Za-z_][A-Za-z0-9_]*`, of at most 128 bytes. It MUST NOT begin with `una_`, `_una` or `__`, and MUST NOT be a name of the import allowlist (rule 9). Each symbol is named once in the package. A violation is U2036 at the symbol.
6. **What an operation may not have.** A foreign operation has no type parameters, no `edit` or `take` parameter, and no default. Its types follow rule 11 (U2036). Its clauses follow rules 20 to 23.

### C. Libraries, linking and the audit

7. **The `[foreign]` table.** A package MAY hold up to 32 tables `[foreign.<library>]`, one per library, where `<library>` matches `[a-z][a-z0-9-]{0,63}`:

   ```toml
   [foreign.zstd]
   module = "codec.zstd"
   model = "codec.zstd_model.zstd_model"
   source = "vendor/zstd"
   stack_kib = 256
   archives = [
     { target = "aarch64-apple-darwin", path = "vendor/zstd/build/aarch64-apple-darwin/libzstd_shim.a", sha256 = "<64 hex digits>" },
     { target = "x86_64-unknown-linux-gnu", path = "vendor/zstd/build/x86_64-unknown-linux-gnu/libzstd_shim.a", sha256 = "<64 hex digits>" },
   ]
   ```

   - `module`: the trusted module that declares the capability with this `library` line. A table whose module declares none, a `library` line with no table, and two capabilities naming one library are U3030 at the declaration, or U7002 at the table when no declaration names it. A `library` text that does not match the pattern is U3030 at the text.
   - `model`: the qualified name of the capability's model (rule 31).
   - `source`: the directory of the package that holds the C source and the recipe of rule 8, so that a reader can rebuild each archive and compare it. Its path is relative to the package root and contains no `..` segment; every component is a directory, none a symbolic link, and an entry inside it that is a symbolic link is U7002. **The toolchain does not check that an archive came from it** (Alternative K). The report lists the directory and a digest of its contents: SHA-256 over the sorted list of its regular files, each as its path relative to the directory, its length and its own SHA-256. A reader then sees when the source changed and the archive did not, or the reverse.
   - `stack_kib`: the size of the compartment's stack, in KiB, from 16 to 8192 (rule 26).
   - `archives`: one entry per supported target (`spec/16 §1`), each at most once. `path` is relative to the package root, contains no `..` segment, and every component is a directory or, last, a regular file, none a symbolic link. `sha256` is the lowercase hex digest of the file.
   - Any other key, a missing key, or a value of the wrong type or out of range is U7002. In an edition that does not admit foreign code, `[foreign]` is an unknown table (U7002).
   - A target without an entry is not an error for `una check`, `una run` or `una test`; `una build` for that target is U7006 at the table.
8. **The build recipe.** The specification chapter documents how an archive is made, and the audit (rule 9) checks its outcome:
   - compile with `-fvisibility=hidden`, `-fno-common`, `-fstack-protector-strong`, stack probes (`-fstack-clash-protection` on Linux; on Darwin they are on by default), `-fno-lto`, `-DNDEBUG`, and none of the flags that ADR 0114 forbids (`-ffast-math` and the others of its rule 6.2);
   - pre-link all objects into one relocatable object (`ld -r`) that exports only the declared symbols: on Mach-O, `ld -r -exported_symbols_list`; on ELF, `ld -r` followed by `objcopy --localize-hidden` and `objcopy --keep-global-symbols`. Hidden visibility alone is not enough, since hidden globals still resolve across objects in a static link;
   - archive that one object.
9. **The archive audit.** Every command that checks a package with a `[foreign]` table, `una lsp` included, audits each listed archive for every target it lists, and reports each finding as U7006 at the table entry. The audit reads the file once into memory, through a directory descriptor of the package root with `O_NOFOLLOW` on every component, and hashes and parses those same bytes. An archive passes when all of these hold:
   - **Digest.** The bytes' SHA-256 equals `sha256`.
   - **Format.** It is a `!<arch>` archive of exactly one ELF or Mach-O relocatable object for the entry's target, at most 256 MiB. No ELF section group (`SHT_GROUP`, COMDAT) exists.
   - **Sections, by name and type together on ELF, and by (segment, section, type) on Mach-O.** Linkers place input sections by name, so neither a name nor a type alone is enough.
     - **ELF, allocated sections:** only these names, each with its type and flags:
       - `.text` and `.text.*`: `SHT_PROGBITS`, `SHF_ALLOC` and `SHF_EXECINSTR`, without `SHF_WRITE`;
       - `.rodata` and `.rodata.*`: `SHT_PROGBITS`, `SHF_ALLOC`, without `SHF_WRITE` or `SHF_EXECINSTR`;
       - `.data.rel.ro` and `.data.rel.ro.*`: `SHT_PROGBITS`, `SHF_ALLOC` and `SHF_WRITE`, the only writable sections allowed, read-only after relocation (rule 10);
       - `.eh_frame`: `SHT_PROGBITS`, or `SHT_X86_64_UNWIND` on x86-64, `SHF_ALLOC`, without `SHF_WRITE` or `SHF_EXECINSTR`;
       - `.note.GNU-stack` and `.note.gnu.property`: `SHT_PROGBITS` or `SHT_NOTE`, without `SHF_EXECINSTR`.

       No allowed section has `SHF_TLS`. Each object MUST have `.note.GNU-stack` without `SHF_EXECINSTR`.
     - **ELF, sections without `SHF_ALLOC`:** `.comment`, `.debug_*`, `.llvm_addrsig` (`SHT_LLVM_ADDRSIG`), `SHT_SYMTAB` and `SHT_STRTAB` sections, and `SHT_REL` and `SHT_RELA` sections whose target (`sh_info`) is an allowed section.
     - **ELF, everything else is rejected, whatever its type or flags.** That includes `.init`, `.fini`, `.init_array*`, `.fini_array*`, `.preinit_array*`, `.ctors*`, `.dtors*`, `.tm_clone_table`, `.data*` other than `.data.rel.ro*`, `.bss*`, `.tdata*`, `.tbss*`, `.got*`, `.plt*`, `.dynamic`, `.interp`, `.deplibs` (`SHT_LLVM_DEPENDENT_LIBRARIES`), `.linker-options` (`SHT_LLVM_LINKER_OPTIONS`), `.gnu.lto_*`, `.llvm.lto`, LLVM bitcode, any section of type `SHT_INIT_ARRAY`, `SHT_FINI_ARRAY`, `SHT_PREINIT_ARRAY` or `SHT_GROUP`, and any other name. A read-only `SHT_PROGBITS` section named `.init_array.5` is rejected by its name.
     - **Mach-O:** only these (segment, section, type) triples: `__TEXT,__text` (regular, pure instructions), `__TEXT,__const`, `__TEXT,__cstring` (`S_CSTRING_LITERALS`), `__TEXT,__literal4`, `__TEXT,__literal8`, `__TEXT,__literal16` (their literal types), `__TEXT,__eh_frame`, `__LD,__compact_unwind`, `__DATA,__const` (regular, read-only after relocation, rule 10), and `__DWARF` debug sections. Everything else is rejected, including every section of type `S_MOD_INIT_FUNC_POINTERS`, `S_MOD_TERM_FUNC_POINTERS`, `S_INIT_FUNC_OFFSETS` or `S_THREAD_LOCAL_*` under any name, any other section of a writable segment, Objective-C metadata (`__objc_*`), and `__LLVM,__bitcode`.
     - **Mach-O load commands:** only `LC_SEGMENT_64`, `LC_SYMTAB`, `LC_DYSYMTAB`, `LC_BUILD_VERSION` or `LC_VERSION_MIN_*`, `LC_DATA_IN_CODE` and `LC_LINKER_OPTIMIZATION_HINT`. `LC_LINKER_OPTION` (autolinking) and every other command are rejected.
   - **Symbols.**
     - The only global symbols the object defines, of any binding (strong, weak, `STB_GNU_UNIQUE`) and visibility, are the `symbol`s its capability declares, each a function defined in an executable section. Every other defined symbol is local.
     - No symbol is `STT_GNU_IFUNC`, absolute (`SHN_ABS`, Mach-O `N_ABS`), except local `STT_FILE` symbols, indirect (`N_INDR`), common (`SHN_COMMON`, Mach-O common), or thread-local.
     - No relocation is `R_*_IRELATIVE` or of a thread-local type.
   - **Imports.** Every undefined symbol is on the import allowlist of the entry's target, matched by its C name after the format's mangling (a leading `_` on Mach-O) and, on ELF, on its base name without a version (`memcpy@GLIBC_2.14` is `memcpy`). A weak undefined reference is treated as an import. The candidate list, written as C names:
     - from the C library: `memcpy`, `memmove`, `memset`, `memcmp`, `strlen`, `bzero`, and `abort`, which ends the process;
     - the stack protector: the function `__stack_chk_fail`, which ends the process and, on glibc, writes a line to stderr, and the read-only datum `__stack_chk_guard`;
     - the fortified `__memcpy_chk`, `__memmove_chk` and `__memset_chk`, which end the process on an overflow;
     - on `aarch64-apple-darwin`: the stack probe `__chkstk_darwin`, `__bzero` and `memset_pattern16`;
     - the integer helpers `__udivti3`, `__umodti3`, `__divti3`, `__modti3` and `__multi3`, which Rust's `compiler_builtins` inside `libuna_rt.a` defines, not the C library.

     The specification chapter pins, per target, the subset that the `foreign` gate's corpus, compiled with the system `cc` and the recipe of rule 8, actually needs, and only an amendment extends it. Nothing that allocates, performs input or output, reads a clock or randomness, handles signals, jumps (`setjmp`, `longjmp`), unwinds, starts threads, changes the floating-point environment, or names `una_rt_*` is on it. The C math library is not on it either, so numeric kernels that need `exp` or `log` are out of the first version.
10. **Linking.** `una build` copies each archive of its target into a private directory of the build, through the same descriptor-relative, `O_NOFOLLOW` open, and hashes and audits the copy before it links that copy, never the original path.
    - It links the copies after the program's objects and before `libuna_rt.a`, by path, and passes no `-l`, `-L`, linker script, framework or flag that the package supplies. It links with `-z noexecstack -z relro -z now` on ELF, and with `-ignore_auto_link` on Mach-O where the linker supports it. The system C library, which the executable already links, resolves the C-library imports of rule 9 and nothing else.
    - **Verifying the executable.** After linking, `una build` reads the linker's map and the executable, and writes no executable (U7006) unless:
      - each declared symbol is defined exactly once, inside sections that come from its copy;
      - the only references that resolve into a section from a copy are the generated code's references to the declared symbols, which it hands to the trampoline (rule 27); no reference from `libuna_rt.a` or `una-sys` does;
      - every section from a copy lies in a segment that is read-only at run time, its code in a read-only executable one, including `.data.rel.ro` under `PT_GNU_RELRO` and `__DATA,__const` in `__DATA_CONST`;
      - on ELF, `PT_GNU_STACK` is present and not executable.

      Where a check cannot be verified on a linker, the allowance it guards (`.data.rel.ro`, `__DATA,__const`) is rejected instead.
    - `una build` never compiles C.

### D. The boundary: types, calling convention and buffers

11. **Types.** A parameter's type is `Bool`, a fixed-width integer type (`I8` to `U64`, `spec/18`), `F64` (ADR 0114), or `List[U8]`, optionally refined (rule 23). A result's type is one of these, or `Unit`. A parameter's type carries no label (its bound is `accepts`, rule 20), and the result's type carries exactly one written label, at the top (rule 21). `Int`, `Text`, kinds, `Option`, `Result`, tuples, lists of other types, maps, function types and type parameters are U2036. The error kind of `fails` has only variants without payloads, at most 255 of them (U2036).
12. **The C signature.** Every foreign operation is a C function that returns `int32_t`, the status, and takes, in order:
    - for each parameter in declaration order: `uint8_t` for `Bool`; `int8_t` to `int64_t` and `uint8_t` to `uint64_t` for the integer types; `double` for `F64`; and, for `List[U8]`, one pointer to an input descriptor, `const struct una_in { const uint8_t *ptr; size_t len; } *`;
    - for a scalar result of C type `T`: one argument `T *out`;
    - for a `List[U8]` result: one pointer to an output descriptor, `struct una_out { uint8_t *ptr; size_t cap; size_t len; } *`, where the runtime fills `ptr` and `cap` and sets `len` to 0, and the callee writes `len`;
    - for `Unit`: nothing.

    The specification chapter publishes the two structures in a header, `una_foreign.h`. Every argument travels in a register: at most 6 integer-class arguments, pointers included, and 8 `double` arguments (U2036 otherwise), so no argument is passed on a stack. A `List[U8]` costs one register whichever its direction, so an HMAC (key, message, output), a dictionary compressor (source, dictionary, level, output) or an authenticated cipher (key, nonce, data, output) fits. The call uses the target's C calling convention, and the caller extends 8-bit and 16-bit integer arguments to 32 bits, as Apple's AArch64 convention requires of it. A library whose own interface has another shape is reached through a shim of C functions with this shape, compiled into the archive.
13. **The status.** Status `0` is success. Status `k`, for `1 ≤ k ≤ n`, is the failure of the `k`-th variant, in declaration order, of the operation's error kind of `n` variants. Any other status, or any status other than `0` for an operation without `fails`, is U6008. On a failure, the outputs are discarded.
14. **Results.** After a status of `0`:
    - a `Bool` result MUST be `0` or `1` (U6008 otherwise);
    - an `F64` result is taken bit for bit, except that every NaN becomes the canonical NaN (ADR 0114 rules 1.1 and 6.1);
    - a `List[U8]` result has length `out->len`, which MUST be at most the capacity the runtime set (U6008 otherwise), and its bytes are copied into a new Una list. The runtime reads the bytes from the address and capacity it set, kept outside the compartment, never from `out->ptr` or `out->cap` as the callee left them.
15. **Buffers, ownership and lifetimes.**
    - **Inputs.** For each `List[U8]` argument, the runtime copies the bytes into the compartment's arena (rule 26), writes an input descriptor there, and passes its address. Foreign code may read `len` bytes from `ptr`. It is valid only during the call. Whatever foreign code writes into it is discarded, and no Una value ever aliases it.
    - **The output.** The runtime reserves `cap` bytes in the arena, zeroed, and passes an output descriptor that points at them. Foreign code may write within them. They are valid only during the call. A large output is zeroed by taking fresh zero pages from the operating system rather than by a pass over its bytes.
    - **Descriptors and result slots.** The descriptors and the scalar result slot `*out` are in the arena too, written or zeroed before the call. What the callee writes into a descriptor other than `out->len` is ignored.
    - **Empty buffers.** An input of length 0 and an output of capacity 0 still get a valid address that is not null.
    - **After the call.** The runtime checks the canaries (rule 28) before it releases or reuses anything, then copies the output into a Una value, and evaluates every clause on that copy, never on the arena.
    - **Retention.** Foreign code MUST NOT use an address it was given after it returns (FP1). An address it leaves on its stack can only point into its own compartment, never into the Una heap (rule 26).
    - **`capacity`.** It is required for a `List[U8]` result and forbidden otherwise (U2036). It is an `Int` in the refinement language over the parameters (`spec/03 §3`), and each call carries the precondition `0 <= capacity and capacity <= 1073741824`, an obligation of kind `precondition` (`spec/12 §3`).
16. **No callbacks.** No function value, closure, capability or runtime object crosses the boundary, and the allowlist names no entry point of the runtime. Foreign code cannot call into Una.

### E. Effects and authority

17. **A foreign capability is a capability.** Foreign code is reached only by an operation `@x.op(...)` on a binding of a foreign capability, so every function that reaches it, directly or through callees, declares or infers that `uses` (`spec/05 §3`), and its authority is in the report's `authority` section, as for any capability. Its operations are effects, traced as host operations are (`spec/11 §5`), with the same event in both engines.
    - **Where it enters.** A foreign capability enters a run only through the `uses` of the program's entry, as a host capability does (`spec/05 §7` rule 1): `main`, or the functions of a `[drive]` module (ADR 0116 rule 2). It is then passed down by name or by `with`, rebound and narrowed like any capability (`spec/05 §4`), and is subject to `[boundaries]` ceilings (ADR 0093) and `[contracts]` locks (ADR 0092). Unlike a host capability, it runs code of the package (Appendix A, ADR 0093).
    - **Grants.** The grant spec `foreign:<library>` grants every operation of that library's capability. It is accepted in `[run] grants`, by `--grant` of `una run`, and by `--grant` of the executable. A `uses` of a foreign capability in the entry that no grant allows is U3012, before anything runs, as for a host capability (`spec/13 §5`, `spec/16 §3` rule 2).
    - **Handlers.** Any handler may implement a foreign capability, such as a fake in a test. Only the binding the entry receives from the runtime runs foreign code.
    - **Examples and tests.** A `do` line that needs a foreign capability that no `given` provides is U3014, as for a host capability (`spec/05 §9` rule 5). A test never runs foreign code in the interpreter.
18. **What the foreign code may do.** Nothing outside the call: no system call, input, output, allocation or global state (FP3). The audit of rule 9 rules out the ways that go through symbols, sections and linker inputs. It does not rule out an inline system-call instruction, a jump into one inside the C library, or a call through an address computed at run time, for example from the address of an allowlisted function plus an offset that a known C library places at `open`. Those need no memory-safety violation, and only a sandbox stops them (Question 1).
19. **No foreign code in a declassification.** For each policy call, the code that the call can run through its `via` function is followed as `spec/07 §6` follows a call into a boundary:
    - the `via` function's body and the functions it calls;
    - the bindings it receives from the caller, since a `via` runs on its caller's bindings (ADR 0092, second amendment), with the handlers they resolve to at the call, those handlers' own `uses`, and handlers installed outside the call and granted on to it by name;
    - the function values and lambdas among the arguments, with the bindings their `uses` resolve to.

    The call is U3031, with the foreign operation or binding as the related span, when that code can perform an operation of a foreign capability, or a binding of a foreign capability reaches it. A function value whose code is not known at the call counts as reaching foreign code whenever the package declares a foreign capability. Separately, a `via` function whose `uses` names a foreign capability is U3031 at that clause, wherever it is called.

### F. Labels

20. **`accepts` is required.** Every foreign operation writes `accepts`, `public` included (U4009 when it is missing). Each argument is a sink at that label, as for any operation (`spec/07 §4`), and a call under a raised program counter needs `pc ⊑ accepts` (`spec/07 §3` rule 4).
21. **The result's label is written and covers every input.** Let `A` be the join of the `accepts` labels of every operation of the capability. The result type of every operation writes its label at the top, `public` included, and that label MUST be at least `A` (U4009 otherwise). Data that stays in the capability's compartment from one call to the next came from arguments that some operation of the same capability accepted (rule 26), so the rule holds whichever operation's input a result depends on. The cost: one library gives all its results one label, and an operation that should return a lower label needs a separate library and archive.
22. **The failure's label.** The failure of a foreign operation is labeled `pc ⊔ A`, where `pc` is the program counter at the call, since foreign code decides it from its arguments, unchecked. As for a policy call (`spec/07 §5` rule 6), `attempt` gives a `Result` with that label, and `if error pass` and `if error fail` are U4002 when it is above `public`.

    A foreign result can therefore never lower a label: foreign code is not a declassifier, and rule 19 keeps it out of the policies that are.

### G. Contracts, verification and the premises

23. **`requires` and parameter refinements** are preconditions, proven at every call site as for any operation (`spec/05 §2`). They are not assumptions. What is trusted is that they are sufficient for the foreign code's own safety, which is part of FP1. The runtime checks them again before each call (rule 28).

    **`ensures` and the result's refinement** are assumptions: the verifier assumes them at every call site, as it assumes a capability's `ensures`. Each is listed in the build report (rule 35), and the model is verified against it like any handler (`spec/05 §5` rule 2). The runtime checks each of them after each successful call (rule 28), so a false one ends the run instead of reaching a proof that relied on it.
24. **The premises.** Foreign code is in the Trusted zone. Every theorem of the specification and of the formal track holds relative to these premises for each foreign capability listed in the report (ADR 0109 rule 5.3). Against an archive written to evade the checks, every premise is an assumption.

    | Premise | Foreign code ... | Checked at build | Checked at each call | Tested | Assumed |
    | --- | --- | --- | --- | --- | --- |
    | FP1 Memory | reads only its inputs, its read-only data, and memory it wrote during the same call; writes only its compartment's stack, its output buffer and its result slots; uses no address it was given after it returns; stays within `stack_kib` | sections, symbols and linker inputs (rules 9, 10) | canaries, length, guard regions (rules 26, 28) | sanitizers on the gate's test archives | the rest |
    | FP2 Determinism | gives a status and outputs that are a function of the argument values | no clock, randomness or writable state can be linked (rule 9) | | `una test --native` on the package's own archive (rule 33); the `foreign` gate on the repository's | within the run |
    | FP3 No effects | performs no system call, input, output or allocation | import allowlist and linker inputs (rules 9, 10) | | | inline or computed calls (rule 18) |
    | FP4 Contract | meets the `ensures` and result refinements, given the `requires` | | every clause (rule 28) | | none: a violation aborts |
    | FP5 Environment | returns with the floating-point environment and the callee-saved registers as it found them, and does not unwind | no `fenv` or unwinding import (rule 9) | floating-point control and callee-saved registers (rules 29, 27) | | |
    | FP6 Termination | returns | | | | out of model, as timing is |

    FP4 is never an assumption of a run: the boundary checks make every violation an abort. It remains one of the verifier, which assumes the clauses at call sites, but no run continues past a violation.

### H. Memory safety at the boundary

25. **What the runtime guarantees.** Una values never alias foreign memory, and foreign code never receives a Una handle, object, list, closure or runtime pointer, or any address in the Una heap or on a Una stack: only scalars and addresses inside its own compartment. At the call, every register but the argument registers in use and the stack pointer is zero, the frame pointer included, and the state needed to return to Una is kept outside the compartment (rule 27). If foreign code respects FP1, no Una value or runtime structure can be changed by it, and no Una data can be read by it beyond its arguments. If it does not, the boundary detects the common violations below; it cannot prevent all of them, since foreign code runs in the process, with the process's permissions.
26. **The compartment.** Each foreign call runs in the compartment of its (worker, capability) pair, created at the first call of that capability on that worker and kept until the worker ends:
    - **Layout.** One mapping, apart from the Una heap and from every Una stack: a guard region of at least 1 MiB, the stack of `stack_kib`, a guard region of at least 1 MiB, the buffer arena, and a guard region of at least 64 KiB. Guard regions are mapped without access, so they cost address space, not memory. The arena has a base size of 64 KiB. A call that needs more grows it by doubling, and after that call it shrinks back to the base size, returning the pages to the operating system.
    - **Separation.** Data left on the compartment's stack or in its arena came only from earlier calls of the same capability on the same worker, whose labels rule 21 already covers. An address kept from an earlier call reaches only this compartment. An over-read of an input runs into the arena's other buffers of the same call or a guard region, never into Una data.
    - **The budget.** The stack and the base arena are charged to the run's memory budget (`spec/11 §7`) for as long as the compartment exists; growth beyond the base is charged only during the call that needs it. A charge that does not fit is U6003 at the call.
    - **The same charge in the interpreter.** When it runs a model's operation, the interpreter charges the same amounts: the stack and base arena of one compartment per foreign capability from its first call, and the growth during the call. With one worker, both engines therefore reach U6003 at the same calls, and the comparison of `spec/16 §5` runs programs with foreign calls with `--workers 1`. With more workers, the native run holds one compartment per worker, and a U6003 at a foreign call that only those extra compartments cause is a stated difference of the engines, not a divergence.
    - **The stack.** Foreign code runs on the compartment's stack, never on a Una stack, so ADR 0117's depth-sized task stacks are not involved. With the stack probes that the recipe requires (rule 8), a frame that would overrun the stack touches the guard region and the process ends by the signal. `spec/16 §3` rule 3 is amended to say so (Appendix A).
27. **The call.** A trampoline in `una-sys`, under ADR 0058's rules as amended in Appendix A, performs every call:
    1. **Save, outside the compartment.** It saves every callee-saved general-purpose register, the lower 64 bits of every callee-saved vector register, the frame pointer, the link register on AArch64, and the Una stack pointer, on the Una stack of the calling thread or task, which foreign code is never given an address in. Nothing it needs to return to Una is ever stored in the compartment. After the call, every general-purpose register it could have used is zero or the callee's, so it finds the save area again through a slot of the thread's runtime state, reached through the thread pointer (`tpidr_el0` on AArch64, the `fs` base on x86-64), which the call does not change. The thread pointer is the one value that reaches foreign code and leads to runtime memory: honest code reads through it only the stack protector's slot, and any other read through it is a violation of FP1.
    2. **Switch.** It sets the stack pointer to the top of the compartment's stack.
    3. **Clear.** It loads the arguments into their registers and sets to zero every other general-purpose and vector register, callee-saved and caller-saved alike, the frame pointer included, and every vector register in full:
       - on AArch64: x0 to x17 not carrying arguments, x19 to x29, and v0 to v31 not carrying arguments; x18 is left untouched on `aarch64-apple-darwin`, where the platform reserves it, and zeroed on Linux;
       - on x86-64: rax, rbx, rcx, rdx, rsi, rdi and r8 to r15 not carrying arguments, rbp, and xmm0 to xmm15 not carrying arguments, with their upper parts as ymm and zmm registers where the processor has them; where it has AVX-512, also zmm16 to zmm31 and the mask registers k0 to k7; the direction flag is clear, as the convention requires.

       A zero frame pointer also ends any walk of frames at the compartment.
    4. **Call** the symbol, through the link register or the return address the call instruction sets.
    5. **Check.** On return, before any saved state is used, it compares every callee-saved general-purpose register, the lower 64 bits of every callee-saved vector register and the frame pointer with zero, the value they held at the call, and the stack pointer with the compartment's stack top. The convention requires the callee to return them unchanged.
    6. **Restore and switch back** from the saved copy, then report a difference found in step 5 as U6008.

    Generated code passes the trampoline the symbol's address, through a relocation the linker resolves to the declared symbol, and an argument block. It never jumps to foreign code itself.
28. **Boundary checks.** Around each foreign call, in the native executable:
    1. **Before:** evaluate every `requires` and every refinement of the parameters, and the `capacity` bound, and end the run with U6008 if one is false. They were proven at the call site, so a false one is a defect of the toolchain, which this check keeps from becoming a call with arguments the foreign code was never meant to receive.
    2. Copy the inputs into the arena, reserve and zero the output and the result slots, and place a 16-byte canary of random bytes, drawn once per compartment, before and after each buffer.
    3. **After:** check the canaries first, then the status (rule 13), `out->len` against the capacity, and a `Bool` result (rule 14); copy the output into a Una value; then, after a status of `0`, evaluate every `ensures` and every refinement of the result on that value. Any failure is U6008, which names the operation and the check.

    These checks are run-time checks, not obligations. ADR 0112 never elides them, and no proof removes them. Clauses are compiled as `checked` compiles its predicate (`spec/06 §6`), opaque predicates included. A canary is readable by the code it guards, so it finds overflows by defect, not by design (rule 18).
29. **The floating-point environment.** After each call, the runtime reads the control register (MXCSR on x86-64, FPCR on AArch64). When its control fields differ from the default environment of ADR 0114 rule 6.4, the run ends with U6006 at the call, with the trace written (Appendix A, ADR 0114). Its status flags are cleared, since no Una code reads them. The x87 control word is not checked: generated code, the runtime and the allowlisted imports perform no x87 arithmetic, so its state reaches no Una value.

    A crash inside foreign code, such as a segmentation fault or an `abort`, ends the process by the signal, with no trace file. It is outside the Una Abstract Machine, and the report says so (rule 35). ADR 0116's signal handlers run on an alternate signal stack (Appendix A), so a signal that arrives during a foreign call does not run on the compartment's stack.
30. **Not interruptible.** A foreign call is not a check point and not an interruptible operation (ADR 0116 rule 12): a deadline, a host stop or a time slice takes effect when it returns, and it holds its worker meanwhile (ADR 0117). One input that makes foreign code run long can therefore pin a worker until the hard stop. The chapter's guidance is to bound input lengths with `requires`, as the example does, and the report marks every foreign operation `"interruptible": false`.

### I. The interpreter, tests and determinism

31. **The model.** The `model` key of rule 7 names a handler `for` the foreign capability, declared in a module of the package that is not trusted, with no parameters and no `uses` (U3031 otherwise; U3010 when it is not a handler of that capability). It is ordinary Una code: type-checked, flow-checked, and verified against each operation's `ensures` (`spec/05 §5`), including the worst-caller rule. It MUST compute what the foreign code computes, bit for bit, on every argument that meets the operation's `requires`: the same status and the same outputs. Rule 33 checks it.
32. **`una run`** installs the model as the entry's binding of each foreign capability, after the grant check of rule 17, and runs the program on it: the interpreter never runs foreign code. In human mode it prints, on stderr before the program starts, one line per capability, `note: foreign capability codec.zstd.Zstd runs its model codec.zstd_model.zstd_model`. With `--json`, `una.run/1` gains `foreign_models`, after `tier`: a list of `{"capability": ..., "model": ...}`, or `[]`. The key is additive, and the schema stays `una.run/1`. `una test` runs tests as always, with the handlers their `given` lines name.
33. **`una test --native`: the model against the archive.** For a package with foreign capabilities:
    - it runs the package's tests and examples in the interpreter, as `una test` does, and records each call of an operation of a model that a `given` line installed, with its arguments and its outcome;
    - it builds, through the link and verification of rule 10, a harness executable that links the host target's archives and the runtime, and replays each recorded call through the compartment and the boundary checks of rules 26 to 29;
    - it compares each outcome, the status and the outputs, with the model's. A difference is a failed test. So is an operation of a foreign capability that no recorded call reaches. What `una build` and `una test --native` print about a difference is bounded: for each operation, only its first differing call, as the operation, the index of the recorded call, and the first byte offset at which the status or an output differs, never the bytes of an outcome. The outcome stream itself is bounded by the size of the recorded outcomes plus 64 KiB; a harness that writes more fails the comparison.

    **This runs the archive's code on the machine that builds**, so the harness is confined:
    - **A separate process,** started with an empty environment, its working directory a fresh temporary directory, and the recorded calls on a pipe. It inherits no file descriptor but its two pipes: it is spawned with `POSIX_SPAWN_CLOEXEC_DEFAULT` on macOS, and on Linux it calls `close_range` on every other descriptor before it confines itself.
    - **Before confining,** it reads all its input, creates the compartment of each foreign capability, draws the canaries, and sets `RLIMIT_AS` to the memory budget plus the compartments' mappings, so that the kernel bounds its memory whatever it maps later. It then writes its outcomes to the second pipe.
    - **On Linux:** a Landlock ruleset that grants no file-system access but reading and writing inside the temporary directory, and no network access, then a seccomp filter that kills the process on any system call outside an allowlist. The allowlist is what an honest harness actually calls, and no more: `read` and `write` on its two pipes; anonymous `mmap` and `mprotect` without `PROT_EXEC`, `munmap`, `mremap`, `madvise` and `brk`, for the arena's shrinking and the allocator; `getrandom`; `futex` and `rt_sigprocmask`, which the standard library may call; `sigaltstack`, `rt_sigreturn`, `exit` and `exit_group`. A test runs an honest harness under the filter's logging mode and fails if it makes any other call, and the list in the chapter is pinned to that test.
    - **On macOS:** `sandbox_init` with a profile that denies everything by default, `(deny default)`, and then allows only reading and writing inside the temporary directory and the other operations that the same test shows an honest harness needs, each listed in the chapter. Network access, process creation, and file reads and writes elsewhere stay denied. The interface is deprecated, but it is the one the system offers to a process confining itself; a later record replaces it when the platform offers a successor.
    - **Fail closed.** If a confinement cannot be applied, for example on a Linux kernel without Landlock, the harness does not run, and `una build` refuses as on a failed comparison, naming the missing mechanism.
    - **A time limit.** The harness is killed after `--model-check-timeout <seconds>`, from 1 to 3600, 300 by default, and the comparison fails with "the comparison of `zstd` did not finish". Its memory is bounded by `--max-memory`, as a run's is.

    Since the archive's operations are pure (FP3), the confinement takes nothing they need away, and it covers building an untrusted package, such as a proposed change in continuous integration. **The comparison finds defects; it does not certify.** Inside the harness, a hostile archive can write outcomes that match the model, so `model_compared: passed` means that honest code agreed with its model on the recorded calls, and no more.

    **It is required.** `una build` of a package with foreign capabilities runs it first, and writes no executable (U7006 at the table, "`zstd` disagrees with its model") unless it passes.
    - **The skip.** `una build --skip-model-check` writes the executable without running the comparison, for code one does not want to run at all. Its `una.build/1` document then records `"model_compared": null` for each library, and so does the report. `--skip-model-check` together with `--release` is a usage error (exit code 2): a release build always compares.
    - **The record.** `una test --native` and `una build` record each result in `.una/foreign/<library>-<target>-<sha256>.json`, and the report's `model_compared` reads it for the archive whose digest it lists, with the number of calls compared. The record is a local file, informative like the rest of the report, not an attestation.

    Output, exit codes and `--json` follow `una test` (`spec/13 §5`), with each comparison reported as an example.
34. **Equivalence.** A foreign operation is a deterministic function of its arguments (FP2). The native backend refines the interpreter only relative to the premises (ADR 0109 rule 5.3): an executable that calls foreign code and the interpreter that runs the model MUST give the same output, failure, abort and trace (`spec/16 §5`). For a user's package, rule 33 tests the operations on the calls of its tests; for the repository, the `foreign` gate runs its corpus both ways.

### J. The report and the Core export

35. **The build report.** The `tcb` section gains `foreign`, after `trusted_modules`, a list in library order, one entry per library and target:

    ```json
    "foreign": [ {
      "library": "zstd", "capability": "codec.zstd.Zstd", "module": "codec.zstd",
      "model": "codec.zstd_model.zstd_model",
      "source": "vendor/zstd", "source_sha256": "<64 hex digits>",
      "target": "aarch64-apple-darwin",
      "archive": "vendor/zstd/build/aarch64-apple-darwin/libzstd_shim.a",
      "sha256": "<64 hex digits>", "bytes": 812345,
      "defined_symbols": 2, "local_symbols": 214, "imports": ["memcpy", "memset"],
      "stack_kib": 256,
      "model_compared": { "passed": true, "calls": 48 },
      "operations": [ {
        "op": "compress", "symbol": "zstd_shim_compress",
        "accepts": "internal", "result": "internal", "failure": "internal",
        "requires": ["level in 1..19", "source.len <= 67108864"], "ensures": [],
        "capacity": "source.len + source.len / 128 + 512",
        "checked_at_call": true, "interruptible": false
      } ],
      "premises": ["FP1", "FP2", "FP3", "FP4", "FP5", "FP6"]
    } ]
    ```

    - `bytes`, `defined_symbols` and `local_symbols` measure the foreign code, as `lines` measures a trusted module. `imports` lists the allowlisted names the archive uses.
    - `model_compared` is `null`, "model not compared", when `una test --native` has not run for this digest on this target, which is always the case for a target other than the host's, and after a build with `--skip-model-check`.
    - `source_sha256` is the digest of the source directory's contents (rule 7).
    - `requires` and `ensures` are the clauses as written, refinements of the parameters and the result included; `checked_at_call` is always `true` (rule 28).
    - Each entry of `trusted_modules` gains `kind`, `"policy"`, `"foreign"` or `"other"`, so the two trusted bases stay apart in the report (ADR 0109 Alternative E).
    - `out_of_model` gains, after its last entry, "foreign code beyond its premises", whenever `foreign` is not empty.

    Every key is additive, and the schema stays `una.report/2` (ADR 0076). This closes the part of issue #769 that concerns foreign code. The runtime, the codegen and the linker remain #769's.
36. **The Core export.** A `cap` declaration of a foreign capability gains `foreign`, after its operations: `{"library": ..., "symbols": {...}, "capacity": {...}, "model": ...}`, and each foreign operation carries its result and failure labels as declared. A consumer that does not know the key reads the capability as an ordinary one, whose operations' `ensures` it assumes; under `spec/13 §8` that only raises what it must trust. The formal re-check treats a foreign operation as a host operation with premises FP1 to FP6, and reports any declaration that reaches one as not re-checked until the formal track models them.

## Verification

- **Obligations.** A call of a foreign operation carries the obligations of a call of any operation: its `requires` and the refinements of its parameters (`spec/12 §3`, kind `precondition`), plus the `capacity` bound of rule 15. No new obligation kind is added.
- **Assumptions.** After the call, the verifier assumes its `ensures` and the refinement of its result, as for any operation. The runtime check of rule 28 makes them hold in every run that continues.
- **The model** is verified against the same `ensures` as any handler. A model that cannot be verified is an error of the package, so `una run` always runs code that meets the contract.
- **Elision.** A proof that rests on a foreign `ensures` may elide what ADR 0112 allows, since the fact holds after the check. No boundary check of rule 28 is itself elided.

## Information flow

- **No release.** Rules 20 to 22 make every result and failure of a foreign operation at least as high as the join of what its capability accepts. So, relative to FP1 and FP2, a foreign call is a function of its arguments whose result is labeled above all of them: the explicit-flow rule (`spec/07 §3` rule 1) applied to code the checker cannot read. Rule 19 keeps foreign code out of every release.
- **State and stale data.** Foreign code can see data from before its call in two places, and both are covered:
  - **its compartment** (rule 26): what it left there came from earlier calls of the same capability on the same worker, whose arguments some operation of the capability accepted, so rule 21 labels any result that depends on them above them;
  - **registers:** every general-purpose and vector register but the arguments in use, callee-saved ones and the frame pointer included, is zero at the call, and the state needed to return to Una is saved outside the compartment (rule 27), so no Una value or address reaches foreign code through a register or a frame walk.

  Foreign code never runs on a Una stack, so an uninitialized local reads its own compartment's data, never a Una frame's. Sharing data across capabilities needs a memory-safety violation (FP1), since each capability has its own compartment and the audit admits no writable or common symbol.
- **Program counter.** A call is an effect, so it needs `pc ⊑ accepts` (rule 20) and its failure is labeled with `pc` (rule 22). A pure foreign operation that accepts `secret` can be called in a `secret` branch, as a handler operation with the same `accepts` can.
- **Timing.** The running time of foreign code may depend on its arguments. Timing stays out of model (`spec/07 §1`). Under ADR 0116 IF5, a foreign operation's latency label `ρ(op)` includes `ℓ*` of its result and its failure, which rule 21 already places at `A` or above.
- **T6** holds relative to FP1 to FP5. FP6 (termination) is outside a termination-insensitive theorem.

## Security

**A foreign archive is trusted as the runtime is: the audit and the boundary checks detect common defects of honest code, and do not confine code written to evade them. Only the sandbox of Question 1 would.** Threats, and what stops each one:

- **Foreign code replacing the runtime's or the C library's functions.** In a static link, a global definition of `malloc`, `write`, `abort` or any other name the runtime or the C library uses would take its place, and foreign code would then run on every allocation, from process start, before any grant. The audit admits no global symbol but the declared ones, no COMDAT group, no indirect, absolute or common symbol (rule 9), and `una build` verifies after linking that no reference of the runtime or the generated code resolves into foreign sections (rule 10).
- **Code before `main`, and linker inputs.** Sections are allowed by type and flags: no initializer or finalizer of any name, no IFUNC resolver, no `.init` or `.fini`, no Objective-C metadata (rule 9). Autolinking load commands, dependent-library sections and link-time-optimization inputs are rejected, so an object cannot bring a library, a framework or uncompiled code of its own (rules 9, 10).
- **Memory corruption of the program by foreign code.** Foreign code never receives a Una object or an address outside its compartment, and runs on its own stack (rules 25 to 27). A linear overflow of a buffer is caught by its canaries at the end of the call, and an overrun of the compartment reaches a guard region. A write elsewhere in the process is not detected. **This is the residual risk of the design, and the reason foreign code is listed in the trusted base.**
- **Una data leaking to foreign code.** No Una stack, heap address or register content reaches it (rules 25 to 27), so an uninitialized read in honest C code returns its own capability's data.
- **A wrong proof becoming memory unsafety.** Every `requires` and parameter refinement is checked at run time before the call (rule 28), so a defect of the verifier, the encoding, the solver or the proof cache cannot pass foreign code arguments that its precondition excludes. This keeps ADR 0112's rule at the boundary.
- **A wrong assumption becoming a wrong proof.** `ensures` and result refinements are checked after the call, on the copied value, so a false assumption about foreign code ends the run before any code relies on it.
- **Authority.** Foreign code is reachable only through a capability that the entry receives by an explicit grant (rule 17). Every function that can reach it says so in its header, every caller of a boundary module grants it explicitly, and no policy call can reach it (rule 19). A test cannot run it in the interpreter.
- **Effects outside the model.** The import allowlist excludes every function that performs input or output, allocates, reads time or randomness, handles signals, jumps or unwinds (rule 9). Foreign code can still issue a system call with an inline instruction, jump into one inside the C library, or compute the address of a C library function from an allowlisted one plus a known offset, with no memory-safety violation (rule 18). Nothing in the first version detects these; a system-call filter around foreign calls, or the sandbox, would.
- **Hidden state.** No writable data, no thread-local data and no initializers (rule 9): foreign code keeps nothing between calls except in its own compartment, which rule 21 covers.
- **Supply chain.** Each archive is pinned by SHA-256 per target, lives inside the package with its source and recipe (rule 7), and is linked from a private copy that was hashed and audited, so a file swapped between the audit and the link is never linked (rule 10). The digest protects against a swap outside the repository, not against a change that updates the archive and its digest together: what the archive was built from is the package authors' responsibility, which the report makes visible (`docs/security/tcb.md` gap 9). Building from source with a recorded compiler (Alternative K) is the fix.
- **Symbol interposition at load time.** `LD_PRELOAD` and `DYLD_INSERT_LIBRARIES` can still replace the C library's functions, as they can for the runtime today.
- **Executable stacks and writable relocations.** Every object carries a non-executable stack note and the executable is linked with `-z noexecstack -z relro -z now`, and `una build` verifies the result (rule 10), so foreign objects cannot make Una's stacks executable or leave relocated data writable.
- **Denial of service.** Buffers and compartments are charged to the memory budget (rules 15, 26). A foreign call that runs long or does not return holds its worker, and a host stop takes effect only when it returns (rule 30); the hard stop of ADR 0116 and SIGKILL remain.
- **Code run at build time.** `una build` runs the archive in the harness of rule 33, in a separate process confined by Landlock and seccomp on Linux and by `sandbox_init` on macOS, with no network, no process creation, no file access outside a temporary directory, no inherited descriptor but its pipes, an empty environment and a time limit, and with what the build prints about a difference bounded to its location, and it refuses to run it when the confinement cannot be applied. On Linux the system-call filter closes even the inline and computed calls of rule 18 for that run. On macOS the profile is the platform's deprecated mechanism, which this record relies on until a successor exists. `--skip-model-check` runs nothing, outside release builds.
- **The audit parser.** The archive audit reads files written by the package's authors, so its parser is an attack surface of `una check` and of `una lsp`. It is a safe-Rust parser with a fuzz target (Implementation plan, step 3), and a malformed archive is U7006, never a panic.
- **The floating-point environment** is checked after each call (rule 29), as ADR 0114 requires of every code that runs in the process.
- **What the report cannot say.** The report lists foreign code by size, digest and source directory, not by audit status. Whether anyone reviewed it is outside what the toolchain knows.

## Release gates, the hunt and the oracle

- **A `foreign` release gate**, run by `scripts/release-gates.sh`:
  - builds the test archives of the conformance suite from C source committed in the repository, with the system `cc` and the recipe of rule 8, and records the compiler's version in the gate record;
  - compiles a small corpus with the same compiler and flags, and fails if its imports leave the pinned per-target allowlist (rule 9);
  - runs the must-reject cases of the audit, each U7006: a digest that differs; an archive of two objects; one built for the other target; a shared object; imports of `open`, `malloc`, `pthread_create`, `setjmp`, `fesetround` and `exp`; definitions of `malloc`, `write`, `memcpy` and `una_rt_poll`, a weak `abort`, and a hidden global that is not declared; a COMDAT group; `.data`, `.bss`, `.mydata` with `SHF_WRITE`, a TLS section, and a common symbol; `.init_array.100`, a read-only `@progbits` section named `.init_array.5`, `.ctors.100`, `.init`, `.fini`, `.tm_clone_table`, `.got` and `.plt` sections, a section of an unlisted name such as `.mytext`, an IFUNC symbol and an `IRELATIVE` relocation; an absolute symbol; a missing `.note.GNU-stack` and an executable one; `.deplibs`, `.linker-options`, `.gnu.lto_` and bitcode; on Mach-O, `__DATA,__data`, `__DATA,__foo`, an initializer under a custom name, a thread-local section, `__objc_classlist`, an `N_INDR` symbol and an `LC_LINKER_OPTION`;
  - runs the foreign conformance programs natively and in the interpreter on their models, and compares them under `spec/16 §5`, and runs `una test --native` on the test packages;
  - **canaries**, each of which MUST be caught or the gate fails: a planted write one byte past the output buffer (U6008), one before an input copy (U6008), a write past the arena into its guard region (the process ends by the signal), a deep recursion past `stack_kib` (the same), an `ensures` that the foreign code violates (U6008), a status out of range (U6008), a `Bool` result of `2` (U6008), a clobbered callee-saved register (U6008), a change of the rounding mode (U6006), a shim that returns, without saving it first, the value it finds in x19 or rbx, and one that returns its frame pointer and the caller-saved registers it does not use, each of which MUST return zero (the comparison fails otherwise), a shim that calls `abort` (the process ends by SIGABRT, with no trace file), a model that diverges from its archive by one byte (`una test --native` fails and `una build` refuses), an archive file replaced between the audit and the link (the copy is linked, the replacement never), and, for the harness of rule 33 only, test archives built outside the audit that open a socket, write a file outside the temporary directory, and start a process, each by an inline system call (the harness is killed and the comparison fails); one that reads, by an inline system call and by absolute path, a file under a `~/.ssh` directory planted for the gate, and a token file whose path the gate puts in the environment of `una build` (the read fails, the variable is absent from the harness's empty environment, and neither file's contents appear in the build's output); one that reads from a descriptor the gate leaves open and inheritable in `una build` (it is closed); one that writes a planted secret into a differing outcome (the build log shows the operation, the call index and the offset, and the secret never); one that writes more than the outcome bound (the comparison fails); and one that never returns (the time limit ends it). An honest harness under the same confinement completes, on both platforms.
- **The `sanitize` gate** (`scripts/sanitize.sh`) gains the new entry points of `ffi.rs` and the trampoline of `una-sys` under AddressSanitizer and Miri where it can run them, with corrupt words, and the test archives built with AddressSanitizer, UndefinedBehaviorSanitizer and the stack protector, run under the equivalence test.
- **The `deps` gate** covers any crate the audit adds (Implementation plan, step 3), through the supply-chain record.
- **The hunt and the oracle (deferred).** The hunt's generators and the two-run oracle (`docs/plan/security-hunt.md`, `docs/plan/security-hunt-oracle.md`) do not generate foreign capabilities in the first version, and the oracle, which runs the interpreter, sees only the models. The checker rules of this record (U2036, U3030, U3031, U4009) join the conformance suite with their implementation. The pre-v1.0 hardening pass adds:
  - a generator family for foreign capabilities, with labels, grants, boundaries and policy calls, for the must-reject drivers;
  - a differential fuzz target per foreign operation of the test archives, native against the model, on generated arguments;
  - the oracle's native sample (#792) on programs that call foreign operations with varied secrets;
  - MemorySanitizer and LeakSanitizer on Linux for the test archives;
  - the sandbox evaluation of Question 1, a v1.0 blocker, and a system-call filter around foreign calls.

## Costs

| Operation | Time | Space |
| --- | --- | --- |
| A foreign call | the foreign code's own time, plus O(Σ input lengths) to copy in, O(output length) to copy out, O(1) to zero the output (fresh pages when large, or O(capacity) for a small one), O(1) for canaries, the trampoline, register checks and the control register, and the clause checks below | the input copies and the output buffer in the arena, during the call |
| Clause checks of one call | O(1) for a clause of scalars, `.len` and arithmetic; O(n) for one that calls a `reflect` or opaque predicate that walks a list of n elements, or for a refined element type (ADR 0103) such as `List[U8 in 0..127]`, checked on each of n elements; a predicate's own cost otherwise, as for `checked` | the predicate's own |
| The first call of a capability on a worker | one mapping of the compartment, O(1) | `stack_kib` plus the arena, charged to the budget |
| The archive audit | O(archive size) to hash and parse, and O(s log s) for the s symbols, kept in ordered sets | O(archive size) to hold the bytes, O(s) for the symbols |
| Verifying the executable | O(size of the map and the executable's headers) | O(sections) |
| `una test --native` | the tests' run, plus one replay per recorded call, one link of the harness, and O(1) to confine it | the call log, O(total size of the recorded arguments and outcomes) |
| Checking a foreign declaration, and the policy-call check of rule 19 | linear in its size, and the reachability of `spec/07 §6`, computed once over the call graph | none beyond it |

- The copies make a foreign call on a large buffer cost a pass over its bytes on top of the foreign work. For the target uses, compression, codecs and checksums, the foreign work is itself at least linear in the input, so the copy is a constant factor. Zero-copy passing is Alternative H.
- The audit runs on every check of a package with `[foreign]`. A cache keyed by the archive's device, inode, size, modification time and digest MAY skip it when all are unchanged and the allowlist has not changed; `una lsp` uses it, so an editor does not rehash up to 256 MiB on each keystroke. `una build` never uses it: it hashes and audits its private copy.
- Programs without foreign code pay nothing.

## Diagnostics

New codes, which join `spec/13 §4` with their implementation. The highest codes in use or reserved are U2035 (ADR 0117), U3029 (ADR 0116), U4008, U6007 (ADR 0116) and U7005; the gaps U4006 and U6002 stay unassigned (ADR 0012).

| Code | Domain | Meaning |
| --- | --- | --- |
| U2036 | types | A foreign operation with a type, parameter or clause the boundary does not allow: a parameter or result outside rule 11, a label on a parameter type, an `edit` or `take` parameter, a type parameter, more arguments than the registers of rule 12, an error kind with payloads or more than 255 variants, `capacity` missing for a `List[U8]` result or present otherwise, or a `symbol` that is not a C identifier, is reserved, or is named twice |
| U3030 | authority | A foreign capability declared where it is not allowed: in an `app` package, outside a trusted module, in a module that also declares a policy or another foreign capability, or with a `library` that is malformed or that no `[foreign]` table assigns to its module |
| U3031 | authority | A foreign capability used where it is not allowed: a policy call whose `via` can reach a foreign operation or binding, or a function value of unknown code when the package declares a foreign capability (rule 19); a `via` function whose `uses` names a foreign capability; a model with parameters or `uses` |
| U4009 | security | A foreign operation without `accepts`, or whose result type does not write its label, or whose result label is below the join of its capability's `accepts` labels |
| U6008 | runtime | A foreign call broke the boundary: a `requires`, parameter refinement or `capacity` check failed before it, or after it a canary changed, a status, length or `Bool` was out of range, a callee-saved register or the stack pointer differed, or an `ensures` or result refinement was false |
| U7006 | project | A foreign archive rejected: missing, outside the package, a digest that differs, not an archive of one relocatable object for its target, a section, symbol, relocation, load command or import that rule 9 does not allow; at `una build`, no archive for the target, a linked executable that fails the verification of rule 10, or a comparison with the model under `una test --native` that fails, does not finish in its time limit, or cannot be confined |

Messages, one sentence each: "foreign capabilities are allowed only in `systems` packages" (U3030); "`compress` must state the label it accepts" (U4009); "the result of `bound` is labeled `public`, below `internal`, which `compress` accepts" (U4009); "this policy call can reach the foreign operation `Zstd.compress` through `@codec`" (U3031); "the foreign call `Zstd.compress` wrote past its output buffer" (U6008); "`vendor/zstd/build/aarch64-apple-darwin/libzstd_shim.a` defines `malloc`, which only the declared symbols may be" (U7006); "`zstd` disagrees with its model on `compress`" (U7006).

Existing codes cover the rest:
- **U0101:** a foreign capability in an edition that does not admit it, and a malformed declaration.
- **U3010:** a model that is not a handler of its capability.
- **U3012:** a `uses` of a foreign capability in the entry that no grant allows.
- **U3014:** a test or example that needs a foreign capability without a `given`.
- **U4002, U4003:** calls and failures under a raised program counter (rules 20, 22).
- **U5001, U5002:** the `requires`, refinements and `capacity` bound at call sites.
- **U6003:** a compartment or buffers over the memory budget.
- **U6006:** a floating-point environment changed by foreign code, at the call (Appendix A).
- **U7002:** an invalid `[foreign]` table, more than 32 of them, or `[foreign]` in an `app` package or an edition that does not admit it.

Usage errors, exit code 2, as for the flags of `spec/13 §5`: `--model-check-timeout` that is not a number from 1 to 3600, and `--skip-model-check` together with `--release`.

`una explain` gives each new code a paragraph and a failing and fixed example; U6008 and U7006 come from foreign code or files rather than program text, so, as for U6005, their texts say how to fix the archive.

## Consequences

- **What can be written.** `systems` packages can call C implementations, built by their own authors, of pure computations on scalars and byte buffers, at native speed apart from the copies, behind a capability that every caller and every run sees.
- **What cannot yet.** Foreign code that performs input or output, allocates, keeps state, uses the C math library, takes callbacks, passes structured data, or implements a declassification. Most C libraries need a shim and the recipe of rule 8, and those that allocate need their allocation replaced by scratch buffers.
- **One label per library.** Every result of a library carries the highest `accepts` among its operations (rule 21). Operations that need different result labels go in different libraries, each with its own archive and compartment.
- **A second implementation.** Every foreign capability has a Una model that must match it bit for bit, and `una build` refuses an archive that disagrees on the package's tests. It costs writing the computation twice, and gives the interpreter, the tests and the comparison something to run.
- **The trusted base grows** by each archive a package links, which the report measures, and by the trampoline, compartment mapping and harness confinement in `una-sys` and the boundary's entry points in `ffi.rs`, under ADR 0058's rules.
- **Building runs code.** Building a package with foreign code runs its archive once, confined, in the harness of rule 33, where building used to run nothing. `--skip-model-check` avoids it outside release builds.
- **`docs/security/tcb.md`** gains foreign code as a component before this record is accepted, as its last paragraph requires (Implementation plan, step 0).
- **Records to amend** on acceptance: ADRs 0058, 0093, 0114 and 0116 (Appendix A); and, under Question 5 (a), `spec/13 §7` through ADR 0056's process. The same question for ADR 0116's `for ever` is raised on that record's own issue.
- **Deferred:** a sandbox (a v1.0 blocker to evaluate); allocation routed to the budget; callbacks; structured types and `Text` results; the C math library; zero-copy buffers; foreign code in `via` functions; builds from source; dynamic libraries, never in the first version; foreign code in package dependencies, since the language has none yet.

## Alternatives considered

- **A. Foreign functions as plain functions,** `extern fn` with no capability. Not chosen: it is ambient authority. Any function could call foreign code without saying so in its header, and no run would grant it.
- **B. Foreign functions that declare the host capabilities they use,** such as `uses @files: Files[read]`. Not chosen: a grant's path prefix and mode cannot confine C code, so the header and the report would claim a confinement that does not exist. The first version admits no effects instead, and a foreign capability is the authority to run its library.
- **C. Dynamic linking and system libraries** (`-lssl`, `dlopen`). Not chosen: the code that runs depends on the machine and the loader, is not pinned by the package, and can be interposed.
- **D. A helper process with a system-call filter.** Strong isolation of memory and effects. Not chosen for the first version: each call crosses a process boundary, tens of microseconds, and the helper adds its own protocol to the trusted base.
- **E. WebAssembly in an embedded engine.** Memory safety and determinism by construction, fuel for termination, and confinement of a malicious archive. Not chosen for the first version: the engine, with its own `unsafe` code, enters the trusted base, and its slowdown against native code works against the purpose of `systems` code. Its evaluation is a v1.0 blocker with the measured criterion of Question 1.
- **F. A routed allocator.** `malloc` resolved to a runtime function that charges the memory budget. Deferred (Question 3): it admits state across calls, and renaming symbols needs the machinery whose risks the symbol rules of rule 9 close.
- **G. Callbacks into Una.** Not chosen: a callback would run Una code on a stack of foreign frames, with foreign state live, and would need reentrancy rules for the runtime.
- **H. Zero-copy buffers** into the Una heap. Not chosen: a foreign write past a buffer would corrupt Una values or the runtime's tables (ADR 0107), and a retained address would outlive its object. The copy costs one pass.
- **I. Trusting `requires` and `ensures` without run-time checks.** Not chosen: a defect of the verifier would then pass foreign code arguments it was not written for, and a memory-safety defect would rest on a proof, against ADR 0112.
- **J. Foreign capabilities without a model,** with `una run` refusing them. Not chosen (Question 2): the interpreter is the reference of both profiles (ADR 0109), and without a model a foreign capability would have no reference and no comparison.
- **K. Building archives from source in `una build`.** Deferred: the C compiler and its flags would become build inputs, which ADR 0106's compiler matrix would have to cover. It is the fix for provenance, and until it exists the first version admits only archives that the package's authors build from source kept in the package.
- **L. Foreign code under the current terms of a preview edition.** Not possible: a preview may only narrow what a program may do (`spec/13 §7`), and foreign code widens it. Question 5 asks whether to amend that rule for constructs behind their own opt-in, or to wait for a full edition; this record decides neither.
- **M. Foreign code in `[trusted]` modules beside policies.** Not chosen: one module would then hold two trusted bases with different rules, which ADR 0109 Alternative E warns against.
- **N. Running foreign code on the caller's stack,** with a check of the stack left. Not chosen: earlier Una frames, and under ADR 0117 other tasks, leave data below the stack pointer, which an uninitialized read in honest C code would return with a lower label, and ADR 0117's task stacks are sized for a depth of Una calls, not for C frames.
- **O. Hiding symbols by visibility only.** Not chosen: hidden globals still resolve across objects in a static link, so an archive could still replace a runtime or C library function. The recipe pre-links and localizes, and the audit checks the outcome.
- **P. Comparing the model with the archive outside `una build`,** only when a developer runs `una test --native`. Not chosen: the comparison would be optional in practice, and an executable could ship with a model that the interpreter, the tests and the oracle run but the archive does not match. Running the archive at build time is the cost, which the confinement of rule 33 bounds, and `--skip-model-check` avoids outside release builds.

## Implementation plan

For issue #33, in order. No step starts before this record is accepted, and before Question 5 is settled.

0. **The risk profile.** `docs/security/tcb.md` gains foreign code as a component, with FP1 to FP6, the gaps of Security, and the sentence that a foreign archive is trusted as the runtime is and that the checks do not confine code written to evade them. Done before acceptance.
1. **Manifest and syntax.** The `[foreign]` table and its U7002 checks; `foreign capability`, `library`, `symbol` and `capacity` in the edition that admits them; the canonical form; the grant spec `foreign:<library>`.
2. **The checker.** U3030 (placement), U2036 (types, registers and clauses), U4009 and rules 21 and 22 (labels), U3031 (the policy-call reachability of rule 19, the `via` `uses`, the model's shape), U3012 and U3014 for foreign capabilities, the `capacity` obligation, and the Core export of rule 36.
3. **The archive audit.** A parser of `ar`, ELF and Mach-O relocatable objects in safe Rust, either written in the repository or a crate added through the supply-chain record and the `deps` gate, with a fuzz target; the descriptor-relative read; the digest, format, sections, load commands, symbols, relocations and imports of rule 9; the per-target allowlists pinned from the gate's corpus; U7006; the report's `tcb.foreign` and `trusted_modules[].kind`.
4. **The interpreter.** The model for `una run`, the `note:` line, and `foreign_models` in `una.run/1`.
5. **`una-sys`, behind its `foreign` feature.** The compartment's mapping with guard regions, the trampoline of rule 27 per target, and the harness confinement of rule 33, under the amendment of ADR 0058 in Appendix A (about 580 lines), with its security review, stress tests, and the test that keeps the feature out of the interpreter's build.
6. **The native backend.** The private copy, the hardened link and the verification of rule 10; the argument block and the call through the trampoline, emitted by `una-codegen`; the boundary entry points in `ffi.rs` (arena, canaries, budget, the control register); the compiled clause checks of rule 28; U6008. A security review of every change to `ffi.rs`, as ADR 0058 requires.
7. **`una test --native`.** The call log, the harness executable, its confinement and time limit, the replay and comparison, the record and `model_compared` in the report, the refusal of `una build`, and `--skip-model-check` with its refusal under `--release`. Notes for this step:
   - **Memory on macOS.** macOS does not enforce `RLIMIT_AS`, so `una build` watches the harness's resident memory there and kills it past the budget, as the time limit kills it past its time; the comparison then fails.
   - **Room under the address-space limit.** `RLIMIT_AS` includes the guard regions and the arena's reserved address space, besides the budget, so that an honest harness never fails closed on address space it only reserves.
   - **macOS allowances.** The operations that the honest-harness test finds necessary stay within system paths; none names the user's home directory or the build's working directory, and the chapter's list says so.
   - **One call per operation.** For each operation, only its first differing call is printed, which also narrows the offset channel.
8. **Gates.** The `foreign` gate, its test archives, corpus, must-reject cases and canaries, and the additions to `sanitize`.
9. **The specification.** The new chapter with the recipe and the allowlists, the amendments listed in the header, and `explain` texts for the six codes.
10. **The formal track.** Foreign operations as host operations with premises; until then, a declaration that reaches one is not re-checked.

## Conformance

- **Parse and format:** a foreign capability with each clause, in the edition that admits it; U0101 in editions `0.1` to `0.3`; `foreign`, `library`, `symbol` and `capacity` as identifiers elsewhere; the canonical order.
- **Placement:** U3030 in an `app` package, in a module not listed in `[trusted]`, in a module without `trusted module`, beside a policy, as a second foreign capability of a module, with a malformed `library`, and with a `library` that no table assigns to its module; U7002 for a `[foreign]` table in an `app` package and for a 33rd table.
- **Types:** U2036 for `Int`, `Text`, a kind, `Option`, `List[I32]` and a function type as parameters and results; a labeled parameter; an `edit` parameter; a type parameter; six `List[U8]` parameters and a `List[U8]` result (seven registers); a payload in the error kind; `capacity` missing and superfluous; a symbol `una_x`, a symbol `memcpy`, and a symbol named twice.
- **Labels:** U4009 for a missing `accepts`, an unlabeled result, and a `public` result in a capability where another operation accepts `internal`; U4003 for a `secret` argument to an operation that accepts `internal`; U4002 for `if error pass` on the failure of an operation whose capability accepts `internal`; a `secret`-accepting operation called in a `secret` branch, accepted.
- **Authority:** U3012 without the grant, and the run with it; U3014 in a test without `given`, and the test with the model as `given`; U3021 for a call into a boundary module whose ceiling lacks the foreign capability.
- **Declassification (rule 19), each U3031:** a `via` that performs a foreign operation; one that calls a helper that does; one that receives the foreign binding from its caller by name; one whose caller rebinds a user capability to a handler whose `uses` name the foreign capability; one with such a handler installed outside the policy call and granted on by name; one given a lambda that calls a foreign operation; one given a function value of unknown code in a package that declares a foreign capability; a `via` whose `uses` name a foreign capability. And the same function value accepted in a package without one.
- **Models:** U3031 for a model with parameters and for one with `uses`; U3010 for a model of another capability.
- **Verification:** a `requires` refuted at a call (U5001); a `capacity` that may exceed its bound (U5001); a fact of `ensures` used after the call; a model refuted against `ensures` (U5001).
- **Manifest and audit:** U7002 for each invalid key of `[foreign]`; U7006 for each must-reject archive of the `foreign` gate, and for `una build` on a target without an archive.
- **Runs, natively and in the interpreter on the model, compared:** a call per scalar type and per buffer shape; a failure of each variant; an empty input and an empty output; a capacity of zero; six `I8` arguments extended by the caller; an operation with two inputs and an output, and one with three inputs and an output; a compartment over the memory budget (U6003); each canary of the gate (U6008, U6006 or the signal, native only).
- **`una test --native`:** a build with `--skip-model-check`, with `model_compared` null in the report, and the flag with `--release` as a usage error; a `--model-check-timeout` of 0 as a usage error; a package whose model matches, with `model_compared` in the report; one whose model differs by one byte (a failed test, and `una build` refuses with U7006); one with an operation no test calls (a failed test).
- **Report:** `tcb.foreign` for a package with two targets; `trusted_modules[].kind`; `out_of_model` with and without foreign code; `una.run/1` with `foreign_models`.

## Appendix A. Amendments to accepted records, to be appended on acceptance

**ADR 0058, Amendment.** `una-sys` (ADR 0058 amendment of ADRs 0116 and 0117) also holds, behind a Cargo feature `foreign` that only `una-rt` enables, the code of ADR 0118:
- the mapping of each compartment's stack and buffer arena with its guard regions, and the arena's growth and shrinking: about 120 lines;
- per target, the trampoline of ADR 0118 rule 27, which saves the Una state outside the compartment, switches stacks, zeroes every register but the arguments, calls the foreign symbol, checks the callee-saved registers, the frame pointer and the stack pointer, restores and switches back: about 90 lines for AArch64, shared by `aarch64-apple-darwin` and `aarch64-unknown-linux-gnu` apart from the handling of x18, and about 80 for x86-64;
- the confinement of the harness of ADR 0118 rule 33: the Landlock ruleset and the seccomp filter on Linux, about 150 lines, and the `sandbox_init` profile on macOS, about 40;
- the safe interface and its `// SAFETY:` comments: about 100 lines.

That is about 580 lines, so the crate's limit rises from 800 to 1,400 lines. The interpreter, `una-interp`, and `una-cli` do not enable the feature, so they carry none of this code, and a test checks each shipped binary's package built on its own, never a workspace build, whose feature unification would mix them: `cargo tree -p una-cli -e features` and the same for `una-interp`, failing if `foreign` is active. The release scripts build each binary by its package, as they do today. Every change to the crate still gets a security review before it merges.

**ADR 0093, Amendment.** Rule 15 says that a binding of an example's own context or of the `[run]` entry's "is a host capability, which runs no code of the package". A binding of a foreign capability that the entry receives (ADR 0118 rule 17) runs code of the package: its archive. It is not a handler, and no Una code of the package runs through it, so the rule's conclusion for ceilings is unchanged; the sentence reads "is a host capability or a foreign capability, which runs no Una code of the package".

**ADR 0114, Amendment.** Una gains a foreign function interface (ADR 0118), so the sentence of rule 6.4 "Una has no foreign function interface" no longer holds.
- Foreign code may run only in native executables of `systems` packages. After each foreign call, the runtime reads the floating-point control register and ends the run with U6006 when its control fields differ from the default environment of rule 6.4, and it clears the status flags (ADR 0118 rule 29). The x87 control word is not checked, since no Una code and no allowlisted import performs x87 arithmetic.
- U6006's row gains this second case: a mid-run abort at the foreign call, with its source location, exit code 1 and the trace written, besides the probe failure before the program starts.
- The scope of rule 6.4, "edition `0.3` only", becomes edition `0.3` and every later edition, in which the write, the self-test and this check apply.
- The import allowlist of ADR 0118 rule 9 contains no function that changes the environment. The interpreter never runs foreign code.

**ADR 0116, Amendment.**
- The handlers of rule 18 are installed with `SA_ONSTACK`, and each thread that may run Una code has an alternate signal stack, so a signal that arrives during a foreign call (ADR 0118) or deep in a coroutine's stack runs on a stack of known size.

The edition question that `for ever` shares with foreign code (ADR 0118 Question 5) is not amended here: it is raised on ADR 0116's own issue, and ADR 0118 only cites the outcome.

**`spec/16 §3` rule 3, Amendment.** "The executable never overflows its stack" holds for Una code. Foreign code runs on its compartment's stack (ADR 0118 rule 26); if it exceeds it despite the stack probes the recipe requires, it reaches a guard region and the process ends by the signal, outside the Una Abstract Machine, as ADR 0118 rule 29 states for any crash of foreign code.
