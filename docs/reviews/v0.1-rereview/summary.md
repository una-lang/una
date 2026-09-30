# Re-review of the v0.1 closure: summary

- **PR:** #1, `dev/v0.1` into `main`. Reviewed on branch `review/v0.1-closure`, 2026-09-30.
- **Heads:** the work ran at `4ea31cf`, which was the head when it started. The PR moved to `85787d7` during the review, and the baseline, all 143 first-review programs and all 89 new programs were rerun there. The only code change between the two heads is commit `3a5f4e9`, which added three prelude functions to the labeled-result list.
- **Evidence:** only what the reviewers reproduced. Every Critical below was reproduced by the lead reviewer at `85787d7` with a cold proof cache.

## Verdict: Not ready

The closure run did most of what it claims:
- all 11 Critical and 11 Major findings of the first review are addressed, and 16 of the 22 are fully fixed with regression tests;
- the baseline is green;
- the new rules for declaration identity, solver namespaces and file links held against 36 new attacks.

But five new Critical findings let `una check` accept programs that break noninterference (T6) or verification soundness:
- **N-001:** labels declared on kind fields are ignored by equality, generics and `contains`. This bug predates the closure; the first review missed it.
- **N-002:** user generic kinds are covariant in labels even where the parameter is a function parameter.
- **N-003:** a generic function bound with `let` leaks its arguments' labels (D-137).
- **N-004:** a policy's `via` contract is assumed but never checked at the call (the rest of R-007).
- **N-005:** an annotated `let` of a refined function type skips the check when the value comes from a call. This is a regression of the R-013 fix.

N-004 and N-005 end in a division by zero at runtime after every obligation was reported proven. N-001 to N-003 print a secret-dependent value to Console. Under your answer to E1 of the first review, the tag waits until these are fixed and re-reviewed.

The fixes look contained:
- N-003 and N-005 are local implementation fixes.
- N-001 needs one function (`deep_label`) to read field labels.
- N-002 and N-004 each need a short ADR, escalations E1 and E2.

## Counts

**Status of the first review's findings**

| Status | Count | Findings |
| --- | --- | --- |
| Fixed | 16 | R-001, R-002, R-003, R-006, R-008 to R-012, R-014 to R-017, R-019, R-021, R-022 |
| Fixed as decided by ADR 0041 | 1 | R-018 |
| Partially fixed | 5 | R-004, R-005 (N-001, N-003), R-007 (N-004), R-013 (N-005), R-020 (ADR 0044) |
| Not fixed | 0 | |

**New findings**

| Severity | Count |
| --- | --- |
| Critical | 5 (N-001 to N-005) |
| Major | 2 (N-006, N-007) |
| Minor | 11 (N-008 to N-018) |
| Note | 6 (N-019 to N-024) |

**Decisions**

| Recommendation | Count | Items |
| --- | --- | --- |
| Keep | 38 | 26 of D-133 to D-159, ADRs 0039 to 0043, R-1, R-3, R-5, and R-023, R-025, R-028, R-031 kept deferred |
| Switch | 1 | D-144 |
| Escalate | 2 | ADR 0044 (Accept), ADR 0045 (Amend and accept) |

**Adversarial programs:** 89 new, in `adversarial/`.
- 67 are in the five requested classes: 18 generics with effects and labels, 13 handler effect floors, 18 declaration identity, 11 solver namespaces, 7 file links.
- 22 are variants of the first review's findings.
- 10 programs are accepted but must be rejected: 3 for N-001, 3 for N-002, 1 for N-003, 2 for N-004, 1 for N-005.

## Baseline (the first review's §3.1), at `85787d7`

| Command | Result |
| --- | --- |
| `cargo build --workspace` | ok |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --workspace` | 348 passed, 0 failed, 0 ignored |
| `RAYON_NUM_THREADS=1 cargo test --workspace` | 348 passed, 0 failed |
| Conformance runner | 375 cases: 375 passed, 0 pending, 0 failed |
| Without Z3, cold caches (at `4ea31cf`) | 64 cases fail with U5003, none skipped |
| Determinism tests, default and 1 thread (at `4ea31cf`) | pass. Cold `una --json obligations` is byte-identical across thread counts for all 7 examples, and every obligation is cached on the warm run (106 of 106) |
| Examples: `una check`, `una test`, `una run` (at `4ea31cf`) | all 7 packages exit 0; 32 examples pass; 106 obligations proven, none refuted or unknown |
| `scripts/demo/checkpoint-1.sh` to `-4.sh` (at `4ea31cf`) | all exit 0; 42 outcomes, none different |
| rustdoc | 0 warnings |
| Hard limits on the closure diff | no new `unsafe`, `unwrap`, panic, printing or `HashMap` in library crates, and no new external dependency. Every changed expectation traces to an accepted ADR or a spec line. `origin/main` is an ancestor of the head, with no rewritten history. |
| CI on `85787d7` | green on `ubuntu-24.04` and `macos-26` |

Two notes on the rows marked `4ea31cf`:
- They were not rerun at `85787d7`, because commit `3a5f4e9` changes only the prelude label list, one conformance case and docs. The full test suite, which includes the example, demo and determinism tests, passes at `85787d7`.
- The debug build is what the tests use. N-006 shows one place where a release build behaves differently.

## Files

- `findings.md`: the status of R-001 to R-022, and the new findings N-001 to N-024.
- `decisions.md`: the table and the answers block.
- `escalations.md`: five questions for the owner.
- `adversarial/`: 89 programs, indexed in `adversarial/README.md`.
