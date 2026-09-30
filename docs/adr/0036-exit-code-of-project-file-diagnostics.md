# 0036. Exit code of project-file diagnostics

- Status: Proposed
- Date: 2026-09-29
- Deciders: Leonardo (pending), on a proposal by Claude (fix request F-0001-03)

## Context

`spec/13 §5` defines the exit codes: "`0` success, `1` errors in the program or failed tests, `2` usage or configuration error, `3` internal compiler error." It does not say which of `1` and `2` a diagnostic about the project files carries.

- An invalid `una.toml` is U7002 and a missing or invalid `verification.lock` is U7004 (`spec/13 §4`, `§9`). Both are problems of configuration in the everyday sense of the word.
- Both are reported as diagnostics: they have a code, a domain (`project`), a position (`una.toml` 1:1 for a missing lock, ADR 0022) and an explain text, and they appear in `una.diagnostics/1`, and in the `diagnostics` of `una.run/1` (status `rejected`) and `una.test/1`.
- The reference compiler exits with `1` for both, as for every other diagnostic. Code `2` is used only for problems that are not diagnostics and go to stderr as plain text: an unknown flag, a path that is not a package (`spec/13 §9`), an unknown name for `una contract` or `una trace`, a bad `--grant` value, an unreadable key file.
- Agents read the exit code before the document. For them, `1` means "read the diagnostics and repair", and `2` means "the command itself was wrong; there is no document to repair from".

The review of the syntax reference guide (`docs/fixes/0001-syntax-guide-review.md`, F-0001-03) found the wording ambiguous. This ADR settles it; it does not change the compiler until it is accepted.

## Decision

Option A is proposed: **keep exit code `1`**, and make `spec/13 §5` say so.

1. Every problem reported as a diagnostic with a registered code, U7002 and U7004 included, exits with `1` (or `3` for U8001). This is the rule the compiler follows today.
2. Exit code `2` is for problems of the invocation that are not diagnostics and are printed to stderr as text: command-line flags and their values, grants given on the command line, a path that is not a package or a file, and the names given to `una contract` and `una trace`.
3. `spec/13 §5` replaces "`2` usage or configuration error" with "`2` usage error: a problem of the command line, its flags, grants or paths, printed to stderr without a diagnostic code", and adds: "A problem of the project files (`una.toml`, U7002; `verification.lock`, U7004) is a diagnostic, and exits with `1`."

## Consequences

- No change to the compiler, the ABI or any test. Agents keep one rule: a diagnostic code means exit `1`.
- The word "configuration" leaves the exit code table, so the guide and the spec cannot disagree about it again.
- A U7004 or U5003 caused by the environment (a missing lock, a solver of another version) still reads as "errors in the program" by exit code alone; the diagnostic's domain (`project`, `verification`) tells them apart.

## Alternatives considered

- **Option B: exit code `2` for U7002 and U7004.** It matches the everyday reading of "configuration error", and it lets a script tell a broken project setup from a broken program without parsing JSON. Not proposed because:
  - it changes the ABI that agents rely on after the v0.1 review, for a distinction the `domain` field already carries;
  - exit code `2` would then sometimes come with a JSON document on stdout and sometimes with none, where today it never has one;
  - `una run` and `una test` would need a second meaning of `rejected`, or a new status, since they carry these diagnostics in their documents.
- **Option C: leave the wording as it is.** Not chosen: the ambiguity is what the review found.
