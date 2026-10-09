# report LOG

Consulted before writing: docs: syntax-reference.md §7.7 (Files.read -> internal Text, fails FileError; grant `files:<path>:read`), §15 (relative grant paths resolve against the package root when from una.toml), una-for-agents.md §9 (policy to print internal data), §16 (`parse_int`, `split`), §5 (`filter` result.len <= original, `map` same length).

## Check run 1
- Diagnostics:
  - U4001 field `count` of `Readings` is `Int`, and this value is `internal Int`: a flow from `internal` to `public`
  - U4001 field `sum` of `Readings` is `Int`, and this value is `internal Int`: a flow from `internal` to `public`
- Consulted: una explain U4001
- Enough: yes
- Change: first version (pure `summarize(lines: internal List[internal Text]) -> internal Readings` with `ensures result.count <= lines.len`; trusted policy `shown_summary`; `main` reads with grant `files:data:read`), then labeled the record fields `count: internal Nat` and `sum: internal Int`.

## Check run 2
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: the field labels above.

## Test run 1
- `una test`: 2 examples, 2 passed, exit 0. `una obligations`: postcondition `result.count <= lines.len` proven.

## Run 1
- `una run`: U6001 `main` failed with `NotFound` (exit 1). The program read `@files.read("data/readings.txt")`.
- Consulted: una explain U6001; docs: syntax-reference.md §7.7 ("Paths are confined to the granted prefix") and §15
- Enough: partly (the docs do not say how the read path is resolved; NotFound rather than Denied suggested it is relative to the granted directory)
- Change: read `@files.read("readings.txt")`, relative to the grant `files:data:read`.

## Check run 3
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: the read path above.

## Test run 2
- `una test`: 2 examples, 2 passed, exit 0.

## Run 2
- `una run`: printed `4 valid readings, sum 56`, exit 0 (also from another working directory).

DOC: syntax-reference.md §7.7 / §15 and una-for-agents.md §8 do not say how the path given to `Files.read` is resolved: it turned out to be relative to the granted directory (`files:data:read` + `readings.txt`), not to the package root or the working directory. "Confined to the granted prefix" reads as if the path should include the prefix (`data/readings.txt`), which gave NotFound.
DOC: una-for-agents.md §9 does not say that a record built from labeled values needs labels on its fields (U4001 at the constructor); `una explain U4001` covers it ("a value flows into ... a field").
