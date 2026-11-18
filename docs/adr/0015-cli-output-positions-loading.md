# 0015. CLI output streams, positions and interim loading

- Status: Accepted
- Date: 2026-09-28
- Deciders: Leonardo

## Context

Milestone M1 prints the first real diagnostics, and M0 left four questions open:

1. `spec/13 §5` requires that `--json` print machine output on stdout and nothing else, but does not say where human output and diagnostics go, nor how a failure to write output is reported (M0 exits with code 2).
2. `spec/13 §1` counts columns in Unicode scalar values, which is undefined for invalid UTF-8. `spec/01 §1` makes invalid UTF-8 and a byte order mark U0001 without saying how lexing and positions continue.
3. `una check [path]` needs sources in M1, but package loading (`una.toml`, module discovery) arrives in M3.
4. Diagnostic paths are relative to the package root (`compiler/architecture.md §6`), but a single file checked on its own, such as a single-module conformance case, has no declared package root.

## Decision

### D1. Output streams

1. Human-mode diagnostics go to stderr.
2. Command results go to stdout: the explain text, contracts, reports, obligation listings and the program output of `una run`.
3. With `--json`, the JSON document goes to stdout and stderr stays empty, except for internal compiler errors (U8001).
4. A failure to write output exits with code 2.

### D2. Columns

Columns count Unicode scalar values after decoding:

1. Each maximal ill-formed subsequence of invalid UTF-8 counts as one column (Unicode §3.9, as implemented by Rust's `String::from_utf8_lossy`).
2. A byte order mark is a scalar value, so it counts as one column: its U0001 is at 1:1 and the next character is at 1:2.
3. A tab counts as one column.
4. After U0001 the lexer continues, reading each invalid sequence as U+FFFD.
5. The conformance runner validates expected positions with the same rule.

### D3. Interim loading until M3 (expired in M3, as it foresaw; see ADR 0021)

Until M3 replaces it with package loading, `una check [path]` accepts a file, which is parsed alone, or a directory, whose `src/**/*.una` files are parsed in sorted path order, with no project checks. The path defaults to the current directory. A missing path, or a directory without `src/`, is a usage error (exit code 2). This decision expires in M3 and changes no specification text.

### D4. File paths in diagnostics

1. Paths are relative to the package root.
2. When a single file is checked, including a single-module conformance case, the package root is the file's directory, so the diagnostic file is the bare file name (for example `cap_as_value.una`).
3. The conformance runner compares file names from M1.

### Changes to other documents

- `spec/13 §5` gains D1; `spec/13 §1` gains D2.1 to D2.3 for positions and D4.1 and D4.2 for files.
- `spec/01 §1` gains D2.4.
- `compiler/testing.md §2` gains D2.5 and D4.3.

## Consequences

- stdout carries only what a command produces, so `una explain U0001 > file` and `una run` output never mix with diagnostics, and `--json` output stays parseable.
- Every position in a file with invalid UTF-8 is defined, so diagnostics after the first U0001 keep exact positions, and conformance cases can pin them.
- Until M3, `una check` parses files without `una.toml`, so it cannot report project errors (U7002, U1008); D3 is removed when package loading lands.
- Single-module conformance cases and ad hoc files report bare file names, which keeps expected positions short and deterministic.

## Alternatives considered

- **Human diagnostics on stdout:** symmetric with `--json`, but mixes diagnostics with the program output of `una run` and with command results.
- **Counting bytes of invalid sequences, or dropping them:** positions after the error would depend on the encoding of the damage rather than on what a person sees.
- **Stopping at the first invalid byte:** simpler, but hides every later diagnostic in the file.
- **Loading packages in M1:** pulls `una.toml` parsing and module discovery forward from M3.
- **Paths relative to the working directory or absolute:** absolute paths break determinism (`compiler/architecture.md §6`), and working-directory paths change with how the command is invoked.
