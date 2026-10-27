# CLAUDE.md

Una is a verification-first programming language for machine-generated software. This repository contains its reference compiler, written in Rust. Editions `0.1` and `0.2` are stable, edition `0.3` is a preview, and edition `0.4` is a preview under the terms of ADR 0121, declared in `ita.una` (`docs/spec/13-diagnostics-and-abi.md` §7). Read `docs/00-charter.md`, identify the package edition, and read its applicable chapters in `docs/spec/` and accepted ADRs before any task.

## Source of truth

When documents disagree, this order wins:

1. `docs/spec/` (normative language specification, RFC 2119 keywords)
2. `docs/adr/` (accepted architecture decisions)
3. `docs/compiler/` (implementation architecture and conventions)
4. `docs/plan/` (milestones, roadmap, backlog)

If code and spec disagree, the spec wins. If the spec is wrong, ambiguous or incomplete, do not resolve it silently in code: follow "Changing the spec" below.

## Scope guard

- In scope: the charter's v0.1 scope (§5) and v0.2 scope (§5b), plus work for editions `0.3` and `0.4` assigned by issue. Follow the edition terms of `spec/13 §7` and the applicable accepted ADRs.
- Everything else in `docs/plan/backlog.md` is out of scope, even when it looks easy.
- Do not add syntax, keywords, diagnostics codes, CLI commands or standard library items that the spec does not define.
- Keywords reserved for another edition (`docs/spec/01-lexical-structure.md`) must be rejected with U0103, not half-implemented. Edition-specific syntax is available only in the editions that `spec/13 §7` and the construct's chapter admit it in; edition `0.1` keeps its own keyword table.

## How to work

1. Board first: every change is a card on the "Una roadmap" board (https://github.com/orgs/una-lang/projects/1) in Ready before any code, with its spec or ADR reference (skill `board`). One card, one branch `dev/<issue>-<topic>` from `origin/main`, one pull request that `Closes #N`; status In progress -> In review -> Done.
2. Before coding, read the spec sections the card names and quote them in the PR. Tests first: conformance cases in `tests/conformance/` encode the rule (`docs/compiler/testing.md`), then the implementation, in both engines.
3. Keep a change scoped to its card. No drive-by refactors across crates.
4. After each task: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --locked`.
5. One session works here with subagents (`.claude/agents/`: builder, reviewer, mechanic, scout); procedures are skills (`.claude/skills/`); the state of the work is `.claude/state/STATE.md` and `.claude/plan/NEXT.md` (skill `state`). Read `.claude/docs/HARNESS.md` and `.claude/docs/WORKFLOW.md` once per session.
6. Only `main` is long-lived (plus `release/v0.1` and `release/v0.3`). A branch and its worktree (`.claude/worktrees/<issue>`) are deleted at merge.

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

GitHub Actions is not used: never wait for it. Validation is local and recorded (`docs/release-gates.md`):

- A pull request merges on a green build gate on this machine: `cargo build --workspace`, `cargo test --workspace --locked`, clippy and fmt, plus an adversarial review (skill `review`) for code changes (skill `merge`).
- The heavy evidence (sanitize, Miri, fuzzing, the hunt, timing oracles, Linux legs, benchmarks, parity measurements) runs on a release candidate, not per pull request (skill `release`). A tag needs `scripts/release-gates.sh <sha> <out>` with every gate PASS on the gated SHA and its summary committed as `docs/validation/<version>.md`.

Tests that need the SMT solver require Z3 at the exact version in `verification.lock`. If it is missing or mismatched, those tests must fail with diagnostic U5003, never skip silently.

## Performance

Every design choice favors the best average-case algorithmic complexity (project rule, 2026-10-06):

- An ADR states the time and space complexity of each option. Prefer O(1) or O(log n) operations and O(n) or O(n log n) passes. A quadratic choice needs a written reason and the project's acceptance.
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
