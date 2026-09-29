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
  una-cli/                the `una` binary (only crate that does I/O to the terminal)
tests/conformance/         language conformance suite (`testing.md`)
examples/                  reference programs
```

Dependency direction (no cycles): `diag` ← everything; `syntax` ← `fmt`, `check`; `project` ← `check`, `cli`; `core` ← `check`, `verify`, `interp`; `check`, `verify`, `interp` ← `report` ← `cli`.

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

## 6. Determinism rules

- No `HashMap` or `HashSet` in anything that affects output order; use `BTreeMap`, `BTreeSet` or `IndexMap`.
- Parallelism (solver calls, per-module checks) collects results and sorts them before reporting.
- No timestamps, random numbers, absolute paths or environment-dependent values in diagnostics, reports or snapshots. Paths are relative to the package root.

## 7. Dependencies

Allowed without an ADR: `clap`, `serde`, `serde_json`, `toml`, `ariadne` (human diagnostics rendering), `indexmap`, `sha2`, `rayon`, `num-bigint` (for `Int`), `insta` and `pretty_assertions` (dev only). Anything else needs an ADR stating why the standard library or an allowed crate is insufficient.

## 8. Targets

- Tier 0 on a 5,000-line package: under one second on a laptop (goal, not gate).
- Interpreter: correctness only in v0.1; no performance target.
