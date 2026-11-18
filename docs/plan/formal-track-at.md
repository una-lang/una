# Formal track: `prelude.at`

- Issue: #545, under #46
- Date: 2026-10-11
- Status: statement approved (2026-10-11); implemented, in review

## 1. Why

After #542, `prelude.at` is the next prelude function the re-check does not read. It is the reason 3 of the replay's S-04 `edit` shapes stop short of their leak (`edit_order_loop_nested_list`, `edit_order_loop_nested_list_of_options`, `edit_order_loop_nested_result`).

## 2. Una's rule

`at[T](xs: List[T], index: Int) -> Option[T]`, `ensures (result is None) == (index < 0 or index >= xs.len)` (`spec/14 §1`). It is structural: "`at`: the top label of `xs` and every label of `index`". The element keeps its own labels inside the option. The checker reads it so: `crates/una-check/src/types/call.rs`, `structural_inputs`, `at` takes `xs`'s top label and `index`'s deep label.

## 3. The model

- `Rhs.at (list index : Atom)`, its own form. Read as `first_where`, or raised by the element's labels, it would reject what Una accepts.
- **T-At:** `xs : List[te] at lx` and `index : Int at li` give `Option[te] at lx ⊔ li`, with no effect. An `Int`'s only label is its top label, so `li` is every label of `index`.
- **Evaluation:** `atIndex xs n`, `Some(xs[n])` for `0 <= n < xs.len`, else `None`. It is total: no failure and no abort, so its decision label (#524) is `⊥`.
- **The importer** reads `prelude.at [xs, index]` as `.at xs index`, and lists `prelude.at` in `preludeF1`.

## 4. Theorems

No theorem changes its statement. T1, the unary and binary halves of T6, erasure, the host-free simulation, `InPlace`, substitution, monomorphization, the checker's soundness, B1 and `Decision.lean`'s `dlOk` gain the case.

In T6, when the observer sees `lx ⊔ li`, the lists are related element by element, so they have the same length, and the indices are equal, so both runs give `None`, or `Some` of related elements (`VAll.atIndex`). Otherwise the option's top label is hidden.

Non-vacuity, `AtExample`:
- `at([s], 0)`, a public list of an `internal` text at a public index, matched only for `Some` or `None` into a public print: accepted, certified, and it runs;
- the same, printing the element: rejected, it leaks, and it has no typing;
- `at([7, 8], len(s))`, a public list of public integers at an `internal` index, matched for `Some` or `None` into a public print: rejected, it leaks (`some` and `none`), and it has no typing. The index decides whether there is an element.

`AtImportTests`, from `una`'s export of `formal/gen/at_import_tests.una`, give each verdict as `una check` does:
- a public list at a public index: accepted;
- the element of a list of secret elements, read into a public result: rejected;
- a list of secret elements matched only for `Some` or `None`: accepted;
- a secret index deciding a public result: rejected.

## 5. Gate

- `scripts/formal.sh` and `docs/reviews/formal-m2-replay/check.sh`, each changed line of `expected.txt` explained in review before merge;
- the corpus delta, a fresh export, with every flip listed and none rejected;
- the hunt's F rerun;
- local CI;
- an adversarial review.

Result (2026-10-11, on `main` at `97294021`):
- `scripts/formal.sh`: PASS, 352 theorems, no `sorry`. The constructor is `Rhs.elemAt`, since `at` is a keyword of Lean's tactics.
- The replay: 42 to 45 rejected (S-04: 10 to 13), the three shapes that stopped at `at`, each until now "not re-checked: a call of `prelude.at`":
  - `edit_order_loop_nested_list` and `edit_order_loop_nested_list_of_options`: at `at` itself, whose annotation in the v0.2.0 dump drops the elements' `internal` label;
  - `edit_order_loop_nested_result`: now read, the check stops at the `or_else` two lines later, whose result the dump annotates `public` although its `Err` holds an `internal Int`: every label of its arguments, `internal`, is the result's under Una's rule and #542's, and the result is printed through `to_text` two lines on.
- The corpus, a fresh export of the 1,352 targets: 1,213 to 1,216 accepted lines, none rejected, so the source headline is 1,196 to 1,199. The flips, all in documents `una check` accepts: `aliasing.main` (`runtime/in_place_aliasing`), `structural_prelude_labels.nth` and `at_secret`. `together_move.main` stays not re-checked, now for `together_for`.
- Family F, seeds 5 to 7 and 11: no change; every program `una check` accepts is `clean`.
