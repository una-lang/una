# Formal track: the arithmetic primitives

- Issue: #526, under #46
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11), with the abort as its own outcome; in progress

## 1. Why

`una.core/1` writes 19 primitive kinds (`spec/13 §8`). F1 had `add`, `sub`, `lt` (and `gt` with its operands swapped), `eq`, `not`, `len`, `concat` and `to_text`. The importer reported every other kind as "the primitive `k`", not re-checked. `neg`, `mul`, `div`, `mod`, `le`, `ge` and `ne` are 294 uses in the corpus (`docs/reviews/security-hunt/coverage.md` §3, "cheap model gains"). They label like `add`: the result carries the join of every label of the operands (`spec/07 §7` rule 3), `ℓ*` for `!=` as for `==`.

## 2. The model

- `Prim` gains `mul`, `div` and `mod`, typed `[Int, Int] → Int` (`PrimTy`, `Check.primTy`), labeled by `Program.primLabel` as `add` is.
- `div` and `mod` are Euclidean, with a non-negative remainder (`spec/11`, `spec/12` SMT-LIB `div` and `mod`, `spec/16`, `spec/18`): `Int.ediv` and `Int.emod`. So `-7 div 2 = -4` and `-7 mod 2 = 1`.
- **A zero divisor aborts.** `Res` gains `aborted`, a defined runtime outcome of Una, which aborts on it (exit 1). It is not `outOfFuel`, which is an artifact of the evaluator's fuel. A later theorem about fuel, such as monotonicity or adequacy against the interpreter, would otherwise cover aborts silently. `Prim.aborts` (a `div` or `mod` whose divisor is `0`) is checked before the primitive is evaluated. The run's outcome is then `aborted`, neither a value nor a failure. The fixed-width overflow abort of ADR 0057 can share it later.
- T1 counts `aborted` as good, as it counts `outOfFuel`: a typed program never gets *stuck*. T6's `Term` excludes it.

## 3. The importer

- `mul`, `div` and `mod` are read as themselves.
- `neg a` is `sub [0, a]`: the literal is `public`, so the label is `a`'s.
- `le`, `ge` and `ne` have no primitive of their own. `let x: T = le(a, b)` is read as `let t: T = lt(b, a)`, then `let x: T = not(t)`. `ge` uses `lt(a, b)`, and `ne` uses `==`. The temporary has `x`'s own annotation, so every flow is checked against the declared type. `not` keeps its operand's label, which is Una's for each: the join of the operands', and `ℓ*` for `!=`.

## 4. What T6 says, and what it does not

T6 is termination-insensitive (`spec/07 §1`). `Term` excludes `aborted`, as it excludes `outOfFuel` and `stuck`, so two runs of which one aborts are outside its premise. Una's soundness at a division rests on two things:
- the divisor obligation, U5001 (`spec/12 §3`: "a nonzero divisor");
- `spec/07 §1`, which puts aborts and termination outside the model.

**The residual.** A program reaches the import with a divisor that depends on a secret when the obligation was discharged under assumptions, or escaped. Such a program has a termination channel: whether it aborts. The same holds for the fixed-width overflow abort of the native backend (ADR 0057). No theorem covers either, and nothing here claims more. The class is that of S-16: an abort is termination.

## 5. Theorems

No theorem changes its statement. The proofs gain the new primitives and the `aborted` outcome.

Non-vacuity, `ArithExample`:
- `mul_rejected`, `mul_leaks` (the runs print `2` and `4`) and `mul_untyped`: `print(to_text(len(s) * 2))`, with `s` read from the environment as `internal`.
- `public_mul_checked` and `public_mul_runs`: `3 * 4` is accepted, typed, and prints `12`.
- `div_euclid` and `mod_euclid`.
- `div_by_zero_aborts`: `7 div 0` gives `aborted` at every fuel that reaches it.

`ImportTests`:
- each new kind at public operands, accepted;
- with an `internal` operand into a public result, rejected;
- `!=` on records with an `internal` field, rejected.

## 6. Gate

The change accepts more programs, so before merge it needs:
- a corpus delta: a fresh export of the base `main` with its own `una`, both sides' `una-core-check` on the same files, every flip listed by document, and the headline split by what `una check` says of each document;
- the replay;
- `scripts/formal.sh`;
- local CI;
- an adversarial review.
