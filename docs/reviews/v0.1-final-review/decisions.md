# Decisions D-160 to D-170: recommendations

Each recommendation rests on the programs named, run at `879c26a`. "Keep" means the chosen option is sound and was exercised; "Switch" means the other option is better.

| Decision | Choice | Recommendation | Evidence and reason |
| --- | --- | --- | --- |
| D-160 | A: a branch join at an invariant argument takes the first branch's labels; a differing later branch is U4001 there | Keep | `RR/rv-gen-kind-contravariant-flows` and the closure's branch-join cells are rejected at the differing branch. Sound, and the site names the branch to change. |
| D-161 | A: unification does not raise a variable solved under an invariant argument | Keep | `F/fr-generic-invariant`: `pick(made(), b: typed)` and `pick(typed, b: made())` are both U4001 at the second argument. |
| D-162 | A: a `let`-bound generic function keeps the labels of its unsolved variables, not generalized | Keep | `RR/rv-gen-generic-fn-value-label` is U4003. `through_value` and `through_value_feed` in `F/fr-generic-invariant` hold. |
| D-163 | A: a constructor takes the expected arguments at invariant parameters when they contain no type variable | Keep | `Sink(eat: fn(x: Int) -> x)` built for `Sink[secret Int]` is U4001 at the lambda parameter, so taking the expected arguments loses no label (`direct` in `F/fr-variance-forms`). |
| D-164 | A: sink labels include the labels of function parameters and results | Keep | Conservative and exercised by the closure's sweep. It does not cover F-001: the sink bounds the argument's own labels, while F-001 needs the flow into the parameter type. |
| D-165 | A: Core IR and traces keep field-by-field labels | Keep | Redaction field by field hides every labeled field. The checker's deep label is what guards flows. |
| D-166 | A: a generic function's own symbolic labels are removed from both sides of its return flow | Keep | `F/fr-generic-return-strip`: results derived from `x == y` through a function, a kind with a function field, a `Box` and a list are all caught at the call (U4001), because the call raises the result by the type arguments' labels. |
| D-167 | A: U3016 at the first value whose own type is second-class, with one wording | Keep | `RR/rv-auth-unify-pure-then-effectful` and `rv-auth-wrap-and-generic-value` moved their U3016 to the offending value, with no change in codes. |
| D-168 | A: labels inside a function-typed field do not count in the kind's deep label | Keep | `F/fr-label-fn-field`: a secret-chosen function stored in a record field, a generic `Box`, next to a data field, or sent to an operation is U4001 at the field, or at the annotated `let` that reads it out of the `Box`. The field's own flow check guards the value, and calls of the field are raised by T-App. |
| D-169 | A: an alias of `to` with no `where` of its own matches it | Keep | `F/fr-via-shapes`: `Alias = Pos` is accepted, while `Tight = Pos where it < 10` and another refined kind are U4005. The refinements, which the rule protects, are identical. |
| D-170 | A: a `do` can fail when it calls anything that can fail, even if the failure is absorbed | **Switch to B** | `F/fr-expect-fails-do`: `do attempt risky(0)` then `expect fails Broke` passes `una check` and always fails `una test` (F-003). With B, the error kind after `attempt` and handling clauses is exact, so B never rejects an example whose `do` can fail. Minor; it does not block the tag. |

## Answers block

```
D-160: Keep A
D-161: Keep A
D-162: Keep A
D-163: Keep A
D-164: Keep A
D-165: Keep A
D-166: Keep A
D-167: Keep A
D-168: Keep A
D-169: Keep A
D-170: Switch to B (F-003)
```
