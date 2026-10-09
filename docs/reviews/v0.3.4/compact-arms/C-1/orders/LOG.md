# orders log

## check run 1
- U0101 x3 "unexpected end of line, expected an expression" (at `-> T =` with the body on the next line)
- consulted: message only (+ una-compact.md §Functions: `=` then the body below)
- enough: yes
- changed: put `=` on its own line before multi-line bodies
- DOC: una-compact.md §Functions says "then `=` and the body below" but does not say that a header without clauses but with a multi-line body must also put `=` on its own line.

## check run 2
- U1001 "cannot find type `Malformed` in this module, its imports or the prelude" (on `kind OrderError = Malformed`)
- U2002 "`fails` names a sum kind, and `OrderError` is not one"
- U1001 x4 "cannot find variant `Malformed` in this module, its imports or the prelude"
- consulted: una explain U2002, then the example `pub kind SlugError = | Malformed` found in una explain U2016
- enough: yes (but only by finding an example in another code's explanation)
- changed: `pub kind OrderError = | Malformed` (leading bar makes a one-variant sum)
- DOC: una-compact.md §Functions shows `kind P = A(x: Int) | B` but not that a one-variant sum needs a leading `|`; `kind E = V` is silently an alias, and U2002/U1001 do not suggest the `|`.

## check run 3
- no diagnostics, exit 0 (PASS)

## test run 1
- 1 example passed ("two order lines"), exit 0

## change: reject malformed input instead of defaulting (non-array `lines`, non-number `qty` now fail Malformed); added an example `expect fails Malformed`

## check run 4
- no diagnostics, exit 0 (PASS)
- consulted: n/a (`expect fails V` from una-compact.md §Examples)

## test run 2
- 2 examples passed, exit 0
