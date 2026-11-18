# 0011. Significant indentation with enforced canonical form

- Status: Accepted
- Date: 2026-09-28

## Context

Reviews raised the risk that generated patches break indentation. Braces are more robust to naive text edits, but indentation is more readable and removes a class of stylistic variation.

## Decision

Keep significant indentation (2 spaces, no tabs) and make canonical formatting part of the language: `una check` reports U7001 for non-canonical source, `una fmt` fixes it, and the Agent ABI returns mechanical edits for indentation errors.

## Consequences

- One canonical form, so diffs from different agents converge.
- Agents must produce correctly indented code; the formatter and precise diagnostics make repairs cheap.
- Revisit if Gate 1 data shows indentation errors dominating repair iterations.

## Alternatives considered

- **Braces and semicolons:** robust to naive edits, more tokens and more stylistic freedom.
