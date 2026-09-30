# 0001. Review of the syntax reference guide against the v0.1 compiler

- **Status:** In progress
- **Date:** 2026-09-29
- **Found at:** commit `3301b05` on `dev/v0.1`, with Z3 at the version of `verification.lock`
- **Source:** the owner's "Una Language Guide: Syntax Reference (v0.1)", checked against the compiler and `docs/spec/`
  - Every code block of the guide was run through `una check`, `una test` or `una run`.
  - Every claim about syntax, diagnostics, the CLI and the project files was tested with a small package.
- **Outcome:** about 220 claims match. This file lists what does not:
  - two compiler defects (part A);
  - one spec ambiguity (part B);
  - errors and omissions in the guide (part C);
  - a sweep of the repository's own guides (part D).

## Instructions for the implementing agent

Read this whole file, then `CLAUDE.md`, before starting.

**Rules for the work**

- **Branch and process.** Work on `dev/v0.1`. Follow `CLAUDE.md`: read the spec section named by each item and quote it in the commit description.
- **Tests first.**
  - For a compiler item, add a conformance case under `tests/conformance/` that fails before the fix and passes after it (`docs/compiler/testing.md`).
  - Then fix the code.
  - Then run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`, with Z3 on the `PATH`.
- **Scope.** These are corrections to what v0.1 already ships.
  - They add no syntax, keywords, diagnostic codes, CLI commands or standard library items.
  - If an item turns out to need a spec change, write a `Proposed` ADR from `docs/adr/0000-template.md`, stop that item, and report it. Items marked `investigate` may end this way.
- **Commits.** One commit per item, or per group of `docs` items in the same file. Use the item id in the subject, for example `fix(interp): source names of operation arguments in traces (F-0001-01)`. Push after each verified commit.
- **Progress.** Update the progress table at the end of this file as each item closes, and set the file's status in `docs/fixes/README.md`.

**Where the guide lives.** Part C edits the owner's guide, which is not yet in the repository.

- Before starting part C, check for `docs/guide/syntax-reference.md`.
- If it is missing, ask the owner for the file, or for permission to add it at that path from the text they provide.
- Do parts A, B and D in the meantime.
- The guide is informative. When it disagrees with `docs/spec/`, the guide changes, never the spec.
- The guide follows the writing conventions of `CLAUDE.md`: American English, and no em dashes (the owner's text uses none).

---

## Part A. Compiler defects

### F-0001-01. Operation arguments in traces can carry the internal binder suffix

- **Kind:** compiler
- **Spec:** `spec/11 §5`, the effect trace. Each entry of `args` is keyed by the operation parameter's source name (`"args": { "key": "<redacted:internal>", "status": "Open" }`). The same shape appears in `una.test/1` and `una.run/1` (`spec/13 §5`).

**Evidence.** With this package, `una --json test .` prints `"args":{"k%2":"3"}` for the `get` operation where `"args":{"k":"3"}` is expected. The `put` entry is correct, and its secret argument is redacted as it should be.

```una
module p.m

pub kind E = | Bad

pub capability Store
  op put(k: secret[PII] Int, v: Int) -> Unit
    fails E
    accepts secret[PII]
  op get(k: Int) -> Option[Int]
    fails E

pub save(k: secret[PII] Int, v: Int) -> Option[Int]
  uses @s: Store
  fails E
=
  @s.put(k, v)
    if error pass
  @s.get(v)
    if error pass

handler mem() for Store
  op put(k, v) = unit
  op get(k) = Some(k)

example "saves and reads"
  given @s = mem()
  do or_else(save(7, v: 3), default: 0)
  expect result == 3
```

`una.toml` for the package: `[package]` with `name = "p"`, `version = "0.1.0"` and `edition = "0.1"`, plus `[labels] categories = ["PII"]`. Take `verification.lock` from `examples/events`.

**Cause, probably.** The two operations share a parameter name, so Core renames one binder to `k%2`. The trace takes the key from the Core binder without mapping it back to its source name.

- `crates/una-core/src/print.rs:112` already strips the `%N` suffix for printing.
- `crates/una-verify/src/generate.rs:2474` (`source_name`) does the same for counterexamples.
- The trace is built in `crates/una-interp/src/machine.rs`, near `render` and `redacted`. Use the operation's declared parameter names from the capability declaration, or strip the suffix as the other two sites do.

**Acceptance**

- A conformance case under `tests/conformance/runtime/` reproduces the package above and asserts the `una.test/1` trace keys `k` and `v` for `put` and `k` for `get`.
- The same holds for `una.run/1` traces.
- Redaction of labeled arguments is unchanged.
- No trace key anywhere contains `%`. Add an assertion over every existing runtime snapshot if the test harness allows it cheaply.

### F-0001-02. A function value with a refined parameter yields a meaningless obligation

- **Kind:** investigate, then compiler (or ADR)
- **Spec:**
  - `spec/03 §6`: function types are contravariant in their parameters.
  - `spec/10 §2`: rule S-Fun, where `Γ ⊢ τ̄2 <: τ̄1` is required of the parameters.
  - `spec/12 §3`: obligations, and the `refinement` kind.
  - `spec/12 §7`: counterexample bindings, "the parameters, `result`, named `let`s and pattern variables ... by name".

**Evidence**

```una
module p.m

pub kind Pos = Int where it > 0

pub apply(f: fn(Int) -> Int, x: Int) -> Int = f(x)

pub takes_pos(x: Pos) -> Int = x

pub main() -> Unit = unit

pub use_it() -> Int = apply(takes_pos, x: 0)
```

`una check` rejects the program, which is right: `fn(Pos) -> Int` is not a subtype of `fn(Int) -> Int`. But the diagnostic is wrong in three ways:

```text
[U5001] Error: refinement `takes_pos > 0` is refuted
  ...
  Help 1: ... strengthen the type of the value to a refined kind that implies `takes_pos > 0`
  Help 4: ... check the value with `checked(value, takes_pos > 0) else fail <Variant>`
  Note 2: counterexample:
```

1. **The goal substitutes the function's name for the refined value.** `takes_pos > 0` compares a function with an integer. The S-Fun premise `Int <: Pos` should produce an obligation over a fresh binder standing for the parameter, of the shape `true implies v > 0`.
2. **The counterexample is empty.** With a proper binder it would show one, such as `v = 0`, per `spec/12 §7`.
3. **The repairs do not apply to function values.** For example, `checked(value, takes_pos > 0)` makes no sense.

**What to do**

1. Find where S-Fun's parameter premise is turned into an obligation (`crates/una-verify/src/generate.rs` or the subtyping code in `crates/una-check`).
2. Check whether `spec/12` defines the obligation for a refined parameter inside a function-type flow: its binder, the goal text, the counterexample and the repairs.
   - **If the spec defines it:** fix the code to match, with a conformance case under `tests/conformance/verification/`.
   - **If the spec is silent, which is likely:** write a `Proposed` ADR. It should settle four things:
     - the binder name for parameter position `i` (for example the parameter's declared name);
     - the goal text;
     - that the counterexample binds that name;
     - which repairs of `spec/13 §3` apply (probably only "strengthen the type" of the expected function parameter, or none).
   - Then stop and report. Do not change the diagnostic until the ADR is `Accepted`.
3. **Related, in the same pass:** a result refinement in covariant position, such as passing `fn(Int) -> Int` where `fn(Int) -> Pos` is expected, must give an obligation with the same care. Test it and include it in the fix or the ADR.

**Acceptance.** Either the ADR is written and reported, or all of the following hold:

- the conformance case shows a goal over a named parameter binder;
- the counterexample is non-empty;
- no repair mentions the function's name as if it were a value;
- the covariant-result case is covered too.

---

## Part B. Spec ambiguity

### F-0001-03. Exit code of project-file errors (U7002, U7004)

- **Kind:** investigate (ADR)
- **Spec:** `spec/13 §5` says exit codes are `2` for a "usage or configuration error" and `1` for "errors in the program". An invalid `una.toml` (U7002) and a missing or invalid `verification.lock` (U7004) are configuration problems by that wording. The compiler reports them as diagnostics and exits with `1`.
- **Evidence:** `una check` on a package with an unknown `una.toml` key, or with no `verification.lock`, exits with `1`.
- **What to do:** do not change the code silently. Write a `Proposed` ADR that settles which exit code these diagnostics carry.
  - Keeping `1` needs `spec/13 §5` to say that "configuration error" means command-line configuration, such as flags and grants, not project-file diagnostics.
  - Moving to `2` changes the ABI for agents, which the ADR must justify.
  - Stop and report.
- **Acceptance:** the ADR exists with status `Proposed` and names both options.

---

## Part C. Errors and omissions in the syntax reference guide

Section numbers refer to the owner's guide, `docs/spec/`-relative references to the spec.

**How to verify each change.**

- A block that is a whole module (§0, §8.5 and every file of §17) must pass `una check` at tier 1.
- A block that is a fragment must stay consistent with the grammar in `spec/02`.
- Re-run the §17 package end to end after editing it.

### F-0001-04. §17: the complete package does not pass `una check`

- **Kind:** docs
- **Problem:** the imports of `src/events/checkin.una` are not in canonical order, so `una check`, `una test` and `una run` stop with U7001 ("lines 3 to 4 are not in canonical form"). Imports are sorted by module path (`spec/15 §7`).
- **Fix:** order them as `events.keys`, `events.seating`, `events.types`, `std.testing`. Check the other files of §17 and §8.5 the same way.
- **Acceptance:** with `keys.una` from §8.5 and the `una.toml` and `verification.lock` of §14, the package gives:
  - `una check`: 11 obligations proven, exit 0;
  - `una test`: 6 of 6 examples pass;
  - `una run --grant console`: prints `Meetup: open`, exit 0.

### F-0001-05. §1.2: line breaks inside brackets are not canonical

- **Kind:** docs
- **Problem:**
  - The guide presents splitting a call across lines inside `( )` as the way to write long argument lists. Its `add_all(` example fails `una check` with U7001.
  - The lexer accepts the split (`spec/01 §2`), but the canonical form joins continued lines into one, since v0.1 has no line width. A comment inside the brackets is the only exception (`spec/15 §2`).
- **Fix:**
  - Rewrite the example on one line.
  - Replace the bullet with the full rule: the parser accepts line breaks inside brackets, `una fmt` joins them, and `una check` reports the split as U7001, unless a comment sits inside the brackets.
  - Cross-reference §13.

### F-0001-06. §1.1: `\r\n` is accepted by the lexer but rejected by `una check`

- **Kind:** docs
- **Problem:** the table says `\r\n` line endings are normalized. That is true of the lexer, but `spec/15 §1` and `§10` make `\n` the only canonical line break. A file with `\r\n` gets one U7001 at the first occurrence, without an edit.
- **Fix:** say both in the table, and add that `una fmt` rewrites the file with `\n`.

### F-0001-07. §13: missing canonical-form rules

- **Kind:** docs
- **Problem:** §13 lists only part of what the formatter enforces.
- **Fix:** rewrite §13 from `spec/15` sections 1 to 10, keeping the guide's bullet style. At least these rules are missing:
  - **§1:** exactly one final line break, no blank line at the start or end of the file, and `\n` only.
  - **§2:** continued lines are joined, except with a comment inside, and tokens keep their source spelling (`_` groups, escapes).
  - **§3:** the spacing rules.
  - **§5:** a sum with two or more variants has no leading `|`; a single-variant sum keeps it.
  - **§6:** constructor fields in declaration order (already there) and how imported constructors are ordered.
  - **§7:** imports sorted by module path, and the names inside each import sorted.
  - **§8:** the blank-line rules between declarations and inside bodies.
  - **§9:** the comment rules.
  - **§10:** how U7001 is reported: one per changed region, with an edit, and the `\r\n` case.

### F-0001-08. §15: the interim note before tier 1 is stale

- **Kind:** docs
- **Problem:** the note "Until proof checking is implemented (milestone M8), `una run` and `una test` perform tier 0 checks only" describes ADR 0027, which ended with M8. `una run` and `una test` now check at tier 1 (`spec/13 §5`), and without Z3 at the pinned version they fail with U5003.
- **Fix:** remove the note. State that `run` and `test` check the package at tier 1 first, and that Z3 at the version of `verification.lock` is required.

### F-0001-09. §15: missing flags, statuses and rules

- **Kind:** docs
- **Fix:** complete the command table and the notes from `spec/13 §5`:
  - **Global flags:** `--color never|auto|always`.
  - **`una run`:**
    - `--grant` may repeat and is combined with `[run] grants`;
    - `--max-depth`;
    - the `una.run/1` statuses `ok`, `failed`, `aborted` and `rejected`;
    - the combined grant `files:<path>:read,write`;
    - relative grant paths resolve against the package root in `una.toml` and against the current directory in flags.
  - **`una check`:** the solver flags, including `--solver-wallclock`. Take the exact names and meanings from `una check --help` and `spec/13 §5`, and document only what the spec defines.
  - **`una contract`:**
    - it reads the package in the current directory;
    - an unknown name is a usage error with exit code 2;
    - a package with diagnostics prints them and exits with 1 (ADR 0029).
  - **Exit codes:** state that U7002 and U7004 currently exit with `1`, pending F-0001-03. Update this line once that ADR is decided.

### F-0001-10. §1.4: wrong case class is U0101, not U1005

- **Kind:** docs
- **Problem:** the guide files every naming problem under U1005, but only problems inside the right case class are U1005 (`f__x`, `g_`, `Bad_Name`, `Events_3`). A name in the wrong case class is a parse error, U0101:
  - `kind bad_kind` gives "expected a kind name";
  - a lowercase variant gives "expected a variant name";
  - `requirement events3` gives "expected a requirement identifier".
- **Fix:** split the paragraph into the two cases, with the messages above.

### F-0001-11. §1.5: the `macro` message

- **Kind:** docs
- **Problem:** the compiler and `spec/01 §5` print "`macro` is not supported (charter §6)". The guide's table leaves out "(charter §6)".
- **Fix:** copy the message exactly.

### F-0001-12. §3.2: "one of five forms"

- **Kind:** docs
- **Problem:** `spec/03 §2` says a `kind` declaration takes one of **four** forms. The guide says five and then shows six lines.
- **Fix:** name the four forms of the spec: alias, refinement (with range sugar as a case), sum (with or without payloads) and record. Present generic kinds as parameters any form may take.

### F-0001-13. §3.4: variants in the refinement language use `is`, not `==`

- **Kind:** docs
- **Problem:** the table lists `==` and `!=` without restriction, but `result != Open` in an `ensures` is U2014: "comparing with variant `Open` is outside the refinement language".
- **Fix:** add a line to the table saying variants are tested with `is` (`result is SoldOut`, `not (result is Open)`) and that `==` or `!=` against a variant is U2014.

### F-0001-14. §4.4: constructor field order, and the unlabeled field

- **Kind:** docs
- **Problem:**
  - "Every field labeled, in any order" is true of the type checker. But `una check` reports a constructor with fields out of declaration order as U7001 until `una fmt` rewrites it.
  - In a constructor with two or more fields, an unlabeled field is U2004 ("every argument needs its field's label"), not U2009.
- **Fix:** say that any order type-checks but the canonical form is declaration order, and that `una check` rejects the other orders as U7001. Add the U2004 case to the constructor rules.

### F-0001-15. §4.4: when U2006 applies

- **Kind:** docs
- **Problem:** a duplicate labeled argument is U2006 only when the argument count is right, as in `f(a: 1, a: 2)` for a two-parameter `f`. With one argument too many, as in `f(1, b: 2, b: 3)`, the compiler reports U2001 "`f` takes 2 arguments, but 3 are given" and no U2006.
- **Fix:** add the argument-count rule (U2001) to the table, and say it is checked before the labels.

### F-0001-16. §4.6 and §7.6: a lambda cannot call an operation that fails

- **Kind:** docs
- **Problem:**
  - The guide says effectful or fallible lambdas may be passed as arguments.
  - But a lambda body is one expression and has no place for `if error`, so a lambda that calls a fallible operation, such as `fn(k) -> @audit.record(k)` with `record` declaring `fails`, is U3007.
  - The prelude's higher-order functions take only pure lambdas: an effectful lambda passed to `map` is rejected.
- **Fix:** state that an effectful lambda passed as an argument can call only operations that cannot fail, and that fallible work belongs in a named function with handling clauses. Give the U3007 case.

### F-0001-17. §7.4: `with` and the handler's own `uses`

- **Kind:** docs
- **Problem:**
  - "The handler's own `uses` must be available to the caller" names no diagnostic. What happens depends on the caller:
    - a `pub` caller that lacks the capability gets U3003, "handler `h` uses `@inner` ... its effects would exceed its header";
    - a private caller has the `uses` inferred, and the error appears where the capability is really missing, for example U3001 in an example: "`g` needs `@inner`, which no `given` line binds".
  - Also, `with` is part of a statement (`spec/02 §6`), so it cannot appear in the short function form: `g() -> Unit = f() with @a = h()` is U0101.
- **Fix:** add both points to §7.4, and add the short-form restriction to §4.1.

### F-0001-18. §11.2: the repair "add a `requires`" has a condition

- **Kind:** docs
- **Problem:** the compiler offers the repair only when the enclosing function is not `pub`: "add `requires p` to the enclosing function, if it is not `pub`; its callers then prove it".
- **Fix:** write the ranked repairs as the compiler words them (`spec/13 §3`), with their escape costs: 0, 0, 0, and 1 for `checked`.

### F-0001-19. Consistency pass over the whole guide

- **Kind:** docs
- **Fix:**
  - After F-0001-04 to F-0001-18, re-run every whole-module block (§0, §8.5 and every file of §17) at tier 1.
  - Check that every diagnostic code the guide names exists in `spec/13` with the meaning given.
  - Check that the common-mistakes table of §16 still gives the stated code for each row. All 16 testable rows passed at `3301b05`.
  - Update the guide's status line to the date of the edit.

---

## Part D. The repository's own guides

### F-0001-20. Sweep `README.md` and `docs/guide/una-for-agents.md` for the same errors

- **Kind:** docs
- **Problem:** both files are tested by `crates/una-cli/tests/guide.rs`, so their code blocks pass. But their prose may repeat the errors of part C. The likeliest are:
  - the interim tier 0 note (F-0001-08);
  - line breaks inside brackets (F-0001-05);
  - "any order" for constructor fields (F-0001-14);
  - U1005 for every naming problem (F-0001-10);
  - the `macro` message (F-0001-11);
  - fallible lambdas (F-0001-16).
- **Fix:** search both files for each topic and correct any prose that disagrees with `docs/spec/`. The tests in `guide.rs` must still pass.
- **Acceptance:** `cargo test -p una-cli --test guide` passes, and the commit lists what changed or states that nothing needed to.

---

## Needs review

Disagreements between the spec, the implementation and the owner's rulings, found while doing this file. They are not resolved here; the answers of the review decide them.

- **R-1. Exit code 1 without a diagnostic (ADR 0036).** The owner's rule says exit `1` means the output contains at least one error diagnostic with a code. But `una obligations`, `una trace` and `una report` exit with `1` for an obligation that is not proven, and `una test` for a failed example, with no diagnostic in their output: their documents carry the status instead. `spec/13 §5`, `spec/08 §1` and ADRs 0033 and 0034 require this, and `spec/13 §5` now states it as part of the rule. Should these outcomes also carry a diagnostic, or does the rule keep them as stated?
- **R-2. The code for an unsupported refinement in a function type (ADR 0037, rule 10).** The owner requires an explicit diagnostic, but `CLAUDE.md` forbids diagnostic codes the spec does not define, and no registered code of `spec/13 §4` names this case. Is it an existing code (for example a type error of the `types` domain), or does the closure run register a new one in `spec/13 §4` under ADR 0037?
- **R-3. Argument count before labels (F-0001-15).** `spec/04 §3` rule 2 makes a duplicate label U2006, and `spec/10 §12` rule 13 makes too many arguments U2001; neither says which is checked first. For `f(1, b: 2, b: 3)` with a two-parameter `f`, the compiler reports only U2001. The guide (§4.4) now describes that behavior. Should the spec state the order, or should both codes be reported?
- **R-4. Repair candidates differ from `spec/13 §3` (F-0001-18).** ADR 0033, in `spec/13 §3`: "a refuted or unknown `refinement` or `precondition` proposes, in order, a stronger parameter type when the value is a parameter, a `requires` clause for a private function, a guard, and `checked`; ... a `division` a guard". The compiler instead:
  - proposes "strengthen the type of the value" when the value is not a parameter (for example a call's result, or a function passed as a value);
  - proposes "add `requires G` to the enclosing function, if it is not `pub`" inside `pub` functions too;
  - proposes, for a `division`, "add `requires b != 0` to the enclosing function, if it is not `pub`" after the guard.
  The guide (§11.2) quotes the compiler's wording. Depending on the answer, the compiler or the spec changes, and the guide follows.
- **R-5. `--solver-wallclock` is not in the command table of `spec/13 §5`.** `spec/12 §5` defines the flag (default 60 s); the compiler accepts it on `check`, `run`, `test`, `obligations`, `trace` and `report`, which the guide (§15) lists. Should `spec/13 §5` list it with those commands?

## Closure run

Items scheduled for the v0.1 closure run, after the review freeze ends. They change compiler code or tests, which this file's docs work does not touch.

- **F-0001-02.** Implement ADR 0037 (Accepted), with its conditions: quantifier-free obligations over fresh variables, an explicit diagnostic for any refinement in a function type that cannot be supported, an internal check that no refinement is dropped between the checker and Core IR, and conformance cases for the correct program accepted, the mismatched lambda annotation rejected, and the covariant result with a counterexample.
- **C-1. `una contract` and `una trace` hide project-file diagnostics (ADR 0036).** With an invalid `una.toml` or no `verification.lock`, an unknown name exits with `2` ("the package has no function ...", "the package has no requirement ...") and U7002 or U7004 is never reported. Every other command reports them with exit `1`. Load and check the project files before resolving the name, so that the diagnostics come first, and add a CLI test for both commands.

## Progress

| Id | Kind | Title | Status | Commit |
| --- | --- | --- | --- | --- |
| F-0001-01 | compiler | Binder suffix in trace argument keys | Done | see `git log --grep F-0001-01` |
| F-0001-02 | investigate | Obligation for a refined parameter of a function value | ADR 0037 Accepted; implementation scheduled for the closure run | see `git log --grep F-0001-02` |
| F-0001-03 | investigate | Exit code of U7002 and U7004 | Done: ADR 0036 Accepted, `spec/13 §5` updated; one code gap in the closure run | see `git log --grep F-0001-03` |
| F-0001-04 | docs | §17 package import order | Done | see `git log --grep F-0001-04` |
| F-0001-05 | docs | §1.2 line breaks inside brackets | Done | see `git log --grep F-0001-05` |
| F-0001-06 | docs | §1.1 `\r\n` and U7001 | Done | see `git log --grep F-0001-06` |
| F-0001-07 | docs | §13 missing canonical-form rules | Done | see `git log --grep F-0001-07` |
| F-0001-08 | docs | §15 stale interim note | Done | see `git log --grep F-0001-08` |
| F-0001-09 | docs | §15 missing flags, statuses and rules | Done; `--solver-wallclock` listing under R-5 | see `git log --grep F-0001-09` |
| F-0001-10 | docs | §1.4 U0101 versus U1005 | Done | see `git log --grep F-0001-10` |
| F-0001-11 | docs | §1.5 `macro` message | Done | see `git log --grep F-0001-11` |
| F-0001-12 | docs | §3.2 number of kind forms | Done | see `git log --grep F-0001-12` |
| F-0001-13 | docs | §3.4 variants use `is` | Done | see `git log --grep F-0001-13` |
| F-0001-14 | docs | §4.4 constructor field order and U2004 | Done | see `git log --grep F-0001-14` |
| F-0001-15 | docs | §4.4 U2006 versus U2001 | Done; order of checks under R-3 | see `git log --grep F-0001-15` |
| F-0001-16 | docs | §4.6, §7.6 fallible lambdas | Done | see `git log --grep F-0001-16` |
| F-0001-17 | docs | §7.4 handler `uses` and `with` in the short form | Done | see `git log --grep F-0001-17` |
| F-0001-18 | docs | §11.2 repair wording | Done; compiler wording quoted, differences under R-4 | see `git log --grep F-0001-18` |
| F-0001-19 | docs | Consistency pass | Done | see `git log --grep F-0001-19` |
| F-0001-20 | docs | Sweep of `README.md` and `una-for-agents.md` | Done | see `git log --grep F-0001-20` |
