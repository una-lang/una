# Inventory: C-emission text in accepted records and documents

- Issues: #1311 (this inventory), #1309 (ADR 0112's amendment); epic #1097; follows #1099.
- Source: ADR 0106, amendment of 2026-10-31 ("the C-emitting backend is withdrawn"): §1 (withdrawn), §2 (what survives), §3 (requirements RP-1 to RP-10), §4 (open questions OQ-1 to OQ-8), §5 (effect on other records).
- Date: 2026-10-31. Line numbers are those of `main` before the dated notes below were appended; a note appended at the end of a record moves no earlier line.

ADR 0106's amendment of 2026-10-31 withdraws the route that compiled emitted C with the system C compiler. The release path of `una build --release` becomes Una's own optimizing code generation over Core IR, inside `una`. Several accepted records and documents still describe release builds in terms of emitted C, the system C compiler, C flags or C vector types. Accepted records are never edited: each keeps its text, and a dated note appended at its end says which passages are read through the amendment. This page lists every such passage, what it says, and how it reads.

## 1. Scope

Listed: a passage that describes the release build as emitted C, names the system C compiler as the release compiler, prescribes or forbids C compiler flags, prescribes C constructs, types or headers in generated code, or names C vector types.

Not listed, after checking:

- **The system `cc` as the linker** of `una build` (ADR 0055 NB-1, `spec/16 §1` rule 1, `docs/guide/getting-started.md` line 15, `docs/foreign-code-inventory.md` lines 19 and 102, ADR 0124 rule 22 and Question 1's history of `cc`). It links the Cranelift object and is not C emission; ADR 0124's own linker removes it (#1104).
- **C as a language for comparison or measurement:** ADR 0096 line 205 (C's `%`), ADR 0058 line 36 (a runtime written in C, an alternative not chosen), ADR 0114 lines 440 and 441 (candidate math libraries written in C, not adopted), ADR 0120 lines 699 and 700 (C as a benchmark baseline), and ADR 0124 line 202 (`-ffp-contract=off` for building the Z3 library, which is not a release build of a Una program).
- **ADR 0114 line 158**, "a `volatile` read in C", which explains `core::hint::black_box` by analogy and prescribes nothing about generated code.
- **ADR 0055 rule 8 and its Alternatives** ("an LLVM backend for release builds comes later"): not C emission, and already amended by ADR 0106 (its header: "Amends: ADR 0055 rule 8"). ADR 0106's amendment, §1, keeps LLVM rejected as foreign code inside `una`.
- **ADR 0106 itself**: its amendment of 2026-10-31 lists every C part of its own text in §1.

## 2. Inventory

"Reads as" is the meaning the passage has through ADR 0106's amendment of 2026-10-31 ("the amendment" below). No rule changes: each row restates a mechanism or a reason, never which checks exist or which programs are accepted.

### ADR 0107, data layout

| Line | Rule | What the text says | Reads as |
| --- | --- | --- | --- |
| 7 | Header, Target | "its optimized backend (ADR 0106, #522)" | The release path of the amendment, §3. |
| 31 | Context | ADR 0106 "adds an optimized backend that emits C, compiled with `-fwrapv -fno-strict-aliasing`"; record passing "fixes the C signatures" | The release path; RB-2's C subset is withdrawn (amendment §1) and its intent is RP-5 and RP-7; the boundary fixes the release path's calling convention (RP-2). |
| 43 | Rule 1 | "the Cranelift backend and the C backend derive the same one" | Cranelift and the release path derive the same representation (RP-2). |
| 67 | Rule 2 | an optimizer may reuse a value only where "Cranelift's IR, or C without undefined behavior" allows; cross-language link-time optimization | The release representation, defined at every input (RP-5). The release path is inside `una` (RP-1), so no cross-language link-time optimization exists; the rule on recomputing addresses binds it unchanged (amendment §2, RP-7). |
| 74 | Rule 2 | "the C backend reads them through struct types from one generated header"; `_Static_assert` in the header | The layout facts are asserted on the runtime's side and in the code generator, and tested against each other (RP-7). |
| 126 | Rule 6 | loop state in "local variables (C)" | The release representation's own values; the form is the design record's (#1305). |
| 127 | Rule 6 | several results "as a returned struct by value (C)" | The multiple-result return of the release path's convention, with the failure tag and payload (RP-2). |
| 136 to 140 | Rule 7 | "ADR 0106 needs these fixed, because they are the C signatures"; `int8_t` to `uint64_t`, `int64_t`; "never passed by value as a C struct" | The calling convention of every native code path, the release path included (RP-2); the C type names name machine types of that width and signedness. |
| 180 | Rule 11, memory safety | "The C backend never dereferences a handle without its checks" | The release path never does (RP-7, RB-3a). |
| 191 | Rule 11, tests | a native test "built with `-fsanitize=address` in the C backend's sanitizer gate (ADR 0106)" | The sanitizer run over emitted C is withdrawn (amendment §1); its replacement is OQ-8 (#1307). The `sanitize` gate over the runtime is unchanged. |
| 193 | Rule 11, tests | static assertions "compile in both the runtime and the generated header" | RP-7: runtime and code generator assert the same sizes and offsets, and a test compares them. |
| 202 to 237 | Evidence | C programs emulate the layouts (`docs/adr/evidence/0107/c/`, `c/k3_layout.c` among them) | Measurement records only (§4 below). |
| 235 | Evidence | gains "show in Una's executables only once a backend emits that code, which for the optimized backend is #522 (ADR 0106)" | Once the release path generates that code (#1305, measured by #1306). |
| 259 | Consequences | "the C backend of ADR 0106 can compile field access, fixed-width arithmetic and loop state into plain C" | The release path can compile them into machine code without runtime calls. |
| 274 | Alternative D | "Pass records by value as C structs" | Records passed by value as aggregates in any backend's convention; stays rejected. |

### ADR 0108, small records off the heap

| Line | Rule | What the text says | Reads as |
| --- | --- | --- | --- |
| 44 | Rule 2 | cells held as "C locals" | The release representation's own values. |
| 45 | Rule 2 | a flat record "is never a C struct parameter, so the C signatures stay scalar" | The release path's signatures stay scalar: cells as separate arguments (RP-2). |
| 135 to 166 | Evidence | kernels in C, `cc -O2 -fwrapv -fno-strict-aliasing -fno-delete-null-pointer-checks`; "the C compiler reduces the loop"; numbers show "once a backend emits this code (#522)" | Measurement records (§4 below); the gains show once the release path generates that code (#1305, #1306). |
| 188 | Consequences | "C signatures grow by up to 4 scalar parameters per flat record, which ADR 0106 states as part of the boundary contract" | The release path's signatures grow so (RP-2). |

### ADR 0109, profiles `app` and `systems`

| Line | Rule | What the text says | Reads as |
| --- | --- | --- | --- |
| 69 | Rule 3, what a wrong proof costs | "the optimized backend's arithmetic is defined at every input (ADR 0106, RB-7)" | RB-7 survives (amendment §2) and RP-5 requires it of every representation of the release path; the sentence holds unchanged. |

### ADR 0112, eliding the checks the verifier proves

Restated by its own amendment of 2026-10-31 (#1309), item by item:

| Line | Rule | What the text says | Reads as |
| --- | --- | --- | --- |
| 7 | Header, Target | "the release backend of ADR 0106 only" | ADR 0112 amendment, item 1. |
| 12 | Context | "before C is emitted" | Item 1: before machine code is generated. |
| 30 | EL-1 | "in the C that ADR 0106 emits, before it is emitted" | Item 2: on Core IR or the release representation, before machine code, at a point where each check carries its Core IR site. |
| 39, 41 | EL-2 | "the unsigned form of RB-7"; retag `((uint64_t)r << 1) \| 1` | Item 3: 64-bit (or width-w) wrapping operations and a logical shift, defined at every input. |
| 45 | EL-2, division | "In C, both cases are undefined behavior" | Item 4: the rule stands; reason restated under RP-5 and the targets' division instructions. |
| 48 | EL-2, `when` | "its only removal form, `__builtin_unreachable`" | Item 5: an "unreachable" construct, excluded by RP-5. |
| 77 to 83 | EL-5 | "Before the C compiler runs"; present checks "read back from the C itself" through a marker | Item 6: read back from the release representation or the machine code, by a separate pass; a fourth set only if the design states removal rules (OQ-4). |
| 87, 88 | EL-6 | "Before the C"; `__builtin_assume`, `__builtin_unreachable`, `restrict` | Item 7: an elision adds no fact; RP-5 excludes every construct with an undefined false premise. |
| 92 | EL-6, `check-survival` | harness builds; the stale-handle trigger of ADR 0106 G2 (a C harness) | Item 7: test builds of the release path; the stale-handle trigger is #1307's. |
| 105 | §8, `elision-record` | "a test build of the emitter" | Item 8: of the code generator. |
| 107 | §8 | "the `sanitize` gate's flags report nothing on it" | Item 8: the evidence that replaces the sanitizer run over generated code (OQ-8, #1307). |
| 131 | Consequences | "the kernels of ADR 0106 and ADR 0107" | Item 10: measured in Una through the release path (#1306). |
| 142 | Alternative C | "Tell the C compiler the proven facts with `__builtin_assume`" | Item 9: stays rejected (RB-7, RP-5). |
| 148 | Alternative F | "in C, a zero divisor or `MIN / -1` is undefined behavior" | Item 4: stays rejected, for the reason restated there. |

### ADR 0113, the copies of a `together` block

| Line | Rule | What the text says | Reads as |
| --- | --- | --- | --- |
| 96 | Rule 5, the portable form's nodes | "ADR 0107's slot layout, ADR 0106's C backend, and any code that inlines a retain or a release keep it" | The release path keeps it: 32-bit counts and marks, every increment compared with 2^32 − 1. |

### ADR 0114, floating point `F64`

| Line | Rule | What the text says | Reads as |
| --- | --- | --- | --- |
| 115 | Rule 4.2 | a float-to-integer conversion "is undefined behavior in C ... (C11 6.3.1.4)"; "The C backend states this guard in the form RB-7 of ADR 0106 requires" | The guard stays on every backend; in the release path the conversion is behind it, defined at every input (amendment §2, rule 6 of the floating-point amendment; RP-5, RP-6). |
| 153 | Rule 6.2 | "The C backend of ADR 0106 compiles with the flags and the canary of Appendix C" | No contraction outside a written `fma`, no reassociation (RP-6); the C flags and canary are withdrawn (amendment §1); their replacement is #1308. |
| 174 | Rule 6.5 | "The C backend: no `#include <math.h>` in the emitted C; `-fno-math-errno`; `sqrt` as `__builtin_sqrt`" | No platform `libm` (RP-6): `sqrt` correctly rounded, every other exact operation a routine of `una-rt` (amendment §2). |
| 324 | Rule 9.1 | an `F64` is passed "as a Cranelift `f64` and a C `double`" | A binary64 value in a floating-point register in every native code path. |
| 420 | Rule 16.8 | "The C backend (Appendix C): the contraction canary ..., the flag probe, and the check of the backend's command line" | Withdrawn with the C route (amendment §1); the evidence for floating point in release builds is #1308 and OQ-8 (#1307). The FMA disassembly gate stays (amendment §2). |
| 458 | Consequences | "the release backend gains flags, a canary and a probe" | The release path carries RP-6's evidence (#1308). |
| 503 to 523 | Appendix C | the amendment to ADR 0106 for floating point | As the amendment of 2026-10-31 divides it: rules 1 to 4, rule 5's `__builtin_sqrt`, rule 7's canary built with `-ffp-contract` and rule 8's flags withdrawn; rule 5's routines, rule 6 and rule 9 survive; intent RP-6. |

### ADR 0120, the numeric foundation

| Line | Rule | What the text says | Reads as |
| --- | --- | --- | --- |
| 140 | Context, FMA in release executables | Appendix C "passes no flag that enables FMA on x86-64, and its rule 7 disassembles a release executable" | RP-6: FMA instructions only inside the allowlist; the disassembly gate stays (`docs/release-gates.md`, amendment §2). |
| 197 | Rule 9 | "`float` in the C backend under `FLT_EVAL_METHOD == 0`" | Binary32 computed in its own precision (RP-6). |
| 509 | Rule 58, lowering | "variant of each such loop with GCC and Clang vector types, under `__attribute__((target("...")))`" | How `simd required` lowers in the release path is OQ-6 (#1308); rules 56 to 58 on targets, features and dispatch stand. |
| 782 | Implementation plan, step 8 | "with the vector forms of Appendix A's amendment of ADR 0106" | With the lowering OQ-6 decides (#1308). |
| 816 | Appendix A, ADR 0106 item 1 | RB-2's form 3 extends to dense headers | Survives (amendment §2). |
| 817 | Appendix A, ADR 0106 item 2 | "the emitted C MAY use GCC and Clang vector types ...", `__builtin_cpu_supports` | Withdrawn (amendment §1); OQ-6. |
| 818 onward | Appendix A, ADR 0106 items 3 and 4 | the FMA allowlist; no contraction outside a written `fma` | Survive (amendment §2). |

### ADR 0124, one binary

| Line | Rule | What the text says | Reads as |
| --- | --- | --- | --- |
| 11 | Header, relations | RB-1 "compiles emitted C with the system C compiler"; where that compiler comes from is for ADR 0106's amendment | Decided: no C compiler; the release path is inside `una` (amendment §5). |
| 40, 41 | Question 1 | the direction "a pinned clang built into `una`", kept as history; "release builds, once implemented, need a C compiler" | History only; release builds need nothing beyond `una` (RP-1). |
| 181, 186 | Context; Departures 1 | the release compiler left to an amendment of ADR 0106 | Closed by that amendment (§5). |
| 243 | Rule 9 | "The release compiler, if ADR 0106's amendment builds one in", run as `una __cc1` | No subject: no release compiler is built in (amendment §5). |
| 282 | Rule 22, the linker's inputs | "under ADR 0106, the release compiler's object" | The release path's object (amendment §5). |
| 462 | Consequences | "No external tool ... except ... until ADR 0106's amendment, the C compiler of release builds" | No external tool is run for release builds either (RP-1). |
| 498 | Implementation plan | "The release compiler (Question 1) follows ADR 0106's amendment" | Closed: the release path is #1305's design. |

### Documents

| File | Line | What the text says | Status |
| --- | --- | --- | --- |
| `docs/security/tcb.md` | 223 to 237 | §5 "Five decisions are proposed and not accepted", and "ADR 0106 (proposed)" in the ADR 0109 item | Stale: all five are accepted. Fixed in this change (§5 rewritten as accepted decisions, with their state). |
| `docs/security/tcb.md` | 241, 243, 247 | gaps 1, 3 and 7 | Already point at the amendment (#1099); unchanged. |
| `docs/foreign-code-inventory.md` | 20, 103 | the C-emitting release backend | Already withdrawn there (#1099); unchanged. |
| `docs/plan/backlog.md` | 11, 70 | ADR 0106 chose C | Already point at the amendment (#1099); unchanged. |
| `docs/plan/roadmap.md` | 30 | optimized release builds | Already points at the amendment (#1099); unchanged. |
| `docs/plan/0124-step3-design.md` | 16 | `una build --release` (ADR 0106, withdrawn by #1099) | Already correct; unchanged. |
| `docs/plan/0120-s6-design.md` | 13 | "There is no C release backend" | Still true; unchanged. |
| `docs/plan/0120-s6-design.md` | 93 | out of step: "the C release backend's vector forms" | Edited in this change: the lowering of `simd required` in the release path (OQ-6, #1308). |
| `docs/release-gates.md` | 52 | the FMA disassembly gate, citing ADR 0114 Appendix C rules 7 and 8 | The gate is kept by the amendment (§2); a pointer to it added in this change. |
| `docs/spec/16-native-backend.md` | whole | describes the Cranelift build only; no C emission | Confirmed: no change now. Release builds get normative text with the design record (#1305), as the amendment's §5 states. |

## 3. Dated notes appended to the records

Each note points at the amendment and names the passages read through it. None changes a rule. The text appended to each record:

- **ADR 0107:**

  > ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend: the release path of `una build --release` is Una's own code generation over Core IR, under that amendment's requirements RP-1 to RP-10. No rule of this record changes, and rules 1 to 7 bind the release path as they bind Cranelift (RP-2, RP-7). Its C passages read through that amendment: the Target line and the Context (an optimized backend "that emits C"), rule 1's and rule 11's "C backend", rule 2's "C without undefined behavior" and cross-language link-time optimization (the release representation, defined at every input, RP-5), rule 2's generated header and `_Static_assert` and rule 11's layout-assertion test (the layout facts asserted on the runtime's side and in the code generator and tested against each other, RP-7), rule 6's C locals and returned struct, rule 7's "C signatures" and C type names (the release path's calling convention, with machine types of those widths), rule 11's sanitizer test over the C backend (OQ-8, #1307), the Evidence's "once a backend emits that code", the Consequences and Alternative D. The C programs of the Evidence (`docs/adr/evidence/0107/c/`) stay measurement records that `una` never builds. `docs/plan/c-emission-inventory.md` lists each passage by line.

- **ADR 0108:**

  > ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend: the release path of `una build --release` is Una's own code generation over Core IR, under that amendment's requirements RP-1 to RP-10. No rule of this record changes. Its C passages read through that amendment: rule 2's "C locals" and "never a C struct parameter, so the C signatures stay scalar" (the release path's convention: cells as separate scalar arguments, RP-2), the Evidence's compiler flags and "once a backend emits this code (#522)" (the release path, #1305 and #1306), and the Consequences' "C signatures". The C programs of the Evidence (`docs/adr/evidence/0108/c/`) stay measurement records that `una` never builds. `docs/plan/c-emission-inventory.md` lists each passage by line.

- **ADR 0109:**

  > ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend; the release path of `una build --release` is Una's own code generation over Core IR. No rule of this record changes. Rule 3's "the optimized backend's arithmetic is defined at every input (ADR 0106, RB-7)" reads as the release path, which that amendment keeps under RB-7 and its requirement RP-5; check elision in `systems` packages follows ADR 0112 as amended on 2026-10-31. `docs/plan/c-emission-inventory.md` lists the passage.

- **ADR 0113:**

  > ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend; the release path of `una build --release` is Una's own code generation over Core IR. No rule of this record changes. Rule 5's "ADR 0106's C backend" reads as that release path, which keeps the 32-bit count and mark and their limits exactly as stated there. `docs/plan/c-emission-inventory.md` lists the passage.

- **ADR 0114:**

  > ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend; the release path of `una build --release` is Una's own code generation over Core IR, and its requirement RP-6 carries this record's floating-point rules into it. No rule of this record changes. Its C passages read through that amendment: rule 4.2's reason in terms of C (the guard stays on every backend, and the release path's conversion is defined at every input), rule 6.2's and rule 16.8's C flags, canary and probe (withdrawn; their replacement is #1308, and the FMA disassembly gate stays), rule 6.5's "C backend" (no platform `libm`: `sqrt` correctly rounded, every other exact operation a routine of `una-rt`), rule 9.1's "C `double`" (a binary64 register value), the Consequences' "flags, a canary and a probe", and Appendix C, divided as that amendment's §1 and §2 state. `docs/plan/c-emission-inventory.md` lists each passage by line.

- **ADR 0120:**

  > ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend; the release path of `una build --release` is Una's own code generation over Core IR. No rule of this record changes. Its C passages read through that amendment: the Context's FMA item (the allowlist and the disassembly gate stay), rule 9's "`float` in the C backend under `FLT_EVAL_METHOD == 0`" (binary32 in its own precision, RP-6), rule 58's lowering with GCC and Clang vector types and `__attribute__((target(...)))`, step 8 of the Implementation plan, and item 2 of Appendix A's amendment of ADR 0106 (withdrawn; how `simd required` lowers is that amendment's OQ-6, #1308, while rules 56 to 58 on targets, features and dispatch stand). Items 1, 3 and 4 of that Appendix A amendment survive. `docs/plan/c-emission-inventory.md` lists each passage by line.

- **ADR 0124:**

  > ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend and closes Question 1, as its §5 states: no release compiler is built into `una`. No rule of this record changes. Its passages on the release compiler read through that amendment: the header's relation to ADR 0106, Question 1's history and its cost line, the Context's and the first Departure's reference to an amendment, rule 9 (which has no subject), rule 22's "the release compiler's object" (the release path's object), the Consequences' exception for "the C compiler of release builds" (none: no external tool is run for release builds either) and the Implementation plan's last line. `docs/plan/c-emission-inventory.md` lists each passage by line.

ADR 0112 gets its own amendment (#1309), which restates its C mechanics rather than only pointing at the passages.

## 4. The evidence kits

The C files of the evidence kits are measurement records:

- ADR 0106: `docs/adr/evidence/0106/c/kernels.c`, `docs/adr/evidence/0106/c-variants/mix_variants.c`;
- ADR 0107: `docs/adr/evidence/0107/c/` (`k1_*.c`, `k1c_*.c`, `k2_*.c`, `k3_boxed.c`, `k3_layout.c`, `rt_boxed.h`);
- ADR 0108: `docs/adr/evidence/0108/c/`.

Checked on 2026-10-31: `una` never builds them, and they are in no shipped artifact and in no input of the C math library check.

- **Not built by `una` or by the workspace:** no crate, build script or script under `scripts/` refers to `docs/adr/evidence`. Only each kit's own `measure.py`, `bench.py` or `prepare.sh` compiles them, by hand, for a measurement.
- **Not shipped:** `scripts/release.sh` packs `una`, `lib/libuna_rt.a`, `LICENSE`, `README.md` and four guides of `docs/guide/`. The docs bundle (`scripts/site-bundle.sh`) takes `docs/adr/*.md` files only, never a `.c` or `.h` file.
- **Not inputs of the #1092 check:** `scripts/no-libm.sh` reads the release `una`, the release `libuna_rt.a`, and the executables that `scripts/no-libm-build.sh` links from `tests/conformance/runtime`.
