# LOG: report

## Check run 1
- Diagnostics: U4001 "parameter `lines` of `summarize` is `List[Text]`, and this value is `internal List[Text]`: a flow from `inte"
- Consulted: una explain U4001; docs: una-for-agents.md §9 (Files.read text is internal; a label covers one position), §16 (split results carry every label).
- Enough: yes.
- Change: labeled the pure function: `summarize(lines: internal List[internal Text]) -> internal Summary`, Summary fields `internal Nat`/`internal Int`, `describe` returns `internal Text`; only the final printed line goes through the policy `shown_summary`.

## Check run 2
- Diagnostics: none (exit 0). `una obligations`: postcondition `result.count <= lines.len` proven (from the contracts of `map` and `filter`).
- Consulted: (nothing new)
- Enough: yes.
- Change: (as above)

## Test run 1
- `una test`: 3 examples, 3 passed (exit 0).

## Run 1
- `una run`: printed `cannot read data/readings.txt` (exit 0). `una --json run` trace: Files.read("data/readings.txt") failed with `NotFound` (not `Denied`), with grant `files:data:read`.
- Consulted: docs: syntax-reference.md §7.7 ("Paths are confined to the granted prefix"), §15 (relative grant paths resolve against the package root).
- Enough: partly: the docs do not say whether the path given to `Files.read` is relative to the granted directory, to the package root or to the current directory.
- DOC: syntax-reference.md §7.7 / §15: how the `path` argument of `Files.read` is resolved (relative to the grant prefix? to the cwd?) is not stated; a file inside the granted directory, named with the grant prefix, gives `NotFound`.

## Check run 3
- Diagnostics: none (exit 0).
- Consulted: the run trace of `una --json run` (NotFound for "data/readings.txt" even with cwd = package root), then inferred that the path is relative to the granted directory.
- Enough: yes.
- Change: `@files.read("readings.txt")` instead of `"data/readings.txt"`; grant stays `files:data:read` (read only).

## Test run 2
- `una test`: 3 examples, 3 passed (exit 0).

## Run 2
- `una run`: printed `4 valid readings, sum 56` (exit 0), from the package dir and from another cwd. (File: 12, 7, x, -3, blank, 40.)

## Check run 4 (final verification sweep)
- Diagnostics: none (exit 0); `una test` also exit 0.
- Consulted: (nothing)
- Enough: yes
- Change: none
