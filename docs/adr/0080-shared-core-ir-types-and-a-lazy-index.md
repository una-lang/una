# 0080. Shared Core IR types and a lazy editor index

- Status: Proposed
- Date: 2026-10-08
- Deciders: Leonardo (pending); proposal by Claude (Una-2), at the coordinator's request (N-58)
- Follows: ADR 0077 (shared types in the checker), issue #335

## Context

ADR 0077 made the checker's own types linear in the program's size. A value nested `d` deep now makes O(d) type nodes in the checker (stage 3a: 3.95× and 3.98× for 4× the depth, against 16× for quadratic growth).

Checking still takes about 14× the time for 4× the depth, against 20× before ADR 0077. Two consumers of the checker are quadratic in their own output. The checker's work does not cause this; what they produce does.

**Core IR types.** `elab` writes the program in A-normal form. Every intermediate value gets a binder, and every binder gets an owned `una_core::ir::Type`:
- `Type::Base { base, refinements, label }`, where `Base::Named { args: Vec<Type> }` holds a kind's arguments, lists included;
- `Type::Fn { params: Vec<Type>, result: Box<Type>, uses, fails, label }`.

A value `d` deep has O(d) binders, each typed by an owned tree of size O(d), so the Core IR holds O(d²) type nodes. Three things follow:
- `elab::convert` builds those trees;
- `elab::lost` compares each of them with the checker's (D-144, ADR 0045);
- una-cli drops them.

A memo on `convert` would save the conversion but not the trees: each binder still owns a copy.

**The editor index.** `Index::types` is a `Vec<(Span, String)>`: every expression's type, formatted. `una lsp` wraps each one as a hover. A value `d` deep has O(d) expressions whose types print in O(d) characters each, so the index holds O(d²) characters. Hovers are served on request, but all of them are built when the package is checked.

**Measured at depth 160** (the #331 program, ten bindings, release build, main 9b9af67 with ADR 0077 stage 3a), by top-of-stack samples:
- `elab::convert` and dropping `ir::Type`: about 7%;
- formatting the index's types: about 3%;
- allocation and copying, much of it for these trees and strings: the larger part of the remaining 37% malloc and free.

**Who reads Core IR types.** Uses of `Type::` by crate:
- una-core: 43;
- una-check: 33, `elab`;
- una-verify: 47, the encoding of obligations into SMT;
- una-interp: 4;
- una-codegen: 48, the native backend;
- una-cli: 2;
- una-rt: 3.

Core IR is the input of the verifier, the interpreter and the native backend. Changing how its types are held changes the trusted computing base, the TCB (`spec/09`, `spec/16`). Core IR also leaves the checking thread (`TypeCheck: Send`), so any sharing must be `Arc`, not `Rc`.

**Priority.** It is low, as #335 says. Real programs nest only a few levels, and the parser bounds nesting at 256 (`spec/02 §9`). At that bound, ten bindings check in under a second. The payoff appears only for generated programs near the bound.

## Options

`n` is the size of the program and `d` the depth of nesting. For the #331 program, n = O(d) expressions, each with a type of size O(d).

### A. Shared Core IR types (`Arc`) and a lazy index

- **Core IR:**
  - The children of `ir::Type` become shared and immutable: `Base::Named { args: Arc<[Type]> }`, `Fn { params: Arc<[Type]>, result: Arc<Type> }`. Consumers read through `Deref`, so most reads are unchanged, while every constructor changes.
  - `elab::convert` converts each checker node once per declaration and shares the result: a memo keyed by the checker node, under ADR 0077's memo rules. Equal checker nodes then give the same Core IR node.
  - `elab::lost` compares memoized pairs once.
- **Index:**
  - `Index::types` holds, for each span, a `Send` handle to its type: the shared Core IR type, or a `Send` snapshot of the checker's. It does not hold its text.
  - `una lsp` formats a type when a hover asks for it.

| | Time | Space |
| --- | --- | --- |
| Core IR types of a program | O(n) | O(n) shared nodes |
| `elab::convert` and `lost` | O(n) for the whole program | O(n) |
| Index | O(n) to build; O(size of one type) per hover | O(n) |
| Checking a value `d` deep | O(d) | O(d) |

- **Pros:**
  - Checking becomes linear end to end.
  - The verifier, the interpreter and the backend read types as before.
- **Cons:**
  - It is a TCB-wide change. Every function that builds an `ir::Type`, in elab, verify and codegen, changes.
  - `Arc` adds an atomic reference count to each node.
  - The index's hover text moves from check time to request time.

### B. A lazy index only

The index as in A. Core IR stays as it is.

| | Time | Space |
| --- | --- | --- |
| Core IR types | O(n·d), unchanged | O(n·d) |
| Index | O(n) | O(n) |
| Checking a value `d` deep | O(d²), with a smaller constant | O(d²) |

- **Pros:**
  - It touches only the index and `una lsp`, outside the TCB.
  - It removes the index's share.
- **Cons:** the quadratic term of Core IR remains.

### C. Keep both and bound the index

Core IR as it is. The index stores at most a fixed number of characters per type (for example 2,000) and an elision mark. A hover past the bound shows the elided text, and a request for the full type recomputes it.

| | Time | Space |
| --- | --- | --- |
| Index | O(n) | O(n) |
| Checking a value `d` deep | O(d²) | O(d²) |

- **Pros:** the smallest change.
- **Cons:**
  - Hover text changes for very large types.
  - The quadratic term of Core IR remains.

### D. Do nothing

The quadratic term stays where it is: at the parser's nesting bound, in Core IR and in the index. ADR 0077 already took the checker's own part.

## Decision (proposed)

None yet: the choice is the owner's. My recommendation is **B now and A later, if a measured workload needs it**:
- B is outside the TCB and removes one of the two remaining quadratic terms.
- A pays off only near the nesting bound, and it changes what the verifier, the interpreter and the backend read. The owner's rule asks for the best average case (CLAUDE.md, "Performance"), and the average program does not nest deeply enough for A's gain to outweigh a TCB change.

If the owner wants checking linear end to end, A is the option that does it.

## Soundness and the TCB

- **Sharing Core IR types changes how they are held, never what they say.** A shared node is immutable: no `Arc::get_mut` or `make_mut`, with a guard test as for the checker's `Shared`.
- **Equality** stays by content. Pointer equality is only ever a fast path to `true`.
- **The memo of `convert`** follows ADR 0077's rules:
  - it keeps its keys alive;
  - a test drops temporaries during a memoized walk;
  - nothing in it depends on variables being solved, since it runs after `Records::absorb`.
- **The verifier's encoding** reads types by structure. Its queries must be byte-identical, and the proof-cache names, which hash each query script, prove it.
- **The native backend's layouts** come from types. Executables must be byte-identical, linked with one runtime.
- **The index** moves text from check time to request time. The text of each hover must be the same.

## Test and identity plan

- **Identity**, old binary against new, on every conformance case, example package and review corpus (about 2,200 targets):
  - `una --json check`;
  - the proof-cache names and bytes, which show that every verifier query is unchanged;
  - `report`, `obligations` and `test`.
- **Core IR dumps:** `una check --dump-core` of every module, before and after, byte-identical. It prints every binder's type, label and refinements (`spec/09`, ADR 0032).
- **Native:** every runnable program built with one runtime gives the same executable bytes, and its run gives the same output, the nondeterministic set of spec/19 §7 rule 6 apart.
- **Index:** every hover of every example and conformance case gives the same text, old and new.
- **Scaling by deterministic counters:**
  - Core IR type nodes made, at depth 40 and 160: at most 6× for 4× the depth;
  - characters formatted for the index at check time: O(n).

  The time test of `checking_deeply_nested_types_is_not_cubic_in_their_depth` tightens from 24× to 10× only with A. B alone takes it to about 12×, to be measured.
- **Review:** an independent adversarial check of the verifier's and the backend's outputs. A is a TCB change.

## Consequences

- **With A:** checking is linear in the program's size end to end, and Core IR uses less memory. Every construction of a Core IR type changes, in three crates of the TCB.
- **With B:** the editor index is linear, and hovers are formatted on request. Core IR keeps its quadratic term.
- **With C or D:** nothing changes in the TCB, and the remaining quadratic term is documented.

## Alternatives considered

- **Hash-consing Core IR types.** It gives the same bound as A, with the hazards ADR 0077 lists for hash-consing: refinements that equality ignores, and stale entries. It also changes the TCB's types more deeply than A does.
- **Typing only the binders that need a type.** This would change the Core IR that `spec/09` defines: "every binder carries its full type, including refinement and label" (`spec/09` rule 2), and the verifier reads them. So it needs a specification change of its own.
