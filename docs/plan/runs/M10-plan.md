# M10 plan: Gate 1 readiness

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `00-charter.md §5` (the v0.1 scope, which the examples cover) and `§7` (success criteria), `plan/roadmap.md §3` (Gate 1: the task suites the examples and the guide serve) and `§4` (usage checkpoint 4); also `spec/01` to `spec/08` and `spec/14`, which the guide summarizes, `spec/13 §5` (commands, flags and exit codes an agent drives), `compiler/architecture.md §7` (dependencies), D-083 (CI runners) and `OVERNIGHT.md` C.5 (never push to `main`).

Codes: none new. Acceptance (`plan/milestones-v0.1.md`): the guide's snippets compile; release artifacts build in CI; the evaluation track can drive `una` headlessly through `--json`. Tasks: at least five additional example packages covering every construct in the charter scope; `docs/guide/una-for-agents.md` with every snippet tested; release binaries for Linux and macOS and the tag `v0.1.0`. Usage checkpoint 4 ("Trust") gets its demo.

## 1. Design

- **Example packages.** Five packages join `examples/events` and `examples/hello`, each a small program in a domain-neutral setting that an evaluation task could start from (roadmap §3), and each checking at tier 1 with every obligation proven and every example passing:
  - `examples/redacted`, a community tool-redacted shelf: records, a sum with payloads, range refinements, a reflect predicate, `requires` and `ensures`, a user capability with handlers, `if none fail`, `if error fail` and `pass`, `attempt` with `Result`, `with` rebinding and handler instances, `Clock`, requirements from a human and from a policy, and examples with `expect performed`.
  - `examples/pantry`, the stock of a shared kitchen: generic kinds and functions, a generic refinement, lambdas with `map`, `filter`, `fold` and `first_where`, method-call syntax, `List`, `Option` and `Nat`, and contracts over list lengths.
  - `examples/journal`, a private journal: label levels and a declared category, the program-counter label, sink labels on a capability, a declassification policy in a trusted module, and `Files`, `Env` and `Console` in `main`.
  - `examples/greenhouse`, a vent controller: capability operations with `requires` and `ensures`, handlers verified against them and handlers with their own `uses`, `with @x = @y` rebinding, an inferred private header, and a requirement from a regulation.
  - `examples/shortlinks`, a link shortener: an opaque predicate established by `checked` within the escape budget, `attempt`, `if error use`, `if none use`, `Result` in a signature, and a proposed requirement from a generated source.
- **Every construct.** A test checks that the examples, taken together, contain each construct of the language list of charter §5, one source pattern per construct, and that each package formats, checks at tier 1 with every obligation proven, passes `una test`, and produces a report with no trivial `ensures`; a package with `[run]` also runs.
- **The guide.** `docs/guide/una-for-agents.md` is a compact reference an agent can hold in context: the file layout, syntax, kinds, headers and contracts, one effect per line and the handling clauses, capabilities and handlers, labels and policies, requirements and examples, and the ABI (commands, `--json` documents, exit codes, repairs, the repair loop). Its snippets are tested (§2 rule 2).
- **Headless use.** A test drives a repair loop through `--json` alone, as the evaluation track will: check a broken package, apply the mechanical edit of its diagnostic, check again, then run `una test`, `una obligations` and `una report`, reading only JSON documents and exit codes.
- **Release.** `scripts/release.sh` builds `una` in release mode and packs it with the license and the readme into `una-<version>-<target>.tar.gz` and its SHA-256 file. `.github/workflows/release.yml` runs it for Linux (x86_64) and macOS (arm64) on a pushed tag `v*` and attaches the archives to a draft release; the CI workflow builds and smoke-tests the Linux archive on every push, which is the evidence that artifacts build in CI. The run does not push the tag (§2 rule 4).
- **Checkpoint 4.** `scripts/demo/checkpoint-4.sh` with programs in `tests/demo/checkpoint-4/`: a false `ensures` refuted with its counterexample and repairs, the fix proven, an obligation the step budget leaves unknown, `una obligations`, `una report` and `una trace` on the reference example, and `una explain U5001`.

## 2. Spec gaps and ADR 0035

ADR 0035 (Provisional), "examples, guide and release for Gate 1", records:
1. **The example packages and their coverage** (D-127): the five themes, what each package must pass, and the construct list with its source patterns.
2. **The guide's snippets** (D-128): every `una` fence is a whole module that checks clean at tier 1, unless its first line is `-- fails with U####`, when it yields exactly that code; `json` fences parse and name a schema the CLI prints; each `una` command of a `sh` fence runs from the repository root and exits with 0, or with the code its `# exit N` comment names.
3. **Release artifacts** (D-129): targets, archive contents and names, checksums, and which workflow builds what, within the runner rule of D-083.
4. **The tag** (D-130): the run prepares the release but does not push `v0.1.0`; the owner tags `main` after review and merge, and the release workflow does the rest.

## 3. Tasks and commits

1. **Plan and ADR 0035** (`docs(plan)`).
2. **The example test and `examples/redacted`** (`test(examples)`, `feat(examples)`): the per-package test first, over every directory of `examples/`, then the package.
3. **`examples/pantry`**, **`examples/journal`**, **`examples/greenhouse`** (`feat(examples)`), one commit each.
4. **`examples/shortlinks` and the coverage test** (`feat(examples)`), once every construct has an example.
5. **The guide** (`docs(guide)`), with its snippet test.
6. **Headless use** (`test(cli)`).
7. **Release** (`ci(release)`): the script, the release workflow and the CI job.
8. **Checkpoint 4** (`feat(demo)`).
9. **Self-review** (`docs(plan)`), the morning report and the Progress section.

Each commit is pushed once it passes.

## 4. Conformance cases

None: M10 adds no rule. The examples, the guide and the demo are checked by CLI tests, and the conformance runner keeps its 190 cases.

## 5. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| The guide's snippets compile | `crates/una-cli/tests/guide.rs` over every fence of `docs/guide/una-for-agents.md` |
| Release artifacts build in CI | the Linux archive job of `.github/workflows/ci.yml` on `dev/v0.1`; the macOS archive built locally with the same script (D-083) |
| The evaluation track can drive `una` headlessly through `--json` | the headless test; the M9 test that every command prints one JSON document |
| At least five additional example packages covering every construct in the charter scope | `crates/una-cli/tests/examples.rs`: each package passes, and the construct list is covered |
| Release binaries for Linux and macOS; tag `v0.1.0` | the release workflow and script; the tag is the owner's (D-130) |

## 6. Known gaps

- The tag `v0.1.0` and the release it triggers wait for the owner (D-130).
- Release archives cover Linux on x86_64 and macOS on arm64; other targets build from source.
- Coverage is checked by source patterns, which show that a construct is written, not that every combination of constructs is exercised.

## 7. Self-review checklist

The milestone's sources, charter §5 and §7 and roadmap §3 and §4, state no MUST, MUST NOT or MAY; the table lists what they and the milestone ask. Evidence names tests of `crates/una-cli/tests/examples.rs` (E), `guide.rs` (G), `headless.rs` (H), `abi.rs` (A), `cli.rs` (L) and `demo_scripts.rs` (D).

### Tasks and rules

| Rule | Status | Evidence |
| --- | --- | --- |
| M10 task 1: at least five additional example packages | done: `redacted`, `pantry`, `journal`, `greenhouse`, `shortlinks` (D-127) | E `every_example_package_is_canonical_verified_and_tested`: each canonical, tier 1 with every obligation proven, every declared example passing, no trivial `ensures`, and the output of `una run` |
| Charter §5: the examples cover every construct of the language list | done: 71 source patterns, each false on `hello` except the six it contains | E `the_examples_contain_every_construct_of_the_charter_scope` |
| M10 task 2: the guide, every snippet tested | done (D-128) | G `every_block_of_the_guide_is_checked`: 15 modules (8 clean, 7 failing with U0103, U2004, U5001, U3007, U3013, U3003 and U4003), 2 packages, 3 JSON documents, 11 commands with their exit codes |
| M10 task 3: release binaries for Linux and macOS | done (D-129): `scripts/release.sh`, `scripts/check-release.sh`, `.github/workflows/release.yml` | CI run 36632878216 at `58f7a07`, step "Pack and check the Linux release archive"; the macOS arm64 archive packed and checked on the owner's Mac |
| M10 task 3: the tag `v0.1.0` | left to the owner, after review and the merge to `main` (D-130) | `release.yml` runs on a pushed tag `v*` |
| Roadmap §3: the evaluation track drives `una` headlessly | done | H `an_agent_repairs_and_verifies_a_package_through_json_alone`; A `every_command_prints_one_json_document_with_its_schema` |
| Roadmap §4: usage checkpoint 4 has its demo | done | D `every_demo_script_finds_its_expected_outcomes` runs `scripts/demo/checkpoint-4.sh` |
| `compiler/testing.md §4`: determinism on each example | done: the test lists `examples/` | L `check_json_is_byte_identical_across_runs`; A `json_outputs_are_byte_identical_across_runs_and_thread_counts` |

### Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| The guide's snippets compile | G, over every block of `docs/guide/una-for-agents.md` |
| Release artifacts build in CI | CI run 36632878216: `una-0.1.0-x86_64-unknown-linux-gnu.tar.gz: OK`, and the packed binary checks, tests and runs the examples |
| The evaluation track can drive `una` headlessly through `--json` | H and A, as above |

### Found during the milestone

- Z3 5.1.0 rejects the quoted symbols `|_|` and `|as|`, and constants named `xor` or `distinct`; a discarded statement, Core's binder `_`, made every return obligation after it U8001. Four names now gain a suffix (`3b83602`, D-115), with a conformance case.
- `spec/12 §3` gave no hypothesis for `attempt`, so a successful attempt lost the callee's `ensures`. The `Ok` payload now keeps them, the `Err` path nothing (`de9ab73`, D-131, ADR 0033 rule 12), with a proven and a refuted case.
- Two obligations of the same kind and goal in one declaration share an id under the formula of `spec/12 §8`; the decision log asks the owner (D-118).
- `fold` carries no contract and refinements are not inferred through generics (`spec/03 §7`), so `pantry.stock.total` returns `Int`; the guide says so.
- The conformance runner: 193 cases, 193 passed, 0 pending.
