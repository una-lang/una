# Formal track: `prelude.first`

- Issue: #532 (its `prelude.first` part), under #46
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11); in progress

## 1. Why

The security hunt's family F (part (b), #513) compares `una check` with the re-check on programs inside the importer's vocabulary. Over seeds 5 to 7, 92 user declarations were "not re-checked: a call of `prelude.first`": the `map` carrier (60) and the `filter` carrier (32), which take a list's head. `first` is common in real code too.

## 2. Una's rule

`first[T](xs: List[T]) -> Option[T]`, `ensures (result is None) == (xs.len == 0)` (`spec/14 §1`). Its result carries the top label of `xs` only (`spec/14 §1`: "`append`, `first` and `is_empty`: the top label of `xs`"): whether there is a first element is the list's length, which the top label decides; the element keeps its own labels inside the option.

## 3. The model

- `Rhs.first (list : Atom)`, its own form. Reading it as `first_where(xs, fn(_) -> true)` would label it wrongly: T-FirstWhere raises the option by the elements' labels and the predicate's result, so a public list of secret elements, matched only for `Some` or `None` into a public sink, would be rejected where Una accepts it, a false finding for part (b).
- **T-First:** `xs : .list te lx` gives `.option te lx`, with no effect. Structurally, `.list t` gives `.option t`.
- **Evaluation:** the head as `some v`, or `none` for the empty list. Total: no failure, no abort.
- **The importer** reads `prelude.first [xs]` as `.first xs`, and lists `prelude.first` among the prelude functions F1 gives a semantics to (`preludeF1`).

## 4. Theorems

No theorem changes its statement. T1, the unary and binary halves of T6, erasure and the host-free simulation gain the case. In T6, when the observer sees the list's top label, the two lists are related at that label, so they have the same length, and their heads are related at the element type; when it does not, the option's top label is hidden.

Non-vacuity:
- `first` of a public list of secret elements, matched only for `Some` or `None` into a public print: accepted and certified, and it runs.
- The same printing the head: rejected, it leaks, and it has no typing.

`ImportTests`:
- a public list, accepted;
- a list with a secret top label, its option matched into a public print: rejected;
- a list of secret elements whose head is read into a public result: rejected;
- a list of secret elements matched only for `Some` or `None`: accepted.

## 5. Gate

- `scripts/formal.sh`;
- `docs/reviews/formal-m2-replay/check.sh`;
- the corpus delta: a fresh export of `main`, both checkers on the same files, every flip by document, and the headline split by what `una check` says of the document;
- the hunt's F rerun on seeds 5 to 7, with the 92 `prelude.first` declarations reached;
- local CI;
- the adversarial review.
