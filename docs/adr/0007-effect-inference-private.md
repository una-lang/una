# 0007. Effect and failure inference for private functions only

- Status: Accepted
- Date: 2026-09-28

## Context

Full headers on every helper make code heavy (review concern: "90% specification"). Public headers are the review and reasoning boundary.

## Decision

`pub` functions declare `uses` and `fails` explicitly. Private functions may omit them; the compiler infers the least solution over the module call graph and shows it in `una contract` and the build report.

## Consequences

- Public boundaries stay fully explicit; internal helpers stay light.
- Inference is module-local, so it never changes a public contract silently.

## Alternatives considered

- **Explicit everywhere:** maximal locality, heavy for helpers.
- **Inference everywhere:** light, but public authority would change without a visible diff.
