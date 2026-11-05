# Compiler architecture (reference implementation, v0.1)

Status: Normative for the reference implementation.

## 1. Goals, in priority order

1. **Faithful to the spec.** The checker implements `spec/10`, the interpreter implements `spec/11`, the verifier implements `spec/12`. When in doubt, the spec wins.
2. **Deterministic.** Same inputs, same outputs, byte for byte (`spec/12 §10`).
3. **Agent-grade diagnostics.** Every error has a code, precise spans, JSON output and repairs where possible.
4. **Simple.** Readable code over clever code. Performance matters only where it keeps tier 0 interactive.

## 2. Workspace

```text
Cargo.toml                 workspace
rust-toolchain.toml        pinned stable toolchain
verification.lock          pinned solver (repository-level, used by tests)
crates/
  una-diag/               Diagnostic, codes registry, human and JSON rendering, explain texts
  una-syntax/             source files, spans, lexer, parser, AST with trivia
  una-fmt/                canonical formatter
  una-project/            una.toml, verification.lock, module discovery
  una-core/               Core IR types, printer, Core RL formulas
  una-check/              resolution, types, effects, capabilities, labels, inference, elaboration to Core
  una-verify/             obligation generation, SMT-LIB encoding, solver driver, cache, counterexamples
  una-interp/             Una Abstract Machine, host capabilities, traces, example harness
  una-report/             reserved for build report assembly; v0.1 assembles the report in una-cli (D-132)
  una-codegen/            native code generation: Core IR to an object file through Cranelift (v0.2, ADR 0055)
  una-http/               the HTTP/1.1 client of `Http`, over TCP or rustls, shared by both engines (v0.2, ADR 0074)
  una-crypto/             own cryptography for TLS: SHA-2, HMAC, HKDF, ChaCha20-Poly1305, X25519, AES-GCM, and Ed25519, ECDSA and RSA verification; pure Rust, `unsafe` forbidden (ADR 0133)
  una-hash/               keyed hashing: SipHash-2-4, the hash encoding, the process and table keys, the ordered hash table, shared by both engines (ADR 0119)
  una-rt/                 the runtime library of native executables; its ffi.rs is the only unsafe code (ADR 0058)
  una-lsp/                the language server of `una lsp`: LSP over stdio, the server's state (v0.2, ADR 0063)
  una-itaty/              the Itaty registry service: log, tiles, name map, checks of submissions, HTTP endpoints (ADR 0122 S8)
  una-z3/                 the call into the Z3 front end built into `una`: its entry, arguments and the solver's build; the third place with `unsafe` (v0.4, ADR 0124)
  una-link/               the internal linker of `una build`: Mach-O for aarch64-apple-darwin, chained fixups, unwind information, the ad-hoc signature, the verifier (v0.4, ADR 0124 step 3); for the musl targets, the scan of the ELF inputs (step 4a) and the ELF reader and relocation engine of rule 24, tested against `ld.lld` and GNU `ld` but not yet linking (step 4b)
  una-cli/                the `una` binary (the only compiler crate that does I/O to the terminal; una-rt is the I/O of native executables)
tests/conformance/         language conformance suite (`testing.md`)
examples/                  reference programs
```

Dependency direction, as the crates' `Cargo.toml` files declare it, where `a` ← `b` means that `b` depends on `a` (no cycles): `diag` ← every other crate but `hash` and `link`, which depend on no workspace crate (ADR 0119, ADR 0124); `syntax` ← `fmt`, `core`, `check`, `verify`, `interp`, `cli`; `project` ← `check`, `cli`; `core` ← `check`, `verify`, `interp`, `cli`; `fmt`, `check`, `verify`, `interp`, `report`, `codegen` ← `cli`; `core` ← `codegen`; `diag`, `core`, `interp` ← `rt`. `una-rt` is a static library that `una build` links into executables, not a dependency of `cli`; it shares the interpreter's grants and trace form. `check`, `verify` and `interp` do not depend on one another: `cli` runs them in turn and passes each one the Core IR that `check` elaborates. `report` is a stub that depends only on `diag` (D-132). (v0.2, ADR 0063.) `diag` ← `lsp` ← `cli`: `cli` passes the server a workspace that checks and formats a package, so `lsp` needs no other crate and no crate depends on `cli`. (ADR 0122.) `syntax`, `project` ← `itaty`: the registry service reuses the client's formats and verification, and no crate depends on it. (v0.4, ADR 0124.) `sys` ← `z3` ← `cli`: `una-z3` depends on `una-sys` alone, for the confinement record, and only `cli` depends on it. (v0.4, ADR 0124 step 3.) `link` ← `cli`: `una-link` depends on no workspace crate, and only `cli` depends on it.

## 3. Pipeline

```text
source files
  → una-syntax   lex (INDENT/DEDENT) → parse → AST (+ trivia for the formatter)
  → una-fmt      canonical check (U7001) ─────────────────────────────── tier 0
  → una-check    resolve modules and names
                  check kinds, types, labeled calls, exhaustiveness
                  check capabilities, effects, failures, inference for private fns
                  check information flow (labels, pc, sinks, policies)
                  elaborate to Core IR (spans preserved)             ─── tier 0
  → una-verify   generate obligations over Core → encode → Z3 → statuses ─ tier 1
  → una-interp   run main or examples over Core IR
  → una-codegen  compile main to an object (una build, v0.2) → cc links it with una-rt
  → una-cli      assemble the report from check, verify and interp results (una-report is reserved for it)
```

A declaration with tier 0 errors is not elaborated and produces no obligations. Other declarations continue, so one run reports as many independent problems as possible.

## 4. Key data structures

| Structure | Crate | Notes |
| --- | --- | --- |
| `FileId`, `Span { file, start, end }` (byte offsets) and `LineIndex` | syntax | Conversion to 1-based line and scalar-value column only at rendering time |
| `Symbol` (interned string) | syntax | Deterministic interner; never iterate the interner for output |
| AST | syntax | Typed node structs; child vectors; every node has a `Span`; comments attached as trivia |
| `DeclId`, `ExprId` | check | Index types into per-module arenas |
| `Type` | core | `Refined { base, pred: Formula, label }`, `Fn { params, ret, uses, fails, label }` |
| `Formula` | core | RL as a small AST; used by types, contracts and the verifier |
| Core IR | core | Owned tree per declaration, ANF, unique binders, spans on every node |
| `Obligation` | verify | `{ id, kind, declaration, span, hypotheses: Vec<Formula>, goal: Formula }` |
| `Diagnostic` | diag | Exactly the JSON shape of `spec/13 §1` |

## 5. Error recovery

- The lexer reports and skips invalid characters; indentation errors resynchronize at the next line.
- The parser synchronizes at the next line with zero indentation that starts a declaration keyword, `pub`, or a lower identifier followed by `(`.
- The checker isolates failures per declaration. An unresolved name gets an error type that suppresses cascading errors.
- (#581.) An unresolved name reports one diagnostic, at the reference. Its error type carries its origin, `Unknown`: only a reference to a name that does not resolve makes it, and only a declaration that such a type reaches stops reporting what inference left unsolved there. That covers an inference variable, a lambda's parameter, and an ordering on an unsolved type. Every other error type, the type of a call of a `Never` value for one, absorbs nothing more. So the absorption only ever applies in a package that U1001 or a syntax error already rejects, and no source that is rejected is accepted because of it.
- (ADR 0096.) A file with a syntax error is resolved and typed without its broken declarations, those that hold a syntax error. The names they would have declared are poisoned: a reference to one is unknown, with the error type, never U1001. When no name of a broken declaration can be read, every unresolved name of its module is unknown. A declaration that reaches a poisoned name is a dependent: its effect, flow and canonical-form diagnostics are withheld, and so is a U1001 or U1006 at a poisoned name, which type checking reports for a variant or a method-syntax call (ADR 0096 rule 9, with the note of 2026-10-13). Only a broken module header leaves the whole module out, as before.
- (ADR 0096 rule 12, #1573.) The recovery pass is O(n log n), not linear: each lookup of the declaration that holds a byte offset (`BrokenFile::in_broken`, the declaration of each syntax error) and of the line that holds an offset (the U0101 and U0001 repairs of `una_syntax::rewrite`) is a binary search over starts that increase strictly, O(log d) or O(log L). The U0001 renames of names that start with `_` read the file's words once into a set, O(n log w), and look each candidate name up in O(log w). Before #1573 these lookups were scans, O(errors x declarations) and O(errors x file size). The scaling tests `checking_past_broken_declarations_scales_linearly` and `renaming_underscored_names_scales_linearly` (`crates/una-cli/tests/recovery.rs`) bound a doubling at 2.5 times. The renames of `_` names still rescan the declaration that holds each error, so one very large declaration with thousands of such names stays quadratic in its own size (follow-up card). The wording of rule 12 is the subject of `docs/plan/0096-rule-12-amendment-draft.md`.

## 6. Determinism rules

- No `HashMap` or `HashSet` in anything that affects output order; use `BTreeMap`, `BTreeSet` or `IndexMap`.
- Parallelism (solver calls, per-module checks) collects results and sorts them before reporting.
- No timestamps, random numbers, absolute paths or environment-dependent values in diagnostics, reports or snapshots. Paths are relative to the package root.

## 7. Dependencies

Allowed without an ADR: `clap`, `serde`, `serde_json`, `toml`, `ariadne` (human diagnostics rendering), `indexmap`, `sha2`, `rayon`, `insta` and `pretty_assertions` (dev only). Anything else needs an ADR stating why the standard library or an allowed crate is insufficient. `Int` and every exact integer step use Una's own big integer, `una_core::bigint` (#1093), not a crate.

Allowed by ADR 0055, for the native backend crates `una-codegen` and `una-rt` only (v0.2): `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-object` and `target-lexicon`.

Allowed by ADR 0074, for `una-http` only (v0.2): `rustls` with its `ring` provider, and `webpki-roots`, the TLS stack of the `Http` capability, with their dependencies (`rustls-webpki`, `rustls-pki-types`, `untrusted`, `subtle`, `zeroize`, `once_cell`, `getrandom`). They are in the trusted base. Executables that never use `Http` do not link them.

Allowed by ADR 0116 (Appendix A, amendment of 2026-10-19), for `una-sys` only: `libc`, the Rust declarations of the operating system's interface (`sigaction`, `pipe`, `fcntl`, `write`, `read`, `raise`). It is bindings only: no C source is built, and it adds nothing to the supply chain, since it is already a dependency of crates in the lockfile (`getrandom`, `cpufeatures`, `ring`, `rustix`).

Allowed by ADR 0119, for `una-hash` only: `getrandom`, the process key of keyed hashing from the operating system's cryptographic random source. Its own `unsafe` code puts it in the trusted base (`docs/security/tcb.md`).

Allowed by ADR 0133 (accepted on 2026-11-02), for `una-http`: `una-crypto`, `getrandom` (already allowed above for `una-http` by ADR 0074 and for `una-hash` by ADR 0119) and `zeroize`, as optional dependencies of the features `own-crypto` and `own-provider`, the own TLS provider (`rustls` on `una-crypto`). `rustls` no longer has the `ring` feature in the workspace; `una-http`'s default feature `ring` enables it until the switch of ADR 0133 rule 17. As dev-dependencies of `una-http` only: `ring` (the reference providers' signer and the differential tests), `rustls` with its `ring` feature (the test servers) and `serde_json`. Without the `ring` feature `cargo tree -p una-http -e normal,build` has neither `ring` nor `cc`.

Allowed by ADR 0133 (accepted on 2026-11-02), for `una-crypto` only: `zeroize`, the volatile wipe of secrets, already in the lockfile through `rustls`; pure Rust. As dev-dependencies of `una-crypto` only: `ring` and `curve25519-dalek` (the references of the differential tests), `serde_json` and `sha2`. No build script, no `-sys` crate, no foreign code.

`una-lsp` (ADR 0063) adds no dependency: its JSON-RPC transport and its LSP types are written on `serde_json`.

Allowed by ADR 0124 rule 22, for `una-link` only: `object`, already in `Cargo.lock` through `cranelift-object`, with its features `std`, `read_core`, `archive`, `macho` and `elf` (readers only), for the inputs of the link; the writer is `una-link`'s own. `una-link` also uses `sha2`, for the signature's hashes and the UUID.

Allowed by ADR 0124, for `una-z3` only: a build script, which links the Z3 front-end library that `scripts/build-z3.sh` built from the pinned source (named by `UNA_Z3_LIB` in a development build, by `UNA_EMBED_Z3` with its digest in a release build), checks the digest with `sha2` and records the pin; it runs no program, downloads nothing and compiles nothing. The library is C++ built outside Cargo, the one foreign-code exception of #1097.

## 8. Targets

- Tier 0 on a 5,000-line package: under one second on a laptop (goal, not gate).
- Interpreter: correctness only in v0.1; no performance target.
