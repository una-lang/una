# CLAUDE.md

Una is a verification-first programming language for machine-generated software. This repository contains the reference compiler, written in Rust: v0.1 ("Una Core") is released, and v0.2 is in development in parallel with Gate 1 (charter §5b). Read `docs/00-charter.md` before any task.

## Source of truth

When documents disagree, this order wins:

1. `docs/spec/` (normative language specification, RFC 2119 keywords)
2. `docs/adr/` (accepted architecture decisions)
3. `docs/compiler/` (implementation architecture and conventions)
4. `docs/plan/` (milestones, roadmap, backlog)

If code and spec disagree, the spec wins. If the spec is wrong, ambiguous or incomplete, do not resolve it silently in code: follow "Changing the spec" below.

## Scope guard

- In scope: the v0.1 scope of `docs/00-charter.md` §5 and, for v0.2 work, §5b (native backend, fixed-width integers, memory model; ADRs 0055 to 0057). Work only on the version and issue you are assigned.
- Everything else in `docs/plan/backlog.md` is out of scope, even when it looks easy.
- Do not add syntax, keywords, diagnostics codes, CLI commands or standard library items that the spec does not define.
- Keywords reserved for future versions (`docs/spec/01-lexical-structure.md`) must be rejected by the parser with U0103, not half-implemented. v0.2 syntax exists only under `edition = "0.2"` in `una.toml` (ADR 0056); edition 0.1 keeps the v0.1 keyword tables.

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

Tests that need the SMT solver require Z3 at the exact version in `verification.lock`. If it is missing or mismatched, those tests must fail with diagnostic U5003, never skip silently.

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
