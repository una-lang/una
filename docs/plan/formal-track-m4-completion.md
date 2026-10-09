# Formal track: M4 completion, the re-check's verdicts as theorems

- Issue: #46
- Plan: `docs/plan/formal-track.md` §7; previous steps `formal-track-m2.md` (the re-check) and `formal-track-m4.md` (T6 with generics, option B1)
- Date: 2026-10-09
- Status: part A1 done (this document's first pull request); A2 and A3 to come, each its own pull request

## 1. Why

M2's re-check (`formal/Recheck.lean`) reports each declaration of a `una.core/1` document as accepted, rejected or not re-checked. ADR 0008's rule is **no accept without a theorem**. M2 and M4 proved T6 of the labeled typing (`LExprTy`, `Program.LWellFormed`), but the re-check runs an algorithm, `Check.lean`, written to follow that typing rule by rule. Until the algorithm is proven to imply the typing, an "accepted" rests on the claim that the two agree.

M4 completion closes that gap in three parts:

| Part | Claim | Status |
| --- | --- | --- |
| A1 | The checker is sound: what `checkRhs`, `checkExpr`, `checkFun` and `checkB1` accept, the labeled typing accepts. | Done |
| A2 | Every declaration of a document accepted ⇒ the program read from it passes `Program.LWellFormed` and `Program.B1WellFormed`. This proves the reachability rule (§3) and the parts of `LWellFormed` the checker tests outside bodies (`checkDecls`, `checkSignature`, categories). | Next |
| A3 | Instances: a generic function checked at each ground instance the program reaches runs as its instances do, since evaluation reads no annotation (eval-erasure; `docs/reviews/formal-m2-replay/README.md`, "Why it is sound"). | After A2 |

With A3, an "accepted" from the re-check is a corollary of `t6_release`, with the reading of `una.core/1` (`Import.lean`) as the remaining trusted base.

## 2. A1: the checker is sound

All of it is in `formal/UnaFormal/F1/`. `scripts/formal.sh` passes with 78 stated theorems, none depending on `sorry`, with only the axioms `propext`, `Classical.choice` and `Quot.sound`.

| File | Result |
| --- | --- |
| `CheckSound.lean` | The checker's Boolean tests imply the typing's propositions: label order (`leB_sound`), flows (`flowB_sound`, `flowsB_sound`), `ℓ_T` in place, ground and scoped types, the effect bounds of T-App and of releases (`boundB_sound`, `relBoundB_sound`), the types of atoms and primitives, joins, and the lemmas that read a `do` block of `Except` that returned `.ok`. |
| `CheckSoundCore.lean` | **`checkRhs_sound` and `checkExpr_sound`**, one mutual induction over every form of F1: what the checker accepts against a type `ann` has a labeled typing whose type flows into `ann`, with the effects the checker returns. **`checkFun_sound`**: a function the checker accepts passes T-Fun (`Program.FunOk`). **`exprPure_sound`**, **`b1Ok_sound`** and **`checkB1_sound`**: a program whose policies the checker accepts is within B1's restrictions (`Program.B1WellFormed`). |
| `CheckSoundExample.lean` | Non-vacuity. The checker accepts `rev`, which declassifies through `reveal`, and `rev_ok` gives its T-Fun from the checker's run alone. `prog_b1_checked` gives `ReleaseExample.prog`'s B1 well-formedness the same way. A `#guard` checks that the checker rejects `underSecret`, which `release_under_secret_untyped` proves has no typing. |

**What A1 does not claim.** It says nothing about the reading of `una.core/1` into F1, which stays trusted, nor about the steps of `checkDocumentWith` that combine per-declaration verdicts: the reachability fixpoint and the instance search. A2 and A3 prove those.

**No difference found between the algorithm and the typing.** Every case of the mutual induction closed against the rule it follows. Had one not, the rule of the track is to report it to the coordinator before changing either side.

## 3. What a declaration reaches

A declaration is checked against its callees' signatures, and T6 needs every function of the program to pass T-Fun. So a declaration is accepted only when everything it reaches, transitively, is accepted too (#452, `formal-track-m4.md` §5). A1 adds two precisions to that rule, from Una-1's review.

**Functions outside the document.** A callee that is not a declaration of the document, a library function or a prelude function, has no verdict. Until now, the fixpoint read a missing verdict as no objection. Now:

- a prelude function F1 gives a semantics to, `map`, `filter`, `first_where`, `fold`, `parse_int` or `sum` (`Import.preludeF1`), is exempt;
- any other one makes the declaration not re-checked: "reaches `<f>`, outside the document".

`rawCallees` keeps prelude calls, so the exemption is decided in one place. `ImportTests.lean` holds three guards: a function value of `lib.helper` and one of `prelude.append` make `user` not re-checked, and the same declaration with `prelude.map` is accepted, the rule's non-vacuity. Before this change, the first two were accepted.

On the corpus (the export of `main` with #443's keys, 1,327 documents) and on the replay of S-01 to S-12, the new reason fires nowhere: in today's `una.core/1`, a lambda's body calls the function it wraps, and the reading already reports a call outside the document as outside F1. The rule closes the case where the two would disagree. The corpus totals are unchanged: 641 accepted, 0 rejected, 1,122 not re-checked. The replay is unchanged: 16 rejection lines, and both mutants are caught.

**Handler bodies.** A declaration reaches everything it calls or uses as a value, **except handler bodies**. F1's operation is an event of the trace; what a capability's handler does with it is not modeled, so a handler's body is not in the reach set. This is M2's assumption, not a theorem. It is stated in `Import.lean`'s header and in `rawCallees`, in `formal-track-m2.md` §1, and in `docs/reviews/formal-m2-replay/README.md`. A handler declaration itself is reported as not re-checked ("a `handler` declaration"). T4 and F2 (M6) model handlers.

## 4. A2 and A3

**A2.** A theorem of the form: `checkDocumentWith` reports every declaration of a document accepted ⇒ the program it read passes `LWellFormed` and `B1WellFormed`. Its parts:

- the reachability fixpoint gives T-Fun for every function the accepted ones reach, from A1's `checkFun_sound`;
- `checkDecls` and `checkSignature` give the record, policy, scoping and in-place fields of `LWellFormed`;
- a `via` function's structural typing, and the categories (`CatsDeclared`).

A2 states it over the program F1 reads. The reading itself stays trusted.

**A3.** For a function checked at its instances: an instance's run equals its generic function's run up to the annotations inside closures, so a program whose reachable instances all pass is the generic-free program of its instances, which `t6_ground` covers.

## 5. Later: a stateful host

The coordinator proposed a stateful host for `Files` and `Http` as the next milestone (B), pending Leonardo. Nothing in this plan depends on it, and nothing is renamed until he answers. #453 holds the widening of `checked` (backlog).
