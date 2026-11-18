# 0014. Reserved keywords and U0103 messages

- Status: Accepted
- Date: 2026-09-28
- Deciders: Owner

## Context

`spec/01 §5` reserves keywords that v0.1 does not implement and requires U0103 "with a message naming the planned version". No document maps these keywords to versions: `plan/backlog.md` lists features, not keywords, and several reserved words (`match`, `return`, `var`, `while`, `loop`, `type`, `unsafe`, `macro`, `async`, `await`) name constructs that Una does not plan to add, or already provides under another name. Milestone M1 implements U0103 and needs a message for every reserved keyword.

## Decision

1. U0103 means "Reserved keyword not available in v0.1" (`spec/13 §4`).
2. The message of U0103 names either the planned version and feature, or the alternative in Una. It is written `` `<keyword>` is <text> ``, with the text of this table, for example `` `match` is not a keyword in Una; use `when` ``.

   | Keywords | Message |
   | --- | --- |
   | `edit`, `take`, `region`, `shared`, `iso` | reserved for v0.2 (memory model) |
   | `together`, `parallel`, `actor` | reserved for v0.2 (concurrency) |
   | `simd`, `profile` | reserved for v0.2 (systems profile) |
   | `trait`, `impl` | reserved for v0.2 (constraints on generics) |
   | `law`, `all` | reserved for v0.3 (laws) |
   | `costs` | reserved for v0.3 or later (cost contracts) |
   | `traced` | reserved for v0.3 (provenance tracing) |
   | `service`, `table`, `query`, `observe`, `commit` | reserved for v0.4 (platform) |
   | `match` | not a keyword in Una; use `when` |
   | `return` | not a keyword in Una; the last expression is the result |
   | `var` | not a keyword in Una; values are immutable, use `let` |
   | `while`, `loop` | not a keyword in Una; use recursion or `fold` |
   | `type` | not a keyword in Una; use `kind` |
   | `unsafe` | not a keyword in Una; use a `trusted` module |
   | `macro` | not supported (charter §6) |
   | `async`, `await` | not supported; see structured concurrency in v0.2 |

3. The table is added to `spec/01 §5` and covers every keyword of its reserved list.

## Consequences

- An agent that writes `match` or `return` out of habit gets the Una construct to use instead, and one that reaches for a planned feature learns when it arrives.
- Adding or removing a reserved keyword means updating the table, and a moved roadmap item means updating its messages.
- The lexer can check mechanically that the table and the reserved list agree.

## Alternatives considered

- **A message naming only the planned version, as `spec/01 §5` said:** impossible for keywords with no planned version, and not actionable for the most common mistakes.
- **Separate codes for "future keyword" and "never a keyword":** two codes for one lexical rule, with no difference in how a program is repaired.
