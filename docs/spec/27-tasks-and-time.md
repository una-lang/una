# 27. Tasks and time

Status: Normative, edition 0.4 preview (ADR 0117).

How tasks run, wait and are bounded: lightweight tasks on a few workers, and later `within`, `first of`, `tasks` and `start`, and limits on grants. The rules of ADR 0117 join this chapter as they land, each with its conformance cases; until a rule has landed, the construct it defines is a syntax error (U0101) or its word is reserved (U0103).

**What has landed.** Sections 1 to 4, the runtime of tasks (ADR 0117 rules 1 to 3, slice S1). They add no construct and change no output, failure or trace of any edition: they say how the native backend runs the tasks of `together` and `together for` (`spec/19 §8`), and which flags bound it.

## 1. Tasks and workers

1. **Tasks are coroutines.** In native code, each task of `together` and `together for` runs on a coroutine: a stack of its own, which the runtime can suspend and resume on the same thread. A task keeps its own heap, and the copies at its fork and its join are those of `spec/19 §8` rule 2 (ADR 0113).
2. **Workers.** Coroutines run on `--workers` operating-system threads, the workers. A task runs on the worker chosen when it is forked, round-robin over the workers, and never moves to another (pinning), so its heap, reference counts and runtime state are only touched by one thread.
3. **The root task** runs `main` on a thread of its own, with the stack that `--max-depth` needs (`spec/16 §3` rule 3). Until started tasks exist, it never runs beside its own tasks: it waits at their join.
4. **No task construct, no cost.** A run that starts no task makes no worker and no helper thread, and performs every host operation on its own thread.
5. **The interpreter** runs the tasks of a block one after another, in index order, on its one thread (`spec/19 §8` rule 1). It accepts the flags of §3 and checks them as the executable does.

## 2. Waiting

1. **Joins park.** A task that waits for the tasks of its block, at their fork or at their join, parks: its worker runs other tasks until the wait ends. So a join never holds a worker that its own tasks need, and a run cannot deadlock on its workers, with `--workers 1` included.
2. **Helper pools.** A host operation of a task that may block runs on a helper thread while the task parks:
   - `Files.read` and `Files.write`, on the file pool of `--file-threads` threads;
   - a `Console.print` whose output cannot take the line without waiting, on the same pool; a line that stdout can take at once is written by the task's worker;
   - every `Http` operation, on the I/O pool of `--io-threads` threads.

   Each pool serves its operations first in, first out. The pools are separate, so requests that wait on a server never delay a `Files` operation or a print of another task. Only the operation's arguments and result cross to a helper; the task's heap, trace and state stay on its worker.
3. **Meaning.** Parking and helpers change when an operation runs, never what it does: its grants, its outcome, its event and its failure are those of `spec/14` and `spec/24`.

## 3. Flags

The executable (`spec/16 §3`), `una run` and `una test` take these flags:

| Flag | Range | Default |
| --- | --- | --- |
| `--workers <n>` | 1 to 1024 | the number of the machine's cores, at most 1024 |
| `--task-depth <n>` | 1 to `--max-depth` | 1000, or `--max-depth` when it is smaller |
| `--io-threads <n>` | 1 to 4096 | 64 |
| `--file-threads <n>` | 1 to 256 | 4 |

A value that is not a number, or is out of its range, is a usage error, exit code 2, before `main` starts. `--task-depth` is checked against `--max-depth` whatever their order on the command line. `--task-depth` bounds the tasks of `first of` and `start`, which have not landed: until they do, it is checked and has no other effect.

## 4. Stacks and memory

1. **Stacks.** A task's stack is sized from the depth it may still use, as `spec/19 §8` rule 2 sizes it, and is mapped with an inaccessible guard region below it, so running past its end faults instead of writing other memory. Stacks come from a pool and are reused.
2. **Depth.** Every call checks the stack against its budget before it proceeds, so a task stops with U6003 at the call, as `spec/11 §7` says, well before its guard region. Reaching the guard region is a defect of the runtime, which ends the process by the signal.
3. **Memory.** The stack each task has used counts toward `--max-memory`, together with the heaps of the run (`spec/11 §7`, ADR 0061 amendment of 2026-10-18). It is measured at each call's depth check as the highest point the task's stack has reached, rounded up to 16 KiB, and released when the task ends. In native code this covers `main`'s stack too.
