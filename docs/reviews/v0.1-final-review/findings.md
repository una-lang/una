# Findings of the limited re-review

- **Scope:** issue #3, on PR #1 (`dev/v0.1` into `main`), head `879c26a` (confirmed with `gh pr view 1`).
- **Paths:**
  - `F` is `docs/reviews/v0.1-final-review/adversarial/`.
  - `RR` is the re-review's `docs/reviews/v0.1-rereview/adversarial/`.
  - `A` is the first review's `docs/reviews/v0.1-review/adversarial/`.
  - `una` is the debug CLI built from `879c26a`, with Z3 5.1.0.
- **Evidence:** only what this reviewer reproduced. The Critical below was reproduced on a copy with a cold proof cache, and its origin was checked on binaries built from `85787d7` and `c2f3386`.

## 1. Status of the items in scope

### Findings of the re-review

| Item | Status | Evidence at `879c26a` |
| --- | --- | --- |
| N-001 | Fixed | `RR/rv-flow-record-field-label-equality` is U4003. `rv-flow-sum-field-label-equality` and `rv-flow-field-label-equality-variants` now report U4001 at every probe, not only at the controls (5 diagnostics each, against 2 at `85787d7`). |
| N-002 | Fixed | `RR/rv-gen-kind-contravariant`, `rv-gen-kind-contravariant-flows` (all 5 sites) and `rv-hdl-result-kind-variance` are U4001. The 7 variance forms of `F/fr-variance-forms` and the generic forms of `F/fr-generic-invariant` hold. |
| N-003 | Fixed | `RR/rv-gen-generic-fn-value-label` is U4003. `let`-bound generic values in `F/fr-generic-invariant` hold. |
| N-004 | Fixed (ADR 0047) | `RR/rv-verify-via-requires` and `rv-verify-via-param` are U4005, as are the first review's `escape-policy-opaque-fact` and `verify-policy-to-refinement`, which used the same shape (U5001 before). `F/fr-via-shapes` holds. |
| N-005 | Fixed | `RR/rv-verify-fn-refinement-annotated-let` is U5001 at all four lets. `F/fr-fn-refinement-lets` (`when`, `if none use`, operation result, field, range-sugar function kind) holds. |
| N-006 | Fixed (ADR 0045) | `RR/rv-verify-refined-fn-kind` is U2002 in the debug build and in a release build of `879c26a` (exit 1). No `cfg!(debug_assertions)` is left in the library crates. |
| N-007 | Fixed (ADR 0044) | `do safe(1)` then `expect fails Broke` is U1001. A `do attempt ...` line still passes (F-003, Minor). |

### ADRs 0044 to 0049

| ADR | Implemented as written | Spec text | Notes |
| --- | --- | --- | --- |
| 0044 | yes | `spec/10 §12` rule 1 | F-003 is a case the ADR does not name |
| 0045 | yes: U2002 at the clause, an alias reported once, release equals debug | `spec/03 §3` | ADR 0038 D1's body still says "debug-build" (F-006) |
| 0046 | yes at every site of its rule 3 | `spec/07 §7` rule 2 | operation arguments are not covered (F-001) |
| 0047 | yes | `spec/07 §5` rule 4, `§7` rule 5 | two message flaws (F-004) |
| 0048 | yes: the report's `out_of_model` has 7 entries, the last "hard links" | `spec/07 §1`, `spec/13 §6`, `spec/14 §3` | |
| 0049 | yes for functions, generics, method calls, operations, handlers and the prelude; function values are positional | `spec/04 §3` rules 2 and 3 | |

### Gate 1 issues

The coordinator's message lists #68, #69 and #71. The owner's comment on issue #3 also adds #70 and #72, so all five were checked.

| Issue | Status | Evidence |
| --- | --- | --- |
| #68 (UI-009) | Fixed | `F/fr-arg-binding-forms` and `fr-arg-binding-prelude-handler`: U2006 for swapped bare first arguments at 8 call forms (method call, generic, operation, three prelude functions, a handler in `with` and in `given`); function values bind positionally. |
| #69 (UI-005) | Partially fixed | Correct at an operation site. At a call whose callee needs the operation, the only repair still widens the attenuated binding at rank 1 (F-002). |
| #70 (UI-002) | Closed as reported, correctly | `List[Nat]` is U2002 (ADR 0040); accepting it is a backlog change. |
| #71 (UI-008) | Fixed, and sound | `F/fr-lambda-param-refinement`: a lambda with a `Pos` parameter flowing where `fn(Int) -> Int` is expected is U5001 at all 9 sites. |
| #72 (UI-004) | Fixed as far as the closure claims | The U5001 message names the uninterpreted product. The status rule (refuted against unknown) is the owner's open question (F-008). |

## 2. New findings

### Critical

#### F-001. An operation argument is not a label flow into the operation's parameter type, so function parameters and invariant kind arguments leak
- **Area:** information flow (T6).
- **Reference:**
  - `spec/07 §7` rule 2 (S-Fun: flows are contravariant through function parameters), and ADR 0046 rules 2 and 3 ("call arguments");
  - `spec/07 §7` rule 4, which says that "the labels of the operation's parameter types do not bound its arguments". That exemption is right for data positions, which `accepts` governs, but it also drops the contravariant and invariant positions.
- **Evidence:**
  - `F/fr-op-arg-variance.una` declares `op open(s: Sink[secret Int]) -> Int` and `op apply(f: fn(secret Int) -> Int) -> Int`, with default `accepts public`.
  - The program passes `made()`, a `Sink[Int]` built from `fn(x: Int) -> x`, and a named `ident(x: Int) -> Int`.
  - The handler `holding(pin: secret Int)` calls `s.eat(pin)` and `f(pin)`, which type-check against the declared parameter types, and returns the results as the operation's public `Int`.
  - `una check` exits 0. `PIN=4321 una run --grant env --grant console` prints `pin is 4321 and 4321`.
  - The same holds under `List` and a generic `Box` (`F/fr-op-arg-fn-components.una`).
  - Controls:
    - the same values as handler instance arguments are U4001 (`F/fr-handler-arg-contravariance.una`);
    - a failing function at an operation argument is U3009 (`fr-op-arg-fn-fails`);
    - a function-type refinement at an operation argument is U5001 (`fr-op-arg-fn-refinement`).
    So only labels are skipped.
- **Origin:**
  - The `85787d7` binary accepts the program and prints the PIN.
  - On the `c2f3386` binary, the only diagnostic is the old `to_int` label rule in `main`, and both attack functions are accepted.
  - So the gap predates the first review. Neither earlier review listed operation arguments as a flow site, and neither did the closure's flow matrix.
- **Root cause:**
  - `crates/una-check/src/types/call.rs:688-716` gives operation arguments only to `sink`, which bounds the argument's own labels by `accepts`.
  - With D-164, that covers the labels a function value carries, but never the flow from the argument into the declared parameter type.
  - Handler bodies then assume the declared type.
- **Fix:** needs a change to `spec/07 §7` rule 4; see escalation E1. The recommendation keeps `accepts` for data positions and adds the flow check at contravariant positions and at invariant kind arguments. Add an `op_argument` row to the closure's flow and variance matrices.

### Minor

#### F-002. U3002 at a call site still ranks widening the attenuated binding first
- **Reference:** issue #69 acceptance criterion "no repair widens an attenuated binding at rank 1 when an alternative exists"; `spec/05 §3`.
- **Evidence:** in `F/fr-repair-attenuated-call.una`, `put_back(count)` needs `@front: Shelf[unstock]`. The caller holds `@front: Shelf[stock]` and `@back: Shelf[unstock]`. U3002's only repair, at rank 1, is "add `unstock` to the `uses` clause of `@front`". `put_back(count) with @front = @back` is available and is not offered. The fix in `10b5614` covers only operation sites (`authority.rs`, `binding_permitting`).
- **Fix:** offer the `with` rebinding at rank 1 at call sites when another binding permits the operations, and rank widening second.

#### F-003. `expect fails V` after `do attempt f(...)` passes `una check`
- **Reference:** ADR 0044, `spec/10 §12` rule 1, D-170.
- **Evidence:** `F/fr-expect-fails-do.una`: `do attempt risky(0)` then `expect fails Broke` gives no diagnostic. `una test` then reports "the `do` line completed with `Err(Broke)`". D-170 option A counts a call whose failure is absorbed as able to fail, and `attempt` absorbs it.
- **Fix:** D-170 option B (see `decisions.md`). It does not block the tag.

#### F-004. Three message flaws in new diagnostics
- The ADR 0037 obligation description names an internal temporary: "refinement `result of %t3: result > 0` is refuted" (`F/fr-fn-refinement-lets.una`, line 30, the `if none use` default).
- U4005 says "must take a `Int`", and for a generic `via` prints `fn(?) -> ?` (`F/fr-via-shapes`).
- **Fix:** name the expression as written, choose the article, and print type variables by name.

### Notes

#### F-005. Invariant flows are worded as a flow from `secret` to `public`
`` `s` is `Sink[secret Int]`, and this value is `Sink[Int]`: a flow from `secret` to `public` ``: the value is less labeled than the target. Under ADR 0046, the problem is that the labels differ at an invariant argument. Wording such as "labels must be equal at the invariant argument `T`" would be accurate.

#### F-006. ADR 0038 D1 body still says "A debug-build consistency check"
The header records the amendment by ADR 0045, but the paragraph was not updated.

#### F-007. Over-rejections, all sound
- **N-020 is inconsistent:** `fn() -> x == y` is rejected as a generic's returned lambda, but accepted inside `Src(get: ...)`. The call site is still caught in both forms (`F/fr-generic-return-strip.una`).
- **ADR 0046 makes a parameter invariant even when it appears only in a function result,** as in `get: fn() -> T`. ADR 0046 decides this, and it is conservative.
- **`g != Miss` on a sum with a `secret` field is now `secret`** (`RR/rv-flow-sum-field-label-equality`, `differ`), although it compares only the tag.

#### F-008. UI-004's status question remains open
A refutation through uninterpreted arithmetic is still reported as `refuted` with a counterexample that may not be an execution. The message now says so. Either status rejects the program, so soundness is unaffected. The closure stopped and reported this as a `spec/12 §6` question for the owner.
