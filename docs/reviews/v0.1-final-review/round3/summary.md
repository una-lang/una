# Limited re-review of closure round 3: summary

- **PR:** #1, `dev/v0.1` into `main`. Head `6e45a5a`, confirmed with `gh pr view 1`.
- **Branch:** `review/v0.1-round3`, 2026-10-01. Issue #80.
- **Scope:**
  - verify F-001 (#77) with ADR 0050 and its spec text, F-002 (#69), F-003 (#78, D-170 B) and F-004 (#79);
  - sweep the operation-argument class;
  - run the baseline;
  - rerun the 250 programs of the three reviews by code.
- **Evidence:** only what this reviewer reproduced, with cold proof caches. Comparisons use a binary built from `879c26a`, the head of the previous review.

## Verdict: Ready

No Critical or Major finding is open:
- **F-001 to F-004 are fixed**, and ADR 0050 is implemented as written, with its spec text in `spec/07 §7` rules 2 and 4, `spec/10 §5` (T-Op) and ADR 0046's header.
- **The operation-argument sweep held**, with 5 new programs:
  - function types at depth 2 and 3, and inside `Option`, `Result`, `List` and nested `List[Option[...]]`;
  - invariant kind arguments under `Box`, `Option` and `List`;
  - generic and closed aliases of function types;
  - label categories;
  - every call site: a function body under `with`, a handler operation body, and a `do` line under `given`.
  At `879c26a` all of these were accepted, except two category probes that hit U4003; all are U4001 now.
- **Data positions are unchanged**, `Files.write` included: the positive controls are accepted and run.
- **Baseline and reruns:** the baseline is green, and the 250 programs change only where F-001's fix says they should.

The sweep found one Minor over-rejection (R3-001) and one Note. Neither affects soundness or blocks the tag.

## Counts

**Items in scope**

| Item | Status | Evidence |
| --- | --- | --- |
| F-001 (#77), ADR 0050 | Fixed | `fr-op-arg-variance` and `fr-op-arg-fn-components` are U4001 in debug and release builds. `r3-oparg-forms` (10 forms), `r3-oparg-sites` (3 sites) and `r3-oparg-categories` are rejected. `r3-oparg-data-accepted` is accepted and runs. |
| F-002 (#69) | Fixed | `r3-repair-call-forms`: rank 1 is `with @front = @back` for a statement and a `let`, and widening is rank 2. With no alternative, widening is rank 1. A nested call is U3013 anyway. Applying either rank-1 edit gives a program that checks. |
| F-003 (#78, D-170 B) | Fixed | `fr-expect-fails-do`: `do attempt risky(0)` then `expect fails Broke` is U1001. Valid examples (a failing `do`, a wrapper that re-fails, `attempt` with `expect result == Err(Broke)`) check and pass `una test`. |
| F-004 (#79) | Fixed | Messages now read "result of fs.first()", "must have type `fn(Pos) -> Pos`, except for labels" and "it is `fn(T) -> T`". |

**New findings**

| Severity | Count | Findings |
| --- | --- | --- |
| Critical | 0 | |
| Major | 0 | |
| Minor | 1 | R3-001 |
| Note | 1 | R3-N1 |

**Adversarial programs:** 5 new, in `adversarial/`.
- `r3-oparg-forms` and `r3-oparg-sites` are accepted at `879c26a` and must be rejected; both are rejected now.
- `r3-oparg-categories` is partly accepted at `879c26a`, now rejected at every probe, and also shows R3-001.
- `r3-oparg-data-accepted` is the positive control.
- `r3-repair-call-forms` checks F-002.

## Baseline, at `6e45a5a`

| Command | Result |
| --- | --- |
| build, `fmt --check`, clippy `-D warnings` (`--locked`) | clean |
| `cargo test --workspace --locked` | 356 passed, 0 failed, 0 ignored |
| `RAYON_NUM_THREADS=1 cargo test --workspace --locked` | 356 passed, 0 failed |
| Conformance runner | 651 cases: 651 passed, 0 pending, 0 failed |
| Without Z3, cold caches | 138 cases fail, each with U5003 only; 0 pending, none skipped |
| Examples, cold: `una check`, `una test` | all 7 exit 0; 106 obligations proven; all 106 cached on the warm run |
| Determinism | cold `una --json obligations` byte-identical by default and with `RAYON_NUM_THREADS=1`, for all 7 examples |
| rustdoc `-D warnings`, release build | clean; the release build agrees with debug on `fr-op-arg-variance` (U4001) and N-006's program (U2002) |
| CI on `6e45a5a` | green: push run 36889093901, PR run 36889099662 |

## Rerun of the 250 programs of the three reviews

`../scripts/rerun-adversarial.sh` ran every program in a cold copy at `6e45a5a` and at `879c26a`; the outputs are in `rerun/`.
- **Changed codes: 2 programs, both expected.** `fr-op-arg-variance` (accepted before, U4001 now) and `fr-op-arg-fn-components` (U5001 before, U4001 now: the new tier-0 error stops verification before the U5001 control).
- **Unchanged codes: the other 248 programs.**

## Findings

### R3-001 (Minor). An operation with a labeled function parameter cannot be called under a lower `accepts`
- **Reference:** ADR 0050 rule 2 ("`accepts` keeps bounding the data at the covariant positions") against D-164 ("every label of the argument's type").
- **Evidence:** `adversarial/r3-oparg-categories` declares `op apply(f: fn(internal[Location] Int) -> Int)` with the default `accepts public`.
  - A public function is U4001, which is correct.
  - A function with the exact type is U4003: the sink counts the argument's parameter label as data.
  So no function can be passed, and the operation is unusable unless its `accepts` is raised to the parameter's label.
- **Before ADR 0050:** such a call was accepted with a lower-labeled function, which was F-001's hole. ADR 0050 closed the hole without making the exact type usable.
- **Soundness:** none at risk; this is an over-rejection.
- **Fix (v0.1.x):** let the sink bound only the labels at covariant positions of the argument's type, as ADR 0050 rule 2 states. The contravariant positions are now covered by the flow, so D-164's reason for counting them is gone.

### R3-N1 (Note). An inline lambda at an operation argument is checked against the declared type, not ADR 0050's view
`@store.later(fn() -> pin)` under `op later(f: fn() -> Int) accepts secret` is U4001 at the lambda result. The same lambda bound with `let` first is accepted. The cause is that the lambda is typed against the declared parameter type before the ADR 0050 view applies. The behavior is the same at `879c26a`, and it is sound. A fix would type the inline lambda against the view.

## Files

- `summary.md`: this file.
- `adversarial/`: 5 programs, each with a header (attack, expected, observed, verdict).
- `rerun/`: codes of the 250 programs at both heads, and of the 5 new programs at both heads.
