# Formal track: M1 and the spike review

- Issue: #46
- Plan: `docs/plan/formal-track.md` (M0 to M2 approved, 2026-10-08, with the go/no-go here)
- Date: 2026-10-09
- Status: for the owner's go/no-go on M3 and M4

M1's exit criterion is met: T1 for F1 is proven without `sorry`, and `scripts/formal.sh` checks that each of the 22 stated theorems depends only on Lean's own three axioms. The plan asks the spike two questions: what the work really cost against the estimate, and whether `ℓ*_T` has a clean semantic reading. This review answers both.

## 1. What M1 delivered

All of it is in `formal/UnaFormal/F1/`, about 2,300 lines of Lean with Batteries only.

| File | Content |
| --- | --- |
| `Labels.lean` | Labels as a level and categories. Labels with symbolic parts: `ℓ_T`, `ℓ*_T`, the static order of `spec/07 §7` rule 8, instantiation. |
| `Syntax.lean` | F1 as §3 of the plan defines it, in A-normal form over Core IR. It covers labeled types at every position, one level of generic functions, records with declared labels, the operation and the host operation, the prelude's `map`, `filter`, `first_where`, `fold` and `parse`, and declassification policies. |
| `Types.lean` | Erasure of labels, structural substitution, labeled instantiation (`lsubst`) and `ℓ*` (`deep`). |
| `Eval.lean` | The big-step semantics. It uses fuel, keeps a trace of events, and takes the host as a parameter (`World`), so two runs can differ in it. |
| `Typing.lean` | The structural typing and the typing of values. |
| `T1.lean`, `Lemmas.lean` | T1 for the structural typing. The lemmas include closure under substitution of type arguments (`ExprTy.subst`). |
| `LTyping.lean` | The labeled judgment `Γ; pc ⊢ e : τ ! ε` of `spec/10 §1` (§4 lists its readings). |
| `Erase.lean` | The labeled typing refines the structural one, so T1 holds for it too (`t1_labeled`). |
| `Spike.lean` | The semantic reading of the symbolic labels (§3). |
| `Examples.lean` | Shows the definitions are not vacuous: a generic function the labeled typing accepts, and a run of it. |

The stated theorems are listed in `formal/Theorems.lean`. The main ones:

- `t1`: a well-typed body never gets stuck, whatever the fuel and the host, and a value it ends with has its type.
- `t1_labeled`: the same for the labeled typing.
- `ExprTy.subst`: typing is closed under substitution of type arguments.
- `LExprTy.toStruct`: a labeled typing erases to a structural one.
- `LExpr.le_sound`: the static order is sound for the reading of §3.
- `Rel.eq_of_deep` and `derived_agrees`: why `ℓ*_T` is the label of a comparison.
- `top_label_not_enough`: why `ℓ_T` alone would leak.

## 2. Real effort against the estimate

| | Estimate (plan §7) | Actual |
| --- | --- | --- |
| M1 | 5 to 8 agent-days | About 30 minutes of one session, 2026-10-09 from 11:08 to 11:35 UTC. |

The 30 minutes break down as follows:

- **About 5 minutes:** T1 for F1a, the monomorphic and label-free core.
- **About 15 minutes:** extending the core to all of F1, then proving T1 again.
- **About 10 minutes:** the labeled rules, erasure, the spike lemmas and the examples.

There were about 20 compile iterations. Most errors were Lean API details, such as deprecated lemma names and `List.lookup`'s argument order. No error was a wrong statement.

Build times, the measurement the plan asked of the spike (§6):

- a cold `lake build` of `formal/`, with Batteries already built, takes 5.8 s;
- a warm build takes 0.2 s;
- a warm `scripts/formal.sh` takes 0.5 s.

**What the number does and does not predict.** The estimate's ranges were wide on purpose: proof engineering by agents had no track record here. M1 gives a first data point, but it is the easy end of the track for three reasons.

- **T1 is syntactic.** It needs no relation between two runs. Its proof is one induction on fuel, with case analysis that the type system makes routine.
- **The hard part of M3 and M4 is still ahead.** That part is a two-run relation for function values: closures, fuel-indexed. It also has to cover the program counter and failure as a channel. M1 states the relation only for first-order values (§3).
- **M1's labeled rules are written, not proven.** A rule that is wrong in a way T6 exposes costs a change in the model, possibly in `spec/07`, and a rerun of the proofs. That is the risk of §8 of the plan, and M1 cannot measure it.

**Recommendation.**

- Keep the table's ranges for M3 and M4 as ceilings, not as expected effort.
- Re-estimate after the generic-free half of M3.
- On this evidence, the cost of the formal track is more likely to come from finding where the rules are wrong than from proof volume.

## 3. Does `ℓ*_T` have a clean semantic reading?

**Yes.** The reading below is stated in Lean (`Spike.lean`) and is what `LTy.lsubst` implements.

- **Instantiation.** Each call of a generic function instantiates `T` by a labeled type `σ(T)` (`spec/10 §5`). Write `ℓ*(σ(T))` for the join of every label of `σ(T)`, the labels its records declare included.
- **`ℓ*_T`** reads as `ℓ*(σ(T))`.
- **`ℓ_T` at a position of type `T`** reads as `σ(T)`'s own labels, in place. Substitution puts them there, which is exactly what ADR 0067 says ("at a position that is a type parameter `T`, `ℓ_T` is the type argument's own labels, which substitution puts there").
- **`ℓ_T` anywhere else** reads as `ℓ*(σ(T))`. The checker's rules do not seem to put it anywhere else, and this conservative reading costs nothing if they do.

**The relation.** Fix an observer at label `L` and two runs. Two values are related at a closed labeled type in one of two cases:

- the type's top label is not visible to `L`, and then any two values are related;
- the values have the same shape and their parts are related at the parts' types.

Concretely:

- integers, Booleans and texts are related when they are equal;
- lists are related element by element;
- options, results and records are related by their contents.

A value of a generic body at a type with `T` is related at the type instantiated. So the relation is parametric: it reads `T` through `σ`.

**What is proven:**

1. **The checker's order is sound for the reading** (`LExpr.le_sound`). When rule 8's static order finds `a ⊑ b` between labels with symbolic parts, the reading of `a` is below the reading of `b` for every instantiation. This includes `ℓ_T ⊑ ℓ*_T` and "a symbolic label is below a declared one only if that one is `⊤` or holds it". A check on symbolic labels is therefore a check for all instances at once.
2. **`ℓ*_T` is the right label for anything computed from values of `T`.** Values related at `σ(T)` are equal when every label of `σ(T)` is visible (`Rel.eq_of_deep`). So a comparison, an equality, a field, or any other operation over `T` values gives the same result in both runs whenever `ℓ*(σ(T))` is visible (`derived_agrees`). That is the label rule 8 gives such a result.
3. **`ℓ_T` would not be enough** (`top_label_not_enough`). At `List[secret Int]` with a public top label, the top label is visible to a public observer, yet two related values differ. An equality of two such values labeled `ℓ_T`, read as the top label, would reveal the secret. This is the exact reason the checker has two symbolic labels (#354).
4. **T-App with a decision is sound in the reading.** This is argued here, and M3 proves it. Suppose an argument carries `ℓ*_T` that its parameter `T` does not accept, as in `k(if a == b then a else b)`. Two cases:
   - if `ℓ*(σ(T))` is visible, both runs choose the same value, and the call is related;
   - if not, the arguments may be unrelated, so the result and the effects of `k` may differ. They must then be invisible, which is what raising the call's result and program counter by the decision does.

**What is not yet proven, and is M3's:**

- the relation at function types, which needs a fuel-indexed definition for closures;
- records related field by field (M1 relates a record as a whole, by every label it declares);
- the lemma that instantiation commutes with the reading;
- T6 itself.

The relation's shape makes none of these look like a new risk. Function values are the standard hard case of a logical relation, and the plan's M3 and M4 ranges were set for them.

## 4. Readings of the specification in the model, for M3

The labeled rules follow `spec/07` and `spec/10`. In some places they choose a reading the text does not spell out. Each choice is marked in `LTyping.lean`. M3's proof either confirms it or shows the rule to change. None of them is a finding against the Rust checker yet; M2's differential run is where any disagreement would show.

1. **A failure that propagates through `let`.** The body after a right-hand side that may fail runs under the label of what decides the failure. A function's own failure must be decided at `⊥`. This is the model's reading of U4002 for `if error pass` (rule 4). A host failure decided by a labeled URL may therefore propagate only inside an `attempt` or a handler.
2. **T-App with a decision.** The model takes the decision `d`, a set of `ℓ*_T`, as part of the argument's top label. The parameter's type is raised by `d` for the flow, and the call's program counter and result are raised by `d`.
3. **The elements of `map`'s result are raised by the function's top label**, as T-App raises a call's result. ADR 0051 names only the top label of the result, so this reading may be stricter than the checker.
4. **`first_where`'s program counter** includes every label of the elements and the predicate's result label, as rule 9 says.
5. **The host operation's failure label** is the program counter joined with every label of the URL. The program counter decides whether the operation runs.
6. **Labels of literals and constructed values are `⊥`.** The enclosing `if` or `case` raises them by the condition, as T-If does.
7. **Not modeled in F1, as the plan's §3 says:**
   - edition 0.1's defense-in-depth rules;
   - `of` clauses (rule 9's callee half);
   - the syntactic placement of second-class lambdas (`spec/10 §9` rule 2); only type arguments are checked for being first-class;
   - generic records.

## 5. Go/no-go

The spike found no reason to stop.

- **Effort.** M1 cost a small fraction of its estimate. The evidence says little about T6's cost, but nothing against it.
- **The symbolic labels.** They have a clean reading, and the checker's order is proven sound for it.
- **Most likely revisions.** The open risk is model fidelity rather than effort: the readings of §4 against the Rust checker, which M2's differential run measures.

**Recommendation: go for M3.** Start with the generic-free half, as the plan orders. Re-estimate M3 and M4 at the end of that half, with the relation for function values in hand.
