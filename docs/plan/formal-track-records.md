# Formal track: user record kinds in the re-check

- Issue: #509, under #46
- Previous step: #508 (a `Result` keeps its error type)
- Date: 2026-10-10
- Status: findings and plan approved by the coordinator (2026-10-10); implemented, in review

## 1. Why

The importer reads a document's record kinds into F1's records (`Import.records`): their fields' types, and the deep label each one carries (`spec/07 §7` rule 2). It looked for a kind's type parameters under `type_params`, but `una core --json` writes a `data` declaration's as `params` (`crates/una-cli/src/core_json.rs`, ADR 0083). No user record kind was ever read, so every declaration that names one was not re-checked; only `prelude.Header` and `prelude.Response`, which the importer hard-codes for `Http` (#470), were records. The gap cost completeness only: it never accepted anything.

## 2. The reading

A `data` declaration is one of F1's records when:
- its `params` is empty (a generic record is outside F1);
- its `record` flag is `true`. The flag is the exporter's (`una-core` `Decl::Data`), and it separates a record kind from a sum kind with one variant, such as `kind Bad = | Oops`, which has one variant and no fields but is a failure's code (`codeKinds`, #507);
- every field's type is one F1 reads, records included, until no more are dropped (unchanged);
- it does not reach itself through its fields. A recursive record, such as `kind Node` with `next: Option[Node]`, holds trees of any depth, and F1's equality of records unfolds a record only so far (`Program.recordEq`), so `==` over one would be rejected where Una allows it (`for_loop_deep_values`). Such a record, and every record that reaches it, is not read.

A kind that would be both a record and a code kind cannot happen under these rules, since a code kind's flag is `false`. The importer still checks it: a document where one name is both makes the declarations that read it not re-checked.

## 3. Rendering a record

`to_text` of a record is the only way a record reaches text: `print` takes scalars only (`Rhs.op`, `Ty.Scalar`), and interpolation is `to_text`. Una writes a record as `Name(f: v, ...)`, as `Name(v)` with one field, or as `Name` with none (`una-interp`, `una_form`, `Form::Written`, every field). F1 writes `q(v, ...)`, with `q` the record's qualified name.

What soundness needs is that Una's text is a function of the F1 value: then, when two runs give equal F1 values, Una prints equal texts. That holds: the constructor's name and the field names are the record type's, and each field's text is a function of the field's value. The proofs use F1's `Val.render` only as such a function. T6's case for `to_text` rewrites with the equality of the two values when the observer sees `ℓ*` (`V.eq_of_deep`) and relates any two texts otherwise, and erasure needs only that a closure renders as a constant, as Una's `<function>` does. So the theorems hold with Una's rendering in place of F1's.

Injectivity of either rendering is not needed, and nothing here relies on it:
- Una escapes a `Text` inside a compound value (`una_core::quote`: `"` and `\`, and control characters), so its rendering of a record does not let two field texts run together.
- F1 does not: `Val.render` writes a `Text` as it is inside a compound value, so `["a, b"]` and `["a", "b"]` render alike. This costs precision only, and predates #509.

## 4. Theorems

No theorem changes its statement. Non-vacuity, `RecordExample` (statement approved by the coordinator):
- **A record that holds a secret**, `Card(s)` with `Card`'s field an `internal Text`, printed at `Console`: `card_rejected`, `card_leaks` (the runs print `main.Card(1, )` and `main.Card(2, )`), `card_untyped` (T6 refutes every typing).
- **A public record**, `Tag("x")`: `public_card_checked` (the check accepts it) and `public_card_runs` (typed, and it prints).
- **The relation**: `card_rel_hidden`, two `Card` values with different fields related for a `public` observer.

`ImportTests`: a user record read and accepted (`recordShown`), a record with an `internal` field rendered as `public Text` rejected (`recordLeak`), a one-variant sum kind not read as a record (`recordSumKind`), a generic record not re-checked (`recordGeneric`), and a kind flagged a record that is also a code not re-checked (`recordAndCode`).

## 5. Gate

The change accepts more programs, so before merge it needs:
- a corpus delta: a fresh export of the base `main` with its own `una`, both sides' `una-core-check` on the same files, and every declaration that changes its verdict listed;
- an adversarial review;
- `scripts/formal.sh`, the replay and local CI.

## 6. Results

- **`scripts/formal.sh`:** PASS, 243 theorems (235 and `RecordExample`'s 8), no `sorry`. Every `#print axioms` line names only `propext`, `Classical.choice` and `Quot.sound`.
- **Corpus.** A fresh export of `main` `0bd927bf` by its own `una` (examples and `tests/conformance`, 1,344 documents; the branch has no Rust change), with both sides' `una-core-check` on the same files: 765 accepted, 0 rejected, 3,473 not re-checked on `main`; 1,028, 0 and 3,210 with #509. 263 declarations of 66 documents move from not re-checked to accepted; none moves the other way, and none to rejected. 39 change only their reason for not being re-checked, from the record's type to a later form.
- **The headline, split by what `una check` says of the document.** Accepted declarations in documents `una check` accepts: 506 on `main`, **529** with #509 (+23). In documents it rejects: 259 and 499, all for a proof or run-time diagnostic (U5001 228 → 468; U5001 with U5002 20, U5002 4, U5003 2, U6003 1, U6005 4, unchanged), none for information flow. The headline is the first figure.
- **The flips and what `una check` says of them.** 240 come from `tests/conformance/verification/`, whose expected diagnostics are all U5001, a proof obligation refuted (a refinement, `requires` or `ensures`): the information flow of those declarations is sound, and Una rejects them for a refinement, which the re-check does not model. None of the flipped documents expects an information-flow diagnostic (U4xxx). The other 23 expect `ok`, `run-output` or `tier0-ok`, or are examples.
- **A false rejection found and fixed.** The first run rejected `for_loop_deep_values`' `deep.main`, `==` over the recursive `deep.Node`, which Una allows. Recursive records are now not read (§2), with `ImportTests.recordRecursive`; five declarations of three documents stay not re-checked (four of them were accepted in the first run, one rejected).
- **`Header` and `Response`.** The prelude's own declarations are not compared away: no document is not re-checked for "`Header` or `Response`, declared otherwise", before or after.
- **The review (Una-1).** Records were never read from a real export before #509, so the field reader is new trusted base, and the corpus cannot exercise its reject side (a document with a record leak is one `una check` rejects, exported incomplete). `ImportTests` pin it, each rejected case with a `public` twin that is accepted: the field index against the declaration order (`recordFieldIndex`), a constructor's arguments into the fields' types (`recordConstructInto`), a field read raised by the record's label (`recordFieldRaise`), the deep label of a nested record (`recordNestedDeep`) and `==` over a record with an `internal` field (`recordEqInternal`). The search for records that reach themselves takes each edge once, since a record with many fields of one type could push it past the search's bound and miss a cycle (`recordCycleDupEdges`).
- **Replay.** Identical to #508's.

