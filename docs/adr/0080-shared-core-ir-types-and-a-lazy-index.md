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

## Implementation plan for option B

A concrete plan, so that B can start as soon as it is chosen. It changes una-check's index, una-cli's `lsp.rs` and una-lsp's hover store. It changes no Core IR, no verifier input and no executable.

### What is formatted today

`types::index::build` walks `Records::types`, a `BTreeMap<Span, Ty>`. For every span it formats `ShowLabeled(ty, &DeclNames)` into a `String`, and `una lsp` (`lsp.rs::index`) wraps each one in Markdown into `Checked::hovers: Vec<(Location, String)>`. The server keeps them per file in `Found::hovers` and, on `textDocument/hover`, returns every text of the innermost range that contains the cursor, without repeats.

`ShowLabeled` and `Show` (`ty.rs`) read only:
- the shape of a type: `List`, `Map`, `Kind` with its arguments, `Fn` with its parameters and result, `Labeled`, and the leaves `Int`, `Fixed`, `Bool`, `Text`, `Unit`, `Hashed`, `Param`, `Var` (`_`), `Never` and `Error` (`?`);
- the label of each `Labeled` node;
- a kind's name, through `Names::kind_name`, and a type parameter's name, through `Names::param_name`.

A function type prints as `fn(params) -> result`: its `uses`, `fails` and refinements are not shown, so a snapshot need not hold them.

### 1. A `Send` snapshot of the checker's types

The checker's types share their nodes through `Rc` (ADR 0077), which cannot leave the checking thread; the index must, since `TypeCheck: Send`. A new module, `types/hover.rs`, defines the snapshot:

```text
pub struct HoverType(Arc<Node>);          // Send + Sync, immutable

enum Node {
    Leaf(&'static str),                   // Int, Bool, Text, Unit, Hashed, Never, `_`, `?`, I8..U64
    Param(usize),                         // printed through the declaration's names
    List(HoverType),
    Map(HoverType, HoverType),
    Kind(KindId, Arc<[HoverType]>),       // printed through the package's kind names
    Fn(Arc<[HoverType]>, HoverType),
    Labeled(Label, HoverType),            // una_core::Label: plain data
}

pub struct HoverNames {                   // one per check, shared by every hover
    kinds: Arc<[String]>,                 // Env::kind_name of each KindId
}

pub struct Hover {                        // one per span
    ty: HoverType,
    params: Arc<[String]>,                // the type parameters of the enclosing declaration
    names: Arc<HoverNames>,
}
```

- `Param` and `Kind` keep indices, not text, so a node does not depend on the declaration it appears in, and one node serves every declaration.
- `HoverNames::kinds` holds `Env::kind_name` of each kind, the bare name that `DeclNames` prints today; "Text" below says why, and how to show the qualified name instead.
- `params` is the `Vec<String>` that `params_at` already computes. It becomes one `Arc<[String]>` per declaration, built once and cloned per span.

### 2. Built once per node, under ADR 0077's memo rules

`hover::snapshot(ty, memo)` converts a checker `Ty` bottom-up:
- **Memo.** It is keyed by the address of each `Shared` node, as `erase` is, and it stores the `Shared` handle beside the result, so no key is freed and reused while the memo lives (ADR 0077, memo rule 1). It lives for one call of `index::build` and is dropped after it.
- **Independence.** No result depends on a solved variable: `Records::types` holds types after `Records::absorb`, and a `Var` that is left prints as `_` whatever its number.
- **Leaves** are not memoized; each is a constant node.

Each distinct checker node is converted once. Since ADR 0077 stage 3a the checker holds O(n) nodes for a program of size n, so building every hover is O(n) time and space, where formatting them was O(n·d).

### 3. The index holds handles, not text

`Index::types: Vec<(Span, String)>` becomes `Vec<(Span, Hover)>`. `Hover` implements `Display` with the printer of step 4, and `Hover::text()` returns the `String`.

### 4. One printer, two trees

Two printers that must agree would drift. `ShowLabeled` and `Show` become generic over a small trait:

```text
trait TypeView {
    fn view(&self) -> View<'_, Self>;     // the shape above, borrowing the children
}
```

`Ty` implements it with its `Names`, and `HoverType` with `Hover`'s `params` and `names`. The printer is written once, over `View`, and both trees print through it. Messages keep `ShowLabeled(&Ty, names)` as their entry point, so no call site of a message changes.

### 5. The language server formats on request

- `una_lsp::Checked::hovers` becomes `Vec<(Location, HoverText)>`, where `HoverText` is either `Ready(String)`, for contracts, obligations and lenses, or `Lazy(Arc<dyn HoverSource>)`, with `trait HoverSource: Send + Sync { fn text(&self) -> String; }`. Debug and Clone are implemented by hand, for the trait object.
- `lsp.rs::index` pushes `Lazy(Arc::new(TypeHover(hover)))`, whose `text()` wraps `hover.text()` in the same ```` ```una ```` fence as today.
- `Server::hover` finds the innermost range as now, from locations only. Then it formats only the hovers of that range: one type, plus perhaps a contract and an obligation. The duplicate check compares texts after formatting, as now.
- The cost of a hover is O(size of the one type shown) per request, where the check paid for every hover of the package.

### 6. Text

Every hover must keep its text, byte for byte, with one deliberate exception to settle before coding. `DeclNames` prints a kind by `kind_name`, the bare name, while messages use `kind_display` since #397. B can keep the bare name, a byte-identical change, or show the qualified name where two kinds share one, a visible improvement. The plan assumes **bare**, so that identity holds everywhere. The qualified form can be a later one-line change of `HoverNames`.

### Tests

- **Same text, whole corpus.** A una-check test checks every conformance case, example package and guide block. For every span of `Records::types`, it asserts that `ShowLabeled(ty, &DeclNames)` and `snapshot(ty).text()` are the same string. This is the old printer against the new tree, inside one binary.
- **Language server.** The existing `crates/una-cli/tests/lsp.rs` hover tests pass unchanged. A new test hovers every expression of one example package through the server and compares the texts with a dump taken before the change: `una lsp`'s answers, old binary against new.
- **Send.** A compile-time assertion that `Index: Send` and `Hover: Send + Sync`.
- **Scaling, by deterministic counters.** Each `index::build` returns its counts in a `HoverStats` struct, not in a process-wide counter: #335 stage 3a showed a global counter makes concurrent tests flaky.
  - Snapshot nodes made at depth 40 and 160 of the #331 program: at most 6× for 4× the depth.
  - Characters formatted for types while checking: zero.
  - Timing stays a wide secondary check, as the owner's rule asks.
- **Identity.** `una --json check`, `report`, `obligations`, `test` and the proof-cache names are byte-identical on every target, and so are builds with one runtime. B touches none of them, and the run proves it.

### Steps and size

1. `types/hover.rs`: the snapshot and its memo, with the corpus text test, and the printer made generic (step 4). About 250 lines with tests, in una-check only.
2. `Index::types` holds `Hover`, and `lsp.rs` and una-lsp's `HoverText` follow (steps 3 and 5). About 120 lines in three crates outside the TCB.
3. Counters and the scaling test. The time bound of `checking_deeply_nested_types_is_not_cubic_in_their_depth` is then measured again; ADR 0080 expects about 12× of the 14× to remain, from Core IR.

One pull request. Review: the language server's behavior, and the memo against ADR 0077's rules. No TCB review is needed, since no output of the verifier, the interpreter or the backend can change, and the identity run shows that.

### Risks

- **Hover latency.** A hover now formats on request. The largest type at the nesting bound prints in about 256 levels, well under a millisecond, but a test asserts no hover of the examples takes more than 10 ms.
- **Lifetime of a check's data.** A `Hover` keeps its nodes alive for as long as the server keeps the check, until the next one replaces it. That is O(n) memory, where the strings were O(n·d).
- **A wrong memo key** would show another type's text. The corpus text test catches it, and the memo keeps its keys alive (ADR 0077).

## Consequences

- **With A:** checking is linear in the program's size end to end, and Core IR uses less memory. Every construction of a Core IR type changes, in three crates of the TCB.
- **With B:** the editor index is linear, and hovers are formatted on request. Core IR keeps its quadratic term.
- **With C or D:** nothing changes in the TCB, and the remaining quadratic term is documented.

## Alternatives considered

- **Hash-consing Core IR types.** It gives the same bound as A, with the hazards ADR 0077 lists for hash-consing: refinements that equality ignores, and stale entries. It also changes the TCB's types more deeply than A does.
- **Typing only the binders that need a type.** This would change the Core IR that `spec/09` defines: "every binder carries its full type, including refinement and label" (`spec/09` rule 2), and the verifier reads them. So it needs a specification change of its own.
