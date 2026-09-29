# Morning report

Updated after every milestone of the overnight run (`OVERNIGHT.md`, C.8).

## 1. Status

- Language name: Una (ADR 0025; previously Troth, codenamed Vera).
- Last completed milestone: M4 (types). Next: M5 (capabilities, effects and failures), on the critical path to checkpoint 3 (`docs/plan/roadmap.md §4`).
- Repository: una-lang/una (private until launch), branch `dev/v0.1`; its history starts at the initial public import `25d865b` (ADR 0030). Earlier commit hashes in this report refer to the legacy repository.
- Last green CI runs: https://github.com/una-lang/una/actions/runs/36580294816 (`main`, Linux and macOS) and https://github.com/una-lang/una/actions/runs/36580295703 (`dev/v0.1`, Linux), both at `25d865b`. Before the import: run 36578625540 of the legacy repository (`a5c8d3d`, M4).

## 2. Consolidated instructions: Una, neutral examples, checkpoints, license

Carried out in order on 2026-09-29, on `overnight/v0.1`.

| Part | Status | Commits and evidence |
| --- | --- | --- |
| 0. Clean state and inventory | done | see below |
| 1. Final name: Una | done | `1fa2523`; ADR 0025 (Accepted) supersedes ADR 0016; D-058 |
| 2. Neutral reference examples | done | `9101398`; ADR 0026 (Accepted); `examples/events`; D-059, D-060 |
| 3. Usage checkpoints | done | `a3ac631`; `docs/plan/roadmap.md §4`; checkpoint 1 demo (§8); ADR 0027 (Accepted) |
| 4. License | done | `580882e`; Apache-2.0 |
| 5. Publish to una-lang/una | done | first deferred by the owner, then run on the owner's correction: `25d865b` imported as a root commit and pushed without force to `main` and `dev/v0.1`; remotes `origin` (una-lang/una) and `legacy`; ADR 0030 (Accepted); CI quota rule D-083 |
| 6. Resume the run | done | `OVERNIGHT.md` Part D and Progress, before and after the publication; M4 completed, M5 resumes on `dev/v0.1` |

**Part 0 inventory**, taken before any change:
- Branch `overnight/v0.1`, last commit `fc9301b` ("docs(plan): M4 plan, ADR 0024 on typing rules, decisions D-047 to D-057"); `37b5309` and `fc9301b` were not yet pushed.
- Remotes: `origin`, the legacy private repository (fetch and push).
- Milestone M4 (types), task 2 (types and the prelude), in progress.
- Last CI: run 36568569555, green on Linux and macOS, at `f98912a` (M3).
- Language name: Troth.
- `git ls-remote https://github.com/una-lang/una.git`: exit 0 and no refs, so the repository exists and is empty; Part 5 was not blocked.
- The commit in progress (M4 task 2) had never been compiled and needed many more steps, so its uncommitted changes were taken off the branch, as Part 0.1 says. Set aside, not committed: the prelude and `std.testing` sources, the type representation, the environment with the lowering of written types (U2002, U2003), unification variables and the first part of the declaration checker (`crates/*-check/src/std/`, `crates/*-check/src/types/`). A scratch copy stays in the ignored `target/` directory, so M4 task 2 resumes from it instead of from nothing.

**Part 1.4 audits**, after all four commits. `grep -rniwE "vera|troth" --exclude-dir=.git --exclude-dir=target .` hits only the files allowed by Part 1.3 (count of matching lines per file):

```text
OVERNIGHT.md:1
docs/adr/0016-final-name-troth.md:15
docs/adr/0025-final-name-una.md:12
docs/reviews/morning-report.md:27
docs/reviews/open-decisions.md:14
docs/reviews/triage.md:5
```

`grep -rnE "\b[VT][0-9]{4}\b" --exclude-dir=.git --exclude-dir=target .` hits only allowed files as well:

```text
docs/adr/0016-final-name-troth.md:1
docs/reviews/morning-report.md:2
docs/reviews/open-decisions.md:2
```

The counts grow as this report quotes the names. Tests that check that the codes of the earlier names are rejected write them in two pieces (D-058).

**Part 2.5 audit.** The whole-word domain search returns nothing, and no hit sits inside third-party license text. No document names a company, an employer or a real policy document; the only external links point to prior-art projects, the Z3 releases and the owner's own repository.

**Part 5.3 audits**, on the staged tree of the initial import, after `git checkout --orphan public-main`:
- `git grep --cached -liwE "vera|troth"`: `OVERNIGHT.md`, `docs/adr/0016-final-name-troth.md`, `docs/adr/0025-final-name-una.md`, `docs/reviews/morning-report.md`, `docs/reviews/open-decisions.md`, `docs/reviews/triage.md`, all allowed by Part 1.3.
- The code-shape audit of Part 1.4: `docs/adr/0016-final-name-troth.md`, `docs/reviews/morning-report.md`, `docs/reviews/open-decisions.md`, all allowed.
- The domain audit of Part 2.5: no output.
- The search of Part 5.3 for the path of the legacy repository, on the staged tree: no output. The two references in this report became "the legacy repository" before the import.

**The new repository:** https://github.com/una-lang/una, private. GitHub Actions runs there: the push to `main` ran Linux and macOS (run 36580294816), the push to `dev/v0.1` Linux only (run 36580295703), both green, as D-083 intends. `Cargo.toml` names it as `repository`, and the README's build section clones it.

**Notes.**
- A slip after the import: commit `83c7ff4` quoted the Part 5.3 search command, which contains the legacy repository's path, in this report. The next commit reworded the line; published history is not rewritten, so `83c7ff4` keeps it.
- The rename applied mechanically to accepted ADRs and the charter (names, codes, fences), and the neutral example replaced identifiers in accepted ADRs where they appeared; no substantive content of an accepted ADR changed, except that ADR 0016 is now `Superseded by 0025`, as Part 1.1 allows.
- Snapshots: the reference example's five syntax-tree snapshots and the three snapshots of the `spec/13 §1` diagnostic sample were regenerated once and reviewed line by line (D-059).
- Conformance counts are unchanged through Parts 1 to 4: 92 cases, 89 passed, 3 pending (M5, M6, M8). Tests: 204, plus the end-to-end test of the demo scripts, 205.

## 3. Part A: rename to Troth (history, with the names of the time)

- Commit: `c198b65` "chore: rename language to Troth (ADR 0016)", on top of `7920c27`, pushed to `main` (the only push to `main` in this run).
- CI: run 36517282489, success on `ubuntu-24.04` and `macos-26`.
- Checks: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass (65 tests); the conformance run reports 3 cases, 0 passed, 3 pending (M5, M6, M8), with `.troth` file names.
- Derived rename: the Vera Abstract Machine (VAM) became the Troth Abstract Machine (TAM) (D-001).

`grep -rni "vera" --exclude-dir=.git --exclude-dir=target .` returns 25 lines:
- 12 in ADR 0016 and 6 in the prior-art section of `docs/reviews/triage.md`, both allowed by A.3;
- 7 that only contain the letters inside the English words "several" (3) and "coverage" (4) (D-003).

At `c198b65`, the whole-word search (`git grep -niw vera`) returns only the 18 allowed lines, in the same two files. Files added later by this run (`OVERNIGHT.md`, this report and the decision log) quote the old name where they describe the rename.

```text
docs/adr/0016-final-name-troth.md:9:The project was developed under the codename Vera. "Vera" collides with github.com/aallan/vera, an unrelated public language for machine-written code that uses the same CLI name (`vera`) and the same file extension (`.vera`) (`reviews/triage.md §5`). A shared CLI name and extension would confuse agents, package tooling and search, and the Agent ABI schema names (`vera.diagnostic/1` and others) would freeze the codename into every consumer once M9 stabilizes them.
docs/adr/0016-final-name-troth.md:17:| language name Vera | Troth |
docs/adr/0016-final-name-troth.md:18:| crates `vera-*` | `troth-*` |
docs/adr/0016-final-name-troth.md:19:| binary `vera` | `troth` |
docs/adr/0016-final-name-troth.md:20:| extension `.vera` and code fences `vera` | `.troth` and `troth` |
docs/adr/0016-final-name-troth.md:21:| `vera.toml` | `troth.toml` |
docs/adr/0016-final-name-troth.md:22:| `.vera/cache` | `.troth/cache` |
docs/adr/0016-final-name-troth.md:23:| Agent ABI schemas `vera.diagnostic/1`, `vera.diagnostics/1`, `vera.obligations/1`, `vera.report/1` | `troth.diagnostic/1`, `troth.diagnostics/1`, `troth.obligations/1`, `troth.report/1` |
docs/adr/0016-final-name-troth.md:24:| lock `encoding_version = "vera-smt-1"` | `troth-smt-1` |
docs/adr/0016-final-name-troth.md:27:Names derived from the language name follow it: the Vera Abstract Machine (VAM) becomes the Troth Abstract Machine (TAM), and T0103 messages say "not a Troth keyword".
docs/adr/0016-final-name-troth.md:29:"Vera" remains only as history: in the prior-art section of `reviews/triage.md` and in this ADR. The rename is mechanical and changes no semantics. Git history is not rewritten, and the owner renames the GitHub repository.
docs/adr/0016-final-name-troth.md:33:- The CLI, the file extension and the Agent ABI schemas no longer collide with aallan/vera, before any public release and before M9 freezes the ABI.
docs/adr/0016-final-name-troth.md:39:- **Keeping Vera:** the collision would grow with every release and every agent trained on either project.
docs/adr/0014-reserved-keywords.md:9:`spec/01 §5` reserves keywords that v0.1 does not implement and requires T0103 "with a message naming the planned version". No document maps these keywords to versions: `plan/backlog.md` lists features, not keywords, and several reserved words (`match`, `return`, `var`, `while`, `loop`, `type`, `unsafe`, `macro`, `async`, `await`) name constructs that Troth does not plan to add, or already provides under another name. Milestone M1 implements T0103 and needs a message for every reserved keyword.
docs/adr/0006-single-error-kind.md:8:Failure sets as unions of variants from several kinds require union types and complicate `attempt` and exhaustiveness.
docs/spec/13-diagnostics-and-abi.md:193:  "coverage": { "pub_functions": 3, "with_ensures": 1, "with_derived_from": 1, "trivial_ensures": 0 },
docs/plan/milestones-v0.1.md:142:3. `troth trace`, `troth report` with every section, including coverage and trivial `ensures` detection.
docs/reviews/triage.md:53:| Separate "intent coverage" metric                                                                   | Rejected as separate concept           | Merged into report `coverage`                                   |
docs/reviews/triage.md:74:### aallan/vera ("Vera: a programming language designed for LLMs to write")
docs/reviews/triage.md:76:Public project (github.com/aallan/vera, veralang.dev, MIT), unrelated to this repository despite the shared name and the same `veritas` etymology. Status at review: v0.1.13, reference compiler in Python, programs compiled to WebAssembly.
docs/reviews/triage.md:78:| Topic                                        | aallan/vera                                                   | This project                                                             | Decision                                                                     |
docs/reviews/triage.md:88:| Benchmark                                    | VeraBench: 60 problems, 5 difficulty tiers, several models    | Gate 1 suites A to D (roadmap §3)                                        | Study VeraBench when designing suite A; B to D have no counterpart there     |
docs/reviews/triage.md:96:The shared name conflicts with an established public project (same CLI name `vera`, same `.vera` extension). "Vera" remains a codename until a final name is chosen; the rename should happen before any public release and preferably before M9 freezes the Agent ABI schema names.
docs/reviews/triage.md:98:This project was codenamed Vera until 2026-09-29; its final name is Troth.
docs/compiler/testing.md:105:## 5. Coverage expectations for v0.1
```

## 4. Milestones

| Milestone | Status | Commits | Tests | Conformance (passed / pending) |
| --- | --- | --- | --- | --- |
| M1 Lexer and parser | done | `2bafddd`, `8a06017`, `9d2b95f`, `a7313bf`, `d95fe43`, `bde3b98`, `b1039e3`, `7900d72`, `045bafa` | 154 | 50 cases: 47 passed, 3 pending (M5, M6, M8), 0 failures |
| M2 Formatter | done | `8a59109`, `95c1ef3`, `697cd2e`, `acf4ec2`, `77e4d5d` | 178 | 63 cases: 60 passed, 3 pending (M5, M6, M8), 0 failures |
| M3 Project and names | done | `0e8030c`, `457ca92`, `9915290`, `b723bd2`, `e3ab28f`, `f98912a` | 204 | 92 cases: 89 passed, 3 pending (M5, M6, M8), 0 failures |
| M4 Types | done | `fc9301b`, `0c07912`, `cafb07b`, `a5c8d3d` | 236 | 116 cases: 113 passed, 3 pending (M5, M6, M8), 0 failures |
| M5 Capabilities, effects and failures | not started | | | |
| M6 Information flow | not started | | | |
| M7 Core IR and interpreter | not started | | | |
| M8 Verification | not started | | | |
| M9 Agent ABI and build report | not started | | | |
| M10 Gate 1 readiness | not started | | | |

M1 evidence, per acceptance criterion (`docs/plan/runs/M1-plan.md §5, §7`):
- Conformance cases for U0001 to U0007 and U0101 to U0103: 47 syntax cases, all passing; U0102 also covers handling clauses (D-020).
- Every example parses: `crates/una-syntax/tests/examples.rs` and six reviewed insta snapshots of the syntax trees; `una check examples/events` and `examples/hello` exit 0 without output.
- Robustness (ADR 0017 decision 7): 168 prefixes of the six example files, each parsed without panic, every position inside the prefix, and every required prefix rejected.
- Found and fixed during M1: deep nesting crashed `una check` with a stack overflow; it is now one U0101 (ADR 0019, Provisional).
- The self-review checklist of every MUST and MUST NOT of `spec/01` and `spec/02` is in `docs/plan/runs/M1-plan.md §7`.

M2 evidence (`docs/plan/runs/M2-plan.md §5, §7`):
- Idempotence and round trip: `crates/una-fmt/tests/properties.rs` formats every `.una` file of the repository that parses, formats the result again, and compares the trees after the formatter's documented rewrites (D-030) and the comments.
- All examples are canonical: the same test, and `una fmt --check` on both example packages; the previous reference example needed its imports sorted.
- U7001 with mechanical edits in `una check`, `una fmt` and `una fmt --check`: 13 conformance cases in `tests/conformance/fmt` and CLI tests.
- The canonical form beyond the specification is ADR 0020 (Provisional).

M3 evidence (`docs/plan/runs/M3-plan.md §5, §7`):
- Conformance cases for every listed code: 29 cases in `tests/conformance/names` and `tests/conformance/project`, including packages, a missing lock and two tours that resolve cleanly.
- All examples resolve: `una check examples/events` and `examples/hello` exit 0 without output; both gained their `verification.lock`.
- Three Provisional ADRs: 0021 (packages and project files; ADR 0015 D3 expired as foreseen), 0022 (diagnostics without a source position, requested at M0), 0023 (name resolution).
- The previous reference example forced two rules: a module's own `NotFound` shadows the prelude's (D-039), and `main.una` matches the variants of a kind it never imports, so variants in patterns and after `fail` wait for their kind in M4 (D-046).

M4 evidence (`docs/plan/runs/M4-plan.md §8`):
- Conformance cases for U2001 to U2015: 24 cases in `tests/conformance/types`, at least one failing case per code, a single-module tour and a package tour; the fifteen explain texts carry failing and fixed examples, checked by the explain test.
- `una check --tier 0 examples/events` reports nothing: `seating.una` and the whole reference example type-check, as does `examples/hello`.
- 26 unit tests in `crates/una-check/tests/types.rs` cover each rule, the pending obligations of `spec/12 §3` and selfified `let`s; the standard modules type-check as ordinary modules, and their names equal `spec/14`.
- The self-review checklist of every MUST and MUST NOT of `spec/03`, `spec/04 §1 to §3, §6, §7`, `spec/06 §1, §2, §5` and `spec/10 §3, §4` is in the M4 plan §8. Effects, failures, one effect per line and capability sufficiency are M5; labels M6; obligations are recorded, not proven, until M8.

## 5. Decisions needing review

The top ten entries of `docs/reviews/open-decisions.md`, each answerable in one line.

| Decision | Choice made | Answer |
| --- | --- | --- |
| D-023 | The canonical form of ADR 0020 (Provisional): one line per logical line, fixed spacing, one blank line between items, comments never moved, LF only | Keep, switch to B (author's layout), or change one rule |
| D-037 | Scopes: locals, the module, explicit imports, the variants of imported kinds, the prelude; the innermost wins (ADR 0023) | Keep, or switch to B (only listed imports) or C |
| D-046 | Variants in patterns, `is`, after `fail` and `expect fails` take their kind from the context, in M4 | Keep, or switch to B (scopes only) |
| D-048 | Where each contextual variant is looked up: the tested value's kind, the declared error kind, the error kinds of the `do` line (ADR 0024 rule 1) | Keep, or switch to B (scopes only) |
| D-063 | A generic function that compares values of a type parameter cannot be instantiated with a type that holds functions (ADR 0024 rule 11) | Keep, or switch to B (no function type arguments) or C (run time) |
| D-018 | After a label level, `[` starts the categories; a labeled list is written `secret List[T]` (ADR 0018) | Keep, or switch to B (backtrack) or C |
| D-022 | Nesting deeper than 256 levels, counting chained operators, is U0101; `una check` runs on a 64 MiB stack (ADR 0019) | Keep, change the limit, or count only true recursion (D) |
| D-019 | A lexical error suppresses the parser's U0101 later in the same declaration | Keep, or switch to A or C |
| D-021 | Doc comments attach across blank lines and plain comments | Keep, or switch to B (only directly above) |
| D-024 | Predicates, handler operations and arms also put a one-line body on their line | Keep, or switch to B (functions only) |

## 6. Blockers and deviations from the spec

No blockers.

Deviations and additions, all logged:
- Six Provisional ADRs fill spec gaps: 0018 (a `[` after a label level), 0019 (nesting limit), 0020 (the canonical form), 0021 (packages and project files), 0022 (diagnostics without a source position) and 0023 (name resolution). The spec is unchanged until they are accepted; each ADR lists what to add.
- ADR 0015 D3 (interim loading) expired in M3 as it foresaw; three CLI tests that built `src/` without `una.toml` now write one (D-032).
- U0102 for handling clauses (`spec/06 §3`) is checked in M1 rather than M6 (D-020).
- The approved explain texts of U0101 and U0102 gained one sentence each, for ADR 0019 and D-020.
- The previous reference example sorted its imports (D-025), which changed two reviewed syntax-tree snapshots in their import lines only.
- ADR 0024 (Provisional) fills the typing rules the specification leaves open, rules 1 to 15, with decisions D-048 to D-055 and D-061 to D-069. The strict readings to review first: recursion only through `List` or `Option` (D-062), equality on type parameters (D-063), punning of the first argument (D-064), recursion of every predicate (D-065).
- Two tour cases of M1 and M3 passed second arguments without labels, which `spec/04 §3` rule 2 forbids; their inputs gained the labels (D-068).

## 7. Hindsight and next steps

- Hindsight: the M1 plan did not consider input depth. Unoptimized builds use about 34 KiB of stack per nesting level, so every later pass should keep its recursion shallow and rely on the depth bound of ADR 0019.
- Hindsight: the first formatter design moved comments out of joined or merged lines, which could turn a plain `---` comment into attached documentation. The self-review caught it; comments now never move.
- Hindsight: running the reference examples early caught two name rules that the specification never states (D-039, D-046). Checking the examples first, before writing the rules, would have saved a revision of ADR 0023.
- The conformance runner now fails any case, pending or not, that reports an implemented code it does not expect (D-044).
- Hindsight: M4 restarted after the consolidated instructions; writing exploratory probes before the unit tests caught a dozen message and cascade problems cheaply, but the plan's small commits became three larger ones.
- Next: M5 (capabilities, effects and failures), then M7, the rest of the critical path to checkpoint 3 (`docs/plan/roadmap.md §4`), which matters more than starting M8. M5 extends the type checker: capability contexts, inferred effects and failures, one effect per line.

## 8. Checkpoint demo

The v0.1 usage checkpoints are in `docs/plan/roadmap.md §4`. Each checkpoint whose milestones are complete has a script that prints every command, its expected outcome and its actual outcome, and exits 1 if any differs; `crates/una-cli/tests/demo_scripts.rs` runs every script in CI.

| Checkpoint | Reached | Script | Programs |
| --- | --- | --- | --- |
| 1. Read (M1, M2) | yes | `scripts/demo/checkpoint-1.sh` | `tests/demo/checkpoint-1/`: a clean module, U0101, U0103, U0007 and U7001, plus `--json`, `una fmt` and `una explain U0101` |
| 2. Understand (M3 to M6) | no | | |
| 3. Run (M7) | no | | |
| 4. Trust (M8 to M10) | no | | |

To run checkpoint 1: `cargo build -p una-cli && scripts/demo/checkpoint-1.sh`.
