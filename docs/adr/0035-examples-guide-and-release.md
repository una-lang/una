# 0035. Examples, guide and release for Gate 1

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo (accepted on 2026-09-29), on a proposal by Claude (overnight run)
- Amended by: the owner's closure instruction of 2026-09-30 ("the macOS release archive must build in CI on the closure pull request"): rule 3's archives are also packed and checked by `ci.yml` on each runner, Linux on every push and macOS on pull requests and pushes to `main` (R-035); ADR 0059 (rule 3: the archive also ships `lib/libuna_rt.a`, and its check builds `examples/hello` natively), on 2026-10-02

## Context

M10 (`plan/milestones-v0.1.md`) prepares Gate 1 (`plan/roadmap.md §3`): at least five more example packages "covering every construct in the charter scope", a guide for agents whose every snippet is tested, and release binaries for Linux and macOS with the tag `v0.1.0`. The plan names neither the examples nor how coverage and snippets are checked, nor the targets and contents of the release. D-083 runs macOS jobs only on pull requests and pushes to `main`, and `docs/history/v0.1/OVERNIGHT.md` C.5 forbids pushing to `main`, where the owner merges after review.

Decision log entries: D-127 to D-130 (`docs/history/v0.1/open-decisions.md`).

## Decision

1. **Example packages.** Five packages join `examples/`: `toolshelf`, `pantry`, `journal`, `greenhouse` and `shortlinks`, small programs in domain-neutral settings. Every package under `examples/` MUST pass `una fmt --check`, check at tier 1 with every obligation proven, pass `una test`, and report no trivial `ensures`; a package with a `[run]` table MUST run with its grants. Taken together, the examples MUST contain every construct of the language list of charter §5, each recognized by a source pattern that the example test lists: modules, imports and `pub`; aliases, records, sums with payloads, generic kinds and functions, `where` refinements and range sugar; reflect and opaque predicates; `requires`, `ensures`, `uses`, `fails` and `derived from`; labeled arguments, method calls and lambdas; `Int`, `Nat`, `Bool`, `Text`, `Unit`, `List`, `Option`, `Result` and `Hashed`; capability declarations, `with` rebinding and handler instances, handlers with and without `uses`, and the five host capabilities; `fail`, the five handling clauses, `attempt` and `checked`; label levels and categories, `accepts` and a policy in a trusted module; requirements from each source, and examples with `given`, `do`, `expect`, `expect fails` and `expect performed`.
2. **The guide's snippets.** In `docs/guide/una-for-agents.md`, every `una` fence is a whole module. It MUST check clean at tier 1, unless its first line is `-- fails with U####`, in which case it MUST yield exactly that code. Every `json` fence MUST parse, and its `schema` MUST be one the CLI prints. Every line of a `sh` fence that starts with `una ` is run from the repository root, or from the directory of a trailing `# in <dir>` comment, and MUST exit with 0, or with the code of a trailing `# exit N` comment. Fences introduced by a file path on the line before form a package, which MUST check clean, with a default `una.toml` unless one of its files is one; a fence of another language outside a package is an error of the guide.
3. **Release artifacts.** `scripts/release.sh <target>` builds `una` with `--release --locked` and writes `una-<version>-<target>.tar.gz`, holding `una`, `LICENSE` and `README.md` in a directory of the same name, and its `.sha256` file; `<version>` is the workspace version, and each target builds natively. `scripts/check-release.sh <target>` checks an archive: its digest, its contents, and the packed binary on the examples. `.github/workflows/release.yml` runs it for `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin` when a tag `v*` is pushed and attaches the archives to a draft release. The CI workflow packs and checks the Linux archive on every push; macOS archives build in the release workflow only (D-083), and locally.
4. **The tag.** The run does not create or push `v0.1.0`. The owner tags the reviewed commit on `main`, which starts the release workflow.

### Changes to other documents on acceptance (made on 2026-09-29)

- `plan/milestones-v0.1.md` M10 names the five packages and the targets; `compiler/testing.md` describes the example and guide tests.

## Consequences

- An agent reading the guide copies modules that check, and the codes the guide shows are the codes it will meet.
- A new construct in a later version needs an example and a pattern before the coverage test passes.
- The first release waits for the owner's review, and a macOS archive is first built in CI by that release.

## Alternatives considered

- **One package per charter bullet:** eleven fragments rather than programs an evaluation task can start from.
- **Coverage by a table in the plan only:** true on the day it is written, and silently false after the next edit of an example.
- **Guide fragments completed by a template:** shorter, but an agent that copies a fragment gets code that does not check on its own.
- **Both archives on every push:** the macOS runner quota of D-083.
- **Tagging `dev/v0.1` during the run:** a release would name history the owner has not reviewed or merged.

## Amendment (2026-10-02)

Rule 3's note on CI follows the runner rule of ADR 0030, which changes to spare the Actions minutes of the private repository (#151, Leonardo, 2026-10-02):
- `push` triggers CI only on `main` and on release tags, and a branch is tested by the run of its pull request, so no change runs twice.
- Linux runs every time.
- macOS runs on pushes to `main`, on release tags and on manual runs. On a pull request, it runs only when the pull request touches what differs between the targets: the native backend (`crates/una-codegen`, `crates/una-rt`, `una build`), the release scripts, the workflows or `Cargo.lock`.
- Compiled dependencies and the Z3 install are cached.

`.github/workflows/ci.yml` states the rule. D-083, in `docs/history/v0.1/`, keeps the rule of v0.1.
