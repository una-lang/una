# report log

## check run 1
- U3007 "`parse_int` can fail with `ParseError`; handle the failure with an `if error` clause, or wrap the call in `attempt`"
- U2001 "`when` with variant arms examines a value of a sum kind, and this value is `Int`"
- U4001 "field `count` of `Readings` is `Int`, and this value is `internal Int`: a flow from `internal` to `public`"
- U4001 "field `sum` of `Readings` is `Int`, and this value is `internal Int`: a flow from `internal` to `public`"
- U4001 "`acc` is `Readings`, and this value is `internal Readings`: a flow from `internal` to `public`"
- U4001 "`acc` is labeled `public`, and this update runs under a `internal` condition, which it would reveal"
- consulted: message only (+ una explain U4001 read earlier: "a field ... cannot be raised" applies to library kinds only)
- enough: yes
- changed: `attempt ...parse_int()` with `is Ok(n)`; fields `count: internal Nat`, `sum: internal Int`; `let edit acc: internal Readings`
- DOC: una-compact.md §Library/Text lists `parse_int()` without saying it fails with `ParseError` (I assumed it returned an Option).
- DOC: U4001 reports `count: Nat` as "`Int`": the message drops the refinement name, which is confusing.

## check run 2
- no diagnostics, exit 0 (PASS); `una obligations`: all 6 proven (incl. `result.count >= 0` and the two postconditions of `add_line`)

## test run 1
- 2 examples passed, exit 0

## run 1 (`una run`)
- U6001 "`main` failed with `Unreadable`" (exit 1): `@files.read("data/readings.txt")` failed under grant `files:data:read`
- consulted: una explain U6001; docs: una-compact.md §Capabilities (grants) — neither says how a `files:<path>` grant maps paths given to `Files.read`
- enough: no
- changed: try reading "readings.txt" (path relative to the granted directory)
- DOC: una-compact.md §Capabilities lists the `files` grant only as "Files[read, write]"; `una run --help` gives `files:<path>:<access>`, but nowhere says whether `Files.read` paths are relative to the package, the grant or the cwd, nor what access values exist.

## check run 3
- no diagnostics, exit 0 (PASS)

## run 2 (`una run`)
- printed "4 valid readings, sum 56", exit 0 (data: 12, 7, x, -3, blank, 40)
- enough: yes — paths given to `Files.read` are relative to the directory of the `files:` grant
