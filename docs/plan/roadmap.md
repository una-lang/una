# Roadmap

Status: Informative.

## 1. Versions

| Version | Theme | Main content | Exit gate |
| --- | --- | --- | --- |
| **v0.1** | Una Core | This repository's milestones M0 to M10 (`milestones-v0.1.md`) | Gate 1 |
| v0.2 | Systems | Native backend (Cranelift) refining the UAM; fixed-width integers with overflow obligations; `edit` and `take` parameter conventions; implicit per-request regions; profiles `app` and `systems` | Gate 2 |
| v0.3 | Verification and review | Laws (tier 2) and property tests (tier 3); semantic diff; full escape governance; requirement approval workflow; obligations grouped by proof domain | Gate 3 |
| v0.4 | Platform | External state semantics (idempotency, delivery guarantees, `observe`, durable effect journal); services and SLOs; typed data access; OpenAPI and TypeScript generation; infrastructure policy generation | Gate 4 |

Each version starts only after the previous gate passes, with one exception: v0.2 runs in parallel with Gate 1 (decided 2026-10-01). The Gate 1 benchmark stays pinned to a v0.1.x release, so v0.2 work never changes what Gate 1 measures. Items and their target versions are in `backlog.md`.

### Edition `0.4`

Edition `0.4` is the next preview edition, under the terms of ADR 0121. Its language and toolchain work, besides the platform theme of v0.4:

| Area | Content | Record | Decision |
| --- | --- | --- | --- |
| Long-running programs | A host-driven `step`, then `for ever` under `diverges`, with `stop`, `stop when`, `skip` and `on stop` | ADR 0116 | Accepted |
| Concurrency for servers | Tasks without async coloring, `within D` with `Timed[E]`, `first of`, and rate limits on grants | ADR 0117 | Accepted |
| The terms of a preview edition | Edition `0.4` may add constructs that let a program do more, with no stability promise until it is declared stable | ADR 0121 | Accepted |
| Collections | `Stack`, `Queue`, `Set`, `HashMap`, `HashSet`, `PriorityQueue` and `BitSet`, with stated costs, and `Map.update` | ADR 0119 | Accepted |
| The numeric foundation | `F32`, `Decimal`, `Complex`, dense arrays and SIMD | ADR 0120 | Accepted |
| Packages | Itá: the package manager, `ita.una` and `ita.lock`, and its registry | ADR 0122 | Accepted |
| HTTP servers | `HttpServer` and `Listener`, granted by address | ADR 0123 | Accepted |
| One binary | `una` checks, proves and builds with nothing else installed | ADR 0124 | Proposed |

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
