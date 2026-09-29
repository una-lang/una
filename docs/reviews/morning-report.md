# Morning report

Updated after every milestone of the overnight run (`OVERNIGHT.md`, C.8).

## 1. Status

- Language name: Una (ADR 0025; previously Troth, codenamed Vera).
- Last completed milestone: M10 (Gate 1 readiness), the last of v0.1: five more example packages cover every construct of the charter's scope, `docs/guide/una-for-agents.md` is a reference for agents with every block tested, release archives build in CI, and usage checkpoint 4 has its demo. Next: the owner's review of the Provisional ADRs and the decision log, the merge of `dev/v0.1` into `main`, and the tag `v0.1.0`, which starts the release workflow (D-130).
- Repository: una-lang/una (private until launch), branch `dev/v0.1`; its history starts at the initial public import `25d865b` (ADR 0030). Earlier commit hashes in this report refer to the legacy repository.
- Last green CI runs: https://github.com/una-lang/una/actions/runs/36633617255 (`dev/v0.1`, Linux, at `08abf15`, M10 self-review), https://github.com/una-lang/una/actions/runs/36632878216 (`dev/v0.1`, Linux, at `58f7a07`, M10, with the Linux release archive), https://github.com/una-lang/una/actions/runs/36628713460 (`dev/v0.1`, Linux, at `4dca6d1`, M9), https://github.com/una-lang/una/actions/runs/36625239354 (`dev/v0.1`, Linux, at `74afc16`, M8), https://github.com/una-lang/una/actions/runs/36618844273 (`dev/v0.1`, Linux, at `511b4eb`, M7), https://github.com/una-lang/una/actions/runs/36610873278 (`dev/v0.1`, Linux, at `b37eaa2`, M6), https://github.com/una-lang/una/actions/runs/36606305565 (`dev/v0.1`, Linux, at `79b7a2c`, M5) and https://github.com/una-lang/una/actions/runs/36580294816 (`main`, Linux and macOS, at `25d865b`). macOS runs only on pull requests and pushes to `main` (D-083); local checks on the owner's Mac pass. Before the import: run 36578625540 of the legacy repository (`a5c8d3d`, M4).

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
| M5 Capabilities, effects and failures | done | `9d34aab`, `3be9490`, `08260d3`, `58dd14d`, `0ded2b9`, `4f959a7`, `ce6f46b`, `79b7a2c` (plan and ADRs 0028, 0029 in the initial import `25d865b`) | 259 | 144 cases: 142 passed, 2 pending (M6, M8), 0 failures |
| M6 Information flow | done | `2348672`, `d519fbb`, `78ad11e`, `e763556`, `0241e93`, `906bd9e` (an M5 fix), `a40584d`, and the self-review | 274 | 160 cases: 159 passed, 1 pending (M8), 0 failures |
| M7 Core IR and interpreter | done | `6c648fb` (plan, ADR 0032), `59a481d`, `d81f200`, `c3d2041`, `54d30c8`, `685bc36`, `bdc7889`, `511b4eb` (self-review) | 296 | 173 cases: 172 passed, 1 pending (M8), 0 failures |
| M8 Verification | done | `3146132` (plan, ADR 0033), `d8d3db6`, `02bb2d5`, `49c73e9`, `ec853fa`, `048cbfa`, `3f10ac5`, `4b212fb`, `a715ff0`, `74afc16` (self-review) | 305 | 190 cases: 190 passed, 0 pending, 0 failures |
| M9 Agent ABI and build report | done | `fa8c4de` (plan, ADR 0034), `61f8f05`, `d57857b`, `92f6b9b`, `fd899f2`, `4dca6d1` (self-review) | 314 | 190 cases: 190 passed, 0 pending, 0 failures |
| M10 Gate 1 readiness | done | `0d65eff` (plan, ADR 0035), `3b83602`, `de9ab73`, `05c174f`, `cc9cca2`, `e13533e`, `5f5955c`, `bf640f2`, `877b6bd`, `b888b44`, `9b23b9f`, `58f7a07`, `08abf15` (self-review) | 319 | 193 cases: 193 passed, 0 pending, 0 failures |

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

M5 evidence (`docs/plan/runs/M5-plan.md §8`):
- Conformance cases for U3001 to U3015 and U7003: 29 cases in `tests/conformance/authority`, at least one failing case per code, U3012 and U7003 as package cases, and passing cases for uses and handlers, inferred headers, examples with `given`, grants and the escape budget; the sixteen explain texts carry failing and fixed examples, checked by the explain test.
- `una contract` (ADR 0029): human output and the JSON schema `una.contract/1`, with reviewed insta snapshots of `events.checkin.check_in` in both forms.
- The whole reference example stays free of diagnostics; the determinism test covers the authority cases.
- 20 unit tests in `crates/una-check/tests/authority.rs`, one per rule; the M4 unit tests leave out the new codes (D-084).
- Inference (ADR 0028 rule 2, D-072) decides a private function's capability references by the declared headers it calls, then its callers' bindings, then the handlers they install, so each disagreement is reported where it is written; `una contract` prints the result with `-- inferred`.
- Diagnostics with one mechanical repair carry its edit (D-086): widening a `uses` clause, adding `uses` or `fails` clauses in canonical order; each applied edit removes its diagnostic and keeps the file canonical.
- The self-review checklist of every MUST and MUST NOT of `spec/05`, `spec/06 §2 to §6`, `spec/04 §2, §4 to §8` and `spec/08 §2` is in the M5 plan §8. Labels, the program counter and sinks are M6; `--grant` and running examples are M7.

M6 evidence (`docs/plan/runs/M6-plan.md §8`):
- Conformance cases for U4001 to U4005 and U4007: 17 cases in `tests/conformance/security`, including the classic implicit flow, the failure under a secret condition that was pending since M1, and labeled text printed to `Console`; passing cases for labels through generics, fields, patterns, lambdas and sinks, and a declassification package in the shape of the reference example. Six explain texts with checked examples.
- Labels live at every position of the checker's types, as Core IR's binder types will (`spec/09 §2`), and unification joins them on type variables, so `first`, `map` and `Option` keep secrets secret without annotations (D-088).
- The reference example passes tier 0, its `via` function unchecked as `spec/07 §5` requires, and its policy call at a `public` program counter.
- Usage checkpoint 2 is reached: `scripts/demo/checkpoint-2.sh` shows a name, a type, two authority, an information-flow and a project error, a mechanical edit in JSON, and `una contract` with inferred clauses.
- 12 unit tests in `crates/una-check/tests/flow.rs`, one per rule; the M4 unit tests leave out the security codes (D-084).
- The self-review checklist is in the M6 plan §8. It found no rule left out; the milestone found an M5 bug (inferred `fails` through an operation on an inferred reference) and a soundness gap in lambdas, both fixed.

M7 evidence (`docs/plan/runs/M7-plan.md §7`):
- `una test examples/events` passes its six examples, and `una run examples/hello --grant console` prints the greeting; `una run examples/events` prints the availability line.
- The interpreter implements the rules of `spec/11 §4` directly over Core IR, with arbitrary-precision integers, Euclidean division, instances with stable ids, handler events before the events of their bodies, and the prelude and host capabilities of `spec/14` (`Files` confined to its prefix through `..` and links, `Keys` with HMAC-SHA-256 checked against RFC 4231).
- Traces never show labeled data: arguments above `public`, labeled fields of public records and labeled results are `<redacted:level>` (D-107); a CLI test searches the JSON of `una run` and `una test` for the secret values.
- Snapshots: the Core IR of the five modules of the reference example, `una.run/1` of `examples/hello` and `una.test/1` of `examples/events`.
- Conformance cases for `run-output`, `test-pass` and `test-fail`: 13 cases in `tests/conformance/runtime`, with U6001, U6003 and U3012 at startup; the runner evaluates them with `una run --json` and `una test --json` (D-110). Explain texts for U6001 and U6003, whose examples run with `una run`.
- Usage checkpoint 3 is reached: `scripts/demo/checkpoint-3.sh` runs a program, shows U3012 before a run and the grant that fixes it, U6001 with public fields only, U6003, the examples with their redacted traces, a failed expectation and the Core IR.
- The self-review checklist is in the M7 plan §7. The milestone found that argument redaction alone would print a `secret` field of a public record, and that a fixed stack estimate could overflow; both are fixed (D-107, D-105).

M8 evidence (`docs/plan/runs/M8-plan.md §7`):
- Core IR carries the refinement language: refinements, `requires` and `ensures` as terms, the contracts of the standard library, and the spans every obligation points at (D-111); the verifier generates obligations from it alone, and its sites equal the ones the type checker has recorded since M4, over 39 packages and cases.
- The reference example verifies: 11 obligations of all five kinds' sites, all proven; its queries are byte-identical across runs (a snapshot), and a second run answers every one from the proof cache.
- A refuted postcondition points at the failing return with a counterexample (`remaining = 0, result = false` in the case pending since M1), related to its `ensures` clause, with ranked repairs; a step budget of 1 gives U5002; a lock pinning another version, and a path without `z3`, give U5003 at the lock's `solver_version`.
- 17 conformance cases in `tests/conformance/verification`, and the pending case of M1 passes: no case is pending any more.
- `una run` and `una test` check at tier 1 without the note of ADR 0027.
- The self-review checklist is in the M8 plan §7. The milestone found that Z3 drops a nonlinear assertion with an error, which could have passed for a status, and that `expect` lines were not verified; both are handled (D-115, D-112).

M9 evidence (`docs/plan/runs/M9-plan.md §7`):
- `una report` checks a package at tier 1 and prints `una.report/1` in the key order of `spec/13 §6`: the reference example has 11 proven obligations, the policy call at `src/events/checkin.una` 30:13 with its purpose and owner, one `checked` against a budget of 1, `events.keys` with 17 lines, the host capabilities `Console` and `Keys`, `EVENTS_3` derived by `availability`, and 5 public functions of which 1 has `ensures` (D-122).
- Trivial `ensures` clauses are found by one more query each, with no hypothesis: `ensures result == result` and `ensures result >= 0 or result < 0` count as missing contracts (D-123).
- `una trace EVENTS_3` prints the requirement, `availability` with its `ensures` clause and its two proven obligations, in human form and as `una.trace/1`; an unknown identifier is a usage error (D-124).
- Every command prints one JSON document with its schema first and nothing on stderr, `una explain` included (`una.explain/1`, D-125); the documents are byte-identical across runs and with one thread. U8001 has its full explain text, without examples (D-126); every other code keeps its checked failing and fixed examples.
- The obligation listing reads as Una: hypotheses show temporaries through their definitions and call texts, with the parentheses of `spec/01 §7` (`not e.remaining in 0..0`); a `let` bound to a branching expression is selfified as `spec/03 §7` asks, which simplified the queries without changing a status.
- Snapshots: the report, the obligation listing and the trace of the reference example, reviewed.
- The self-review checklist is in the M9 plan §7. The milestone found that the report ignored a solver failure on the trivial queries; it now reports U5003 or U8001.

M10 evidence (`docs/plan/runs/M10-plan.md §7`):
- Five example packages join `events` and `hello`, each in a neutral setting an evaluation task can start from: `redacted` (a tool-redacted shelf: records, sums with payloads, reflect predicates in contracts, `attempt` under a `with` rebinding, `Clock`), `pantry` (a shared kitchen: generic kinds and functions, `NonEmpty[T]` proving a division, lambdas through method calls), `journal` (a private journal: a label category, the program counter, a sink, a policy in a trusted module, `Files` and `Env`), `greenhouse` (a vent controller: operation contracts, handlers verified against them and with their own `uses`, an inferred header, a regulation) and `shortlinks` (a link shortener: an opaque predicate and `checked`, `Result`, `if error use`, a generated requirement, proposed).
- Every package is canonical, checks at tier 1 with every obligation proven (106 across the seven), passes every declared example, reports no trivial `ensures`, and prints its expected output with `una run`. A test finds each of 71 constructs of the charter's language list in the examples, and checks that each pattern tells programs apart (D-127).
- `docs/guide/una-for-agents.md` covers the loop of check and repair, packages, syntax, kinds, functions, contracts, failures, capabilities, labels, requirements and examples, and the ABI. Its test checks 15 modules (7 of them failing with the code their first line names), 2 packages, 3 JSON documents and 11 commands (D-128).
- A test plays an agent that reads only JSON and exit codes: it applies the mechanical edit of U3003, then checks, tests, lists and reports the package.
- `scripts/release.sh` and `scripts/check-release.sh` pack and check `una-0.1.0-<target>.tar.gz`; CI packs the Linux archive on every push and runs the packed binary on the examples, and `.github/workflows/release.yml` builds Linux and macOS archives into a draft release when the owner pushes a tag (D-129, D-130). The macOS arm64 archive also built and ran on the owner's Mac.
- Usage checkpoint 4 is reached: `scripts/demo/checkpoint-4.sh`.
- The self-review checklist is in the M10 plan §7. The examples found two verifier issues, both fixed with conformance cases: Z3 rejects a few quoted symbols, among them Core's `_`, which turned valid programs into U8001 (`3b83602`, D-115); and a successful `attempt` lost the callee's `ensures`, a gap of `spec/12 §3` (`de9ab73`, D-131).

## 5. Decisions needing review

The top ten entries of `docs/reviews/open-decisions.md`, each answerable in one line.

| Decision | Choice made | Answer |
| --- | --- | --- |
| D-023 | The canonical form of ADR 0020 (Provisional): one line per logical line, fixed spacing, one blank line between items, comments never moved, LF only | Keep, switch to B (author's layout), or change one rule |
| D-037 | Scopes: locals, the module, explicit imports, the variants of imported kinds, the prelude; the innermost wins (ADR 0023) | Keep, or switch to B (only listed imports) or C |
| D-046 | Variants in patterns, `is`, after `fail` and `expect fails` take their kind from the context, in M4 | Keep, or switch to B (scopes only) |
| D-048 | Where each contextual variant is looked up: the tested value's kind, the declared error kind, the error kinds of the `do` line (ADR 0024 rule 1) | Keep, or switch to B (scopes only) |
| D-063 | A generic function that compares values of a type parameter cannot be instantiated with a type that holds functions (ADR 0024 rule 11) | Keep, or switch to B (no function type arguments) or C (run time) |
| D-071 | U3004 for `@` in pure code, U3001 for a missing binding or another capability, U3002 for fewer operations, U3003 for effects beyond an empty `uses` (ADR 0028 rule 1) | Keep, or switch to B (U3001 for every missing capability) |
| D-072 | Private headers are inferred; a reference's capability comes from the headers it calls, then its callers' bindings, then installed handlers (ADR 0028 rule 2) | Keep, or switch to B (from the operation's name) or C (callers only) |
| D-088 | Labels at every position of the checker's types, joined on type variables (ADR 0031 rule 1) | Keep, or switch to B (one label per value) or C (a separate pass) |
| D-098 | Pass 2 of the checker records its resolutions by span, and the elaborator builds Core IR from them (ADR 0032 rule 1) | Keep, or switch to B (an elaborator that resolves again) |
| D-111 | Core IR carries refinement-language terms for refinements and contracts, spans for obligation sites, and `unreachable` after a `when` without `else` (ADR 0033 rule 1) | Keep, or switch to B (formulas as A-normal expressions) or C (obligations from the syntax tree) |

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
- ADR 0028 (Provisional) fills the authority rules the specification leaves open, rules 1 to 12, and ADR 0029 (Provisional) defines the output of `una contract`. Additions to review first: `una check` reports U3012 from `[run] grants` and for capabilities of `main` that are not host capabilities (D-076); function values and parameters of effectful function types are second-class like lambdas (D-079); S-Fun violations are U3003 and U3009 (D-085); a capability reference bound twice is U1002 (D-087).
- The M4 unit tests compare only the M4 codes from M5 on (D-084): one M4 input uses `checked` twice in a single file, which M5 reports as U7003; no M4 expectation changed in M5. In M6 one line left the M4 policy test: a `via` function of another type is U4005, as `spec/07 §5` rule 4 requires, not the interim U2001 of D-066; `flow.rs` tests it.
- ADR 0035 (Provisional) fixes what M10 leaves open: the five example packages and the test of their coverage (D-127), how the guide's blocks are tested (D-128), and the release archives (D-129). The run does not push the tag `v0.1.0`: the owner tags `main` after review (D-130). ADR 0033 gained rule 12 in M10: a successful `attempt` keeps the callee's `ensures`, which `spec/12 §3` does not list (D-131); and binders named `_`, `as`, `xor` or `distinct` gain a suffix in queries (D-115). The decision log also asks whether two obligations with the same goal in one declaration should keep sharing an id, as `spec/12 §8` gives (D-118).
- ADR 0034 (Provisional) fills the build report and ABI details the specification leaves open, rules 1 to 5. Readings to review first: the report lists every function with `uses`, private ones included, and only the host capabilities the package uses (D-122); a trivial `ensures` is one the solver proves with no hypothesis (D-123); `una explain` gained a JSON document, replacing M1's usage error (D-125); U8001's text has no examples (D-126).
- ADR 0033 (Provisional) fills the verification details the specification leaves open, rules 1 to 11. One deviates from the text of `spec/12 §2`: a division by a variable is an uninterpreted function, since QF_UFLIA has no nonlinear terms and Z3 rejects them (D-115). Readings to review first: one obligation per `ensures` clause over every return path (D-113), record field refinements as ground facts (D-114), and tier 1 only after a clean tier 0 (D-112).
- ADR 0032 (Provisional) fills the runtime details the specification leaves open, rules 1 to 11: elaboration, the Core dump, values in traces, the output of `una run` and `una test`, grants, the prelude, the call depth, examples, the entry and faults. Readings to review first: a `do` line that fails with no `expect fails` line fails its example (D-106); labeled fields and results are redacted beyond the arguments of `spec/11 §5` (D-107); a division by zero before tier 1 is U8001 (D-108); `una run` checks `main` against `[run] grants` and `--grant` together, and runs the module of a single file (D-109).
- ADR 0031 (Provisional) fills the information-flow rules the specification leaves open, rules 1 to 10. Readings to review first: `checked` on a labeled value is U4002, stricter than T-Checked's premise `pc = ⊥` alone (D-091); an operation's arguments are bounded by its `accepts` only, not by the labels of its parameter types, which the prelude's `Files.write` needs (D-092); a `via` function may be used only through its policy (D-094); a lambda records the labels its body assumed for its parameters, so a `fold` over labeled values needs a labeled initial value (D-089).

## 7. Hindsight and next steps

- Hindsight: the M1 plan did not consider input depth. Unoptimized builds use about 34 KiB of stack per nesting level, so every later pass should keep its recursion shallow and rely on the depth bound of ADR 0019.
- Hindsight: the first formatter design moved comments out of joined or merged lines, which could turn a plain `---` comment into attached documentation. The self-review caught it; comments now never move.
- Hindsight: running the reference examples early caught two name rules that the specification never states (D-039, D-046). Checking the examples first, before writing the rules, would have saved a revision of ADR 0023.
- The conformance runner now fails any case, pending or not, that reports an implemented code it does not expect (D-044).
- Hindsight: M4 restarted after the consolidated instructions; writing exploratory probes before the unit tests caught a dozen message and cascade problems cheaply, but the plan's small commits became three larger ones.
- Hindsight (M5): the run moved to a new session in the middle of the milestone, with the implementation uncommitted; a saved note of the next steps made the restart cheap, but committing a compiling checkpoint before the move would have been safer.
- Hindsight (M5): the first inference took a reference's capability from its callers only, so a wrong handler or binding showed up as a conflict far from where it was written, and a helper without callers could not be inferred. Probing with small programs before writing the unit tests exposed both, and ranking the sources fixed them (D-072). Writing the unit tests then exposed four more holes: `std.testing` in `expect` lines, parameters that let effectful lambdas escape, capabilities of `main` that no grant can provide, and references bound twice.
- Hindsight (M6): choosing labels inside the checker's types rather than one label per value cost a careful change to the unifier, but every M4 and M5 test passed unchanged once `resolve` erased labels, and generic code needs no annotations. The places that build a result type from a resolved one lost labels silently; a list of them, made before writing the code, found them all.
- Hindsight (M6): the demo programs caught an M5 inference bug that no unit test covered, since those programs mix features the way users will.
- Hindsight (M7): recording the checker's resolutions by span made elaboration a translation rather than a second checker, and the interpreter could follow `spec/11 §4` rule by rule. Checking the conformance programs against outputs worked out by hand found no interpreter bug; the failures were in the test programs, which the checker explained precisely.
- Hindsight (M7): redaction by argument labels looked complete until a record test carried a `secret` field; writing the redaction test with nested data first would have shown it at once. Likewise a fixed stack estimate per call held for simple recursion only, so the evaluator now measures the stack it uses.
- Hindsight (M8): the specification generates obligations from Core IR, and M7's Core had neither formulas nor enough spans; carrying terms and spans into Core cost a commit of its own, and made the verifier independent of the checker. Comparing the verifier's sites with the checker's, recorded since M4, then gave a cheap test of both.
- Hindsight (M8): probing the pinned Z3 before writing the driver showed that it drops a nonlinear assertion after an error; without the probe, a wrong encoding could have passed for a proof.
- Hindsight (M9): the snapshot review of the obligation listing, not a test, showed hypotheses full of temporaries and a missing selfification; reading the documents an agent will read, before accepting them, is the review that catches what assertions do not state.
- Hindsight (M10): writing five programs the way users will found two verifier bugs that 190 conformance cases had missed, both at the meeting point of features (a discarded statement before a return, `attempt` with a contract). Realistic programs belong in every milestone, not only the last.
- Next: the owner's review. The ten decisions above and the Provisional ADRs 0018 to 0024, 0028, 0029 and 0031 to 0035 are the questions; once they are answered, merge `dev/v0.1` into `main` and push the tag `v0.1.0`, and the release workflow attaches the Linux and macOS archives to a draft release. Then Gate 1 (`plan/roadmap.md §3`), with the examples and the guide as the starting material of its tasks.

## 8. Checkpoint demo

The v0.1 usage checkpoints are in `docs/plan/roadmap.md §4`. Each checkpoint whose milestones are complete has a script that prints every command, its expected outcome and its actual outcome, and exits 1 if any differs; `crates/una-cli/tests/demo_scripts.rs` runs every script in CI.

| Checkpoint | Reached | Script | Programs |
| --- | --- | --- | --- |
| 1. Read (M1, M2) | yes | `scripts/demo/checkpoint-1.sh` | `tests/demo/checkpoint-1/`: a clean module, U0101, U0103, U0007 and U7001, plus `--json`, `una fmt` and `una explain U0101` |
| 2. Understand (M3 to M6) | yes | `scripts/demo/checkpoint-2.sh` | `tests/demo/checkpoint-2/`: a clean module, U1001, U2001, U3001 with its JSON edit, U3004, U4003 and U7002, plus `una contract` in human and JSON form and `una explain U4003` |
| 3. Run (M7) | yes | `scripts/demo/checkpoint-3.sh` | `tests/demo/checkpoint-3/`: a program with a kind, recursion, a lambda and interpolation, U3012 before a run and its fix, U6001 with public fields only, U6003, a failed expectation, plus `una run examples/hello`, `una test` on the reference example with redacted traces, `--dump-core` and `una explain U6001` |
| 4. Trust (M8 to M10) | yes | `scripts/demo/checkpoint-4.sh` | `tests/demo/checkpoint-4/`: a false `ensures` refuted with its counterexample and its fix proven, U5002 from a step budget, U5003 from a lock pinning another Z3, a trivial `ensures` in the report, plus `una obligations` in human and JSON form, `una report` and `una trace` on the reference example and `una explain U5001` |

To run checkpoint 1: `cargo build -p una-cli && scripts/demo/checkpoint-1.sh`; checkpoints 2 to 4 likewise with `scripts/demo/checkpoint-2.sh`, `scripts/demo/checkpoint-3.sh` and `scripts/demo/checkpoint-4.sh`, the last with Z3 at the pinned version on `PATH`.
