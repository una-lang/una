# 0019. Nesting limit

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo (accepted on 2026-09-29), on a proposal by Claude (overnight run)

## Context

The grammar of `spec/02` puts no bound on nesting: parentheses, arguments, prefix operators, operator chains, types and blocks can nest without limit. Every pass over the syntax tree recurses over it: the parser, the formatter, the checker, the verifier's encoding, the interpreter, and even the release of the tree's memory. Before this decision, `una check` aborted with a stack overflow on 20 000 nested parentheses, on a sum of 200 000 terms, and on 5 000 text literals nested in interpolations. An abort is not a diagnostic: it violates the robustness criterion of M1 (ADR 0017, decision 7) in spirit and gives an agent nothing to act on.

The limit must be deterministic, the same in every build, so it counts syntax, not bytes of stack. Measured stack use in unoptimized builds is about 34 KiB per level of nested parentheses, so no useful limit fits the 2 MiB of a default thread in every build.

Decision log entry: D-022 (`docs/reviews/open-decisions.md`).

## Decision

1. **The parser rejects nesting deeper than 256 levels** with U0101 "nesting deeper than 256 levels, counting each chained operator; split the code with `let` or a helper function", at the token where the limit is exceeded. One level is each nested line expression (parentheses, arguments, list items, interpolations, branches, lambda bodies), each prefix `not` or `-`, each operator of a chain (`a + b + c` counts two), each call, field or method in a postfix chain, each nested type, and each nested block, including each `else if`. The rest of the declaration is skipped by the usual recovery.
2. **A text literal nested in an interpolation**, already U0006, has no interpolation of its own: `{` and `}` inside it are text. This bounds the lexer's recursion.
3. **`una check` runs its passes on a thread with a 64 MiB stack.** Library callers that parse deeply nested input do the same; `una_syntax::parse` documents it.

### Changes to other documents on acceptance (made on 2026-09-29)

- `spec/02` gains a section "Limits": "An implementation MUST accept nesting up to 256 levels, counted as in ADR 0019, and MUST reject deeper nesting with U0101."

## Consequences

- No input crashes the lexer or parser, and every later pass can rely on a tree at most about 256 levels deep.
- A sum of more than 256 terms, or an `if ... then ... else if` chain of more than 256 branches, is rejected; `when`, lists and `let` bindings express the same programs without depth.
- The limit can be raised later without breaking any program.

## Alternatives considered

- **No limit, larger stack only.** A larger stack moves the crash further out but never removes it, and every later pass would need the same care.
- **A stack-size probe.** Measuring the remaining stack would make acceptance depend on the build and the platform, which breaks determinism.
- **Counting only true recursion, not operator chains.** Chains cost the parser no stack because it loops over them, but they still make the tree deep for every later pass and for releasing its memory.
- **A dependency that grows the stack on demand (`stacker`).** Not in the dependency allowlist (`compiler/architecture.md §7`), and it would still leave the depth of the tree unbounded for later passes.
