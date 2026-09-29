# 0017. Lexical and syntactic decisions for M1

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo

## Context

Implementing the lexer and parser of M1 (`spec/01`, `spec/02`) raised questions the specification did not answer: which code reports a malformed integer, whether ASCII control characters and invisible Unicode controls may appear in literals and comments, which code reports characters that cannot start a token, what a bare `}` in text means, how a missing interpolation `}` is reported, whether capability operation clauses have a canonical order, what an orphan doc comment is, and how to state the M1 robustness criterion, which as written ("always yields diagnostics") cannot hold because some truncated examples are valid programs.

## Decision

1. **U0007 "Malformed literal"** (domain `syntax`) is a new code. It reports:
   - an integer literal with a leading zero other than `0` itself, a doubled underscore, or a leading or trailing underscore, with the message "malformed integer literal `007`" and a mechanical edit to the canonical spelling (`007` to `7`, `1_` to `1`, `1__0` to `1_0`);
   - an unescaped `}` in a text literal outside an interpolation, with the message "unescaped `}` in text; write `\}`" and the mechanical edit.
2. **Control characters.** U+0000 to U+001F other than tab and line feed, U+007F, and the Unicode bidirectional controls U+202A to U+202E and U+2066 to U+2069 are U0001 anywhere in a file, including text literals and comments. Text that needs them writes `\u{...}` escapes. This defends reviewed code against Trojan Source attacks.
3. **Characters that cannot start a token** are U0001, one diagnostic per character. The message adds a hint for `;` (statements end at line breaks), `!` not followed by `=` (use `not`), `&` (use `and`), `'` (text literals use double quotes) and `_` starting a name (names start with a letter).
4. **A missing interpolation `}`**, as in `"hello {name"`, is a single U0004 at the outer opening quote.
5. **Capability operation clauses** have a canonical order checked by U0102: every `requires`, then every `ensures`, then at most one `fails`, then at most one `accepts`.
6. **A doc comment not followed by a declaration** is a plain comment, without error.
7. **The M1 robustness criterion** reads: for every example and every line boundary, parsing the prefix never panics and every reported position lies inside the prefix; a prefix that ends right after the first line of a multi-line declaration, or right after the `=` line of a long-form function, yields at least one diagnostic.

The design choices of the approved M1 plan (`plan/runs/M1-plan.md` §6) apply: positions and recovery per code, mechanical edits for U0002 and U0003, the explain file format, and `--tier` values 0 and 1.

### Changes to other documents

- `spec/13 §4` gains U0007.
- `spec/01 §1` gains decision 2 and the U0001 rule of decision 3; `spec/01 §3` gains decision 6; `spec/01 §6` gains decisions 1 and 4.
- `spec/02 §4` gains decision 5.
- `plan/milestones-v0.1.md` M1: acceptance covers U0001 to U0007 and uses the wording of decision 7.

## Consequences

- Agents get a precise code and a mechanical repair for malformed numbers and braces instead of a generic unexpected token.
- Reviewed source can no longer hide code behind bidirectional overrides or invisible control characters; the same text stays expressible through escapes.
- Capability declarations have one canonical form, which the M2 formatter can rely on.
- The robustness test checks a property that holds, while still catching truncations that the parser would otherwise accept silently.

## Alternatives considered

- **U0101 for malformed integers and braces:** no new code, but a generic message without a repair for a lexical mistake.
- **Allowing control characters in literals and comments, as `spec/01 §1` implied:** invisible in review, and the escapes already cover every legitimate use.
- **Leaving operation clauses unordered until M2:** would let two canonical-looking forms coexist until the formatter decides.
- **Keeping "always yields diagnostics":** unsatisfiable, because `hello.una` cut after its `module` line is a valid program.
