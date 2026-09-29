# CLAUDE.md

Una is a verification-first programming language for machine-generated software. This repository contains the v0.1 reference compiler ("Una Core"), written in Rust. Read `docs/00-charter.md` before any task.

## Source of truth

When documents disagree, this order wins:

1. `docs/spec/` (normative language specification, RFC 2119 keywords)
2. `docs/adr/` (accepted architecture decisions)
3. `docs/compiler/` (implementation architecture and conventions)
4. `docs/plan/` (milestones, roadmap, backlog)

If code and spec disagree, the spec wins. If the spec is wrong, ambiguous or incomplete, do not resolve it silently in code: follow "Changing the spec" below.

## Scope guard

- Only the v0.1 scope in `docs/00-charter.md` section 5 is in scope.
- Everything in `docs/plan/backlog.md` is out of scope, even when it looks easy.
- Do not add syntax, keywords, diagnostics codes, CLI commands or standard library items that the spec does not define.
- Keywords reserved for future versions (`docs/spec/01-lexical-structure.md`) must be rejected by the parser with U0103, not half-implemented.

## How to work

1. Work milestone by milestone, in order (`docs/plan/milestones-v0.1.md`). A milestone is done only when all its acceptance criteria pass.
2. Before coding a task, read the spec sections the milestone lists. Quote the section in the PR or commit description.
3. Tests first: add conformance cases in `tests/conformance/` that encode the spec rule (see `docs/compiler/testing.md`), then implement.
4. Keep changes scoped to one milestone task. No drive-by refactors across crates.
5. After each task: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

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

- No `unsafe`.
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
