# CLAUDE.md

Una is a verification-first programming language for machine-generated software. This repository contains its reference compiler, written in Rust. Edition `0.1` is stable, edition `0.2` is the systems edition, edition `0.3` is a preview with explicit exceptions, and edition `0.4` is an opt-in preview governed by ADR 0121. Read `docs/00-charter.md`, identify the package edition, and read its applicable chapters in `docs/spec/` and accepted ADRs before any task.

## Source of truth

When documents disagree, this order wins:

1. `docs/spec/` (normative language specification, RFC 2119 keywords)
2. `docs/adr/` (accepted architecture decisions)
3. `docs/compiler/` (implementation architecture and conventions)
4. `docs/plan/` (milestones, roadmap, backlog)

If code and spec disagree, the spec wins. If the spec is wrong, ambiguous or incomplete, do not resolve it silently in code: follow "Changing the spec" below.

## Scope guard

- In scope: the charter's v0.1 scope (§5) and v0.2 scope (§5b), plus work for editions `0.3` and `0.4` assigned by issue. Follow the edition terms in `docs/spec/13 §7` and the applicable accepted ADRs. Edition `0.4` is an opt-in preview governed by ADR 0121; editions `0.1` to `0.3` keep their stated terms.
- Everything else in `docs/plan/backlog.md` is out of scope, even when it looks easy.
- Do not add syntax, keywords, diagnostics codes, CLI commands or standard library items that the spec does not define.
- Keywords reserved for another edition (`docs/spec/01-lexical-structure.md`) must be rejected with U0103, not half-implemented. Edition-specific syntax is available only when the selected edition admits it under `docs/spec/13 §7`; edition `0.1` keeps its own keyword table.

## How to work

1. Work milestone by milestone, in order (`docs/plan/milestones-v0.1.md`). A milestone is done only when all its acceptance criteria pass.
2. Before coding a task, read the spec sections the milestone lists. Quote the section in the PR or commit description.
3. Tests first: add conformance cases in `tests/conformance/` that encode the spec rule (see `docs/compiler/testing.md`), then implement.
4. Keep changes scoped to one milestone task. No drive-by refactors across crates.
5. After each task: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

## Work rules

GitHub issues are the unit of work, tracked on the org project "Una roadmap" (https://github.com/orgs/una-lang/projects/1); `docs/plan/backlog.md` stays the scope guard, and each of its rows links to its issue.

1. Work only on an issue in Ready that the coordinator assigned to you (the board's Agent field). Anything new you find goes to the coordinator first, who creates its issue before work starts. New issues use `.github/ISSUE_TEMPLATE/work-item.md`.
2. Branch `dev/<issue>-<topic>` from `main`, for example `dev/6-integration-rule`. One issue per branch and per pull request, and one writer per branch.
3. Every commit references its issue: `Refs #6`.
4. The pull request into `main` closes its issue. Write the keyword once per issue, `Closes #6` and `Closes #7` on separate lines: a list after one keyword, `Closes #6, #7`, closes only the first.
5. Status on the board: set In progress when you start, and In review when the pull request is pushed with CI green. Only the coordinator sets Done. An item is Done when it is merged into `main`.

## Commands

```bash
cargo build --workspace
cargo test --workspace
cargo insta review                                    # review snapshot changes; never accept blindly
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p una-cli -- check examples/events        # end-to-end smoke test
```

## Validation

GitHub Actions is not used: never wait for it. Validation is local and recorded (#232, `docs/release-gates.md`):

- A pull request merges on a `scripts/local-ci.sh` PASS on its exact tip, on macOS. Linux is confirmed on `main` by the next gate run (`scripts/local-ci.sh --linux`, in Docker).
- Except a pull request that changes what the Linux run runs (`scripts/local-ci.sh`, `scripts/local-ci/`, a script one of its steps calls, or a new tool a step needs): it also needs a `scripts/local-ci.sh --linux-arm64` PASS on its exact tip (#605, `docs/release-gates.md` §1 rule 1).
- A tag needs `scripts/release-gates.sh <sha> <out>` with every gate PASS on the gated SHA, and its public summary committed as `docs/validation/<version>.md` (and, until the repository is public, the full record as `docs/releases/<version>-validation.md`) in the tagged commit (`docs/release-gates.md` §1 rule 2). The release archives come from `scripts/local-release.sh`.

Tests that need the SMT solver require Z3 at the exact version in `verification.lock`. If it is missing or mismatched, those tests must fail with diagnostic U5003, never skip silently.

## Performance

Every design choice favors the best average-case algorithmic complexity (owner's rule, 2026-10-06):

- An ADR states the time and space complexity of each option. Prefer O(1) or O(log n) operations and O(n) or O(n log n) passes. A quadratic choice needs a written reason and the owner's acceptance.
- A pull request that touches a hot path (the runtime, the interpreter, codegen, the checker's fixpoints, the verifier's encoding) states the complexity it achieves. It includes a scaling test at growing sizes that would fail on quadratic behavior.
- Value semantics stay cheap: updates of uniquely held values happen in place (#30, #201), and shared values are copied only when written.

## Rust rules (full list in `docs/compiler/conventions.md`)

- No `unsafe`, with one exception: the FFI module of `una-rt` (`ffi.rs`), at the boundary with generated native code, where every unsafe block carries a `// SAFETY:` comment (ADR 0058).
- No `unwrap`, `expect` or `panic!` in library crates, except for internal invariants marked `// INVARIANT: <reason>`. User-facing problems are `Diagnostic` values with a registered code.
- Library crates never print. Only `una-cli` writes to stdout or stderr.
- Deterministic output: no `HashMap` iteration in anything user-visible (use `BTreeMap` or `IndexMap`), no timestamps, no randomness, stable sort orders.
- Every public item has a doc comment. Every diagnostic code has an entry in `docs/spec/13-diagnostics-and-abi.md` and an `explain` text.

## Changing the spec

1. Write `docs/adr/NNNN-short-title.md` from `docs/adr/0000-template.md` with status `Proposed`.
2. Stop the task and report the ADR. Do not implement a proposed change until it is marked `Accepted`.
3. Once accepted, update the spec in the same change as the implementation.

## Writing conventions for docs and messages

- English, American spelling. No em dashes.
- Normative text uses MUST, MUST NOT, SHOULD, MAY.
- Diagnostic messages: one sentence, lowercase start, no trailing period, name the construct (`capability @audit is not in scope`).
