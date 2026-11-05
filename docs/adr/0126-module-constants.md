# 0126. Module constants: named values declared at the top level

- Status: Proposed
- Date: 2026-10-27 (revised 2026-11-04: ownership, authority, the costs of each option, Questions 7 and 8; revised 2026-11-05: the syntax `const` under ADR 0137, edition `0.5`, the decisions of the questions)
- Deciders:
- Resolves: issue #949
- Related:
  - ADR 0137 (`const` and `var` replace `let` and `let edit`), which makes `const` the word of every immutable binding, and with which this record is accepted;
  - ADR 0116 (long-running programs), whose amendment of 2026-10-19 made `millisecond`, `second` and `minute` built-in names of `std.time` because Una declares no module-level value;
  - ADR 0120 (the numeric foundation), whose programs want named constants: π, physical constants, tolerances and limits;
  - ADR 0104 and `spec/25` (units of measure), whose rule 3 of §3 gives a constant expression the unit its context expects;
  - ADR 0114 and `spec/29` (`F64`), whose operations are exactly specified, so that a value folded at compile time equals the one computed at run time;
  - ADR 0121 (the terms of a preview edition), whose rule 3 lets a preview edition accept a construct that widens what a program can do, by the terms issue #1620 gives edition `0.5`, and issue #1620, which numbers the editions after the release that closes them;
  - ADR 0061 (the memory budget), which counts every value a run constructs;
  - `spec/17` (the memory model), whose `edit` and `take` conventions apply to bindings and places;
  - `spec/05 §6` and ADR 0038 D2, by which capability references and effectful function values are second-class and cannot be bound, stored or returned.
- Target: edition `0.5`, with ADR 0137, numbered as issue #1620 numbers editions. Editions `0.1` to `0.4` do not change: in edition `0.4`, `const` at the top level is U0101, as `let` at the top level is there today.
- Amends:
  - **Grammar:** `spec/02 §1` (`decl` gains `constant_decl`), `§7` (a constant name is a `primary`), and, under Question 4, `§2` (a range bound may name a constant);
  - **Lexical structure:** `spec/01 §5` (no keyword of its own: `const`, a keyword of edition `0.5` under ADR 0137, at the top level; `let` at the top level is U7007 there, as everywhere in edition `0.5`, ADR 0137 rule 1);
  - **Memory model:** `spec/17 §2`, `§3` (a constant is neither an editable binding nor a local binding, Question 7);
  - **Capabilities:** `spec/05 §6` (a constant's type holds no capability and no function type, Question 8);
  - **Static semantics:** `spec/10 §8` (the typing of a constant), `§11` (the lower-case namespace holds constants; a cycle among constants), `§12`;
  - **Verification:** `spec/12 §3` (the `refinement` obligation of a constant), `§4` (a reference is its value);
  - **Dynamic semantics:** `spec/11 §2` (a reference evaluates to the value);
  - **Core IR:** `spec/09 §3` (references are substituted at elaboration; Core IR gains no declaration);
  - **Canonical form:** `spec/15 §4`, `§8` (one line per constant, blank lines as for other declarations);
  - **Diagnostics:** `spec/13 §4` (two codes), `§7` (edition gating);
  - **Standard library:** `spec/14 §3` and `spec/26 §2` (`std.time` declares its constants in Una);
  - **Accepted records:** ADR 0116, by the dated amendment of Appendix A.

> **In plain words.**
> - **What a programmer writes.** A module can name a value at the top level, with its type, and export it: `pub const pi: F64 = 3.141592653589793`, `pub const max_items: Int in 1..1000 = 500`, `pub const light: Int of Meter / Second = 299_792_458`. The name is used like any value, in bodies and in contracts.
> - **What the value may be.** Something the compiler can work out from the source alone: literals, arithmetic on them, constructors and lists of such values, and other constants. No call, no capability, no lambda. The compiler computes it once, at check time, and every use sees that exact value: the verifier proves contracts with the value itself, the interpreter and native code use it as a literal.
> - **What it is not.** It is not a `var` (a module has none, ADR 0137 rule 3), not a function (nothing runs at a use), and not a place where code runs when a module loads: a module still does nothing until a function is called.

## Questions for acceptance

Each question changes what a programmer writes or can rely on, and states its recommendation; the text below follows the recommendations. The decisions are the project's: it decided all eight on 2026-11-05, and this record stays Proposed until it is accepted together with ADR 0137 (Decision).

1. **The syntax: `const` at the top level, `let`, or a new keyword?**
   - **(a) Chosen:** `[pub] const name: type = value`, on one line. Under ADR 0137, `const` binds an immutable name in edition `0.5`, in a block and at the top level, and `let` is not a keyword there; at the top level it binds one for the module. The type is required, so that the unit, the refinement and the label are stated where the value is declared, and so that the declaration reads as a signature does. No keyword is added beyond those of ADR 0137.
   - **(b)** `[pub] let name: type = value`, with `let` as the binding word of editions `0.1` to `0.4`.
   - **(c)** A new contextual keyword, such as `constant pi: F64 = ...`, as `measure` is a contextual keyword for units.
   - **Cost of (a):** a block-local `const` and a module `const` share a word, as they share a meaning, an immutable name; the type annotation, required at the top level and optional in a block, and the constant expression of rule 3, required at the top level only, are the position rule of ADR 0137 rule 3.
   - **Cost of (b):** `let` would be the module word just as edition `0.5` replaces it with `const`; the two words would coexist.
   - **Cost of (c):** one more keyword, and two words for one meaning, which Una does not have.
   - **Complexity:** the same for all: parsing a declaration is O(its length) and adds no lookahead, since a top-level `const` is a declaration in every position where it can appear. (c) also adds one entry to the contextual-keyword check of each top-level line, O(1).
   - **Decided:** the project, 2026-11-05: (a), `const`, instead of `let` or a new contextual keyword; ADR 0137 states the full rename of `let` and `let edit` to `const` and `var`.

   ```una
   module geometry

   --- The ratio of a circle's circumference to its diameter.
   pub const pi: F64 = 3.141592653589793

   pub area(radius: F64) -> F64 = pi * radius * radius
   ```

2. **Must the value be a compile-time constant?**
   - **Recommended: yes,** a constant expression (rule 3). The compiler computes it at check time, once, with the same results as a run would give, and the value is exact everywhere: in queries, in traces, in native code.
   - **The alternative** is a module value computed when the program starts, from any expression without effects. It needs an initialization order, a story for a value that fails to compute, and an evaluation the verifier cannot see through.
   - **Cost to the programmer:** a value that needs a call, as `sqrt(2.0)` or a parsed `Decimal`, is written as a function instead, as today.
   - **Complexity:** with the recommendation, checking costs O(total size of the values) once, a scalar reference costs O(1) at run time, and program start costs nothing. With the alternative, start costs the evaluation of every module value, O(sum of their costs), which is unbounded for an arbitrary effect-free expression (a recursive function), plus an O(modules + edges) initialization order; each reference costs O(1) at run time, but the verifier must treat it as an uninterpreted symbol, so each obligation that uses it needs a hypothesis it cannot have, or a proof of the initializer's postcondition.
   - **Decided:** the project, 2026-11-05: yes, compile-time constant expressions (rule 3). Also decided: the size caps of rule 5 (4,300 decimal digits, 1,000,000 elements, fields and characters), and substitution at each use for large values too (rule 8), with Alternative D kept as the fallback if measurement shows the need.

   ```una
   pub const origin: Point = Point(x: 0, y: 0)      -- yes: a constructor of constants
   pub const root_two: F64 = sqrt(2.0)               -- no, U2041: a call; write a function
   ```

3. **Labels on a constant's type.**
   - **Recommended:** the declared type may carry a label, as any type may, and the value, a literal, flows into it (`spec/07 §7`); a constant cannot hold internal data, since its value is in the source, so a label above `public` only makes every use carry that label. Nothing is forbidden, and nothing new is added.
   - **Cost to the programmer:** none.
   - **Decided:** the project, 2026-11-05: as recommended.

4. **May a constant appear in types and contracts?**
   - **Recommended: yes.** In `requires`, `ensures`, `where` refinements and `given` lines, a constant name is an ordinary expression. In a range of a type (`Int in 1..max_items`), a bound may name a constant of an integer type, since after substitution it is a literal; `spec/02 §2`'s `range` gains that form. The verifier sees the literal in every case.
   - **Cost to the programmer:** none. The cost to the implementation is a resolution of names inside types, which refinements already need for `where`.
   - **Complexity:** resolving a bound is one O(1) lookup in the module's name table; the substituted bound is a literal, so the query is what a literal range gives. The alternative, constants only in bodies, saves that lookup and nothing else.
   - **Decided:** the project, 2026-11-05: yes, in types and contracts.

   ```una
   pub const max_items: Int = 1000

   pub kind Batch
     items: List[Item] where it.len <= max_items
   ```

5. **Which types may a constant have?**
   - **Recommended:** every type whose values a constant expression can write (rule 3): `Int` and the fixed-width integers, `F64`, `Decimal[s]`, `Text`, `Bool`, `Unit`, measured and refined forms of these, and record, sum and list types whose contents are such values. Not function types, not capability types, not `Map` or the collections of `spec/28`, which have no literal.
   - **Cost to the programmer:** a constant map is written as a list of pairs, or built by a function.
   - **Complexity:** a scalar constant costs O(1) per reference everywhere; a record or list constant costs O(size) per reference in Core and in queries (rule 8, with the encoder's sharing) and O(size) per construction at run time, unless the engine shares it. Admitting `Map` would need a check-time hash or tree construction, O(n log n) or O(n) expected, and a canonical order for its encoding; it is left out for that reason, not for a soundness one.
   - **Decided:** the project, 2026-11-05: as recommended.

6. **`std.time` declares its constants in Una.**
   - **Recommended: yes.** `millisecond`, `second` and `minute` become `pub const` declarations of `crates/una-check/src/std/time.una` in the source that edition `0.5` serves, and the built-in special case (`STD_TIME_CONSTANTS`, `Entity::Constant`) is not used there; it stays for edition `0.4`, which has no module constants (rule 12). Nothing changes for a program: the names, types and values stay.
   - **Cost to the programmer:** none.
   - **Complexity:** the lookup of a `std.time` name moves from a built-in table to the module's name table, both O(1); one special case of the checker goes away.
   - **Decided:** the project, 2026-11-05: yes, `std.time` declares its constants in Una.

7. **Ownership: may a constant be passed with `edit` or `take`?**
   - **Recommended: no marker applies to a constant.** A constant is not a binding of the function that uses it: it is neither an editable binding (`spec/17 §2` rule 1) nor a local binding (`spec/17 §3` rule 2). So `max_items = 2` and `f(edit max_items)` are U2019, as for a local `const`, and `f(take origin)` is U2020, as for a field. A plain use of a constant is a value, and a function that takes a parameter by `take` receives it after `const p = origin`, since any local binding, a `const` included, can be taken (`spec/17 §3` rule 2, ADR 0137 rule 5).
   - **The alternative** lets `take origin` mean "a fresh copy of the value". It is sound, since the value is substituted (rule 8), but it makes `take` mean two things: consume a binding, or copy a constant. It also hides an O(size) construction behind a marker that, for a binding, promises a move.
   - **Cost to the programmer:** one `const` line where a constant is handed to a `take` parameter.
   - **Complexity:** the recommendation needs one new check, O(1), at the root of each `edit` argument, each `take` argument and each assignment: whether the root names a constant. The checker does not refuse these today: for a root that is not a local, it gives no error at an `edit` argument or an assignment and accepts a `take` (`crates/una-check/src/types/memory.rs`, `editable_place` and `take`), since no non-local value name exists yet. Under the alternative, each `take` of a record or list constant constructs it, O(size), as the recommendation's `const` line does, so the run-time cost is the same and only visibility differs.
   - **Decided:** the project, 2026-11-05: as recommended: no marker applies to a constant; an assignment and an `edit` argument rooted at one are U2019, a `take` of one is U2020, and a program copies it to a local `const` before passing it to a `take` parameter. ADR 0137 rule 5 explains `take` under `const` and `var`.

   ```una
   pub kind Point
     x: Int
     y: Int

   pub const origin: Point = Point(x: 0, y: 0)

   pub moved(take p: Point) -> Point = Point(x: p.x + 1, y: p.y)

   pub start() -> Point
   =
     const p = origin             -- a local binding, which can be taken
     moved(take p)                -- not moved(take origin): U2020
   ```

8. **Capabilities and authority: what may a constant's type hold?**
   - **Recommended: no capability and no function type,** at any depth, so that a constant can never carry authority. A capability reference is not a value (`spec/05 §6`): naming one as a constant's value is U3005, as a local binding of one is. A declared type that is, or contains, a function type is U2041, "a constant cannot hold a function": rule 3 admits no lambda, so no value could flow into it, and ruling the type out says so at the declaration rather than at the value. Importing a constant therefore grants nothing: authority still enters a program only through `uses`, handlers and the grants of `una run` (`spec/05`, `spec/13 §5`).
   - **The alternative** admits pure function values, `fn` types without `uses` and `fails`, named at the top level. It needs lambdas in rule 3, a check-time closure representation, and a reference that is a call target rather than a literal; the verifier would see an application, not a value. A named function already does this job.
   - **Cost to the programmer:** none beyond writing a function as a function.
   - **Complexity:** checking the declared type is one traversal, O(size of the type). The alternative adds O(size of the closure) at each reference in Core and a call at run time.
   - **Decided:** the project, 2026-11-05: as recommended, no capability and no function type.

## Glossary

- **Constant.** A module-level `const`: a name bound to a value the compiler computes at check time.
- **Constant expression.** An expression the compiler can evaluate from the source alone (rule 3).
- **Substitution.** Replacing a reference to a constant by its value's expression at elaboration, so that Core IR holds no reference.
- **Fit.** Whether a value lies in a fixed-width integer type's range (`spec/18 §2`).

## Context

- **No module-level value.** `spec/02 §1`'s `decl` has kinds, predicates, functions, capabilities, handlers, policies, requirements, examples and, from edition `0.3`, measures. A library cannot export a named value. `std.time`'s `millisecond`, `second` and `minute` are built-in names for that reason: the checker resolves them by a table (`crates/una-check/src/names/prelude.rs`, `STD_TIME_CONSTANTS`) and types them as `Int of Millisecond` through a special entity (`crates/una-check/src/types/env.rs`, `Entity::Constant`), which the standard module's own source cannot express (`crates/una-check/src/std/time.una`).
- **What wants constants.** The numeric foundation (ADR 0120): π, e, physical constants, tolerances, iteration limits, the sizes of matrices; servers (ADR 0123): limits, time-outs, header names; every program: a `max_items`, a `default_port`, a `version` text.
- **What exists for literals.** Literals are `public` (`spec/07 §7` rule 3); a constant expression of integer literals takes the unit its context expects (`spec/25 §3` rule 3) and the fixed-width type its context expects (`spec/18 §2`); `F64` arithmetic is specified operation by operation (`spec/29`), with round-to-nearest-even and no fast-math, so folding an expression at compile time gives the run's value; text literals are distinct constants in queries (`spec/12 §4`).
- **The verifier's names.** Binders of a query are the program's parameters, local bindings and pattern variables (`spec/12 §4`), quantifier-free; a constant that entered a query as an uninterpreted symbol would need an equation per query; one that enters as its literal needs nothing.
- **The preview edition.** ADR 0121 rule 3 lets a preview edition accept constructs that widen what a program can do, by the terms issue #1620 gives edition `0.5`; rule 4 keeps every accepted program's meaning or rejects it with a diagnostic. The construct lands in edition `0.5` with ADR 0137, so that `let` and `const` never name a binding in one edition.
- **The way Una is written.** Una's syntax is its own (`spec/01`, `spec/02`): declarations read as signatures, values are immutable, and a word means one thing. From edition `0.5`, `const` is the word for binding a value that does not change (ADR 0137).

## Decision

This record and ADR 0137 are accepted together: the syntax below depends on ADR 0137, which is Proposed, and the project accepts both in one step.

In one paragraph: a module may declare a constant, `[pub] const name: type = value`, whose value is a constant expression that the compiler evaluates at check time by the language's own semantics; the value must flow into the declared type, with its refinement an obligation like that of an annotated `const` or `var` binding; a reference to the constant is substituted by its value at elaboration, so Core IR, the verifier, the interpreter and native code see a literal; constants live in the lower-case namespace, are exported with `pub` and imported by name, may refer to one another without a cycle, and may appear in contracts, refinements and, as bounds, in ranges; the construct exists from edition `0.5`, and `std.time` uses it for its three constants.

1. **Syntax.** In `spec/02 §1`, `decl` gains:

   ```ebnf
   constant_decl = [ "pub" ] "const" lower_ident ":" type "=" line_expr NEWLINE ;
   ```

   The type is required. The declaration is one logical line, continued inside brackets as any line is (ADR 0125). A doc comment (`---`) may lead it, as for any declaration. In `spec/02 §7`, a `primary` that is a `lower_ident` may resolve to a constant (rule 4). In editions `0.1` to `0.4`, `let` at the top level is U0101 at the keyword, as today, and so is `const`, an identifier there, with the help that ADR 0137 rule 1 gives. In edition `0.5`, `let` at the top level is U7007 with the message of ADR 0137 rule 1 and the exact rewrite to `const`.

2. **Names.** A constant is in the module's lower-case namespace (`spec/10 §11` rule 1), with functions, predicates, handlers and policies; two declarations of one name are U1002 (`spec/10 §11` rule 5); the conventions of `spec/10 §11` rule 9 apply. `pub` exports it; an import brings it in by name (`spec/10 §11` rule 6). A local of the same name shadows it, as a local shadows any non-local name (`spec/10 §11` rule 2). A constant is identified by its module path and its name (`spec/10 §11` rule 8).

3. **Constant expressions.** The value is one of:
   - a literal: an integer, a float or a decimal (edition `0.3` and later, `spec/29`), a text without interpolation, `true`, `false`, `unit`;
   - `-` applied to a constant expression of a numeric type;
   - `+`, `-`, `*`, `/`, `mod` on two constant expressions of one integer type, with `/` and `mod` by a divisor that is not zero (U2041 otherwise, at check time, never an obligation); `+`, `-`, `*`, `/` on two of type `F64`; and `+`, `-` on two decimals of one scale (`spec/29 §16`);
   - `and`, `or`, `not`, `==`, `!=`, `<`, `<=`, `>`, `>=` on constant expressions, where the operators are defined (`spec/10 §3`);
   - a constructor of a record or a sum kind whose every field value is a constant expression, with the labels of the arguments written as anywhere;
   - a list literal whose every element is a constant expression;
   - parentheses around a constant expression;
   - the name of a constant of this module or an imported one.

   Nothing else: not a call, a method, a lambda, a capability reference (U3005, rule 14), `attempt`, `fail`, `checked`, `if`, `when`, `result`, `it`, `error`, an interpolated text or a field access. A value outside this set is U2041 at the expression, naming the construct. The set is closed: a later record widens it by a dated amendment.

4. **Cycles.** A constant may name constants declared before or after it; a constant that depends on itself, directly or through others, is U1011 at the first declaration of the cycle in path order, once per cycle, as U1003 reports an import cycle.

5. **Evaluation.** The compiler evaluates the value at check time, in dependency order, by the dynamic semantics of `spec/11` for those operators: integers with arbitrary precision, `F64` as `spec/29` defines each operation, texts as written. The evaluation is deterministic and total: a division by zero is U2041 (rule 3), and no other failure exists in the set of rule 3.
   - **Caps.** A constant integer with more than 4,300 decimal digits, and a constant value holding more than 1,000,000 elements, fields and characters together, are U2041, "the constant is too large": a chain of constants that square one another, or nest one list in another, must not make `una check` compute without bound. The caps match the digit cap of `parse_decimal` (ADR 0120) and the order of a source file's own size.

6. **Typing** (`spec/10 §8`, T-Const). The value's type must flow into the declared type, as at an annotated local `const`: a mismatch of base types is U2001, of units U2028, of labels U4001. An integer literal or expression takes the declared fixed-width type and must fit it (`spec/18 §2`); an expression whose value does not fit is U2041, "the constant's value `v` does not fit `I32`", at check time, never an `overflow` obligation. A constant expression of integers takes the declared unit (`spec/25 §3` rule 3); one that mixes two constants of different units is U2028, and `*` and `/` combine units as `spec/25 §3` rule 2 says.

7. **Refinements** (`spec/12 §3`). A declared type with a refinement gives one `refinement` obligation at the value, as an annotated local `const` does, whose goal is the refinement over the value and which has no hypotheses: `max_items: Int in 1..1000 = 500` obliges `1 <= 500 and 500 <= 1000`. Tier 0 leaves it unknown as it leaves every obligation; tier 1 proves it, and a refuted one is U5001 at the value with no counterexample note, since the goal is closed and has no binder to show (`spec/12 §7`, R-021). A nested refinement, as of a record field or a list element, gives its obligation at the position, as `spec/12 §3` states for nested positions.

8. **Substitution.** At elaboration (`spec/09 §3`), every reference to a constant is replaced by its value's Core expression: a literal, or a constructor or list of such. Core IR gains no declaration, and `una core` shows the value where the source wrote the name. So:
   - **the verifier** encodes a reference as the literal (`spec/12 §4`): an integer or Boolean as itself, a text as that literal's constant, a float by `spec/29 §8`, a constructor by its tag and field equations, a list by its elements and length. A contract that names a constant is proven with its value, and a counterexample never binds a constant: it is not a binder;
   - **the interpreter and native code** evaluate the literal expression at each use. A scalar costs what a literal costs. A record or list constant is constructed at each use, and each construction counts against the memory budget (ADR 0061), as a literal written at that place would; an engine MAY construct it once per run and share it, which no program can observe, since values are immutable and Una has no identity on values.

9. **In types and contracts** (Question 4). A constant name is an expression in `requires`, `ensures`, `where`, `given` lines and examples. In `spec/02 §2`, `range` gains `signed_int | lower_ident` for each bound; a bound that names a constant must resolve to a constant of an integer type (U1001 if unresolved, U2002 if it is not a constant or not an integer), and is substituted before the range is read; a range with both bounds constants whose values are out of order is the error a literal range gives today. A constant may not be used in its own type.

10. **Editions and canonical form.** The construct exists in edition `0.5` and later (`spec/13 §7`). The canonical form writes a constant as one logical line, `pub const name: type = value`, with one space around `:` and `=`, the value as `spec/15` writes expressions, and the blank-line rules of `spec/15 §8` between declarations; declarations keep their order, as today.

11. **Tooling.** `una contract` does not list constants; `una core` shows substituted values; the language server (ADR 0063) shows a constant's type and value on hover, and goes to its declaration; `una fmt` formats it; the `una std` listing of `docs/std/listing.json` lists the constants of standard modules with their types and values.

12. **`std.time`** (Question 6). `crates/una-check/src/std/time.una` declares, in the source that edition `0.5` serves, `pub const millisecond: Int of Millisecond = 1`, `pub const second: Int of Millisecond = 1000` and `pub const minute: Int of Millisecond = 60_000`; `STD_TIME_CONSTANTS` and `Entity::Constant` serve edition `0.4` only, and are removed when no served edition needs them. The names, types and values that `spec/26 §2` states are unchanged.

13. **Ownership** (Question 7, `spec/17`). A constant is neither an editable binding nor a local binding. An assignment to a constant, and an `edit` argument rooted at one, are U2019; a `take` argument that is a constant is U2020. A use of a constant is a value: a backend MAY build it in place, share it or move it into a call (`spec/17 §7` rule 3), since none of these is observable. The checker refuses these with one O(1) test of the root's name at each `edit` argument, `take` argument and assignment; today it gives no error for a root that is not a local, since no non-local value name exists before this record.

14. **Authority** (Question 8, `spec/05`). A constant's declared type contains no capability type and no function type, at any depth (U2041 at the type); a capability reference as its value is U3005. A constant's value holds no authority, and the top level of a module still performs nothing, grants nothing and installs no handler.

## Security

- **No code runs at load.** A module declares values; it still does nothing until a function is called. There is no initialization order, no module initializer and no effect at the top level: rule 3 admits no call and no capability.
- **The evaluator is small, and the same as the run's.** The check-time evaluation of rule 5 covers literals, the operators of rule 3, constructors and lists, by the semantics every engine implements. A divergence between the folded value and the run's value would be a soundness bug of the verifier, which proves with the folded value: the conformance cases compare `una check`'s substituted Core, the interpreter's and the native backend's results on every operator of rule 3, in both engines (`spec/16 §5`).
- **Bounded compilation.** Rule 5's caps make the cost of checking a module's constants linear in their size: a package, which under ADR 0122 may be someone else's, cannot make `una check` compute a number with 2^40 digits or a list of 2^30 elements.
- **Information flow.** A constant's value is in the source: it is `public` by construction, and a label on its type only raises the uses (Question 3). No release or declassification is involved, and no rule of `spec/07` changes.
- **Native code.** Substitution means the native backend compiles literals it already compiles; no new runtime entry point and no new `unsafe` code.

## Verification

- **Obligations.** One `refinement` obligation per refined position of a constant's type (rule 7), with a closed goal. Nothing else is new.
- **Queries.** A reference is a literal (rule 8); the query's size grows by the literal's size at each reference. For a record or list constant referenced many times in one obligation, the encoder MAY bind the value once as a Core `let` temporary before the uses, which is what a source `const` of the same value gives, and which keeps the query linear in the number of references.
- **Determinism** (`spec/12 §10`): the folded value is a function of the source, so queries are.
- **The formal model** (`formal/`) does not model declarations by name; the substitution of a closed value for a name is the standard substitution lemma, and no theorem changes.

## Information flow

No label, rule or theorem of `spec/07` changes. Literals are `public`; a constant is a literal by rule 8, and its declared label applies to each use by the flow rules that apply to any annotated binding.

## Ownership and authority

- **Ownership** (rule 13). A constant is a value, not a binding of any function: no `edit`, no `take`, no assignment. No new aliasing exists, since every use is a fresh value or an unobservable share of an immutable one; the affine rule of `take` (`spec/17 §3` rule 3) is unchanged, because a constant is never taken.
- **No ambient authority** (rule 14). A constant cannot hold a capability or a function value, so a module that exports constants exports no authority, and an import of a constant from a package (ADR 0122) cannot widen what a program may do. The rules of second-class values (ADR 0038 D2) are unchanged.

## Costs

| Operation | Time | Memory |
| --- | --- | --- |
| Check-time evaluation of all constants | O(total size of their values), each constant once, in dependency order, under rule 5's caps | O(total size of the values) |
| Cycle detection | O(constants + references), one traversal | O(constants) |
| A reference, at check | O(1) lookup | O(1) |
| The ownership check of rule 13 | O(1) per `edit` argument, `take` argument and assignment | O(1) |
| A reference, in Core | O(size of the value) written at the reference; a scalar is O(1) | the same |
| A reference, at run time | a scalar: that of a literal, O(1); a record or list: one construction, O(size), unless the engine shares one | the construction counts against the budget |
| A reference, in a query | O(size of the value), with the encoder's Core `let` sharing for repeated uses | the same |

The whole addition is linear in the source and in the values. **Scaling tests:** modules of 10, 100, 1,000 and 10,000 constants, each referencing the previous one, check in time growing at most linearly (each doubling at most ×2.3, medians of 9 runs); a constant list of 10, 100, 1,000 and 10,000 elements referenced 100 times in one function checks, elaborates and runs in time linear in the product, in both engines; the cap of rule 5 refuses a chain of 40 squarings at the 4,300-digit cap in time linear in the chain.

### The costs of the options

`n` is the number of constants, `v` the total size of their values, `r` the number of references, `s` the size of one referenced value.

| Option | Check time | Run time | Query size | Memory |
| --- | --- | --- | --- | --- |
| **Recommended:** check-time constant, substituted (rules 3, 5, 8) | O(v + n + r) | O(1) per scalar reference, O(s) per record or list construction, or O(1) if shared | O(s) per reference, O(s + r) with `let` sharing | O(v) at check |
| Module values computed at start (Alternative C) | O(n + edges) for the order; the values are opaque | O(sum of the initializers' costs) at start, unbounded; O(1) per reference | O(1) per reference, but no fact about the value | O(v) for the whole run |
| Core IR declaration with facts in queries (Alternative D) | O(v + n + r) | O(1) per reference, the value built once per run | O(s) once per query that names it, plus O(1) per reference | O(v) per run |
| Functions without parameters (Alternative A) | O(r) calls to check | a call per reference, O(s) to build its result | an application per reference, opaque unless inlined | O(s) per call |
| Scalars only (Alternative E) | O(v + n + r) | O(1) per reference | O(1) per reference | O(n) |

The recommendation has the best average case where it matters: O(1) per scalar reference at run time with no start-up cost, and closed queries the solver discharges with literals. Alternative D is better in two cases: a large record or list named many times in one query, where rule 8's `let` sharing closes most of the gap; and the same constant used many times at run time, where D builds the value once per run and substitution builds it at each use, O(s) each, unless the engine shares it as rule 8 permits. D remains the path if measurement shows that either cost matters.

## Diagnostics

Two new codes, which join `spec/13 §4` with the implementation. U1010 and U2040 are the highest in their domains on `main`; U1011 and U2041 are free on `main` and in every open pull request at the time of writing.

| Code | Domain | Meaning |
| --- | --- | --- |
| U1011 | names | A constant depends on itself, directly or through other constants |
| U2041 | types | The value of a constant is not a constant expression (a call, a lambda, an interpolation, a control construct), divides by zero, does not fit its fixed-width type, or exceeds the size caps; or its declared type holds a capability type or a function type, at any depth |

Existing codes cover the rest: U2019 for an assignment or `edit` rooted at a constant, U2020 for `take` of a constant (rule 13); U3005 for a capability reference as a constant's value (rule 14); U0101 for `let` and for `const` at the top level in editions `0.1` to `0.4`, and for a missing type annotation; U7007 for `let` at the top level in edition `0.5`, with the rewrite to `const` (ADR 0137 rule 1); U1001 and U1002 for names; U2001, U2002, U2028 and U4001 for a value that does not flow into the declared type; U5001 for a refuted refinement (rule 7); U1005 for the conventions.

## Consequences

- **Libraries export values.** A package can name π, limits, defaults and units of its domain once, and its users and its contracts read them by name.
- **`std.time` is written in Una** for its constants too, in edition `0.5`; the special case of the checker serves edition `0.4` only, and leaves when no served edition needs it.
- **One new ownership check** (rule 13): the roots of `edit` arguments, `take` arguments and assignments are tested against constants, O(1) each, since today's checker accepts a non-local root silently.
- **No run-time cost for scalars;** a record or list constant costs a construction per use unless the engine shares it, which a later record may require if measurement shows the need.
- **Core IR is unchanged,** and so are the Core export (`una.core/1`), the report and the trace: a reference leaves no mark. A program's trace is the same whether a value was written as a constant or inline.
- **A closed set of constant expressions.** Widening it (`if` over constants, a call of a pure function, decimal multiplication) is a dated amendment, not a quiet change.
- **Edition `0.5` gains the construct,** with ADR 0137; no program of editions `0.1` to `0.4` changes meaning.

## Alternatives considered

- **A. Functions without parameters,** `pub pi() -> F64 = 3.14...`, used as `pi()`. Not chosen: a call where a value is meant; the verifier would see an uninterpreted application unless the function were inlined, and the parentheses say "something runs".
- **B. A new keyword** (`constant`, `fixed`, `value`), or `let` at the top level. Not chosen (Question 1): from edition `0.5`, `const` means "bind this value" (ADR 0137), a new keyword would be a second word for that meaning, and `let` is not a keyword of edition `0.5`; the required annotation and rule 3 tell a module `const` from a block one.
- **C. Module values computed at start,** from any effect-free expression. Not chosen (Question 2): an initialization order, failures at start, and values the verifier cannot see through.
- **D. A Core IR declaration** (`Decl::Const`) with references by name, and a fact `name = value` in queries. Not chosen for now: every engine, the encoder and the Core export would gain a form, for a benefit, sharing one construction of a record or list, that rule 8 lets an engine have anyway. If measurement shows that substitution bloats Core or queries, a later record moves to it.
- **E. Constants only of scalar types.** Not chosen (Question 5): an `origin: Point` or a list of allowed values is what programs name most; the caps of rule 5 keep them bounded.
- **F. Allowing text interpolation of constants.** Not chosen: it is computed text, which `spec/07 §7` rule 3 treats with care in patterns (S-28); a constant text is written out.
- **G. Keeping `std.time`'s built-ins in edition `0.5`.** Not chosen (Question 6): the special case exists only because the language lacked the construct; edition `0.4`, which lacks it, keeps them (rule 12).

## Implementation plan

Each step ships behind edition `0.5`, after ADR 0137's keywords (issue #1628), with its conformance cases, in both engines, and with a changelog fragment.

1. **Syntax, names and form.** The grammar (`spec/02 §1`), U0101 in earlier editions, the lower-case namespace with U1002 and U1005, the cycle check U1011, the canonical form and `una fmt`, the language server's hover and definition.
2. **Evaluation and typing.** The constant evaluator over rule 3's set with the caps of rule 5 and U2041; the flow into the declared type with units, labels and fixed-width fit; the `refinement` obligation of rule 7.
3. **Substitution and the engines.** Elaboration substitutes values; `una core` shows them; the differential cases of `spec/16 §5` cover every operator and every type of rule 3 in the interpreter and the native backend; the encoder's `let` sharing for repeated references.
4. **Types and contracts** (Question 4): constants in `requires`, `ensures`, `where`, `given`, and as range bounds.
5. **`std.time` and the documents.** `time.una` declares its constants; the built-in table and `Entity::Constant` are removed; `spec/14 §3`, `spec/26 §2`, the guide's chapter on modules, Una by Example's chapter on numerics and the `una std` listing show the construct.

## Conformance

- **Syntax and names:** a `pub const` with a type parses and resolves; one without a type is U0101 at `=`; `let` and `const` at the top level in editions `0.3` and `0.4` are U0101, and `let` at the top level in edition `0.5` is U7007 with the rewrite to `const`; two constants of one name, and a constant named as a function of the module, are U1002; a `pub const` imported by name resolves, and one without `pub` imported is U1004; a local shadows a constant; a constant named `max__items` is U1005.
- **Constant expressions:** each form of rule 3 is accepted; a call, a lambda, an interpolated text, an `if`, a `when`, a field access and `result` are each U2041 naming the construct; `1 / 0` and `1 mod 0` are U2041; a cycle of two constants is U1011 once; a constant of 4,301 digits, and a list constant that nests another past 1,000,000 elements, are U2041.
- **Typing:** `const n: I32 = 3_000_000_000` is U2041; `const d: Int of Meter = 5 * 2` is accepted and `const d: Int of Meter = other_seconds` is U2028; `const s: secret Int = 1` is accepted and `print(s)` is U4003; `const m: Int in 1..1000 = 500` has a proven `refinement` obligation, `= 0` a refuted one with U5001 at the value and no counterexample note; a record constant whose field is refined gives the obligation at the field.
- **Substitution and engines:** `una core` shows the value where the source wrote the name; for every operator and type of rule 3, a program that computes with a constant and one that writes the literal give the same output, trace and status in both engines, and the same obligation statuses; a list constant used in a loop of 1,000 iterations under a memory budget behaves as the literal would.
- **Ownership and authority:** `max_items = 2` and `f(edit max_items)` are U2019; `f(take origin)` is U2020, and `const p = origin` then `f(take p)` is accepted; a constant whose value names a capability is U3005; `pub const g: fn(Int) -> Int = ...` is U2041 at the type, as is a record field of a function type.
- **Types and contracts:** `requires n <= max_items` is proven and refuted by callers as with the literal; `Int in 1..max_items` as a parameter type; a range bound that names a function is U2002 and one that names a `Text` constant is U2002.
- **`std.time`:** `import std.time (second)` and `2 * second` keep their type and value; the `una std` listing lists the three constants with their types and values.
- **Canonical form:** `una fmt` writes `pub const x: Int = 1` from `pub const x :Int=1`, and keeps declaration order.

## Appendix A. Amendments to records

To be appended, dated, when this record is accepted:

**ADR 0116, Amendment.** The amendment of 2026-10-19 made `millisecond`, `second` and `minute` built-in names of `std.time`. Under ADR 0126 they are declared in `std.time` itself, as `pub const` constants of type `Int of Millisecond`, in edition `0.5`, with the same values.
