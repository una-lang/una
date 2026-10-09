# Formal track: M3, the generic-free half

- Issue: #46
- Plan: `docs/plan/formal-track.md` §7 (M3: T6 for F1 without generics)
- Review of M1: `docs/plan/formal-track-m1-review.md`
- Date: 2026-10-09
- Status: the generic-free half is proven; the re-estimate below is for the owner

## 1. What is proven

All of it is in `formal/UnaFormal/F1/`, and `scripts/formal.sh` passes with 46 stated theorems, none depending on `sorry`.

| File | Result |
| --- | --- |
| `Inst.lean` | The owner's condition 3, first in M3. Instantiation commutes with the semantic reading of labels (`LExpr.interp_inst`), with the top and in-place lemmas of labeled instantiation. |
| `Order.lean` | The static order of labels and flows are preorders, and joins are least upper bounds. |
| `Unary.lean`, `UnarySound.lean` | **The unary half** (`unary_sound`). A run of a body the labeled typing accepts does three things. It performs only the events its effects allow, each at a sink whose `accepts` is above the program counter: T2, with the program counter, which is confinement. It fails only with a failure labeled above the program counter: T3 with labels. It ends with a value of its labeled type. |
| `Binary.lean`, `BinaryLemmas.lean` | The two-run relation for an observer at `L`, and its lemmas. |
| `T6.lean` | **T6 for the generic-free part of F1** (`t6`), at one fuel. |
| `Mono.lean` | Fuel monotonicity (`eval_mono`): a run that ends at some fuel ends the same way at any larger one. So `t6_ended` compares any two runs that end, whatever fuel each needed. |
| `T6Example.lean` | A worked instance. A typed body computes with a `secret` input, prints a line to a `public` sink and returns its `public` input. `t6` is applied to two runs whose secrets differ, and the runs are computed. |

**The theorem.** Take two runs of a body that the labeled typing accepts, at the same fuel. Suppose their environments have the context's labeled types and an observer at `L` cannot tell them apart. Whenever both runs end:

- the observer sees the same events;
- they end with related values: equal scalars, texts and shapes at the positions the observer sees, closures with the same code and related environments, and anything at `Unit`;
- they fail with the same failure when the observer sees what decides failure;
- one fails while the other ends with a value only when the observer does not see what decides failure.

**The relation.** Two values are related at a labeled type in two cases:

- the type's top label is hidden from the observer;
- or they have the same shape and related parts.

A related pair of closures at a visible function type has the same code and related captured environments. A visible closure comes from the same program point in both runs, so the relation needs no step-indexing, and the proof is one induction on fuel.

**Every form of F1 is covered:**

- primitives, calls, function values with decisions, the operation and the host operation;
- records, lists, options, lambdas and named functions as values;
- the prelude's `map`, `filter`, `first_where`, `fold` and `parse`;
- `==` over values with equality;
- `let`, `if`, `case` on options and results, `fail`, `attempt` and handlers.

**Scope, stated as `T6.lean` states it.** The theorem covers F1 as `Syntax.lean` defines it, and no more:

- one capability with one operation and one host operation, each with one `accepts` for the whole program;
- a stateless host (S-14 and S-15 are open);
- failures as codes, without payloads;
- no sums but `Option` and `Result`, no `Map`, no `with` or handlers of capabilities, no `together`, no generics and no declassification.

`T6Example.lean` has a worked instance and its negative:

- **The instance.** A body branches on a `secret` comparison, so two runs with different secrets take different branches and return 1 and 2. They show a public observer the same line, and their values are related at the hidden type (`branching_noninterferent`).
- **The negative.** A body that writes a different line in each branch of a `secret` condition has no typing (`leaking_untyped`).

## 2. Assumptions, and what M3 found

T6 here assumes the following. Each assumption is a hypothesis of `t6` or part of the model, and is stated in `T6.lean`.

1. **The generic-free part.** No function has type parameters, and the program has no policy (`Program.GenericFree`). Generics and declassification are M4's.
2. **The host's response is labeled at least what its operation accepts** (`hget`). A response may depend on everything the request sent. `crates/una-check/tests/host_table.rs` checks this property of the prelude's host table, so the model and the compiler cannot drift apart. Every host operation of `spec/14 §3` and `spec/24` meets it.
3. **The host is a function of the request and is the same in both runs.** Its failures depend only on the arguments that decide them, and no operation of the program changes it.

**What M3 found.** Checking assumption 3 against the real host table found two violations:

- **S-14:** a file's existence. A write to an `internal` path is observed by a read of a `public` path, whose failure is `public`. The Coordenador reproduced it on v0.3.3, in both editions, and on v0.1.5.
- **S-15 (a candidate):** a server's failures. They can depend on a request's body and header values, which do not label them.

Their fix, under way separately, discharges the assumption. The model can then drop it, or model state.

**`spec/10` against the model and the checker.** The model has two rules that `spec/10`'s formal rules lack:

- its T-Let runs the body after a failable right-hand side under the label of what decides the failure;
- its T-Handle runs the handler under that label and raises the value by it.

Both rules follow `spec/07 §7` rule 4 (edition 0.2); `spec/10`'s formal T-Let and T-Handle do not have them. The shipped checker is not laxer than the model, because the surface language cannot write the programs where the two differ. Checked with `una check` (debug build of 2026-10-09) in both editions:

- `if error pass` and `if error fail` on a failure decided by an `internal` path are U4002;
- a handler with an effect is U3013, since an effectful call must be a whole statement;
- a failure recovered by `attempt` lets the next statement run, which is right, since `attempt` does not fail.

So there is no finding. `spec/10` needs an append-only correction to state the two rules as `spec/07` does.

No rule of `spec/07` or `spec/10` had to change for the proof. The readings of the M1 review §4 held as written.

## 3. What remains

**Of M3:** nothing. Fuel monotonicity and the worked example are proven too (`t6_ended`, `T6Example.lean`).

**Of M4**, held until the adversarial review of `t6`'s statement returns:

- generic functions: the labeled substitution lemma, with T-App's decisions over `ℓ*_T`;
- declassification, for "up to declassification";
- T2 and T3 as corollaries, most of which `unary_sound` already gives.

## 4. Effort, and the re-estimate

| | Estimate (plan §7) | Actual |
| --- | --- | --- |
| M1 | 5 to 8 agent-days | About 40 minutes, the owner's conditions included |
| M3, generic-free half | Part of 8 to 12 agent-days | About 35 minutes of one session, 2026-10-09 from 11:42 to 12:16 UTC: `interp_inst`, the unary half, the relation and T6 |

That time includes checking the host table, which found S-14 and S-15. About 45 compile iterations were needed, all of them Lean API and elaboration details.

**Re-estimate.**

- **The rest of M3:** one session-hour or less.
- **M4:** a few session-hours. The uncertainty is the labeled substitution lemma: whether the static order of rule 8 is stable under instantiation, so that a generic body's typing transfers to every instance. If it is not, the rule changes, and that would be a finding.

The table's agent-day ranges overestimated proof volume by about two orders of magnitude. On M1 and M3's evidence, the cost of the track is in finding where the specification and the shipped checker disagree with the model, as S-14 shows, not in proof engineering.

## 5. Notes for the checker (#440)

- **`ℓ_T` only in place.** The model's reading of `ℓ_T` needs a plain symbol to label only a position of its own type parameter (`LTy.InPlace`, M4). The checker places plain symbols only on type parameters (`crates/una-check/src/types/ty.rs`). The labels that Core IR gives an operation's arguments and interpolation temporaries now star them, so the export keeps the invariant too; `crates/una-cli/tests/core_symbols.rs` checks it over the corpus, and `summary.rs` asserts it in debug builds.
- **`when` on a value of `T`.** `when x`, with `x` of a type parameter, raises the program counter by the plain label of `x` (`crates/una-check/src/types/stmt.rs`, `fn when`). That is harmless today because such a `when` can have only an `else` arm: no arm tests the value. A rule that let arms inspect a value of `T` would need the raise to use `ℓ*_T` instead.
