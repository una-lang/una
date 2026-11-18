# 0027. Interim run behavior before tier 1

- Status: Accepted
- Date: 2026-09-29
- Deciders: Owner

## Context

Checkpoint 3 of the v0.1 usage checkpoints (`plan/roadmap.md §4`) makes Una runnable after M7, before M8 verifies proof obligations. A program with an unproven obligation does not compile (`spec/06 §7`), but until M8 the toolchain cannot prove any, so `una run` and `una test` must either wait for M8 or run programs whose obligations are not yet verified, and say so.

## Decision

1. From M7 until M8, `una run` and `una test` perform the tier 0 checks (`spec/12 §1`) before executing. A program with tier 0 errors is not run.
2. In human mode they print one line to stderr: `note: proof obligations are not verified yet (tier 1 arrives in M8)`.
3. In `--json` mode their output carries `"tier": 0` and no note.
4. M8 removes this behavior: from then on, `una run` and `una test` check at tier 1, as `spec/12` requires. The M8 plan records the removal.

## Consequences

- Checkpoint 3 is usable before M8, and every run says that its obligations are unverified.
- Agents reading `--json` output can tell an interim run from a verified one.
- M8 must remove the note and the `tier` field, or bump the schema version if the field stays.

## Alternatives considered

- **Wait for M8 before running anything:** delays checkpoint 3, the first usable version.
- **Run without any note:** hides that no obligation has been proven, against charter P8.
