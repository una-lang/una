# Roadmap

Status: Informative.

## 1. Versions

| Version | Theme | Main content | Exit gate |
| --- | --- | --- | --- |
| **v0.1** | Una Core | This repository's milestones M0 to M10 (`milestones-v0.1.md`) | Gate 1 |
| v0.2 | Systems | Native backend (Cranelift) refining the UAM; fixed-width integers with overflow obligations; `edit` and `take` parameter conventions; implicit per-request regions; profiles `app` and `systems` | Gate 2 |
| v0.3 | Verification and review | Laws (tier 2) and property tests (tier 3); semantic diff; full escape governance; requirement approval workflow; obligations grouped by proof domain | Gate 3 |
| v0.4 | Platform (historical theme, superseded by ADR 0139) | Not in R0.4, and in no release of the plan of ADR 0139 rule 6: external state semantics (idempotency, delivery guarantees, `observe`, durable effect journal); services and SLOs; typed data access; OpenAPI and TypeScript generation; infrastructure policy generation. R0.4 ships the contents of the next section | Gate 4 |

Each version starts only after the previous gate passes, with one exception: v0.2 runs in parallel with Gate 1 (decided 2026-10-01). The Gate 1 benchmark stays pinned to a v0.1.x release, so v0.2 work never changes what Gate 1 measures. Items and their target versions are in `backlog.md`. The themes of this table predate the numbering of ADR 0139: the releases, compiler versions and editions are numbered as the next section states.

### Releases and editions

Release R0.N ships compiler version `v0.N.0`, and edition `0.N` is named by the release that closes it, as a stable edition (ADR 0139). Only releases that change syntax create an edition, so there is no edition `0.7` or `0.9`. `main` serves at most two editions, the stable one and the current preview; an older edition leaves `main` by a plan accepted for it (#1585 for editions `0.1` and `0.2`) and stays served by its release line. Editions `0.1` to `0.3` already carry the number of the release that introduced them.

| Release | Compiler | Edition it closes | Contents |
| --- | --- | --- | --- |
| R0.4 | `v0.4.0` | `0.4` | Long-running programs (ADR 0116), tasks, `within` and `first of` (ADR 0117), HTTP servers (ADR 0123), the core of Itá (ADR 0122) |
| R0.5 | `v0.5.0` | `0.5` | Collections (ADR 0119) and the numeric foundation (ADR 0120), moved from `0.4`; module constants (ADR 0126); `const` and `var` (ADR 0137) |
| R0.6 | `v0.6.0` | `0.6` | Data-driven and multi-step tests (the testing family, #1590 to #1594) |
| R0.7 | `v0.7.0` | none | A toolchain release |
| R0.8 | `v0.8.0` | `0.8` | General protocols and typestate (ADR 0130); the laws record (#1623) is planned for R0.8 |
| R0.9 | `v0.9.0` | none | A toolchain release |
| 1.0 | `v1.0.0` | | The last closed edition becomes the stable edition |

### Edition `0.4`

Edition `0.4` is the current preview edition, under the terms of ADR 0121. R0.4 (`v0.4.0`) ships it as a preview, and a later release makes it stable (ADR 0139, amendment of 2026-11-10, Proposed). Its language and toolchain work, with the records that ADR 0139 moves to edition `0.5` marked:

| Area | Content | Record | Decision |
| --- | --- | --- | --- |
| Long-running programs | A host-driven `step`, then `for ever` under `diverges`, with `stop`, `stop when`, `skip` and `on stop` | ADR 0116 | Accepted |
| Concurrency for servers | Tasks without async coloring, `within D` with `Timed[E]`, `first of`, and rate limits on grants | ADR 0117 | Accepted |
| The terms of a preview edition | Edition `0.4` may add constructs that let a program do more, with no stability promise until it is declared stable | ADR 0121 | Accepted; extended to every preview edition (ADR 0139) |
| Collections | `Stack`, `Queue`, `Set`, `HashMap`, `HashSet`, `PriorityQueue` and `BitSet`, with stated costs, and `Map.update` | ADR 0119 | Accepted; moves to edition `0.5` (ADR 0139, #1621) |
| The numeric foundation | `F32`, `Decimal`, `Complex`, dense arrays and SIMD | ADR 0120 | Accepted; moves to edition `0.5` (ADR 0139, #1621), `F64` stays in `0.3` |
| Packages | Itá: the package manager, `ita.una` and `ita.lock`, and its registry. In `0.4.0` the registry is designed but not served: there is no https transport, registry packages are read only from a mirror directory, and `publish` and `yank` send nothing | ADR 0122 | Accepted |
| HTTP servers | `HttpServer`, granted by address; a `Listener` of raw connections waits for a later record (ADR 0123 Question 1) | ADR 0123 | Accepted |
| One binary | `una` checks, proves and builds with nothing else installed | ADR 0124 | Accepted (2026-10-27) |
| Optimized release builds | `una build --release` through Una's own optimizing code generation over Core IR; the C-emitting route of ADR 0106 is withdrawn | ADR 0106, amendment of 2026-10-31 | Design open (#1305) |

Una admits no foreign code: no foreign function interface, in any form, in any edition.

## 2. Parallel tracks

| Track | Content | Constraint |
| --- | --- | --- |
| Formal | Lean 4 mechanization of Una Core (`spec/10 §10`, T1 to T7) | Starts when M6 is done (rules stable); must finish before v0.2 is released |
| Evaluation | Gate 1 benchmark suite and harness, in a separate repository | Ready when M10 is done |

## 3. Gate 1: the thesis test

Gate 1 compares agents working on the same tasks in TypeScript, Rust and Una, with the same model and tool budget.

| Suite | Tasks | Measures |
| --- | --- | --- |
| A. Functional | Implement and modify small services and libraries from specifications | Tasks completed, defects found by hidden tests, repair iterations |
| B. Security | Tasks with traps: SSRF, secret leakage into logs, privilege escalation, confused deputy, unintended side effects | Violations shipped |
| C. Specification gaming | Deliberately underspecified tasks | Invented assumptions, trivial or weakened contracts, escape hatch usage |
| D. Review | Engineers review agent pull requests in each language | Time to detect a seeded dangerous change |

Gate 1 passes when Una shows fewer violations in B and C, no more repair iterations in A, and faster detection in D, each by a margin fixed before the runs. If it fails, the result decides what to change before v0.2, not whether to add features.

## 4. v0.1 usage checkpoints

The milestone order does not change (`milestones-v0.1.md`). These checkpoints say what a user can do after each group of milestones.

| Checkpoint | Reached after | What the user can do | Demo |
| --- | --- | --- | --- |
| 1. Read | M1, M2 | Write `.una` files, get syntax diagnostics with exact positions, format code | `una check --tier 0`, `una fmt`, `una explain U0101` |
| 2. Understand | M3 to M6 | Get name, type, authority and information-flow errors | a missing `uses` (U3001), `@` in a pure function (U3004), a `secret` value printed to `Console` (U4003), `una contract` |
| 3. Run (first usable version) | M7 | Write `main`, check it and run it; run the examples with effect traces | `una run examples/hello --grant console`, `una test examples/events`, `una check --dump-core` |
| 4. Trust | M8 to M10 | Proven contracts with counterexamples, build report, release v0.1.0 | a false `ensures` gives U5001 with a counterexample; `una obligations`, `una report` |

The critical path to checkpoint 3 is M1, M3, M4, M5, M7.

Each checkpoint whose last milestone is complete has a demo script, `scripts/demo/checkpoint-<n>.sh`, which prints each command, its expected outcome and its actual outcome, with its programs in `tests/demo/checkpoint-<n>/`; an end-to-end test runs every demo script. From M7 until M8, `una run` and `una test` check at tier 0 before executing (ADR 0027).
