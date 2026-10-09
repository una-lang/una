# Formal track M2: the replay of S-01 to S-12

- Issue: #46
- Plan: `docs/plan/formal-track.md` §4 ("Re-check" column) and §7 (M2)
- Date: 2026-10-09

## Method

The question of the plan's §4: would the Lean checker, run on a leaking program's Core IR, have rejected it?

The Core of a leaking program as the shipped checker saw it exists only in the release that accepted the program. `una check` of today rejects the repro, and `una core` exports only declarations that check clean. So each case runs as follows.

1. Take the repro of the finding, a conformance case of `tests/conformance/security/`.
2. Take the last release before the finding's fix, from `coordination/releases/`. It accepts the repro: `una check` exits 0.
3. Print that release's Core of the repro with `una check --dump-core <module>`. The output is in `dumps/`, named `<finding>.v<release>.txt`. Its labels are the ones that checker inferred, the bug included.
4. Read the dump into `una.core/1` with `dumpcore.py`, and run `una-core-check` on it.

Nothing is edited by hand. The label that each rejection names is the pre-fix checker's own. `run.sh` repeats steps 4 and 5 from the dumps, runs the tests of the instance checking, and checks that both mutants are caught.

## Results

| Finding | Repro | Release | Fix | Lean checker |
| --- | --- | --- | --- | --- |
| S-01 | `decided_argument_to_function_0_1` | v0.1.0 | #253, backported in #343 (v0.1.4) | **Rejected** `apply`: its `==` on `T` values is typed `public Bool`, and rule 8 gives it `ℓ*_T` |
| S-06 | `decided_argument_to_prelude_callback_0_1` | v0.1.0 | 978852e in #343 (v0.1.4) | **Rejected** `mapped`, `kept`, `folded` and `counted`, at the same `==`. v0.1.0 labels no decision anywhere. `passed`, which compares nothing, is accepted |
| S-07 | `decided_argument_to_prelude_callback_0_1` | v0.3.0 | #346 (v0.3.1) | **Rejected** `mapped`, `kept` and `folded`, at the call of `map`, `filter` and `fold`: elements labeled `ℓ*_T` reach a callback whose parameter is `T`. `counted` and `passed` are accepted, as `una check` of today accepts them |
| S-08 | `hof_run_effect_user_hof` | v0.3.0 | #349 (v0.3.1) | **Rejected** `g` at its instance `g[public List[internal Int]]`, instantiated in `run`: the callback `k` is given to `apply_eff` with the decided value, which does not flow into `k`'s parameter |
| S-09 | `hof_run_prelude_annotated_lambda` | v0.3.0 | #349 (v0.3.1) | **Rejected** `g`, at the lambda annotated `x: T` bound to a function type whose parameter carries `ℓ*_T` |
| S-10 | `hof_annotated_nested__effect__parens` | v0.3.0 | #349 (v0.3.1) | **Rejected** `g` at its instance `g[public List[secret Int]]`, instantiated in `leak`, at the lambda annotated `x: T`. Core has no wrappers, so S-10 is S-09's form behind a user generic callee |
| S-11 | `hof_run_mixed_params_lambda` | v0.3.0 | #349 (v0.3.1) | **Rejected** `g` at its instance `g[public List[internal Int]]`, instantiated in `run`, at the lambda whose annotated parameter drops the decision |
| S-12 | `hof_run_let_annotation_direct` | v0.3.1 | #354 (v0.3.2) | **Rejected** `g`, at `let y: T = if a == b …`: the value raised by the decision does not flow into `T` |

**Not replayed, out of fragment:**

- **S-02** (`parse_json`): F1 has no `Json`.
- **S-03** (Http failures): the importer does not read `Http`'s operations, which take several arguments and fail.
- **S-04** (`let edit`): the memory model, F1+ (M5).
- **S-05** (`Files.write`): an operation with two arguments that fails.

**Summary.** The checker rejects all 8 findings that F1's forms can write.
- **S-01, S-06, S-07, S-09 and S-12** are rejected at the label the pre-fix checker got wrong, at the binder or call where it went wrong.
- **S-08, S-10 and S-11** are rejected at the instance where the decided value reaches the callback. Their leaking bodies call a generic function at their own type parameter, which the checker checks at the function's instances (below).

## Instance checking

**What it is.** A function that calls a generic function at its own type parameters is checked at every ground instance the program reaches. F1's typing has one level of generics, so the checker does not check such a function generically. Each instance is the function with its type parameters instantiated by the model's labeled instantiation, `LTy.lsubst`, in its signature and in every annotation of its body (`Check.instantiate`).

**What it claims.** It re-checks programs at their instances. It is not a modular check of generic declarations.
- For the program given, it covers every run, since a run executes only the instances the program reaches.
- It says nothing about the shipped checker's judgment of such a generic function for code it has never seen.
- When a mislabeled generic function is rejected only at an instance, the rejection names the declaration, the instance and the declaration that instantiates it.

**Where instances come from.** Every call with type arguments in every declaration's Core, searched in full:
- function values, which Core writes as a lambda around the call;
- handlers;
- `main`;
- the instances themselves, through a worklist.

A function whose instances cannot all be known is reported as not re-checked, never as accepted. That happens when it is:
- instantiated from code outside F1;
- at types outside F1;
- never instantiated at a ground type;
- in polymorphic recursion past a depth of 16;
- in a document of more than 256 instances.

**Tests**, in `tests/`, each derived from S-08's pre-fix Core and stating its derivation:
- **`fnvalue.S-08`** hides the leak behind function values. `run` reaches `g`, and `g` reaches `apply_eff`, only through lambdas around the calls. The instance of `g` is found inside the value and rejected.
- **`mutants.S-08`** moves the decision into `g`'s signature, so that only instantiation sees the leak. `lsubst` rejects `g`. Two mutants of it accept `g`, and `run.sh` checks that both are caught:
  - one that does not raise a type argument by the labels of the position it fills;
  - one that reads `ℓ*_T` as the type argument's top label.

  `formal/UnaFormal/F1/CheckTests.lean` checks `lsubst` itself at build time. It covers `ℓ_T`, `ℓ*_T`, and a decision off a position of type `T`, alone and through two levels: `h[List[internal T]]` at `T := Int`.

**Why it is sound**, for the proof of option B, which is later:
- An instance runs as its generic function does, because evaluation reads no annotation. `eval` ignores the type of every `let` and the type arguments of every call. A closure keeps its parameters' types, but a call binds the parameters by name only. So the lemma is an equivalence of runs up to the annotations inside closures, not an equality.
- A program whose reachable instances all pass is therefore the generic-free program of its instances, which `t6_ground` covers.

Two behaviors of the real runtime read types. Neither reads the labels of a type argument:
- **(i) Type descriptors.** The native backend passes a type descriptor for each constrained type parameter (`spec/20 §7`) and dispatches `==` and ordering on it. A descriptor depends only on the structure of the type argument, which instantiation keeps (`LTy.erase_lsubst`).
- **(ii) Trace rendering.** The interpreter and the native backend redact a record's fields in a trace by their declared deep labels, and hide a field of a type parameter's type. They read declared kinds, never type arguments. F1's trace has no rendering, so this is an assumption about the real runtime, stated here.

## What a declaration reaches

A declaration is accepted only when everything it reaches is accepted too, except handler bodies. An operation is an event of F1's trace, so a capability handler's body is not in the reach set: this is M2's assumption (`docs/plan/formal-track-m2.md` §1). A function outside the document, from a library or a prelude function F1 does not model, makes what reaches it not re-checked.

## What changed in the model for the replay

- The operation writes any scalar, `Int`, `Bool` or `Text`, since every repro writes an `Int`.
- The prelude's `sum` is a primitive, a fold of `+` with every label of its list.

T1, the unary half and T6 carry both changes, and `scripts/formal.sh` passes with 59 theorems. For edition 0.1 documents, the importer constrains by `Eq` every type parameter of a function that applies `==`, since edition 0.1 has no `Eq` constraint. This only adds the check, at each call, that the type argument has equality.
