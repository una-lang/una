# 0018. A bracket after a label level starts its categories

- Status: Accepted
- Date: 2026-09-29
- Deciders: Owner (accepted on 2026-09-29), on a proposal by Owner (overnight run)

## Context

`spec/02 §2` defines `type = [ label ] type_core [ "in" range ]` and `label = level [ "[" upper_ident { "," upper_ident } "]" ]`, and `type_core` may itself start with `[` (`"[" type "]"`, a list type). After a level such as `secret`, a `[` followed by an upper-case name can start either the label's categories (`secret[PII] Phone`) or a list type (`secret [Int]`). The introduction of `spec/02` says the grammar is LL(2) except for kind bodies; telling the two readings apart here needs unbounded lookahead or backtracking, because the category list can be long. Labels and categories are defined in `spec/07 §2`; `List[T]` is the named form of a list type (`spec/03 §1`, `spec/14`).

Decision log entry: D-018 (`docs/history/v0.1/open-decisions.md`).

## Decision

After a level, `[` always starts the label's categories. A labeled list type is written with the named form, `secret List[Int]`, or after the categories, `secret[PII] [Int]`.

When a label with categories is not followed by a type, U0101 reports it with a hint: "unexpected `)`, expected a type after the label; a labeled list type is written `secret List[T]`".

### Changes to other documents on acceptance (made on 2026-09-29)

- `spec/02 §2` gains: "After a `level`, `[` always starts the label's categories; a labeled list type is written `secret List[T]` (ADR 0018)."

## Consequences

- The parser stays LL(2) outside kind bodies, as `spec/02` states.
- `secret [Int]` is rejected with a precise message and a fix, instead of being accepted under a guess.
- Accepting `secret [Int]` later (alternative B) would break no program, so the decision is cheap to relax.

## Alternatives considered

- **B) Backtrack.** Read the categories; when no type follows, reread `[Int]` as a list type. It accepts more programs, but the meaning of the brackets then depends on what follows them, and `secret [A] [B]` would still read `A` as a category, which is easy to misread.
- **C) Require `List[T]` after any label.** Forbid `[T]` after a label even when categories come first. Stricter than needed, since `secret[PII] [Int]` is unambiguous.
