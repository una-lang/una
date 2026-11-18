# 0096. Exact rewrites for foreign syntax, and checking past a broken declaration

- Status: Accepted
- Date: 2026-10-12
- Deciders: Una project (accepted on 2026-10-12)
- Resolves: #576 (exact rewrites, applied by `una fix`) and #575 (every independent error in one check). Roadmap #578, phase "repairs and fixes".
- Target: every edition. No source that is rejected today is accepted, and editions `0.1` and `0.2` accept the same programs. The Gate 1 pin (v0.1.9) does not move.
- Amends: `spec/13 §1`, `§3`, `§5` (`una check`, `una fix`), `spec/15` (opening paragraph and `§10`), `compiler/architecture.md §5`

## Context

**The evidence** is a measurement of the Una runs of una-bench `full-b` and `full-c` (N-197). It covers 84 runs and 1,101 check outputs with a diagnostic. Each run's sources were rebuilt at every check from its transcript, and they match the source line printed by 98% of the diagnostics.

1. **Syntax is most of the loop.** 72% of the check outputs report only syntax errors (U0101, U0001). Those errors come in 1,733 occurrences at 1,242 distinct sites, and twelve shapes cover 89% of them. Each shape is a habit from another language:

   | Shape | Occurrences | Runs |
   | --- | --- | --- |
   | an `if … then … else` expression wrapped across lines | 562 | 71 of 84 |
   | `=` at the end of a function header, or indented under its clauses | 282 | 29 |
   | a name that starts with `_` | 116 | 48 |
   | a handling clause on the same line as its expression | 102 | 22 |
   | a lambda whose body spans lines | 78 | 28 |
   | a reserved word as a name (`result`, `error`, `unit`, `it`) | 75 | 43 |
   | a `when` with its arms on one line | 66 | 26 |
   | a guard `if c then fail X` without `else` | 59 | 31 |
   | `let` inside an expression | 58 | 20 |
   | a list pattern (`is []`, `h :: t`) | 51 | 30 |
   | a tuple | 46 | 25 |
   | a `when` inside an expression | 40 | 19 |

   - Only 152 of the 1,733 carry a repair, all of them text without an edit. The first two shapes, 49% of the occurrences, carry none.
   - `una fmt` refuses a file with a syntax error (`spec/15`), so it cannot help. The agents ran it 31 times, and 12 of those stopped at U0101.

2. **The rest of what the agents fixed by hand already has an edit.**
   - Of 383 U2004 (a missing argument label), 365 carry the exact edit. The other 17 were cut from the output by the agent's own `| head`.
   - Of 285 U7001 (canonical form), 281 carry the exact edit.
   - `una fix` (ADR 0084) applies both. The agents ran v0.1.9, which predates it.

3. **The gate between phases costs checks.**
   - A file with a syntax error is neither resolved, typed nor checked for canonical form (`una-cli` `analyze`, `una-check` names). Obligations run only when nothing else is reported (`spec/12`).
   - In 80 of the 84 runs, errors appeared only after a file's syntax error was fixed, on lines that were already there while the file was gated: 937 such diagnostics. 53% of them are U2004 or U7001.
   - A loop that fixes everything reported each time needs a median of 3 checks (`full-b`) and 4 (`full-c`) with today's gate, and 2 and 3 with syntax and the later phases reported in one check. The observed medians are 8 and 16 checks with diagnostics.
   - So the gate costs one or two checks per run. Repairs that bring the same errors back cost the rest, and those are mostly the syntax shapes of item 1.

**What the toolchain already has.**
- The parser reports at most one U0101 per declaration and resumes at the next line with zero indentation that starts a declaration (`compiler/architecture.md §5`). The declarations that parsed are in the tree, and the broken one is dropped.
- A syntax error can carry repairs with edits (`SyntaxRepair`).
- `una fix` applies, in rounds, the first repair of each diagnostic when its edit is mechanical, its `escape_cost` is 0 and it widens nothing (ADR 0084). It then formats every module that parses.

**What must not change.**
- Charter P4: one canonical form, and non-canonical source is an error. A foreign shape is never accepted as a second syntax.
- A package with any error never builds, runs or exports a complete Core (`spec/13 §5`, `§8`).

## Decision

### A. Exact rewrites (#576)

1. **The shapes stay errors.**
   - Each shape below remains U0101, or U0001 for `_`, with its message as today. ADR 0095 rule 4 keeps messages to the problem.
   - The fix is the diagnostic's first repair, and no source form is added to the grammar.
2. **Exact.** A repair edit is exact when it meets all of these:
   - (a) It is computed from the tokens of the declaration it is reported in, and it changes nothing outside that declaration.
   - (b) After it is applied, parsing the file reports no syntax error whose span meets the edited range.
   - (c) The rewritten construct is the reading the table gives for the shape: the same conditions, branches, arms, expressions and names, token for token, in Una's form.
   - (d) It is deterministic.
   - An exact edit has `escape_cost` 0, widens nothing, and is a `una fix` candidate under ADR 0084 unchanged.
3. **The table.** Where the condition holds, the repair carries the exact edit. Otherwise it carries the same description with `edit: null`, naming Una's form.

   | Shape | Una's form, and the edit | Condition for the edit |
   | --- | --- | --- |
   | `if c then` at the end of a line, then an indented block, then `else` | delete `then`: the `if_block` of `spec/02 §6` | the `if` is the final of a block, the value of a `let`, or an arm's body |
   | `if c then x else` at the end of a line, the chain continued on the next lines | an `if_block` chain: `if c` / `x` / `else if d` / … / `else` / `z`, each branch indented | same |
   | `then` or `else` at the start of a line | `if c` / branch / `else` / branch, as an `if_block` | same |
   | `sig -> T =` at the end of the header line, or `=` indented under the clauses, or `= expr` after a long header | the long form: `=` alone at column 0, the body indented (`spec/02 §3`) | the declaration is a function, handler or predicate |
   | a name `_x` | `x`, renamed at every occurrence in its scope, or the first of `x2`, `x3`, … that rule 3a allows | a local, a parameter or a binding of a pattern |
   | `_` as a binding, or as a whole pattern | the binding left out (`is Some`), or an `else` arm | the arm is the last one, for `else` (U2008 otherwise) |
   | `e if error …` or `e if none …` on one line | the handling on the next line, indented by two more spaces | `e` is the whole line of a `let`, a statement or a final |
   | `result`, `error`, `unit` or `it` as a local or parameter name | `<word>_value`, renamed in its scope, or the first of `<word>_value2`, `<word>_value3`, … that rule 3a allows | a local or a parameter, never a field |
   | `when x is A -> a is B -> b` on one line | a `when` block, one arm per line. A new arm starts at each `is`, `in` or `else` at depth 0 that is followed at depth 0 by `->` before the next one | the `when` is the final, the value of a `let`, or an arm's body |
   | `if c then X` with no `else`, followed by the rest `R` of its block | `if c` / `X` / `else` / `R`, with `R` indented | `R` has at least one line |
   | `-> let x = …` or `then let x = …`, with the rest of the block on the following lines | the arm's or branch's body as a block on the next line | the `let` begins the body |
   | `-> when x`, with its arms on the following lines | the arm's body as a block holding the `when` | same |
   | `fn name(…) -> T` at column 0 | `fn ` deleted | the header otherwise parses |
   | `;` at the end of a line, or between two statements | deleted, or a line break with the statement's indentation | outside a text literal |
   | `priv name(…)` | `priv ` deleted, since a declaration without `pub` is private | the header otherwise parses |

   3a. **Fresh names.** A rename's new name must be visible nowhere it would be written. At none of the renamed binding's occurrence sites, nor anywhere in its scope, may it name any of these:
   - a local or a parameter of the scope or of an enclosing one, a pattern's binding included;
   - a declaration of the module, the poisoned names of rule 8 included;
   - an imported name;
   - a name of the prelude.

   The suffix is the first one that meets this. So no occurrence changes what it refers to: the rename neither captures an outer binding nor shadows a function that the scope calls. A conformance case renames `_x` in a scope where `x` is both an outer local and a function of the module, and expects `x2` with every reference to both unchanged.

   The shapes with no exact rewrite carry a description of Una's form only:
   - a lambda body over several lines: "a lambda's body is one line; move the block into a named function";
   - list patterns: "`when xs.at(index: 0)` with `is Some(…)` and `is None`, or `if xs.len == 0`";
   - tuples: "declare a record kind with named fields";
   - braces, `|>`, `as`, `A | B` in a type, and `fail` of a stored value (that last one already has its description).
4. **One shape per declaration per check.** The parser still reports one U0101 per declaration. A declaration with several shapes is fixed over several `una fix` rounds, each of which re-checks in memory (ADR 0084). Its limit of 16 rounds stays: when it stops, the output says `stopped: limit`, and running `una fix` again continues.

### B. `una fix` applies them, and `una check` says how many (#576)

5. **No new command, and no change to `una fmt`.**
   - The exact edits of A are mechanical, cost 0 and widen nothing, so `una fix` applies them under ADR 0084's rules as they stand.
   - `una fmt` keeps `spec/13 §3`: no command but `una fix` applies a repair.
   - U2004 and U7001 are applied by `una fix` today, the first as a candidate and the second by its format step. Nothing changes for them.
6. **The count.**
   - `una.diagnostics/1` gains `"fixable": n` after `diagnostics`. It is an added key, so the schema stays at version 1 (ADR 0076 rule 1).
   - `n` counts the diagnostics whose first repair is a candidate of `una fix` (ADR 0084), plus every U7001 with an edit, which `una fix`'s format step applies.
   - In human mode, when `n > 0`, the last line on stderr is `<n> of <m> diagnostics have an edit that \`una fix\` applies (\`una fix --dry-run\` lists them)`.
   - The count is of the first round only. Later rounds may apply more, since a file that parses after round 1 reports its later phases.

### C. Checking past a broken declaration (#575)

7. **Later phases run on what parsed.** In a file with a syntax error:
   - the declarations that parsed are resolved and typed as in a file without one;
   - the broken declarations are left out, as the parser already leaves them out of the tree.
   - Imports and the module header that parsed are kept. A broken import is broken for its whole line: each name it lists is a poisoned name of rule 8.
8. **Poisoned names.** The names a broken declaration would have declared are poisoned, read from its tokens:
   - the name at its start (the name before `(` or `[`, or the name after `kind`, `capability`, `handler`, `predicate reflect|opaque`, `policy` or `requirement`);
   - for a `kind`, every upper-case name in it, since those may be its constructors;
   - for a `capability`, every `op` name in it.

   A reference to a poisoned name resolves to the unknown that a broken module's names resolve to today (`Found::Unknown`). Its type is the error type that suppresses cascades (`compiler/architecture.md §5`). So a caller of a broken function gets no U1001, no U2004 and no type error from the call.

   When no name can be read at the start of a broken declaration, every unresolved name of the module resolves to the unknown, and so does every name imported from it.
9. **Dependents report less.** A declaration is a dependent when it reaches a poisoned name through its calls, the types it names, the constructors it uses or the functions it passes as values, directly or through other declarations of the package.
   - Dependents are resolved and typed, with the error type absorbing what depends on the broken one.
   - Their diagnostics of effects and information flow (U3xxx, U4xxx) and of canonical form are withheld, since those read an inferred header that a broken callee leaves incomplete.
   - Declarations that are not dependents report everything.
10. **Canonical form, per declaration.**
    - In a file with a syntax error, each declaration that parsed and is not a dependent is compared with its canonical text, as `una fmt` prints that declaration. Each difference is U7001 with its edit.
    - What only a whole file has is checked when the file parses: the blank lines between declarations, the order of imports and the final line break.
    - `spec/15`'s opening paragraph becomes: "A file with a syntax error has no canonical form as a whole: it is not formatted, and only its declarations that parsed are checked for U7001 (ADR 0096)".
    - `una fmt` still refuses such a file.
11. **Verification is unchanged.** Obligations still run only when nothing else is reported (`spec/12`). The verification waves of the measurement (29 in `full-c`) are left to a later decision.
12. **Complexity.**
    - The file is lexed and parsed once, with the recovery it has today, linear in its tokens. Nothing is re-parsed in a loop.
    - The poisoned names are read in one pass over the broken declarations' tokens.
    - The dependents are one reverse reachability over the package's dependency graph, `O(D + E)` for `D` declarations and `E` references.
    - The per-declaration canonical comparison formats each declaration once, linear in its size.
    - Resolution and typing are the existing passes, run on more declarations.
    - So a check stays linear in the source and in the dependency graph.
    - A scaling test checks files of 500, 1,000, 2,000 and 4,000 declarations, every fourth one broken, and fails when the time of a doubling grows by more than 2.5 times.

### D. Soundness

13. **A broken package never runs.**
    - A file with any error, broken declarations included, still makes `una check` fail, and `una build`, `una run`, `una test` and the complete Core export refuse the package (`spec/13 §5`, `§8`).
    - Rules 7 to 10 only report more diagnostics in a package that is already rejected. In a package without a syntax error, the output is unchanged.
    - The rewrites of A are repairs. `una fix` applies them and then checks the package again, so a rewrite that is wrong for its context shows up as a diagnostic, never as an accepted program.
14. **No spurious diagnostic.** For every conformance case with a syntax error, take the diagnostics of the declarations that are neither broken nor dependents. They must equal the diagnostics of the same file with the broken declarations deleted, after dropping those that the deletion causes (U1001 at a reference to a deleted name). A test checks this over the whole conformance suite, with one declaration broken in turn.

## Consequences

- **Easier.**
  - A fix loop of `una check` then `una fix` repairs the two largest shapes (49% of the syntax errors measured) without the agent writing the edit.
  - A check reports the names, types and labels of a file's other declarations together with its syntax error.
  - `una check` says how many of its diagnostics `una fix` would apply.
- **Harder.**
  - Every exact rewrite needs a conformance case of its own, the renames with rule 3a's case besides: the foreign source, the expected fixed source, a check of the fixed source with no syntax error, and a case where the condition fails and the repair has no edit.
  - The dependency closure of rule 9 is new checker code.
  - `spec/15` gains a per-declaration comparison.
- **Measured before implementation.** A replay of una v0.1.9 on the rebuilt snapshots counts what rules 7 to 10 would report in the same check. For each syntax-gated snapshot, it removes the broken declarations and checks what is left. It runs in a machine slot after the primed-arm campaigns.
- **Deferred.**
  - More than one U0101 per declaration.
  - Verification run on the declarations without other diagnostics.
  - Exact rewrites for lambdas, list patterns and tuples, which need a new declaration.

## Alternatives considered

- **Accept the common foreign forms as alternatives** (a multi-line `if … then … else`, `sig =` on one line). Rejected by P4. Two spellings of one construct make the canonical form a choice, and every agent and reader would meet both.
- **Let `una fmt` apply repairs.** Rejected: `spec/13 §3` keeps repairs out of every command but `una fix`, which has the rules on escapes and widening that `una fmt` lacks. `una fmt` stays a printer of canonical form.
- **Re-parse after each repair inside `una check`.** It would report a later phase's errors as if the repair had been made, which is a different file from the one on disk. Its cost grows with the number of errors. `una fix` already does rounds where the agent asks for them.
- **Report a broken declaration's dependents in full.** Their headers are inferred from a callee that is missing, so their effect and flow diagnostics could be wrong in both directions. Withholding them keeps rule 14's guarantee of no spurious diagnostic.
- **Check the whole broken file for canonical form** with the broken declarations printed as they are. A broken declaration has no canonical text, and the blank lines around it depend on what it was meant to be.

## Amendment (2026-10-12): reserved words as names

The table's row for a reserved word used as a local or parameter name is narrowed. `result` keeps the exact scoped rename that main already gives it (#206, #257). For `error`, `unit` and `it`, the syntax layer cannot tell a use of the binding from the keyword in the same scope: `if error use …` names the failure, and `unit` is a literal. So the edit's condition is not met, and the repair keeps its description without an edit, as rule 3 provides. This narrows the decision and needs no new acceptance.

## Amendment (2026-10-12): a dependent reports only its syntax

Rule 9 is narrowed. A dependent keeps only its syntax diagnostics: its name, type, effect, flow and canonical-form diagnostics are all withheld. The implementation's test of rule 8 found cascades that the error type does not absorb in a dependent. Examples are a variant of a broken kind (U1001), a function of a broken declaration called with method syntax (U1001), and a generic call or an empty list whose type comes from an unknown value (U2001). Withholding them keeps rule 8 and rule 14 exact. The cost is small: in the replay of `full-b` and `full-c`, 92% to 95% of what checking past a broken declaration adds lies outside dependents. This narrows the decision and needs no new acceptance.

## Amendment (2026-10-13): the rest of a block becomes an `else` only after a `fail`

Two rows of rule 3's table are narrowed, since their rewrite can change what runs once the result checks clean.

- **`if c then X` with no `else`, followed by the rest `R` of its block.** The edit is given only when `X` is `fail` of a variant, and the `if` has nothing before it on its line. In any other case the repair keeps its description with `edit: null`. Example: `if n > 0 then @console.print("positive")` followed by `@console.print("done")` would become `done` printed only when `n <= 0`.
- **`if c then x else` at the end of a line, the chain continued at the same indentation.** The rest of the block becomes the `else` branch only under the same condition, that `x` is `fail` of a variant. An `else` followed by a deeper indented block keeps its edit: the indentation already delimits the branch.

The table's other rows were checked against the same question: can the edit change what the program does, when the result then checks clean? Their edits only move tokens into the layout Una reads, or rename a binding to a name visible nowhere it is written. This narrows the decision and needs no new acceptance.

## Note (2026-10-13): dependents report their own name and type errors again

The amendment "a dependent reports only its syntax" (2026-10-12) is relaxed. #581 closed the cascades it worked around at the source. An unresolved name now reports one diagnostic, at the reference: an inference variable, a lambda parameter or an ordering that only the missing name could have typed reports nothing (`compiler/architecture.md §5`). Rule 9 holds as first written: a dependent withholds its effect, flow and canonical-form diagnostics. It also withholds a U1001 or U1006 at a poisoned name, which type checking reports for a variant or a method-syntax call that name resolution leaves alone. The tests of rules 8 and 14 pass without the amendment. This widens what a rejected package reports, never what is accepted, so no new acceptance is needed.

## Amendment (2026-10-15): `%`, and declaration keywords as names

- Status: Accepted
- Deciders: Una project (accepted on 2026-10-15)

**`%` gets a description, not an edit.** C, TypeScript and Rust write the remainder as `%`, which truncates: `-7 % 3` is `-1`. Una's `mod` is Euclidean: `-7 mod 3` is `2`. The two agree when both operands are non-negative, and differ otherwise. So rewriting `%` to `mod` would not keep the reading of rule 2(c) for negative operands, and an applied edit could silently change what the writer meant. The shape joins the list of shapes with no exact rewrite (rule 3). U0001 at a `%` keeps its message and gains this description: "Una writes the remainder as `mod`, which is Euclidean: `-7 mod 3 == 2`, where `%` in C, TypeScript and Rust gives `-1`; the two agree when both operands are non-negative". It is shown only for a `%` that the lexer reports as U0001, which is never one inside a text literal or a comment.

An edit applied only where the verifier proves both operands non-negative was considered and not chosen. The syntax layer computes edits, before tier 1 runs. A proof-guarded edit would be a new kind of repair: a tier-0 diagnostic whose edit depends on a tier-1 result, with its own obligation and determinism rules. The description gives the writer both readings and costs nothing.

**A declaration keyword used as a local name gets a rename edit.** One row joins the table of rule 3. It is exact under rule 2: it is computed from the tokens of one declaration and changes nothing outside it, it keeps the program's meaning token for token, and it is deterministic. It does not change what is accepted, widen authority or drop a condition, and the shape stays an error (rule 1).

| Shape | Una's form, and the edit | Condition for the edit |
| --- | --- | --- |
| a declaration keyword as a local name: `accepts`, `capability`, `do`, `example`, `expect`, `given`, `handler`, `import`, `kind`, `module`, `op`, `opaque`, `performed`, `policy`, `predicate`, `pub`, `reflect`, `requirement`, `trusted` | `<word>_value`, renamed in its scope, or the first of `<word>_value2`, `<word>_value3`, … that rule 3a allows | A binding inside a function body or a handler operation body: a `let` binding, a lambda parameter, a binding of a pattern, or a parameter of a handler operation. Never a parameter of a function, and never a parameter of a handler declaration. |

**Why not a function's parameters.** A call names its arguments by the parameters' names (`f(op: 1)`, or the pun `f(op)`). Renaming a function's parameter changes its interface, and every call in another declaration would stop matching it. That is outside the declaration (rule 2(a)), so a function parameter keeps the description. A handler operation's parameters are named by the capability's operation, not by the handler, and a lambda's arguments are passed without labels (a function value takes its arguments without labels, U2005), so renaming those changes no call.

**Why not a handler declaration's parameters.** Their scope holds the handler's own `op` declarations, where `op` is the keyword.

**What the rename covers.** Every use of the binding in its scope, including a use inside an interpolation hole, `"{op}"`. It never touches a name after `.`, which is a field or a method, or an argument label, which names the callee's parameter.

**Why these keywords.** The amendment of 2026-10-12 kept the rename edit for `result` only. The syntax layer cannot tell a use of a binding named `error`, `unit` or `it` from the keyword in the same scope. The keywords listed here are the opposite case: none of them can stand in an expression, a statement, a type or a header clause of a function. So within the binding's scope, the function's body and its clauses, every token of the word is a use of the binding, and the rename is exact. Every other keyword keeps the description without an edit, because it can appear in a scope as a keyword:
- in expressions: `if`, `when`, `let`, `and`, `fn`, `attempt`;
- in types: the label words `secret`, `internal`, …, and `where`;
- in header clauses: `requires`, `uses`, `derived`.
The examples' own keywords `do`, `expect`, `given` and `performed` appear only in `example` declarations, never in a function's scope.

The list must be checked again whenever a construct brings one of these words into a body. One example is the `test` declarations of ADR 0105 (Proposed), whose bodies would hold `expect`. Until such a check, the row applies only inside a function body or a handler operation body, as its condition says.

**Hygiene.** The fresh name follows rule 3a: it is visible nowhere it would be written, so no occurrence changes what it refers to.

**Determinism.** The edit depends only on the declaration's tokens.

Conformance cases, each with the source before and after and a check of the fixed source:
- `%` between operands: the description, and no edit;
- `op` as a `let` binding, renamed through its block, with a use inside an interpolation hole. The same case has a `.op` after a value and an argument label `op:`, which the rename leaves as they are. `op` is a keyword, so neither can be legal: the check of the fixed source still reports their own U0101, each with its description and no edit, and nothing else;
- `op` as a lambda parameter, and as a handler operation's parameter;
- no edit for `op` as a function's parameter or a handler declaration's parameter, with the description only;
- a fresh name that avoids a function of the module, and one that avoids a name the scope already binds (`op_value` taken, so `op_value2`) (rule 3a);
- no edit for `when` as a name, with the description only.
