# Review of the native backend and runtime (issue #176)

Status: in progress. The findings so far are filed as issues #180 to #185; `findings.md`, `summary.md` and the verdict follow once the fixes are re-checked.

Each case under `adversarial/` is a package that passes `una check` at tier 1 in canonical form. To reproduce one, copy it outside the repository, then compare the interpreter with the native executable:

```sh
una run .
una build . && ./target/una/<name>
```

| Area | Cases | Issues |
| --- | --- | --- |
| `adversarial/integers/` | fixed-width semantics and machine widths (FW-1); `reflect-predicate-fixed-overflow` is INT-1 | #180, #185 |
| `adversarial/memory/` | the unsafe boundary, reference counts and in-place updates; `stale_alias_reuse*` is MEM-1, `loop_bound_is_state` is MEM-2 | #181, #182, #185 |
| `adversarial/host/` | grants, host capabilities, redaction and limits; `grant-ops` is HOST-1, `sigpipe` is HOST-2; `cmp.sh` runs both sides and diffs output, exit code and trace | #183, #184, #185 |
