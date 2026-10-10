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
- **S-03** (Http failures): not replayable from any release. `Http` first ships in v0.3.0, after the fix (#317, merge `51a7a2fc`): v0.2.0 has no `std.host` (`una check` of the repro is U1001), and the v0.2.0 alphas predate `Http` too. The coordinator decided not to build the unreleased commit before the fix, since a binary never released adds little evidence (#498).
- **S-04** (`let edit`): the memory model, M6. Its shapes are now dumped and read (below, "S-04"), and none is re-checked yet.
- **S-05** (`Files.write`): now replayed from v0.1.3 (below, "S-05"), with one input taken from a later release.

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

**Why it is sound**, proven in M4 completion part A3 (`formal/UnaFormal/F1/Erasure.lean`, `eval_erasure`, and `Monomorph.lean`, `t6_release_erasure`):
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

## S-04 (M6, #478)

**Provenance.** S-04 leaked in v0.2.0, which was never published. The release gates built it from commit `5361a0e364a4116cafb65a7cebe92c3733525eea` (`coordination/releases/v0.2.0/record.md`). The archive `una-0.2.0-aarch64-apple-darwin.tar.gz` has the sha256 the record states, `ffb299c770a9d6239ce0fc79620dceb35839277337f6603cb4715a2943895e7c`, and its `una` is the binary these dumps come from. Commit `2a69456`, named in N-45, only adds the validation record on top of `5361a0e3`.

**The shapes.** These are the 16 conformance cases of `tests/conformance/security/` that this v0.2.0 binary accepts (`una check` exits 0) and that v0.3.0, the first release with the fix (ADR 0075), rejects (exit 1):
- `edit_order_loop_` `list`, `option`, `result_ok`, `result_err`, `closure`, `edit_param`, `fold`, `generic_init`, `nested_list`, `nested_list_of_options`, `nested_option`, `nested_result` and `together`;
- `edit_order_` `list_of_lists_leak`, `list_of_options_leak` and `option_of_list_leak`.

`edit_order_if_no_loop` is also accepted by v0.2.0 and rejected by v0.3.0. It is ADR 0075's intentional tightening, a read printed before the only `internal` assignment, which leaks nothing, so it is not replayed. The `Map` shapes do not check on v0.2.0.

`dumps-0.2/` holds each shape's `una check --dump-core main` output from that binary, edited by no one. `run.sh` reads them in edition 0.2 after S-01 to S-12.

**Result with M6b (#485): 6 of 8 rejected at the leak.** M6b models `to_text`, `concat` and `Env.get`.
- **6 rejected**, each at the interpolation: "the primitive's result, `internal Text`, does not flow into `public Text`". These are `edit_order_loop_list`, `option`, `closure`, `edit_param`, `fold` and `generic_init`.
- **2 not re-checked**, at "a value of `prelude.Result`": `result_ok` and `result_err` build a `Result` with `Ok` and `Err`, which F1 lacks.
- **The other 8** are not re-checked, at "a call of `prelude.or_else`" (7) or "the form `together_for`" (1). `at`, `or_else` and `first` come in a later item.

The helpers `edit_param`'s `push` and `generic_init`'s `same` are accepted, and neither prints.

**The controls** (the coordinator's answer: existing `*_accepted` programs only). `edit_order_list_of_lists_accepted` and `edit_order_option_of_list_accepted` are dumped by v0.3.0, which accepts them. The re-check does not reject either: both are not re-checked, at `or_else`, as are their leaking counterparts. The 8 loop shapes have no `*_accepted` counterpart, and are unpaired. The `Map` controls pair with no replayed shape, since the `Map` leaks do not check on v0.2.0.

**The reader.** `dumpcore.py` reads M6's syntax:
- tuple types `(T, ...)`;
- tuple values `(a, ...)`;
- `let (x: T, ...) = a in e`;
- `loop x: T in s with (b: T = a, ...) do e`, over a list or a range.

It reports a body with `together` or `together_for` as that form, outside F1. Two changes came with M6b, each checked against a real `una core --json` export:
- **The `host` key.** It lists `std.host`'s `Console` and `Env` when a declaration uses them, as the export writes them: the same `print(line: Text) -> Unit` and `get(name: Text) -> Option[internal Text]`, both accepting `public`. Before, `host` was empty, which S-01 to S-12 never needed, since their repros use their packages' own capabilities.
- **`Ok(a)` and `Err(a)`** are read as constructs of `prelude.Result`, as the export writes them, not as calls of a variable.

## S-05 (#498)

**Provenance.** S-05 leaked in edition 0.1 from v0.1.0 to v0.1.3, and v0.1.4 fixed it (#343). The dumps come from v0.1.3, the last release before the fix, downloaded from its GitHub release: `una-0.1.3-aarch64-apple-darwin.tar.gz`, whose sha256 matches the release's `.sha256` file. Its `una` has the sha256 `2ea67e80bd6c8d464fd7433d9c25e4362b1e6fcc275b714340e84d861e87b52b`.

**The shapes.** The leaking `n39_write_path_` cases of `tests/conformance/security/`: `attempt_branch`, `error_payload`, `fail`, `pass` and `use_into_public`, each in edition 0.1. v0.1.0 and v0.1.3 accept all five (`una check` exits 0); today they are U4001, U4002 or U4003. The control is `n39_ok_public_path_content_internal_0_1` (`expect: ok`), which writes `internal` content to a `public` path and branches on the result. `dumps-s05/` holds each case's `una check --dump-core main` output from v0.1.3, edited by no one.

**The one input not taken from the release: the file-state label.** The re-check reads `Files` only with the package's file-state label (ADR 0085), which v0.1.3 does not have. For each dump, `run.sh` takes the label that v0.3.6's `una core --json` infers for the same repro, the least label that covers every write. `dumps-s05/file_state.tsv` records it per dump, with the command and the binary: v0.3.6's `una` from `coordination/releases/v0.3.6/una-0.3.6-aarch64-apple-darwin.tar.gz`, sha256 `83fc3781e69f086aa8a8f0fbeb277cd5fa45c7056c16e0b62634bd13ead9815f`. It is `internal` for the five leaks and `public` for the control.

**Result: 4 of 5 rejected, the control accepted.**
- `fail` and `pass` are rejected at the failure (U4002), where `una check` of today reports them.
- `attempt_branch` and `use_into_public` are rejected at the `attempt`: "the result of `attempt` is below what decides the failure". `una check` of today reports them where that result is used (U4003 and U4001).
- `error_payload` is not re-checked: it branches on `FileError`'s variants, a case over a data kind, which F1 lacks.
- The control is accepted, with "rests on a lawful host (H1 to H3): the program uses `Files`".

**What this cannot separate.** S-05's rule, that a write's failure carries its path's label, and ADR 0085's file-state term coincide on these repros: the path is `internal`, and the inferred file state is `internal` because of that same path. So the rejections show that the re-check rejects S-05's programs. They do not show which of the two rules rejects them.

**Why neither fixed label is faithful.** `run.sh` also reads every dump with each fixed label:
- **`public`:** every leak is rejected by ADR 0085's write guard ("a write whose path or program counter is above the file-state label"), a rule that postdates S-05, and the control is accepted.
- **`internal`:** the leaks are rejected as with the inferred label, but so is the control, at its `attempt`: with an `internal` file state, every write's failure is decided at `internal`, whatever its path. This is a false rejection, and it is why the coordinator's first choice, a fixed `internal`, was replaced.

**The reader.** `dumpcore.py` now reads `attempt e` and `handle e with error ⇒ h` (`with error(x) ⇒ h` when the handler names the failure), reports a case over a data kind as that form, outside F1, and lists `std.host`'s `Files` in `host` as v0.1.3's `spec/14 §3` declares it, which today's keeps and v0.3.6's export writes: `read(path)` accepting `public` and returning `internal Text`, and `write(path, content)` accepting `internal`, both failing with `FileError`. It takes the file-state label as an optional fourth argument. The replay of S-01 to S-12 and S-04 is unchanged.
