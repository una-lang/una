# ADR 0117 slice S3: `first of` (design note)

- Issue: #968 (part of #896, ADR 0117 rules 4, 6, 10, 12 and 13, IF3, Implementation plan step 3)
- Base: slice S2 (#967, `docs/plan/0117-s2-design.md`), the cut by the failure path (`STOPPED`) and the deadlines of `within`
- Normative text: `spec/27 §6`; decisions in the ADR 0117 amendment of 2026-10-19 on slice S3

## 1. What S3 delivers

1. `first of` with two or more branches, in edition `0.4`, as the value of a `let` or an assignment, or as the last line of a block (rule 6); U2035 for a body with fewer than two branches or a line that is not an expression.
2. Branches as tasks: U2023 for captures, U2034 for `stop` and `skip`, U3027 for running forever, U2001 for branches of different types.
3. IF3: the value labeled with `t`, the join of `τ` of every branch, and every branch checked under the timing floor `pc ⊔ t`; each `first of` above `public` in the report's `timing` list, and the `out_of_model` entry of rule 13.
4. The Core form `first_of(e₁ | ... | eₙ)`, in the verifier, the interpreter and native code.
5. The paths `p.f<c>.<i>`, the trace with the winner first, and the `interrupted` outcome of rule 4's first bullet.

## 2. The front end

- **Syntax.** `BlockExpr::FirstOf` holds the body as parsed; the parser opens it at `first`, `of` and the end of the line, in edition `0.4` only.
- **Checker** (`types/stmt.rs`, `first_of`). Each branch is checked as a `handled` statement, with the task mark of `together` (U2023), and the type of the first branch that can return as the expected type of the others. The race is a timed site of `types/within.rs` (`race_open`, `race_close`, `Construct::FirstOf`), keyed by the span of `first of`: its seed covers every branch, so the probe of S2 gives `t` before pass 2 checks, and pass 2 raises the timing floor of every branch by it.
- **Elaboration.** Each branch becomes a task closed over the bindings before the block, its tail never a return, and the race is bound to a temporary of the block's type.
- **Verifier.** Each branch is walked alone in the context before the block; with a refined result type, each branch's values flow into it; the result's path has no value, so only the type's facts hold.

## 3. The interpreter

`Machine::first_of` runs the branches in index order and stops at the first that returns. A failure is kept when it is the first; an abort, a cut or an exit goes on at once. Each branch pushes `f<c>` and its index on the task path, a counter of races per task in progress giving `c`, and lowers `max_depth` to `--task-depth` below the block for its duration. `winner_first` then moves the winner's events before the others', renumbering `seq` and the ids of the handlers each branch installed as native code numbers them at its join: O(events + installations).

## 4. Native code

- **Codegen.** `Expr::FirstOf` makes the list of the branches' closures, as `together` does, and calls `una_rt_first_of`; status 2 goes on as `STOPPED` by the failure path, 1 is the failure, 0 the winner's value.
- **Runtime** (`tasks.rs`). A race is a block of shape `Race`: one batch of every branch, no result list, a winner slot, and a count whose entries carry `RACE_ENTRY`. A branch that returns takes the winner slot with one compare-and-swap and lowers the count to 0, which cancels the others; an abort does the same; a failure cancels nothing. The join decides by a defect, then the lowest abort, then the winner, then, when every branch failed, the lowest failure, adopts the winner's events first, and copies only the deciding value in.
- **Virtual time.** The batch forks one branch, lets it run to its end, and forks the next only when that one failed (rule 12).
- **Paths and depth.** `State::for_race` counts the races of the task and gives the template its `race` and a depth limit of `--task-depth` below the block, within `--max-depth`; `task_path` writes `p.f<c>.<i>`.
- **Releasing what losers hold.** A cancelled branch stops at its next check point and leaves its frames by the failure path of S2 (`STOPPED`), and its heap is torn down when its coroutine ends.

## 5. Interruption

- `sched::blocking_interruptible` parks the task on a mailbox that both the helper's result and an interruption can close; the first close counts, and a later result is dropped on the helper.
- `interrupt.rs` keeps the waits by the address of the counts they can be cancelled by, behind one mutex, each holding the count's `Arc`. `runtime::interruptible_entries` gives the race entries that the operations around the request do not shield. A race's report calls `interrupt::lowered` once it lowers its count, which wakes every wait registered on it. A cancellation before the registration is seen by a check right after it, so the request is never sent.
- `Http` in a task uses it: an interrupted request returns the word of a stopped task with `host_failed` 2, its event ends `interrupted`, and the stop goes on by the failure path; the join then adds the task's `cancelled` event.
- Costs: O(d log w) to register and to remove, for the d race entries of the task and the w waits; O(k log w) per lowering for the k waits on that race. Unit tests: `interrupt.rs` (wakes only what a count cancels, a dropped registration is never woken, linear scaling) and `sched.rs` (a parked task is woken while its helper still waits).

## 6. Tests

- Conformance (`tests/conformance`): `syntax/first_of_forms`, `syntax/first_of_before_0_4`, `types/first_of_shape`, `types/first_of_types`, `types/first_of_capture`, `types/first_of_failures`, `security/first_of_timing_floor`, `security/first_of_internal`, `runtime/first_of_runs` (both engines), `runtime/first_of_virtual` (`una test`: winner by index, ties, all failing, nested three levels).
- Native (`crates/una-cli/tests/native.rs`): the same winner, output and trace in both engines under `--virtual-time` with one worker and several, with branches that end at the same virtual time, and three levels of nested `together` and `first of`; ten thousand races whose losers hold memory, under a small `--max-memory`, with no U6003, in both engines; a loser parked on a request to a server that never answers is interrupted at once (`crates/una-cli/tests/http.rs`).
