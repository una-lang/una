# v0.1 milestones

Status: Informative, binding for task order. Each milestone lists the spec sections to read first, the tasks, and acceptance criteria that must all pass before the next milestone starts. Every milestone also adds `explain` texts for the diagnostic codes it introduces and conformance cases for each (`compiler/testing.md`).

## M0. Workspace and infrastructure

Read: `compiler/architecture.md`, `compiler/conventions.md`, `compiler/testing.md`, `spec/13 §1, §4, §5`.

Tasks:
1. Cargo workspace with the crates of `architecture.md §2` as stubs, `rust-toolchain.toml`, `#![forbid(unsafe_code)]` everywhere.
2. `una-diag`: `Diagnostic` exactly as `spec/13 §1`, code registry mirroring `spec/13 §4`, human rendering (ariadne) and JSON rendering, `explain/` directory with one file per code (stub text allowed until the owning milestone).
3. `una-cli`: `una check` (no-op returning success) and `una explain`.
4. Conformance runner with directive parsing (`testing.md §2`).
5. CI: fmt, clippy, tests on Linux and macOS; installs the pinned Z3.
6. Repository-level `verification.lock` with the latest stable Z3 release at the time, and a check command that prints the detected version.

Acceptance:
- `cargo test --workspace` passes; CI is green.
- `una explain U0001` prints text; `una check --json` prints an empty list with schema `una.diagnostics/1`.
- A snapshot test pins the JSON shape of a sample diagnostic.

## M1. Lexer and parser

Read: `spec/01`, `spec/02`.

Tasks:
1. Source database, spans, line index with scalar-value columns.
2. Lexer: indentation algorithm, comments and doc comments, identifiers, all keyword classes, literals, interpolation, operators.
3. Recursive-descent parser for the full grammar, producing AST with spans and trivia.
4. Error recovery (`architecture.md §5`); `una check --tier 0` reports syntax diagnostics.

Acceptance:
- Conformance cases for U0001 to U0007 and U0101 to U0103.
- Every `.una` file under `examples/` parses.
- Robustness test: for every example and every line boundary, parsing the prefix never panics and every reported position lies inside the prefix; a prefix that ends right after the first line of a multi-line declaration, or right after the `=` line of a long-form function, yields at least one diagnostic (ADR 0017).

## M2. Formatter

Read: `spec/01 §2`, `spec/02 §3`, `spec/04 §1, §3`, ADR 0011.

Tasks:
1. Pretty printer from AST and trivia: indentation, clause order, short versus long function form, constructor field order, spacing.
2. `una fmt`, `una fmt --check`, U7001 in `una check` with a mechanical edit.

Acceptance:
- Idempotence: `fmt(fmt(x)) == fmt(x)` for every `.una` file in the repository.
- Round trip: `parse(fmt(x))` equals `parse(x)` modulo spans.
- All examples are canonical.

## M3. Project and names

Read: `spec/13 §7`, `spec/01 §4`, `spec/08 §1`, `spec/07 §6`, `spec/12 §9`.

Tasks:
1. `una.toml` (U7002) and `verification.lock` (U7004) parsing.
2. Module discovery and path check (U1008); imports, visibility, cycles, duplicates, unresolved names (U1001 to U1004).
3. Naming conventions (U1005); requirement rules (U1002, U1006, U1007); trusted module agreement (U7005).

Acceptance: conformance cases for every listed code; all examples resolve.

## M4. Types

Read: `spec/03`, `spec/04 §1 to §3, §6, §7`, `spec/06 §1, §2, §5`, `spec/10 §3, §4`.

Tasks:
1. Kinds (alias, refinement, record, sum, generic) and the recursion restriction (U2002, U2003).
2. Prelude signatures from `spec/14 §1`.
3. Bidirectional checking with unification for generics; selfification of RL lets.
4. Calls: labeled arguments, punning, method-call syntax (U2004 to U2006); constructors (U2009).
5. `when` exhaustiveness over variants and arm order (U2007, U2008); statement types (U2011); positions of `result`, `it`, `error` (U2012, U2013).
6. RL membership (U2014); predicates (U2015); lambdas (U2010); interpolation and `to_text` rules.
7. Refinement subsumption records pending obligations (not yet verified).

Acceptance: conformance cases for U2001 to U2015; `examples/events/src/events/seating.una` passes tier 0.

## M5. Capabilities, effects and failures

Read: `spec/05`, `spec/06 §3 to §6`, `spec/04 §4, §5, §8`, `spec/10 §5, §6, §9`, `spec/14 §3, §4`, ADRs 0005 to 0007, 0009.

Tasks:
1. Capability declarations, `uses`, sufficiency, `with` rebinding and handler instances (T-Rebind, T-Handler).
2. Handler checking (U3010, U3011); second-class rules (U3005, U3006).
3. Effect computation; inference for private functions (least fixed point); U3001 to U3004.
4. Failure typing: `fails`, `fail`, handling clauses, `pass`, `attempt`, `checked` (U3007 to U3009); one effect per line (U3013).
5. `main` and host capabilities (U3012); examples restrictions (U3014); `std.testing` restriction (U3015); `checked` escape budget (U7003).
6. `una contract` with declared and inferred headers, human and JSON.

Acceptance: conformance cases for U3001 to U3015 and U7003; `una contract --json` snapshot for the events example.

## M6. Information flow

Read: `spec/07`, label premises in `spec/10`, sinks in `spec/14 §3`.

Tasks:
1. Categories (U4007) and the label lattice.
2. Explicit flows, pc tracking and implicit flows including failures and calls under raised pc (U4001, U4002); `bound(f)`.
3. Sinks (U4003); interpolation propagation; example `expect` exemption.
4. Policies and trusted modules (U4004, U4005); `via` exemption.

Acceptance: conformance cases for U4001 to U4007 including the classic implicit flow, failure under pc, and labeled text printed to `Console`; the whole events example passes tier 0.

## M7. Core IR and interpreter

Read: `spec/09`, `spec/11`, `spec/14`, `spec/08 §2`, ADR 0002, ADR 0004.

Tasks:
1. Elaboration to Core IR implementing the full desugaring table; `una check --dump-core`.
2. UAM evaluator (big-step, arbitrary-precision `Int`, Euclidean division).
3. Host capabilities including `Files` prefix confinement and `Keys` HMAC-SHA-256 (implemented over `sha2`).
4. Traces with instance ids and redaction.
5. `una run` with grants (U3012 at startup, U6001, U6003); `una test` harness with all expectation forms.

Acceptance:
- `una test examples/events` passes; `una run examples/hello --grant console` prints the greeting.
- Snapshot tests of Core dumps and trace JSON; a redaction test proves labeled arguments never appear in traces.
- Differential check: every conformance `run-output` case matches.

## M8. Verification

Read: `spec/12`, obligation premises in `spec/10`, `spec/13 §1 to §3`, ADRs 0003, 0008, 0010.

Tasks:
1. Obligation generation for all kinds with path contexts and hypotheses.
2. SMT-LIB encoding with deterministic emission; ground facts; literal distinctness.
3. Z3 driver: lock check (U5003), preamble, statuses, models, reasons, wall-clock guard.
4. Counterexample mapping; ranked repairs; obligation ids; proof cache; parallel execution with deterministic ordering.
5. `una obligations`.

Acceptance:
- Conformance cases with proven, refuted (with counterexample) and unknown outcomes; U5001 to U5003.
- SMT queries for the examples are byte-identical across runs (snapshot).
- Second run of `una check` on the examples reports every obligation as cached.
- All examples verify with zero refuted and zero unknown.

## M9. Agent ABI and build report

Read: `spec/13`, `spec/08 §1`.

Tasks:
1. `--json` consistency across all commands; exit codes.
2. `una explain` complete for every code, each with a failing and a fixed example.
3. `una trace`, `una report` with every section, including coverage and trivial `ensures` detection.
4. Determinism tests (`testing.md §4`).

Acceptance:
- The conformance runner compiles every `explain` example and checks it fails with its code and that the fixed version passes.
- Report and obligations JSON snapshots for the events example.

## M10. Gate 1 readiness

Read: `00-charter.md §7`, `plan/roadmap.md §3`.

Tasks:
1. At least five additional example packages covering every construct in the charter scope.
2. `docs/guide/una-for-agents.md`: a compact reference an agent can hold in context (syntax, headers, handling, capabilities, ABI usage). Every snippet is tested.
3. Release binaries for Linux and macOS; tag `v0.1.0`.

Acceptance: the guide's snippets compile; release artifacts build in CI; the evaluation track can drive `una` headlessly through `--json`.
