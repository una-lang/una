# Adversarial check of v0.3.2 (PR #358, #354): summary

- **Candidate:** `1d3aaf0` on v0.3.1 (`4e48a99`). **Reviewer:** Claude (Una-2). **Author:** Una-1.
- **The change:** edition 0.1 tells `ℓ*_T` from `ℓ_T` in its flows, as edition 0.2 does (`deep_distinct` in both editions).
  - Before it, edition 0.1 read `T*` as `T` in every flow. A written annotation therefore dropped a decision on values of `T` (H-6 of the v0.3.1 review, `docs/reviews/v0.3.1/findings.md`).

## Verdict at `1d3aaf0`: Ready

| Area | Result | Evidence |
| --- | --- | --- |
| The v0.3.1 matrix, 194 cells (`docs/reviews/v0.3.1/matrix`) | `FAIL: 0` in sections 1 to 15. On v0.3.1, section 15 failed in 5 cells. 90 cells gain a code, all of them already rejected, by an added U4001. The controls are unchanged; `launder__effect__generic_identity__control` gains a second U4001 and stays a `rule`. | `results/matrix-1d3aaf0.txt`, `results/matrix-v0.3.1.txt` |
| The runnable repros (`docs/reviews/v0.3.1/cases/run`) | `let-annotation-direct`, the last one that ran, is U4001. Every other leak stays rejected. The no-decision control prints the same for both secrets. The edition 0.2 forms are unchanged. | `results/run-forms-1d3aaf0.txt` |
| The 30 attacks (`docs/reviews/v0.3.1/attacks`) | No case changes between accepted and rejected. 16 rejected cases gain U4001. | `results/attacks-v0.3.1-vs-1d3aaf0.txt` |
| The 47 cases of the v0.1.4 review (release/v0.1, `docs/reviews/v0.1.4/cases`) | No case changes between accepted and rejected. 21 rejected cases gain U4001. The 13 edition 0.2 and the 8 N-39 cases are byte-identical. | `results/v0.1.4-cases-v0.3.1-vs-1d3aaf0.txt` |
| Sound programs (`probes/`) | Three shapes are newly rejected in edition 0.1, exactly as edition 0.2 rejects them; eight stay accepted in both. | `results/probes-v0.3.1-vs-1d3aaf0.txt` |

## Sound programs that edition 0.1 now rejects

The probes are 11 sound generic programs, each in edition 0.1 and 0.2, checked on v0.3.1 and `1d3aaf0`.

**Newly rejected in edition 0.1 (U4001).** Edition 0.2 already rejects each of them in the same way:
- `let y: T = if a == b then p else q`, then `y` returned as `-> T` (`annotated_local_returned`);
- `let xs: List[T] = [if a == b then p else q]`, returned (`annotated_list`);
- `let h: Hold[T] = Hold(value: if a == b then p else q)`, returned (`annotated_record`).

**Accepted in both editions, before and after:**
- the decision returned directly as `-> T` (`return_decided`);
- the decision through a `let` without annotation (`unannotated_local_returned`);
- the decision passed to a generic function that stores it (`stored_by_generic_param`);
- a record built and returned without annotation (`record_returned_unannotated`);
- `xs.append(item: …)` (`append_to_param_list`);
- an `Option[T]` from an `if` (`option_returned`);
- a decided `Bool` returned (`decided_bool_returned`);
- nested generic calls (`nested_generic_return`).

At each concrete call site, a `secret` instantiation stays `secret` and a `public` one stays `public`.

**The rule for authors**, the same in both editions:
- In a generic body, a value decided by comparing values of `T` cannot be bound to a written annotation that mentions `T` (`T`, `List[T]`, a kind applied to `T`).
- Omit the annotation instead: inference keeps the decision's label, and the call site raises its result by the type argument's label.
- The return type `-> T` is fine.

## Running it

```sh
cd docs/reviews/v0.3.1/matrix && UNA=<una> ./run-matrix.sh   # FAIL: 0
cd docs/reviews/v0.3.2/probes && python3 -I gen.py && OLD=<una v0.3.1> NEW=<una> ./run-probes.sh
```
