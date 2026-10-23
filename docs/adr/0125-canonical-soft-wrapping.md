# 0125. Canonical soft wrapping at legal argument and element separators

- Status: Accepted
- Date: 2026-10-23
- Deciders: Una project (owner approved this decision on 2026-10-23)
- Resolves: issue #1113
- Amends: ADR 0020 rule 2; `spec/15 §2`

## Context

ADR 0020 rule 2 joins lines continued inside brackets and states that no line width applies. Long calls, signatures, and collections consequently remain difficult to review even when they contain legal places to continue. The repository corpus confirms this is a real, bounded problem: among 3,562 source lines in the measured examples and guide snippets, 89 lines have at least 80 Unicode scalar values, 50 at least 88, 30 at least 100, and 13 at least 120. The longest measured line is 221 scalars in `examples/pantry/src/pantry/main.una:6`; a fold call in that example is 152 scalars. These counts use Unicode scalar values, exclude blank and comment-only lines, and are a snapshot rather than a promise about future source.

`una fmt` and `una check` share one canonical form. A width policy therefore changes canonical output and can produce U7001 differences. It must be mechanical, preserve syntax and comments, and not split indivisible tokens or arbitrary expressions.

## Decision

1. The formatter uses a soft line-width target of 100 Unicode scalar values, counting indentation and every character in the rendered line. The target is not a hard maximum: a line may exceed it when no legal delimiter separator can keep it within the target or when an indivisible token or comment is longer.
2. Automatic line breaks are permitted only after a comma that separates arguments or elements inside parentheses or square brackets, where the grammar accepts a continued line. The formatter does not wrap at operators, before a comma, inside a token, or at arbitrary whitespace. It greedily keeps the latest legal separator whose preceding line fits the target; if no separator before overflow fits, it uses the first available legal separator so subsequent items can continue on their own lines.
3. A wrapped continuation is indented two spaces deeper than the indentation of the delimiter containing the chosen separator. Each nested parenthesis or square-bracket delimiter adds two spaces. If a delimiter group wraps, its closing delimiter is written on its own line at the indentation of its opener; a comma following that close stays on the same line. The normal two-space indentation rule still applies.
4. If a logical line contains a comment, its existing physical line breaks and comment placement take precedence over automatic wrapping. The formatter does not move, split, or rewrite a comment to meet the target.
5. The policy applies identically in all editions. It changes formatting only; it does not change the grammar, token spelling, AST, diagnostics other than canonical-form U7001 differences, or edition semantics. `una fmt` rewrites eligible lines and `una check` checks the resulting canonical text.
6. The formatter processes each output line in O(n) time and O(n) additional space in its token and character count, storing the comma-separated segments and rendered output. The greedy pass measures each segment once; it does not search globally for an optimal layout. It does not query terminal dimensions and has no width option.
7. Existing examples and source files are not mass-reformatted as part of this change. Any repository-wide canonical migration is a separate, reviewable change.

## Consequences

Calls and collections with legal separators become easier to scan and produce stable output independent of editor width. Wrapped groups visibly return their closing delimiter to the opener's indentation. A long atom, string literal, comment, or expression without an eligible comma remains long. Comment-bearing logical lines can exceed the target to preserve their source placement. The new canonical rule can cause U7001 for existing overlong lines with eligible separators; a formatter migration must be reviewed separately.

The width and continuation rule are now language-wide canonical-form requirements, rather than editor preferences. All formatter implementations must count Unicode scalar values and use the same separator and indentation rules.

## Alternatives considered

- **Keep unlimited lines.** This preserves every existing canonical file but leaves the measured long calls and collections unchanged.
- **Hard-wrap at 100 or split arbitrary tokens and expressions.** This would require splitting tokens, changing expression layout at non-delimiter boundaries, or rejecting source that the grammar accepts; it is not selected.
- **Break at any whitespace or operator.** This permits line layouts whose validity or grouping is harder to determine and does not meet the approved restriction to legal argument and element separators.
- **Use display columns or terminal width.** Display width depends on Unicode rendering and terminal state; terminal-dependent output would not be canonical across environments.
- **Apply wrapping only in a future edition.** Formatting is edition-independent and does not alter parsing or semantics; edition-specific behavior would create unnecessary canonical differences.
- **Mass-reformat the repository with the implementation.** This would mix implementation and mechanical migration, obscuring the behavior change; migration is deferred to a separate review.
