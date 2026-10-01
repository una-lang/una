# 0030. Canonical public repository

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo

## Context

Una was developed in a private repository whose history carries names and material that the public project must not: earlier names of the language, the previous reference example and the commit messages that describe them. Git history is not rewritten, so the public repository starts a new history instead. The repository stays private until launch, which limits its GitHub Actions quota.

## Decision

1. The canonical repository is una-lang/una. Its history starts at the initial public import, `25d865b` ("Initial public import of Una (v0.1, through M4)"), a root commit made from the working tree after the audits of ADR 0025 and ADR 0026.
2. The earlier history stays private in the legacy repository. Commit hashes cited in documents written before the import, such as the plans of `docs/history/v0.1/runs/`, the decision log, the morning report and `docs/history/v0.1/OVERNIGHT.md`, refer to the legacy repository.
3. Work continues on `dev/v0.1`; nothing is pushed directly to `main`, which the owner updates by merging after review.
4. No branch whose history includes commits from before the initial import, such as the legacy `main` and `overnight/v0.1`, is ever pushed to una-lang/una, and nothing is pushed to the legacy repository again.
5. CI runs on Linux for every push, and on macOS only for pull requests and pushes to `main` (D-083).

## Consequences

- The public history contains only the final name and the neutral examples.
- Links from the public repository to legacy commits do not resolve; the documents cite those commits by hash as history.
- macOS coverage during the run comes from local checks, and from CI on pull requests and merges to `main`.

## Alternatives considered

- **Rewriting the legacy history:** forbidden by the run's rules, and it would still carry the earlier names in other places.
- **Publishing the legacy repository:** exposes the earlier names and material.
