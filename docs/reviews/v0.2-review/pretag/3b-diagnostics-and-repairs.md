# Pre-tag review v0.2.0, area 3b: diagnostics and repairs

- Candidate: `ef34f2f604657572da0c8ec97612f2e923549ef8`
- Reviewer: Claude (Una-2), 2026-10-07
- **Verdict: no Critical.** Two Major findings and three Minor ones in mechanical application, plus two cosmetic notes. **No applied repair changed a program's meaning:** every repaired program that checks ran with the intended output.

## Method

- **Scope:** every repair added or changed since v0.2.0-alpha.3 (from `git diff v0.2.0-alpha.3..ef34f2f`):
  - #205, the U2009 `is` repair;
  - #206, reserved binding names (an edit for `let result`, advice otherwise);
  - #211, a same-line handling clause;
  - the U2026 `[T: C]` repair (#172), which #229 also uses for interpolation;
  - ADR 0068 D8's U4003 note, which is advice only.
  - #202 and #187 add no repair.
- **The harness:** `una --json check`, then the rank-1 edit of every diagnostic with the target code is applied mechanically, as an agent would apply one check's repairs, and the package is checked again. Where the program runs, `una run` is compared with the intended output.
- **Corpus:** 61 packages, in edition 0.1 and 0.2 where the construct exists in both. It covers nesting, lambdas, `together`, loops, clauses, examples, text, precedence (`not`/`and`/`or`, operands on either side), shadowing and name collisions.

## Findings

### F3, Major: #206's rename misses uses after a `together` block

```una
  together
    let result = three()
    let b = four()
  @console.print("{result} {b}")
```

- The repair renames `result` up to the end of the block the `let` is in (`uses_in_block`). A task's `let` is visible after the `together` block, though, so the use on the last line keeps `result`.
- After the edit, the program is U2012 (`result` outside `ensures`). It doesn't check, but its meaning doesn't change silently either.
- **Fix:** for a `let` inside a `together` block, extend the scan to the end of the block that holds the `together`; or offer advice only there.

### F5, Major: several U2026 on one type parameter carry conflicting edits

```una
pub both[T](a: T, b: T) -> Bool = a == b or a < b
```

- Each operator gives its own U2026, with an edit of the same span `T`: one to `T: Eq`, one to `T: Ord`. Applying a check's repairs together yields `[T: Ord: Eq]`, which is U0101.
- The same happens for `==` with `to_int()` (`[T: Integer: Eq]`), and for `<` with two interpolations (`[T: Eq: Eq: Ord]`). Applied one at a time, with a check between them, each converges.
- **Fix:** one edit per type parameter, writing the strongest constraint its body needs (their join), on the first diagnostic; the others repeat it as advice.

### F1, Minor: U2009 and U2014 carry the same edit in a clause

- With `requires p != Over`, where `Over` has fields, U2014 (outside the refinement language) and U2009 (needs its fields) each carry `write 'not (p is Over)'` over the same range.
- One of them fixes both. Applying both duplicates the text.
- **Fix:** keep the edit on one diagnostic. The other gives advice or no repair.

### F4, Minor: #211 with two clauses on the line

```una
  let v = fetch(x) if error use None if none use 0
```

- The edit moves the rest of the line, both clauses, to one new line. The second clause is then still on the first clause's line, and that U0101 ("unexpected keyword `if`") has no repair, so applying again doesn't converge.
- **Fix:** break before each clause, each one on its own line at the same depth.

### F2, Minor: #206 renames two `let result` in one scope to the same name

- With `let result = 1` followed by `let result = result + 3`, both become `outcome`, which is U1002. Rebinding in one scope is invalid anyway, but distinct names (`outcome`, `outcome2`), with each use renamed to the latest one, would check.

### Cosmetic

- **N1, D8:** when the text is labeled by another operand (`"{a} {s}"` with `s: secret Int`), the note still names `a`'s `internal` field. That's true, but the larger cause is `s`.
- **N2, D8:** a nested secret (`Outer.inner: Option[Inner]`, with `Inner.pin` secret) is named as field `inner` of `Outer` being `secret`. Naming `pin` of `Inner`, reached through `inner`, would be more precise.

## Repairs that hold (both editions)

- **#205:**
  - `a == V`, `V != a`, `q and p == V or not q`, `not p == V`;
  - `get(a).state == V`, a lambda predicate, `(if c then … else …) == V`, and a `requires` clause.
  - Every one checks, and runs as intended.
- **#206 `let result`:**
  - plain, nested `if` block, loop body, a later function's `ensures` untouched, inside interpolation, a text literal untouched;
  - a fresh name when `outcome` is taken (`outcome2`).
  - Every one checks, and runs as intended.
  - `it`, `error` and other keywords give advice only, as documented. An example's `do` block can't hold a `let` (a separate U0101).
- **#211:**
  - `let … if error pass`, a nested block, `if none use`, an operation statement, and inside `together`: each is moved one level deeper and checks.
  - A `when` arm gets no repair. That's correct, since an arm body can't take a clause on its own line either.
- **U2026:**
  - a single need: `[T]` → `[T: Eq]`;
  - a second parameter: `[A, T]` → `[A, T: Eq]`;
  - a weaker written constraint: `[T: Eq]` → `[T: Ord]`.
- **D8:** advice only, with no edit. It names the labeled field through a list.

The corpus is in the reviewer's scratchpad (`r3b/cases`), with each case's sources before and after.
