# 3d: recheck of #200 (fix PR #213)

The candidate is ef34f2f. In each probe, task 1 fails after some work, while task 2 performs operations of a Una handler whose body is long. The native executable ran each probe 20 times with `--trace`. `tools-200/invariants.py` checks each trace:

- no event is `aborted`;
- each `cancelled` event names no operation;
- each `cancelled` event is the last event of its task path.

| Probe | Handler operation body | Result, 20 native runs |
| --- | --- | --- |
| opmulti | several calls and a recursion | rc 0, `failed`, 0 violations |
| opnested | performs an operation of a second handler | rc 0, `failed`, 0 violations |
| ophost | prints (a host operation) | rc 0, `failed`, 0 violations |
| opfails | fails on even inputs | rc 0, `failed`, 0 violations; the `fail` outcomes are kept |
| opinnertog | a nested `together` | rc 0, `failed`, 0 violations |
| opinnercancel | a nested `together` whose own task 1 fails | rc 0, `failed`; the inner task 2.2 is cancelled inside the shielded operation, as it should be |
| optogfor | a `together for` of 64 tasks | rc 0, `failed`, 0 violations |
| bigtogfor | none: 300 tasks, each in an operation | rc 0, `failed`, 0 violations |

The status and the output equal the interpreter's in every run.

In opinnercancel, the path `2.2` repeats across steps, once per inner block. So in 3 of the 20 runs, the checker flags a `cancelled` event of `2.2` that is followed by later events of `2.2`. These are later blocks, not a violation.

**Ordered runs.** In each probe's ordered variant, task 1 does not fail (`if w < 0`). Five native traces of each variant are byte-equal to the interpreter's `--json run` trace, and the outputs match.

**Repros.** `equiv/togcancel` and `tasks/recheck_nested_cancel` ran natively 20 times each: 0 `aborted` events, and the status and the output equal the interpreter's.

**Tests.** The two tests that #213 added pass at the candidate.
