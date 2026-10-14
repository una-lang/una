# 0105. The test declaration is written `test` in edition 0.3

- Status: Proposed
- Date: 2026-10-14
- Deciders:
- Resolves: issue #740
- Target: edition `0.3`. Editions `0.1` and `0.2` do not change: they keep `example`, their keyword tables and `una.test/1`. The Gate 1 pin (v0.1.9, edition `0.1`) does not move.
- Amends, in edition `0.3`: `spec/01 §5` (keywords), `spec/02 §5` (`example_decl`), `spec/08 §2`, `spec/13 §5` (`una test`), and the wording of the rules that name examples (`spec/05`, `spec/07 §4`, `spec/14 §4`); `docs/guide`

## Context

A package states its executable evidence with a declaration written `example` (`spec/08 §2`):

```una
example "an unknown attendee fails and is not audited"
  given @registrations = no_registrations()
  do check_in(sample_phone())
  expect fails NotRegistered
```

The declaration is a test: it installs handlers, runs code in a fresh machine, checks expectations, and fails the run of `una test`. The word `example` reads as documentation, as a sample of use that nothing checks. A reader or an agent new to the language takes it for prose, and looks elsewhere for the tests. The command that runs it is already called `una test`, and its library is `std.testing`.

The project has decided that edition `0.3` writes the declaration `test`, and that editions `0.1` and `0.2` keep `example`.

The constraints:
- **Editions keep their promise** (`spec/13 §7`). A program of edition `0.1` or `0.2` is read the same way by every later release.
- **Edition `0.3` is a preview that only gets stricter.** A new construct must give, once erased, a program that edition `0.2` accepts, with no fewer allowed behaviors.
- **No synonym.** Una has one way to write each construct (charter §6). Edition `0.3` accepts `test`, not both words.
- **Migration is mechanical.** `una fix` applies exact rewrites (ADR 0096 part A), so an edition `0.3` package that still says `example` is fixed by one command.

**The corpus** (`main` at 5cc15672, `una` and `una-bench`):
- 80 packages of edition `0.3` hold 3 `example` declarations. The rest of the 8,203 `.una` files scanned are of edition `0.1` or `0.2`, or single files, which are edition `0.1`.
- `test` as an identifier, outside comments and text literals, occurs 8 times, in 4 files: a generic function `test[T]` and a local `let test`. All 4 are of edition `0.1` or `0.2`, where nothing changes. No package of edition `0.3` uses `test` as a name.
- The documents' `una` blocks (407) use `test` as a name nowhere, and write 30 `example` declarations.

## Decision

1. **The declaration.** In edition `0.3`, the test declaration is written `test`:

   ```ebnf
   test_decl = "test" text_lit NEWLINE INDENT
                 { "given" cap_ref "=" handler_call NEWLINE }
                 "do" line_expr NEWLINE
                 expectation { expectation }
               DEDENT ;
   ```

   The body is `example_decl`'s, unchanged: `given`, `do` and `expect` keep their grammar and meaning. Every rule that `spec/08 §2` and the other chapters state of an example holds of a test, and of nothing else:
   - host capabilities (U3014) and `std.testing` (U3015, `spec/14 §4`);
   - the sink exemption of `spec/07 §4`;
   - the verification of `do` and `expect` lines (`spec/12 §3`);
   - names unique per module (U1002);
   - the fresh abstract machine per run.

   In editions `0.1` and `0.2` the declaration stays `example`, and `test` stays an identifier.

2. **Keywords** (`spec/01 §5`). In edition `0.3`:
   - `test` is a reserved keyword, in the table of keywords that cannot be identifiers;
   - `example` stays a reserved keyword, so a name `example` is still an error, and the word cannot come back with another meaning.

   The table of edition `0.3` gains one row for `example`, with the message of rule 3.

3. **`example` in edition `0.3`.** An `example` declaration in an edition `0.3` package is U0103, at the keyword, with the message "`example` is written `test` in edition 0.3".
   - Its first repair is an exact rewrite (ADR 0096 rule 2) that replaces the keyword `example` by `test`, and nothing else. It meets (a) to (d) of that rule:
     - it is computed from the declaration's first token;
     - the result parses, since the declaration's body is unchanged and `test_decl` has the same body;
     - it is the same declaration;
     - it is deterministic.
   - The rewrite has `escape_cost` 0 and widens nothing, so it is a candidate of `una fix` (ADR 0084), and `una check` counts it in `fixable` (ADR 0096 rule 6). One `una fix` run migrates a package.
   - No other code is added. The keyword is a construct of the wrong edition, which is what U0103 reports.
   - **Recovery.** After the U0103, the parser reads `example` at the start of a declaration as `test_decl`. Today a reserved word that reaches the parser is only read as a name or skipped, so without this rule the declaration would also get a U0101, and its body would be skipped. With it, the body is parsed, type-checked and flow-checked as a test's. Its obligations (`spec/12 §3`) are verified once the keyword is fixed, since tier 1 runs only on a clean tier 0. The U0103 is then the only diagnostic the keyword gets, and a tier-0 error in the body is reported beside it.
   - **What the rewrite leaves.** It replaces one token, so the doc comments, comments and attributes attached to the declaration stay as they are. The canonical form survives it: the body's indentation is one fixed level, not aligned to the keyword, and a text literal never wraps, so a header three characters shorter cannot change the layout. Other edits on the same line in the same round are taken by ADR 0084 rule 3, as for any edit: they do not overlap the keyword and are applied by offset, so the shorter keyword moves none of them.
   - **Rounds of `una fix`.** The U0103 is a tier-0 diagnostic, so a round that takes it does not check tier 1 (ADR 0084, amendment of 2026-10-14). A test whose `do` line needs a callee's condition (ADR 0092's amendment of the same date) is therefore repaired in two rounds of one run: the keyword first, then the condition once tier 0 is clean.

4. **`test` as a name in edition `0.3`.** A local, a parameter, a function, a field, a variant's field, a handler or a predicate named `test` is the U0101 of a keyword used as a name, as `result` or `error` is today.
   - ADR 0096's row for `result`, `error`, `unit` and `it` gains `test` in edition `0.3`. A local or a parameter named `test` gets the exact rename to `test_value`, or the first of `test_value2`, `test_value3`, … that ADR 0096 rule 3a allows, in its scope.
   - A function, a field, a handler, a predicate or a module segment named `test` gets the rename as a description, with `edit: null`. Renaming those reaches every caller, every constructor or every import, and a field's name is part of the data a program shows, so the change is the author's.
   - **Modules.** A module path segment comes from a file or directory name (`src/test.una`, `src/test/…`), so in edition `0.3` its `module` header and every `use` that names it are U0101. The repair is a description, "rename the file or directory", which `una fix` cannot apply, since it writes only module texts (ADR 0084 rule 2). A directory named `test` for a package's own tests is a common habit, so the guide's page on packages says it. The standard library has no member named `test` (`spec/14`), so no prelude or `std` name changes.
   - The corpus has no such name in an edition `0.3` package (Context).

5. **The erasure, and the preview rule** (`spec/13 §7`).
   - The erasure of this construct writes `example` for the keyword of every `test` declaration. For an edition `0.3` package that this rule accepts, that gives a package whose declarations edition `0.2` accepts, since `example_decl` and `test_decl` have the same body and the same rules, and with the same behavior: `una test` runs the same declarations in the same order, with the same outcomes and traces.
   - So the construct only narrows. Edition `0.3` gains errors (U0103 for `example`, U0101 for a name `test`) and accepts nothing that edition `0.2` rejects once erased.
   - The keyword `test` at the start of a declaration is a syntax error in earlier releases (U0101), so it is one of the constructs the preview rule admits.

6. **`una test` output** (`spec/13 §5`).
   - **Human output.** For a package of edition `0.3`, each line reads `test <module> "<name>" ... ok` or `... FAILED`, and the summary `<n> tests: <p> passed, <f> failed`. Editions `0.1` and `0.2` keep `example` and `examples`. Diagnostic messages that name the declaration say "test" in an edition `0.3` package and "example" in the others, with the same codes.
   - **JSON.** A package of edition `0.3` prints the schema `una.test/2`: the document of `una.test/1`, with the key `examples` renamed `tests`, holding the same objects in the same order with the same keys. Renaming a key changes the document's meaning for a consumer, so the version changes (ADR 0076 rule 2).
   - Packages of edition `0.1` and `0.2` keep `una.test/1`, byte for byte. So the harness of the Gate 1 benchmark, which reads `una.test/1` from edition `0.1` packages, does not change.
   - A consumer that reads both editions dispatches on `schema`, as ADR 0076 asks. The known consumers of the key `examples`:
     - in this repository: the conformance runner (`tests/conformance/main.rs`, behind its test-pass and test-fail directives, which edition `0.3` cases reach), `crates/una-cli/tests/runtime.rs` with its snapshot of the JSON events, and `scripts/demo/checkpoint-3.sh`;
     - in `una-bench`: `harness/bench/oracle.py`, which reads edition `0.1` packages for Gate 1 and is unaffected, and `scripts/build_primer.py` and `scripts/native-dogfood/dogfood.py`, which must dispatch on `schema` before an edition `0.3` package goes through them.

     The implementation updates the first group, and the second group is updated in `una-bench` before it ships.

7. **What keeps the word "example".** These are not renamed:
   - **Core IR and its export.** The Core node of a test, and the key `examples` of a module in `una.core/1` (`spec/13 §8`), keep their names. The export already carries `edition`, and the node is the same in every edition. Renaming the key would change `una.core/1` and the importer of the formal track for no gain in meaning.
   - **`std.testing`**, `una test` and the U-codes keep their names. U3014 and U3015 keep their codes. Their messages and `explain` texts name "a test (an `example` in editions 0.1 and 0.2)".
   - **The project's examples directory** (`examples/`), whose packages are sample programs, and the word "example" in prose that means a sample, as in "for example".

8. **The formatter** (`spec/15`). The canonical form prints the keyword as written. `una fmt` never rewrites `example` to `test`: it changes layout only, and the rewrite is a repair that `una fix` applies (rule 3). A canonical `test` declaration has the layout of a canonical `example` declaration.

9. **The language server** (ADR 0063). In an edition `0.3` package:
   - completion offers `test` and not `example`;
   - semantic tokens mark `test` as a keyword;
   - the outline lists a test declaration by its name, with the symbol kind it has today;
   - the U0103 and its repair appear as any diagnostic and code action does.

   The server takes each file's edition from its own package root, as package discovery does: the keyword tables are chosen per file's package, never per session. So in a session with an edition `0.2` and an edition `0.3` package open, each gets its own keywords. A single file is its own package with the default settings, and so of edition `0.1` (ADR 0021 rule 1, `spec/20`), so a lone file keeps `example`. Editions `0.1` and `0.2` see no change. The outline is unaffected: it lists a test by its name, with the symbol kind it has today.

10. **The documents.**
    - `spec/08 §2` is titled "Tests", writes its sample with `test`, and states that editions `0.1` and `0.2` write `example`. The chapter's title becomes "Requirements and tests".
    - The other chapters' rules about examples say "test", with the same note once per chapter where the word first appears.
    - The guide writes `test` in its edition `0.3` pages and keeps `example` where a page is about editions `0.1` or `0.2`. `docs/guide/language/examples/example.md` becomes `test.md`, with a redirect note at the old path.
    - **Una by Example** (#733) keeps its title. Its chapters default to edition `0.2`, so its chapter on testing shows `test` with the edition `0.3` preview badge, beside the `example` form of edition `0.2`. The title names the learning format (a sequence of examples, as Rust by Example), not the declaration.

11. **Diagnostic codes.** None is added. U0103 gains a row for `example` in the edition `0.3` table (rule 3), and U0101 covers a name `test` (rule 4). The registry's meaning of U0103, "Reserved keyword not available in v0.1", has also covered the keywords of other editions since ADR 0056. It becomes "Reserved keyword not available in the package's edition". Only the description changes, not the code's use.

## Complexity

- **Lexer and parser.** The keyword set is chosen per edition, as today: one lookup per identifier, O(1) on average. The check of rule 3 is one token.
- **Repairs.** The rewrite of rule 3 is one token: O(1). The rename of rule 4 is ADR 0096's, linear in the scope of the binding.
- **`una test`.** The schema is chosen once per run, by the package's edition. The output is the same size.

Nothing grows with the program beyond what the existing passes cost.

## Tests

- An edition `0.3` package with a `test` declaration: it checks, and `una test` runs it with the outcomes and traces of the same package written with `example` in edition `0.2` (rule 5). `una.test/2` and the human lines say "test".
- An edition `0.3` package with an `example` declaration: U0103 with the message of rule 3. `una fix` rewrites it to `test`, and the package then checks and runs as above. `una check --json` counts it in `fixable`.
- Editions `0.1` and `0.2`: `example` unchanged, `test` usable as a name (the 4 corpus files of the Context), `una.test/1` byte-identical to the current release's on every conformance case and example.
- A local, a parameter and a function named `test` in edition `0.3`: U0101. The first two get the exact rename, and the third gets a description with `edit: null`.
- `example` as a name in edition `0.3`: still an error, as in earlier editions.
- U3014 and U3015 in an edition `0.3` test, with messages that say "test".
- The language server: completion, semantic tokens and the outline in an edition `0.3` and an edition `0.2` package open in one session, and in a single file.
- The formatter: a canonical `test` declaration is a fixed point of `una fmt`, and `una fmt` leaves an `example` declaration in edition `0.3` as it is. The result of `una fix` on a canonical `example` declaration is a fixed point of `una fmt`.
- Recovery: an `example` in edition `0.3` whose body has a tier-0 error reports the U0103 and the body's error, and no U0101.
- Rounds: an edition `0.3` package with an `example` whose `do` call needs a callee's condition. One `una fix` run gives a package that checks.
- Modules: a file `src/test.una` in edition `0.3` gives U0101 at its `module` header and at each `use` of it, with the description "rename the file or directory".
- Consumers: the conformance runner reads `una.test/2` for edition `0.3` cases and `una.test/1` otherwise, by `schema`. The guide's `una` blocks on edition `0.3` pages are checked as edition `0.3` (`crates/una-cli/tests/guide.rs`).

## Consequences

- An edition `0.3` package reads as it means: its tests are written `test`, and `una test` runs them.
- Edition `0.3` packages that use `example` gain one error each, fixed by `una fix`. The corpus has 3.
- Editions `0.1` and `0.2`, the Gate 1 pin and `una.test/1` do not change.
- Tools that read `una test --json` from edition `0.3` packages move to `una.test/2`. Core IR and its export keep "example" (rule 7), so the formal track does not change.
- The documents mention both words once per page where both editions matter.

## Alternatives considered

- **Accept both `example` and `test` in edition `0.3`.** Not chosen: two words for one construct, which the charter rules out (§6), and a reader still meets `example` and takes it for documentation.
- **Rename in every edition.** Not chosen: editions `0.1` and `0.2` promise that a later release reads their programs the same way (`spec/13 §7`), and Gate 1 is pinned to edition `0.1`.
- **Keep `una.test/1` for edition `0.3` too,** with the key `examples`. Not chosen: the output would keep the old word in exactly the place agents read, the JSON, and the schema can change cleanly per edition, as the Core export and the report already carry the edition.
- **Rename the key in `una.core/1` as well.** Not chosen: Core IR is internal to the toolchain and the formal track, the node does not change between editions, and the change would cost a schema version and an importer change for no meaning.
- **A new diagnostic code for `example` in edition `0.3`.** Not chosen: U0103 already reports a keyword of another edition, with a message per keyword (`spec/01 §5`).
- **`check` or `case` as the keyword.** Not chosen: `checked` already exists (`spec/06 §6`), `case` reads as a branch of `when`, and `test` matches `una test` and `std.testing`.
