# Formal track: M2, the re-check of Core

- Issue: #46
- Plan: `docs/plan/formal-track.md` §5 (option A as the first deliverable of option B) and §7 (M2)
- Date: 2026-10-09
- Status: done; the export gaps it found are #443

## 1. What M2 delivers

**The checker.** `lake exe una-core-check <doc.json>...` reads `una.core/1` documents (ADR 0083) and prints an outcome for each declaration with code: `accepted`, `rejected` with the rule that failed, or `not re-checked` with the form outside F1.

| File | Content |
| --- | --- |
| `formal/UnaFormal/F1/Check.lean` | The labeled typing of `LTyping.lean` as total functions, rule by rule. It checks each expression against the type it must flow into, the type Core gives every binder, so it never computes a join of types. |
| `formal/UnaFormal/F1/Import.lean` | The reading of `una.core/1` into F1, one function at a time. Each mapping is stated in the file's header. A form F1 lacks is reported as not re-checked, never as accepted. |
| `formal/Recheck.lean` | The executable. |
| `docs/reviews/formal-m2-replay/` | The replay of S-01 to S-12 (§3). |

**A program is re-checked** when every declaration of its document with code is accepted. A declaration is checked against the signatures of the functions it calls, so an accepted caller of an unaccepted callee says nothing on its own.

**Changes to the model**, with every proof carried (`scripts/formal.sh`: 59 theorems, no `sorry`):

- the operation writes any scalar, `Int`, `Bool` or `Text`, since the corpus and every repro write integers;
- the prelude's `sum` is a primitive, with every label of its list.

## 2. The corpus

The corpus is v0.3.4's export of every conformance case and example: 1,276 documents, 366 of them complete with code.

| Outcome | Declarations |
| --- | --- |
| Accepted | 601 |
| Not re-checked | 1,090, mostly types outside F1 (tuples, fixed-width integers, `Map`, kinds with variants), operations that fail or take several arguments, primitives F1 lacks, and generic calls inside generic bodies |
| Rejected | 7 |

**The 7 rejections are not leaks.** Each one needs a fact the checker has and the export lacks, so #443 adds them to `una.core/1`:

- result summaries (ADR 0067), in 5 cases;
- the effects of a lambda's type and the `of` clauses, in 2 cases.

Each gap makes the re-check stricter, never laxer, at the declaration that needs the fact. There is one exception at call sites. A caller reads a callee's declared result without its summary, so it can accept what depends on the summary. Such a callee is itself rejected, so no program is re-checked through it. #443 closes this.

**Non-vacuity, by mutation of the accepted declarations:**

- **Parameters.** Raising a public scalar parameter to `secret` gave 162 mutants and 110 rejections. Every one of the 52 survivors raises a parameter the body never reads.
- **Labels.** Lowering a non-public `let` or result label to `public` gave 23 mutants and 16 rejections. Of the 7 survivors:
  - 3 lower an annotation that is above its value;
  - 1 removes an in-place `ℓ_T`, which is the same type;
  - 3 depend on the call-site exception above.

**Two readings of the export, stated in `Import.lean`:**

- **`ℓ_T` in place.** A value of `T` carries `ℓ_T` in place, and a signature writes `T` alone. Instantiation gives both the same type, so the reading drops `ℓ_T` there.
- **Edition 0.1's `==`.** In edition 0.1, `==` applies to values of any type parameter. The reading constrains by `Eq` the type parameters of a function that compares, which adds the check at each call that the type argument has equality.

**Found on the way:** v0.3.4's export labels `to_text` of a value of `T` with a plain `ℓ_T`, off place. #440 fixes it.

## 3. The replay of S-01 to S-12

The Core that each leaking program had is the one the release before its fix printed. That release accepted the program, and `--dump-core` printed its inferred labels. `docs/reviews/formal-m2-replay/` holds those dumps, the reader `dumpcore.py` and the results, with no label edited by hand.

| | Findings |
| --- | --- |
| Rejected at the label the pre-fix checker got wrong | S-01, S-06, S-07, S-09, S-12 |
| Rejected at the instance where the decided value reaches the callback | S-08, S-10, S-11 |
| Out of fragment: other forms | S-02 (`Json`), S-03 (`Http`), S-04 (`edit`, F1+), S-05 (`Files.write`) |

For S-07, the checker rejects the three leaking functions and accepts the two that today's `una check` accepts.

## 4. Instance checking

A function that calls a generic function at its own type parameters, two levels of generics, is checked at every ground instance the program reaches. The model's labeled instantiation gives each instance (`docs/reviews/formal-m2-replay/README.md`, "Instance checking").

**Claim scope.** This re-checks programs at their instances. It is not a modular check of generic declarations.
- It validates every run of the program given.
- It does not validate the shipped checker's judgment of such a generic function for code it has never seen.

**Outcomes.** A function whose instances cannot all be known, or are never reached, is not re-checked, with its reason. The causes are a source outside F1, no ground instance, polymorphic recursion past a depth of 16, and more than 256 instances. None of these is a pass.

**On the corpus.** It moves the 62 declarations with generic calls inside generic bodies:
- 46 accepted at their instances;
- 5 never instantiated;
- 1 with a source outside F1;
- 10 blocked by other forms.

Totals: 647 accepted, the same 7 rejections, 1,044 not re-checked. No polymorphic recursion or cap is met.

**Soundness, for option B.** Evaluation reads no annotation, so an instance runs as its function does, up to the annotations inside closures. The generic-free program of the instances is covered by `t6_ground`. Two behaviors of the real runtime read types, and neither reads a type argument's labels:
- the type descriptors of constrained type parameters depend only on structure, which instantiation keeps;
- trace rendering redacts by declared kinds, which is outside F1's trace and stated as an assumption.

## 5. What comes next

- Declassification, option B1 (`docs/plan/formal-track-m4.md` §3).
- #443, the export's missing facts.
- Option B of the plan: a proof that an accepted function passes T-Fun, which makes the re-check a proven one.
- The `formal` gate runs the re-check on the corpus.
