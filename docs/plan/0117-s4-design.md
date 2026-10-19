# ADR 0117 slice S4: `tasks` and `start` (design note)

- Issue: #969 (part of #896, ADR 0117 rules 7, 10, 11 and 12, Implementation plan step 4)
- Base: slice S3 (#968, `docs/plan/0117-s3-design.md`), on S2 (#967) and S1 (#966)
- Normative text: `spec/27 §7` and `§8`; decisions in the ADR 0117 amendment of 2026-10-19 on slice S4

This note fixes how S4 is built, and lists the decisions it takes where the ADRs leave a choice. Each decision takes the most secure option the ADRs allow.

## 1. What S4 delivers

1. `tasks`, with and without `at most n at once`, and `start`, in edition `0.4` (rule 7).
2. The checks: U2035 for a `start` outside the block of a `tasks`, inside a task within it, or with a `with` clause; U3029 for a `start` that can fail; U2011 for a `start` of a value other than `Unit`; U2023 for captures; U5001 for a bound below 1; `diverges` on a started call (ADR 0116 rules 8 and 9).
3. The Core forms `tasks(b) e` and `start e` (`spec/09`), in the verifier, the interpreter and native code.
4. The paths `p.s<c>.<k>`, the trace written as it happens (rule 10), `--trace <file>` on `una run`, and the `aborted` outcome of a started task that ends alone.
5. The interpreter's `Machine` split into the state of a task and the state of the run, with started tasks on `una-sys` coroutines under a deterministic first-in, first-out scheduler (rule 11).
6. Check points counted once, the same way in both engines, the round-robin of 1024 check points between started tasks, and `--virtual-time` equivalence (rule 12), with executables built with check elision refused under it.
7. The messages of `async` and `await` in edition `0.4`.

## 2. Syntax

- **Decision S4-D1: `start` is a statement wherever a statement may begin, in edition `0.4`, when an expression follows it and that expression does not begin with `(`.** Rule 7 makes `start` a keyword "at the start of a statement inside the block of a `tasks`". A `start` outside that block must still parse, so that the checker reports U2035 there, as rule 7 and the Conformance section ask. A line that begins with `start` followed by a name, a capability reference, a literal or `attempt` is no expression of any edition, so reading it as a `start` changes the meaning of no valid program: `start(x)`, `start = 1`, `start.method()`, `start + 1` and a final `start` stay a name.
- **`tasks`** is a statement in edition `0.4` when, at the start of a statement, a newline and an indent follow it, or the word `at` does; elsewhere it is a name, so a block may still end in `tasks`.
- `start` takes a `handled` (`spec/02 §6`), with an optional `within` (S2) and handling clauses. A `with` clause parses, so that the checker reports U2035 at it with its own message.
- Before edition `0.4`, both lines are U0101 (a name followed by an expression, or by `at`).
- A block whose last line is a `tasks` or a `start` has the value `unit` (rule 7, `spec/17 §1`).

## 3. The checker

- **Where `start` may stand.** The checker keeps the frame of the innermost `tasks` whose block encloses the statement: its task depth and lambda depth. A `start` with no frame, or under a deeper task (a branch of `together` or `first of`, a body of `together for`) or a lambda, is U2035 at `start`. A lambda is a boundary too (decision S4-D2): it is a value that may be called after the `tasks` statement has ended, so a `start` in it could outlive its scope, which rule 8 of `spec/19 §6` forbids.
- **The started expression** is checked as a branch of `together` (`spec/19 §2`): under the program counter of the statement (IF1), with the task mark of U2023, and as a task for U3027 and U2034, except that a started call of a `diverges` function is allowed where the enclosing function says `diverges` (ADR 0116 rules 8 and 9).
- **U3029.** The started expression MUST NOT fail after its handling clauses. The checker reuses the machinery of `on stop` (ADR 0116 rule 10): an unhandled failure, `if error pass`, `if error fail` and `if none fail` are U3029, with a message that names `start`. U3029's meaning in the registry, "A failure that must be handled where it is not", already covers it, so the registry keeps its words and spec/13 and the explanation list the case (decision S4-D3).
- **The bound** is an `Int` (U2001), evaluated once in the statement's task, with a `refinement` obligation `>= 1` (U5001), as a deadline has.
- **IF1.** A started task runs under the program counter of its `start`, as a `together` branch does. A `tasks` statement adds nothing to the timing label: waits are out of model (IF6). The statement's block is checked as a block.

## 4. Core and the verifier

```text
expr e ::= ...
         | let (x₁, ..., xₙ) = tasks(b) e₁ in e     -- b an atom, or none for the default bound
         | start e                                  -- of type Unit, inside the e₁ of a tasks
```

- `Expr::Tasks { bound, body, span }`: `body` is the block, elaborated as the body of an `if` without `else` that updates the outer bindings it assigns: it returns the tuple of their new values, which the statement rebinds after it (`spec/17 §4`, joins). `stop` and `skip` of an enclosing loop inside it are `exit`s that cross it, so the statement ends, after its tasks, with the exit.
- `Expr::Start { task, span }`: `task` is the started expression with its deadline and handling clauses, closed over the bindings before it, of type `Unit`, that cannot fail.
- **The verifier** proves the bound `>= 1` at the bound, walks the block as a block, and walks each started expression alone in the context of the `start`, as a `together` branch; no fact about an outer binding changes across a `start`, and the `tasks` statement adds none (rule 7, Verification).

## 5. The schedule both engines share (rules 11 and 12)

The interpreter's schedule is the reference, and native code follows it under `--virtual-time`:

1. **Streaming tasks.** The root task and every started task are *streaming* tasks: their events are written as they happen (rule 10). The tasks of `together`, `together for` and `first of` are not: their events are buffered until their join.
2. **The run queue.** Streaming tasks wait in one first-in, first-out queue. The task that runs keeps running until it parks (a `start` at its bound, the end of a `tasks` statement waiting for its tasks), ends, or has passed 1024 check points since its turn began while another task is ready; then the task at the front runs. A task that passed its 1024 check points goes to the back.
3. **A started task** joins the back of the queue when it is started; the task that started it goes on. A parked task that becomes ready (a task it waits for ended) joins the back.
4. **`together`, `together for` and `first of`** run inside the turn of the streaming task that runs them, in index order, each to its end; their check points count toward that turn. Natively, under virtual time, their tasks are pushed to the front of the queue, and their joins wake their parent at the front, so they run as if inline.
5. **Check points** are counted at the same places in both engines: each call, each operation, each loop iteration and each element of a prelude function over a list. The interpreter counts them in its check (`within_budget`) and at each operation; native code at `una_rt_enter`, `una_rt_budget` and `una_rt_poll`.
6. **Host operations** never park under virtual time: natively they run on the worker, as the interpreter runs them, so a helper's timing does not reorder tasks (decision S4-D4).
7. Under the real clock the interpreter keeps this schedule; native code runs streaming tasks on `--workers` workers, and a turn ends at a check point after 10 ms while another task is ready on the worker (rule 2, Fairness): one slicer thread per run, started with the first started task, raises the attention word of such a worker every 10 ms, so the fast path of a check point is unchanged. Under virtual time a worker's check points all take the attention path, where they are counted.

## 6. The interpreter

### 6.1 The split `Machine`

`Machine` keeps the state of one task: its depth, `max_depth`, stack base and budget, frames, path, race counts, `in_ever`, deadline, `within`s, shields, `later` and `callers`, its trace buffer and the counts of its `tasks` statements. The state of the run moves to `Run`, shared through `Rc`: the index, the world, the grants and keys, the clock, the host stop, the iteration count, the attention word, the memory budget and probe, the installation counts, the caches of `reads` and `binders`, the trace sink, and the scheduler's queue. Every field of `Run` that changes is a `Cell` or a `RefCell`, and each borrow ends before the code that holds it can reach a check point or a park, so no borrow is held across a switch: a borrow of the world ends before a host operation's result is used; a borrow of the trace ends before any evaluation.

### 6.2 Coroutines

- **Decision S4-D5: the root runs as a coroutine in a run whose program has a `tasks` statement**, so that it takes turns with the started tasks in the one queue, as rule 11 and the reference schedule need; a program without one runs as today, on the thread's own stack, with no switch and no scope (rule 11, Cost). The driver of the run is a loop on the thread's stack inside one `una_sys::coroutine::scope`, which resumes the task at the front of the queue until the root has ended.
- **Borrowed bodies (`una-sys`).** A task's body borrows the program, the world and the run (`'w`). `coroutine::scope` gains a lifetime: `scope<'env, R>(f: impl for<'s> FnOnce(&'s Scope<'env>) -> R) -> R`, and `Scope::spawn` takes `impl FnOnce() + 'env`, as `std::thread::scope` takes `'env` closures. It stays sound for the reason that makes the scope sound for `'static` bodies: the scope never returns, nor unwinds, while one of its coroutines is unfinished, a leaked one included, so everything a body borrows outlives every frame of its stack. `Scope<'env>` is invariant in `'env`, so a body cannot borrow a local of the closure that runs the scope. The native runtime keeps `'static` bodies.
- **Stacks.** A started task's stack is `stack_bytes(task_depth)`, as native code sizes it, and its depth limit `--task-depth` calls below its `start`, within `--max-depth` (S3 amendment, point 3). The root's stack, when it is a coroutine, is the `--max-depth` stack it has on its thread today. Stacks come from a pool of at most 64, each discarded before reuse.
- **Parking.** A task that parks records what it waits for, and suspends; the driver resumes it when the scheduler puts it back in the queue. Nothing of `Run` is borrowed at a switch. A task suspends only while the driver serves the run (a flag of `Run`), never in a coroutine of another owner; the driver clears the queue and the bodies not yet started when it begins and when it ends.
- **Failing closed** (review of S4, S4U-1 and S4U-2). A panic of the interpreter in a task is caught on the task's own stack, kept by the run, which stops the other tasks as after an abort, and raised again on the thread's stack once the driver has ended, so the command reports U8001 with exit code 3 as before. A task left parked with nothing to wake it, a defect, makes the driver panic with a message naming the condition before its scope aborts the process. A root whose stack cannot be mapped stops the run with U6003; it never runs inline, where a `tasks` statement could not wait.

### 6.3 Host operations

Decision S4-D4: the interpreter's host operations stay on its thread, as before. Rule 11's helper thread for a waiting operation changes only when a parked task's siblings run under the real clock, not the outcome, the order of a task's own events or the trace of a run under virtual time; it lands with the reactor (step 6), when the interpreter's `World` can hand an operation to a helper.

## 7. Native code

- **The root (decision S4-D5).** An executable whose program has a `tasks` statement (the embedded `tasks` flag) runs `main` as a coroutine on worker 0, with the stack `--max-depth` needs, so it takes turns with its started tasks on the workers (S1 decision D3 ends here). One without keeps its `una-main` thread, as today.
- **`una_rt_tasks_begin(bound, site)`** opens a statement in the task's state: its counts, its bound (the default of rule 7 when the bound is 0), its path prefix `p.s<c>`, and a cancellation entry for its started tasks. **`una_rt_start(closure, site)`** exports the closure as a `together` task's is exported (ADR 0113), parks while as many started tasks run as the bound allows, and starts the task on the next worker. **`una_rt_tasks_end(failed)`** cancels the started tasks when the block failed or was cancelled, parks until every one has ended, and closes the statement. Each way out of the block, its value, its failure, an exit of an enclosing loop and a stop, passes through `una_rt_tasks_end`.
- **A started task** keeps its own heap and its own state; its events are written as they happen. It ends alone after U6003 for depth or stack, in itself or in a task of a block it runs (the abort's message says so), with its `aborted` event and one line on stderr, `[U6003] Error: the started task <path> ended alone: <message> at <file>:<line>:<column>`; an abort of the run's memory budget, a header value, host text, the hard stop or a defect ends the run as before. Its cancellation entry carries the mark that lets a cancellation interrupt a waiting `Http` request (rule 4).
- **The Linux cap.** On Linux the runtime reads `vm.max_map_count` once, and a run holds at most a quarter of it in started tasks at once. The default bound is `max(64, M)` capped by it; a `start` that would pass the cap with an explicit bound above it aborts the run with U6003 at the `start`, with a message that names the cap (decision S4-D6), never a failed mapping.
- **Releasing.** A cancelled started task leaves its frames by the failure path of S2 (`STOPPED`), and its heap is torn down when its coroutine ends, so cancelled tasks hold no memory after the statement.

## 8. Traces (rule 10)

- **The sink.** A run keeps one sink: the trace file of `--trace`, made before `main` starts (decision S4-D9: a file that cannot be made is a usage error then), written as a JSON array whose `[` comes with the first event, each event as it is written, and `]` at the end, so a finished run's file is the document of before; or, for `una run --json`, the document's list in memory. `seq` is the sink's count.
- **Writing.** A streaming task keeps its events in its buffer until the event is complete and no operation of the task is in progress, then moves them to the sink, numbering `seq` there: an operation's event is written when it ends, after the events of the handler code that served it. Events of `together` and `first of` tasks are adopted at their join, as before, into the task that runs the block, which writes them when it is streaming.
- **Installations.** A streaming task numbers its handler installations with run-wide counts, `with:<handler>#<n>`, in the order they happen; the tasks of `together` and `first of` keep their local ids until their join renumbers them, as before. The interpreter does the same, so its ids match native code's under the reference schedule.
- **Started tasks' events** carry their paths `p.s<c>.<k>`; a cancelled one ends with `cancelled`, and one that ran too deep with `aborted`.

## 9. Check elision under virtual time (rule 12)

The embedded description gains `elision`, true for an executable built with check elision (ADR 0112). No backend of this repository elides checks yet, so every executable embeds `false`; one that embeds `true` refuses `--virtual-time` with a usage error, exit code 2, before `main` starts (decision S4-D7).

## 10. Further decisions

- **S4-D8: the interpreter's stack is not charged** to `--max-memory` (decision D5 of slice S1 stands): its frames per call are far larger than native code's.
- **S4-D10: the out-of-model entry** "waits at limits and at `start`" lands before "which branch of `first of` wins, beyond its label", in the order of rule 13.
- **S4-D11: the per-worker timing wheels** of the amendment on S2 stay future work: the slicer needs none, and the one timer thread of S2 keeps serving the deadlines.

## 11. Costs

| Operation | Time | Space |
| --- | --- | --- |
| `start` | O(1), plus the copies of its captures (ADR 0113) and the pool's O(log k) | a stack, used as it grows, and a heap |
| `start` at its bound | O(1) to park and to wake | one queue entry |
| the end of `tasks` | O(1) per task that ended, and one wake | none |
| a switch | O(1): a register swap and the task's slots | none |
| a trace event of a streaming task | O(1) amortized, written at once | none held after it is written |

A program without `tasks` keeps its cost: no coroutine, no scope, no queue, and a trace written at the end as before when it has no streaming sink.

## 12. Tests

- Conformance (`tests/conformance`): `syntax/tasks_forms`, `syntax/tasks_before_0_4`, `syntax/tasks_names`, `syntax/async_0_4`, `types/start_place`, `types/start_with`, `types/start_fails`, `types/start_unit`, `types/start_capture`, `verification/tasks_bound`, `runtime/tasks_runs` (both engines), `runtime/tasks_virtual` (`una test`: waiting, the bound, a failing block, `stop`, the depth).
- Unit: `una-sys` (a scope over a borrowed body), the interpreter's scheduler (first-in, first-out order, a switch every 1024 check points, a parked `start`), `una-rt` (the bound, the cap, the end of a statement).
- Native (`crates/una-cli/tests/native.rs`): equivalence under `--virtual-time` with one worker; a started task past `--task-depth` ending alone; a host stop with `on stop` and drained tasks; ten thousand cancelled started tasks under a small budget in both engines; and the scale tests of 1,000, 10,000 and 15,000 started tasks parked on requests, linear in time and memory, and 20,000 on Linux reporting the cap (`crates/una-cli/tests/http.rs`), the two largest `#[ignore]`d for the gates.
