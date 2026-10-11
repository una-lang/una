# Formal track: `prelude.first`

- Issue: #532 (its `prelude.first` part), under #46
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11); implemented, in review

## 1. Why

The security hunt's family F (part (b), #513) compares `una check` with the re-check on programs inside the importer's vocabulary. Over seeds 5 to 7, 92 user declarations were "not re-checked: a call of `prelude.first`": the `map` carrier (60) and the `filter` carrier (32), which take a list's head. `first` is common in real code too.

## 2. Una's rule

`first[T](xs: List[T]) -> Option[T]`, `ensures (result is None) == (xs.len == 0)` (`spec/14 §1`). Its result carries the top label of `xs` only (`spec/14 §1`: "`append`, `first` and `is_empty`: the top label of `xs`"): whether there is a first element is the list's length, which the top label decides; the element keeps its own labels inside the option.

## 3. The model

- `Rhs.first (list : Atom)`, its own form. Reading it as `first_where(xs, fn(_) -> true)` would label it wrongly: T-FirstWhere raises the option by the elements' labels and the predicate's result, so a public list of secret elements, matched only for `Some` or `None` into a public sink, would be rejected where Una accepts it, a false finding for part (b).
- **T-First:** `xs : .list te lx` gives `.option te lx`, with no effect. Structurally, `.list t` gives `.option t`.
- **Evaluation:** the head as `some v`, or `none` for the empty list. Total: no failure, no abort.
- **The importer** reads `prelude.first [xs]` as `.first xs`, and lists `prelude.first` among the prelude functions F1 gives a semantics to (`preludeF1`).

## 4. Primitives F1 does not model

The corpus delta found one declaration that `first` made reachable and that the re-check rejected where Una accepts it: `runtime/short_circuit_and_interpolation`'s `main`, which compares texts with `<`. Una orders `Int`, `Text` and variants without fields; F1's `lt` is on `Int` only, so the importer's reading of `<`, `>`, `<=` and `>=` gave F1 an `lt` it has no typing for, and the check rejected it ("a primitive is applied to the wrong types"). The rejection was over-strict, never an accept, but a reading that changes an operator's types is where an accept could slip through.

Una checked every operand's type before the export, so a primitive F1 has no typing for is an overload F1 does not model, not a flow it forbids. The check now names it an F1 restriction, which the re-check reports as not re-checked (#512's rule): `` F1 restriction (`lt` on Text, Text) ``. The sweep covered every primitive the importer maps:

| Una | F1 | Operand types F1 types | Otherwise |
| --- | --- | --- | --- |
| `+`, `-`, `*`, `div`, `mod`, unary `-` | `add`, `sub`, `mul`, `div`, `mod`, `0 - a` | `Int` | F1 restriction |
| `<`, `>`, `<=`, `>=` | `lt`, swapped or under `not` | `Int` | F1 restriction (`Text`, variants) |
| `not` | `not` | `Bool` | F1 restriction |
| `.len` | `len` | `List`, `Text` | F1 restriction |
| interpolation | `concat` | `Text` | F1 restriction |
| `==`, `!=` | `eq`, under `not` | one type with equality in F1 | F1 restriction, as `==` on a sum (#512) |
| `to_text` | `toText` | a type with equality in F1 | F1 restriction, as `to_text` of a sum |

No mapping reads one overload as another: each F1 primitive's typing admits only the operand types it models, and the importer's reading changes only the operand order (`>`) or adds a `not` (`<=`, `>=`, `!=`), whose labels are the operands' as Una's. A primitive the importer does not name is not re-checked, as before.

## 5. Theorems

No theorem changes its statement. T1, the unary and binary halves of T6, erasure and the host-free simulation gain the case. In T6, when the observer sees the list's top label, the two lists are related at that label, so they have the same length, and their heads are related at the element type; when it does not, the option's top label is hidden.

Non-vacuity:
- `first` of a public list of secret elements, matched only for `Some` or `None` into a public print: accepted and certified, and it runs.
- The same printing the head: rejected, it leaks, and it has no typing.

`FirstImportTests`, from `una`'s export of `formal/gen/first_import_tests.una`:
- a public list, accepted;
- a list with a secret top label, its option matched into a public print: rejected;
- a list of secret elements whose head is read into a public result: rejected;
- a list of secret elements matched only for `Some` or `None`: accepted;
- `<`, `>`, `<=` and `>=` on `Text`: not re-checked, and `<` on `Int`: accepted.

## 6. Gate

- `scripts/formal.sh`;
- `docs/reviews/formal-m2-replay/check.sh`;
- the corpus delta: a fresh export of `main`, both checkers on the same files, every flip by document, and the headline split by what `una check` says of the document;
- the hunt's F rerun on seeds 5 to 7, with the 92 `prelude.first` declarations reached;
- local CI;
- the adversarial review.

Result (2026-10-11, una 0.3.7 from `main` at `cee87e3e`):
- `scripts/formal.sh`: PASS, 291 theorems, no `sorry`; the replay: 38 rejected, as `expected.txt`.
- The corpus, 1,352 documents: 1,162 to 1,173 accepted, none rejected before or after. The 11 flips are declarations that call `first`, from not re-checked to accepted, all in documents `una check` accepts: `print_values.firsts`, `c83.fine`, `n07e.leak`, `sw.g`, `sw.leak`, `ifc_tour.first_balance`, `prelude_result_labels.first_labeled` and `public_data`, `structural_prelude_labels.first_secret` and `head`, `callee_ensures_proven.first_or_zero`. `short_circuit_and_interpolation.main` stays not re-checked, now for `` F1 restriction (`lt` on Text, Text) ``.
- Family F, seeds 5 to 7: 113, 62 and 423 to 126, 67 and 497 accepted declarations, the 92 that called `first`; every program `una check` accepts is `clean`.
