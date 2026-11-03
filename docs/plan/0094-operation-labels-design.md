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

## 6. Slice S2, exactly: inferred labels, locks, `una contract`

### 6.1 Inferred labels of private functions (rule 3, third bullet)

- **Where.** `crates/una-check/src/types/authority.rs`, `infer`: the fixpoint that gives a private function without a `uses` clause its bindings and operations (`spec/05 §9` rule 2). Today it joins, per inferred binding, the operations its body performs and the operations its callees need. S2 adds, per inferred binding and operation, a label: the join over the labels that the function's callers pass for it.
- **The data.** `Collected.calls` already records each call edge with the callee's needs resolved in the caller's frames. S2 records beside each need the label the caller's binding gives each operation (`given(&gives, op)` of the resolved `Bound::Cap`, through rebindings and narrowings, `public` for a handler): `CallNeed { callee, name, ops, gives: Vec<(String, Label)> }`. One extra vector per call edge, filled where `needs_site` resolves the binding in pass 1.
- **The rule.** For an inferred function `f` and its inferred binding `@x`, `gives_f(@x, op) = ⊔ { l | some caller passes a binding to f's @x whose op gives l }`, over all callers, direct and through `with @x = @y` (which keeps `@y`'s labels) and `with @x = @y[ops]` (which keeps the kept ones). A caller that installs a handler, `with @x = h(args)`, contributes `public`. A function value or a lambda never contributes, since it reads at `public` (rule 1) and its own call is checked at its site.
- **Monotone, terminating.** The join is monotone in a finite lattice: five levels and the declared categories, so each `(f, @x, op)` changes at most `4 + |categories|` times. The existing loop over `collected.calls` runs until nothing changes; S2 adds the label join to the same loop, so the pass stays linear in call edges times operations, times at most the lattice height.
- **The result.** The inferred `FnUse.gives` of `f` holds the joined labels (`public` left out). From then on everything of S1 applies to `f` as to a declared function: its body reads `@x.op` at the inferred label (the answer, the decision and the payload carry it, §4), its callers are compared against it (§5: every caller passes by construction, since the join is at least each caller's label), and the functions it calls see its inferred labels as their callers' (the fixpoint propagates downward through chains of inferred functions). `una contract` shows them as inferred (6.3).
- **Recursion and cycles.** Inferred functions that call each other converge as the operations do; a cycle's labels are the join over the cycle's outside callers.
- **The edge that stays refused.** A declared-header callee reads at its written label, never an inferred one: a private function with a declared `uses` that lists the operation without `gives` still gets U3022 when a labeled binding reaches it (D4), since a header is a promise to the reader. The repair advice names the header.
- **Diagnostics.** None new: a labeled answer that an inferred function returns `public` is U4001 at its body, as for any function; the message names the binding and operation whose label the value carries (rule 7, second bullet, below).
- **Rule 7, second bullet (repair texts).** The U4001, U4002 and U4003 that a labeled answer causes gain, in the message's advice and in the first repair's description, "the answer of `@service_files.owner`, labeled `internal[Service]`"; the checker knows the source of a label through the `Ty` the answer carries, so S2 keeps, in `Checker`, a map from the span of each operation call to its `(binding, op, label)` and names the first whose label is not within the sink when it reports the flow. Where a value joins several labeled answers, the message names the first in source order. One lookup per flow report; no cost on the clean path.

### 6.2 Locked labels under `[contracts]` (ADR 0092 rule 8)

- **The lock parses `gives`.** `una_syntax::parse_uses_clause` already lexes with `KeywordSet::V0_3` and calls `cap_type(true)`, so a locked clause `"@service_files: Files[owner gives internal[Service] where not dry_run]"` parses in S1; S1 refuses it with U7002 in `lock.rs`. S2 removes that refusal and reads the lock's labels: `Entry` gains `gives: Vec<(&str, Label)>`, lowered with `lower_label` against the package's categories (an undeclared category in a lock is U7002 at the clause: a lock cannot name what the package does not declare).
- **The rule, for each locked operation of a locked header.** Let `l_lock` be the lock's label on `op` (`public` when the lock writes none) and `l_src` the header's (`public` when none, inferred labels never apply: a locked function is public and declares its `uses`). The header MUST read the operation at least as high as the lock: `l_lock ⊑ l_src`, else U3023 at the header's operation, "the header of `create_share_links` reads `@service_files.owner` at `public`, which `una.toml` locks at `internal[Service]`". A header that gives more than the lock passes: it reads the answer higher, which grants less to its callers' flows (a stronger promise), as a dropped operation or a strengthened condition passes today.
- **Why `⊑` and not equality.** The lock is the consumer's floor on what the function's callers may expect: a caller that passes a binding labeled `l` needs `l ⊑ l_src`; raising `l_src` only accepts more callers' bindings and labels the function's own reads higher. Lowering it is the drift the lock exists to catch.
- **The repair.** The first repair of the U3023 carries the exact edit that restores the locked label on the header's operation: it writes `op gives l_lock` where the header writes `op`, or `op gives (l_lock ⊔ l_src)` where it writes a lower label, keeping the operation's condition in place after the label. `una fix` applies it, as it applies the restore of a locked condition (ADR 0092, amendment of 2026-10-15). The edit changes the module only, never `una.toml`.
- **With conditions.** A lock may write both: `owner gives internal[Service] where not dry_run`. The label rule above and the condition rule of ADR 0092 (a `contract_lock` obligation) are checked independently, in that order, so a header that fails both gets two diagnostics at the operation.
- **A binding the lock labels and the header leaves out** grants less, and passes, as today. A binding the header writes with `gives` and the lock does not name is U3023 at the binding already (an unlocked binding).
- **Complexity.** One label comparison per locked operation: linear in the locks.

### 6.3 `una contract` and `una.contract/1`

- `una contract <module.function>` prints the header's `uses` as text, declared or inferred. S2 prints `gives l` after each operation that gives a label, before its `where` condition, in the capability's declaration order: `uses @service_files: Files[create_link, owner gives internal[Service]]`; an inferred header prints its inferred labels the same way, marked inferred as its operations are today (`uses_inferred`). The keys of `una.contract/1` do not change: the text of the `uses` strings changes only where a label is written, which is additive for consumers that read the keys (ADR 0076).
- `una report`'s `generic_results`, `result_labels` and the report's other label fields do not change in S2.

### 6.4 B09 in `una-bench` (Consequences)

- The scaffold's `src/sharing/links.una` writes `uses @service_files: Files[create_link, role gives internal[Service], owner gives internal[Service]]` on `create_share_links`; `una.toml` gains `[labels] categories = ["Service"]` and `[contracts] "sharing.links.create_share_links" = ["@service_files: Files[create_link, role gives internal[Service], owner gives internal[Service]]", "@requester_files: Files[role, owner]", "@mailer: Mailer[send]"]`. Edition `0.3`.
- The reference builds: it reads `owner` and `role` through `@requester_files` (`owner_seen`), and creates links through `@service_files`, whose `create_link` is unlabeled.
- The violating variant fails: its `owner` through `@service_files` answers at `internal[Service]`; the refusal naming the owner flows into `ShareSummary`, U4001 at the flow; a branch on the lookup's outcome raises the program counter, so the mail under it is U4002. The oracle's predicates (`oracle/predicates.json`) name those two codes.
- This is a `una-bench` change, filed as its own issue when S2 is READY, with the `una` version it needs.

### 6.5 Tests of S2

- Conformance: `authority/gives_inferred` (ok: a private helper without `uses` called with a labeled binding reads at the inferred label and returns it `internal[Service]`), `authority/gives_inferred_lower` (U4001 at the helper's body when it returns the answer as `public`), `authority/gives_inferred_chain` (two inferred helpers in a row), `authority/gives_inferred_join` (two callers with different categories: the helper reads at the join, and the caller with the lower label still passes), `authority/gives_declared_private_lower` (a private function with a declared header without `gives` stays U3022), `project/gives_locked_ok`, `project/gives_locked_lower` (U3023 with the edit), `project/gives_locked_undeclared_category` (U7002), `project/gives_locked_with_condition` (both checks).
- `crates/una-cli/tests/lock_edits.rs`: `una fix` restores a locked label, and keeps a condition after it.
- `crates/una-cli/tests/`: a `una contract` snapshot of a declared and an inferred header with labels.
- The scaling test of S1 gains a chain of inferred helpers (no `uses` clauses) of 200 and 1,600 functions: linear.

## 7. Slice S3, exactly: H5 and the report

### 7.1 The report's Assumed zone (`spec/13 §6`, rule 5)

- **Where.** `crates/una-cli/src/report.rs` builds `assumptions`, today the capabilities with their operations' `ensures` clauses. S3 adds one additive key at the document's top level, after `assumptions`: `premises`, the host premises the package's acceptance rests on, each `{"id": "H1", "statement": "...", "used_for": [...]}`:
  - `H1` to `H4` and the `Env` note, with `used_for` the host capabilities and operations the package uses that each premise covers (`Files.read` for H1, `Files.write` for H2, `Http` for H3 and H4, `Env` for the `Env` note), listed from the existing `host_capabilities` and the operations the authority pass records;
  - `H5` with `used_for` every labeled operation of the package, as `"<module>.<function>: @<binding>.<op> gives <label>"`, in declaration order, declared and inferred alike, and `[]` when the package labels no operation, in which case `H5` is still listed (the premise set is the model's, not the package's). The statement is the ADR's: "what an operation answers through a binding where it is labeled l′, and whether it fails, is a function only of the arguments and of the part of the host state at or below l′".
- **Why a new key and not `tcb` or `assumptions`.** `assumptions` lists what the package declares of its capabilities; `tcb` lists code. A host premise is neither: it is what the deployment must make true. An added key keeps `una.report/2` (ADR 0076).
- **Human output** of `una report` prints a section "Assumed of the host" after the assumptions, one line per premise with its uses.
- **`docs/spec/13 §6`** gains the key, and the sentence that a premise is listed with what it is used for.

### 7.2 The status page and the re-check note

- `docs/plan/formal-track-status.md`, "What accepted rests on", gains H5 after H4 as the ADR states it, with the note that F1 does not model operation labels until #563, so a declaration that labels an operation, or that calls through a binding whose label is above `public`, is accepted on the checker's rules alone, not re-checked by the formal model.
- The re-check note of the formal track (the verdict's note that names the premises used) lists H5 for such declarations once #563 lands; until then S3 adds the sentence above to the status page only, since the note is produced by the formal track's tooling, not by `una`.

### 7.3 Tests of S3

- A report snapshot (`crates/una-cli/tests/`, beside the existing `una.report/2` snapshot) for a package with one labeled operation: `premises` holds H1 to H5 and the `Env` note with the right `used_for`; and for a package without labels or host use: every premise with `[]`.
- `crates/una-cli/tests/guide.rs`'s schema list is unchanged (no new schema).

## 8. Diagnostics (all slices)

- **U3022** (new, `authority`): "Labeled operation passed to code that reads it lower". Registry row in `spec/13 §4`, explain text with a failing and a fixed example (B09's shape), repairs as in §5.
- **U0103** gains the `gives` message in editions `0.1` and `0.2` [6]; **U0101** for `gives` in a function type; **U7002** for `gives` in a ceiling, and for a lock that writes `gives` in S1, or an undeclared category in a lock in S2; **U4007** for an undeclared category; **U3006** for a labeled function used as a value; **U3023** for a lock whose header reads lower (S2), with the exact edit.
- The U4001, U4002 and U4003 that a labeled answer causes keep their codes; their repair texts name the binding and operation [7].

## 9. Tests

- Conformance (S1): `syntax/gives_in_0_3` (parses and formats), `syntax/gives_before_0_3` (U0103 at `gives` in `0.2`), `syntax/gives_in_function_type` (U0101), `project/gives_in_ceiling` (U7002), `security/gives_answer_flows` (U4001 at the flow to the result), `security/gives_decision_pc` (U4002 under the decision), `security/gives_failure_passed_on` (U4002 at `if error pass`), `authority/gives_passed_lower` (U3022), `authority/gives_passed_within` (ok, `l ⊑ l′`), `authority/gives_rebinding` and `gives_narrowed` (labels kept), `authority/gives_function_value` (U3006), `security/gives_category_undeclared` (U4007), `fmt/gives_canonical`.
- S2 and S3: as §6.5 and §7.3 list them.
- Scaling (S1, `crates/una-cli/tests/`): a chain of `n` functions each passing a labeled binding to the next and reading one labeled operation, for `n` in 200, 400, 800: check time grows linearly (the comparison per call edge), asserted with the suite's growth helper.
- Editions: no edition `0.2` conformance package writes `gives` in a header, so `editions.rs` lists nothing new; the `0.3` cases that write it are refused by `0.2` as `Syntax` differences only if they are also `0.2` packages, which none is.
- `una-bench` B09 (S2): the scaffold's `links.una` writes `role gives internal[Service], owner gives internal[Service]` on `@service_files`, `una.toml` declares the category `Service` and locks `create_share_links`; the reference builds, the violating variant fails with U4001 at the refusal and U4002 at the mail under the decision.

## 10. Decisions

- D1 `gives` before edition `0.3` is U0103 at the word, reported by the parser where `where` is refused, so the two edition `0.3` forms of a `cap_op` are refused alike.
- D2 Core IR and `una.core/1` do not change in S1 to S3; #563 extends them. (2026-11-03, #1518.) Core's `Use` gains `gives` and `una.core/1` an additive `gives` key on a `uses` entry, so that the re-check can report a labeled operation as not re-checked, as rule 8 requires until #563 lands; without it the re-check could not tell one apart.
- D3 The answer's label is joined deep, at every position of the result type.
- D4 A labeled operation passed to a callee that lists it without `gives` is U3022 unless the label is `⊥`; an inferred header reads every operation at `⊥` until S2 infers its labels.
- D5 The repairs of U3022 never lower a label; passing another binding of the caller ranks first and is the only exact edit.
- D6 The report lists H5 beside H1 to H4 under a top-level `premises` key, additive, each with what it is used for.
- D7 A locked label is a floor: the header must read the operation at `l_lock` or above (`l_lock ⊑ l_src`); the restore edit writes the join.
- D8 Inferred labels are the join over the callers' bindings, in the `uses` fixpoint; a private function with a declared header without `gives` stays refused (D4), since a header is a promise to the reader.
- D9 (S3) H5's `used_for` also lists the labeled operations of handlers' `uses`, after the functions', as `module.handler: @binding.op gives label`: a handler may write `gives` (rule 1), and a premise the package relies on must not be left out of the report. The checker's outcome gains the handlers' `uses` with their labels for it.
- D10 (S2) The label inference is a worklist over the call edges, separate from the `uses` fixpoint and run after it, since labels do not change which operations are inferred: a change of a function's labels reprocesses only the calls it makes, so the pass is linear in call edges times operations, times at most the lattice height, in any source order.
- D11 (S2) A function whose inferred `uses` label an operation is not a function value (U3006), as one whose header writes the label (rule 1); a call missed by the inference, in a lambda or through a function value, is still compared at its site in pass 2 (U3022), so the inference can only make a header read higher, never let a binding reach a read below its label.
- D12 (S2) `una.contract/1` lists the labels in each `uses` entry under an additive key `gives`, after `ops`, left out when empty, so that documents of packages without labels, every package of editions `0.1` and `0.2`, do not change; a consumer that reads `ops` alone would not see a label otherwise (6.3 kept the keys; this takes the more transparent option ADR 0076 allows).
- D13 (S2) The restore edit of a locked label (6.2) is a tier-0 candidate of `una fix`: `spec/13 §5` now says so, since its "never a U3023 of tier 0" was written for the U3023s that carry no edit. Justification (review of #1464, L2): ADR 0084's amendment of 2026-10-15 (`docs/adr/0084-una-fix.md`, "Which tier-1 edits") already scopes the exclusion to "a U3023 of tier 0, for a binding or an operation that the lock does not grant", which "has no obligation and no edit"; it describes the U3023s of that date and does not forbid one that carries an edit. ADR 0084 rule 2's general criteria admit the label restore: its first repair has an edit, no escape cost, and does not widen authority or visibility (`Repair::widens` is false; it adds no binding or operation and never touches `una.toml`). `una-cli/src/fix.rs` never excluded tier-0 U3023, so the behaviour follows from the existing rule. The edit raises the header's label: the body is checked against more, and callers may pass bindings labeled up to it. No accepted ADR text is changed; whether ADR 0094 or ADR 0084 gets a dated note naming this tier-0 kind is left to the owner (ADR 0084 lists its candidate kinds explicitly). The condition restore of the same lock goes after the header's label (`op gives l where C`), so the two-round sequence (label at tier 0, condition at tier 1) writes a module that parses (review M1). The rule 7 repair texts that name the binding and operation of a labeled answer (6.1, last bullet) are not in S2: they change the messages of every flow report and land with their own card.
