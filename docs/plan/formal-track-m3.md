# Formal track: M3, the generic-free half

- Issue: #46
- Plan: `docs/plan/formal-track.md` §7 (M3: T6 for F1 without generics)
- Review of M1: `docs/plan/formal-track-m1-review.md`
- Date: 2026-10-09
- Status: the generic-free half is proven; the re-estimate below is for the owner

## 1. What is proven

All of it is in `formal/UnaFormal/F1/`, and `scripts/formal.sh` passes with 39 stated theorems, none depending on `sorry`.

| File | Result |
| --- | --- |
| `Inst.lean` | The owner's condition 3, first in M3. Instantiation commutes with the semantic reading of labels (`LExpr.interp_inst`), with the top and in-place lemmas of labeled instantiation. |
| `Order.lean` | The static order of labels and flows are preorders, and joins are least upper bounds. |
| `Unary.lean`, `UnarySound.lean` | **The unary half** (`unary_sound`). A run of a body the labeled typing accepts does three things. It performs only the events its effects allow, each at a sink whose `accepts` is above the program counter: T2, with the program counter, which is confinement. It fails only with a failure labeled above the program counter: T3 with labels. It ends with a value of its labeled type. |
| `Binary.lean`, `BinaryLemmas.lean` | The two-run relation for an observer at `L`, and its lemmas. |
| `T6.lean` | **T6 for the generic-free part of F1** (`t6`). |

**The theorem.** Take two runs of a body that the labeled typing accepts, at the same fuel. Suppose their environments have the context's labeled types and an observer at `L` cannot tell them apart. Whenever both runs end:

- the observer sees the same events;
- they end with values related at every position the observer sees;
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

## 2. Assumptions, and what M3 found

T6 here assumes the following. Each assumption is a hypothesis of `t6` or part of the model, and is stated in `T6.lean`.

1. **The generic-free part.** No function has type parameters, and the program has no policy (`Program.GenericFree`). Generics and declassification are M4's.
2. **The host's response is labeled at least what its operation accepts** (`hget`). A response may depend on everything the request sent. `crates/una-check/tests/host_table.rs` checks this property of the prelude's host table, so the model and the compiler cannot drift apart. Every host operation of `spec/14 §3` and `spec/24` meets it.
3. **The host is a function of the request and is the same in both runs.** Its failures depend only on the arguments that decide them, and no operation of the program changes it.

**What M3 found.** Checking assumption 3 against the real host table found two violations:

- **S-14:** a file's existence. A write to an `internal` path is observed by a read of a `public` path, whose failure is `public`. The Coordenador reproduced it on v0.3.3, in both editions, and on v0.1.5.
- **S-15 (a candidate):** a server's failures. They can depend on a request's body and header values, which do not label them.

Their fix, under way separately, discharges the assumption. The model can then drop it, or model state.

No rule of `spec/07` or `spec/10` had to change for the proof. The readings of the M1 review §4 held as written.

## 3. What remains

**Of M3:**

- **Fuel monotonicity.** A run that ends at some fuel ends the same way at any larger one. `t6` compares two runs at the same fuel, so this lemma is what lifts it to two runs that end at different fuels. The lemma is routine.
- **A worked non-vacuity example of `t6`.**

**Of M4:**

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
