# 0137. `const` and `var` replace `let` and `let edit`

- Status: Proposed
- Date: 2026-11-05
- Deciders:
- Resolves: issue #1627
- Related:
  - ADR 0126 (module constants), whose constants are written `const` under this record, and which moves with it to edition `0.5`;
  - ADR 0056 (the memory model, editions) and `spec/17`, which define editable bindings, places, `edit` and `take`;
  - ADR 0121 (the terms of a preview edition), whose rules 4 and 5 forbid a silent change of meaning;
  - ADR 0105 (`test` replaces `example`), the precedent for a word that an edition replaces with an exact rewrite;
  - ADR 0096 (exact rewrites), whose renames and repairs `una fix` applies;
  - issue #1620 (edition numbering follows the release that closes it), which names edition `0.5`;
  - issue #1591 (data-driven and multi-step tests), which may add bindings to tests.
- Target: edition `0.5`. Editions `0.1` to `0.4` do not change. The benchmark baseline, pinned to the release line of edition `0.1`, does not move.
- Amends (when accepted, with the implementation):
  - **Lexical structure:** `spec/01 §5` (the keyword tables of edition `0.5`);
  - **Grammar:** `spec/02 §6` (`let_stmt`), `§8` (the context restrictions that name `let`);
  - **Memory model:** `spec/17 §1` to `§5` and `§9` (editable bindings are `var` bindings and `edit` parameters);
  - **Concurrency:** `spec/19 §1` to `§3` (the branches of `together`);
  - **Static semantics and verification:** `spec/10 §5`, `§11` and `spec/12 §4`, where they name `let`;
  - **Tests:** `spec/08 §2` (a binding in a test is written `const`);
  - **Canonical form:** `spec/15`;
  - **Diagnostics:** `spec/13 §4` (U0103, U2019 and the messages that name `let`), `§7` (edition `0.5`);
  - **Accepted and proposed records:** ADR 0126, by a revision while it is Proposed (Appendix A).

> **In plain words.**
> - **Two words, one meaning each.** `const total = 0` names a value that never changes. `var total = 0` names a value that the function may assign again. `let` and `let edit` are their names in editions `0.1` to `0.4`, and edition `0.5` does not have them.
> - **Parameters do not change.** `edit` and `take` stay the markers of parameters and arguments: `bump(edit n: Int)`, `bump(edit count)`, `consume(take order)`.
> - **Module constants are `const` too.** `pub const pi: F64 = 3.141592653589793` at the top level of a module (ADR 0126). At the top level the value must be one the compiler can compute; in a function body it can be any expression.
> - **Moving is mechanical.** A package that changes its edition to `0.5` gets one diagnostic per `let`, each with its exact rewrite, and `una fix` applies them all.

## Questions for acceptance

Each question changes what a programmer writes. The decisions are the project's; the text below follows the decisions taken, and the recommendations where none is taken yet.

1. **Which words bind a name?**
   - **(a) Chosen:** `const` for an immutable binding, `var` for a rebindable one.
   - **(b)** Keep `let` and `let edit`.
   - **(c)** `let` and `var`, as Swift writes them.
   - **(d)** `const` and `let`, as JavaScript writes them.
   - **Cost of (a):** every binding of every program is rewritten once, mechanically (Costs).
   - **Decided:** the project, 2026-11-05: (a).

2. **What does the rename cover?**
   - **Chosen:** every binding statement: locals, the branches of `together`, bindings of tests, and the constants of ADR 0126. `let` leaves the language in edition `0.5`, reserved and rejected with its rewrite. `edit` and `take` stay what they are, the markers of parameters, arguments and method receivers; they no longer appear in a binding statement.
   - **Decided:** the project, 2026-11-05.

3. **Which edition?**
   - **Chosen:** edition `0.5`, together with ADR 0126, so that `let` and `const` never name a binding in one edition. Its number follows the release that closes it (issue #1620).
   - **Decided:** the project, 2026-11-05.

4. **Does `una fix` turn a `var` that is never assigned into a `const`?**
   - **Recommended: no.** The rewrite maps forms one to one (rule 9): `let edit x` becomes `var x` whether or not the body assigns `x`, so the rewrite never depends on an analysis and is idempotent. A later lint of Una Quality may report a `var` that is never assigned, as advice.
   - **Cost of the alternative:** a rewrite that reads assignments, `edit` arguments and joins, which would change if the analysis changed.
   - **Decided:**

5. **Does the printed form of Core IR keep its `let`?**
   - **Recommended: yes.** Core IR is not Una source: it has no `edit`, no `take` and no assignment, and its `let x = e in ...` is the binder of the formal model and of `una.core/1` (`spec/09`). Renaming it would change a published schema and every formal rule for no change of meaning. The documents say "a Core `let`" where they mean it.
   - **Decided:**

## Glossary

- **`const` binding.** A name bound once to a value, in a block or at the top level of a module. It cannot be assigned.
- **`var` binding.** A name bound to a value that the function may assign again; with `edit` parameters, the editable bindings of `spec/17 §2`.
- **Marker.** `edit` or `take` written before a parameter, an argument or a method receiver (`spec/17 §3`).
- **Position rule.** At the top level of a module, a `const` needs a written type and a constant expression; in a block, neither.

## Context

- **The forms today.** Edition `0.1` binds with `let x = e`. Edition `0.2` adds `let edit x = e`, a binding that can be assigned (`spec/17 §1`, `§2`). `var` is reserved in every edition and rejected with U0103: "not a keyword in Una; values are immutable, use `let`" in edition `0.1`, and "not a keyword in Una; use `let edit`" from edition `0.2` (`spec/01 §5`, `spec/17 §9`). `const` is an ordinary identifier, and no `.una` file of the language repository uses it as one.
- **How much code uses them.** On `main` at the time of writing, the language repository has about 4,300 `.una` files; about 1,900 of them hold about 6,700 lines with `let`, of which about 940 are `let edit`. Its Markdown documents hold about 1,000 lines with `let`, about 900 of them under `docs/`. The other repositories of the project hold about 1,000 `.una` files with about 3,600 lines with `let`, and Una code in pages, primers and grammars besides.
- **What agents and programmers read into the words.** In JavaScript and TypeScript, `let` declares a *mutable* binding and `const` an immutable one: a reader trained on them reads Una's `let` as the rebindable form, the opposite of its meaning. In Swift, `let` is immutable and `var` mutable. In Rust, `let` is immutable, `let mut` mutable, and `const` a compile-time item. In JavaScript, `const` is a runtime immutable name. `const` therefore means "this name does not change" in every language a reader is likely to know, and `var` means "this one does". `let` means opposite things in the two families.
- **Module constants.** ADR 0126 adds named values at the top level of a module. Written with `let`, the word would bind both a local and a module value; written with a new word next to `let`, Una would have two words for one meaning, an immutable name. Una's syntax keeps one word per meaning (`spec/01`, `spec/02`).
- **`edit` in two roles.** Today `edit` marks a parameter and an argument (passed in and out) and also a binding (`let edit`, assignable). The two are related, since an `edit` argument needs an editable place, but they are different things: one is a calling convention, the other a kind of binding.
- **Editions.** A change of a keyword lands in a new edition: editions `0.1` and `0.2` are stable, and `0.3` and `0.4` keep their terms (ADR 0121 rules 13 and 18). ADR 0105 replaced `example` with `test` in edition `0.3` with a U0103 and an exact rewrite, the model followed here.

## Decision

In one paragraph: in edition `0.5`, `const` and `var` are keywords; a binding statement is `const` or `var`; a `const` binding is immutable and a `var` binding can be assigned; `let` is reserved and rejected with U0103 and the exact rewrite to `const` or `var`; `edit` and `take` are only markers of parameters, arguments and receivers; a module-level `const` is ADR 0126's constant; every message names the edition's words; `una fix` rewrites a package from edition `0.4`; editions `0.1` to `0.4` are unchanged.

1. **Keywords** (`spec/01 §5`). In edition `0.5`:
   - `const` and `var` are reserved keywords;
   - `let` leaves the keywords and joins the table of reserved words with U0103, with the message `` use `const`, or `var` for a binding that is reassigned ``, written `` `let` is not a keyword in edition 0.5; use `const`, or `var` for a binding that is reassigned ``;
   - the row of `var` in the U0103 tables is removed, since it is a keyword;
   - every other row is that of edition `0.4`.

   In editions `0.1` to `0.4` nothing changes: `let` is a keyword, `var` gives its U0103 message of that edition, and `const` is an identifier. There a line that starts with `const` or `var` followed by a lower-case name, at the top level or in a block, keeps its diagnostic (U0101 for `const`, U0103 for `var`) and gains the help `` `const` and `var` are keywords of edition 0.5 ``.

2. **Binding statements** (`spec/02 §6`, `spec/17 §1`). In edition `0.5`:

   ```ebnf
   let_stmt        = ( "const" | "var" ) lower_ident [ ":" type ] "=" ( handled | block_expr ) ;
   ```

   The production keeps its name, as `spec/09` keeps the Core `let`. `const x = e` is what `let x = e` is in edition `0.4`, and `var x = e` what `let edit x = e` is, with the same typing, desugaring, verification and information flow (`spec/10 §5`, `spec/17 §4` to `§6`). `const edit x` and `var edit x` are U0101.

3. **One meaning and a position rule.** The specification states it in one sentence, in `spec/02 §6`: "`const` binds a name to a value that does not change; at the top level of a module its type MUST be written and its value MUST be a constant expression (ADR 0126 rule 3), and in a block it MAY be any expression." A top-level `const` is ADR 0126's `constant_decl` (Appendix A). A `var` at the top level is U0101, `` a module declares no `var`; a module value is a `const` ``: a module has no mutable state.

4. **Places and editable bindings** (`spec/17 §2`). An editable binding is a `var` binding or a parameter declared `edit`. A place is rooted at one. An assignment to a `const` binding, to a parameter without `edit`, to a loop variable, to a pattern binding, to a lambda parameter, to a binding captured by a lambda, or to a module constant is U2019. Rule 4 of `spec/17 §2`, the fixed type of an editable binding, holds of `var` bindings as written.

5. **Parameter conventions** (`spec/17 §3`). Unchanged. `edit` before a parameter, an argument or a receiver, and `take` before a parameter or an argument, mean what they mean in edition `0.4`. Their rules read:
   - **`edit`.** The argument of `edit` is a place rooted at a `var` binding or an `edit` parameter; one rooted elsewhere is U2019.
   - **`take`.** Any local binding can be taken: a `const`, a `var` or a parameter. After `take a`, every use of `a` is U2017 until an assignment gives it a new value, and only a `var` can be assigned, so only a `var` is revived. A binding taken in a loop body is assigned before the end of the body, which needs a `var`, or is declared inside the body.
   - **A module constant** is not a local binding (ADR 0126 rule 13): `f(take origin)` is U2020, and a program copies the value to a local first, `const p = origin`, then `f(take p)`.

6. **Bindings without a keyword.** Pattern bindings of `when` arms, loop variables of `for` and `together for`, lambda parameters and handler parameters are introduced without a keyword, as today, and are immutable: they behave as `const` bindings.

7. **`together`** (`spec/19 §1` to `§3`). Each branch of a `together` statement is a binding statement, `const` or `var`, as each was `let` or `let edit`; a `together for` stands as the value of a `const`, a `var` or an assignment. U2024's message names "binding statements": `` the body of `together` holds only `const` or `var` statements, one per branch ``.

8. **Tests** (`spec/08 §2`). A test's `do` line is an expression and binds nothing, in every edition. Where a test needs a value it can edit, it writes a function with `var`, as `spec/17 §3` rule 5 shows: `var copy = n`, `bump(edit copy)`, `copy`. When a later record lets a step of a test bind a name (issue #1591), the step is written `do const name = e`, and `do var` where it can be assigned; no edition has `do let`.

9. **Canonical form** (`spec/15`). `una fmt` writes `const x = e`, `const x: T = e`, `var x = e` and `var x: T = e`, with one space after the keyword, around `:` and around `=`, as it writes `let` today; a module constant is one logical line (ADR 0126 rule 10). In edition `0.5`, `una fmt` refuses a file with U0103, as it refuses any file with a syntax error.

10. **Messages.** Every message, help, repair, `una explain` text and standard-library description that names a binding form names the form of the package's edition: `let` and `let edit` up to edition `0.4`, `const` and `var` in edition `0.5`. In particular:
    - **U2019** on a local: `` `x` is not editable; declare it `var x`, or the parameter `edit x` ``;
    - **U2019** on the variable of a `for` loop (issue #931): `` `x` is the variable of a `for` loop, which is read-only; bind a copy in the body with `var`, as `var next_x = x` `` (and the same for `together for`);
    - **U2011, U3013 and the `edit`-method messages:** "bind its result with `const` first";
    - **U1002** is unchanged: two module constants of one name, or a constant and a function of one name, are duplicate definitions.

    The domain and meaning of every code stay as they are; no code is added.

11. **The rewrite** (`una fix`, ADR 0096, `spec/13 §5`). The U0103 of `let` in edition `0.5` carries an exact repair: `let edit x` becomes `var x`, `let x` becomes `const x`, keeping the annotation and the value, in blocks, in `together` branches, at the top level (where ADR 0126's form follows) and in any form that a test may bind (rule 8). A local named `const` in an edition `0.4` package, which edition `0.5` reads as a keyword, gets U0101 with the exact rename to `const_value`, as ADR 0096 renames a local named `test`. `una fix`:
    - applies these repairs under its usual conditions;
    - is idempotent: on a file it has rewritten, it finds nothing to rewrite;
    - changes no meaning: the rewritten program has the result of the original in edition `0.4`, since rule 2 maps each form to one with the same semantics;
    - lists, with file and line, each site it cannot rewrite exactly, such as a `let` inside a doc comment's code or in a file that does not parse, and leaves it unchanged; `una check` still reports it.

    The repairs exist only in a package of edition `0.5`: a package moves by changing `edition` in its manifest and running `una fix`.

12. **Editions and preview terms.** The change is a new edition's, not a change inside a released preview: editions `0.1` to `0.4` keep their forms, and no program of those editions changes its result. Within edition `0.5`, the old form is loud, never silently read: `let` is U0103 with its rewrite (rule 1), which is the requirement of ADR 0121 rules 4 and 5 for a form that changes. If edition `0.5` is already served as a preview when this record lands, a release that accepted `let` in edition `0.5` makes it U0103 with the repair, and its changelog lists the change under the preview heading of ADR 0121 rule 6.

13. **Standard modules.** The Una sources of the standard modules (`crates/una-check/src/std/`) are checked in their own edition. Those that edition `0.5` serves are written with `const` and `var`; those of earlier editions keep their forms, so a package of edition `0.4` sees the same standard modules as today.

14. **Tooling.** The language server's completions offer `const` and `var` in edition `0.5` and `let` before it; hover shows `const x: T` or `var x: T`; the grammars of the editors and of the website highlight the words by edition (issue #1631).

## Examples

A loop with a sum. Edition `0.4`, then edition `0.5`:

```una
pub total(xs: List[Int]) -> Int
=
  let edit sum = 0
  for x in xs
    sum = sum + x
  sum
```

```una
pub total(xs: List[Int]) -> Int
=
  var sum = 0
  for x in xs
    sum = sum + x
  sum
```

A function with an `edit` parameter, called on a local copy. The markers do not change:

```una
pub bump(edit n: Int) -> Unit
=
  n = n + 1

pub twice(start: Int) -> Int
=
  let edit copy = start
  bump(edit copy)
  bump(edit copy)
  copy
```

```una
pub bump(edit n: Int) -> Unit
=
  n = n + 1

pub twice(start: Int) -> Int
=
  var copy = start
  bump(edit copy)
  bump(edit copy)
  copy
```

A test. Its lines do not change; the function it calls uses `var`, and a step that binds a name, if issue #1591 adds one, writes `do const`:

```una
test "twice adds two"
  do twice(3)
  expect result == 5
```

A module constant (ADR 0126), and `take` of its value:

```una
module geometry

--- The point at the center of the plane.
pub const origin: Point = Point(x: 0, y: 0)

pub moved(take p: Point) -> Point = Point(x: p.x + 1, y: p.y)

pub start() -> Point
=
  const p = origin
  moved(take p)
```

`moved(take origin)` is U2020: a module constant is not a local binding (rule 5). After `moved(take p)`, `p` is taken for the rest of the body, since a `const` is never revived.

## Security

- **No ambient authority changes.** A binding still cannot hold a capability reference (U3005) or be a second-class value (`spec/05 §6`, ADR 0038 D2); the rename changes no rule of `spec/05`.
- **No module state.** Rule 3 forbids a top-level `var`: a module has no mutable value, as today.
- **No information-flow change.** Labels of a `var` binding are fixed as those of a `let edit` binding are (`spec/17 §6`); no rule of `spec/07` changes.
- **No new trusted code.** The change is in the lexer, the parser, the messages and `una fix`; the checker's rules, the verifier, the interpreter and the native backend see the same trees.

## Verification

No obligation is added or removed. A `const` binding gives the obligations of an annotated `let`, a `var` binding those of an annotated `let edit` (`spec/17 §5`), and the queries are the same, binder for binder. The formal model (`formal/`) names Core's `let`, which stays (Question 5); no theorem changes.

## Information flow

No label, rule or theorem of `spec/07` changes (Security).

## Ownership and authority

Rules 4 and 5. `edit` and `take` keep their meaning and their checks; `take` is pass by value with transfer of the binding, any local binding can be taken, and only a `var` is revived by assignment. A module constant is never taken (ADR 0126 rule 13). No authority enters or leaves through a binding.

## Costs

| Operation | Time | Memory |
| --- | --- | --- |
| Lexing `const`, `var` and the reserved `let` | O(1) per word, a lookup in the keyword table of the edition | none |
| Parsing a binding statement | O(its length), as today; one token of lookahead decides `const` or `var` | none |
| `una fix` on a package | O(size of its sources): each repair is a local edit at a U0103 or U0101 site | O(size of a file) |

**Migration.**
- **The language repository:** about 6,700 lines in about 1,900 `.una` files, and about 1,000 Markdown lines, rewritten by `una fix` and by a check of the documents' Una blocks (issue #1630). Conformance cases of editions `0.1` to `0.4` keep their forms, since those editions keep them.
- **The other repositories:** about 1,000 `.una` files with about 3,600 lines, and the Una code of pages, primers and editor grammars (issue #1631).
- **Gate 1:** the benchmark baseline stays pinned to the release line of edition `0.1`, whose forms do not change; it is not migrated.

## Diagnostics

No new code. In edition `0.5`: U0103 for `let`, with its exact rewrite (rules 1 and 11); U0101 for `const edit`, `var edit` and a top-level `var` (rules 2 and 3), and for a local named `const`, with its rename (rule 11); U2019 and U2017 with the words of rules 4, 5 and 10; U1002 unchanged. In editions `0.1` to `0.4`: the same codes as today, with the help of rule 1 at a line starting with `const` or `var`.

## Consequences

- **One word per meaning.** `const` is the immutable name, in a block and at the top level; `var` the rebindable one; `edit` and `take` the conventions of calls. A reader from JavaScript, TypeScript, Swift or Rust reads `const` and `var` as Una means them.
- **ADR 0126 needs no word of its own.** Its constants are `const` at the top level, under the position rule.
- **Every program moves once,** mechanically, when its package changes its edition to `0.5`. Packages that stay on earlier editions do not move.
- **Two sets of messages,** by edition, as the U0103 tables already are; the conformance suite covers both.
- **Readers of Rust may expect a local `const` to be compile-time.** The position rule answers that: only the top level needs a constant expression, and U2041 says so where it applies.
- **Deferred:** a lint for a `var` never assigned (Question 4).

## Alternatives considered

- **A. Keep `let` and `let edit`.** Not chosen: `let` reads as mutable to readers of JavaScript and TypeScript, and ADR 0126 would put a second meaning, or a second word, next to it.
- **B. `let` and `var`, as Swift.** The smallest migration: about 940 lines of `let edit`. Not chosen: `let` keeps its opposite meaning for JavaScript and TypeScript readers, and module constants would be `let` at the top level, which says nothing about the compile-time rule.
- **C. `const` and `let`, as JavaScript.** Not chosen: `let` would change from immutable to rebindable, so every existing program and document would be misread by anyone who knows the edition `0.4` meaning; a word must not change meaning across editions.
- **D. A contextual keyword `constant` for module values only, `let` in blocks.** Not chosen: two words for one meaning, an immutable name, which the principle of one word per meaning rules out.
- **E. `val` and `var`, as Kotlin and Scala.** Not chosen: the two words differ by one letter, which is easy to misread and to mistype, and `val` carries no meaning for most readers.

## Implementation plan

Each step ships behind edition `0.5`, with its conformance cases, in both engines, and a changelog fragment. Nothing is implemented before this record is accepted.

1. **Lexer, parser and checker** (issue #1628). The keyword tables of rule 1; the grammar of rules 2 and 3; rules 4 to 7 on the new forms; the messages of rule 10 by edition; the help of rule 1 in earlier editions; the language server's completions and hover (rule 14).
2. **`una fix`** (issue #1629). The repairs of rule 11, the list of sites it leaves, and the idempotence test.
3. **The language repository** (issue #1630). The standard modules served by edition `0.5` (rule 13), the examples that move to edition `0.5`, the guide and the specification's Una blocks for edition `0.5`, with a document check that each block of edition `0.5` uses only its words.
4. **The other repositories** (issue #1631). Una by Example, the website and its Learn pages, the editor grammars, and the benchmark repository except its pinned baseline.
5. **Module constants** (issue #1632). ADR 0126 with `const`, in edition `0.5`.

## Conformance

- **Keywords:** in edition `0.5`, `let x = 1` is U0103 with the message of rule 1 and the repair `const x = 1`; `let edit x = 1` is U0103 with the repair `var x = 1`; `const` and `var` as names are U0101; in edition `0.4`, `const x = 1` is U0101 and `var x = 1` U0103, each with the help of rule 1, and `let` programs check as today.
- **Bindings:** `const x = 1` then `x = 2` is U2019 with the message of rule 10; `var x = 1` then `x = 2` is accepted; `var n: Pos = 1` then `n = 0` gives a refuted obligation, as `let edit` does; `const edit x = 1` and a top-level `var` are U0101.
- **Conventions:** `f(edit c)` with `c` a `const` is U2019; `f(take c)` with `c` a `const` is accepted and a later use is U2017; `f(take v)` with `v` a `var`, then `v = e`, then a use, is accepted; a `const` taken in a loop body and declared outside it is U2017; `f(take origin)` of a module constant is U2020.
- **`together`:** branches written `const` and `var` are accepted; a body with another statement is U2024 with the message of rule 7.
- **Equivalence:** for every conformance package of edition `0.4` that uses `let` or `let edit`, the package rewritten by `una fix` and checked in edition `0.5` gives the same output, trace, status and obligation statuses, in both engines.
- **`una fix`:** a second run changes nothing; a `let` inside a doc comment is listed and left; a local named `const` is renamed to `const_value`.
- **Canonical form:** `una fmt` writes `var x: Int = 1` from `var x :Int=1`.

## Appendix A. Amendments and revisions

To be made, dated, when this record is accepted, with the implementation:

- **ADR 0121:** none. Its rules govern edition `0.4`; the terms of edition `0.5` come with issue #1620, and rule 12 above follows ADR 0121 rules 4 and 5.
- **ADR 0126:** revised while it is Proposed, before acceptance: its syntax becomes `[pub] const name: type = value`, its target edition `0.5`; in edition `0.5`, `let` at the top level is U0103 with the message of rule 1, and in edition `0.4`, `const` at the top level is U0101, as `let` is there today.
- **`spec/01 §5`:** the keyword tables of edition `0.5` (rule 1).
- **`spec/02 §6`, `§8`:** `let_stmt` (rule 2), the sentence of rule 3, and the context restrictions that name `let`.
- **`spec/08 §2`:** rule 8.
- **`spec/10 §5`, `§11`; `spec/12 §4`:** the binding forms by edition, where they name `let`.
- **`spec/13 §4`, `§7`:** the messages of rule 10 and edition `0.5`.
- **`spec/15`:** rule 9.
- **`spec/17 §1` to `§5`, `§9`:** rules 2, 4 and 5; the U0103 table of edition `0.5`.
- **`spec/19 §1` to `§3`:** rule 7.
