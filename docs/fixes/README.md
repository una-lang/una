# Fix requests

Status: Informative. This folder holds work orders for corrections to the v0.1 compiler and its documentation, found by reviews after a milestone closed. It does not define the language: `docs/spec/` does.

## How this folder relates to the others

| Folder | Holds | Scope |
| --- | --- | --- |
| `docs/spec/` | the normative language | defines what is correct |
| `docs/adr/` | decisions that change or complete the spec | changes the spec |
| `docs/plan/backlog.md` | features deferred to later versions | out of scope for v0.1 |
| `docs/fixes/` | defects and documentation errors in what v0.1 already ships | in scope for v0.1 |

A fix request never adds syntax, keywords, diagnostic codes, CLI commands or standard library items. If a fix would need one, or if the spec itself is wrong or silent, follow "Changing the spec" in `CLAUDE.md`: write an ADR with status `Proposed`, stop that item, and report it.

## File format

One file per review, named `NNNN-short-title.md`, numbered in order. Each file has:

1. A header: status, date, source of the findings, and the commit they were found at.
2. A list of items, each with a stable id (`F-NNNN-NN`), a kind (`compiler`, `docs`, `investigate`), the evidence, the expected behavior with its spec reference, and acceptance criteria.
3. A progress table at the end, updated by the implementing agent as items close.

Status values for a file: `Open`, `In progress`, `Done`. An item is done only when its acceptance criteria pass together with `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.

## Index

| File | Title | Status |
| --- | --- | --- |
| [0001-syntax-guide-review.md](0001-syntax-guide-review.md) | Review of the syntax reference guide against the v0.1 compiler | In progress |
