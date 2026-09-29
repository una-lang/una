# 0021. Packages and project files

- Status: Provisional (implemented, pending human review)
- Date: 2026-09-29
- Deciders: Claude (overnight run), pending review by Leonardo

## Context

ADR 0015 D3 defined interim loading until M3: a file alone, or a directory's `src/**/*.una` with no project checks. M3 replaces it with packages (`plan/milestones-v0.1.md` M3, task 1). `spec/13 §7` gives the layout ("`una.toml` and `verification.lock` at the root, sources under `src/`"), an example `una.toml` and one rule ("Unknown keys are U7002"); `spec/12 §9` gives the six keys of `verification.lock`; `spec/07 §6` requires trusted modules to be declared in both places (U7005); `compiler/testing.md §2` checks single-module cases "with default `una.toml` settings and without the module path check (U1008)". The specification does not say which keys are required, which values are valid, or how a single file is configured.

Decision log entries: D-032 to D-035 (`docs/reviews/open-decisions.md`).

## Decision

1. **Packages.** `una check`, `una fmt` and later commands accept a file or a directory, `.` by default.
   - A directory is a package only if it contains `una.toml` and `src/`; otherwise the command stops with a usage error (exit code 2). Its modules are the files `src/**/*.una`, hidden entries excepted (D-009), and the module in `src/a/b/c.una` is declared `module a.b.c` (U1008).
   - A file is a package of one module, whose root is the file's directory (ADR 0015 D4). It has the default settings of rule 2, the default lock of rule 3, and no module path check.
2. **`una.toml`.** A TOML document with these tables and keys, and no others (U7002):
   - `[package]`, required: `name`, `version` and `edition`, non-empty strings; `edition` is `"0.1"`.
   - `[labels]`: `categories`, an array of category names (UpperCamelCase, U1005; unique, U1002).
   - `[trusted]`: `modules`, an array of module paths, each a module of the package whose header says `trusted module` (U7005).
   - `[escape_budget]`: `checked_per_module`, an integer, at least 0 (default 0).
   - `[run]`: `entry`, the path of a module of the package; `grants`, an array of grant specs of `spec/13 §5`.
   - A TOML syntax error, a missing required key and a value of the wrong type or outside its range are U7002. The default settings are an empty `[labels]`, `[trusted]` and `[run]`, and a budget of 0.
3. **`verification.lock`.** A TOML document with exactly the keys of `spec/12 §9`: `encoding_version = "una-smt-1"`, `solver = "z3"`, `solver_version` a non-empty string, `logic = "QF_UFLIA"`, `step_budget` an integer of at least 1, and `random_seed` an integer of at least 0. A missing file, a TOML syntax error, a missing or unknown key and a wrong value are U7004. Whether the installed solver matches is U5003 (M8). A single file uses the default lock, which holds the values of the repository's `verification.lock`.
4. **Best effort.** Project errors do not stop checking: the modules are still checked, with the default settings in place of an invalid `una.toml`.

### Changes to other documents on acceptance

- `spec/13 §7` states rules 1 to 3; ADR 0015 D3 is marked expired, as it foresaw.

## Consequences

- Every package is self-describing, and a single file stays checkable without any project file, which keeps conformance cases short.
- A single file with a `trusted module` header is U7005, since it has no `una.toml` to agree with (ADR 0023).
- Stricter than the example in `spec/13 §7` requires: loosening a key later breaks no package.

## Alternatives considered

- **Keep loading `src/` without `una.toml`:** a directory would be checked without its settings, so labels, trusted modules and budgets would silently take defaults.
- **Accept unknown keys with a warning:** `spec/13 §7` makes them U7002, and v0.1 has no warnings.
- **Require a lock for single files:** every conformance case would need a directory.
