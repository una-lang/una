# Cross-review of Map and Text for v0.3.0: findings

Candidate `5495b61d65508fe1d26785f335d861799a58757f`, and a delta pass on `bc05a8d` (after #320). Severity as in `plan.md`. Each case is a package under `cases/`, and `cases/run.sh` runs them all.

## M-1, Minor: a generic function cannot count by key

**Area 1, Map semantics; area 2, labels.** Case: `cases/1/generic-keys`.

```una
tally[T: Ord](xs: List[T]) -> Map[T, Int]
=
  let edit m = empty_map()
  for x in xs
    let n = m.get(key: x)
      if none use 0
    edit m.set(key: x, value: n + 1)
  m
```

- `una check` reports U4001 at `n + 1`: "`m` is `Map[label(T*) T, Int]`, and this value is `Map[label(T*) T, label(T*) Int]`". The result is the same with the annotation `Map[T, Int]`.
- **Why.**
  - `get`'s top label carries the key's labels, which are `ℓ*_T` in a generic body (`spec/21 §3` rules 2 and 5), so `n` is `T*`-labeled.
  - An editable binding keeps its declared labels (rule 4), and the value position of `empty_map()` is fixed `public` when the binding is made.
  - No source syntax names `ℓ*_T` in a type.
- **The rejection is sound and conservative.** The same body with concrete `Text` keys verifies, and with `secret Text` keys one can declare `Map[secret Text, secret Int]`. The cost is that the commonest map idiom (counts, grouping, indexing by a key) cannot be written once for every key type.
- **Decided (N-43):** option (a), #323 with ADR 0075, after the v0.3.0 tag. For an un-annotated `let edit`, the label positions that the initializer leaves undetermined become label variables, solved as the join of every assignment in the body.
- **In v0.3.0** it is a known limit, with the concrete-key workaround. Una-1 adds it to #322's CHANGELOG.

## Notes (not findings)

- **N-1. The length of an interpolation is unknown to the verifier.** After `needle = "{needle}b"`, `needle.len > 0` is unknown (U5002) (`cases/5/scaling`). This is `spec/22 §7`: there is no theory of strings. A ground fact `len("{x}lit") = len(x) + len(lit)` for interpolations of literals and texts would be cheap in QF_UFLIA, if the owner wants it later.
- **N-2. A map argument with any labeled position is redacted whole in a trace,** as `<redacted:internal>`, identically in both engines (`cases/8/trace-map-redaction`). `spec/11 §5` allows it, and it is conservative. A structural redaction, key by key, would show more of public keys, but nothing requires it.
- **N-3. A text `fold` that starts from a literal copies the literal once,** at every size (delta pass, `cases/10/delta-function-values`). This is O(1), as ADR 0072's N-42 amendment states.
