# Formal track: M6b, `to_text`, `concat` and `Env.get`

- Issue: #485, under #46
- Previous step: `docs/plan/formal-track-m6.md` (M6, #478)
- Date: 2026-10-10
- Status: plan approved by the coordinator (2026-10-10), with the answers of §8; proofs in progress

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
