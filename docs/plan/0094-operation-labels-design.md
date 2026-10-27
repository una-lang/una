# ADR 0094: per-operation labels in edition 0.3 (design note)

- Issue: #566 (ADR 0094, accepted 2026-10-11; formal extension #563)
- Base: `main`; edition `0.3` only. Editions `0.1` and `0.2` keep today's rules, and the Gate 1 pin does not move.
- Normative text, landing with the slices: `spec/01 §5`, `spec/02 §3`, `spec/05 §3`, `§4`, `§9`, `spec/07 §2`, `§3`, `spec/13 §4` (U3022), `spec/13 §6` (H5)

This note fixes how ADR 0094 is built, in three slices, and lists the decisions it takes where the ADR leaves a choice. Each decision takes the most secure option the ADR allows. Numbers in brackets are the ADR's rules.

## 1. What lands, in what order

| Slice | Delivers | Rules |
| --- | --- | --- |
| S1 (#566 part 1) | `gives` in a header's `uses` brackets; the label on the binding; the answer, the failure decision and the failure payload carry it; passing a labeled binding to a declared callee, by name or by `with`, and rebindings; U3022 with its repairs; U3006 for a function value with labeled `uses`; U0103 before edition `0.3`, U0101 in function types, U7002 in ceilings; the registry row and the explain text; a scaling test | 1, 2, 3 (declared callees, rebindings), 4, 6, 7 |
| S2 (#566 part 2) | Inferred labels of private functions (the join over the bindings their callers pass, in the `uses` fixpoint); locked labels under `[contracts]`; `una contract` prints `gives`; B09's scaffold in `una-bench` (labels on `role` and `owner`, the lock) builds its reference and refuses its violation | 3 (inference), ADR 0092 rule 8, Consequences |
| S3 (#566 part 3) | H5 in the build report's Assumed zone with every operation label it is used for, in the status page's list of host premises, and in the re-check's note; a labeled operation reads as not re-checked until #563 lands | 5, 8 |

S1 is sound on its own: until S2, a private function with inferred `uses` whose callers pass a labeled operation is refused at the call (U3022), since an inferred header has no `gives` and a labeled operation flows only to a callee that reads it at the label or above (rule 3, second bullet). S2 lifts that by inference; nothing it adds loosens S1.

## 2. Syntax (S1)

- **Grammar** (`spec/02 §3`). `cap_op = lower_ident [ "gives" label ] [ "where" line_expr ]`, with `label = level [ "[" upper_ident { "," upper_ident } "]" ]` as a type's label is written (`spec/07 §2`). `gives` comes before `where` [1]; the other order is U0101 "expected `,`, `where` or `]`". The AST's `CapType` gains `gives: Vec<Option<Label>>`, one per operation like `conditions`.
- **Where it parses.** Only `cap_type` of a function's or a handler's header `uses` in edition `0.3` (`uses_clause(conditions: true)` today; the same flag admits `gives`), and the lone clause of a `[contracts]` string (S2). A function type's `uses` keeps `cap_type` without it: `gives` there is U0101 in every edition [1]. A `[boundaries]` ceiling names operations only, so `gives` in a ceiling is U7002 from the project reader [1].
- **Keyword** [6]. `gives` is a contextual keyword, an identifier everywhere else, as `of` is: the lexer does not change. In editions `0.1` and `0.2`, a header's `uses` brackets that read `gives` after an operation name are U0103 at `gives`, "`gives` is a keyword of edition 0.3 (operation labels)", a parse error of the clause (decision D1: the parser reports it where `where` is refused today, so the two forms of ADR 0092 and ADR 0094 are refused alike before edition `0.3`). `gives` is not reserved: a function or a local named `gives` is accepted in every edition, and so is an operation named `gives`, since the word is contextual only after an operation name.
- **Categories.** A category after `gives` is checked as a type's category is: undeclared is U4007 at the category [1].
- **Formatter.** The canonical form is `op gives level[Cats] where c`, one space around `gives` and `where`, as the tokens are written; a conformance case under `fmt/` pins it.

## 3. The label on the binding (S1)

- `Bound::Cap` and the function's `FnUse` carry `gives: BTreeMap<String, Label>`, keyed by operation; absent means `⊥`, and `⊥` is never stored, so a header without `gives` is unchanged in every structure, Core IR included (decision D2: Core IR and `una.core/1` do not change in S1; #563 adds what the formal track reads, under ADR 0076).
- A rebinding `with @x = @y` keeps `@y`'s labels; `with @x = @y[ops]` keeps those of the operations it keeps [3]. `resolve_in` returns the labels with the binding it leads to.
- A handler installed with `with @x = h(args)` or `given @x = h(args)` answers at `⊥` in the installing body, since that body's own header names no label on `@x`; inside a callee that receives it under `uses @x: C[op gives l]`, the callee's own header labels the answers [4]. A handler cannot lower a label: it has no `gives` of its own, and its operation bodies are checked against the capability's declared result label as today.

## 4. The answer, the decision and the payload (S1) [2]

At `@x.op(args)` where `@x`'s binding labels `op` with `l` (`⊥` when unlabeled):

1. **The value** returned is `def.result` with `l` joined deep, at every position of the result type (decision D3: `raise_deep`, not the top label only, so a list of owners or an `Option[Text]` carries the label in each component; the ADR says the answer "carries" the label, and a component that did not would be a flow around it).
2. **The decision.** `decided`, the label of whether the operation fails, joins `l` (`spec/07 §3` rule 4 for host operations, and now for every binding): the `when`, `attempt` or `if error` that branches on the outcome runs under `pc ⊔ l`, through `decided_failures` as ADR 0090 does for a policy's failure. So `if error pass` and `if error fail`, a `fail` in a branch on it, and the function's own failure are U4002 where the function's failure is observed below `l` (S-18).
3. **The payload** is the failure kind with `decided` raised deep, as today for host operations.

Nothing downstream changes: U4001, U4002 and U4003 report the flows, and their repair texts name the binding and operation whose label the value carries, "the answer of `@service_files.owner`, labeled `internal[Service]`" [7].

## 5. Passing a labeled binding (S1 for declared callees, S2 for inferred ones) [3]

At a call of a named function `f` whose header lists `uses @x: C[op gives l′, ...]`, for each operation `op` of each binding, with the caller's binding labeling `op` with `l` (the resolved binding's label, through rebindings and narrowings):

- `l ⊑ l′` passes: `f` reads the answers at `l′` or above.
- otherwise U3022 at the call, "`@service_files.owner` gives `internal[Service]`, which is not within what `owner_seen` reads it at (`public`)". A callee that lists `op` without `gives` reads it at `⊥`, so a labeled operation reaches it only when `l = ⊥`.
- The check is one label comparison per operation of each need at each call site: linear in call edges times operations, with the labels already joined.
- `with @x = h(args)` installs a handler: the callee reads its answers at its own `l′`, and the handler answers at `⊥` for the caller (§3), so nothing is compared. `with @x = @y` compares `@y`'s labels as a passing by name does.
- A function value: a named function whose `uses` carry a label is U3006 where it is used as a value [1], with the message "`f` labels an operation it uses, so it is called by name only: a function type carries no `gives`". A lambda cannot write `gives` (its expected type is a function type), so a lambda that calls a labeled operation of the enclosing body reads it at the body's label, which is sound, and its own result carries it.
- **Repairs of U3022** [7], in order, neither lowering a label: (1) when the caller holds another binding of the same capability whose `op` label is within `l′`, pass it (`with @x = @other`), an exact edit at the call, ranked first; (2) raise the callee's `gives` to `l ⊔ l′` (the join, so that an incomparable label is never dropped), a description, offered only to a callee with a header (a named function or a handler), since it changes the callee's callers. One U3022 per need, listing its operations, and only when the capability and the operations themselves pass (U3001 and U3002 come first).
- **Inference (S2).** A private function with inferred `uses` infers labels too: for each binding and operation, the join of the labels its callers pass, in the fixpoint of `infer` (`spec/05 §9` rule 2). The join is monotone in a finite lattice (five levels, the declared categories), so the fixpoint terminates; each binding's label changes at most `4 + |categories|` times, and the pass stays linear in call edges times operations. Until S2, such a call is U3022 (section 1).

## 6. Locks and contracts (S2)

- A `[contracts]` string parses `gives` as a header does [ADR 0092 rule 8]. For a locked function, the header's `gives` on an operation must be at least the lock's: a header that drops or lowers a locked label is U3023 at the operation, "the header of `create_share_links` reads `@service_files.owner` at `public`, which `una.toml` locks at `internal[Service]`", with the exact edit that restores the locked label (as the restore of a locked condition), which `una fix` applies.
- `una contract` prints `gives l` on the operations of a header, declared or inferred, in its `uses` strings; `una.contract/1` keeps its keys (the text of a string changes only where a label is written).

## 7. H5 and the report (S3) [5, 8]

- The build report's Assumed zone: `assumptions` gains a `premises` key (additive, `una.report/2` stays): the host premises the package rests on, `H5` listed with every labeled operation of the package as `binding.op gives l` under the function that labels it. H1 to H4 are listed there too, by the host capabilities the package uses, so an agent reads the whole set in one place.
- `docs/plan/formal-track-status.md`, "What accepted rests on", gains H5 as the ADR states it.
- The re-check (ADR 0088's `una verify --recheck`, where it exists) reads a declaration that labels an operation, or calls through a labeled binding, as not re-checked, with a note naming H5, until #563 extends F1 and T6.

## 8. Diagnostics

- **U3022** (new, `authority`): "Labeled operation passed to code that reads it lower". Registry row in `spec/13 §4`, explain text with a failing and a fixed example (B09's shape), repairs as in §5.
- **U0103** gains the `gives` message in editions `0.1` and `0.2` [6]; **U0101** for `gives` in a function type; **U7002** for `gives` in a ceiling; **U4007** for an undeclared category; **U3006** for a labeled function used as a value; **U3023** for a lock (S2).
- The U4001, U4002 and U4003 that a labeled answer causes keep their codes; their repair texts name the binding and operation [7].

## 9. Tests

- Conformance (S1): `syntax/gives_in_0_3` (parses and formats), `syntax/gives_before_0_3` (U0103 at `gives` in `0.2`), `syntax/gives_in_function_type` (U0101), `project/gives_in_ceiling` (U7002), `security/gives_answer_flows` (U4001 at the flow to the result), `security/gives_decision_pc` (U4002 under the decision), `security/gives_failure_passed_on` (U4002 at `if error pass`), `authority/gives_passed_lower` (U3022), `authority/gives_passed_within` (ok, `l ⊑ l′`), `authority/gives_rebinding` and `gives_narrowed` (labels kept), `authority/gives_function_value` (U3006), `security/gives_category_undeclared` (U4007), `fmt/gives_canonical`.
- S2: `authority/gives_inferred` (ok through a private helper), `authority/gives_inferred_lower` (U3022 at the helper's own labeled use), `project/gives_locked` (U3023 and its fix), `una contract` snapshot.
- S3: a report snapshot with `premises`, and the status page.
- Scaling (S1, `crates/una-cli/tests/`): a chain of `n` functions each passing a labeled binding to the next and reading one labeled operation, for `n` in 200, 400, 800: check time grows linearly (the comparison per call edge), asserted with the suite's growth helper.
- Editions: no edition `0.2` conformance package writes `gives` in a header, so `editions.rs` lists nothing new; the `0.3` cases that write it are refused by `0.2` as `Syntax` differences only if they are also `0.2` packages, which none is.
- `una-bench` B09 (S2): the scaffold's `links.una` writes `role gives internal[Service], owner gives internal[Service]` on `@service_files`, `una.toml` declares the category `Service` and locks `create_share_links`; the reference builds, the violating variant fails with U4001 at the refusal and U4002 at the mail under the decision.

## 10. Decisions

- D1 `gives` before edition `0.3` is U0103 at the word, reported by the parser where `where` is refused, so the two edition `0.3` forms of a `cap_op` are refused alike.
- D2 Core IR and `una.core/1` do not change in S1 to S3; #563 extends them.
- D3 The answer's label is joined deep, at every position of the result type.
- D4 A labeled operation passed to a callee that lists it without `gives` is U3022 unless the label is `⊥`; an inferred header reads every operation at `⊥` until S2 infers its labels.
- D5 The repairs of U3022 never lower a label; passing another binding of the caller ranks first and is the only exact edit.
- D6 The report lists H5 beside H1 to H4 under `assumptions.premises`, additive.
