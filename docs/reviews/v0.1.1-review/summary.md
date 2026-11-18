# Limited review of PR #93 (ADR 0051): summary

- **PR:** #93, `dev/89-generic-fold-lambda` into `main`. Head `65388fa`, confirmed with `gh pr view 93`. The base is `main` at `f5df029`.
- **Branch:** `review/90-adr-0051`, 2026-10-01. Issue #90.
- **Scope:**
  - soundness of ADR 0051's position-precise result labels;
  - the U4001s dropped from existing cases;
  - ADR 0051 and its spec text;
  - the baseline;
  - a rerun by code of the 255 adversarial programs of the earlier reviews.
- **Out of scope:** user generic functions and nested calls (D-137, issue #92).
- **Evidence:** only what this reviewer reproduced, with cold proof caches. Comparisons use a binary built from `main` at `f5df029`.

## Verdict: Ready for v0.1.1

No Critical or Major finding.
- **Soundness:** ADR 0051 is sound as implemented. Four new programs try to leak through the ten structural functions:
  - alone and composed;
  - through `Option` and `Result` shapes and records with labeled fields;
  - through predicates, mappings and secret indexes;
  - through fold accumulators decided by secret elements through control flow;
  - under a raised program counter, at operation arguments (ADR 0050), and inside generic functions.
  Every leak is rejected. Everything accepted reveals only a shape that no element value decides.
- **Dropped U4001s:** all 67 read no element value. The PR says 59 (Note V11-N1).
- **Spec and ADR:** ADR 0051 is in `spec/14 §1`, `spec/07 §7` rule 8, and the Amended-by note on ADR 0038. One wording point is left for `fold` (Note V11-N2).
- **Baseline:** green.
- **Rerun:** of the 255 programs, only 3 changed codes, each losing a U4001 that ADR 0051 makes unnecessary.

## Soundness sweep

| Program | What it attacks | Result |
| --- | --- | --- |
| `adversarial/v11-fold-top.una` | fold accumulators whose shape a secret element decides through control flow (`Option`, `Bool`, list), from secret and public initial values | 4 of 4 rejected. The result keeps the accumulator's own top label, joined with the tops of `xs` and `step` |
| `adversarial/v11-structural-leaks.una` | the ten functions alone and composed; predicates and mappings over secret elements and a secret record field; `at` with a secret index; `first` and `at` payloads; a secret-top list; a secret-chosen predicate; a counting fold by a secret test; `contains` on records | 18 diagnostics, at least one at each of the 16 leaks. The inner `Option` of `Option[Option[secret Int]]` is accepted: its shape is declared public |
| `adversarial/v11-structural-pc-oparg.una` | secret-chosen lists through `map`, `filter`, `fold`, `append` and `first`; effects under `first() is Some`, `at(index: pin) is Some` and `first_where`; elements, lengths, records and options at operation arguments | Rejected wherever a secret decides the value, the condition or the data (U4001, U4003). An effect under `xs.first() is Some` over secret elements, a length sent to a public operation, and records sent to an operation that accepts `secret` are accepted, correctly |
| `adversarial/v11-structural-generic.una` | a count, a search, a mapping and a counting fold by `x == y` in generic functions; issue #89's `concat`; their calls on secret data | The lambdas that read `label(T)` are U4001. `flags` may carry `label(T)` in its declared result (rule 8), and its call is caught. `concat` is accepted, and its call result is raised by D-137 (#92) |

At `f5df029` all four programs give the same codes (`rerun/codes-new-at-*.tsv`), so no leak that `main` rejected became accepted.

## The dropped U4001s

`rerun/dropped-u4001.txt` lists every U4001 that `main`'s binary reports and the PR's binary does not, on the `main` versions of the changed cases, with each source line. There are 65 in 8 files, plus one cascade U4001 in each of `policy_as_value` and `via_as_value`, which stay rejected by U2001 and U4005. That makes 67, which matches the expectation diff file by file: 100 U4001 expectations removed and 33 re-added at moved positions.

Each drop is one of the following, and none reads an element value:
- a length or emptiness: `.len`, `is_empty`, `append(...).len`, a `filter` with a constant predicate;
- an `is_some` or `is_ok` of a value whose shape is public;
- `first`, `at` or `first_where(keep: fn(y) -> true)` tested with `is Some`;
- `first` returning `Option[secret Int]`, which keeps the element's label;
- a `map` to a constant, or a `fold` that counts.

`filter_len` drops only the result U4001. Its predicate `fn(x) -> x > 0` is still U4001 at the lambda.

## Counts

| Severity | Count | Findings |
| --- | --- | --- |
| Critical | 0 | |
| Major | 0 | |
| Minor | 0 | |
| Note | 2 | V11-N1, V11-N2 |

**Adversarial programs:** 4 new, in `adversarial/`. All hold.

## Baseline, at `65388fa`

| Command | Result |
| --- | --- |
| build, `fmt --check`, clippy `-D warnings` (`--locked`) | clean |
| `cargo test --workspace --locked` | 356 passed, 0 failed, 0 ignored |
| `RAYON_NUM_THREADS=1 cargo test --workspace --locked` | 356 passed, 0 failed |
| Conformance runner | 656 cases: 656 passed, 0 pending, 0 failed |
| Without Z3, cold caches | 138 cases fail, each with U5003 only; 0 pending, none skipped |
| Examples, cold: `una check`, `una test` | all 7 exit 0; 106 obligations proven; all 106 cached on the warm run |
| Determinism | cold `una --json obligations` byte-identical by default and with `RAYON_NUM_THREADS=1`, for all 7 examples |
| rustdoc `-D warnings`, release build | clean |
| CI on `65388fa` | green: push run 36906486406, PR run 36906548310 |

## Rerun of the 255 programs of the earlier reviews

`../v0.1-final-review/scripts/rerun-adversarial.sh` ran every program in a cold copy at `65388fa` and at `f5df029`; the outputs are in `rerun/`. Three programs changed codes:
- `v0.1-rereview/rv-gen-prelude-option-labels`: loses U4002 and keeps U4001. The U4002 was an effect under `is_some(xs.first())` in a generic, which depends only on whether `xs` is empty. Its `b`, `append([1], item: pin).len`, is also accepted now, and it never depended on `pin`.
- `v0.1-review/flow-policy-as-value` and `flow-via-as-value`: each loses a cascade U4001 on `ts.map(each: mask)`, whose result is `List[Text]`. Both stay rejected by U2001 and U4005.

## Notes

### V11-N1. The PR counts 59 dropped U4001s; there are 67
The expectation diff and both binaries agree on 67 (see above). Every one is sound, so only the PR text and the commit message are off.

### V11-N2. `spec/14 §1` gives `fold`'s top label as the tops of `xs` and `step` only
`fold`'s result is the accumulator `U` itself. The implementation keeps `U`'s own top label and joins those of `xs` and `step`, and that is what makes `v11-fold-top` sound. A reader of the spec line could implement a replacement instead, which would leak. Suggested wording: "`fold`: the label of the accumulator type, joined with the top labels of `xs` and `step`".

## Files

- `summary.md`: this file.
- `adversarial/`: 4 programs, each with a header (attack, expected, observed, verdict).
- `rerun/`: codes of the 255 earlier programs and the 4 new ones at both heads, and the dropped U4001s with their source lines.
