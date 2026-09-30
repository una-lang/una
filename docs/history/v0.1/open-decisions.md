# Open decisions (overnight run)

Decision log of the autonomous run (`OVERNIGHT.md`, C.4). Every choice not literally dictated by the specification is recorded here. Entries are sorted by review priority: low confidence combined with high reversal cost first. Code and tests implementing a decision carry the comment `DECISION D-NNN`.

**Reviewed on 2026-09-30.** The independent review of v0.1 (`docs/reviews/v0.1-review/decisions.md`) recommended keeping 131 decisions and escalated D-093; the owner kept all 131 and resolved D-093 through ADR 0038 D4. Each entry records it on a `- Review` line. D-001 to D-132 are final, so their `DECISION D-NNN` tags were removed from the code and tests: the entries and their ADRs are the record. Decisions from D-133 on belong to the v0.1 closure run, await the owner's answer, and keep their tags until then.

**Answered on 2026-09-29.** The owner reviewed the log and kept every chosen option, so each entry below records its answer, and the Provisional ADRs became Accepted, with their rules stated in the specification (`OVERNIGHT.md` C.9).

## Blockers

None.

## Decisions

### D-023: The canonical form beyond the specification (M2)
- Question: The specification fixes indentation, trailing whitespace, the short form of functions, clause order and constructor order, but not comments, blank lines, spacing inside a line, continued lines, line endings or how U7001 reports differences. U7001 rejects every non-canonical program, so these rules are part of the language.
- Context: ADR 0011; `spec/01 §2`, `spec/04 §1, §3`; charter P4; `spec/13 §4, §5`.
- Options: A) the rules of ADR 0020: one line per logical line (breaks kept only around comments), fixed token spacing, one blank line between top-level items and collapsed runs elsewhere, comments never moved, LF only; B) keep the author's layout except where the specification requires otherwise; C) A with a line width and line breaking.
- Chosen: A, because ADR 0011 wants agents to converge on the same bytes and B leaves many forms of one program; C contradicts `spec/04 §1` (no width limit in v0.1). Keeping every comment on its line, with the long form or the line breaks where a comment requires them, guarantees that documentation and doc-comment attachment never change.
- Confidence: medium
- Reversal cost: medium, `crates/una-fmt` and a reformat of every `.una` file of the repository (mechanical, `una fmt`)
- Implemented in: M2 formatter commit (`697cd2e`), `crates/una-fmt/src/`, `crates/una-fmt/tests/`, tag `DECISION D-023`
- ADR: `docs/adr/0020-canonical-form.md` (Accepted)
- Answer needed: Keep A? Or switch to B, or change a specific rule?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-037: What an import brings, and which scope wins (M3)
- Question: `spec/02 §1` makes a module's top-level names visible throughout it and `spec/14 §1` imports the prelude into every module, but nothing says what an import of a kind brings or which of two visible names wins. The previous reference example used a variant after importing only its kind, and declared a variant `NotFound` that the prelude kind `FileError` also has.
- Context: `spec/02 §1`, `spec/14 §1, §3`; the reference example; ADR 0023 rules 1 and 2.
- Options: A) scopes from innermost: locals, the module's declarations, explicit imports, the variants of imported sum kinds, the prelude; the innermost holder wins, and a tie within one scope goes to the type checker (M4); B) an import brings only the names listed, and prelude names cannot be reused; C) every variant is resolved by the expected type only.
- Chosen: A, because it makes both uses valid as written, keeps every later prelude addition from breaking programs, and leaves genuinely ambiguous names to the only pass that can disambiguate them. B rejects the reference example; C leaves bare constructors such as `SoldOut` meaningless outside a typed context.
- Confidence: medium
- Reversal cost: medium, `crates/una-check/src/names/` and the M4 disambiguation that builds on it
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-037`
- ADR: `docs/adr/0023-name-resolution.md` (Accepted)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-046: Variants in patterns and after `fail` take their kind from the context (M3)
- Question: `examples/events/src/events/main.una` matches `is SoldOut`, `is Few` and `is Open` without importing `Availability` or its variants. How are such variants resolved?
- Context: `spec/02 §6, §7`, `spec/06 §2`, `spec/08 §2`; `examples/events`; ADR 0023 rule 3.
- Options: A) a variant whose kind the context gives (a `when` pattern or `is` test: the tested value's kind; `fail` in an expression, a handling clause or `checked`: the function's error kind; `expect fails`: the `do` expression's error kind) is resolved by that kind in M4, and M3 does not report it; B) resolve every variant through scopes, which rejects the reference example.
- Chosen: A, because the reference example requires it and the kind is always known in these positions; bare constructors still resolve through scopes in M3.
- Confidence: medium
- Reversal cost: medium, `resolve.rs` in `crates/una-check/src/names/` and the M4 variant check that relies on it
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/resolve.rs`, `crates/una-check/tests/names.rs`, tag `DECISION D-046`
- ADR: `docs/adr/0023-name-resolution.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-048: Variants after `fail`, in patterns and after `expect fails` (M4)
- Question: ADR 0023 rule 3 leaves contextual variants to M4. Where is each looked up, and which code reports a miss?
- Context: `spec/06 §2` (U3009), `spec/10 §4` (T-Case); ADR 0023; ADR 0024 rule 1.
- Options: A) in the tested value's kind for patterns and `is` (U1001 if absent, U2001 if not a sum); in the declared error kind after `fail`, through scopes when none is declared (a variant outside the declared kind is U3009 in M5); in the `do` call's error kind after `expect fails`; B) always through scopes.
- Chosen: A, because the kind is known in every one of these positions and the reference example relies on it; B cannot type `main.una`.
- Confidence: medium
- Reversal cost: medium, the variant lookups of the type checker
- Implemented in: `0c07912` (M4 type checker), `crates/una-check/src/types/`, tag `DECISION D-048`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep; the implementation is fixed for R-020 (`expect fails` looks only in the error kind of the `do` expression).

### D-063: Equality on type parameters instantiated with function types (M4)
- Question: `spec/03 §1` forbids `==` on function types, and `spec/03 §2` rule 4 allows `==` on type parameters. A generic function that compares values of `T`, called with a function type for `T`, would compare functions at run time.
- Context: `spec/03 §1, §2`, `spec/14 §1` (`contains`); charter theorem T1 (`spec/10 §10`); ADR 0024 rule 11.
- Options: A) record which type parameters each generic function compares, directly or through the generic functions it calls (the prelude's `contains` compares its items), and report U2001 at an instantiation that gives such a parameter a type whose values hold functions; B) forbid every type argument that holds functions; C) leave it to run time.
- Chosen: A, because it keeps type safety and rejects only the calls that would compare functions. B rejects harmless programs such as mapping over a list of functions; C needs a meaning for function equality that the specification denies.
- Confidence: medium
- Reversal cost: medium, `facts::generic_equality` and `KindFacts` in `crates/una-check/src/types/facts.rs`; switching to C would make the interpreter define function equality
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-063`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted), rule 11
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-071: Which code reports a missing capability (M5)
- Question: `spec/05 §4` reports a capability that a callee needs and the caller lacks as U3001, `spec/04 §2, §5` reports effects beyond declared `uses` as U3003, and `spec/04 §2` rule 4 reports `@` in a function without `uses` as U3004. Which code applies where?
- Context: `spec/04 §2, §5`, `spec/05 §3, §4`, `spec/10 §2` (Suff); ADR 0028 rule 1.
- Options: A) U3004 for `@c` in a body whose `uses` are declared empty; U3001 for `@c` that no binding holds, and for a callee requirement whose name is absent or bound to another capability; U3002 for fewer operations; U3003 for a call with effects from a body whose `uses` are declared empty; B) U3001 for every missing capability.
- Chosen: A, because each statement of the specification keeps a case of its own, and every code gets one site. A rebinding `with @x = @y` whose `@y` is not held is reported once, at `@y`, and not again where `@x` is used; clauses are in the refinement language, so U2014 reports an operation there and the authority codes do not.
- Confidence: medium
- Reversal cost: medium, the sites in `crates/una-check/src/types/` that report these codes
- Implemented in: `9d34aab`, `0ded2b9` (edits), `crates/una-check/src/types/authority.rs`, tag `DECISION D-071`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 1
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-072: Inferring the headers of private functions (M5)
- Question: ADR 0007 infers the `uses` and `fails` of private functions over the module's call graph, but a body's `@x.op` does not say which capability `@x` is, and a function may declare some clauses and omit others.
- Context: `spec/04 §5`, `spec/05 §4`; ADR 0007; ADR 0028 rule 2.
- Options: A) a private function with no `uses` clause infers them, one with any declares them all, and likewise for `fails`; an inferred reference takes its capability from the first of three sources that decides it: the declared requirements of the code the function calls (a callee's `uses`, a function type's, a handler's), then the bindings of its callers after `with`, then the handlers its callers install around the call; a caller or handler that disagrees with the decision is reported at its call (U3001, U3010); sources of the deciding kind that disagree, or no source at all, are U3001 at the function with a hint; the inferred error kind is the kind of the first variant raised or failure passed, and every other kind is reported where it is raised (U3009) or passed (U3008); B) infer a reference's capability from its operation's name; C) take the capability from callers only, reporting every disagreement as a conflict at the function.
- Chosen: A, because capabilities flow downward by name (`spec/05 §4`): a callee's declared requirement fixes what the name is, and the callers must then provide it; ranking the sources reports each disagreement at the call an agent edits, and lets a private helper without callers yet infer a complete header. B is ambiguous when two capabilities share an operation name; C reports a wrong handler or binding far from where it is written.
- Confidence: medium
- Reversal cost: medium, the inference pass and the second checking pass
- Implemented in: `9d34aab`, `crates/una-check/src/types/authority.rs` (`infer`, `flow`), `env.rs`, tag `DECISION D-072`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 2
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-088: Structural labels in the checker's types (M6)
- Question: Labels can sit at any position of a type (`Option[internal Text]`, `List[secret Int]`), and a generic call must carry the labels of its arguments; how does the checker represent them?
- Context: `spec/03 §5`, `spec/07 §2`, `spec/09 §2`, `spec/10 §2`; ADR 0031 rule 1.
- Options: A) labels at every position of the checker's types, ignored by type equality and joined on type variables by unification; B) one label per value, the join of every label in its type; C) a separate information-flow pass over the typed program.
- Chosen: A, because it is the model of Core IR (`spec/09 §2`) and keeps `Option[internal Text]` precise; B rejects a public-topped option of internal text wherever it flows, and C repeats the type checker's resolution of fields, methods and instantiations.
- Confidence: medium
- Reversal cost: medium, the labeled node of `Ty` and the unifier in `crates/una-check/src/types/`
- Implemented in: `d519fbb`, `78ad11e`, `crates/una-check/src/types/label.rs`, `ty.rs`, `unify.rs`, tag `DECISION D-088`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 1
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-098: Elaboration from the checker's resolutions (M7)
- Question: Core IR needs every call resolved, arguments positional and every binder typed (`spec/09 §1`); the checker computes all of this but keeps none of it.
- Context: `spec/09`, `compiler/architecture.md §2, §3`; ADR 0032 rule 1.
- Options: A) pass 2 records its resolutions by span and the elaborator reads them; B) an elaborator that resolves names and types again.
- Chosen: A, because one set of resolution rules cannot disagree with itself; the records cost memory only.
- Confidence: medium
- Reversal cost: medium, the records of `crates/una-check/src/types/` and the elaborator
- Implemented in: `59a481d`, `crates/una-check/src/types/records.rs`, `elab.rs`, tag `DECISION D-098`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 1
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-111: Formulas and spans in Core IR (M8)
- Question: `spec/12 §3` generates obligations from Core IR, but M7's Core names refinements without their predicates, has no `requires` or `ensures`, keeps spans on few nodes, and drops the condition of the last arm of a `when` without `else`.
- Context: `spec/09 §2`, `spec/10` preamble, `spec/12 §3`; ADR 0032 rule 1; ADR 0033 rule 1.
- Options: A) terms of the refinement language for refinements, clauses, reflect bodies and `checked`, with text and spans; library signatures; spans on returned atoms, `let` values, arguments and divisions; an `unreachable` branch after the last arm of a `when` without `else`; B) formulas as A-normal Core expressions; C) obligations from the syntax tree, as M4 records them.
- Chosen: A, because obligations then come from Core IR as the specification requires, goals print as written, and the exhaustiveness obligation keeps every arm's condition; C would verify something other than what runs.
- Confidence: medium
- Reversal cost: medium, `crates/una-core`, the elaborator and the verifier
- Implemented in: `d8d3db6`, `02bb2d5`, `crates/una-core/src/ir.rs`, `print.rs`, `crates/una-check/src/types/elab.rs`, tag `DECISION D-111`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 1
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-113: One obligation per `ensures` clause over every return path (M8)
- Question: `spec/12 §3` generates a postcondition at "each normal return path", and `§8` gives an obligation an id from its declaration, kind and goal, so the paths of one clause would share an id; `spec/13 §2` lists a postcondition with the clause as its `source`.
- Context: `spec/12 §3, §6, §8`, `spec/13 §1, §2`; ADR 0033 rule 3.
- Options: A) one obligation per clause, `Π ∧ (path₁ ∨ … ∨ pathₙ) ∧ ¬goal`, whose diagnostic points at the returned expression of the path the model takes; the refinement of a declared result type alike; B) one obligation per path, with the path in the id.
- Chosen: A, because ids stay as `spec/12 §8` defines them and unique, the listing matches `spec/13 §2`, and the diagnostic still points at the failing return as `spec/12 §3` asks.
- Confidence: medium
- Reversal cost: medium, the generation of return obligations and their ids
- Implemented in: `49c73e9`, `crates/una-verify/src/generate.rs` (`returns`), `report.rs`, tag `DECISION D-113`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 3
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-018: `[` right after a label level (M1)
- Question: After a level such as `secret`, `[` can start the label's categories (`secret[PII] Phone`) or a list type (`secret [Int]`). The grammar of `spec/02 §2` allows both readings, and telling them apart needs unbounded lookahead.
- Context: `spec/02` introduction (LL(2) except kind bodies) and §2; `spec/03 §1`, `spec/07 §2`, `spec/14` (`List[T]`).
- Options: A) `[` after a level always starts categories; a labeled list type is written `secret List[T]`, and the error says so; B) backtrack and reread `[Int]` as a list type when no type follows; C) forbid `[T]` after any label.
- Chosen: A, because it keeps the parser LL(2) as the spec states, rejects the ambiguous spelling instead of guessing (strictness), and gives agents a precise fix. B could be adopted later without breaking any program; C is stricter than needed, since `secret[PII] [Int]` is unambiguous.
- Confidence: medium
- Reversal cost: low, `label` and `type_core` in `crates/una-syntax/src/parser.rs`; one sentence of `spec/02 §2` on acceptance
- Implemented in: M1 parser commit (`d95fe43`), `crates/una-syntax/src/parser.rs`, `crates/una-syntax/tests/parser.rs`, tag `DECISION D-018`
- ADR: `docs/adr/0018-label-bracket-starts-categories.md` (Accepted)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-022: Nesting limit (M1)
- Question: `una check` aborted with a stack overflow on 20 000 nested parentheses, a sum of 200 000 terms, and 5 000 text literals nested in interpolations. The grammar sets no bound on nesting. What should happen?
- Context: `spec/02`; ADR 0017 decision 7 (robustness); `compiler/architecture.md §7` (dependency allowlist); `CLAUDE.md` (deterministic output).
- Options: A) reject nesting deeper than 256 levels with U0101, counting nested expressions, prefix operators, chained operators, postfix links, types and blocks; bound the lexer's literal-in-interpolation recursion; run `una check` on a 64 MiB stack; B) only a larger stack; C) a limit measured in bytes of stack; D) count only true recursion, not operator chains.
- Chosen: A, because it is deterministic (the same in every build), bounds the depth of the tree for every later pass as well as the parser, and turns a crash into a diagnostic with a fix. B and D leave deep trees crashing later passes, and C depends on the build. 256 levels is far beyond hand-written or generated code, and raising it later breaks no program.
- Confidence: medium
- Reversal cost: low, `MAX_NESTING` and `deeper` in `crates/una-syntax/src/parser.rs`, `text_literal_with` in `crates/una-syntax/src/lexer.rs`, `on_large_stack` in `crates/una-cli/src/lib.rs`
- Implemented in: M1 nesting commit (`bde3b98`), those files, `crates/una-syntax/tests/robustness.rs`, `crates/una-cli/tests/cli.rs`, tag `DECISION D-022`; M1 explain commit (`b1039e3`), `crates/una-diag/explain/U0101.md`
- ADR: `docs/adr/0019-nesting-limit.md` (Accepted)
- Answer needed: Keep A with 256? Or change the limit, or switch to D?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-019: Parse errors after a lexical error in the same declaration (M1)
- Question: The lexer's recovery (skipping a character, snapping an indentation, closing brackets, skipping a reserved word) can leave tokens the grammar rejects, as in `a & b` read as `a b`. Does the parser report U0101 for them?
- Context: `compiler/architecture.md §5`; approved M1 plan §6 (at most one U0101 per declaration); ADR 0017.
- Options: A) report the parser's U0101 anyway; B) do not report a parser U0101 when a lexical error lies between the start of its declaration and the unexpected token; C) as B, but only after lexical errors whose recovery changes the tokens (U0001 to U0004, U0101, U0103).
- Chosen: B, because such a U0101 points at code that is not wrong (after `a & b`, an agent would also be told that `b` is unexpected), the declaration is rejected either way, and a genuine second error appears once the lexical error is fixed, as under the one-U0101-per-declaration rule. C is more precise but couples the parser to the lexer's recovery details.
- Confidence: medium
- Reversal cost: low, `report` in `crates/una-syntax/src/parser.rs`
- Implemented in: M1 parser commit (`d95fe43`), `crates/una-syntax/src/parser.rs`, `crates/una-syntax/tests/parser.rs`, tag `DECISION D-019`
- ADR: none (minor)
- Answer needed: Keep B? Or switch to A or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-021: Doc comments separated from their declaration (M1)
- Question: `spec/01 §3` attaches consecutive doc comment lines to the next declaration. Does a doc comment attach when blank lines or plain comments separate it from the declaration?
- Context: `spec/01 §3`; ADR 0017 decision 6.
- Options: A) every doc comment between the previous token and the declaration attaches; B) only the doc lines directly above the declaration attach, and the others are plain comments.
- Chosen: A, because the spec attaches doc comments to the next declaration without conditions on blank lines, and ADR 0017 makes only a doc comment "not followed by a declaration" plain. B would silently demote documentation.
- Confidence: medium
- Reversal cost: low, `docs_before` in `crates/una-syntax/src/parser.rs`; the formatter (M2) and `una contract` read the result
- Implemented in: M1 parser commit (`d95fe43`), `crates/una-syntax/src/parser.rs`, `crates/una-syntax/tests/parser.rs`, tag `DECISION D-021`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-024: One-line bodies for predicates, handler operations and arms (M2)
- Question: `spec/04 §1` makes the short form canonical for a function without clauses whose body is one line. Predicate bodies, handler operation bodies and `when` arms have the same `= line | block` choice. Is the rule the same for them?
- Context: `spec/04 §1`, `spec/02 §3, §4, §6`; ADR 0020 rule 4.
- Options: A) the same rule for all four; B) only functions, as written.
- Chosen: A, because the four constructs share one grammar shape, one canonical form for all of them follows charter P4, and it is the stricter option (C.3), which can be relaxed without breaking programs.
- Confidence: medium
- Reversal cost: low, `body` in `crates/una-fmt/src/rewrite.rs`
- Implemented in: M2 formatter commit (`697cd2e`), `crates/una-fmt/src/rewrite.rs`, `crates/una-fmt/tests/format.rs`, `tests/conformance/fmt/inline_arm_body.una`, tag `DECISION D-024`
- ADR: `docs/adr/0020-canonical-form.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-025: Imports are sorted (M2)
- Question: Does the canonical form order imports, and the names inside an import?
- Context: ADR 0011 (diffs from different agents converge); ADR 0020 rule 7.
- Options: A) sort imports bytewise by module path and names bytewise; B) keep the author's order.
- Chosen: A, because two agents adding imports in a different order would otherwise produce different files, and sorting is the stricter option. The examples were reformatted, which reordered the imports of two modules of the previous reference example and their two syntax-tree snapshots (reviewed: only import lines moved).
- Confidence: medium
- Reversal cost: low, `imports` in `crates/una-fmt/src/layout.rs` and `collect` in `crates/una-fmt/src/rewrite.rs`
- Implemented in: M2 formatter commit (`697cd2e`), those files, `crates/una-fmt/tests/format.rs`, tag `DECISION D-025`
- ADR: `docs/adr/0020-canonical-form.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-028: The shape of U7001 (M2)
- Question: How does U7001 report a non-canonical file: once per file, per line, or per changed region? With which edit?
- Context: `spec/13 §1` (edits), ADR 0011 (mechanical edits), ADR 0020 rule 10.
- Options: A) one U7001 per changed region of a line diff, at its first line, with a classified message (trailing whitespace, missing or extra blank line, missing final line break, lines not canonical) and an edit replacing the region; B) one U7001 per file with an edit replacing the whole file; C) one per differing line.
- Chosen: A, because it points at each difference, keeps edits small and independent, and names the common cases precisely. A file with `\r\n` line breaks gets one U7001 at the first of them without an edit, since positions count decoded text, in which the carriage return no longer exists.
- Confidence: medium
- Reversal cost: low, `differences` in `crates/una-fmt/src/diff.rs`
- Implemented in: M2 CLI commit (`acf4ec2`), `crates/una-fmt/src/diff.rs`, `tests/conformance/fmt/`, tag `DECISION D-028`
- ADR: `docs/adr/0020-canonical-form.md` (Accepted)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-029: What `una fmt` does (M2)
- Question: `spec/13 §5` says `una fmt [path]` rewrites to canonical form and `--check` reports U7001 without writing. What happens with syntax errors, what is printed, and how are files written?
- Context: `spec/13 §5`; ADR 0012 (JSON list for `una fmt --check`); ADR 0015 D1 and D3.
- Options: A) files with syntax errors are reported and left untouched (exit 1); other files are rewritten through a temporary file and a rename, keeping their permissions; nothing is printed on success; `--json` prints the diagnostic list in both modes; B) refuse to write anything when any file has syntax errors; C) print the paths of rewritten files.
- Chosen: A, because formatting the other files is safe and useful while one file is broken, a rename never leaves a truncated source, and silence on success matches `una check`. A failure to write a file exits 2, like other output failures (ADR 0015 D1).
- Confidence: medium
- Reversal cost: low, `fmt_command` in `crates/una-cli/src/lib.rs`
- Implemented in: M2 CLI commit (`acf4ec2`), `crates/una-cli/src/lib.rs`, `crates/una-cli/src/args.rs`, `crates/una-cli/tests/cli.rs`, tag `DECISION D-029`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-030: Reading the round-trip criterion (M2)
- Question: M2 requires that `parse(fmt(x))` equals `parse(x)` modulo spans, but the formatter itself rewrites the tree by design: short forms (`spec/04 §1`), constructor order (`spec/04 §3.4`) and sorted imports.
- Context: `plan/milestones-v0.1.md` M2; ADR 0020 rules 4, 6 and 7.
- Options: A) compare the trees after applying those rewrites to both sides, and check separately that every comment survives; B) exclude files that the formatter rewrites; C) compare only canonical files, which makes the test trivial.
- Chosen: A, because it checks every file while allowing exactly the documented rewrites, and normalizing both sides keeps the comparison valid when a comment prevents a rewrite. Declaration order of constructor arguments is checked by unit tests.
- Confidence: medium
- Reversal cost: low, `normalize` in `crates/una-fmt/tests/properties.rs`
- Implemented in: M2 formatter commit (`697cd2e`), `crates/una-fmt/tests/properties.rs`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-039: Shadowing (M3)
- Question: May a local rebind another local (`let x` after a parameter `x`), and may declarations, imports and locals reuse prelude or module names?
- Context: ADR 0023 rule 4; C.3 strictness; the previous reference example declared a variant `NotFound`, which the prelude kind `FileError` also has.
- Options: A) a local rebinding a local of an enclosing scope is U1002; a declaration or import may reuse a prelude name, and a local any non-local name; B) allow every rebinding; C) forbid every reuse.
- Chosen: A, because rebinding locals makes a reader track which binding each use means, which matters most in contracts, while forbidding reuse of global names would make common local names such as `sum` or `first` collide with the prelude and break programs whenever the prelude grows. C also rejects a module that declares its own `NotFound`.
- Confidence: medium
- Reversal cost: low, the local scope stack in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-039`
- ADR: `docs/adr/0023-name-resolution.md` (Accepted)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-033: The keys and values of `una.toml` (M3)
- Question: `spec/13 §7` shows an example and says unknown keys are U7002, but not which keys are required or which values are valid.
- Context: `spec/13 §5, §7`; ADR 0021 rule 2.
- Options: A) `[package]` with non-empty `name`, `version` and `edition = "0.1"` required; `[labels] categories` UpperCamelCase and unique; `[trusted] modules`; `[escape_budget] checked_per_module` at least 0; `[run] entry` naming a module of the package and `grants` in the syntax of `spec/13 §5`; any other key, a wrong type or a value out of range is U7002; B) accept any values and check them only where used.
- Chosen: A, because a manifest error should be reported where it is written, with the whole package checked, and every rule can be loosened later without breaking a package.
- Confidence: medium
- Reversal cost: low, `crates/una-project/src/manifest.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-project/src/manifest.rs`, tag `DECISION D-033`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-036: Locating diagnostics that have no source position (M3)
- Question: A missing `verification.lock` (U7004) has no position in any file, yet every diagnostic has a required `primary` location (M0 decision, ADR 0012). The owner asked for this ADR when U7004 is implemented.
- Context: `spec/13 §1`; ADR 0012; `compiler/testing.md §2` (expected positions lie in existing files); ADR 0022.
- Options: A) the empty range at 1:1 of the file that declares what is checked: `una.toml` for a package, the file itself for a single file; B) a position in the missing file; C) make `primary` optional.
- Chosen: A, because the ABI keeps one shape and the position is displayable and testable; B points into a file that does not exist, and C changes every diagnostic's shape for a handful of codes.
- Confidence: medium
- Reversal cost: low, the missing-lock branch in `crates/una-check/src/project.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-check/src/project.rs`, tag `DECISION D-036`
- ADR: `docs/adr/0022-diagnostics-without-a-source-position.md` (Accepted)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-038: Where U1001 ends and U2002 begins, and what M3 resolves (M3)
- Question: An undeclared type name is both an "Unresolved name" (U1001) and an "Unknown ... type" (U2002). Which code, and which names can M3 resolve before types exist?
- Context: `spec/13 §4`; `spec/04 §3` (method calls depend on types); ADR 0023 rule 3.
- Options: A) U1001 for any name that no scope holds, type names included; U2002 (M4) for a name that resolves to something that is not a type, or an ill-formed type; fields, methods and argument labels wait for M4; B) U2002 for every unknown type name.
- Chosen: A, because one code then means "this name exists nowhere", reported by one pass, and U2002 keeps a precise meaning for types that exist but are wrong.
- Confidence: medium
- Reversal cost: low, the type branch of the resolver in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-038`
- ADR: `docs/adr/0023-name-resolution.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-043: Explain texts with package examples (M3)
- Question: The approved explain format has one `una` block per example (M1 plan §6). U1003, U1004, U1008, U7002, U7004 and U7005 cannot be shown in a single file.
- Context: `spec/13 §4`; `plan/runs/M1-plan.md §6`; `crates/una-cli/tests/explain_examples.rs`.
- Options: A) an example may list several files, each fenced block introduced by its path in backticks on its own line; the test builds a package from them, adding the default `una.toml` and the repository's `verification.lock` unless the example gives its own; B) describe those codes in prose, without examples.
- Chosen: A, because every code keeps a checked failing and fixed example, and the files an agent must touch are visible.
- Confidence: medium
- Reversal cost: low, the example parser of `crates/una-cli/tests/explain_examples.rs` and six explain texts
- Implemented in: M3 explain commit (`b723bd2`), `crates/una-cli/tests/explain_examples.rs`, `crates/una-diag/explain/`, tag `DECISION D-043`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-050: Text literals in the refinement language (M4)
- Question: `spec/03 §3` lists integer and boolean literals among RL atoms but not text literals, while `spec/12 §4` encodes text literals. Is `it != ""` a valid refinement?
- Context: `spec/03 §3`, `spec/12 §4`; ADR 0024 rule 3.
- Options: A) text literals are RL atoms; B) they are not (U2014).
- Chosen: A, because the encoding already supports them, so rejecting them would forbid refinements that can be verified.
- Confidence: medium
- Reversal cost: low, `Checker::text` in `crates/una-check/src/types/expr.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-050`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-051: What U2008 covers (M4)
- Question: U2008 is "Unreachable arm or misplaced `else`". Which arms are unreachable?
- Context: `spec/02 §8` (an `else` arm is the last arm), `spec/10 §4`; ADR 0024 rule 4.
- Options: A) an arm after `else`, a repeated variant or literal arm, and an `else` after arms that cover every variant; B) only an arm after `else`.
- Chosen: A, because each of these arms can never run, and reporting them is the stricter option.
- Confidence: medium
- Reversal cost: low, the arm checks in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-051`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-062: Reading the recursion restriction of kinds (M4)
- Question: `spec/03 §2` rule 5 allows recursive kinds "only through `List` or `Option` fields". Is a sum with a variant without fields (`Tree = Leaf | Branch(left: Tree, right: Tree)`), a type argument (`inner: Pair[Wrapped, Int]`) or a function type (`make: fn() -> Deferred`) recursion through something else?
- Context: `spec/03 §2` rule 5; ADR 0024 rule 10.
- Options: A) yes: a record or sum kind that contains itself anywhere except under `List` or `Option` is U2002, one diagnostic per cycle, at the first field that leads back into it, with an edit that wraps the field's type in `Option`; B) accept sums with a variant without fields, and function types, whose values are finite.
- Chosen: A, the literal reading and the stricter option; a tree is written with `List` or `Option` fields.
- Confidence: medium
- Reversal cost: low, `facts::kind_recursion` in `crates/una-check/src/types/facts.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-062`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted), rule 10
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-064: Punning of the first argument, and unlabeled constructor arguments (M4)
- Question: `spec/04 §3` rule 3 makes a bare name equal to a parameter name count as labeled. Does that hold for the first argument, which may go unlabeled anyway? Which code reports an unlabeled argument of a constructor with several fields, extra arguments of a constructor with one field, and a variant without fields called with parentheses?
- Context: `spec/04 §3` rules 1 to 4; ADR 0024 rule 12.
- Options: A) a bare name equal to a parameter's name is labeled with it wherever it appears, so `max(b, a)` passes `b` as `b`; an unlabeled constructor argument where labels are required is U2004 with an edit that adds the label; extra arguments of a constructor with one field are U2009; `V()` for a variant without fields is U2009 with an edit to `V`; B) the first argument is positional even when it names another parameter.
- Chosen: A, the literal reading of rule 3; under B, `max(b, a)` would report U2006.
- Confidence: medium
- Reversal cost: low, `Checker::punned` and `Checker::construct` in `crates/una-check/src/types/call.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-064`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted), rule 12
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-065: Recursion and results of predicates (M4)
- Question: `spec/03 §4` forbids a `reflect` predicate from calling itself "directly or indirectly", and the title of U2015 lists "recursive" for every predicate. Does the rule cover opaque predicates and cycles through functions? What reports a predicate whose result is not `Bool`?
- Context: `spec/03 §4`, `spec/13 §4`; ADR 0024 rule 13.
- Options: A) any cycle of calls or references that contains a predicate, reflect or opaque, through predicates or functions, is U2015 at the name of its first predicate, once per cycle; a result type other than `Bool` is U2001 at the result type; B) only reflect predicates calling reflect predicates.
- Chosen: A, because predicates are total and recursion endangers totality whatever closes the cycle, and the stricter reading can be relaxed later.
- Confidence: medium
- Reversal cost: low, `facts::predicate_recursion` and `Checker::predicate`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-065`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted), rule 13
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-061: Details of the refinement language (M4)
- Question: `spec/03 §3` lists the forms of the refinement language. Are `<` on `Text`, `is` with a literal, comparing with a variant by `==`, `unit` and interpolated text inside it?
- Context: `spec/03 §3`; `spec/12 §4` (no encoding orders text; variants are tested through tags); ADR 0024 rule 9; D-050.
- Options: A) outside it: `<` and the other orderings on `Text`, `is` with a literal (with an edit to `==`), `x == V` for a variant `V` (with an edit to `x is V`), `unit`, and interpolated text; plain text literals are inside it (D-050); B) accept all of them as sugar.
- Chosen: A, because each form is either absent from the list of `spec/03 §3` or has no encoding in `spec/12 §4`; the edits keep the cost to agents low, and relaxing later breaks no program.
- Confidence: medium
- Reversal cost: low, the calls of `outside_rl` in `crates/una-check/src/types/expr.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-061`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted), rule 9
- Answer needed: Keep A? Or accept some of the forms (B)?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-075: Host capabilities in examples (M5)
- Question: `spec/08 §2` rule 1 forbids host capabilities in examples (U3014) and takes every capability of the `do` line from `given` (U3001 if missing). Which applies to a missing `given`?
- Context: `spec/08 §2`, `spec/14 §3`; ADR 0028 rule 5.
- Options: A) a missing `given` for a host capability is U3014, for any other capability U3001; `expect performed @x.op` names a `given` binding (U3001) and an operation of its capability (U3002); B) U3001 for every missing `given`.
- Chosen: A, so that U3014 has its own case: a `do` line that would reach the host.
- Confidence: medium
- Reversal cost: low
- Implemented in: `9d34aab`, `crates/una-check/src/types/check.rs` (`example`, `performed`), tag `DECISION D-075`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 5
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-076: Grants checked by `una check` (M5)
- Question: U3012 is reported when `main` uses a host capability that `una run` does not grant; `[run] grants` is in `una.toml`, while `--grant` flags come only with `una run` (M7).
- Context: `spec/04 §8`, `spec/05 §7`, `spec/13 §5`; ADR 0028 rule 6.
- Options: A) when `una.toml` lists `[run] grants`, `una check` reports a host operation of `main`'s `uses` that no grant allows (U3012 at the clause), and when it names a `[run] entry`, a capability of `main`'s `uses` that is not a host capability, which no grant can provide; `una run` checks its own grants in M7; B) only `una run` reports U3012.
- Chosen: A, because the manifest's grants are known statically, and checking early is stricter; a package without `[run] grants` leaves the host capabilities to `una run`, while a capability that is not a host capability is ungranted whatever the grants (`spec/05 §7` rule 2).
- Confidence: medium
- Reversal cost: low
- Implemented in: `9d34aab`, `4f959a7` (capabilities that are not host capabilities), `crates/una-check/src/types/authority.rs` (`grants`), tag `DECISION D-076`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 6
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-079: Escaping lambdas and function values with effects (M5)
- Question: `spec/05 §6` and `spec/10 §9` rule 2 let an effectful or failing lambda appear only directly as a call argument (U3006). Does the rule cover named functions used as values, and fields of such function types?
- Context: `spec/03 §1`, `spec/05 §6`, `spec/10 §9`; ADR 0028 rule 9.
- Options: A) a lambda whose expected type has `uses` or `fails`, a function value of a function with `uses` or `fails`, and a parameter or local of such a function type appear only directly as call arguments or as the callee, and a kind field of such a type is U3006 at the type; B) lambdas only.
- Chosen: A, because a function value with effects carries the same authority as a lambda with them, a parameter that receives such a lambda would otherwise let it escape by being returned or stored, and `spec/03 §1` rules such values out of kinds.
- Confidence: medium
- Reversal cost: low
- Implemented in: `9d34aab`, `4f959a7` (parameters and locals), `crates/una-check/src/types/expr.rs`, `authority.rs` (`stored_effects`), tag `DECISION D-079`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 9
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep; the implementation is fixed for R-002 (ADR 0038 D2, U3016 for second-class type arguments).

### D-081: The output of `una contract` (M5)
- Question: `spec/13 §5` defines `una contract <qualified.name>` without a JSON shape, without saying which package it reads, or how inferred clauses appear; M5 requires a `--json` snapshot.
- Context: `spec/13 §5`; ADR 0029.
- Options: A) the package in the current directory, checked at tier 0, whose diagnostics are reported instead of the contract; human output as canonical header syntax with `-- inferred` after inferred clauses and a line per requirement; JSON schema `una.contract/1`; B) add a path argument; C) print the contract even when the package has diagnostics.
- Chosen: A, because it adds no command-line syntax, keeps the human output valid Una, and never shows a header whose inferred clauses may be incomplete.
- Confidence: medium
- Reversal cost: low, `crates/una-cli/src/contract.rs` and the schema
- Implemented in: `3be9490`, `crates/una-cli/src/contract.rs`, tag `DECISION D-081`
- ADR: `docs/adr/0029-contract-output.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-085: Function values with more effects or failures than the expected type (M5)
- Question: `spec/10 §2` (S-Fun) lets a function value stand for a function type only when its `uses` are sufficient for the expected type's and its error kind is included in the expected one, without naming the code of a violation, for example `write`, which uses `@log`, passed as the pure `each` of `map`.
- Context: `spec/10 §2`, `spec/04 §2, §6`; ADR 0028 rule 11.
- Options: A) U3003 when the value uses a capability that the expected type does not allow, U3009 when it can fail with a kind that the expected type does not declare; B) U2001, as a type mismatch.
- Chosen: A, because the codes then say which part of the expected contract is exceeded, as they do for a body that exceeds its header; both options reject the same programs.
- Confidence: medium
- Reversal cost: low, `function_argument` in `crates/una-check/src/types/call.rs`
- Implemented in: `9d34aab`, `crates/una-check/src/types/call.rs` (`function_argument`), tag `DECISION D-085`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 11
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep; the implementation is fixed for R-001 (ADR 0038 D1: S-Fun at every flow, and T-App).

### D-089: Where flows are checked, and implicit flows into a result (M6)
- Question: `spec/10` compares labels through subtyping (S-Refine, S-Fun) wherever a value meets a type, without listing the places or saying where an implicit flow into a result is reported.
- Context: `spec/07 §3`, `spec/10 §2, §4, §8`; ADR 0031 rule 2.
- Options: A) arguments, constructor fields, annotated `let`s, return paths, a policy's `from` and a lambda's result, covariant through data and contravariant through function parameters, arguments compared after the whole call is typed, and an implicit flow into a result reported once at the condition in return position; B) compare at every unification; C) report implicit flows at each return path.
- Chosen: A, because these are the places where a declared type meets a value; B would compare types that inference joins, such as branches, and C repeats one cause at every branch. A lambda's type records the labels its body assumed for its parameters, which keeps a body from branching on a parameter whose label rises later (a `fold` over labeled values needs a labeled initial value); a `Unit` value carries no information.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d519fbb`, `78ad11e`, `crates/una-check/src/types/flow.rs` (`flow`, `implicit_result`), `call.rs`, `stmt.rs`, tag `DECISION D-089`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 2
- Answer needed: Keep A? Or switch to C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-090: The labels of values (M6)
- Question: `spec/07 §3` rule 1 and `spec/10 §3` give operators the join of their operands, but not what `.len`, `is`, field access or pattern bindings take from structured values.
- Context: `spec/07 §3`, `spec/10 §3, §4`; ADR 0031 rule 3.
- Options: A) operators, equality and interpolation join every label of their operands; `.len`, `is` and `in` take the top label; fields and bindings take their declared type raised by the top label of the record or scrutinee; B) the top label everywhere.
- Chosen: A, because an equality of two lists or an interpolation reads the whole value, while `.len`, `is` and a field read only the structure the top label covers.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d519fbb`, `crates/una-check/src/types/expr.rs`, `call.rs`, `stmt.rs`, tag `DECISION D-090`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 3
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-091: `checked` branches on its value (M6)
- Question: T-Checked requires `pc = ⊥`, yet the failure of `checked(v, p) else fail V` reveals whether `p` holds of `v`, and failures are observable (`spec/07 §1`).
- Context: `spec/07 §1, §3`, `spec/10 §7`; ADR 0031 rule 4.
- Options: A) U4002 when `pc ⊔ ℓ(v)` is above `⊥`, with every label of `v`'s type; B) T-Checked's premise only.
- Chosen: A, because `spec/07 §3` rules 2 and 3 make the failure of a branch on a labeled value an implicit flow, and B would let a secret leak through `checked`.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d519fbb`, `crates/una-check/src/types/expr.rs` (`checked`), tag `DECISION D-091`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 4
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-092: Operation arguments are sinks (M6)
- Question: T-Op both types the arguments against the parameter types, labels included, and bounds them by `accepts`; `Files.write` declares `content: Text` and accepts `internal`, and `Keys.hmac` accepts "any label" without a way to write it.
- Context: `spec/05 §2`, `spec/07 §4`, `spec/10 §5`, `spec/14 §3`; ADR 0031 rule 6.
- Options: A) arguments are bounded by `accepts` only (U4003); handler operation parameters are raised by `accepts`; `Keys.hmac` accepts `⊤`; B) both bounds, so `accepts` above a parameter's label is unreachable.
- Chosen: A, because `accepts` is the sink label of `spec/07 §4`, and the prelude's declarations only make sense with it; raising the parameters inside handlers keeps what callers pass labeled.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d519fbb`, `crates/una-check/src/types/env.rs` (`accepts`), `flow.rs` (`sink`), `check.rs` (`handler`), tag `DECISION D-092`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 6
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-094: Policies, trusted modules and `via` functions (M6)
- Question: `spec/07 §5, §6` require a trusted module (U4004) and a private `via` of matching types (U4005); ADR 0024 rule 14 reported the types as U2001 before M6, and nothing says whether other code may call the `via` function.
- Context: `spec/07 §5, §6`, `spec/02 §8`; ADR 0024 rule 14, D-066; ADR 0031 rule 7.
- Options: A) U4004 when the header does not say `trusted` (U7005 covers a missing listing); U4005 for a `via` that is not a private function of the module, for its types, and for any other use of it; B) allow other code of the trusted module to call the `via` function.
- Chosen: A, because the policy is "the only entry point" (`spec/07 §5` rule 3), and a direct call would lower labels without the policy's audit trail.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d519fbb`, `crates/una-check/src/types/check.rs` (`policy`), `flow.rs` (`via_site`), tag `DECISION D-094`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 7
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-100: Values in traces and outputs (M7)
- Question: `spec/11 §5` shows trace values as JSON strings (`"Open"`, `"unit"`) without defining the representation, and `Hashed` has no text form.
- Context: `spec/11 §5`, `spec/14 §2`; ADR 0032 rule 3.
- Options: A) a JSON string with the Una form of the value, `"hashed:<hex>"` for `Hashed`, `"<redacted:level>"` for labeled arguments; B) JSON objects per value kind.
- Chosen: A, because it matches the example of `spec/11 §5` and needs no second representation.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d81f200`, `c3d2041`, `crates/una-interp/src/machine.rs`, `crates/una-cli/src/runtime.rs`, tag `DECISION D-100`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 3
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-101: The output of `una run` (M7)
- Question: `spec/13 §5` defines `una run` and its streams, but no JSON shape, and not how U6001 and U6003 are reported.
- Context: `spec/04 §8`, `spec/11 §7`, `spec/13 §5`; ADR 0027; ADR 0032 rule 4.
- Options: A) printed lines on stdout as the program prints them, U6001 and U6003 as diagnostics on stderr, and `una.run/1` with `--json`, whose `status` is `ok`, `failed`, `aborted` or `rejected` (the checks reported errors, so nothing ran) and whose `diagnostics` hold the diagnostics of the checks or of the run; B) a trace on stdout in human mode too.
- Chosen: A, because the program's output belongs on stdout, and agents get everything, the trace included, with `--json`.
- Confidence: medium
- Reversal cost: low
- Implemented in: `c3d2041`, `crates/una-cli/src/runtime.rs`, `crates/una-cli/tests/runtime.rs`, tag `DECISION D-101`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 4
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-102: The output of `una test` (M7)
- Question: `spec/08 §2` rule 6 lists what `una test --json` reports per example without a shape.
- Context: `spec/08 §2`, `spec/11 §6`, `spec/13 §5`; ADR 0032 rule 5.
- Options: A) `una.test/1` with the diagnostics of the checks and of aborted examples, and, per example, the module, name, outcome, the reason it fails, the first expectation that does not hold (file, line and text), the result and the trace; human mode prints one line per example, the expectation and the reason under a failed one, and a summary; B) one JSON document per example.
- Chosen: A, because one document keeps `--json` to one value on stdout, as `spec/13 §5` requires.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d81f200`, `c3d2041`, `crates/una-interp/src/harness.rs`, `crates/una-cli/src/runtime.rs`, tag `DECISION D-102`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 5
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-103: Grants: how they combine, and where their paths resolve (M7)
- Question: `spec/05 §7` and `spec/14 §3` confine `Files` to a prefix and load keys from a file, without saying how `una.toml` grants and `--grant` flags combine, where relative paths resolve, how paths normalize or what the key file holds.
- Context: `spec/04 §8`, `spec/05 §7`, `spec/13 §5, §7`, `spec/14 §3`; ADR 0032 rule 6.
- Options: A) the grants add up; `una.toml` paths resolve against the package root and flag paths against the current directory; `Files` paths normalize lexically and through the file system, and any path outside the prefix is `Denied`; the key file is a TOML table of base64 keys; B) flags replace `[run] grants`; C) lexical normalization only.
- Chosen: A, because a flag then adds what a run needs without restating the manifest, and resolving existing paths closes the escape through links that C leaves open.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d81f200`, `c3d2041`, `crates/una-interp/src/host.rs`, `machine.rs`, `crates/una-cli/src/runtime.rs`, tag `DECISION D-103`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 6
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep; the implementation is fixed for R-011 (ADR 0038 D9: the final component is never followed).

### D-104: Semantics the prelude leaves open (M7)
- Question: `spec/14 §1` gives the prelude's signatures and contracts, but not what `.len` counts, whether the empty text `is_digits`, what `trim` removes or what `to_int` accepts.
- Context: `spec/14 §1, §2`; ADR 0032 rule 7.
- Options: A) scalar values; `is_digits("")` holds; Unicode whitespace; an optional `-` and ASCII digits; B) bytes, and `is_digits("")` false.
- Chosen: A, because columns are counted in scalar values (`spec/13 §1`), and "every character is a digit" holds of the empty text.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d81f200`, `crates/una-interp/src/machine.rs`, `value.rs`; conformance case `runtime/prelude_functions`, tag `DECISION D-104`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 7
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-106: When an example fails beyond its expectations (M7)
- Question: `spec/08 §2` defines each expectation and says that `expect` lines with `result` fail when `do` failed, but not what happens when `do` fails and no `expect fails` line expects it, whether `expect` lines without `result` still run then, or which events `performed` counts.
- Context: `spec/08 §2`, `spec/11 §5, §6`; ADR 0032 rule 9.
- Options: A) the expectations are checked in order and the first that does not hold fails the example; then a `do` line that failed with no `expect fails` line fails it too; an `expect` line that does not mention `result` runs even when `do` failed; `performed` counts the completed or failed events of the `given` instance, and `not performed` any event of it; an abort fails the example; B) an example passes whenever its expectations hold.
- Chosen: A, because a failure nobody expected is what an example exists to catch; with B, `do` lines with only `performed` lines would pass whatever happened after the operation.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d81f200`, `crates/una-interp/src/harness.rs`; conformance cases `runtime/examples_pass`, `runtime/examples_fail`, tag `DECISION D-106`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 9
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-107: Redaction beyond the labels of arguments (M7)
- Question: `spec/11 §5` redacts arguments whose static label is above `public`, and records host results "subject to redaction", but a record argument whose own label is `public` can hold a field declared `secret`, and an operation's result has a label too.
- Context: `spec/07 §3` rule 1, `spec/11 §5`; ADR 0032 rule 3.
- Options: A) every rendered value shows a field whose declared type is labeled above `public` as `<redacted:level>`; an operation's outcome is redacted by the label of the binder of its result; `unit` is never redacted; B) redact arguments only, as the example of `spec/11 §5` shows.
- Chosen: A, because a trace must never show labeled data, and field labels are part of the static information the redaction rule uses; B would print a `secret` field of a public record in full.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d81f200`, `crates/una-interp/src/machine.rs`; CLI test `traces_never_show_labeled_arguments`, tag `DECISION D-107`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 3
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-108: Runtime faults before tier 1 (M7)
- Question: Until M8 verifies proof obligations, `una run` and `una test` run programs whose divisors are not proven nonzero (ADR 0027). What does a division by zero do, and what reports a fault that a checked program cannot reach?
- Context: `spec/11 §4`, `spec/12 §3`, `spec/13 §4`; ADR 0027; ADR 0032 rule 11.
- Options: A) the run aborts with U8001 at the name of the function in progress, or at 1:1 of the package root (ADR 0022), and exit code 3; B) a new runtime code for division by zero.
- Chosen: A, because once tier 1 runs first, reaching such a fault is a compiler bug, and a new code needs the specification (`CLAUDE.md`); until then the message says that a proof obligation excludes it.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d81f200`, `c3d2041`, `crates/una-interp/src/machine.rs`, `crates/una-cli/src/runtime.rs`, tag `DECISION D-108`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 11
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-109: The entry of `una run`, and U3012 at startup (M7)
- Question: `spec/04 §8` runs `main` of the module that `una run` names and reports ungranted capabilities with U3012 before execution, but `una check` (D-076) only compares `main` with `[run] grants`, which `--grant` flags extend, and a single file has no `una.toml`.
- Context: `spec/04 §8`, `spec/05 §7`, `spec/13 §5`; ADR 0021; ADR 0032 rule 10.
- Options: A) `una run` checks the package with the grants of `[run] grants` and `--grant` together as final grants, even when there are none, and U3012 then says that neither grants the operation; the entry is `[run] entry`, or the module of a single file; a package without `[run] entry`, an entry without `pub main() -> Unit`, a grant flag that is not a grant spec and an unreadable key file are usage errors (exit code 2); B) keep D-076 and check the flags at run time without a source position.
- Chosen: A, because the checker already locates U3012 on the `uses` clause, and conformance cases are single files that `una run` must run.
- Confidence: medium
- Reversal cost: low
- Implemented in: `c3d2041`, `crates/una-check/src/names/mod.rs`, `types/authority.rs`, `crates/una-cli/src/lib.rs`, `runtime.rs`, tag `DECISION D-109`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 10
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-112: What is verified, and when (M8)
- Question: `spec/12 §1` makes tier 1 "tier 0 plus every proof obligation", and `spec/08 §2` rule 4 verifies examples, but nothing says whether tier 1 runs when tier 0 fails, or whether the standard library's bodies are verified.
- Context: `spec/08 §2`, `spec/12 §1`, `spec/14` preamble (the Trusted zone); ADR 0033 rule 2.
- Options: A) functions, handler operations, opaque predicate bodies, and the `do` and `expect` lines of examples (an `expect` line with `result` the value of one of the `do` line's paths), once tier 0 reports nothing; the standard library trusted; B) verify the declarations without diagnostics even when others have some.
- Chosen: A, because a declaration with errors has no Core IR, so obligations that depend on its contract would be refuted for the wrong reason; tier 0 errors come first either way.
- Confidence: medium
- Reversal cost: low
- Implemented in: `ec853fa`, `a715ff0`, `crates/una-cli/src/verification.rs`, `crates/una-verify/src/generate.rs`, tag `DECISION D-112`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 2
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-114: Ground facts beyond the hypothesis table (M8)
- Question: `spec/12 §3` lists the hypotheses of a path context and `§4` the ground facts for `len` and `tag`, but a record's field refinements, which construction obligations guarantee, and the shape of constructed values are not listed; the example listing of `spec/13 §2` shows `0 <= e.remaining` for a parameter `e: Event`.
- Context: `spec/12 §2 to §4`, `spec/13 §2`; ADR 0033 rule 4.
- Options: A) for every term in a query, the refinements of record fields (under the variant's tag for a sum); constructor tag and field equations, list and text literal lengths; B) the table only.
- Chosen: A, because every added fact is guaranteed by an obligation proven elsewhere or by evaluation, and without them `spec/13 §2`'s own example could not be proven.
- Confidence: medium
- Reversal cost: low
- Implemented in: `49c73e9`, `048cbfa`, `crates/una-verify/src/generate.rs` (`ground_facts`), tag `DECISION D-114`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 4
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-118: Goals, hypotheses and ids as text (M8)
- Question: `spec/12 §8` hashes a "normalized goal text", and `spec/13 §1, §2` show goals and hypotheses as text, without saying how terms print or what `source` is.
- Context: `spec/12 §8`, `spec/13 §1, §2`; ADR 0033 rule 8.
- Options: A) terms print in the surface syntax with `it` replaced by the value's source text, normalized by collapsing white space; `source` is the clause of a return obligation and the site otherwise; hypotheses are the path context at the site, or at entry for return obligations; B) goals as SMT-LIB text.
- Chosen: A, because agents read and write the surface syntax, and the id then survives edits that do not change the goal.
- Confidence: medium
- Reversal cost: low
- Implemented in: `49c73e9`, `02bb2d5`, `048cbfa`, `crates/una-verify/src/generate.rs`, `crates/una-core/src/print.rs`, tag `DECISION D-118`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 8
- Answer needed: Keep A? Or switch to B? And a question on `spec/12 §8` itself: two obligations of the same kind and goal in one declaration, such as two calls of `lend` in the example package later replaced under R-022 (`can_borrow(member)`), get one id from its formula; `una obligations` lists both with their positions. Keep the formula, or add an occurrence number?
- Answer: kept A, and the formula of `spec/12 §8`, under which such obligations share an id (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-119: What a counterexample shows (M8)
- Question: `spec/12 §7` maps a model to source names and shows uninterpreted values "as opaque identities with their known projections", without naming which names appear or the form of an identity.
- Context: `spec/12 §7`, `spec/13 §1`; ADR 0033 rule 9.
- Options: A) parameters, `result`, named `let`s and pattern variables, and the field and `len` terms over them in the query, by name; a sum shows its variant, a text equal to a literal of the query that literal, another value `<Kind>#<n>`; B) every constant of the model, temporaries included.
- Chosen: A, because temporaries have no source name, and the projections are what the goal talks about.
- Confidence: medium
- Reversal cost: low
- Implemented in: `49c73e9`, `048cbfa`, `crates/una-verify/src/run.rs`, `generate.rs` (`shown`), tag `DECISION D-119`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 9
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-120: The repairs proposed for each kind (M8)
- Question: `spec/13 §3` lists four candidates "when applicable", without saying when each applies.
- Context: `spec/13 §3`; ADR 0033 rule 10.
- Options: A) refinement and precondition: a stronger parameter type when the value is a parameter, a `requires` clause for a private function, a guard, `checked`; postcondition: a guard or a changed `ensures`; division: a guard; exhaustive: an `else` arm; all without edits; B) only mechanical edits.
- Chosen: A, because each candidate follows the table's order and cost, and edits for them would guess the author's intent.
- Confidence: medium
- Reversal cost: low
- Implemented in: `49c73e9`, `crates/una-verify/src/report.rs`, tag `DECISION D-120`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 10
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-121: The output of `una obligations` (M8)
- Question: `spec/13 §2` defines `una.obligations/1`, but not the human output or the exit code.
- Context: `spec/13 §2, §5`; ADR 0033 rule 11.
- Options: A) one line per obligation with status, id, kind, position and goal, `(cached)` when cached; exit code 1 when an obligation is not proven or tier 0 reports diagnostics; `domain` in the JSON is `semantic`, the only one `spec/13 §2` shows; B) exit code 0 whenever the listing prints.
- Chosen: A, because the exit codes of `spec/13 §5` mean errors in the program, which an unproven obligation is.
- Confidence: medium
- Reversal cost: low
- Implemented in: `ec853fa`, `crates/una-cli/src/verification.rs`, tag `DECISION D-121`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 11
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-122: What the build report lists (M9)
- Question: `spec/13 §6` shows one `una.report/1` without saying which functions, policy calls, escapes, capabilities and host capabilities its sections list, how the lock is hashed, or how a single file names its package.
- Context: `spec/13 §6`, `spec/07 §5` rule 5, `spec/07 §6`, `spec/06 §6`; ADR 0034 rule 1.
- Options: A) every function with `uses`, every policy call, every module that uses `checked`, every declared capability with its `ensures`, the host capabilities the package uses, the SHA-256 of the lock's contents, a single file named after its module with version `0.0.0`; the report after a clean tier 0, exit code 1 with unproven obligations; B) only public functions and only the capabilities `main` uses.
- Chosen: A, because the report is the reviewer's view of the whole package's authority, trust and assumptions; B would hide private code with effects.
- Confidence: medium
- Reversal cost: low
- Implemented in: `92f6b9b`, `crates/una-cli/src/report.rs`, tag `DECISION D-122`
- ADR: `docs/adr/0034-report-and-abi-details-left-open.md` (Accepted), rule 1
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-123: Trivial `ensures` and the coverage counts (M9)
- Question: `spec/13 §6` counts "ensures clauses the solver proves valid without any hypothesis" and reports them as missing contracts, without saying which clauses or how coverage counts functions.
- Context: `spec/13 §6`, `spec/12 §5`; ADR 0034 rule 2.
- Options: A) one query per `ensures` clause of a public function, with its parameters and `result` unconstrained; `with_ensures` counts public functions with a clause that is not trivial; B) detect `ensures true` syntactically.
- Chosen: A, because the solver catches every valid clause (`result == result`, `x >= x`), and a function whose only clauses are trivial has no contract.
- Confidence: medium
- Reversal cost: low
- Implemented in: `92f6b9b`, `crates/una-cli/src/report.rs` and `crates/una-verify/src/generate.rs`, tag `DECISION D-123`
- ADR: `docs/adr/0034-report-and-abi-details-left-open.md` (Accepted), rule 2
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-124: The output of `una trace` (M9)
- Question: `spec/08 §1` says `una trace` "lists every function deriving from the requirement, its contract clauses and the verification status of each obligation", without a shape or exit codes.
- Context: `spec/08 §1`, `spec/13 §5`; ADR 0029; ADR 0034 rule 3.
- Options: A) the package in the current directory at tier 1, as `una contract`; the requirement, then each derived function with its clauses and obligations; `una.trace/1` with `--json`; an unknown identifier is a usage error; B) a path argument.
- Chosen: A, because `spec/13 §5` gives the command no path, as for `una contract`.
- Confidence: medium
- Reversal cost: low
- Implemented in: `92f6b9b`, `crates/una-cli/src/trace.rs`, tag `DECISION D-124`
- ADR: `docs/adr/0034-report-and-abi-details-left-open.md` (Accepted), rule 3
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-125: `--json` for every command (M9)
- Question: `spec/13 §5` makes `--json` a global flag, but `una explain` has answered it with a usage error since M1, and commands differ on what they print for a package that does not check.
- Context: `spec/13 §4, §5`, ADR 0012, ADR 0015 D1, D-010; ADR 0034 rule 4.
- Options: A) `una.explain/1` for `una explain --json`; the diagnostic list for `contract`, `trace`, `obligations` and `report` on a rejected package, the diagnostics inside the documents of `run` and `test`; B) keep the usage error of `una explain --json`.
- Chosen: A, because an agent can then drive every command headlessly (`plan/milestones-v0.1.md` M10), and each command prints exactly one document. The M1 test that expected the usage error now expects the document.
- Confidence: medium
- Reversal cost: low
- Implemented in: `d57857b`, `crates/una-cli/src/lib.rs`, tag `DECISION D-125`
- ADR: `docs/adr/0034-report-and-abi-details-left-open.md` (Accepted), rule 4
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-002: Title of the historical design exploration in the charter (Part A)
- Question: The charter supersedes the design exploration "Vera Language Specification v0.3". Should that historical title keep the codename?
- Context: Part A.2 and A.3 (Vera stays only in `reviews/triage.md §5` and ADR 0016); A.5 (mechanical edits only); charter status line.
- Options: A) rename it mechanically to "Troth Language Specification v0.3"; B) keep "Vera" there as a historical title, a third allowed place.
- Chosen: A, because A.3 lists the only places where "Vera" may remain, and the design exploration belongs to this same project, so renaming it follows the codename rule rather than falsifying another project's name.
- Confidence: medium
- Reversal cost: low, one line of `docs/00-charter.md`
- Implemented in: `c198b65`, `docs/00-charter.md`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-010: Usage errors with `--json` (M1)
- Question: ADR 0015 D1 keeps stderr empty with `--json` except for internal errors. Where does a usage error such as a missing path go when `--json` is given?
- Context: `spec/13 §5` (exit code 2), ADR 0015 D1 and D3.
- Options: A) message on stderr, nothing on stdout, exit 2; B) nothing at all, exit 2; C) a JSON document on stdout describing the error.
- Chosen: A, because the usage error happens before the command runs, clap already reports its own usage errors this way, and no diagnostic code exists for a missing path to put in a JSON document. D1 governs the documents that commands produce.
- Confidence: medium
- Reversal cost: low, `check_command` in `crates/una-cli/src/lib.rs`
- Implemented in: M1 CLI commit (`9d2b95f`), `crates/una-cli/src/lib.rs`, `crates/una-cli/tests/cli.rs`, tag `DECISION D-010`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-014: A keyword after `@` (M1)
- Question: `cap_ref` is `@` followed by a `lower_ident` (`spec/01 §4`), and keywords are not identifiers. What does `@if` produce?
- Context: `spec/01 §4, §5`; ADR 0017.
- Options: A) U0101 "keyword `if` cannot name a capability", keeping the reference for recovery; B) accept it as a capability named `if`; C) U0001 at `@`.
- Chosen: A, because the program is ill-formed at the token level and U0101 is the code for tokens the grammar rejects, while the character `@` itself is valid (B is lenient, C blames the wrong character).
- Confidence: medium
- Reversal cost: low, `capability_reference` in `crates/una-syntax/src/lexer.rs`
- Implemented in: M1 lexer commit (`a7313bf`), `crates/una-syntax/src/lexer.rs`, `crates/una-syntax/tests/lexer.rs`, tag `DECISION D-014`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-016: Letters right after digits (M1)
- Question: What does `12abc` lex as? ADR 0017 lists the integer malformations U0007 covers (leading zero, doubled, leading or trailing underscore); letters after digits are not among them.
- Context: `spec/01 §6`; ADR 0017 decision 1.
- Options: A) an integer `12` then a name `abc`, so the parser reports U0101 at `abc`; B) U0007 "malformed integer literal `12abc`".
- Chosen: A, because ADR 0017 enumerates what U0007 covers for integers, and A still rejects the program with a precise position.
- Confidence: medium
- Reversal cost: low, `integer` in `crates/una-syntax/src/lexer.rs`
- Implemented in: M1 lexer commit (`a7313bf`), `crates/una-syntax/src/lexer.rs`, `crates/una-syntax/tests/lexer.rs`, tag `DECISION D-016`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B (extend U0007)?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-017: Width of a tab when choosing the recovery level (M1)
- Question: A line whose indentation contains a tab reports U0002 and continues at "the nearest valid level" (approved M1 plan). How wide is the tab for that choice?
- Context: `spec/01 §2`; approved M1 plan §6 (recovery rules).
- Options: A) two columns, one level; B) four; C) eight.
- Chosen: A, because a tab most often stands for one level in indentation-sensitive code; the choice only affects recovery after the error is reported.
- Confidence: medium
- Reversal cost: low, `indentation` in `crates/una-syntax/src/lexer.rs`
- Implemented in: M1 lexer commit (`a7313bf`), `crates/una-syntax/src/lexer.rs`, `crates/una-syntax/tests/lexer.rs`, tag `DECISION D-017`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-127: The example packages and what "every construct" means (M10)
- Question: M10 asks for at least five more example packages "covering every construct in the charter scope", without naming them or saying how coverage is shown.
- Context: `plan/milestones-v0.1.md` M10; charter §5; `plan/roadmap.md §3`; ADR 0026 (neutral examples).
- Options: A) five packages with domain-neutral themes (the example package later replaced under R-022, `pantry`, `journal`, `greenhouse`, `shortlinks`), each passing fmt, tier 1, `una test` and the report with no trivial `ensures`, and a test that finds every construct of charter §5's language list in some example by a source pattern; B) one small package per charter bullet; C) the same five packages with a coverage table in the plan instead of a test.
- Chosen: A, because the examples are the code evaluation tasks start from (roadmap §3), so each should be a small program rather than a fragment, and a test keeps the coverage claim true as the examples change.
- Confidence: medium
- Reversal cost: low, the packages under `examples/` and the construct list of `crates/una-cli/tests/examples.rs`
- Implemented in: `05c174f`, `cc9cca2`, `e13533e`, `5f5955c`, `bf640f2`, `examples/`, `crates/una-cli/tests/examples.rs`, tag `DECISION D-127`
- ADR: `docs/adr/0035-examples-guide-and-release.md` (Accepted), rule 1
- Answer needed: Keep A? Or switch to B or C, or change a theme?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-128: How the guide's snippets are tested (M10)
- Question: "Every snippet is tested", but M10 does not say what a snippet is or what its test asserts.
- Context: `plan/milestones-v0.1.md` M10; `spec/13 §4` (explain examples, which D-043 tests the same way); `spec/13 §5`.
- Options: A) every `una` fence is a whole module that checks clean at tier 1, or yields only the code named by a first line `-- fails with U####`; fences after file paths form a package that checks clean; `json` fences parse and name a schema the CLI prints; every `una` line of a `sh` fence runs from the repository root, or the directory of its `# in <dir>`, and exits with 0 or its `# exit N`; B) `una` fences only, all clean; C) fragments completed by a template module.
- Chosen: A, because agents copy snippets verbatim, so each must check on its own; failing snippets show the codes agents will meet; and commands and documents are the ABI usage the guide teaches.
- Confidence: medium
- Reversal cost: low, `crates/una-cli/tests/guide.rs` and the fences of the guide
- Implemented in: `877b6bd`, `docs/guide/una-for-agents.md`, `crates/una-cli/tests/guide.rs`, tag `DECISION D-128`
- ADR: `docs/adr/0035-examples-guide-and-release.md` (Accepted), rule 2
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-129: Release artifacts (M10)
- Question: M10 asks for release binaries for Linux and macOS built in CI, without naming targets, contents or workflows, while D-083 runs macOS only on pull requests and on `main`.
- Context: `plan/milestones-v0.1.md` M10; D-083; `compiler/architecture.md §7`; `OVERNIGHT.md` C.5.
- Options: A) `scripts/release.sh <target>` packs `una`, `LICENSE` and `README.md` into `una-<version>-<target>.tar.gz` with a `.sha256` file, and `scripts/check-release.sh <target>` checks the digest, the contents and the packed binary on the examples; `release.yml` builds `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin` on a pushed tag `v*` into a draft release; `ci.yml` packs and checks the Linux archive on every push; B) both archives on every push; C) a third-party release tool.
- Chosen: A, because it proves on every push that the archive builds and runs, keeps macOS within D-083, and needs no tool beyond cargo, tar and the preinstalled `gh`.
- Confidence: medium
- Reversal cost: low, `scripts/release.sh` and the two workflows
- Implemented in: `9b23b9f`, `scripts/release.sh`, `scripts/check-release.sh`, `.github/workflows/release.yml` and `ci.yml`, tag `DECISION D-129`
- ADR: `docs/adr/0035-examples-guide-and-release.md` (Accepted), rule 3
- Answer needed: Keep A? Or add targets, or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-131: What an `attempt` that succeeded tells the verifier (M10)
- Question: `spec/12 §3` adds the callee's `ensures` after `let x = f(ā)`, but says nothing of `let x = attempt f(ā)`, so a program that attempts a call lost what the callee proves about its result.
- Context: `spec/12 §3`; `spec/06 §5` (`attempt` returns `Result`); `spec/04 §2` rule 2 (`ensures` describes normal returns); found by the example package later replaced under R-022 in M10.
- Options: A) the hypothesis `x is Ok implies P`, with `P` one of the attempted call's normal return paths, its facts and `ensures`, and `x.value` its value; nothing for `Err`; B) no hypothesis for `attempt`, so contracts cannot be used through it; C) A for `let` only.
- Chosen: A, because `Ok` is exactly a normal return of the call, so the hypothesis is true in every run and weakens nothing, while B pushes programs away from handling failures; the same rule serves every place the value flows (a `let`, a return, an example's `do` line).
- Confidence: medium
- Reversal cost: low, `attempted` and `path` in `crates/una-verify/src/generate.rs`
- Implemented in: `de9ab73`, `crates/una-verify/src/generate.rs`, tag `DECISION D-131`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 12
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-132: Where the build report is assembled (M9)
- Question: `compiler/architecture.md §2` gives build report assembly to `una-report`, but the report needs the loaded package, the syntax trees, the analysis and the solver settings, which live in `una-cli`, the crate that depends on all the others.
- Context: `compiler/architecture.md §2, §3`; `spec/13 §6`; ADR 0034.
- Options: A) assemble the report in `una-cli` (`src/report.rs`) and keep `una-report` as a reserved stub; B) move the package and analysis types into a library crate so that `una-report` can assemble it; C) have `una-cli` pass `una-report` plain inputs.
- Chosen: A, because the report reads the same package and analysis as `una check`, `una contract` and `una trace`, which `una-cli` already holds; B and C would move shared types for one command. The architecture document now says so. Found while writing the readme.
- Confidence: medium
- Reversal cost: low, `crates/una-cli/src/report.rs` moves behind a function of `una-report`
- Implemented in: `92f6b9b`, `crates/una-cli/src/report.rs`, tag `DECISION D-132`
- ADR: `docs/adr/0034-report-and-abi-details-left-open.md` (Accepted), rule 1
- Answer needed: Keep A? Or switch to B or C?
- Answer: kept A, with `compiler/architecture.md` updated (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-001: The abstract machine acronym follows the rename (Part A)
- Question: "Vera Abstract Machine (VAM)" appears in the spec, ADR 0002, the plan and the backlog. The rename table covers the language name but not the acronym.
- Context: Part A.2; `spec/11`, ADR 0002, `plan/milestones-v0.1.md` M7, `plan/backlog.md`, `plan/roadmap.md`.
- Options: A) rename to "Troth Abstract Machine (TAM)" everywhere; B) rename the expansion but keep "VAM".
- Chosen: A, because an acronym that no longer matches its expansion would read as a leftover of the codename and confuse every reader of `spec/11`.
- Confidence: high
- Reversal cost: low, a text replacement in seven places
- Implemented in: `c198b65`, `docs/spec/11-dynamic-semantics.md`, `docs/adr/0002-interpreter-first.md`, `docs/plan/*.md`, recorded in ADR 0016
- ADR: `docs/adr/0016-final-name-troth.md` (Accepted, mentions it as a derived name)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-013: Tabs leading lines without layout (M1)
- Question: Is a tab that leads a comment-only line, or a line joined inside brackets, U0002? Such lines produce no layout tokens.
- Context: `spec/01 §2` rule 1 ("A tab in leading whitespace is an error") and rule 4 (indentation is ignored inside brackets).
- Options: A) U0002 for every line with a leading tab, without affecting layout; B) U0002 only for lines that produce layout.
- Chosen: A, because rule 1 applies to every line and rule 4 only says layout ignores the indentation; A is also stricter and keeps one canonical form (P4). Whitespace-only lines are left to U7001 (trailing whitespace, M2).
- Confidence: high
- Reversal cost: low, `tab_outside_layout` in `crates/una-syntax/src/lexer.rs`
- Implemented in: M1 lexer commit (`a7313bf`), `crates/una-syntax/src/lexer.rs`, `crates/una-syntax/tests/lexer.rs`, tag `DECISION D-013`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-015: A generic function start is a declaration start (M1)
- Question: Recovery resumes "at the next line with zero indentation that starts a declaration keyword, `pub`, or a lower identifier followed by `(`" (`compiler/architecture.md §5`). A generic function starts with `name[T](`.
- Context: `compiler/architecture.md §5`; `spec/02 §3` (`signature` with `type_params`).
- Options: A) also accept a lower identifier followed by `[`; B) accept only `(`.
- Chosen: A, because otherwise a bracket left open before a generic function would swallow it and hide its diagnostics.
- Confidence: high
- Reversal cost: low, `starts_declaration` in `crates/una-syntax/src/lexer.rs` (shared with the parser)
- Implemented in: M1 lexer commit (`a7313bf`), `crates/una-syntax/src/lexer.rs`, `crates/una-syntax/tests/lexer.rs`, tag `DECISION D-015`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-020: Handling clause order is checked by the M1 parser (M1)
- Question: `spec/06 §3` says each handling clause appears at most once per statement, `if error` before `if none` (U0102). The approved M1 plan listed U0102 only for function and operation clauses. Is the handling rule checked in M1, and does "each clause" mean each of the five forms or each of `if error` and `if none`?
- Context: `spec/06 §3`; `spec/02 §6`; ADR 0017 decision 5; approved M1 plan §2.
- Options: A) check it in the M1 parser with the other U0102 rules, allowing one `if error` and one `if none` clause; B) defer it to M6 with the rest of `spec/06`; C) allow one clause of each of the five forms.
- Chosen: A, because U0102 is a syntax code and the rule is purely syntactic, so all U0102 checks live in one place with one message format; C would allow two clauses that both handle the same error. Two conformance cases were added, and the U0102 explain text mentions handling clauses.
- Confidence: high
- Reversal cost: low, `HANDLING_CLAUSES` in `crates/una-syntax/src/parser.rs`
- Implemented in: M1 parser commit (`d95fe43`), `crates/una-syntax/src/parser.rs`, `crates/una-syntax/tests/parser.rs`, `tests/conformance/syntax/handling_clause_out_of_order.una`, `tests/conformance/syntax/repeated_if_error.una`, tag `DECISION D-020`; M1 explain commit (`b1039e3`), `crates/una-diag/explain/U0102.md`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-026: Constructor order for kinds declared in the same file (M2)
- Question: `spec/04 §3.4` has the formatter order constructor arguments by declaration, which needs the declaration. Imported kinds need name resolution, which arrives in M3.
- Context: `spec/04 §3.4`; `plan/milestones-v0.1.md` M2 and M3; ADR 0020 rule 6.
- Options: A) in M2, order constructors of kinds and variants declared in the same file whose name is declared once and not imported; add imported ones in M3; B) wait for M3 entirely.
- Chosen: A, because it delivers the rule where M2 can apply it exactly and needs no guess about imports. The examples already use declaration order everywhere, so M3 will not make them non-canonical.
- Confidence: high
- Reversal cost: low, `constructors` in `crates/una-fmt/src/rewrite.rs`
- Implemented in: M2 formatter commit (`697cd2e`), `crates/una-fmt/src/rewrite.rs`, `crates/una-fmt/tests/format.rs`; the imported kinds, which M3 left out, in `c99e638` after the owner's review, with `Constructors` and a conformance case; tag `DECISION D-026`
- ADR: `docs/adr/0020-canonical-form.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-027: Files with syntax errors have no canonical form (M2)
- Question: Does `una check` report U7001, and does `una fmt` rewrite, a file that has syntax errors?
- Context: `compiler/architecture.md §3` (U7001 at tier 0, after parsing); ADR 0020.
- Options: A) no: U7001 only for files without syntax errors, and `una fmt` leaves the others untouched; B) format what parsed and report U7001 anyway.
- Chosen: A, because formatting a partial tree could delete code, and U7001 on a broken file would repeat what the syntax errors already say.
- Confidence: high
- Reversal cost: low, `check_file` in `crates/una-cli/src/lib.rs`
- Implemented in: M2 CLI commit (`acf4ec2`), `crates/una-cli/src/lib.rs`, tag `DECISION D-027`
- ADR: `docs/adr/0020-canonical-form.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-031: `.una` files are checked out with line feeds (M2)
- Question: With `core.autocrlf`, a Windows checkout would give every `.una` file `\r\n` line breaks, which U7001 rejects.
- Context: ADR 0020 rule 1; `.gitattributes`.
- Options: A) `*.una text eol=lf`, with byte-exact exceptions for the `syntax` and `fmt` conformance areas; B) leave it to each checkout.
- Chosen: A, because the canonical form is part of the language and must not depend on the checkout.
- Confidence: high
- Reversal cost: low, `.gitattributes`
- Implemented in: M2 source commit (`95c1ef3`), `.gitattributes`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-032: A directory is a package only with `una.toml` and `src/` (M3)
- Question: ADR 0015 D3 (interim loading) expires in M3. What does a directory without `una.toml` or `src/` mean now?
- Context: `spec/13 §7`; ADR 0015 D3; ADR 0021 rule 1.
- Options: A) a usage error (exit 2) for either; B) check `src/` with default settings when `una.toml` is missing.
- Chosen: A, because a package's settings (labels, trusted modules, budgets) live in `una.toml`, and B would check a package silently without them. Two CLI tests that built `src/` without `una.toml` now also write one, as ADR 0015 D3 foresaw.
- Confidence: high
- Reversal cost: low, `discover` in `crates/una-project/src/package.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-project/src/package.rs`, `crates/una-cli/tests/cli.rs`, tag `DECISION D-032`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-034: The values of `verification.lock`, and the default lock (M3)
- Question: `spec/12 §9` lists six keys. Which values are valid, and which lock does a single file use?
- Context: `spec/12 §9`; `compiler/testing.md §3`; ADR 0021 rule 3.
- Options: A) exactly the six keys, with `encoding_version = "una-smt-1"`, `solver = "z3"`, `logic = "QF_UFLIA"`, a non-empty `solver_version`, `step_budget` of at least 1 and `random_seed` of at least 0; a single file uses the repository's values; B) check only the syntax.
- Chosen: A, because the compiler implements one encoding, one solver and one logic, so another value cannot be honored; the installed solver's version is U5003 in M8.
- Confidence: high
- Reversal cost: low, `crates/una-project/src/lock.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-project/src/lock.rs`, tag `DECISION D-034`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-035: Project errors do not stop checking (M3)
- Question: When `una.toml` or `verification.lock` is invalid, are the modules still checked?
- Context: `compiler/architecture.md §3` ("one run reports as many independent problems as possible"); ADR 0021 rule 4.
- Options: A) yes, with the default settings in place of an invalid `una.toml`; B) stop after the project errors.
- Chosen: A, because the modules do not depend on the lock until M8, and default settings are the strictest (no trusted modules, a budget of 0).
- Confidence: high
- Reversal cost: low, `check` in `crates/una-cli/src/lib.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-cli/src/lib.rs`, tag `DECISION D-035`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-040: How U1003 and U1004 are reported (M3)
- Question: Where does an import cycle point, and what repair does a private import get?
- Context: `spec/13 §1` (repairs); ADR 0023 rule 5.
- Options: A) one U1003 per cycle, at the first import of the cycle's first module in path order, naming every module of the cycle; U1004 at the imported name with an edit adding `pub` to the declaration in the other file; B) one U1003 per import of the cycle.
- Chosen: A, because one cycle is one problem, and the `pub` edit is mechanical.
- Confidence: high
- Reversal cost: low, `imports` in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-040`
- ADR: `docs/adr/0023-name-resolution.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-041: Modules with syntax errors are not resolved (M3)
- Question: How do name checks treat a module whose parse lost declarations?
- Context: `compiler/architecture.md §3, §5`; ADR 0023 rule 6.
- Options: A) register the declarations that parsed and check them for U1002, U1005 and U1008, but resolve no body and check no import from that module; B) resolve everything, reporting what cannot be found.
- Chosen: A, because a name defined by a declaration that failed to parse would otherwise be reported as unresolved everywhere it is used.
- Confidence: high
- Reversal cost: low, the syntax-error flag of each module in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-041`
- ADR: `docs/adr/0023-name-resolution.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-042: Naming conventions for type parameters, owners and module paths (M3)
- Question: `spec/01 §4` lists the conventions of kinds, variants, capabilities, categories, lower-case names and requirement identifiers, but not of type parameters, owners or module path segments.
- Context: `spec/01 §4`, `spec/08 §1` (owners are "lower snake case"); ADR 0023 rule 7.
- Options: A) type parameters are UpperCamelCase, owners and path segments follow the lower-case rule, all checked where defined; B) check only the listed names.
- Chosen: A, because each of these names already has the shape of one of the listed classes, and checking it is the stricter option.
- Confidence: high
- Reversal cost: low, `conventions.rs` in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/conventions.rs`, tag `DECISION D-042`
- ADR: `docs/adr/0023-name-resolution.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-044: Pending cases cannot hide implemented codes (M3)
- Question: A pending case (for example on M5) may start reporting codes of M3 by mistake; the runner would still classify it as pending.
- Context: ADR 0013; `compiler/testing.md §2` (pending cases); the M1 runner rule for syntax codes.
- Options: A) a case fails when it reports an implemented code it does not expect, pending or not; a code is implemented when its explain text is no longer a stub; B) keep the rule for syntax codes only.
- Chosen: A, because every finished milestone then protects its codes in every case, with no list to maintain.
- Confidence: high
- Reversal cost: low, `crates/una-cli/tests/conformance/main.rs`
- Implemented in: M3 runner commit (`e3ab28f`), `crates/una-cli/tests/conformance/main.rs`, tag `DECISION D-044`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-045: A single file with a `trusted module` header (M3)
- Question: `spec/07 §6` requires a trusted module to be declared in its header and in `una.toml`. A single file has no `una.toml`.
- Context: `spec/07 §6`; `compiler/testing.md §2` (default settings); ADR 0021.
- Options: A) U7005, since the default settings trust no module; B) trust the file when its header says so.
- Chosen: A, because the double declaration exists to make trust deliberate at the package level, and a single file has no second declaration. Conformance cases about trusted modules are packages.
- Confidence: high
- Reversal cost: low, the trusted check in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), tag `DECISION D-045`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-047: Typing capability operations before M5 (M4)
- Question: M4 types every expression, but capability contexts belong to M5. How is `@c.op(args)` typed in M4?
- Context: `spec/05 §3, §4`, `spec/10 §5` (T-Op); `plan/milestones-v0.1.md` M4 and M5.
- Options: A) with the operation of the capability bound to `@c` by the function's `uses`, a `with`, a `given` or a handler's `uses`; the error type when nothing binds it, so M5 reports U3001; B) type every operation call as the error type until M5.
- Chosen: A, because the reference example must type its bodies in M4, and the binding rules are the ones M5 will check; M5 adds sufficiency and operation permissions.
- Confidence: high
- Reversal cost: low, the capability context of the type checker
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-047`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-049: What `fail` joins with, and how branches combine (M4)
- Question: `fail` ends the function; `spec/10 §5` gives it any type τ. How do `if` and `when` branches combine when one fails or when their refinements differ?
- Context: `spec/10 §4, §5` (T-If, T-Case, T-Fail); ADR 0024 rule 2.
- Options: A) `fail` has a bottom type that joins anything; other branches share one base type, and a known target type is checked in each branch, with refinements left to obligations; B) require every branch to have exactly the same type, refinements included.
- Chosen: A, because it is what T-Fail's free τ means, and refinements are the verifier's concern (`spec/03 §2.2`).
- Confidence: high
- Reversal cost: low, the join in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-049`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-052: Resolving `e.f` and `e.f(args)` (M4)
- Question: `spec/04 §3` resolves `e.f(args)` as a field of function type or a function `f` in scope, and `e.f` as a field or a one-parameter function. What if neither exists, and in which order?
- Context: `spec/04 §3`; ADR 0024 rule 5.
- Options: A) `.len` on `Text` and `List`, then a field, then a function or predicate declared in scope (locals are not searched); U1001 when nothing matches, with an edit to the closest field; a receiver whose type is not known yet is D-069; B) report U2001 for an unknown field.
- Chosen: A, because the order follows the specification, and a name that matches nothing is unresolved.
- Confidence: high
- Reversal cost: low, field and method resolution in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-052`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-053: Calls of function values and constructors as values (M4)
- Question: Function types carry no parameter names, and the Core IR of `spec/09` has no constructor values. Can a function value be called with labels, and can `Some` be passed as a function?
- Context: `spec/03 §1`, `spec/04 §3`, `spec/09 §2`; ADR 0024 rules 6 and 7.
- Options: A) function values take positional arguments (a label is U2005, with an edit that removes it), and so does a policy call (D-066); a constructor with fields must be applied (U2009 otherwise); B) allow both.
- Chosen: A, because labels need parameter names that function types lack, and constructor values would need a Core form that does not exist. Both can be relaxed later.
- Confidence: high
- Reversal cost: low, calls in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-053`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-054: A type that inference leaves unknown (M4)
- Question: An empty list that nothing constrains, or a generic call whose parameter nothing fixes, leaves a unification variable unsolved. Core IR needs every type explicit (`spec/09 §1`).
- Context: `spec/03 §7`, `spec/09 §1`; ADR 0024 rule 8.
- Options: A) U2001 with a hint to annotate, at the empty list or the generic use that introduced the type, reported only when the declaration has no other type diagnostic, since an earlier error often leaves the type unknown; B) default the type to `Unit`.
- Chosen: A, because a default hides a mistake, and an annotation always resolves it.
- Confidence: high
- Reversal cost: low, the end-of-declaration check in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-054`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-055: The prelude as Una sources (M4)
- Question: M4 needs the signatures and contracts of `spec/14 §1, §3, §4`. How are they written?
- Context: `spec/14`; `compiler/architecture.md §2`.
- Options: A) embedded Una sources `std/prelude.una` and `std/testing.una`, parsed with the ordinary parser, with placeholder bodies the checker never reads, and a test that keeps their names equal to `spec/14`; B) a table in Rust.
- Chosen: A, because signatures and contracts are then written in the language they describe and go through the same lowering as user code; the interpreter implements the bodies in M7.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/std/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-055`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-056: U2015 for opaque predicates before effect inference (M4)
- Question: `spec/03 §4` requires opaque predicate bodies to be pure and non-failing (U2015). Calls to effectful or failing functions need the effect inference of M5.
- Context: `spec/03 §4`, `spec/04 §5`.
- Options: A) M4 reports what the body and the declared headers show: capability operations, `fail`, `checked`, `if error fail`, `if error pass`, `if none fail`, and calls of functions and policies that declare `uses`, or that declare `fails` when no `attempt` or `if error` clause handles the failure; M5 adds inferred effects and failures of private functions; B) wait for M5 for all of U2015 on opaque bodies.
- Chosen: A, because M4 can report most violations now, and M5 completes the rule where it has the information. `attempt` alone is no violation: it handles the failure of its call.
- Confidence: high
- Reversal cost: low, the predicate checks in `crates/una-check/src/types/rl.rs`
- Implemented in: `0c07912` (M4 type checker), `Checker::impure` and `Checker::call_function`, tag `DECISION D-056`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-057: Labels are not compared by the type checker (M4)
- Question: Types carry labels (`spec/03 §5`), but label rules are M6. Does M4 compare them?
- Context: `spec/07`, `plan/milestones-v0.1.md` M4 and M6.
- Options: A) M4 compares bases only and keeps labels on types for M6; B) compare labels as part of type equality.
- Chosen: A, because label flow has its own rules (joins, pc, sinks) and codes (U4001 to U4007), and comparing them as equality would reject valid flows.
- Confidence: high
- Reversal cost: low
- Implemented in: `0c07912` (M4 type checker)
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-066: Policies: the `via` signature and the calls (M4)
- Question: `spec/10 §7` types a policy call `p(v)` as a declassification from `from` to `to`, computed by the `via` function. Nothing checks that the function takes `from` and returns `to`, or says how `p(v)` passes its argument.
- Context: `spec/07 §5`, `spec/09 §3`, `spec/10 §7`; ADR 0024 rule 14.
- Options: A) the `via` function takes one parameter of the base type of `from` and returns the base type of `to` (U2001 at `via` otherwise); a policy call takes one argument without a label (U2005 for a label) and fails and uses capabilities as its `via` function does; B) no check until M6.
- Chosen: A, because a mismatch would make the declassified value ill-typed at run time; labels stay with M6. Since M6 the mismatch is U4005, as `spec/07 §5` rule 4 requires (D-094).
- Confidence: high
- Reversal cost: low, `Checker::policy` and `Checker::call_policy`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-066`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted), rule 14
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-067: Wrong numbers of arguments (M4)
- Question: U2004 to U2006 cover labels. Which code reports too many arguments, or a parameter that receives none?
- Context: `spec/04 §3`, `spec/10 §5` (T-Call); ADR 0024 rule 15.
- Options: A) U2001: "takes N arguments, but M are given" at the call, or "is missing argument `x`", which is not reported when a label error already explains the call; B) U2004.
- Chosen: A, because a wrong count is a mismatch between the call and the callee's type, while U2004 is about labels.
- Confidence: high
- Reversal cost: low, `Checker::arguments` and `Checker::call_value`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-067`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted), rule 15
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-069: Values whose type is not known yet (M4)
- Question: With unification, a value's type can still be unknown where a field, a method or a call needs it, for example `fn(x) -> x.len` passed where the element type is not fixed yet.
- Context: `spec/03 §7`; ADR 0024 rule 8; D-054.
- Options: A) check lambda arguments after the other arguments of a call or constructor, so that labeled arguments in any order still fix lambda parameter types; where a type is still unknown, report U2001 asking for an annotation; B) accept the access silently with the error type.
- Chosen: A, because B would let a wrong field pass unreported, and checking lambdas last removes the common case.
- Confidence: high
- Reversal cost: low, `lambdas_last` in `crates/una-check/src/types/call.rs` and `Checker::unknown_type`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-069`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Accepted), rule 8
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-070: What M4 records of obligations and selfification (M4)
- Question: M4 records pending obligations (task 7) and selfified `let`s (task 3) before M7 elaborates to Core IR and M8 generates the obligations. What exactly is recorded?
- Context: `spec/03 §7`, `spec/12 §3`; `compiler/architecture.md §3`.
- Options: A) `check_types` returns the pending obligations of the kinds of `spec/12 §3`, with their primary span and the declared type they come from, sorted by position and then kind, and the spans of selfified `let`s; M8 generates the real obligations over Core IR; B) build hypotheses and goals now.
- Chosen: A, because formulas need Core IR (M7), and A is enough to test where each obligation arises.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/mod.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-070`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-068: Two tour cases broke the labeled-argument rule (M4)
- Question: `tests/conformance/syntax/grammar_tour.una` (M1) and `tests/conformance/names/resolution_tour.una` (M3) expect no diagnostics, but pass second arguments without labels (`map(xs, fn(...) -> ...)`, `fold(items, 0, fn(...) -> ...)`, `max(best, size(item))`), which `spec/04 §3` rule 2 makes U2004.
- Context: `spec/04 §3`; OVERNIGHT.md C.5 (expectations change only when the specification shows them wrong).
- Options: A) add the labels to the inputs; B) keep the inputs and expect U2004.
- Chosen: A, because the cases exist to tour valid programs, and the specification shows their inputs were wrong; only labels were added.
- Confidence: high
- Reversal cost: low
- Implemented in: `0c07912` (M4 type checker), the two case files
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-073: Where handler problems are reported (M5)
- Question: `spec/05 §5` requires every operation (U3011) with a matching parameter count and types from the capability (U3010). Where do these point, and what reports `for` naming something other than a capability?
- Context: `spec/05 §5`; ADR 0028 rule 3.
- Options: A) U3010 at `for` for a name that is not a capability, at an operation the capability lacks or with another parameter count; U3011 at the handler's name listing the missing operations; operation bodies fail only with the operation's error kind (U3009, U3008 for `pass`); B) one diagnostic per handler.
- Chosen: A, because each problem gets the site an agent edits. An operation body whose type differs from the operation's result type is U2001, as for any body, since the types come from the capability declaration.
- Confidence: high
- Reversal cost: low
- Implemented in: `9d34aab`, `crates/una-check/src/types/check.rs` (`handler_rules`), tag `DECISION D-073`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 3
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-074: Installing handlers with `with` and `given` (M5)
- Question: `with @x = h(args)` and `given @x = h(args)` must name a handler for the capability expected at `@x` (U3010), and the handler's own `uses` must be held by the caller (`spec/05 §4` rule 2), without a code.
- Context: `spec/02 §8`, `spec/05 §4`, `spec/08 §2`; ADR 0028 rule 4.
- Options: A) U3010 when `h` is not a handler or implements another capability than the one the statement's expression or the `do` line needs at `@x`; the handler's `uses` are checked like a callee's requirements (U3001, U3002); B) no check of the handler's `uses`.
- Chosen: A, because a handler with effects would otherwise act with authority its installer lacks. `given` lines install their handlers in order, so a handler's `uses` are held by the `given` lines before it, and the handlers of one `with` clause are installed in the context outside the statement.
- Confidence: high
- Reversal cost: low
- Implemented in: `9d34aab`, `crates/una-check/src/types/check.rs` (`install`), tag `DECISION D-074`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 4
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-077: Where `std.testing` may appear (M5)
- Question: `spec/14 §4` allows `std.testing` only in handler bodies and example lines (U3015). Is an import outside them an error, or only a use?
- Context: `spec/14 §4`; ADR 0028 rule 7.
- Options: A) a call of, or reference to, a `std.testing` function outside a handler operation body or an example is U3015, and imports are free; B) the import itself.
- Chosen: A, because modules mix handlers and functions, and the import serves the handlers.
- Confidence: high
- Reversal cost: low
- Implemented in: `9d34aab`, `crates/una-check/src/types/authority.rs` (`testing_site`), tag `DECISION D-077`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 7
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-078: Reporting the escape budget (M5)
- Question: `spec/06 §6` counts `checked` per module against `[escape_budget] checked_per_module` (U7003 when exceeded) without a site.
- Context: `spec/06 §6`, `spec/13 §7`; ADR 0028 rule 8.
- Options: A) the first use beyond the budget is U7003, with the later ones as related spans; the budget is 0 by default and for a single file; B) one diagnostic per use beyond the budget.
- Chosen: A, one diagnostic per module, which a change of the budget or of the code resolves.
- Confidence: high
- Reversal cost: low
- Implemented in: `9d34aab`, `crates/una-check/src/types/authority.rs` (`budget`), tag `DECISION D-078`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 8
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-080: Positions for one effect per line (M5)
- Question: `spec/04 §4` allows an effectful or fallible call only as the entire expression of a statement, `let` or final expression, "optionally wrapped in `attempt`". What about parentheses, `checked`, and function values?
- Context: `spec/04 §4`, `spec/08 §2` rule 5; ADR 0009; ADR 0028 rule 10.
- Options: A) parentheses are transparent; calls of functions, policies and function values with `uses` or `fails`, declared or inferred, and operations count; `checked` is not a call; example `do` lines are exempt; B) count only operations and named calls.
- Chosen: A, because a function value with effects is as effectful as a named function.
- Confidence: high
- Reversal cost: low
- Implemented in: `9d34aab`, `crates/una-check/src/types/authority.rs` (`position_site`), tag `DECISION D-080`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 10
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-082: Two checking passes for inference (M5)
- Question: Inferred headers need the call graph and the operations of every body, while typing the operations of a private function needs its inferred capabilities.
- Context: ADR 0007; `compiler/architecture.md §3`.
- Options: A) pass 1 runs the checker and collects authority facts, inference computes the headers, and pass 2 checks again with them and reports every diagnostic; B) infer syntactically before checking, resolving calls without types.
- Chosen: A, because method calls and function values need types to resolve, and the cost of a second pass is small.
- Confidence: high
- Reversal cost: low
- Implemented in: `9d34aab`, `crates/una-check/src/types/mod.rs`, `check.rs`, tag `DECISION D-082`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-084: The M4 unit tests leave out the codes of later milestones (M5)
- Question: `crates/una-check/tests/types.rs` compares every name and type diagnostic of its inputs, and M5 adds codes that some M4 inputs trigger: a single file that uses `checked` twice now also gets U7003, since the budget of a single file is 0.
- Context: `OVERNIGHT.md` C.5 (tests and expectations), `spec/06 §6`, `compiler/testing.md`.
- Options: A) the M4 helper `check()` leaves out the authority codes and U7003, which `crates/una-check/tests/authority.rs` covers; B) add the U7003 line to the M4 expectation, and the codes of each later milestone as they arrive; C) give the M4 inputs a manifest with a larger budget.
- Chosen: A, because each unit-test file then states the rules of its own milestone and no M4 expectation changes; the conformance suite still checks complete outputs. From M6 on, the helper also leaves out the security codes, which `crates/una-check/tests/flow.rs` covers; one M4 expectation, U2001 for a `via` function of another type, left the M4 test because `spec/07 §5` rule 4 makes it U4005 (D-094), which `flow.rs` tests.
- Confidence: high
- Reversal cost: low, the `later` filter in `crates/una-check/tests/types.rs`
- Implemented in: `9d34aab`, `crates/una-check/tests/types.rs`, tag `DECISION D-084`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-086: Mechanical edits of authority diagnostics (M5)
- Question: `spec/13 §3` lets diagnostics other than U5001 and U5002 include repairs. Which authority diagnostics carry a mechanical edit, and which edit?
- Context: `spec/13 §1, §3`, `spec/02 §3` (clause order); ADR 0028, Consequences.
- Options: A) edits only where one fix follows from the diagnostic: widen the declaring `uses` clause (U3002); add the callee's requirements as `uses` clauses of the caller's header (U3001 at a call, U3003); add `fails E` to a function without `fails` (U3009, U3008), inserted in canonical clause order and turning a short-form function into the long form; B) also guess a capability for `@c.op` in a pure function from the operation's name; C) no edits, the messages only.
- Chosen: A, because each edit is the only repair that keeps the program's meaning, and applying it removes the diagnostic and keeps the file canonical; B guesses when two capabilities share an operation name, and C leaves agents to rebuild clauses the compiler already knows.
- Confidence: high
- Reversal cost: low, `widen` and `add_clause` in `crates/una-check/src/types/authority.rs`
- Implemented in: `0ded2b9`, `crates/una-check/src/types/authority.rs` (`widen`, `add_clause`), tag `DECISION D-086`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-087: A capability reference bound twice (M5)
- Question: A capability context maps each name to one capability (`spec/05 §1`), but nothing reports a header with two `uses @log` clauses, two `given @log` lines or `with @log = @a, @log = @b`; ADR 0023 rule 5 left capability references to M5.
- Context: `spec/05 §1, §3, §4`, `spec/08 §2`; ADR 0023 rule 5; ADR 0028 rule 12.
- Options: A) U1002 at the second binding in one construct, the first binding winning, as for other names defined twice; B) merge the two bindings; C) accept them, the last one winning.
- Chosen: A, because a duplicate is almost always a mistake, merging would hide which capability a name has, and U1002 already means a name defined twice.
- Confidence: high
- Reversal cost: low, `duplicate_refs` in `crates/una-check/src/types/authority.rs` and `first_of_each` in `env.rs`
- Implemented in: `ce6f46b`, `crates/una-check/src/types/authority.rs` (`duplicate_refs`), `env.rs` (`first_of_each`), tag `DECISION D-087`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Accepted), rule 12
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-093: Calls under the program counter (M6)
- Question: `spec/10 §5` states `pc ⊑ bound(f)` for named functions; function values and handler installations also run effects and failures.
- Context: `spec/07 §3` rule 4, `spec/10 §5, §6`; ADR 0031 rule 5.
- Options: A) the same bound for function values, from their type, and for handler installations, from the handler's `uses`; the result of a function value is raised by the value's label; B) named functions only.
- Chosen: A, because a function value or an installed handler performs the same effects as a named function.
- Confidence: high
- Reversal cost: low
- Implemented in: `d519fbb`, `crates/una-check/src/types/flow.rs` (`bound`, `call_under_pc`), `call.rs`, `check.rs` (`install`), tag `DECISION D-093`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 5
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Escalated (E2), resolved by the owner through ADR 0038 D4: handler operation bodies have an effect floor at the operation's `accepts`, and failures and results follow data flow. This amends ADR 0031 rules 4 and 5; the installation-site check of this entry stays.

### D-095: What is not checked for labels (M6)
- Question: `spec/07 §4, §5` exempt `expect` lines and `via` bodies; clauses are in the refinement language, whose labels the verifier erases (`spec/12 §4`).
- Context: `spec/07 §4, §5`, `spec/10 §9` rule 3, `spec/12 §4`; ADR 0031 rule 8.
- Options: A) `expect` lines, `via` bodies and clauses are not checked, and `do` lines are checked at `pc = ⊥`; B) check clauses too.
- Chosen: A, because clauses produce no value at run time, and checking them would only reject specifications.
- Confidence: high
- Reversal cost: low
- Implemented in: `d519fbb`, `crates/una-check/src/types/check.rs`, `flow.rs` (`labels`), tag `DECISION D-095`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 8
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-096: Undeclared label categories (M6)
- Question: `spec/07 §2` makes an undeclared category U4007 without a site; a single file has no `una.toml`.
- Context: `spec/07 §2`, `spec/13 §7`; ADR 0031 rule 9.
- Options: A) U4007 at each undeclared category, in type labels and `accepts` clauses, a single file declaring none; B) one diagnostic per module.
- Chosen: A, because each category is a separate edit.
- Confidence: high
- Reversal cost: low
- Implemented in: `d519fbb`, `crates/una-check/src/types/env.rs` (`lower_label`), tag `DECISION D-096`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 9
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-097: Where information-flow diagnostics point (M6)
- Question: The specification gives no primary spans for U4001 to U4005.
- Context: `spec/13 §1`; ADR 0031 rule 10.
- Options: A) U4001 at the value, or at the condition for an implicit flow into a result; U4002 at the `fail`, clause, `checked` or call, with the condition that raised the program counter as a related location; U4003 at the exceeding argument, or at the operation when the program counter alone exceeds; U4004 at the policy's name; U4005 at `via`'s name or the use; U4007 at the category; B) every diagnostic at the enclosing declaration.
- Chosen: A, because each points at the code an agent edits, and the related condition explains an implicit flow.
- Confidence: high
- Reversal cost: low
- Implemented in: `d519fbb`, `crates/una-check/src/types/flow.rs`, tag `DECISION D-097`
- ADR: `docs/adr/0031-information-flow-rules-left-open.md` (Accepted), rule 10
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-099: The text of the Core dump (M7)
- Question: `spec/09 §4` makes `--dump-core <module>` print Core IR "deterministically", without a layout or saying what happens with diagnostics.
- Context: `spec/09 §2, §4`, `spec/13 §5`; ADR 0032 rule 2.
- Options: A) the module's declarations in the syntax of `spec/09 §2`, one per block, with every binder's type, label and refinements, and the package's diagnostics on stderr; B) no dump when there are diagnostics.
- Chosen: A, because the dump is a debugging aid, and declarations without diagnostics are still worth seeing.
- Confidence: high
- Reversal cost: low
- Implemented in: `59a481d`, `crates/una-core/src/print.rs`, `crates/una-cli/src/lib.rs`, tag `DECISION D-099`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 2
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-105: What the call depth counts (M7)
- Question: `spec/11 §7` limits the call depth to 10,000 frames without saying which calls are frames.
- Context: `spec/04 §7`, `spec/11 §7`; ADR 0032 rule 8.
- Options: A) calls of functions, predicates, lambdas, policies and handler operations; the evaluator runs on a thread whose stack is sized from the maximum depth (32 KiB per call and 16 MiB more), and aborts with U6003 when that stack runs short before the depth does, so it never overflows; B) function calls only.
- Chosen: A, because each of them recurses the same way, and nested expressions inside a recursive function use stack that the depth alone does not bound.
- Confidence: high
- Reversal cost: low
- Implemented in: `d81f200`, `c3d2041`, `crates/una-interp/src/machine.rs`, `lib.rs`, `crates/una-cli/src/runtime.rs`, tag `DECISION D-105`
- ADR: `docs/adr/0032-runtime-details-left-open.md` (Accepted), rule 8
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-110: How the conformance runner evaluates `una run` and `una test` (M7)
- Question: `compiler/testing.md §2` defines `run-output`, `test-pass`, `test-fail` and `-- grant:`, but not how the runner obtains them, whether errors reported by `una run` count toward `error` expectations, or where the relative paths of grants resolve.
- Context: `compiler/testing.md §2`; ADR 0013.
- Options: A) the runner calls `una run --json` and `una test --json` in-process; `una run` runs when a case expects output, grants a capability or expects an error of the runtime domain, and the errors of both documents join those of `una check`; relative `files:` and `keys:` paths resolve against the case's directory; B) a separate expectation for runtime errors.
- Chosen: A, because a case then states every error it expects in one form, and a case does not depend on the directory the tests run in.
- Confidence: high
- Reversal cost: low
- Implemented in: `54d30c8`, `crates/una-cli/tests/conformance/main.rs`, `outcome.rs`, `docs/compiler/testing.md`, tag `DECISION D-110`
- ADR: none (`compiler/testing.md §2` gains the rules)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-115: Encoding details (M8)
- Question: `spec/12 §4` gives examples of sort names and the shape of field, `len` and `tag` functions, but not the names of every sort, how binders become symbols, or how `Unit`, text ordering, `concat` and `to_text` are encoded.
- Context: `spec/12 §2, §4`; ADR 0033 rule 5.
- Options: A) the names of ADR 0033 rule 5, binders as quoted symbols (with a `!` suffix on `_`, `as`, `xor` and `distinct`, which Z3 5.1.0 rejects even quoted; found in M10 by an example whose discarded statement is the binder `_`), text ordering, `concat` and `to_text` as uninterpreted functions, and multiplication of two non-literals and division or remainder by a non-literal as the uninterpreted `mul!Int`, `div!Int` and `mod!Int`; B) no encoding for text ordering, which would make such refinements unverifiable; C) SMT-LIB `div` and `mod` for a division by a variable, as `spec/12 §2` says.
- Chosen: A, because uninterpreted functions add no axioms (ADR 0010's reasoning), and doubled underscores keep sort names distinct for distinct module paths. C is not possible: QF_UFLIA has no nonlinear terms, and Z3 5.1.0 answers such an assertion with an error and drops it, which would make an obligation pass or fail for the wrong reason. This deviates from the text of `spec/12 §2`, whose change the ADR lists.
- Confidence: high
- Reversal cost: low
- Implemented in: `49c73e9`, `3b83602` (the symbols of M10), `crates/una-verify/src/smt.rs` (`symbol`), `generate.rs` (`arithmetic`, `order`), tag `DECISION D-115`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 5
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-116: The solver protocol (M8)
- Question: `spec/12 §5` gives the preamble and says the compiler issues `(get-model)` or `(get-info :reason-unknown)`, without saying how processes, models and the version check are handled.
- Context: `spec/12 §5, §7`, ADR 0003, ADR 0022 rule 4; ADR 0033 rule 6.
- Options: A) one process per query, the whole script written at once with `(get-model)`, `(get-value ...)` for the source-level terms and `(get-info :reason-unknown)`; the wall-clock guard kills the process; the version read once per run; B) one interactive session per run.
- Chosen: A, because queries stay independent and parallel, a hang affects one query, and the script is the query the cache keys.
- Confidence: high
- Reversal cost: low
- Implemented in: `49c73e9`, `ec853fa`, `crates/una-verify/src/solver.rs`, `run.rs`, tag `DECISION D-116`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 6
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-117: The proof cache (M8)
- Question: `spec/12 §9` fixes the directory, the key and the value of the cache, but not the files, concurrency or what is not cached.
- Context: `spec/12 §5, §9`; ADR 0033 rule 7.
- Options: A) one JSON file per query named by its key, written through a rename; unreadable entries solved again; wall-clock unknowns not cached; B) one database file per package.
- Chosen: A, because parallel runs and tests then never corrupt the cache, and a result that depends on time must not be reused.
- Confidence: high
- Reversal cost: low
- Implemented in: `49c73e9`, `crates/una-verify/src/cache.rs`, tag `DECISION D-117`
- ADR: `docs/adr/0033-verification-details-left-open.md` (Accepted), rule 7
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-126: U8001's explain text has no examples (M9)
- Question: `spec/13 §4` asks a failing and a fixed example of every code, and U8001 is "Internal compiler error (always a bug)".
- Context: `spec/13 §4`; ADR 0034 rule 5.
- Options: A) a text without examples, saying how to report the bug; B) an example that triggers the error, which would be a known bug left in the compiler.
- Chosen: A, because a program that makes the compiler fail is a bug to fix, not an example to keep.
- Confidence: high
- Reversal cost: low
- Implemented in: `d57857b`, `crates/una-cli/tests/explain_examples.rs` and `crates/una-diag/explain/U8001.md`, tag `DECISION D-126`
- ADR: `docs/adr/0034-report-and-abi-details-left-open.md` (Accepted), rule 5
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-083: CI runners for the private repository (Part 5)
- Question: The repository una-lang/una stays private until launch, and a private repository has a limited GitHub Actions quota, which macOS runners use fastest. When does each job run?
- Context: the owner's instructions of 2026-09-29 (publication); `.github/workflows/ci.yml`; ADR 0030.
- Options: A) Linux on every push; macOS only on pull requests and on pushes to `main`, with the matrix chosen by an expression on the event; B) both on every push; C) two separate jobs with duplicated steps.
- Chosen: A, as the owner asked; local checks on the owner's Mac cover macOS during the run, and one matrix keeps the steps in one place.
- Confidence: high
- Reversal cost: low, the `matrix.os` expression and the `on.push` filter in `.github/workflows/ci.yml`
- Implemented in: the initial public import, `.github/workflows/ci.yml`, tag `DECISION D-083`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep. The entry's "`on.push` filter" does not exist: `ci.yml` runs Linux on every push and macOS on pull requests and pushes to `main` (R-044).

### D-009: Hidden entries under `src/` are not sources (M1)
- Question: Interim loading reads `src/**/*.una` (ADR 0015 D3). Are hidden entries, such as editor lock files `.#name.una`, sources?
- Context: ADR 0015 D3; ADR 0013 D6 (hidden entries ignored by conformance discovery).
- Options: A) skip entries whose names start with `.`; B) load every `.una` file.
- Chosen: A, consistent with ADR 0013 D6 and with how `**` globs usually behave; a stray lock file would otherwise produce spurious errors.
- Confidence: high
- Reversal cost: low, `is_hidden` in `crates/una-cli/src/load.rs`; M3 replaces interim loading
- Implemented in: M1 CLI commit (`9d2b95f`), `crates/una-cli/src/load.rs`, `crates/una-cli/tests/cli.rs`, tag `DECISION D-009`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-011: M0 CLI tests that checked the repository root (M1)
- Question: Two M0 tests ran `una check` in the repository root and expected success; ADR 0015 D3 (Accepted) makes a directory without `src/` a usage error.
- Context: ADR 0015 D3; `OVERNIGHT.md` C.5 (tests may change only when the spec shows the old expectation was wrong).
- Options: A) point them at `examples/hello`; B) keep them and give the root a `src/`.
- Chosen: A, because the accepted ADR makes the old expectation wrong and the approved M1 plan says so; the tests keep checking what they checked (empty output, exit 0, JSON bytes).
- Confidence: high
- Reversal cost: low
- Implemented in: M1 CLI commit (`9d2b95f`), `crates/una-cli/tests/cli.rs`, tag `DECISION D-011`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-012: `--color auto` looks at stderr (M1)
- Question: Which stream decides `--color auto`?
- Context: `spec/13 §5`; ADR 0015 D1 (human diagnostics on stderr).
- Options: A) stderr, where colored output goes; B) stdout.
- Chosen: A, because only diagnostics are colored and they go to stderr.
- Confidence: high
- Reversal cost: low, `crates/una-cli/src/main.rs`
- Implemented in: M1 CLI commit (`9d2b95f`), `crates/una-cli/src/main.rs`, tag `DECISION D-012`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-060: Choices beyond the renaming map of the neutral reference example (ADR 0026)
- Question: The owner's renaming map fixes most names of `examples/events`, but a few choices remain: the map gives `CheckInError = NotRegistered | Unavailable`, while the required coverage includes a sum with payload, which the previous reference example lacked; the parameter of `Audit.record`; and the neutral names of test fixtures.
- Context: ADR 0026; `spec/06 §1, §3` already show `Unavailable(reason: Text)`.
- Options: A) `Unavailable(reason: Text)`, with `if error fail Unavailable(reason: "...")` in `check_in`, which matches `spec/06`; `Audit.record(key, status: Availability)`, so `@audit.record(key, status)` still exercises punning; fixtures use neutral names (`Seat_Count`, `EVENTS3`, `box_office`, `Booking`, `sold_out`), and fixtures whose point is a prelude variant keep `NotFound`; B) follow the map literally and cover a sum with payload only in `spec/03`.
- Chosen: A, because the coverage list asks for a sum with payload in the example itself, and the payload makes the example and `spec/06` agree.
- Confidence: high
- Reversal cost: low, the example and its snapshots
- Implemented in: the neutral examples commit, `examples/events`, `tests/conformance`, tests of `una-check`, `una-diag`, `una-syntax`, `una-fmt`, `una-project`, `una-cli`
- ADR: `docs/adr/0026-neutral-reference-examples.md` (Accepted)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-059: One snapshot regeneration for the neutral reference example (ADR 0026)
- Question: ADR 0026 replaces the reference example, whose syntax trees are insta snapshots, and the diagnostic sample of `spec/13 §1`, which two snapshot tests mirror. Snapshots are otherwise never regenerated to make tests pass.
- Context: ADR 0026; `compiler/testing.md`; OVERNIGHT.md C.5.
- Options: A) regenerate the five example snapshots and the three diagnostic snapshots once, with `INSTA_UPDATE=always` (`cargo-insta` is not installed, and installing global tools is out of bounds), and review every line; B) write the new snapshots by hand.
- Chosen: A, as the owner allowed this one regeneration. Review: each syntax tree matches its source line by line; the diagnostic snapshots changed only in the names, the spans of the new sample and the texts of its obligation, counterexample and repair.
- Confidence: high
- Reversal cost: low
- Implemented in: the neutral examples commit, `crates/una-syntax/tests/snapshots/examples__events__*.snap`, `crates/una-diag/tests/snapshots/`
- ADR: `docs/adr/0026-neutral-reference-examples.md` (Accepted)
- Answer needed: none (recorded as a consequence of ADR 0026)
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-058: Test inputs probing the earlier code shapes (Una rename)
- Question: After the rename to Una (ADR 0025), tests still check that the codes of the earlier names are rejected, but the rename audit requires that no `T####` or `V####` code remains in the repository outside the history files.
- Context: ADR 0025; `crates/una-diag/src/codes.rs` tests, `crates/una-cli/tests/cli.rs`; D-004.
- Options: A) keep the probes and write them in two pieces (`concat!("T", "0001")`), so they still run and the audit finds nothing; B) drop the probes of the earlier prefixes; C) keep them literally and accept two audit hits.
- Chosen: A, because rejecting the earlier prefixes is still worth testing, and the audit stays exact. The probes of the current prefix become `u0001` (lowercase) and `U00001` (five digits).
- Confidence: high
- Reversal cost: low, test data only
- Implemented in: the Una rename commit, `crates/una-diag/src/codes.rs`, `crates/una-cli/tests/cli.rs`, `crates/una-diag/tests/registry.rs` (row filter `| U`)
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-003: English words that contain "vera" (Part A)
- Question: `grep -rni vera` (A.7) also matches "coverage" and "several", which contain the letters but are not the codename.
- Context: Part A.7.
- Options: A) leave them and classify them in the morning report; B) reword the sentences to avoid the substring.
- Chosen: A, because they are ordinary English words and rewording normative text for a grep would be a semantic risk with no benefit. A word-boundary grep (`grep -rniw vera`) finds only the occurrences allowed by A.3.
- Confidence: high
- Reversal cost: low
- Implemented in: `c198b65` (no change), morning report Part A
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-004: Test inputs probing the old code shape (Part A)
- Question: Tests fed `v0001` (lowercase) and `V00001` (five digits) to check that non-codes are rejected; the prefix rule of A.4 only matches four-digit `V####`.
- Context: Part A.4; `crates/troth-diag/src/codes.rs` tests, `crates/troth-cli/tests/cli.rs`.
- Options: A) update them to `t0001` and `T00001` so they keep testing the same property, and add `V0001` as an old, now unregistered code; B) leave them as they were.
- Chosen: A, because the tests exist to check lowercase and wrong-length rejection of the current prefix; the old prefix is also worth rejecting explicitly.
- Confidence: high
- Reversal cost: low, test data only
- Implemented in: `c198b65`, `crates/troth-diag/src/codes.rs`, `crates/troth-cli/tests/cli.rs`, `crates/troth-diag/tests/registry.rs` (row filter `| T`)
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep; superseded by D-058.

### D-005: CI trigger for the overnight branch (setup)
- Question: `ci.yml` ran on pushes to `main` only, so pushes to `overnight/v0.1` would not be checked.
- Context: Part C.1.5.
- Options: A) add `overnight/**` to the push branches; B) run on every branch.
- Chosen: A, the smallest change that enables CI for this run without changing behavior for other branches.
- Confidence: high
- Reversal cost: low, one line of `.github/workflows/ci.yml`
- Implemented in: setup commit, `.github/workflows/ci.yml`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B, or remove the entry after the run?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep; superseded by D-083.

### D-006: Temporary files of tests stay inside the repository (setup)
- Question: A runner test created a directory under the system temporary directory, outside the repository, which C.5 forbids for this run.
- Context: Part C.5 (files outside the repository); `crates/troth-cli/tests/conformance/discovery.rs`.
- Options: A) use `CARGO_TARGET_TMPDIR` (under `target/`, ignored by git) and clear it first; B) keep the system temporary directory.
- Chosen: A, because it keeps every write inside the repository and makes the test independent of leftovers.
- Confidence: high
- Reversal cost: low
- Implemented in: setup commit, `crates/troth-cli/tests/conformance/discovery.rs`, tag `DECISION D-006`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-007: Where the codename line goes in triage.md (Part A)
- Question: A.3 asks for one line in `triage.md §5`; the section ends with a "Naming consequence" paragraph that says a final name is still to be chosen.
- Context: Part A.3; `docs/reviews/triage.md §5` (written by the owner).
- Options: A) append the line after that paragraph; B) put it at the top of §5.
- Chosen: A, because it resolves the paragraph it follows and keeps the owner's text untouched.
- Confidence: high
- Reversal cost: low
- Implemented in: `c198b65`, `docs/reviews/triage.md`
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-008: Reading "`007` �7`" in B.2.1 (setup)
- Question: One character of B.2.1 was lost: "Attach a mechanical edit when the fix is unambiguous: `007` �7`".
- Context: `OVERNIGHT.md` B.2.1.
- Options: A) read it as "`007` → `7`", like the other two examples of the same list.
- Chosen: A, the only reading consistent with "leading zero" and with the other examples.
- Confidence: high
- Reversal cost: low
- Implemented in: `OVERNIGHT.md` progress notes; T0007 repairs (M1)
- ADR: none (minor)
- Answer needed: Keep A?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

### D-130: The tag `v0.1.0` is left to the owner (M10)
- Question: M10 asks for the tag `v0.1.0`, but the run never pushes to `main`, where the owner merges after review; a tag on `dev/v0.1` would name a release the owner has not reviewed.
- Context: `plan/milestones-v0.1.md` M10; `OVERNIGHT.md` C.5 and D.1.
- Options: A) prepare everything the tag triggers, but leave creating and pushing it to the owner, after the merge; B) tag `dev/v0.1` now; C) tag locally without pushing.
- Chosen: A, because a release is outward-facing and should point at reviewed history on `main`; the release workflow then builds and attaches the archives without further work.
- Confidence: high
- Reversal cost: low, pushing the tag
- Implemented in: `9b23b9f`, `.github/workflows/release.yml`, tag `DECISION D-130`
- ADR: `docs/adr/0035-examples-guide-and-release.md` (Accepted), rule 4
- Answer needed: Keep A? Or tag `dev/v0.1` before the merge?
- Answer: kept the chosen option (Leonardo, 2026-09-29)
- Review (2026-09-30): Keep

## Decisions of the v0.1 closure run

Choices of the closure run that neither the specification nor an ADR dictates (`docs/plan/runs/v0.1-closure-plan.md` §5). They await the owner's review, like the entries above did.

### D-133: A `Files` path that names the grant prefix itself (closure, R-011)
- Question: ADR 0038 D9 rule 3 opens the final component without following a link. A path such as `""`, `.` or `sub/..` normalizes to the prefix itself and has no final component.
- Context: `spec/14 §3`, ADR 0038 D9.
- Options: A) deny it (`FileError.Denied`); B) let the operation act on the prefix directory, which fails with `Io`.
- Chosen: A, because a file operation on the grant directory itself is never meaningful, and denying it keeps rule 3 total.
- Confidence: high
- Reversal cost: low, `crates/una-interp/src/host.rs` (`confine`)
- Implemented in: `342fe7b`, `crates/una-interp/src/host.rs`, `crates/una-cli/tests/runtime.rs`, tag `DECISION D-133`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-134: Every intermediate directory of a `Files` path stays inside the prefix (closure, R-011)
- Question: ADR 0038 D9 rule 2 says the resolved directory lies inside the canonical prefix. A path can pass through a directory outside the prefix and come back inside, for example through a link and `..`.
- Context: `spec/14 §3`, ADR 0038 D9.
- Options: A) check every intermediate directory; B) check only the directory that holds the final component.
- Chosen: A, because it is stricter, and a path that passes outside the grant reveals whether directories outside it exist.
- Confidence: high
- Reversal cost: low, `crates/una-interp/src/host.rs` (`confine`)
- Implemented in: `342fe7b`, `crates/una-interp/src/host.rs`, tag `DECISION D-134`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-135: How the verifier recognizes the prelude (closure, R-010)
- Question: ADR 0038 D7 asks that prelude kinds be identified by an identity user code cannot spell. The Core path of the prelude is `prelude`.
- Context: `spec/10 §11` rule 8, ADR 0038 D7.
- Options: A) keep the Core path `prelude` as the identity, which U1009 now reserves, since tier 1 runs only when tier 0 reports nothing (D-112), and let a kind the package declares always win; B) a new Core path that no source can spell, which changes the interpreter and the elaborator.
- Chosen: A, because U1009 makes the path unspellable in any program that reaches the verifier, and the change stays in one function.
- Confidence: medium
- Reversal cost: low, `is_prelude` in `crates/una-verify/src/generate.rs` and `una_core::PRELUDE_MODULE`
- Implemented in: `eab57c7`, `crates/una-verify/src/generate.rs`, tag `DECISION D-135`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-136: Labeled arguments of prelude functions (closure, R-005)
- Question: ADR 0038 D3.4 labels the results of the listed prelude functions by their arguments; it does not say whether those arguments may carry any label.
- Context: ADR 0038.
- Options: A) the listed functions accept arguments of any label, except function-typed parameters, which keep S-Fun, and `is_some`, `is_ok` and `append`, which spec/14 §1 does not list, keep strict parameters; B) every prelude parameter keeps its declared label.
- Chosen: A, because the result label now carries the argument labels, so accepting them loses nothing, and B would reject `abs(x)` on a labeled `x` whose result the rule labels anyway.
- Confidence: medium
- Reversal cost: low, `prelude_result_labels` in `crates/una-check/src/types/call.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-136`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?
- Answer (Leonardo, 2026-09-30): add `is_some`, `is_ok` and `append` to the functions whose results join the labels of their arguments, as ADR 0038 D3.4 requires for any result that depends on its arguments; `spec/14 §1` lists them.

### D-137: Which positions of a generic call's result are raised (closure, R-004)
- Question: `spec/10 §5` (generic instantiation) replaces each symbolic label of the result by the labels of the instantiating type.
- Context: ADR 0038.
- Options: A) raise every data position of the result by the join of the labels of all type arguments, lazily for function values; B) a per-function summary of which type parameters reach the result.
- Chosen: A, because it is sound and simple; B is more precise and can replace it without rejecting more programs.
- Confidence: medium
- Reversal cost: medium, `crates/una-check/src/types/call.rs` and `expr.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-137`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-138: Where an inferred U3016 is reported (closure, R-002)
- Question: `spec/05 §9` rule 6: at the value whose type instantiates the type parameter.
- Context: ADR 0038.
- Options: A) the first argument whose declared type mentions the parameter, skipping sites already reported by U3016 or U3006; B) every such argument.
- Chosen: A, because one diagnostic per instantiation is enough to repair it.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/generic.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-138`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-139: A function type with a second-class parameter or result (closure, R-002)
- Question: ADR 0038 D2: a type that contains a second-class type is not a type argument.
- Context: ADR 0038.
- Options: A) a function type whose parameter or result holds a second-class type counts as containing one; B) only data positions count.
- Chosen: A, because such a function value, stored, lets the second-class value escape through its parameter or result.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/ty.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-139`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-140: List literals instantiate `List` (closure, R-002)
- Question: ADR 0038 D2 names type parameters; a list literal fills the element type of `List`.
- Context: ADR 0038.
- Options: A) a list literal of second-class values is U3016 at its first element; B) only U3006 applies.
- Chosen: A, because `[note]` stores `note` exactly like `List[...]` does.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/expr.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-140`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-141: The effect floor of an operation that accepts every label (closure, R-003)
- Question: ADR 0038 D4 checks handler bodies at the operation's `accepts`; `Keys.hmac` accepts every label.
- Context: ADR 0038.
- Options: A) its floor is `⊤`, so a handler body for it performs no effects of lower bound; B) no floor.
- Chosen: A, because the worst caller may pass anything, and handlers of `Keys` return fixed values.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/check.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-141`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-142: Where a written U3016 is reported (closure, R-002)
- Question: `spec/05 §9` rule 6: at the written type argument.
- Context: ADR 0038.
- Options: A) once, at the innermost function-type argument; B) at every enclosing type argument.
- Chosen: A, because one edit repairs it.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/env.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-142`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-143: U3006 or U3016 for a field under a type argument (closure, R-002)
- Question: `spec/05 §9` rule 6 gives U3006 for a field whose type is an effectful function type, and U3016 for a type argument.
- Context: ADR 0038.
- Options: A) U3006 only when the field's type is directly a function type, U3016 when a function type is under a type argument (`List[fn ...]`); B) U3006 for both.
- Chosen: A, because each code then names the construct to change.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/authority.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-143`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-144: The Core consistency check runs in debug builds (closure, R-001)
- Question: ADR 0038 D1: "A debug-build consistency check compares checker types with Core IR binder types."
- Context: ADR 0038.
- Options: A) debug builds only, reported at the declaration; B) every build.
- Chosen: A, as the ADR says, since the check costs a second walk of Core IR.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/mod.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-144`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-145: A lambda annotation weaker than the expected parameter (closure, R-001)
- Question: S-Fun makes a lambda's written parameter type bound what it receives.
- Context: ADR 0038.
- Options: A) an annotation that drops a label, `uses` or `fails` of the expected type is reported at the annotation; B) at the call.
- Chosen: A, because the annotation is what the agent edits.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/expr.rs`
- Implemented in: `c4aea7c` (track a), tag `DECISION D-145`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-146: The effect floor of calls in handler bodies (closure, R-003)
- Question: ADR 0038 D4: "Every call inside must satisfy accepts ⊔ pc ⊑ bound(f)", and failures follow data flow with the body at `pc = ⊥`; `bound(f)` is `⊥` for a failing `f` (`spec/10 §5`), so the literal rule forbids every failing call in a handler of an operation that accepts a label above `public`.
- Context: ADR 0038.
- Options: A) the floor uses `bound_eff(f)`, the meet of the `accepts` labels of `f`'s effects, and a failing callee is checked against its full `bound(f)` at the body's own pc (`spec/07 §7` rule 4); B) the literal rule.
- Chosen: A, because the owner's answer says failures follow data flow, and B would reject the fake stores that fail, which the ADR keeps valid.
- Confidence: medium
- Reversal cost: low, `spec/07 §7` rule 4 and `crates/una-check/src/types/check.rs`
- Implemented in: `cf9a0a5` (spec text), `c4aea7c` (track a)
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-147: When a verification repair applies (closure, R-021)
- Question: `spec/13 §3` proposes "a stronger parameter type when the value is a parameter, a `requires` clause for a private function, a guard, and `checked`", each "when applicable", without defining when the value is a parameter or when a `requires` can reach the goal.
- Context: `spec/13 §3`, ADR 0033.
- Options: A) the value is a parameter when the goal's only binder, before temporaries are expanded, is a parameter of the enclosing function; `requires` only when every free binder of the goal is such a parameter; handler operations and opaque predicates get neither; a goal outside the encoding, and the `false` goal of an exhaustiveness obligation, are never ground; B) propose every repair of the kind, as before.
- Chosen: A, because each proposed repair must be one an agent can apply at that site.
- Confidence: medium
- Reversal cost: low, `place()` in `crates/una-verify/src/generate.rs` and `crates/una-verify/src/report.rs`
- Implemented in: `dd116aa`, tag `DECISION D-147`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-148: The report keeps the schema `una.report/1` with two new fields (closure, R-018)
- Question: ADR 0041 adds `obligations.cached` and `tcb.proof_cache` to the build report. `spec/13` says that changes to the Agent ABI "follow the ADR process and bump the version in the `schema` strings".
- Context: `spec/13` preamble and §6, ADR 0041.
- Options: A) keep `una.report/1`, since no release has published the ABI yet (`v0.1.0` is not tagged) and the new fields only add to the shape; B) bump to `una.report/2` now.
- Chosen: A, because version 1 is first published by the `v0.1.0` release, which includes these fields; the bump rule protects consumers of a published ABI.
- Confidence: medium
- Reversal cost: low, the schema string in `crates/una-cli/src/report.rs`, spec/13 §6 and the report snapshots
- Implemented in: `fe90186`, `crates/una-cli/src/report.rs`
- ADR: `docs/adr/0041-proof-cache-trusted.md` (Accepted), consequences
- Answer needed: Keep A? Or switch to B?
- Answer (Leonardo, 2026-09-30): kept A. No schema has been released yet; schema versions freeze at the `v0.1.0` tag, and any change after that bumps the version (`spec/13`, preamble).

### D-149: Which clauses may name an opaque predicate (closure, R-016)
- Question: ADR 0042 allows an opaque predicate only inside `checked` and opaque bodies, and makes "any other call, in code, a clause or an example" U2016. But `spec/03 §4` defines opaque refined kinds such as `kind Slug = Text where valid_slug(it)`, and an operation's `ensures` may establish an opaque fact.
- Context: `spec/03 §4`, `spec/06 §6`, ADR 0042.
- Options: A) a kind's `where` clause and a capability operation's `requires` and `ensures` may name an opaque predicate, since they name the fact without establishing it; function `requires` and `ensures` and `reflect` bodies may not (U2016); B) U2016 in every clause, which rejects the opaque kinds of `spec/03 §4`.
- Chosen: A, because B contradicts `spec/03 §4`, and A is the strictest reading that keeps it.
- Confidence: medium
- Reversal cost: low, `crates/una-check/src/types/call.rs`
- Implemented in: `b832ee9`, tag `DECISION D-149`; ADR 0042 rules 1 and 2 state it
- ADR: `docs/adr/0042-opaque-predicates-only-in-checked.md` (Accepted)
- Answer needed: Keep A? Or switch to B?

### D-150: Joins of refined function types (closure, F-0001-02)
- Question: ADR 0037 rule 1 carries refinements in function types; branches of `if` and `when` join their types.
- Context: ADR 0037.
- Options: A) a joined function type keeps, for each parameter, the refinements of either branch, and for the result only those both branches share; B) drop every refinement of a join.
- Chosen: A, because it is the strongest type that both branches satisfy (contravariant parameters, covariant result).
- Confidence: medium
- Reversal cost: low, the join in `crates/una-check/src/types/unify.rs`
- Implemented in: `9cf2b99`, tag `DECISION D-150`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-151: The repair of a refuted result refinement (closure, F-0001-02)
- Question: ADR 0037 rule 6 names the expected function type's position for a parameter; for the result it gives no wording.
- Context: ADR 0037.
- Options: A) the repair names the value's function type ("strengthen the result of `fn(...)`..."); B) no repair.
- Chosen: A, because the result is the value's own declaration, which the agent edits.
- Confidence: medium
- Reversal cost: low, `crates/una-verify/src/report.rs`
- Implemented in: `9cf2b99`, tag `DECISION D-151`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-152: A lambda's result against its expected type (closure, F-0001-02)
- Question: ADR 0037 rule 2 gives a covariant result goal; a lambda has no declared result.
- Context: ADR 0037.
- Options: A) the lambda's body paths are checked against the expected result, reported in the body, with a guard repair; B) one goal over the lambda value.
- Chosen: A, because the checker already checks lambda results on their body paths, and the guard is the edit that applies.
- Confidence: medium
- Reversal cost: low, `crates/una-verify/src/generate.rs`
- Implemented in: `9cf2b99`, tag `DECISION D-152`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-153: When a refinement counts as carried (closure, F-0001-02)
- Question: ADR 0037 rule 2: "a goal that is trivially implied (no refinement in the target, or the same refined kind) gives no obligation".
- Context: ADR 0037.
- Options: A) one obligation per refinement the target lacks; a refinement is carried when it is the same refined kind or the same range; B) one obligation per position.
- Chosen: A, because it follows the rule's wording and keeps each goal a single refinement.
- Confidence: high
- Reversal cost: low, `crates/una-verify/src/generate.rs`
- Implemented in: `9cf2b99`, tag `DECISION D-153`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-154: Names of nested positions in goal text (closure, F-0001-02)
- Question: ADR 0037 rule 4 names the flowing function; a function type nested in a parameter has no name of its own.
- Context: ADR 0037.
- Options: A) nested positions are named after their parent (`parameter arg1 of parameter g of takes_fn_int`), and other values by their source text; B) `the value`.
- Chosen: A, because the agent can find the position from the text.
- Confidence: high
- Reversal cost: low, `crates/una-verify/src/generate.rs`
- Implemented in: `9cf2b99`, tag `DECISION D-154`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-155: The nested-pattern repair is a help, not an edit (closure, B-001)
- Question: the owner asked that a nested pattern report that nested patterns are not supported in v0.1, "with a repair that binds and matches in two steps". A repair can be a help text or a mechanical edit (`spec/13 §3`).
- Context: `spec/02 §6`, `spec/13 §3`.
- Options: A) a help that shows the two-step shape (bind the field, then match it in a second `when` inside the arm); B) a mechanical edit that rewrites the arms.
- Chosen: A, because the edit would also have to regroup every arm that shares the outer variant, which no local edit can do safely.
- Confidence: high
- Reversal cost: low, `crates/una-syntax/src/parser.rs`
- Implemented in: `9902633`, tag `DECISION D-155`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-156: U3013 in predicate bodies (closure, R-029)
- Question: `spec/04 §4` puts one effect on each line; a predicate body is pure, and U2015 already reports effects and unhandled failures there.
- Context: the v0.1 review.
- Options: A) U3013 in a predicate body only for a nested fallible call that `attempt` handles, everything else staying with U2015; B) U3013 for every nested call.
- Chosen: A, because each problem gets exactly one code.
- Confidence: high
- Reversal cost: low, `position_site` in `crates/una-check/src/types/authority.rs`
- Implemented in: `73f90b9` (merge), tag `DECISION D-156`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-157: The U2005 and U2009 advice when nothing is left to suggest (closure, R-030)
- Question: The repairs now suggest only parameters or fields that no other argument gives.
- Context: the v0.1 review.
- Options: A) advice to remove the argument when every name is given, and a list of the open names when none is close; B) no repair.
- Chosen: A, because it tells the agent the next step.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/types/call.rs`, `stmt.rs`
- Implemented in: `73f90b9` (merge), tag `DECISION D-157`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-158: U1002 when both definitions have the same kind (closure, R-034)
- Question: The message named the construct twice ("as a function and as a function").
- Context: the v0.1 review.
- Options: A) "defined twice in this module, both times as a <construct>"; B) drop the construct.
- Chosen: A, because it keeps the construct in the message, as `CLAUDE.md` asks.
- Confidence: high
- Reversal cost: low, `crates/una-check/src/names/`
- Implemented in: `73f90b9` (merge), tag `DECISION D-158`
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?

### D-159: A case left out of the promoted Euclidean division program (closure, adversarial)
- Question: The adversarial `runtime-euclidean-division` has a line with negative literal divisors, on which the verifier and the checker disagree about division obligations (R-031).
- Context: the v0.1 review.
- Options: A) leave that line out of `runtime/euclidean_division_signs.una` and record the mismatch in the R-031 backlog row; B) fix R-031 now.
- Chosen: A, because R-031 needs a spec line first (`spec/12 §2`).
- Confidence: high
- Reversal cost: low, add the line back when R-031 is done
- Implemented in: `73f90b9` (merge), `tests/conformance/runtime/euclidean_division_signs.una` (no tag: the case is left out, and the backlog row records it)
- ADR: none (minor)
- Answer needed: Keep A? Or switch to B?
