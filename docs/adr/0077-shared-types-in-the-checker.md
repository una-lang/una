# 0077. Shared types in the checker

- Status: Proposed
- Date: 2026-10-08
- Deciders: Leonardo (pending); proposal by Claude (Una-2)
- Resolves: issue #335 (follow-up to #331, decision N-47)

## Context

The type checker represents a type as a tree that owns its children. Every expression node keeps its own copy of its type. After #331, checking a value nested `d` deep costs O(d²) time and memory, where it used to cost O(d³).

### The representation today (main `7194119`)

- **`Ty`** (`crates/una-check/src/types/ty.rs`) is an enum whose children are owned: `List(Box<Ty>)`, `Map(Box<Ty>, Box<Ty>)`, `Kind(KindId, Vec<Ty>)` and `Fn(Box<FnTy>)`. A labeled position is `Labeled(Label, Box<Ty>)`.
  - `FnTy` holds its parameters, result, `uses`, `fails` and `FnRefinements`.
  - Cloning a type copies the whole tree.
- **`Label`** (`crates/una-core/src/label.rs`) is a level plus two `BTreeSet<String>`: the categories and the symbols. The symbols are `T` for `ℓ_T`, `T*` for `ℓ*_T`, and `?n` for the label of unification variable `n`. Cloning a label allocates every string again.
- **Unification variables** (`unify.rs`, `Vars`) are slots of one declaration, holding `Option<Ty>`. `Ty::Var(n)` and `Ty::Param(n)` are indexes into the declaration being checked. The same index means different things in different declarations.
- **Equality.** `Ty` derives `PartialEq`, which compares labels. `FnRefinements` compares always equal, because refinements are not part of the base type. Type checking compares shapes through unification, not through `==`.
- **Per-expression types.** Each expression's type is stored by value in `Records::types` (`BTreeMap<Span, Ty>`). At the end of a declaration, `Records::absorb` resolves each one. Then:
  - `elab` converts each type to Core IR;
  - `elab::lost` compares it with the Core IR type (D-144, ADR 0045);
  - the LSP index formats it.

  Each of these steps is O(size of the type) per expression node.
- **Threading.** Checking runs on one thread, the large-stack `una-check` thread of `una-cli`. The `TypeCheck` it returns holds Core IR types (`una_core::ir::Type`), diagnostics and the index, never a `Ty`.

### Measurements

The workload is the package of the test `checking_deeply_nested_types_is_not_cubic_in_their_depth`: 10 bindings, each a value alternating `[...]` and `Some(...)` `d` deep. It uses edition 0.2 and runs `una check --tier 0` with a release build of `7194119`. The `instructions` column is the "instructions retired" count of `/usr/bin/time -l`, which repeated within 0.5% across runs at every depth above 10. Time is the best of three.

| depth `d` | instructions | ratio for the step | best time | peak memory |
| --- | --- | --- | --- | --- |
| 10 | 68 M | | < 0.01 s | 7.3 MB |
| 20 | 139 M | 2.05 | 0.01 s | 9.0 MB |
| 40 | 403 M | 2.89 | 0.03 s | 14.4 MB |
| 80 | 1,418 M | 3.52 (from 40) | 0.11 s | 33.0 MB |
| 160 | 5,372 M | 3.79 (from 80) | 0.44 s | 102.0 MB |

- **Growth.** Doubling `d` multiplies the instructions by nearly 4 and the memory by about 3. This is the quadratic term.
- **Depth limit.** Nesting past about 200 is U0101, the parser's limit of 256 levels (`spec/02 §9`). So the quadratic cost is bounded, but at its bound a package of ten such bindings needs 100 MB.

A sampling profile at `d = 160`, with 40 bindings, attributes the active samples as follows (by the function on top of the stack):

| share | where |
| --- | --- |
| 52.6% | `malloc`, `free`, `__rust_alloc`, `__rust_dealloc` |
| 14.7% | dropping `Ty` trees and the `BTreeSet<String>` of labels |
| 4.0% | `Ty::clone`, `memmove` |
| 13.3% | type walks: `resolve_shape`, `erase`, `map_labels`, `combine`, `occurs` |
| 2.5% | the flow checks: `violation`, `below` |
| 2.4% | `elab` (`convert`, `lost`) |
| 3.2% | formatting types |
| 7.4% | other |

About 71% of the time goes to making, copying and freeing types that are equal to a part of a type that already exists.

**Ordinary programs are not affected today.**
- Every example package checks in 27 to 40 M instructions and 5 to 6 MB. Of that, 27 M is the prelude and the standard modules alone (`examples/hello`).
- The fixpoints of ADR 0067 (result summaries) are linear in the number of generic functions up to 400. That holds for a chain and for one recursive cycle of one-line generic functions passing `List[Option[T]]`: about 1.8 M instructions per function.

Beyond about 1,000 functions, those fixpoint workloads grow faster than linearly: ×2.12 to ×2.66 per doubling, up to 16,000 functions. The profile places that cost in a loop of `check_types` itself, not in copying types. It is reported separately and is out of scope here.

### Forces

- **The owner's rule** (CLAUDE.md, "Performance"): each choice takes the best average-case complexity, and an ADR states the time and space complexity of every option.
- **Generated code.** Una is written for machine-generated software (`docs/00-charter.md`). Generated code nests more than hand-written code does, so the checker's cost should be linear in the program's size.
- **Every type operation is affected.** The checker, `unify`, `elab`, the flow checks of `spec/07 §7` (labels, `ℓ*_T`, `deep_distinct` since #354) and the LSP index all touch types. An error in the representation is a soundness error.

## Options

`n` is the size of the program. `m` is the number of distinct type nodes an operation reaches. `t` is the size of a type written out as a tree. For deep nesting, `t = O(d)` per expression and `m = O(d)` in total, because each expression's type is a part of its parent's.

### A. Shared immutable nodes (`Rc`) with sharing-preserving operations

- **Handles.** `Ty` becomes a handle to an immutable node, `Rc<TyNode>`, and the children of a node are handles. `Label` keeps its meaning, with its symbols and categories interned (A1 below).
- **Sharing-preserving operations.** Each operation that rebuilds a type returns its input handle when none of the input's children changed. This covers `resolve_shape`, `erase`, `map_labels`, `with_label`, `raise_deep`, `substitute`, `combine` and `starred`. A type that an expression shares with its parent therefore stays shared.
- **Memoized walks.** A walk over a shared type memoizes its result by node address, for the duration of that walk, so it visits each distinct node once.
- **One memo per declaration at the end.** `Records::absorb` resolves all of a declaration's types with one memo, once unification for that declaration is done.
- **No in-place changes.** Nothing changes a node in place: there is no `Rc::get_mut` and no interior mutability on a node another handle can reach.
- **A1, interned labels.** The checker interns the strings of label symbols and categories (`T`, `T*`, `?n`, `PII`) in a table per package. A set of them becomes a small sorted vector of ids, or an `Rc` of one, so a label is cloned without allocating. The table keeps each name, so messages show the same names in the same order as today: labels print sorted by name, not by id.

| | Time | Space |
| --- | --- | --- |
| Clone a type | O(1) | O(1) |
| A walk (resolve, erase, raise, substitute) | O(m), O(1) expected per node for the memo | O(m) for the memo, freed after the walk; O(changed nodes) for the result |
| `==` | O(1) when both handles are the same node; otherwise O(t), as today | O(1) |
| Checking a value `d` deep | O(d) | O(d) |
| Checking a program | O(n) for the type work, given the bounded arity of type constructors | O(n) |

- **Pros:**
  - Linear time and memory for #335's case.
  - Each change is local: one module at a time, behind the same function signatures.
  - No global identity, so equality keeps its exact meaning.
  - Types are acyclic (the occurs check), so `Rc` cannot leak through a cycle.
- **Cons:**
  - `Rc` is not `Send`. That is sound only because types never leave the checker's thread. It must stay so, and the compiler enforces it, because `TypeCheck` crosses a thread boundary in `una-cli`.
  - Each walk pays for a hash map per call when the type is shared. A walk on a small type should skip the memo, with a threshold set by measurement.

### B. Hash-consed types in an arena (`TyId`)

- **The arena.** One arena per package holds each distinct node once, keyed by its whole content. A type is a `TyId(u32)`, `Copy`.
- **Equality** of structure is `TyId` equality.
- **Memoized operations.** Operations are memoized for the whole run by `(operation, TyId)` for types without unsolved variables. A type with variables is memoized by `(operation, TyId, generation)`, where the generation of the declaration's `Vars` grows at every solved variable.

| | Time | Space |
| --- | --- | --- |
| Make a node | O(arity) expected (hash of the children's ids) | O(1) when it already exists |
| Clone, `==` | O(1) | O(1) |
| An operation on a type seen before | O(1) expected, memoized | O(1) |
| Checking a value `d` deep | O(d) | O(d) |
| Checking a program | O(n) for the type work | O(distinct nodes ever made), freed only when the check ends |

- **Pros:**
  - Equal types are shared across the whole program.
  - Equality is constant time.
  - Repeated operations on the same type are free.
- **Cons, about soundness:**
  - **Refinements.** The arena key must include labels and `FnRefinements`, while `PartialEq` deliberately ignores refinements. Keying nodes by `PartialEq` would merge two function types that differ only in refinements, and the verifier would then lose an obligation (ADR 0037).
  - **Indexes.** `Var(n)` and `Param(n)` are per declaration. Either the arena is per declaration, losing most of the sharing, or every memo carries the declaration.
  - **Stale memos.** A memo of a type with variables goes stale as unification proceeds. A missing generation bump returns a stale type, which is a silent unsoundness.
- **Cons, about cost:**
  - The fixpoints of ADR 0067 and ADR 0075 re-check bodies, so the arena keeps every intermediate type until the check ends.
  - Every function of `types/` (about 20,000 lines), `elab` and the index take the arena by reference. It is the largest change of the three.
  - Every node made pays for hashing.

### C. Keep owned trees; remove the constant costs

- **Labels:** interned symbols and categories, as A1.
- **Fewer stored types:** `Records::types` keeps only the types that later passes read. That is the types of names, calls, lambdas and annotations, not every subexpression of a literal.
- **Records:** resolved with a per-declaration memo keyed by span parent.

| | Time | Space |
| --- | --- | --- |
| Clone a type | O(t) | O(t) |
| Checking a value `d` deep | O(d²), with a smaller constant | O(d²) |

- **Pros:**
  - The smallest change.
  - Labels are cheaper everywhere.
- **Cons:** it keeps the quadratic term that #335 asks to remove.

## Decision (proposed)

**Option A**, in three stages. Each stage is its own pull request and is gated by the identity and scaling tests below.

1. **Interned labels (A1).** `Label`'s symbols and categories become interned ids within the checker. Display keeps the names and their order. This is measurable on its own, and option C would need it too.
2. **`Ty` as `Rc<TyNode>`.**
   - Every rebuilding operation preserves sharing: it returns the input when nothing changed, and memoizes by node within one walk.
   - No code mutates a reachable node.
   - `TypeCheck` stays free of `Ty`, so that `Send` keeps proving that types stay on one thread.
3. **Stored types and later passes.**
   - `Records::types` holds handles.
   - `Records::absorb` resolves with one memo per declaration.
   - `elab::convert`, `elab::lost` and the index's formatting memoize by node for one declaration.

Hash-consing (B) can follow A later, if a measured workload needs program-wide sharing or constant-time equality. A's handles are its prerequisite.

## Soundness

- **Labels, `ℓ_T` and `ℓ*_T`.** Sharing changes how a type is stored, never which labels it has.
  - Every operation that changes a label makes new nodes on the path to that label: `with_label`, `raise_deep`, `map_labels`, `starred`, `without_decisions`, `combine` and the settling of variable labels.
  - A shared node is never relabeled in place, so another type that shares it is unchanged.
  - Interning keeps `T` and `T*` as distinct symbols. The operations that relate them work on the symbol table, which records for each name its deep form, instead of on string suffixes:
    - `flow::below`, whose `ℓ_T ⊑ ℓ*_T` holds since #354;
    - `starred`;
    - the stripping in `without_decisions`.
- **Equality.**
  - The meaning of `==` stays the derived one: labels compared, refinements ignored.
  - Pointer equality is only a fast path to `true`. Different addresses never imply `false`.
  - No code may use pointer identity where it means type equality, or the reverse.
- **The flows of `spec/07 §7`** (`violation`, `below`, `deep_distinct` in both editions) read labels through the same accessors, and their results do not change.
- **Variables.**
  - A memo lives for one walk, or for one declaration's absorb once its unification is done.
  - No memo outlives an iteration of the fixpoints of ADR 0067 (result summaries) or ADR 0075 (label variables, pass 1).
  - `Var(n)` and `Param(n)` keep their per-declaration meaning, because A has no global identity.
- **Refinements** stay in the node that carries them. A never merges two nodes, so no refinement can be dropped (ADR 0037, D-150, D-153).
- **Determinism.** Memos keyed by address are internal and are never iterated into output. Diagnostics, Core IR, SMT queries and the index stay byte-identical.

## Test and scaling plan

- **Identity**, at every stage, old binary against new, with the method of #326 and #332:
  - Every conformance case, example package and review corpus under `docs/reviews/` is checked in a fresh copy per binary.
  - The comparison covers the full `una --json check` output, the names of the proof-cache files (each a hash of a query script, so equal names mean byte-identical queries), and the cached verdicts.
  - `una contract` and `una report` are compared too, and so is the LSP index of the examples.

  Any difference blocks the stage.
- **Scaling, by a deterministic counter.** This follows the owner's rule and N-29/#341.
  - A test-only counter of type nodes made, behind `cfg(test)` or a feature that release builds do not enable.
  - The test checks the #331 workload at `d = 40` and `d = 160`. The node count may grow at most 6x: linear growth is 4x, and quadratic growth is 16x.
  - A second counter, of label-symbol allocations, checks stage 1. Labels cloned per check must not grow with `d`.
- **Scaling, by time, as the secondary check.** The existing test `checking_deeply_nested_types_is_not_cubic_in_their_depth` tightens its bound from 40x to 10x for 4x the depth, after stage 3.
- **Measurements.** The table of this ADR is measured again after each stage and recorded in its pull request: instructions retired, peak memory, and the profile's allocation share.
- **Properties.** On random types with labels, symbols `T`, `T*` and `?n`, refinements and variables, the shared operations give the same results as the tree operations they replace:
  - `resolve`, `erase`, `map_labels`, `raise_deep`, `combine` and `starred`, compared through a conversion back to an owned tree;
  - `==`, for both equal and different pairs.
- **Review.** An independent adversarial check, as #335 asks. It gets a matrix like `docs/reviews/v0.3.1/matrix` for labels through shared subterms. For example: two bindings that share a subterm, where one is relabeled; and a decided `T*` at a shared position.

## Consequences

- **Easier:**
  - Deep and generated programs check in linear time and memory.
  - Cloning a type is free, which removes the reason for much defensive code that avoids clones.
  - Labels stop allocating on every join.
- **Harder:**
  - Code that rebuilds a type must use the sharing-preserving constructors, or it silently loses the linear bound. The scaling counters catch that, the identity runs do not.
  - `Ty` cannot move to another thread.
- **Deferred:**
  - Hash-consing (option B).
  - The superlinear growth of very large numbers of generic functions, measured in the Context. It is a separate issue, and its cost is not in copying types.
- **Priority:** low, as #335 says. Real programs today nest only a few levels, and none of this changes what any program means.

## Alternatives considered

- **B, hash-consing, as the first step.** Not chosen. It gives the same linear bound for #335's case, at the largest refactor and with three soundness hazards A does not have: refinements merged by a wrong key, per-declaration indexes, and stale memos across unification. Its extra benefits (constant-time equality, program-wide sharing) are not where the measured time goes.
- **C alone.** Not chosen. It keeps the quadratic term. Its label interning is kept as stage 1 of A.
- **Do nothing.** Not chosen. The parser's nesting limit bounds the quadratic cost, but at about 100 MB for ten deep bindings. The owner's rule asks for the best average case.
