# Review of the native backend and runtime (issue #176)

Status: done, 2026-10-06. The verdicts, the fixes' rechecks and the open findings (#198, #200) are in `summary.md`. The first round's findings are #180 to #185.

Each case under `adversarial/` is a package that passes `una check` at tier 1 in canonical form, except `equiv/refinefn`, which `main` now rejects (`summary.md`). To reproduce one, copy it outside the repository, then compare the interpreter with the native executable:

```sh
una run .
una build . && ./target/una/<name>
```

| Area | Cases | Issues |
| --- | --- | --- |
| `adversarial/integers/` | fixed-width semantics and machine widths (FW-1); `reflect-predicate-fixed-overflow` is INT-1 | #180, #185 |
| `adversarial/memory/` | the unsafe boundary, reference counts and in-place updates; `stale_alias_reuse*` is MEM-1, `loop_bound_is_state` is MEM-2 | #181, #182, #185 |
| `adversarial/host/` | grants, host capabilities, redaction and limits; `grant-ops` is HOST-1, `sigpipe` is HOST-2; `cmp.sh` runs both sides and diffs output, exit code and trace | #183, #184, #185 |
| `adversarial/tasks/`, `adversarial/equiv/tog*` | native `together` (PR #175); `equiv/togcancel` and `tasks/recheck_nested_cancel` show #200 | #162, #200 |

The `recheck_*` cases are the second round: new variants of each fixed finding's class. `host/recheck_pipe_then_fail` shows #198.
