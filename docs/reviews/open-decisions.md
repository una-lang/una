# Open decisions (overnight run)

Decision log of the autonomous run (`OVERNIGHT.md`, C.4). Every choice not literally dictated by the specification is recorded here. Entries are sorted by review priority: low confidence combined with high reversal cost first. Code and tests implementing a decision carry the comment `DECISION D-NNN`.

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
- ADR: `docs/adr/0020-canonical-form.md` (Provisional)
- Answer needed: Keep A? Or switch to B, or change a specific rule?

### D-037: What an import brings, and which scope wins (M3)
- Question: `spec/02 §1` makes a module's top-level names visible throughout it and `spec/14 §1` imports the prelude into every module, but nothing says what an import of a kind brings or which of two visible names wins. The previous reference example used a variant after importing only its kind, and declared a variant `NotFound` that the prelude kind `FileError` also has.
- Context: `spec/02 §1`, `spec/14 §1, §3`; the reference example; ADR 0023 rules 1 and 2.
- Options: A) scopes from innermost: locals, the module's declarations, explicit imports, the variants of imported sum kinds, the prelude; the innermost holder wins, and a tie within one scope goes to the type checker (M4); B) an import brings only the names listed, and prelude names cannot be reused; C) every variant is resolved by the expected type only.
- Chosen: A, because it makes both uses valid as written, keeps every later prelude addition from breaking programs, and leaves genuinely ambiguous names to the only pass that can disambiguate them. B rejects the reference example; C leaves bare constructors such as `SoldOut` meaningless outside a typed context.
- Confidence: medium
- Reversal cost: medium, `crates/una-check/src/names/` and the M4 disambiguation that builds on it
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-037`
- ADR: `docs/adr/0023-name-resolution.md` (Provisional)
- Answer needed: Keep A? Or switch to B or C?

### D-046: Variants in patterns and after `fail` take their kind from the context (M3)
- Question: `examples/events/src/events/main.una` matches `is SoldOut`, `is Few` and `is Open` without importing `Availability` or its variants. How are such variants resolved?
- Context: `spec/02 §6, §7`, `spec/06 §2`, `spec/08 §2`; `examples/events`; ADR 0023 rule 3.
- Options: A) a variant whose kind the context gives (a `when` pattern or `is` test: the tested value's kind; `fail` in an expression, a handling clause or `checked`: the function's error kind; `expect fails`: the `do` expression's error kind) is resolved by that kind in M4, and M3 does not report it; B) resolve every variant through scopes, which rejects the reference example.
- Chosen: A, because the reference example requires it and the kind is always known in these positions; bare constructors still resolve through scopes in M3.
- Confidence: medium
- Reversal cost: medium, `resolve.rs` in `crates/una-check/src/names/` and the M4 variant check that relies on it
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/resolve.rs`, `crates/una-check/tests/names.rs`, tag `DECISION D-046`
- ADR: `docs/adr/0023-name-resolution.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

### D-048: Variants after `fail`, in patterns and after `expect fails` (M4)
- Question: ADR 0023 rule 3 leaves contextual variants to M4. Where is each looked up, and which code reports a miss?
- Context: `spec/06 §2` (U3009), `spec/10 §4` (T-Case); ADR 0023; ADR 0024 rule 1.
- Options: A) in the tested value's kind for patterns and `is` (U1001 if absent, U2001 if not a sum); in the declared error kind after `fail`, through scopes when none is declared (a variant outside the declared kind is U3009 in M5); in the `do` call's error kind after `expect fails`; B) always through scopes.
- Chosen: A, because the kind is known in every one of these positions and the reference example relies on it; B cannot type `main.una`.
- Confidence: medium
- Reversal cost: medium, the variant lookups of the type checker
- Implemented in: `0c07912` (M4 type checker), `crates/una-check/src/types/`, tag `DECISION D-048`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

### D-063: Equality on type parameters instantiated with function types (M4)
- Question: `spec/03 §1` forbids `==` on function types, and `spec/03 §2` rule 4 allows `==` on type parameters. A generic function that compares values of `T`, called with a function type for `T`, would compare functions at run time.
- Context: `spec/03 §1, §2`, `spec/14 §1` (`contains`); charter theorem T1 (`spec/10 §10`); ADR 0024 rule 11.
- Options: A) record which type parameters each generic function compares, directly or through the generic functions it calls (the prelude's `contains` compares its items), and report U2001 at an instantiation that gives such a parameter a type whose values hold functions; B) forbid every type argument that holds functions; C) leave it to run time.
- Chosen: A, because it keeps type safety and rejects only the calls that would compare functions. B rejects harmless programs such as mapping over a list of functions; C needs a meaning for function equality that the specification denies.
- Confidence: medium
- Reversal cost: medium, `facts::generic_equality` and `KindFacts` in `crates/una-check/src/types/facts.rs`; switching to C would make the interpreter define function equality
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-063`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional), rule 11
- Answer needed: Keep A? Or switch to B or C?

### D-071: Which code reports a missing capability (M5)
- Question: `spec/05 §4` reports a capability that a callee needs and the caller lacks as U3001, `spec/04 §2, §5` reports effects beyond declared `uses` as U3003, and `spec/04 §2` rule 4 reports `@` in a function without `uses` as U3004. Which code applies where?
- Context: `spec/04 §2, §5`, `spec/05 §3, §4`, `spec/10 §2` (Suff); ADR 0028 rule 1.
- Options: A) U3004 for `@c` in a body whose `uses` are declared empty; U3001 for `@c` that no binding holds, and for a callee requirement whose name is absent or bound to another capability; U3002 for fewer operations; U3003 for a call with effects from a body whose `uses` are declared empty; B) U3001 for every missing capability.
- Chosen: A, because each statement of the specification keeps a case of its own, and every code gets one site.
- Confidence: medium
- Reversal cost: medium, the sites in `crates/una-check/src/types/` that report these codes
- Implemented in: M5 authority commits, tag `DECISION D-071`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 1
- Answer needed: Keep A? Or switch to B?

### D-072: Inferring the headers of private functions (M5)
- Question: ADR 0007 infers the `uses` and `fails` of private functions over the module's call graph, but a body's `@x.op` does not say which capability `@x` is, and a function may declare some clauses and omit others.
- Context: `spec/04 §5`, `spec/05 §4`; ADR 0007; ADR 0028 rule 2.
- Options: A) a private function with no `uses` clause infers them, one with any declares them all, and likewise for `fails`; an inferred reference takes the capability its callers bind to its name, after `with`; a reference that no caller binds, or that callers bind differently, is U3001 with a hint to declare `uses`; the inferred error kind is the one kind of the variants raised and the kinds passed, a second kind being U3009; B) infer a reference's capability from its operation's name.
- Chosen: A, because capabilities flow downward by name (`spec/05 §4`), so the callers define what a reference is; B is ambiguous when two capabilities share an operation name.
- Confidence: medium
- Reversal cost: medium, the inference pass and the second checking pass
- Implemented in: M5 authority commits, tag `DECISION D-072`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 2
- Answer needed: Keep A? Or switch to B?

### D-018: `[` right after a label level (M1)
- Question: After a level such as `secret`, `[` can start the label's categories (`secret[PII] Phone`) or a list type (`secret [Int]`). The grammar of `spec/02 §2` allows both readings, and telling them apart needs unbounded lookahead.
- Context: `spec/02` introduction (LL(2) except kind bodies) and §2; `spec/03 §1`, `spec/07 §2`, `spec/14` (`List[T]`).
- Options: A) `[` after a level always starts categories; a labeled list type is written `secret List[T]`, and the error says so; B) backtrack and reread `[Int]` as a list type when no type follows; C) forbid `[T]` after any label.
- Chosen: A, because it keeps the parser LL(2) as the spec states, rejects the ambiguous spelling instead of guessing (strictness), and gives agents a precise fix. B could be adopted later without breaking any program; C is stricter than needed, since `secret[PII] [Int]` is unambiguous.
- Confidence: medium
- Reversal cost: low, `label` and `type_core` in `crates/una-syntax/src/parser.rs`; one sentence of `spec/02 §2` on acceptance
- Implemented in: M1 parser commit (`d95fe43`), `crates/una-syntax/src/parser.rs`, `crates/una-syntax/tests/parser.rs`, tag `DECISION D-018`
- ADR: `docs/adr/0018-label-bracket-starts-categories.md` (Provisional)
- Answer needed: Keep A? Or switch to B or C?

### D-022: Nesting limit (M1)
- Question: `una check` aborted with a stack overflow on 20 000 nested parentheses, a sum of 200 000 terms, and 5 000 text literals nested in interpolations. The grammar sets no bound on nesting. What should happen?
- Context: `spec/02`; ADR 0017 decision 7 (robustness); `compiler/architecture.md §7` (dependency allowlist); `CLAUDE.md` (deterministic output).
- Options: A) reject nesting deeper than 256 levels with U0101, counting nested expressions, prefix operators, chained operators, postfix links, types and blocks; bound the lexer's literal-in-interpolation recursion; run `una check` on a 64 MiB stack; B) only a larger stack; C) a limit measured in bytes of stack; D) count only true recursion, not operator chains.
- Chosen: A, because it is deterministic (the same in every build), bounds the depth of the tree for every later pass as well as the parser, and turns a crash into a diagnostic with a fix. B and D leave deep trees crashing later passes, and C depends on the build. 256 levels is far beyond hand-written or generated code, and raising it later breaks no program.
- Confidence: medium
- Reversal cost: low, `MAX_NESTING` and `deeper` in `crates/una-syntax/src/parser.rs`, `text_literal_with` in `crates/una-syntax/src/lexer.rs`, `on_large_stack` in `crates/una-cli/src/lib.rs`
- Implemented in: M1 nesting commit (`bde3b98`), those files, `crates/una-syntax/tests/robustness.rs`, `crates/una-cli/tests/cli.rs`, tag `DECISION D-022`; M1 explain commit (`b1039e3`), `crates/una-diag/explain/U0101.md`
- ADR: `docs/adr/0019-nesting-limit.md` (Provisional)
- Answer needed: Keep A with 256? Or change the limit, or switch to D?

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

### D-024: One-line bodies for predicates, handler operations and arms (M2)
- Question: `spec/04 §1` makes the short form canonical for a function without clauses whose body is one line. Predicate bodies, handler operation bodies and `when` arms have the same `= line | block` choice. Is the rule the same for them?
- Context: `spec/04 §1`, `spec/02 §3, §4, §6`; ADR 0020 rule 4.
- Options: A) the same rule for all four; B) only functions, as written.
- Chosen: A, because the four constructs share one grammar shape, one canonical form for all of them follows charter P4, and it is the stricter option (C.3), which can be relaxed without breaking programs.
- Confidence: medium
- Reversal cost: low, `body` in `crates/una-fmt/src/rewrite.rs`
- Implemented in: M2 formatter commit (`697cd2e`), `crates/una-fmt/src/rewrite.rs`, `crates/una-fmt/tests/format.rs`, `tests/conformance/fmt/inline_arm_body.una`, tag `DECISION D-024`
- ADR: `docs/adr/0020-canonical-form.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

### D-025: Imports are sorted (M2)
- Question: Does the canonical form order imports, and the names inside an import?
- Context: ADR 0011 (diffs from different agents converge); ADR 0020 rule 7.
- Options: A) sort imports bytewise by module path and names bytewise; B) keep the author's order.
- Chosen: A, because two agents adding imports in a different order would otherwise produce different files, and sorting is the stricter option. The examples were reformatted, which reordered the imports of two modules of the previous reference example and their two syntax-tree snapshots (reviewed: only import lines moved).
- Confidence: medium
- Reversal cost: low, `imports` in `crates/una-fmt/src/layout.rs` and `collect` in `crates/una-fmt/src/rewrite.rs`
- Implemented in: M2 formatter commit (`697cd2e`), those files, `crates/una-fmt/tests/format.rs`, tag `DECISION D-025`
- ADR: `docs/adr/0020-canonical-form.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

### D-028: The shape of U7001 (M2)
- Question: How does U7001 report a non-canonical file: once per file, per line, or per changed region? With which edit?
- Context: `spec/13 §1` (edits), ADR 0011 (mechanical edits), ADR 0020 rule 10.
- Options: A) one U7001 per changed region of a line diff, at its first line, with a classified message (trailing whitespace, missing or extra blank line, missing final line break, lines not canonical) and an edit replacing the region; B) one U7001 per file with an edit replacing the whole file; C) one per differing line.
- Chosen: A, because it points at each difference, keeps edits small and independent, and names the common cases precisely. A file with `\r\n` line breaks gets one U7001 at the first of them without an edit, since positions count decoded text, in which the carriage return no longer exists.
- Confidence: medium
- Reversal cost: low, `differences` in `crates/una-fmt/src/diff.rs`
- Implemented in: M2 CLI commit (`acf4ec2`), `crates/una-fmt/src/diff.rs`, `tests/conformance/fmt/`, tag `DECISION D-028`
- ADR: `docs/adr/0020-canonical-form.md` (Provisional)
- Answer needed: Keep A? Or switch to B or C?

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

### D-039: Shadowing (M3)
- Question: May a local rebind another local (`let x` after a parameter `x`), and may declarations, imports and locals reuse prelude or module names?
- Context: ADR 0023 rule 4; C.3 strictness; the previous reference example declared a variant `NotFound`, which the prelude kind `FileError` also has.
- Options: A) a local rebinding a local of an enclosing scope is U1002; a declaration or import may reuse a prelude name, and a local any non-local name; B) allow every rebinding; C) forbid every reuse.
- Chosen: A, because rebinding locals makes a reader track which binding each use means, which matters most in contracts, while forbidding reuse of global names would make common local names such as `sum` or `first` collide with the prelude and break programs whenever the prelude grows. C also rejects a module that declares its own `NotFound`.
- Confidence: medium
- Reversal cost: low, the local scope stack in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-039`
- ADR: `docs/adr/0023-name-resolution.md` (Provisional)
- Answer needed: Keep A? Or switch to B or C?

### D-033: The keys and values of `una.toml` (M3)
- Question: `spec/13 §7` shows an example and says unknown keys are U7002, but not which keys are required or which values are valid.
- Context: `spec/13 §5, §7`; ADR 0021 rule 2.
- Options: A) `[package]` with non-empty `name`, `version` and `edition = "0.1"` required; `[labels] categories` UpperCamelCase and unique; `[trusted] modules`; `[escape_budget] checked_per_module` at least 0; `[run] entry` naming a module of the package and `grants` in the syntax of `spec/13 §5`; any other key, a wrong type or a value out of range is U7002; B) accept any values and check them only where used.
- Chosen: A, because a manifest error should be reported where it is written, with the whole package checked, and every rule can be loosened later without breaking a package.
- Confidence: medium
- Reversal cost: low, `crates/una-project/src/manifest.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-project/src/manifest.rs`, tag `DECISION D-033`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

### D-036: Locating diagnostics that have no source position (M3)
- Question: A missing `verification.lock` (U7004) has no position in any file, yet every diagnostic has a required `primary` location (M0 decision, ADR 0012). The owner asked for this ADR when U7004 is implemented.
- Context: `spec/13 §1`; ADR 0012; `compiler/testing.md §2` (expected positions lie in existing files); ADR 0022.
- Options: A) the empty range at 1:1 of the file that declares what is checked: `una.toml` for a package, the file itself for a single file; B) a position in the missing file; C) make `primary` optional.
- Chosen: A, because the ABI keeps one shape and the position is displayable and testable; B points into a file that does not exist, and C changes every diagnostic's shape for a handful of codes.
- Confidence: medium
- Reversal cost: low, the missing-lock branch in `crates/una-check/src/project.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-check/src/project.rs`, tag `DECISION D-036`
- ADR: `docs/adr/0022-diagnostics-without-a-source-position.md` (Provisional)
- Answer needed: Keep A? Or switch to B or C?

### D-038: Where U1001 ends and U2002 begins, and what M3 resolves (M3)
- Question: An undeclared type name is both an "Unresolved name" (U1001) and an "Unknown ... type" (U2002). Which code, and which names can M3 resolve before types exist?
- Context: `spec/13 §4`; `spec/04 §3` (method calls depend on types); ADR 0023 rule 3.
- Options: A) U1001 for any name that no scope holds, type names included; U2002 (M4) for a name that resolves to something that is not a type, or an ill-formed type; fields, methods and argument labels wait for M4; B) U2002 for every unknown type name.
- Chosen: A, because one code then means "this name exists nowhere", reported by one pass, and U2002 keeps a precise meaning for types that exist but are wrong.
- Confidence: medium
- Reversal cost: low, the type branch of the resolver in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-038`
- ADR: `docs/adr/0023-name-resolution.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

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

### D-050: Text literals in the refinement language (M4)
- Question: `spec/03 §3` lists integer and boolean literals among RL atoms but not text literals, while `spec/12 §4` encodes text literals. Is `it != ""` a valid refinement?
- Context: `spec/03 §3`, `spec/12 §4`; ADR 0024 rule 3.
- Options: A) text literals are RL atoms; B) they are not (U2014).
- Chosen: A, because the encoding already supports them, so rejecting them would forbid refinements that can be verified.
- Confidence: medium
- Reversal cost: low, `Checker::text` in `crates/una-check/src/types/expr.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-050`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

### D-051: What U2008 covers (M4)
- Question: U2008 is "Unreachable arm or misplaced `else`". Which arms are unreachable?
- Context: `spec/02 §8` (an `else` arm is the last arm), `spec/10 §4`; ADR 0024 rule 4.
- Options: A) an arm after `else`, a repeated variant or literal arm, and an `else` after arms that cover every variant; B) only an arm after `else`.
- Chosen: A, because each of these arms can never run, and reporting them is the stricter option.
- Confidence: medium
- Reversal cost: low, the arm checks in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-051`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

### D-062: Reading the recursion restriction of kinds (M4)
- Question: `spec/03 §2` rule 5 allows recursive kinds "only through `List` or `Option` fields". Is a sum with a variant without fields (`Tree = Leaf | Branch(left: Tree, right: Tree)`), a type argument (`inner: Pair[Wrapped, Int]`) or a function type (`make: fn() -> Deferred`) recursion through something else?
- Context: `spec/03 §2` rule 5; ADR 0024 rule 10.
- Options: A) yes: a record or sum kind that contains itself anywhere except under `List` or `Option` is U2002, one diagnostic per cycle, at the first field that leads back into it, with an edit that wraps the field's type in `Option`; B) accept sums with a variant without fields, and function types, whose values are finite.
- Chosen: A, the literal reading and the stricter option; a tree is written with `List` or `Option` fields.
- Confidence: medium
- Reversal cost: low, `facts::kind_recursion` in `crates/una-check/src/types/facts.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-062`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional), rule 10
- Answer needed: Keep A? Or switch to B?

### D-064: Punning of the first argument, and unlabeled constructor arguments (M4)
- Question: `spec/04 §3` rule 3 makes a bare name equal to a parameter name count as labeled. Does that hold for the first argument, which may go unlabeled anyway? Which code reports an unlabeled argument of a constructor with several fields, extra arguments of a constructor with one field, and a variant without fields called with parentheses?
- Context: `spec/04 §3` rules 1 to 4; ADR 0024 rule 12.
- Options: A) a bare name equal to a parameter's name is labeled with it wherever it appears, so `max(b, a)` passes `b` as `b`; an unlabeled constructor argument where labels are required is U2004 with an edit that adds the label; extra arguments of a constructor with one field are U2009; `V()` for a variant without fields is U2009 with an edit to `V`; B) the first argument is positional even when it names another parameter.
- Chosen: A, the literal reading of rule 3; under B, `max(b, a)` would report U2006.
- Confidence: medium
- Reversal cost: low, `Checker::punned` and `Checker::construct` in `crates/una-check/src/types/call.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-064`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional), rule 12
- Answer needed: Keep A? Or switch to B?

### D-065: Recursion and results of predicates (M4)
- Question: `spec/03 §4` forbids a `reflect` predicate from calling itself "directly or indirectly", and the title of U2015 lists "recursive" for every predicate. Does the rule cover opaque predicates and cycles through functions? What reports a predicate whose result is not `Bool`?
- Context: `spec/03 §4`, `spec/13 §4`; ADR 0024 rule 13.
- Options: A) any cycle of calls or references that contains a predicate, reflect or opaque, through predicates or functions, is U2015 at the name of its first predicate, once per cycle; a result type other than `Bool` is U2001 at the result type; B) only reflect predicates calling reflect predicates.
- Chosen: A, because predicates are total and recursion endangers totality whatever closes the cycle, and the stricter reading can be relaxed later.
- Confidence: medium
- Reversal cost: low, `facts::predicate_recursion` and `Checker::predicate`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-065`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional), rule 13
- Answer needed: Keep A? Or switch to B?

### D-061: Details of the refinement language (M4)
- Question: `spec/03 §3` lists the forms of the refinement language. Are `<` on `Text`, `is` with a literal, comparing with a variant by `==`, `unit` and interpolated text inside it?
- Context: `spec/03 §3`; `spec/12 §4` (no encoding orders text; variants are tested through tags); ADR 0024 rule 9; D-050.
- Options: A) outside it: `<` and the other orderings on `Text`, `is` with a literal (with an edit to `==`), `x == V` for a variant `V` (with an edit to `x is V`), `unit`, and interpolated text; plain text literals are inside it (D-050); B) accept all of them as sugar.
- Chosen: A, because each form is either absent from the list of `spec/03 §3` or has no encoding in `spec/12 §4`; the edits keep the cost to agents low, and relaxing later breaks no program.
- Confidence: medium
- Reversal cost: low, the calls of `outside_rl` in `crates/una-check/src/types/expr.rs`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-061`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional), rule 9
- Answer needed: Keep A? Or accept some of the forms (B)?

### D-075: Host capabilities in examples (M5)
- Question: `spec/08 §2` rule 1 forbids host capabilities in examples (U3014) and takes every capability of the `do` line from `given` (U3001 if missing). Which applies to a missing `given`?
- Context: `spec/08 §2`, `spec/14 §3`; ADR 0028 rule 5.
- Options: A) a missing `given` for a host capability is U3014, for any other capability U3001; `expect performed @x.op` names a `given` binding (U3001) and an operation of its capability (U3002); B) U3001 for every missing `given`.
- Chosen: A, so that U3014 has its own case: a `do` line that would reach the host.
- Confidence: medium
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-075`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 5
- Answer needed: Keep A? Or switch to B?

### D-076: Grants checked by `una check` (M5)
- Question: U3012 is reported when `main` uses a host capability that `una run` does not grant; `[run] grants` is in `una.toml`, while `--grant` flags come only with `una run` (M7).
- Context: `spec/04 §8`, `spec/05 §7`, `spec/13 §5`; ADR 0028 rule 6.
- Options: A) when `una.toml` lists `[run] grants`, `una check` reports a host operation of `main`'s `uses` that no grant allows (U3012 at the clause), and `una run` checks its own grants in M7; B) only `una run` reports U3012.
- Chosen: A, because the manifest's grants are known statically, and checking early is stricter; a package without `[run] grants` is left to `una run`.
- Confidence: medium
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-076`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 6
- Answer needed: Keep A? Or switch to B?

### D-079: Escaping lambdas and function values with effects (M5)
- Question: `spec/05 §6` and `spec/10 §9` rule 2 let an effectful or failing lambda appear only directly as a call argument (U3006). Does the rule cover named functions used as values, and fields of such function types?
- Context: `spec/03 §1`, `spec/05 §6`, `spec/10 §9`; ADR 0028 rule 9.
- Options: A) a lambda whose expected type has `uses` or `fails`, and a function value of a function with `uses` or `fails`, appear only directly as call arguments, and a kind field of such a type is U3006 at the type; B) lambdas only.
- Chosen: A, because a function value with effects carries the same authority as a lambda with them, and `spec/03 §1` rules such values out of kinds.
- Confidence: medium
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-079`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 9
- Answer needed: Keep A? Or switch to B?

### D-081: The output of `una contract` (M5)
- Question: `spec/13 §5` defines `una contract <qualified.name>` without a JSON shape, without saying which package it reads, or how inferred clauses appear; M5 requires a `--json` snapshot.
- Context: `spec/13 §5`; ADR 0029.
- Options: A) the package in the current directory; human output as canonical header syntax with `-- inferred` after inferred clauses and a line per requirement; JSON schema `una.contract/1`; B) add a path argument.
- Chosen: A, because it adds no command-line syntax and keeps the human output valid Una.
- Confidence: medium
- Reversal cost: low, `crates/una-cli/src/contract.rs` and the schema
- Implemented in: M5 contract commit, tag `DECISION D-081`
- ADR: `docs/adr/0029-contract-output.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

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

### D-026: Constructor order for kinds declared in the same file (M2)
- Question: `spec/04 §3.4` has the formatter order constructor arguments by declaration, which needs the declaration. Imported kinds need name resolution, which arrives in M3.
- Context: `spec/04 §3.4`; `plan/milestones-v0.1.md` M2 and M3; ADR 0020 rule 6.
- Options: A) in M2, order constructors of kinds and variants declared in the same file whose name is declared once and not imported; add imported ones in M3; B) wait for M3 entirely.
- Chosen: A, because it delivers the rule where M2 can apply it exactly and needs no guess about imports. The examples already use declaration order everywhere, so M3 will not make them non-canonical.
- Confidence: high
- Reversal cost: low, `constructors` in `crates/una-fmt/src/rewrite.rs`
- Implemented in: M2 formatter commit (`697cd2e`), `crates/una-fmt/src/rewrite.rs`, `crates/una-fmt/tests/format.rs`, tag `DECISION D-026`
- ADR: `docs/adr/0020-canonical-form.md` (Provisional)
- Answer needed: Keep A?

### D-027: Files with syntax errors have no canonical form (M2)
- Question: Does `una check` report U7001, and does `una fmt` rewrite, a file that has syntax errors?
- Context: `compiler/architecture.md §3` (U7001 at tier 0, after parsing); ADR 0020.
- Options: A) no: U7001 only for files without syntax errors, and `una fmt` leaves the others untouched; B) format what parsed and report U7001 anyway.
- Chosen: A, because formatting a partial tree could delete code, and U7001 on a broken file would repeat what the syntax errors already say.
- Confidence: high
- Reversal cost: low, `check_file` in `crates/una-cli/src/lib.rs`
- Implemented in: M2 CLI commit (`acf4ec2`), `crates/una-cli/src/lib.rs`, tag `DECISION D-027`
- ADR: `docs/adr/0020-canonical-form.md` (Provisional)
- Answer needed: Keep A?

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

### D-032: A directory is a package only with `una.toml` and `src/` (M3)
- Question: ADR 0015 D3 (interim loading) expires in M3. What does a directory without `una.toml` or `src/` mean now?
- Context: `spec/13 §7`; ADR 0015 D3; ADR 0021 rule 1.
- Options: A) a usage error (exit 2) for either; B) check `src/` with default settings when `una.toml` is missing.
- Chosen: A, because a package's settings (labels, trusted modules, budgets) live in `una.toml`, and B would check a package silently without them. Two CLI tests that built `src/` without `una.toml` now also write one, as ADR 0015 D3 foresaw.
- Confidence: high
- Reversal cost: low, `discover` in `crates/una-project/src/package.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-project/src/package.rs`, `crates/una-cli/tests/cli.rs`, tag `DECISION D-032`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Provisional)
- Answer needed: Keep A?

### D-034: The values of `verification.lock`, and the default lock (M3)
- Question: `spec/12 §9` lists six keys. Which values are valid, and which lock does a single file use?
- Context: `spec/12 §9`; `compiler/testing.md §3`; ADR 0021 rule 3.
- Options: A) exactly the six keys, with `encoding_version = "una-smt-1"`, `solver = "z3"`, `logic = "QF_UFLIA"`, a non-empty `solver_version`, `step_budget` of at least 1 and `random_seed` of at least 0; a single file uses the repository's values; B) check only the syntax.
- Chosen: A, because the compiler implements one encoding, one solver and one logic, so another value cannot be honored; the installed solver's version is U5003 in M8.
- Confidence: high
- Reversal cost: low, `crates/una-project/src/lock.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-project/src/lock.rs`, tag `DECISION D-034`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Provisional)
- Answer needed: Keep A?

### D-035: Project errors do not stop checking (M3)
- Question: When `una.toml` or `verification.lock` is invalid, are the modules still checked?
- Context: `compiler/architecture.md §3` ("one run reports as many independent problems as possible"); ADR 0021 rule 4.
- Options: A) yes, with the default settings in place of an invalid `una.toml`; B) stop after the project errors.
- Chosen: A, because the modules do not depend on the lock until M8, and default settings are the strictest (no trusted modules, a budget of 0).
- Confidence: high
- Reversal cost: low, `check` in `crates/una-cli/src/lib.rs`
- Implemented in: M3 project commit (`457ca92`), `crates/una-cli/src/lib.rs`, tag `DECISION D-035`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Provisional)
- Answer needed: Keep A?

### D-040: How U1003 and U1004 are reported (M3)
- Question: Where does an import cycle point, and what repair does a private import get?
- Context: `spec/13 §1` (repairs); ADR 0023 rule 5.
- Options: A) one U1003 per cycle, at the first import of the cycle's first module in path order, naming every module of the cycle; U1004 at the imported name with an edit adding `pub` to the declaration in the other file; B) one U1003 per import of the cycle.
- Chosen: A, because one cycle is one problem, and the `pub` edit is mechanical.
- Confidence: high
- Reversal cost: low, `imports` in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-040`
- ADR: `docs/adr/0023-name-resolution.md` (Provisional)
- Answer needed: Keep A?

### D-041: Modules with syntax errors are not resolved (M3)
- Question: How do name checks treat a module whose parse lost declarations?
- Context: `compiler/architecture.md §3, §5`; ADR 0023 rule 6.
- Options: A) register the declarations that parsed and check them for U1002, U1005 and U1008, but resolve no body and check no import from that module; B) resolve everything, reporting what cannot be found.
- Chosen: A, because a name defined by a declaration that failed to parse would otherwise be reported as unresolved everywhere it is used.
- Confidence: high
- Reversal cost: low, the syntax-error flag of each module in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/`, tag `DECISION D-041`
- ADR: `docs/adr/0023-name-resolution.md` (Provisional)
- Answer needed: Keep A?

### D-042: Naming conventions for type parameters, owners and module paths (M3)
- Question: `spec/01 §4` lists the conventions of kinds, variants, capabilities, categories, lower-case names and requirement identifiers, but not of type parameters, owners or module path segments.
- Context: `spec/01 §4`, `spec/08 §1` (owners are "lower snake case"); ADR 0023 rule 7.
- Options: A) type parameters are UpperCamelCase, owners and path segments follow the lower-case rule, all checked where defined; B) check only the listed names.
- Chosen: A, because each of these names already has the shape of one of the listed classes, and checking it is the stricter option.
- Confidence: high
- Reversal cost: low, `conventions.rs` in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), `crates/una-check/src/names/conventions.rs`, tag `DECISION D-042`
- ADR: `docs/adr/0023-name-resolution.md` (Provisional)
- Answer needed: Keep A?

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

### D-045: A single file with a `trusted module` header (M3)
- Question: `spec/07 §6` requires a trusted module to be declared in its header and in `una.toml`. A single file has no `una.toml`.
- Context: `spec/07 §6`; `compiler/testing.md §2` (default settings); ADR 0021.
- Options: A) U7005, since the default settings trust no module; B) trust the file when its header says so.
- Chosen: A, because the double declaration exists to make trust deliberate at the package level, and a single file has no second declaration. Conformance cases about trusted modules are packages.
- Confidence: high
- Reversal cost: low, the trusted check in `crates/una-check/src/names/`
- Implemented in: M3 names commit (`9915290`), tag `DECISION D-045`
- ADR: `docs/adr/0021-packages-and-project-files.md` (Provisional)
- Answer needed: Keep A?

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

### D-049: What `fail` joins with, and how branches combine (M4)
- Question: `fail` ends the function; `spec/10 §5` gives it any type τ. How do `if` and `when` branches combine when one fails or when their refinements differ?
- Context: `spec/10 §4, §5` (T-If, T-Case, T-Fail); ADR 0024 rule 2.
- Options: A) `fail` has a bottom type that joins anything; other branches share one base type, and a known target type is checked in each branch, with refinements left to obligations; B) require every branch to have exactly the same type, refinements included.
- Chosen: A, because it is what T-Fail's free τ means, and refinements are the verifier's concern (`spec/03 §2.2`).
- Confidence: high
- Reversal cost: low, the join in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-049`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional)
- Answer needed: Keep A?

### D-052: Resolving `e.f` and `e.f(args)` (M4)
- Question: `spec/04 §3` resolves `e.f(args)` as a field of function type or a function `f` in scope, and `e.f` as a field or a one-parameter function. What if neither exists, and in which order?
- Context: `spec/04 §3`; ADR 0024 rule 5.
- Options: A) `.len` on `Text` and `List`, then a field, then a function or predicate declared in scope (locals are not searched); U1001 when nothing matches, with an edit to the closest field; a receiver whose type is not known yet is D-069; B) report U2001 for an unknown field.
- Chosen: A, because the order follows the specification, and a name that matches nothing is unresolved.
- Confidence: high
- Reversal cost: low, field and method resolution in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-052`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional)
- Answer needed: Keep A? Or switch to B?

### D-053: Calls of function values and constructors as values (M4)
- Question: Function types carry no parameter names, and the Core IR of `spec/09` has no constructor values. Can a function value be called with labels, and can `Some` be passed as a function?
- Context: `spec/03 §1`, `spec/04 §3`, `spec/09 §2`; ADR 0024 rules 6 and 7.
- Options: A) function values take positional arguments (a label is U2005, with an edit that removes it), and so does a policy call (D-066); a constructor with fields must be applied (U2009 otherwise); B) allow both.
- Chosen: A, because labels need parameter names that function types lack, and constructor values would need a Core form that does not exist. Both can be relaxed later.
- Confidence: high
- Reversal cost: low, calls in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-053`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional)
- Answer needed: Keep A?

### D-054: A type that inference leaves unknown (M4)
- Question: An empty list that nothing constrains, or a generic call whose parameter nothing fixes, leaves a unification variable unsolved. Core IR needs every type explicit (`spec/09 §1`).
- Context: `spec/03 §7`, `spec/09 §1`; ADR 0024 rule 8.
- Options: A) U2001 with a hint to annotate, at the empty list or the generic use that introduced the type, reported only when the declaration has no other type diagnostic, since an earlier error often leaves the type unknown; B) default the type to `Unit`.
- Chosen: A, because a default hides a mistake, and an annotation always resolves it.
- Confidence: high
- Reversal cost: low, the end-of-declaration check in `crates/una-check/src/types/`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-054`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional)
- Answer needed: Keep A?

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

### D-066: Policies: the `via` signature and the calls (M4)
- Question: `spec/10 §7` types a policy call `p(v)` as a declassification from `from` to `to`, computed by the `via` function. Nothing checks that the function takes `from` and returns `to`, or says how `p(v)` passes its argument.
- Context: `spec/07 §5`, `spec/09 §3`, `spec/10 §7`; ADR 0024 rule 14.
- Options: A) the `via` function takes one parameter of the base type of `from` and returns the base type of `to` (U2001 at `via` otherwise); a policy call takes one argument without a label (U2005 for a label) and fails and uses capabilities as its `via` function does; B) no check until M6.
- Chosen: A, because a mismatch would make the declassified value ill-typed at run time; labels stay with M6.
- Confidence: high
- Reversal cost: low, `Checker::policy` and `Checker::call_policy`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-066`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional), rule 14
- Answer needed: Keep A?

### D-067: Wrong numbers of arguments (M4)
- Question: U2004 to U2006 cover labels. Which code reports too many arguments, or a parameter that receives none?
- Context: `spec/04 §3`, `spec/10 §5` (T-Call); ADR 0024 rule 15.
- Options: A) U2001: "takes N arguments, but M are given" at the call, or "is missing argument `x`", which is not reported when a label error already explains the call; B) U2004.
- Chosen: A, because a wrong count is a mismatch between the call and the callee's type, while U2004 is about labels.
- Confidence: high
- Reversal cost: low, `Checker::arguments` and `Checker::call_value`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-067`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional), rule 15
- Answer needed: Keep A?

### D-069: Values whose type is not known yet (M4)
- Question: With unification, a value's type can still be unknown where a field, a method or a call needs it, for example `fn(x) -> x.len` passed where the element type is not fixed yet.
- Context: `spec/03 §7`; ADR 0024 rule 8; D-054.
- Options: A) check lambda arguments after the other arguments of a call or constructor, so that labeled arguments in any order still fix lambda parameter types; where a type is still unknown, report U2001 asking for an annotation; B) accept the access silently with the error type.
- Chosen: A, because B would let a wrong field pass unreported, and checking lambdas last removes the common case.
- Confidence: high
- Reversal cost: low, `lambdas_last` in `crates/una-check/src/types/call.rs` and `Checker::unknown_type`
- Implemented in: `0c07912` (M4 type checker), tag `DECISION D-069`
- ADR: `docs/adr/0024-typing-rules-left-open.md` (Provisional), rule 8
- Answer needed: Keep A?

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

### D-073: Where handler problems are reported (M5)
- Question: `spec/05 §5` requires every operation (U3011) with a matching parameter count and types from the capability (U3010). Where do these point, and what reports `for` naming something other than a capability?
- Context: `spec/05 §5`; ADR 0028 rule 3.
- Options: A) U3010 at `for` for a name that is not a capability, at an operation the capability lacks or with another parameter count; U3011 at the handler's name listing the missing operations; operation bodies fail only with the operation's error kind (U3009, U3008 for `pass`); B) one diagnostic per handler.
- Chosen: A, because each problem gets the site an agent edits.
- Confidence: high
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-073`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 3
- Answer needed: Keep A?

### D-074: Installing handlers with `with` and `given` (M5)
- Question: `with @x = h(args)` and `given @x = h(args)` must name a handler for the capability expected at `@x` (U3010), and the handler's own `uses` must be held by the caller (`spec/05 §4` rule 2), without a code.
- Context: `spec/02 §8`, `spec/05 §4`, `spec/08 §2`; ADR 0028 rule 4.
- Options: A) U3010 when `h` is not a handler or implements another capability than the one the statement's expression or the `do` line needs at `@x`; the handler's `uses` are checked like a callee's requirements (U3001, U3002); B) no check of the handler's `uses`.
- Chosen: A, because a handler with effects would otherwise act with authority its installer lacks.
- Confidence: high
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-074`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 4
- Answer needed: Keep A?

### D-077: Where `std.testing` may appear (M5)
- Question: `spec/14 §4` allows `std.testing` only in handler bodies and example lines (U3015). Is an import outside them an error, or only a use?
- Context: `spec/14 §4`; ADR 0028 rule 7.
- Options: A) a call of, or reference to, a `std.testing` function outside a handler operation body or an example is U3015, and imports are free; B) the import itself.
- Chosen: A, because modules mix handlers and functions, and the import serves the handlers.
- Confidence: high
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-077`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 7
- Answer needed: Keep A?

### D-078: Reporting the escape budget (M5)
- Question: `spec/06 §6` counts `checked` per module against `[escape_budget] checked_per_module` (U7003 when exceeded) without a site.
- Context: `spec/06 §6`, `spec/13 §7`; ADR 0028 rule 8.
- Options: A) the first use beyond the budget is U7003, with the later ones as related spans; the budget is 0 by default and for a single file; B) one diagnostic per use beyond the budget.
- Chosen: A, one diagnostic per module, which a change of the budget or of the code resolves.
- Confidence: high
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-078`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 8
- Answer needed: Keep A?

### D-080: Positions for one effect per line (M5)
- Question: `spec/04 §4` allows an effectful or fallible call only as the entire expression of a statement, `let` or final expression, "optionally wrapped in `attempt`". What about parentheses, `checked`, and function values?
- Context: `spec/04 §4`, `spec/08 §2` rule 5; ADR 0009; ADR 0028 rule 10.
- Options: A) parentheses are transparent; calls of functions, policies and function values with `uses` or `fails`, declared or inferred, and operations count; `checked` is not a call; example `do` lines are exempt; B) count only operations and named calls.
- Chosen: A, because a function value with effects is as effectful as a named function.
- Confidence: high
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-080`
- ADR: `docs/adr/0028-authority-rules-left-open.md` (Provisional), rule 10
- Answer needed: Keep A?

### D-082: Two checking passes for inference (M5)
- Question: Inferred headers need the call graph and the operations of every body, while typing the operations of a private function needs its inferred capabilities.
- Context: ADR 0007; `compiler/architecture.md §3`.
- Options: A) pass 1 runs the checker and collects authority facts, inference computes the headers, and pass 2 checks again with them and reports every diagnostic; B) infer syntactically before checking, resolving calls without types.
- Chosen: A, because method calls and function values need types to resolve, and the cost of a second pass is small.
- Confidence: high
- Reversal cost: low
- Implemented in: M5 authority commits, tag `DECISION D-082`
- ADR: none (minor)
- Answer needed: Keep A?

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
