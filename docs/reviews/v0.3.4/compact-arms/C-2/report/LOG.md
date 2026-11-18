# report LOG

## Check run 1
- U0101: unexpected `=`, expected end of line
- Consulted: message only
- Enough: yes
- Change: a `when` arm is an expression, not a statement; compute the new Summary with `when` and assign it once after.
- DOC: una-compact.md §Functions/§Loops do not say that `when` arms take expressions only, so an assignment `x = e` cannot be an arm.

## Check run 2
- U3007: `parse_int` can fail with `ParseError`; handle the failure with an `if error` clause, or wrap the ca
- U2001: `when` with variant arms examines a value of a sum kind, and this value is `Int`
- U4001: parameter `text` of `summarize` is `Text`, and this value is `internal Text`: a flow from `internal`
- Consulted: message only; una explain U4001 (read earlier)
- Enough: yes
- Change: `attempt line.trim().parse_int()` and match `Ok(value)`; labeled the pure function `summarize(text: internal Text) -> internal Summary` and `add_line` likewise.
- DOC: una-compact.md §Library lists `parse_int()` without saying it fails `ParseError` (other entries, such as parse_json, name their failure).

## Check run 3
- U4001: field `count` of `Summary` is `Int`, and this value is `internal Int`: a flow from `internal` to `pu
- U4001: field `sum` of `Summary` is `Int`, and this value is `internal Int`: a flow from `internal` to `publ
- U4001: `summary` is `Summary`, and this value is `internal Summary`: a flow from `internal` to `public`
- U4001: `summary` is labeled `public`, and this update runs under a `internal` condition, which it would rev
- Consulted: message only
- Enough: yes
- Change: labeled the record fields `count: internal Nat`, `sum: internal Int`, and the loop binding `let edit summary: internal Summary`.

## Check run 4
- No diagnostics; exit 0. Obligations: all 7 proven (postconditions of add_line and summarize, refinements, `"\n".len > 0`).
- Consulted: n/a
- Enough: yes
- Change: none.

## una test (after run 4)
- `example report "blank and non-numeric lines are skipped" ... ok`, 1 passed, exit 0.

## una run (after run 4)
- Printed `0 valid readings, sum 0`, exit 0: wrong. `una --json run` trace showed `Files.read("data/readings.txt")` failed `NotFound`, hidden by `if error use ""`.
- Consulted: una explain U3012, then the run trace
- Change: read `"readings.txt"`: the path is relative to the granted directory `files:data:read`. `una run` then printed `4 valid readings, sum 56`.
- DOC: una-compact.md §Capabilities lists grants `console`, `env`, `http:https://host` but not the `files:<path>:<access>` form (only in `una run --help`), nor that a Files path is relative to the granted directory.

## Check run 5
- U1001: module `std.host` declares no `FilesError`
- Consulted: message only; una explain U1001 (says std.host holds the host failure kinds but does not name them)
- Enough: no
- Change: removed the import and `fails` and kept `if error pass`, to let U3008 name the error kind.
- DOC: una explain U1001 / una-compact.md do not list the failure kinds of std.host (names for Files, Env, Http errors).

## Check run 6
- U3008: `if error pass` passes a `FileError` failure, and this code declares no error kind; declare `fails F
- Consulted: message only (it carries the repair)
- Enough: yes
- Change: declared `fails FileError` on main, with `import std.host (FileError)`.

## Check run 7
- No diagnostics; exit 0.
- Consulted: n/a
- Enough: yes
- Change: none. A missing or unreadable file now makes `main` fail (U6001) instead of printing a false `0 valid readings`.

## una test (after run 7)
- 1 example, 1 passed, exit 0.

## una run (after run 7)
- Printed `4 valid readings, sum 56`, exit 0. Grants: `console`, `files:data:read` (read only).
