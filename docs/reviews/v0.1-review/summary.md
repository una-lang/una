# Una v0.1 review: summary

- **Tree:** `main` at `c2f3386` (the v0.1 merge), reviewed on branch `review/v0.1`, 2026-09-29.
- **Not covered:** the five commits of fix request 0001 on `dev/v0.1` after the merge (`b2c7b0a` to `0095fd5`).
- **Evidence:** only what the reviewers reproduced. Every Critical finding was reproduced a second time by the lead reviewer.

## Verdict: Not ready

The build, the tests, the examples and every acceptance criterion reproduce. But 11 Critical findings let `una check` accept programs that break the guarantees the charter lists as proven:
- effect and failure soundness (T2, T3), capability confinement (T4), refinement soundness (T5) and noninterference (T6);
- grant confinement at runtime.

Four of these end in a division by zero at runtime, after verification reported every obligation as proven. None needs an unusual program: generics, handlers, a policy, or a field named `tag` are enough.

The decisions are sound: 131 of 132 should be kept. The defects are gaps in implementation and specification, not wrong choices.

## Counts

| Severity | Count |
| --- | --- |
| Critical | 11 |
| Major | 11 |
| Minor | 17 |
| Note | 6 |

| Decisions | Count |
| --- | --- |
| Keep | 131 (4 of them with an implementation fix: D-048, D-079, D-085, D-103) |
| Switch | 0 |
| Escalate | 1 (D-093) |

`escalations.md` holds 10 questions for the owner.

## Top 10 findings

1. **R-001 (Critical):** function types drop `uses` and `fails` under unification and type arguments, so a `pub` function with no `uses` prints to Console, and a `main` with no `fails` fails.
2. **R-002 (Critical):** effectful function values escape through generics and run after their `with` scope, on the wrong handler.
3. **R-003 (Critical):** handler operation bodies are checked at pc public, so a handler's Console effect reveals a secret branch.
4. **R-004 (Critical):** generic functions erase labels, so `same(pin, b: "1234")` prints whether an internal value matches.
5. **R-007 (Critical):** a policy's `to` refinement is assumed, never proven, so a division by zero passes verification.
6. **R-008 (Critical):** capability contracts are looked up by bare name, so any module can give the host `Files.read` a false `ensures`.
7. **R-009 (Critical):** a field named `tag` collides with the SMT tag function, and every later obligation is proven.
8. **R-010 (Critical):** a user module named `prelude` is treated as the standard prelude, and every later obligation is proven.
9. **R-011 (Critical):** a dangling symbolic link lets `Files.write` escape its grant.
10. **R-012 (Major):** refinements inside type arguments are neither checked nor assumed, and the gap is recorded only in a plan file.

The other Criticals are R-005 (prelude `contains` leaks labels) and R-006 (calling a labeled function value does not raise the pc).

## Baseline (3.1)

| Command | Result |
| --- | --- |
| `cargo build --workspace` | ok |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --workspace` | 320 passed, 0 failed, 0 ignored |
| `RAYON_NUM_THREADS=1 cargo test --workspace` | 320 passed, 0 failed |
| Conformance runner | 194 cases: 194 passed, 0 pending, 0 failed |
| Determinism tests (4), default and with `RAYON_NUM_THREADS=1` | all pass; cold and warm `una --json obligations` byte-identical for all 7 examples |
| Examples: `una check` / `una test` / `una run` | all 7 packages exit 0 for all three commands |
| `scripts/demo/checkpoint-1.sh` to `-4.sh` | all exit 0, 42 outcomes, none different |
| Without Z3 on `PATH`, cold cache | 22 conformance cases fail with U5003 and none are skipped, as CLAUDE.md requires |

CI on `c2f3386` is green on Ubuntu and macOS (run 36644973753).

## Hard-limit audit (3.4)

| Limit | Verdict | Evidence |
| --- | --- | --- |
| `unknown` never passes | Pass, with a caveat | Every path maps to an error, including timeout, step budget, solver error, missing solver and a cached `unknown` (`una-verify/src/solver.rs:220-265`, `report.rs:22-26`). Caveat: a forged cache entry passes (R-018). |
| Information flow matches spec/07 | **Fail** | R-003, R-004, R-005 and R-006 are accepted leaks; R-014 and R-015 are spec gaps. About 100 other flow and authority probes behaved correctly. |
| Capabilities never behave as values | Pass for capabilities, **fail** for effectful functions | U3005 holds everywhere a capability was probed. Effectful function values escape (R-002). |
| No obligation kind skipped | Pass, with gaps | All five kinds have generation sites (`una-verify/src/generate.rs`). Refinements in type arguments are skipped (R-012), and some facts are assumed without an obligation (R-007, R-008). |
| One effect per line | Pass | U3013 in every nested position probed. Predicate bodies are exempt (R-029). |
| Tests not bent | Pass | 3 `expect:` edits, each only removing `pending`. Snapshot and assertion changes are backed by accepted ADRs. No `#[ignore]`. History before M5 is squashed (R-043). |
| No leftover `pending` markers | Pass | None under `tests/` or `examples/`. |
| Dependencies | Pass | All direct dependencies are on the allowlist of `architecture.md` §7. `getrandom` and `tempfile` come only through dev-only `insta`. |
| Code hygiene | Pass | `forbid(unsafe_code)` in every crate. The lint set denies unwrap, expect, panic and printing. The one `// INVARIANT:` (`una-syntax/src/source.rs:165`) holds. |
| Determinism | Pass | No `HashMap` or `HashSet` in the workspace. |
| Git history | Pass | `main` is `25d865b`, then `6c583aa`, then `c2f3386`, each an owner-requested merge. Nothing was rewritten. |
| Neutrality and naming | **Fail** | Command 1 has 82 hits, all in history files that ADR 0025 allows. Command 2 has 82 hits in 17 files (R-022). |

## Coverage and quality (3.5, 3.7)

- **Diagnostic codes:** every registered code except U8001 has a failing conformance case. 34 codes lack a positive case in the same section, though every one except U8001 has a fixed explain example that is checked (`coverage.md`, R-037).
- **Untested MUST rules:** most are in spec/05, spec/10 and spec/12, and several turned out to be violated (R-002, R-012, R-013).
- **`una explain`:** covers all 65 codes. Each failing example yields exactly its code, and each fixed example yields nothing at tier 1.
- **Error recovery:** independent errors are reported together, without cascades.
- **Mechanical repairs:** 32 of 34 fix their error. The exceptions are U2005 and U2009 (R-030).
- **Verification repairs:** they ignore applicability (R-021).
- **Crate dependencies:** no cycles and no upward edges, but `architecture.md` §2 is out of date (R-038).

## Files

- `findings.md`: every finding, with evidence and fix.
- `decisions.md`: the decision table and answers block.
- `acceptance.md`: M0 to M10 criteria.
- `escalations.md`: owner questions.
- `coverage.md` and `scripts/diag-coverage.py`: spec coverage.
- `adversarial/`: 143 programs, with an index in `adversarial/README.md`.
