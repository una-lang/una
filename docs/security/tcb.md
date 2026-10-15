# The trusted computing base: a risk profile

Every Una guarantee holds "assuming the trusted base behaves as specified" (charter §3). The build report counts part of that base: its `tcb` section lists a package's `trusted` modules with their line counts, the host capabilities it uses, the solver and the proof cache (`spec/13 §6`). Line counts do not say what a bug in a component could do, what catches it, or what is left open. This document says that for each component of the toolchain's trusted base, and ranks the open gaps.

It describes the repository at commit `3e5b800b` (edition `0.3` in preview). Sizes are lines of Rust under each crate's `src/`, unit tests included. Proposed decisions are described as proposals.

## 1. What a bug can cause

Each component is rated by the worst outcome a bug in it can have on a program the toolchain accepts:

| Class | Meaning |
| --- | --- |
| **Soundness** | The checker accepts, or the verifier proves, what the specification rejects: a leak through the information-flow rules (`spec/07`), a capability used beyond its grant (`spec/05`), a false contract or refinement. |
| **Wrong result** | A run computes a value the Una Abstract Machine (`spec/11`) would not, without touching memory it does not own. |
| **Memory safety** | A run reads or writes memory outside the objects it owns. This is the class that turns any other bug into arbitrary behavior. |
| **Leak of the build** | An artifact reveals the machine, account or paths that built it. |
| **Supply chain** | An artifact differs from what its source and pinned inputs determine. |

## 2. The ranking

Ranked by the worst class, then by what guards it today:

| Rank | Component | Worst class | Main guard | Main gap |
| --- | --- | --- | --- | --- |
| 1 | Checker: names, types, labels, capabilities (`una-check`, `una-syntax`) | Soundness | Conformance suite; Lean re-check of fragment F1 | The re-check covers part of the language; capability confinement (T4) is not proven |
| 2 | Verifier and Z3 (`una-verify`) | Soundness; memory safety once proven checks are elided | Pinned solver; model re-checks of `sat`; obligations that are not proven are errors | `unsat` (proven) is trusted with no certificate; refinement soundness (T5) is not modeled |
| 3 | Native runtime (`una-rt`) and the code that calls it | Memory safety | `unsafe` confined to `ffi.rs`; native and interpreter compared on every runnable program | No sanitizer, Miri or fuzzing run |
| 4 | Host capabilities and the prelude | Soundness (labels), wrong result | Host-table checks; two implementations compared | Two implementations of each operation; the TLS stack is trusted |
| 5 | Native codegen (`una-codegen`) and the system linker | Wrong result | Native and interpreter compared | Programs outside the runnable corpus; `cc` unpinned and unrecorded |
| 6 | Reference interpreter (`una-interp`) | Wrong result | It is the reference; native runs are compared with it | Not related to the formal model |
| 7 | Elaboration and the bridge to the formal model | Soundness | Identity runs of the Core export | Elaboration and the importer are trusted |
| 8 | The Rust toolchain and the crate dependencies | Supply chain, and any class of the crate's own bug | Pinned toolchain; `Cargo.lock` with `--locked`; reproducible gate | No advisory or provenance check on dependencies |
| 9 | Release scripts and archives | Supply chain, leak of the build | Reproducible gate; SHA-256 digests | No signature or provenance statement |
| 10 | Proof cache | Soundness | HMAC-SHA-256 tags under the user's key | The key file is trusted |

## 3. The components

### 3.1 The checker

`una-check` (about 32,700 lines) and `una-syntax` (about 8,900) decide names, types, labels and information flow (`spec/07`), capabilities and authority (`spec/05`), and generate the obligations the verifier discharges. The prelude, `crates/una-check/src/std/prelude.una` (177 lines), is part of it.

- **A bug can cause:** soundness. A leak accepted by the flow rules, or a capability used where it is not granted. This is where most past defects were: 8 of the 14 advisories of `docs/security/advisories.md` are checker defects in information flow or authority (UNA-SA-001, 002, 004, 005, 006, 011, 012 and 013), four of them Critical.
- **Guards:**
  - the conformance suite (`tests/conformance`: `authority`, `security`, `types`, `verification` and others), run on every change by `scripts/local-ci.sh`;
  - unit and scaling tests in `crates/una-check/tests`;
  - the determinism gate of `scripts/release-gates.sh`, which runs the suite twice and compares outputs;
  - adversarial reviews of each change that touches these rules, where the advisories above were found.
- **Formal coverage:** the Lean development in `formal/` (`docs/plan/formal-track.md`, `formal-track-status.md`) proves type safety (T1), the unary halves of T2 and T3, and noninterference (T6) for fragment F1, with no `sorry` and no axioms beyond Lean's standard three (checked by `scripts/formal.sh`). A Lean checker proven sound re-checks the exported Core (`una core --json`, ADR 0083) of each declaration it can read: it accepts only what it proves, and answers "not re-checked" for a form outside F1. The replay of past findings on pre-fix releases is rejected by the re-check for most of them.
- **Open gaps:**
  - The re-check reads a part of the language: on the corpus of the status document, about a quarter of all declarations are accepted by it, and none rejected. What it does not read is guarded by tests and review only.
  - Capability confinement (T4, fragment F2, milestone M7) is not proven. Handlers declared outside a document, and `given`, are outside the re-check until then.
  - Refinements, `together`, maps, text, JSON and HTTP are outside F1 (formal plan §3).

### 3.2 The verifier and the solver

`una-verify` (about 9,400 lines) encodes obligations in SMT-LIB (`spec/12 §4`) and runs `z3 -in -smt2`, one process per query, at the version that `verification.lock` pins (`scripts/install-z3.sh` checks the download's SHA-256).

- **A bug can cause:** soundness. An encoding error, or a solver bug, that answers `unsat` for a false obligation proves a contract, a refinement or the absence of an overflow that does not hold. For a fixed-width integer, whose overflow is proven absent and not checked at run time (`spec/18 §3`), the run then computes a wrong value. Today it cannot access memory out of bounds through that, because index, length and handle checks always run. **That holds only while no such check is elided.** Once a run-time check is removed because the verifier proved it unneeded (#737, ADR 0106), the verifier and the solver guard memory safety for every elided bounds, handle or stack check: a false `unsat` behind an elided bounds check is an out-of-bounds access. ADR 0106's proposed rule RB-3a keeps proof-based elision to checks whose failure stays a wrong value, and asks a certificate (#66) for the others (§5).
- **Guards:**
  - an obligation that is not proven is an error (charter P8), so a timeout or an unknown never passes;
  - a `sat` answer is re-checked when it uses uninterpreted arithmetic (ADR 0053, ADR 0054), and a model that disagrees with real arithmetic is not a counterexample;
  - a malformed query is U8001, never a status (ADR 0033);
  - the verification tests and the `verification` conformance cases.
- **Formal coverage:** none. Refinement soundness (T5) is planned relative to the pinned solver, and is not started.
- **Open gaps:**
  - **No certificate for `unsat`.** A proven obligation is trusted as Z3 answers it. Proof certificates checked by a small kernel are on the backlog (#66).
  - The encoding is not modeled. A divergence between `spec/12 §4` and the encoder shows only through tests.

### 3.3 The native runtime

`una-rt` (about 6,000 lines) holds the values, tasks, host operations and the C ABI that generated code calls.

- **A bug can cause:** memory safety, at the boundary with generated code. Elsewhere, a wrong result.
- **Guards:**
  - `unsafe` is forbidden in the workspace and allowed only in `crates/una-rt/src/ffi.rs` (ADR 0058): 9 `unsafe` blocks, each with a `// SAFETY:` comment, 4 exported `unsafe extern "C"` functions (`una_rt_start`, `una_rt_int_literal`, `una_rt_text_literal`, `una_rt_closure`), and one `unsafe extern "C"` block that imports the generated code's entry points;
  - one `transmute` turns a closure's code address into a function pointer (`invoke`). It is the one place where a wrong code word becomes a call to an arbitrary address. `invoke` checks that the word is a closure of the right arity before it (#424), but the address itself is the one generated code passed to `una_rt_closure`;
  - handles are slot indices, not pointers, and the runtime's lookup refuses a word that is not a live handle (`crates/una-rt/src/values.rs`). A stale handle whose slot was reused names another live object: a wrong result, not a memory error. Handles carry no generation yet (ADR 0107 proposes one, §5);
  - the allocator `cap` enforces the memory budget (ADR 0061);
  - the native gate compares every runnable program of the repository's suites with the interpreter (`crates/una-cli/tests/native.rs`, `spec/16`).
- **Formal coverage:** none.
- **Open gaps:**
  - **No sanitizer, Miri or fuzzing run** covers `ffi.rs` or the code that calls it. The differential test compares outputs; it does not detect an out-of-bounds read that happens to produce the right output.
  - The build report's `tcb` section does not list the runtime, although it is in the Trusted zone.

### 3.4 Host capabilities and the prelude

The host operations (`Clock`, `Env`, `Files`, `Keys`, `Http`) have two implementations, one in the interpreter (`crates/una-interp/src/host.rs`) and one in the runtime (`crates/una-rt/src/host.rs`). `Http` uses `una-http` (about 1,100 lines) over `rustls` with the `ring` provider and `webpki-roots`, which are trusted (`docs/compiler/architecture.md`).

- **A bug can cause:** soundness through labels: a failure or an outcome that depends on internal data but is labeled public. Four advisories were of this kind (UNA-SA-003, 008, 009 and 010). Also a wrong result where the two implementations differ.
- **Guards:**
  - the host table, which states each operation's labels and failures, and the conformance cases on it;
  - the differential native gate, which compares the two implementations on the runnable programs;
  - the release-mode TLS test, which shows that a release build never trusts the test authority (ADR 0074).
- **Formal coverage:** the files and HTTP host are modeled as shared state (milestone M5), under stated premises about the host (H1 to H4 in the status document). The premises are assumptions, not checked properties of the implementation.
- **Open gaps:** each operation exists twice, and only the runnable programs compare them; the host premises are not tested against the implementation.

### 3.5 Native codegen and the linker

`una-codegen` (about 4,500 lines) lowers Core to Cranelift and emits an object file; `una build` links it with `libuna_rt.a` through the system `cc` (`crates/una-cli/src/build.rs`).

- **A bug can cause:** a wrong result. The generated code has no `unsafe` of its own and reaches objects only through the runtime's entry points, which refuse a word that is not a live handle (§3.3).
- **Guards:** the native gate, the `unsupported` list that names every runnable program native does not compile yet (it must be exact), and the reproducible and determinism gates.
- **Formal coverage:** none.
- **Open gaps:**
  - A program outside the runnable corpus is compared with nothing.
  - The system `cc` and linker are trusted, their version is neither pinned nor recorded in the build report.

### 3.6 The reference interpreter

`una-interp` (about 4,000 lines) implements the Una Abstract Machine over Core IR (`spec/11`). It is the meaning that every backend must refine (ADR 0055).

- **A bug can cause:** a wrong result, and every backend compared with it inherits the wrong expectation.
- **Guards:** the `runtime` conformance cases, and the expected outputs of the examples.
- **Formal coverage:** the Lean model has its own semantics of F1, but it is not compared with the interpreter.
- **Open gap:** a disagreement between the interpreter and the model's semantics is not tested.

### 3.7 Elaboration and the bridge to the formal model

The theorems hold for the Core that the Lean importer reads from `una.core/1`. Elaboration from the surface language to Core, the importer (`formal/UnaFormal/F1/Import.lean`) and the Lean kernel, at the version that `formal/lean-toolchain` pins, are trusted (formal status document).

- **A bug can cause:** soundness: a program whose Core is proven while its source means something else.
- **Guards:** the identity runs of the export (#397, #407), and the conformance suite, which checks the source.
- **Open gap:** elaboration is not modeled.

### 3.8 The Rust toolchain and the crate dependencies

Every component is compiled by `rustc` at the version that `rust-toolchain.toml` pins (1.98.1), from crates of crates.io, among them `cranelift`, `rustls` with `ring`, `webpki-roots` and `num-bigint`, at the versions that `Cargo.lock` records.

- **A bug can cause:** whatever the crate or compiler can cause, in every artifact: a soundness defect, a wrong result, memory unsafety in a crate's own `unsafe`, or a compromised dependency (supply chain).
- **Guards:** the pinned toolchain; `Cargo.lock`, which `scripts/local-ci.sh` enforces with `--locked`; the reproducible gate, which shows that an archive is what its source and lock determine.
- **Open gaps:** no gate checks the dependencies against the RustSec advisory database or for provenance (such as `cargo-audit`, `cargo-deny` or `cargo-vet`), so a known-vulnerable or compromised version in `Cargo.lock` reaches every artifact unseen.

### 3.9 Release scripts and archives

`scripts/local-release.sh` builds the archives from a clean worktree of the tagged commit, and `scripts/release-gates.sh` runs every gate on that commit in a fresh clone (`docs/release-gates.md`).

- **A bug can cause:** an archive that is not what its source determines, or one that reveals the build environment. Two Low advisories were of the second kind (UNA-SA-007, build paths in executables, and UNA-SA-014, account names in archives).
- **Guards:** the reproducible gate rebuilds the archives and compares their digests; every archive has a SHA-256 file; the gate summary is checked against a privacy pattern before it can be published.
- **Open gaps:** no signature or provenance statement accompanies an archive, so a downloader can check a digest but not who built it from which commit.

### 3.10 The proof cache

The cache answers a query that was solved before. Since ADR 0064, each entry carries an HMAC-SHA-256 tag under the user's key, and an entry whose tag fails is solved again.

- **A bug can cause:** soundness, if a forged entry were taken as proven.
- **Guards:** the tag, with a test against RFC 4231; no key means no cache.
- **Open gap:** the key file is in the Trusted zone; anyone who can write it can forge entries.

## 4. History by component

The 14 advisories of `docs/security/advisories.md`, by component:

| Component | Advisories | Class |
| --- | --- | --- |
| Checker (information flow, authority) | 8 | Soundness |
| Host operations and prelude (failure labels) | 4 | Soundness (labels) |
| Build and release (paths, account names) | 2 | Leak of the build |
| Runtime, codegen, verifier | 0 | |

No advisory has been a memory-safety defect. That fits the design (`unsafe` in one module, checked handles), but it is also what one would see if no tool looked for one: none of the tools that find memory errors has run (§3.3).

## 5. How the proposed decisions would move it

Four decisions are proposed and not accepted. Each would change the base as follows.

- **ADR 0106, an optimized release backend through C (#757).** It adds the C compiler and generated C to the base for release builds.
  - It proposes a discipline under which no generated C has undefined behavior: checked arithmetic, unsigned shifts, checks before division and narrowing, fixed flags, no `__builtin_assume` or `restrict`, so that "a verifier or encoding bug must stay a wrong answer, never become a memory-safety bug".
  - Its gates would close part of §3.3 and §3.5: a supported-compiler matrix that fails closed, with the version recorded in the build report; a `sanitize` gate (AddressSanitizer and UndefinedBehaviorSanitizer, zero reports); a `check-survival` gate with a canary; equivalence of the interpreter, Cranelift and C outputs; and reproducible release builds.
- **ADR 0107, data layout for the native backends (#749).** Generated code would read the runtime's slot table and objects directly, a new memory interface outside `una-rt`.
  - Handles gain a generation, and the layout is checked on each read, so a stale or corrupt handle fails a check or reads a live object of the expected layout. `unsafe` stays in `ffi.rs`.
  - It proposes static assertions on sizes and offsets on both sides, and sanitizer cases for table growth, moved storage and reused slots.
- **ADR 0108, small records off the heap (#754).** It adds no `unsafe` and no new trusted tool. It moves correctness into the lowering (ownership of the unboxed values and their companions), guarded by equivalence and allocation tests, and states a stack-depth regression risk.
- **ADR 0109, profiles `app` and `systems` (#762).** It adds nothing to the base. In `systems`, every `Int` operation gets an overflow obligation, so more results rest on the verifier (§3.2). Combined with the elision of proven checks (#737), a false proof of an overflow obligation would give a wrong value where `app` gives the right one, and, under ADR 0106's discipline, no undefined behavior. That holds for overflow checks, whose failure is a wrong value. If elision also removes a bounds, handle or stack check, a false proof there is a memory-safety defect (§3.2), so elision must be restricted to checks whose failure stays a wrong value. ADR 0106 (proposed) states this as its rule RB-3a: only such checks may be elided on a proof, and a bounds, handle or stack check needs a checked certificate of its `unsat` (#66).

The largest addition to come is foreign code: `trusted` foreign functions and the FFI boundary (#23, #33). It would add code that no Una check covers, and memory-safety risk at a new boundary. This profile is the baseline that its decision builds on.

## 6. Open gaps, ranked

1. **No memory-safety tooling on the runtime boundary** (§3.3), #766. A sanitizer run of the runtime and of generated code over the runnable suites, Miri on `ffi.rs`'s callers where it applies, and a fuzzer on the runtime's entry points. ADR 0106's `sanitize` gate would cover part of it. This rank reflects the class, not the history: no defect is known.
2. **Capability confinement unproven, and the re-check partial** (§3.1), #768. Milestone M7 of the formal track, and widening what the re-check reads.
3. **Proven obligations trusted without certificate** (§3.2), #66. Today a false proof gives a wrong value. **It becomes a memory-safety gap the day a bounds, handle or stack check is elided on a proof** (#737), unless elision is restricted to checks whose failure stays a wrong value, such as overflow, or an elided check needs a certificate. ADR 0106, still proposed, now states that restriction as its rule RB-3a. Refinement soundness relative to the solver (T5) is not started.
4. **No advisory or provenance check on the crate dependencies** (§3.8), #770. A gate that checks `Cargo.lock` against the RustSec database and the dependencies' provenance.
5. **The adversarial generators are a campaign, not a gate** (`docs/plan/security-hunt.md`), #767. They are run on demand and are not part of `scripts/release-gates.sh`.
6. **The build report's `tcb` section is incomplete**, #769. It omits the runtime, the codegen, the linker or C compiler and its version, and the TLS stack.
7. **The system `cc` and linker are unpinned and unrecorded** (§3.5). ADR 0106's matrix would address it.
8. **Host premises and the two host implementations** (§3.4). Testing the premises of the model against the implementations, and comparing the two implementations beyond the runnable programs.
9. **No provenance for archives** (§3.9). A signature, or a provenance statement that ties each archive to its commit and its gate record.
10. **Elaboration and the interpreter are not related to the model** (§3.6, §3.7).

This document is revised when a component enters or leaves the base, when a gap closes, and before any decision that adds foreign code.
