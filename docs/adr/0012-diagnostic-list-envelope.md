# 0012. Diagnostic lists and unassigned code numbers

- Status: Accepted
- Date: 2026-09-28
- Deciders: Owner

## Context

`spec/13 §1` defines the JSON shape of one diagnostic, tagged `"schema": "una.diagnostic/1"`, and `spec/13 §5` says that `--json` prints machine output on stdout and nothing else. Neither section defines the document a command prints when it reports a list of diagnostics, including the empty list. The other Agent ABI documents are objects with a top-level `schema` field: `una.obligations/1` wraps a list of obligations (§2) and `una.report/1` is the build report (§6).

Milestone M0 needs this document. Its acceptance criteria require that `una check --json` print an empty diagnostic list (`plan/milestones-v0.1.md`), and `compiler/testing.md §4` requires byte-identical output across runs. The JSON shapes of `spec/13` are the versioned Agent ABI, so this shape must be fixed through the ADR process rather than chosen in code.

Separately, `spec/13 §4` states that retired codes stay in the table marked retired, but the numbering has gaps (U4006, U6002) that are neither listed nor marked. The code registry built in M0 mirrors the table, so it must know whether these are retired codes.

## Decision

### 1. Diagnostic lists

1. When a command reports diagnostics and `--json` is given, stdout carries exactly one JSON object followed by a single newline:

   ```json
   { "schema": "una.diagnostics/1", "diagnostics": [] }
   ```

2. `diagnostics` is an array of diagnostics with the shape of `spec/13 §1`, in the order §1 prescribes (file, start position, code). Each element keeps its own `"schema": "una.diagnostic/1"` field, exactly as §1 defines it, so a diagnostic copied out of the list stays self-describing.
3. The list schema `una.diagnostics/1` mirrors `una.obligations/1`. Its elements are diagnostics of schema `una.diagnostic/1`; a new version of the element shape also bumps the list version.
4. When there are no diagnostics, `diagnostics` is the empty array. Exit codes are those of `spec/13 §5` and do not depend on `--json`.
5. Whitespace is not significant to consumers. The reference implementation prints compact JSON with object keys in the order shown in `spec/13 §1`, so output is byte-identical across runs.
6. This decision covers `una check --json`, and `una fmt --check --json` once `una fmt` exists (M2). Commands whose JSON output has another shape (`una obligations`, `una report`, and `una test` per `spec/08 §2`) are unaffected; how they carry diagnostics is decided with `--json` consistency in M9.

### 2. Unassigned code numbers

Gaps in the code numbering, such as U4006 and U6002, are unassigned numbers, not retired codes. The code registry mirrors the table of `spec/13 §4` as is.

### Changes to other documents

- `spec/13 §1` gains a paragraph stating items 1 to 5 of decision 1, in the same change as the M0 JSON rendering.
- `spec/13 §4` gains a sentence stating decision 2, in the same change as the M0 code registry.
- The M0 acceptance criterion in `plan/milestones-v0.1.md` becomes: `una check --json` prints an empty list with schema `una.diagnostics/1`.

## Consequences

- Agents read one document per invocation and find `schema` at the top level, as with obligations and reports.
- An empty result is explicit and versioned: `{"schema":"una.diagnostics/1","diagnostics":[]}`.
- Lists and single diagnostics have distinct schema names, so a consumer never mistakes one shape for the other.
- Each element repeats its schema string, which costs a few bytes per diagnostic.
- Diagnostics cannot be streamed while the compiler runs. v0.1 does not need streaming, and the ordering rule of §1 already requires collecting all diagnostics before printing.
- Top-level fields (for example a summary) can be added later under a new list schema version.
- The registry has no entries for U4006 and U6002.

## Alternatives considered

- **Bare array of diagnostics:** the simplest shape, but the empty list `[]` carries no schema, and the top level cannot gain fields without breaking consumers.
- **JSON Lines, one diagnostic per line:** streams well, but an empty result is empty output with no schema, and sorted output removes the benefit of streaming.
- **An envelope reusing `una.diagnostic/1`:** matched the original wording of the M0 criterion, but one schema name would denote two shapes.
- **Treating U4006 and U6002 as retired:** would require listing them in §4 as retired codes although they were never assigned.
