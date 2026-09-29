# Engineering conventions

Status: Normative for the reference implementation.

## 1. Rust

- Edition 2024, toolchain pinned in `rust-toolchain.toml`.
- Every crate starts with `#![forbid(unsafe_code)]` and `#![deny(missing_docs)]` for public items.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` pass on every commit.
- No `unwrap`, `expect`, `panic!`, `unreachable!` or indexing that can panic in library crates, except for internal invariants written as `// INVARIANT: <why this cannot fail>` next to the call. Tests may unwrap freely.
- Library crates return `Result<T, Vec<Diagnostic>>` or accumulate into a `Diagnostics` sink; they never print, exit or read environment variables.
- Prefer plain enums and exhaustive `match` over trait objects. Avoid macros except `derive`.
- Name things after the spec: `T-Call` is implemented in `fn check_call`, with a doc comment linking `spec/10 §5`.

## 2. Diagnostics

- Every diagnostic uses a code from `una-diag/src/codes.rs`, which mirrors `spec/13 §4` exactly. Adding a code means updating both, plus `una-diag/explain/U####.md`.
- Messages follow `CLAUDE.md` writing conventions.
- Spans point at the smallest construct that explains the error; related spans explain the other side (declaration, contract, capability).
- Internal errors use U8001 and include the stage and a short description; they are always bugs.

## 3. Tests

- Every spec rule with a diagnostic code has at least one failing and one passing conformance case.
- Unit tests live next to the code (`#[cfg(test)]`); conformance and end-to-end tests live in `tests/`.
- Snapshot changes are reviewed line by line; an unexplained snapshot change is a bug.

## 4. Commits and pull requests

- Conventional commits with milestone tags: `feat(check): M5 capability sufficiency (spec/10 §2)`.
- One milestone task per pull request. The description lists spec sections implemented and conformance cases added.
- A pull request that changes `docs/spec/` must reference an accepted ADR.

## 5. Documentation

- Public items have doc comments stating what, not how.
- Algorithms that implement a formal rule cite it (`/// Implements T-Op, spec/10 §5.`).
- No em dashes; American English.
