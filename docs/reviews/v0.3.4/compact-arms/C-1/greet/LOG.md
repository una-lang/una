# greet log

## check run 1
- U0101 "unexpected end of line, expected `(` and the imported names" (import line)
- U0101 "unexpected `,`, expected end of line" (uses line with two capabilities)
- U7004 "the package has no verification.lock; add one next to una.toml"
- consulted: una explain U7004 (lock content); una explain U0101 + message only for the parse errors
- enough: yes for U7004; partly for U0101 (message names the expected token, but not the import syntax nor how to declare two capabilities)
- changed: added verification.lock from the explain example; `import release (shown)`; one `uses` line per capability
- DOC: una-compact.md says "Full guide: `una-for-agents.md`" but only una-compact.md ships in docs/guide; the README's links (syntax-reference.md, getting-started.md) are also missing from the archive.
- DOC: una-compact.md §Package mentions verification.lock but not its content; only `una explain U7004` gives it.
- DOC: una-compact.md never shows `import` syntax nor how to declare several capabilities in one header.

## check run 2
- no diagnostics, exit 0 (PASS)
- consulted: n/a
- changed: nothing

## run 1 (`una run`)
- printed "Hello, <user>!", exit 0
- the USER value is `internal`; it is lowered to public through the trusted policy `release.shown` (pattern from una-compact.md §Labels)

## test run 1 (`una test`)
- 0 examples, exit 0 (the task asks for no example)
