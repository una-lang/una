# Formal track: M4 completion, the re-check's verdicts as theorems

- Issue: #46
- Plan: `docs/plan/formal-track.md` §7; previous steps `formal-track-m2.md` (the re-check) and `formal-track-m4.md` (T6 with generics, option B1)
- Date: 2026-10-09
- Status: parts A1 and A2 done, each its own pull request; A3 to come

## 1. Why

M2's re-check (`formal/Recheck.lean`) reports each declaration of a `una.core/1` document as accepted, rejected or not re-checked. ADR 0008's rule is **no accept without a theorem**. M2 and M4 proved T6 of the labeled typing (`LExprTy`, `Program.LWellFormed`), but the re-check runs an algorithm, `Check.lean`, written to follow that typing rule by rule. Until the algorithm is proven to imply the typing, an "accepted" rests on the claim that the two agree.

M4 completion closes that gap in three parts:

| Part | Claim | Status |
| --- | --- | --- |
| A1 | The checker is sound: what `checkRhs`, `checkExpr`, `checkFun` and `checkB1` accept, the labeled typing accepts. | Done |
| A2 | The re-check certifies one whole program per document, and `progOk_sound` proves that it passes `Program.LWellFormed` and `Program.B1WellFormed`, with `t6`'s `hget` discharged. `reach_closed` is the reachability rule's formal side. | Done (§4) |
| A3 | Instances: a generic function checked at each ground instance the program reaches runs as its instances do, since evaluation reads no annotation (eval-erasure; `docs/reviews/formal-m2-replay/README.md`, "Why it is sound"). It discharges A2's one hypothesis, `hA3`. | Next |

With A3, an "accepted" from the re-check is a corollary of `t6_release`, with the reading of `una.core/1` (`Import.lean`) as the remaining trusted base.

## 2. A1: the checker is sound

All of it is in `formal/UnaFormal/F1/`. `scripts/formal.sh` passes with 78 stated theorems, none depending on `sorry`, with only the axioms `propext`, `Classical.choice` and `Quot.sound`.

| File | Result |
| --- | --- |
| `CheckSound.lean` | The checker's Boolean tests imply the typing's propositions: label order (`leB_sound`), flows (`flowB_sound`, `flowsB_sound`), `ℓ_T` in place, ground and scoped types, the effect bounds of T-App and of releases (`boundB_sound`, `relBoundB_sound`), the types of atoms and primitives, joins, and the lemmas that read a `do` block of `Except` that returned `.ok`. |
| `CheckSoundCore.lean` | **`checkRhs_sound` and `checkExpr_sound`**, one mutual induction over every form of F1: what the checker accepts against a type `ann` has a labeled typing whose type flows into `ann`, with the effects the checker returns. **`checkFun_sound`**: a function the checker accepts passes T-Fun (`Program.FunOk`). **`exprPure_sound`**, **`b1Ok_sound`** and **`checkB1_sound`**: a program whose policies the checker accepts is within B1's restrictions (`Program.B1WellFormed`). |
| `CheckSoundExample.lean` | Non-vacuity. The checker accepts `rev`, which declassifies through `reveal`, and `rev_ok` gives its T-Fun from the checker's run alone. `prog_b1_checked` gives `ReleaseExample.prog`'s B1 well-formedness the same way. A `#guard` checks that the checker rejects `underSecret`, which `release_under_secret_untyped` proves has no typing. |

**What A1 does not claim.** It says nothing about the reading of `una.core/1` into F1, which stays trusted, nor about the steps of `checkDocumentWith` that combine per-declaration verdicts: the reachability fixpoint and the instance search. A2 (§4) and A3 (§5) cover those.

**No difference found between the algorithm and the typing.** Every case of the mutual induction closed against the rule it follows. Had one not, the rule of the track is to report it to the coordinator before changing either side.

## 3. What a declaration reaches

A declaration is checked against its callees' signatures, and T6 needs every function of the program to pass T-Fun. So a declaration is accepted only when everything it reaches, transitively, is accepted too (#452, `formal-track-m4.md` §5). A1 adds two precisions to that rule, from Una-1's review.

**Functions outside the document.** A callee that is not a declaration of the document, a library function or a prelude function, has no verdict. Until now, the fixpoint read a missing verdict as no objection. Now:

- a prelude function F1 gives a semantics to, `map`, `filter`, `first_where`, `fold`, `parse_int` or `sum` (`Import.preludeF1`), is exempt;
- any other one makes the declaration not re-checked: "reaches `<f>`, outside the document".

`rawCallees` keeps prelude calls, so the exemption is decided in one place. `ImportTests.lean` holds three guards: a function value of `lib.helper` and one of `prelude.append` make `user` not re-checked, and the same declaration with `prelude.map` is accepted, the rule's non-vacuity. Before this change, the first two were accepted.

On the corpus (the export of `main` with #443's keys, 1,327 documents) and on the replay of S-01 to S-12, the new reason fires nowhere: in today's `una.core/1`, a lambda's body calls the function it wraps, and the reading already reports a call outside the document as outside F1. The rule closes the case where the two would disagree. The corpus totals are unchanged: 641 accepted, 0 rejected, 1,122 not re-checked. The replay is unchanged: 16 rejection lines, and both mutants are caught.

**Handler bodies.** A declaration reaches everything it calls or uses as a value, **except handler bodies**. F1's operation is an event of the trace; what a capability's handler does with it is not modeled, so a handler's body is not in the reach set. This is M2's assumption, not a theorem. It is stated in `Import.lean`'s header and in `rawCallees`, in `formal-track-m2.md` §1, and in `docs/reviews/formal-m2-replay/README.md`. A handler declaration itself is reported as not re-checked ("a `handler` declaration"). T4 and F2 (M6) model handlers.

## 4. A2: the whole program

**Why.** `Import.checkWith` checks each declaration in its own program: the function, its callees as signature stubs (`.atom .unit` bodies), the document's policies, and the one operation it mentions. `t6_release` speaks of one program, which passes `LWellFormed` as a whole. Before A2, an "accepted" rested on the per-declaration programs, and on three facts the re-check never tested (G1 to G3 below), which the Rust checker was trusted for.

**The certificate.** After the reachability fixpoint, `checkDocumentWith` reads the document into one program `P`:

- the accepted declarations, with their real bodies;
- every `via` function, body read;
- all of the document's policies, records and categories;
- its one operation (`acceptsOp`);
- the host's labels: `Http.get`'s `accepts` and the label of its result, or `⊥` when the host has none. Never `⊥` in place of real ones: a document whose labels F1 cannot read is not re-checked.

`Check.progOk` (`CheckProgram.lean`) then tests every field of `LWellFormed` and `B1WellFormed` on `P` itself, as Booleans:

- T-Fun of each function, by A1's `checkFun`, in `P`, not against stubs;
- each `via` function's structural typing, by `checkExpr` against its result raised to `⊤`, then `LExprTy.toStruct`;
- the host's labels ordered, `acceptsGet ⊑ getResult` (`t6`'s `hget`);
- records ground, closed, with `ℓ_T` in place, below their deep labels;
- policies closed, with `ℓ_T` in place, and a `via` function whose structural signature is the policy's (G1);
- every declared label's categories declared (`CatsDeclared`, G2);
- signatures scoped, `Eq` constraints on the function's own type parameters, and B1's restrictions.

**The theorems** (`CheckProgramSound.lean`):

| Theorem | Statement |
| --- | --- |
| `Check.progOk_sound` | `progOk cond P = true` and `hA3 : ∀ fn ∈ P.funs, fn.name ∈ cond → ¬ P.IsVia fn.name → P.FunOk fn` ⇒ `P.LWellFormed ∧ P.B1WellFormed ∧ P.le acceptsGet getResult`. |
| `Check.progOk_sound_nil` | The same with `cond = []`: no hypothesis left. |
| `reach_closed` | In a program the labeled typing accepts, every function a function's body calls or takes as a value, lambdas included, is a function of the program that passes T-Fun. This is "no accept without a theorem" stated formally: the reach set the re-check computes is closed in the program it certifies. |

Non-vacuity is in `CheckProgramExample.lean`. A program of three functions, the `via` function `check`, `rev` and `caller`, passes `progOk` by evaluation (`prog3_certified`), and `caller_reach` follows. The same program with a `via` function that disagrees with its policy fails `progOk`. `ImportTests.lean` holds a guard for each demotion, G1 (`g1ViaResult`) and G3 (`g3TwoOps`), next to a control that stays accepted (`b1Public`, `g3OneOp`).

**What "accepted" means now.** A declaration is accepted when it is a function of the document's whole program `P_U`, made of the accepted declarations that reach no function checked at its instances, and `progOk [] P_U` passed. `progOk_sound_nil` then gives `P_U.LWellFormed ∧ P_U.B1WellFormed` and `hget`, which `t6_release` needs, and `reach_closed` covers everything the declaration reaches. Handler bodies are excepted, as stated in §3. The reading of `una.core/1` stays trusted.

**Accepted, conditional on A3.** A function checked at its instances, or a declaration that reaches one, is reported in a bucket of its own. It is a function of the whole program `P`, which passed `progOk cond P` with `cond` the instance-checked functions. Its theorem is `progOk_sound` with the one hypothesis `hA3`, their T-Fun, which part A3 discharges. These declarations are not counted as theorem-backed until A3 lands. Then they move to accepted with no other change.

**Demotions.** A function that fails in `P` is demoted with its reason, "whole program: …", with everything that reaches it, and `P` is checked again. The rounds are bounded by the number of declarations; if they run out, every declaration left is not re-checked. Whatever happens, the verdict rests on the last check of `P` itself.

- **G3, one operation.** F1 has one operation. When accepted declarations mention different operations, those that mention one are not re-checked: "the document's accepted declarations use several operations; F1 has one". This is conservative; it never rejects.
- **G1, G2, hget and a `via` function's structural typing.** A failure demotes every declaration of the document, since each of them is checked against all of its policies.

**The corpus** (`main`'s export with #443's keys, 1,327 documents), `main` → A2:

| Outcome | `main` | A2 |
| --- | --- | --- |
| Accepted | 641 | 556 |
| Accepted, conditional on A3 | | 82, in 40 documents: 43 checked at their instances, 39 that reach one |
| Rejected | 0 | 0 |
| Not re-checked | 1,122 | 1,125 |

The 3 moves to not re-checked are all G3, in `types/alias_effectful_fn`: `run` and `shout` use `Console.print`, and `record` uses `Log.write`. G1, G2, `hget` and the `via` functions' structural typing demote nothing, and no declaration accepted in its own program fails in the whole program. The replay of S-01 to S-12 is unchanged: 16 rejection lines, and both mutants are caught.

## 5. A3

For a function checked at its instances: an instance's run equals its generic function's run up to the annotations inside closures, so a program whose reachable instances all pass is the generic-free program of its instances, which `t6_ground` covers. A3 discharges `hA3`, and the 82 declarations conditional on it move to accepted.

## 6. Later: a stateful host

The coordinator proposed a stateful host for `Files` and `Http` as the next milestone (B), pending Leonardo. Nothing in this plan depends on it, and nothing is renamed until he answers. #453 holds the widening of `checked` (backlog).
