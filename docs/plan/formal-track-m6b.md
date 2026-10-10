# Formal track: M6b, `to_text`, `concat` and `Env.get`

- Issue: #485, under #46
- Previous step: `docs/plan/formal-track-m6.md` (M6, #478)
- Date: 2026-10-10
- Status: done. Proven with no `sorry` (§9). The labeled-loop cases found a gap in how M6 labels tuples, fixed here (§10)

## 1. Why

S-04's 16 shapes, dumped by v0.2.0, are not re-checked (`docs/reviews/formal-m2-replay/README.md`, "S-04"). Each reads its secret with `Env.get`, and each prints it through interpolation, `to_text` then `concat`, where the dumps show the leak. `concat` is also among the most common reasons a corpus declaration is not re-checked (62).

## 2. The forms

All three are modeled primitives. Each has a semantic function, a typing rule, and the rule's soundness proven in every proof: T1, the unary and binary halves, erasure, host-free code and fuel monotonicity. None is a Lean `axiom`.

| Form | Core | Model | Typing | Its label |
| --- | --- | --- | --- | --- |
| `to_text` | `prim to_text(a)` | `Rhs.toText a`: `Val.render v`, a total rendering of a value as text | the argument's type has equality (`p.eqIn E t`, as `==` asks: no function type) | `deep(t)` (`spec/07 §7` rule 3, `spec/10 §5`) |
| `concat` | `prim concat(a, ...)` | `Prim.concat`, n-ary over `Text` | every argument is a `Text` | `joinAll (deep tᵢ)`, as other primitives (`Program.primLabel`) |
| `Env.get` | `op env.get(name)` of `std.host`'s `Env` | `Rhs.envGet name`: `H.env name`, a function of the host | `name : Text`, `deep(name) ⊑ public`, the `accepts` of `spec/14 §3` | the result `Option[internal Text]`, its tag `public`, as `spec/14 §3` declares |

**`to_text`'s label agrees with the checker.** `una check` 0.3.5, on a probe:
- `"{xs}"` with `xs: List[internal Int]` is `internal Text`: the elements' label;
- with `xs: internal List[Int]`, it is `internal Text` too: the top label;
- `"{a}{b}"` with `b: internal Text` is `internal Text`: `concat` joins.

Each is U4001 when returned as `Text`, and accepted as `internal Text`. That is the deep label of the argument, and the join for `concat`.

**Why `to_text` asks for equality.** Two related values of a type whose labels the observer all sees are equal only at a type with equality (`V.eq_of_deep`, M4). A function value has no text the observer could compare. Rendering the same value gives the same text, so the binary case is `==`'s.

## 3. `Env.get`'s premise

The environment is the host's: `Host.env : String → Option String`. It is the same in both runs, and no operation of the program changes it. This is M3's assumption 3, which `Console` keeps (`docs/plan/formal-track-m5.md` §5, assumption 6). It joins the list of what stays assumed.

**Invalid UTF-8 aborts.** Since v0.3.5 (S-16, ADR 0086), a variable that is not valid UTF-8 aborts the run with U6005. The model's `H.env` returns texts, and an abort is termination (`spec/07 §1`).

**What this premise gives up.** With one environment in both runs, `Env.get` is not a source of secrets in T6: its result is the same in both runs, and its `internal` label is conservative. The replay is about the checker's verdicts on the dumped Core, where `internal` flows into `public` at the interpolation, so it does not need more.

A stronger reading is possible, with the environment's values differing between runs while the names it holds stay the same. It would put the environment in the `World`, related by `WorldRel`, and touch every world of the proofs. It is #487.

## 4. The verdict rules: what changes

- The importer reads `prim to_text` and `prim concat`, and `env.get` of `std.host`'s `Env`, instead of reporting them as not re-checked. `Env.get` stays a sink for the whole-package guard's purposes: it reads no file, so `writeSite` is unchanged.
- No rule of the checker becomes laxer. Each form is checked by its own case, proven sound.
- **The corpus:** declarations whose first gap is `concat`, `to_text` or `Env.get` may move. The diff goes in the PR. Nothing moves into rejected without the coordinator.

## 5. The replay

The 8 shapes that use no other form are re-checked: `edit_order_loop_` `list`, `option`, `result_ok`, `result_err`, `closure`, `edit_param`, `fold` and `generic_init`.
- **Each leaking shape**, dumped by v0.2.0, must be rejected, at the interpolation or wherever the checker meets the wrong label first.
- **Its fixed version** must not be rejected. That needs a version a fixed checker accepts. A shape prints the binder, and its content is `internal`, so raising the binder's label, which is what ADR 0075 does, still leaves the print rejected. The fixed version is therefore the same loop with a `public` sink: it prints the loop index instead, and the `internal` binder is returned at `internal`. It is dumped by v0.3.0 and accepted by it (question 3).

## 6. Labeled-loop conformance cases (the coordinator's request)

The corpus has no loop whose source is labeled, so T-Loop's raise is exercised only by probes. M6b adds accepted and rejected pairs to `tests/conformance/security/`:
- **An `internal`-length list:**
  - accepted: a count into an `internal` state;
  - rejected: a count into a `public` state (U4001, the rebind under a raised program counter).
- **`internal` elements deciding the state:**
  - accepted: `if x > 0` adding to an `internal` state;
  - rejected: the same adding to a `public` state (U4001).

The accepted cases join the corpus, where the re-check must accept them. The rejected ones have no export, since `una core` exports clean declarations only. They are the checker's half of the pair, and `LoopExample.public_count_untyped` is the model's.

## 7. Questions asked (answered in §8)

1. `Env.get`'s premise: one environment, or the stronger reading.
2. `to_text`'s domain.
3. The fixed versions of the replay.

## 8. The coordinator's answers (2026-10-10)

1. **The one-environment premise**, stated and named as §3 writes it, with what it gives up. The stronger reading is its own issue, #487.
2. **`to_text` only where the model matches the checker**, at types with equality. If the checker accepts `to_text` at a type without equality, such a declaration stays not re-checked, which is conservative.
3. **The replay pairs each leaking shape with an existing `*_accepted` control**, a real program that v0.3.0 accepts, not a newly written one. Each control must not be rejected by the re-check: it is accepted, or not re-checked at a stated form. A shape with no counterpart is listed as unpaired, not given an invented one. §5's proposal of a newly written fixed version is withdrawn.

The labeled-loop pairs of §6 are approved. Their rejected halves carry their `expect` lines (U4001).

## 9. What is proven

`scripts/formal.sh` passes with 203 stated theorems, none depending on `sorry` (194 after M6). Every proof carries the three forms.
- **`concat`:** `Prim.concat`, with `concatTexts_typed`. Its binary case is `VList.eq_texts_vis` (and `…σ`): texts whose every label the observer sees are equal.
- **`to_text`:** `Rhs.toText`, rendered by `Val.render`, which reads no annotation (`Val.render_anon`). Its binary case is `==`'s, `V.eq_of_deep` at a type with equality.
- **`Env.get`:** `Rhs.envGet`, from `Host.env`.
  - Its name is `public`, so both runs read the same variable, and the environment is the host's, the same in both runs.
  - It is not host-free (`Rhs.hostFree`), so a declaration that reaches it never gets the any-host note.
  - A document that reads it counts in the whole-document scan of host operations.
  - Its note names `Env` and its premise: "rests on a lawful host (H1 to H3) and one environment in both runs: the program uses `Env`".
- **The importer** reads `prim concat`, `prim to_text` and `std.host`'s `Env.get`.

**#475, folded in (the coordinator's decision).** A declaration whose certified program performs no host operation, but which takes a closure, now carries "rests on a lawful host (H1 to H3): a caller may hand it a closure that uses the host", not "the program uses `Files`". The file-state guard still applies to it (`usesFilesNote`).

**A gap in the importer, found and fixed.** A lambda given to an `of` parameter performs its caller's effects (ADR 0062). The importer gave its type the caller's operation, but not the caller's host effects, so a lambda that calls a function reading a file failed its own type.
- `s14_ok_count_public.probe`, which `una check` accepts, moved into rejected on a first run. Before M6b it stopped earlier, at `concat`.
- Such a lambda now carries its enclosing function's declared host effects. That only widens its declared type, so every use site checks more.

## 10. Tuples carry no label (a finding of the labeled-loop cases)

M6 raised a tuple's own top label by what decides it:
- the loop's value by `ℓs`;
- an `if`, a `case` or a handler at tuple type, Core's join of the bindings it updates, by its condition, scrutinee or failure.

T-LetTuple then pushed that label into the components. Rust labels the components only, and asks each updated binding to be at least the deciding label (U4001). So the re-check rejected two secure programs that `una check` accepts, the accepted halves of §6's pairs:
- a count over a list of `internal` length;
- a count decided by `internal` elements.

M6's corpus had no labeled loop, so none of its 73 accepts met this.

**The fix.** A tuple carries no label of its own. T-Loop's value is `tuple S ⊥`, and new rules `iteT`, `caseOptionT`, `caseResultT` and `handleT` give a join at tuple type the value `tuple ts ⊥`. Each asks `Flows (ts.map (raise ℓ)) ts`: every component is at least the deciding label `ℓ`. A `Unit` component meets it trivially, which is sound only because any two units are related (`V.unit`). In T6's hidden branches, the runs' typed tuples are related component by component, each hidden (`VList.hide_covσ`, `V.tuple_covσ`, `brel_confined_typed`, `brel_quiet_typed`). The checker checks the premise ("a component of the joined state is below …").

**No laxer than Rust.**
- `una check` 0.3.5 gives U4001 on four surface probes, each a `public` count updated under an `internal` decision: by `if`, by `when` on an option, by `when` on a result, and by a loop over a list of `internal` length.
- Their `internal` versions, exported by `una core --json` and with the count's labels lowered to `public`, are each rejected by the re-check at the premise.
- A handler's join is the model's only. Its deciding label is a function's failure, decided at `public` (T-Fun), so no surface program puts a component below it. `LoopExample`'s guards check its premise with a `parse` of `internal` text.

**`Unit` components** (`LoopExample`, `#guard`): a loop over a list of `internal` length whose state is a `Unit` is accepted, and returns only a `Unit`. The same loop with a `public` count is rejected at the premise.

**The pairs.** `loop_label_length_accepted` and `loop_label_elements_accepted` are accepted by the re-check. Their rejected halves carry `expect: error U4001`, at the positions `una check` reports.

## 11. The corpus

The 1,329 documents of M5b, against M6's checker:
- 37 move from not re-checked to accepted, those whose first gap was `concat`, `to_text` or `Env.get`;
- 39 stay not re-checked with another reason;
- none moves into rejected. Accepted: 716 → 753.

The tuple-label fix moves no verdict of the corpus. Two notes change, `hof_run_ok_user_hof_no_decision`'s `sw.apply_eff` and `sw.g`, to the closure note above.

## 12. The replay

S-01 to S-12's verdicts are unchanged. Five notes of S-08's documents change to the closure note: those documents read `Env.get` in `main`.

S-04: 6 of the 8 shapes are rejected exactly at the leak, "the primitive's result, `internal Text`, does not flow into `public Text`": `loop_list`, `option`, `closure`, `edit_param`, `fold` and `generic_init`. `result_ok` and `result_err` build a `Result` with `Ok` and `Err`, which F1 lacks, and are not re-checked. The details and the controls are in `docs/reviews/formal-m2-replay/README.md`.

