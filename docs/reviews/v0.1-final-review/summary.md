# Limited re-review of the v0.1 closure: summary

- **PR:** #1, `dev/v0.1` into `main`. Head `879c26a`, confirmed with `gh pr view 1`. The PR did not move during the review.
- **Branch:** `review/v0.1-final`, 2026-10-01. Issue #3.
- **Evidence:** only what this reviewer reproduced. The Critical below was reproduced with a cold proof cache, run to its printed leak, and dated on binaries built from `85787d7` and `c2f3386`.
- **Scope:** as issue #3 and its comment set it:
  - N-001 to N-007;
  - ADRs 0044 to 0049;
  - the Gate 1 fixes #68, #69 and #71, plus #70 and #72 from the owner's comment;
  - the two class-level sweeps;
  - the full baseline and the 232 earlier programs;
  - D-160 to D-170.
  The review went beyond `una-check` types and flows only where F-001 led: the operation-argument code in `call.rs` and `spec/07 §7` rule 4.

## Verdict: Not ready

Closure round 2 fixed everything it set out to fix:
- **Re-review findings:** N-001 to N-007 are fixed. Every one of the re-review's 10 accepted-but-must-reject programs is now rejected, and N-006 behaves the same in release and debug builds.
- **ADRs:** 0044 to 0049 are implemented as written, with their spec text.
- **Gate 1 issues:** #68 and #71 are fixed, and #71's lambda refinements are sound at 9 flow sites.
- **Baseline and reruns:** the baseline is green, and the 232 earlier programs change only where a fix says they should.

The flow-site sweep found one new Critical, outside the closure's matrix:
- **F-001:** an operation argument is never compared, as a flow, with the operation's declared parameter type.
  - A `fn(Int) -> Int`, or a `Sink[Int]` built from an identity, passes where an operation declares `fn(secret Int) -> Int` or `Sink[secret Int]`.
  - A handler holding a secret feeds the secret to that argument and returns it as a public `Int`.
  - `una check` exits 0, and `una run` prints the PIN.
  - The bug predates the first review. Handler instance arguments, refinements and `fails` at the same site are checked correctly; only labels are skipped.

The cause sits in the spec: `spec/07 §7` rule 4 exempts operation arguments from their parameter types' labels, which is right for data and wrong for contravariant and invariant positions. So the fix needs a short ADR (escalation E1, option A recommended). Under the owner's rule, an open Critical means Not ready.

## Counts

**Items in scope**

| Item | Status |
| --- | --- |
| N-001 to N-007 | 7 fixed |
| ADRs 0044 to 0049 | 6 implemented as written. F-001 is outside ADR 0046's list of sites, and F-003 is a case ADR 0044 does not name. |
| #68 (UI-009), #71 (UI-008) | fixed |
| #69 (UI-005) | partially fixed: correct at operation sites, still widening at rank 1 at call sites (F-002) |
| #70 (UI-002) | correctly closed as not reproduced; accepting it is a backlog change |
| #72 (UI-004) | message fixed; the status question stays with the owner (F-008) |

**New findings**

| Severity | Count | Findings |
| --- | --- | --- |
| Critical | 1 | F-001 |
| Major | 0 | |
| Minor | 3 | F-002, F-003, F-004 |
| Note | 4 | F-005 to F-008 |

**Decisions**

| Recommendation | Count | Items |
| --- | --- | --- |
| Keep | 10 | D-160 to D-169 |
| Switch | 1 | D-170 to B (F-003) |
| Escalate | 1 | E1, for F-001 |

**Adversarial programs:** 18 new, in `adversarial/`.
- 1 is accepted but must be rejected (F-001), and 1 more shows F-001 under `List` and `Box` next to a control.
- 2 show Minor findings (F-002, F-003).
- The rest held: 3 on argument binding and lambda refinements, and 11 on variance, labels and flow sites, including 3 controls for F-001.

## Baseline, at `879c26a`

| Command | Result |
| --- | --- |
| `cargo build --workspace --locked` | ok |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --workspace --locked` | 354 passed, 0 failed, 0 ignored |
| `RAYON_NUM_THREADS=1 cargo test --workspace --locked` | 354 passed, 0 failed |
| Conformance runner | 625 cases: 625 passed, 0 pending, 0 failed |
| Without Z3, cold caches | 130 cases fail with U5003, 0 pending, none skipped |
| Examples, cold: `una check`, `una test` | all 7 exit 0; 106 obligations proven, none refuted or unknown; all 106 cached on the warm run. `una run` and the demo scripts are covered by `cargo test` (`examples.rs`, `demo_scripts.rs`). |
| Determinism | cold `una --json obligations` byte-identical by default and with `RAYON_NUM_THREADS=1`, for all 7 examples |
| rustdoc, `-D warnings` | clean |
| Release build | ok; N-006's program is U2002, exit 1, as in debug |
| Build report | `out_of_model` has the 7 entries of ADR 0048 |
| CI on `879c26a` | green on `ubuntu-24.04` and `macos-26` |

## Rerun of the 232 programs of both reviews

`scripts/rerun-adversarial.sh` ran every program in a cold temporary copy with the `879c26a` binary and with a `85787d7` binary built for comparison. The outputs are in `rerun/`, as codes per program and as every diagnostic with its position.

- **Changed codes: 13 programs, all expected.** The 10 must-reject programs of N-001 to N-005, N-006's program (U8001 to U2002), N-008's program (U2016), and the first review's `escape-policy-opaque-fact` and `verify-policy-to-refinement`, whose `via` shape ADR 0047 now rejects with U4005 instead of U5001.
- **Same codes, different diagnostics: 5 programs.**
  - Two N-001 programs now report every probe, not only their controls.
  - `rv-gen-kind-fn-field-step` gains a U4001 at its lambda, as ADR 0046 requires.
  - Two programs moved U3016 to the offending value, as D-167 decided.
- **Accepted programs:** 16 of the first review's and 10 of the re-review's, as the closure reported. These are the valid programs, the controls, the runtime link programs and the hard-link program (out of model, ADR 0048).

## Origin and root cause of F-001

| Finding | Origin | Root cause | Needs an ADR |
| --- | --- | --- | --- |
| F-001 | Predates `c2f3386`; both earlier reviews and the closure's matrix left operation arguments out of the flow sites | **Spec**: `spec/07 §7` rule 4 says an operation's parameter types do not bound its arguments. The code (`call.rs:688-716`) follows it, checking only `accepts` against the argument's own labels. | **Yes** (E1) |

## Scope of the next review

A **targeted** check is enough: F-001's fix and ADR, plus an `op argument` row in the flow and variance matrices. It should rerun the 250 programs of the three reviews by code.

The other flow sites outside the closure's matrix were probed and held:
- handler instance arguments;
- failure payloads;
- `if error use`;
- `attempt`;
- `given`;
- operation-argument refinements and `fails`.

F-002 to F-004 do not block the tag.

## Files

- `findings.md`: the status of everything in scope, and F-001 to F-008.
- `decisions.md`: D-160 to D-170, and the answers block.
- `escalations.md`: E1.
- `adversarial/`: 18 programs, indexed in `adversarial/README.md`.
- `rerun/`: codes and diagnostics of the 232 earlier programs at `85787d7` and `879c26a`, and of the 18 new programs.
- `scripts/rerun-adversarial.sh`: the rerun.
