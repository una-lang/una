# ADR 0117 slice S1: coroutines, workers and parking joins (design note)

- Issue: #966 (part of #896, ADR 0117 rules 1 to 3 and 11, Implementation plan step 1)
- Base: the crate `una-sys` of #943 (ADR 0116 rule 18, ADR 0058 amendment of 2026-10-18)
- Scope: no construct is added. The existing suite passes unchanged.

This note fixes how S1 is built, and lists the decisions it takes where the ADRs leave a choice. Each decision takes the most secure option the ADRs allow.

## 1. What S1 delivers

1. `una-sys::coroutine`: guard-paged stacks and a context switch per target, behind a safe interface.
2. A scheduler in `una-rt` (`sched.rs`, safe code): workers, pinning, parking, the stack pool, and the helper pools.
3. The tasks of `together` and `together for` in native code run as coroutines on the workers, and their joins park (ADR 0113 amendment).
4. Host operations of a task that may block run on helper threads while the task parks: `Files` and a `Console.print` whose output would block on the file pool, `Http` on the I/O pool.
5. The flags `--workers`, `--task-depth`, `--io-threads` and `--file-threads`, on the executable and on `una run`, with usage errors (exit code 2).
6. The stack a task uses counts toward `--max-memory` (ADR 0061 amendment).
7. The interpreter accepts and checks the same flags. It runs no coroutine yet (section 7).

## 2. The `una-sys` coroutine interface

```rust
pub struct Stack { /* one mapping: a guard below, the usable part above */ }
impl Stack {
    pub fn new(usable: usize) -> Result<Stack, StackError>;
    pub fn usable(&self) -> usize;   // bytes above the guard, a multiple of the page size
    pub fn top(&self) -> usize;      // the address the stack grows down from
}

pub struct Coroutine<'a> { /* owns its stack and its body */ }
impl<'a> Coroutine<'a> {
    pub fn new(stack: Stack, body: impl FnOnce() + 'a) -> Coroutine<'a>;
    pub fn resume(&mut self) -> Resumed;               // Suspended or Finished
    pub fn into_stack(self) -> Result<Stack, Coroutine<'a>>;  // Err while suspended
}

pub fn suspend() -> bool;   // false, and no switch, outside a coroutine
pub fn inside() -> bool;
```

- **Ownership.** A coroutine owns its stack and its body (a boxed `FnOnce() + 'a`). The lifetime `'a` keeps the coroutine from outliving data its body borrows. `Coroutine` is neither `Send` nor `Sync`: it is resumed only on the thread that made it, which is the pinning of ADR 0117 rule 2, and so thread-local storage read on its stack is always its worker's.
- **Suspending.** `suspend()` switches from the innermost running coroutine of the calling thread back to its resumer. The thread keeps a pointer to that coroutine, set by `resume` for exactly the time the coroutine's stack is the running one and restored afterwards, so `suspend` can be called at any depth of the body, from the runtime's park, without passing a handle down. Outside a coroutine it returns `false` and does nothing, and the caller blocks its thread instead.
- **No unwinding across a switch.** The entry function on the new stack calls the body inside `catch_unwind`. A panic that escapes the body aborts the process (`std::process::abort`): no unwinder ever walks from a coroutine's stack into its resumer's. The first frame of each stack ends the frame chain with a zero frame pointer, so a backtrace that walks frame pointers stops there; the trampoline has no unwind table entry, so a DWARF unwinder stops there too, and a panic never reaches it, being caught in the entry function. The runtime catches its own panics inside the body, as it does today on task threads, so a defect of the runtime is still U8001 at the block, not an abort.
- **Dropping.**
  - A coroutine that has finished, or never started, releases its stack (its body, if it never ran, is dropped on the caller's stack, where it was made).
  - **Decision D1: dropping a suspended coroutine aborts the process.** The security review asks that a dropped suspended coroutine release its stack without running code on it. Releasing the memory of a suspended stack without running its destructors is not sound for a safe interface: a frame on that stack may hold data that something else still points to, such as a pinned value (the drop guarantee of `Pin`) or the locals that `std::thread::scope` lends to other threads. Running its destructors would mean unwinding it, which rule 1 excludes. So the safe interface never frees a suspended stack: dropping one aborts, running no code on it, and the operating system takes the stack back with the process. `mem::forget` of a suspended coroutine leaks its stack, still mapped, which is sound. The runtime never drops a suspended coroutine: every task runs to its end, since cancellation is cooperative, and a hard stop exits the process. A drop there would be a defect, which fails closed.
- **Re-entrancy.** `resume` of a coroutine that is running (reachable only through a defect, since it takes `&mut self`) aborts.

## 3. The stack switch

**Decision D2: hand-written switches in `#[unsafe(naked)]` functions with `naked_asm!`** (stable since Rust 1.88; the toolchain is 1.98.1), one per target, about 25 instructions each:

- `aarch64-apple-darwin` and `aarch64-unknown-linux-gnu` (one body, AAPCS64): store `x19`-`x30` and `d8`-`d15` below the stack pointer (160 bytes), store `sp` through the first argument, load `sp` from the second, load the same registers, `ret`. `x18`, the platform register on macOS, is never touched.
- `x86_64-unknown-linux-gnu` (System V): push `rbp`, `rbx`, `r12`-`r15`, and the MXCSR and x87 control words, which System V makes callee-saved; store `rsp`; load the other `rsp`; pop them; `ret`.
- **A new stack** is prepared as if the switch had saved it: the saved callee-saved registers hold the address of the coroutine's shared record and of its entry function, the saved return address is a trampoline, and the saved frame pointer is zero. The trampoline moves the record's address into the first argument register and calls the entry function, which never returns: it ends with a last switch back to the resumer.
- **Why not `ucontext`.** `makecontext`/`swapcontext` are deprecated on macOS, save the signal mask with a system call at each switch, and would be more `unsafe` surface through `libc`. **Why not `corosensei`:** ADR 0117 Alternatives.
- **Shadow stacks** (x86 CET) and branch targets (aarch64 BTI) are not enabled for Rust binaries on these targets; a switch through `ret` is valid with either. If the toolchain turns them on, the switch must be revisited, which the note on the targets in `lib.rs` says.

## 4. Stacks and guard pages

- `Stack::new(usable)` maps `guard + usable` bytes with `mmap(PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON)` (plus `MAP_NORESERVE` on Linux, so the reservation is address space only), then `mprotect`s the lowest `guard` bytes to `PROT_NONE`. Sizes round up to the page size from `sysconf(_SC_PAGESIZE)` (16 KiB on Apple silicon).
- **The guard** is `max(page, 64 KiB)`. Una code never reaches it in a correct run: every call checks the stack against its budget, `STACK_RESERVE` below the end (ADR 0032, ADR 0061 rule 1). The guard catches a defect of that check, or a native frame larger than the reserve. A fault on it ends the process with SIGSEGV or SIGBUS: Rust's handler sees a fault outside the thread's own guard, restores the default action and returns, and the access faults again. No memory past the guard is ever written.
- `Drop` unmaps the whole mapping with `munmap`. On Linux, a stack is two mappings (the guard and the rest), which the cap of ADR 0117 rule 2 counts in S4.

## 5. The scheduler in `una-rt`

All of it is safe code in `crates/una-rt/src/sched.rs`. `una-rt` stays free of `unsafe` outside `ffi.rs`.

- **One scheduler per run,** made at the run's first `together`, kept in an `Arc` that the run's state and every task's template hold. A run with no task construct never makes one, and keeps today's cost exactly: no worker thread, no helper, no switch (the inline path). One scheduler per run also keeps the unit tests of `una-rt`, which run many runs in one process, apart from each other.
- **Workers.** `--workers` threads, made when the scheduler is, each with a first-in, first-out queue of jobs behind a mutex and a condition variable: start a task, or resume a parked one. A worker that has nothing to run waits on its condition variable. The workers end with the scheduler.
- **Pinning.** A task's worker is chosen when it is forked, round-robin over the workers with one atomic counter, in O(1). Its coroutine is made on that worker and never moves.
- **The root task (decision D3).** In S1, `main` keeps its own thread, `una-main`, with the stack of `--max-depth`, as today. It is not a worker, so its join blocks only its own thread and never a worker that its children need. Until `start` exists (S4), the root never runs beside its own children: it waits at their join. So the program observes the schedule of rule 2 exactly: at most `--workers` threads run Una code at once. S4 moves the root onto worker 0, when started tasks run beside it.
- **The slots of a task.** The runtime keeps a task's state in thread-locals: `runtime::STATE` and its mirror `HOT` (with the depth), `values::HEAP`, `tasks::STATUS`, `json::STOPPED`, and in a debug build the count of `ffi::THREAD_ALLOCATED` and the test counters. A worker swaps them with the task's saved slots when it resumes the task, and swaps them back when the task parks or ends, so a task's heap, reference counts and state are only touched by its own code, on its own worker. Between tasks, the worker's thread-locals are empty. A swap moves a few words for each slot: O(1).
- **Parking.** `sched::Mailbox<T>`, a one-value channel behind a mutex, replaces each channel and thread join of `tasks.rs`. A receiver that finds it empty records how to wake itself (its worker and task id, or its thread when it is not a coroutine) and parks: `una_sys::coroutine::suspend()` inside a coroutine, `std::thread::park()` outside. A sender stores the value and wakes the receiver: it pushes a resume job to the receiver's worker, or unparks its thread. A receiver checks again after each wake, so a spurious or stale wake costs one check. A resume job for a task that ended is ignored, by a generation number in the task id.
- **Joins park.** The block's task waits for its tasks' answers, reports and values through mailboxes, so with one worker the children run while the parent is parked. A batch of 64 (ADR 0113, `WAVE`) is unchanged, and so is the order of every copy of ADR 0113 rules 1 to 4: the protocol of `tasks.rs` keeps its messages and their order, only their transport changes.
- **The end of a task.** The body of a task's coroutine runs `tasks::task` inside `catch_unwind`; `task` tears its heap down before it returns. Once the coroutine has finished, the worker drops the task's slots, returns its stack to the pool, and only then sends `done`, as a thread's join returned only once its thread-locals were gone. The block's wait for `done` replaces `JoinHandle::join`, and a caught panic is the same U8001 as a thread that panicked today.
- **The stack pool.** The scheduler keeps its free stacks by usable size in a `BTreeMap<usize, Vec<Stack>>` behind a mutex, at most 64 stacks, and a start takes the smallest one at least as large as it needs, in O(log k). Sibling tasks of a block have one size (`stack_bytes(left)`), so a batch reuses the stacks of the batch before it. A stack beyond the pool's bound is unmapped. A stack that cannot be mapped is U6003 at the block, as a thread that cannot start is today. `Stack` is `Send` (plain memory), so the forking task takes it from the pool and a failure is known at the fork.
- **Fairness and time slices.** Tasks of `together` run to a park or to their end in S1. The time slice that ends a turn through the attention word needs the timers of S2, and only matters once tasks run long beside each other with fewer workers than tasks; S1 runs each together task until it parks, which ADR 0113's batches already bound.

## 6. Helper pools

- `sched::blocking(pool, job)` runs `job`, a `Send` closure over plain owned data (paths, texts, bytes, a request), on a helper thread of `pool` while the calling task parks, and returns its result. Outside a coroutine (the root, or a run without a scheduler), it runs `job` inline, as today.
- Two pools, made lazily at their first use: the file pool (`--file-threads`, default 4, for `Files.read`, `Files.write` and a blocked `Console.print`) and the I/O pool (`--io-threads`, default 64, for `Http`). Each has a first-in, first-out queue, so a pool full of stalled `Http` requests does not delay a `Files` operation or a print of another task.
- `Console.print` in a task first asks `poll` whether stdout can take a write without waiting (`una_sys::io::writable`, with a zero timeout). If it can, the line is written on the worker; otherwise it goes to the file pool. A line longer than the room the pipe reports can still wait on the worker for the rest; S1 accepts that, and the reactor of step 6 removes it.
- The trace, the state and the values never cross to a helper: the worker reads the arguments out of the task's heap, and builds the result in it after the helper returns.

## 7. The interpreter (rule 11)

The interpreter has no construct that needs a coroutine in S1: `together` and `together for` keep running their tasks in index order on the evaluator's one thread (`spec/19 §8` rule 1), and every host operation runs inline. In S1 it:
- accepts `--workers`, `--task-depth`, `--io-threads` and `--file-threads` on `una run` and `una test`, with the same ranges and usage errors as the executable;
- keeps `--task-depth` for the tasks of `first of` (S3) and `start` (S4), which will run on `una-sys` coroutines with stacks sized by `stack_bytes(task_depth)`, the split `Machine` of rule 11, and borrows of the shared run state never held across a switch.

## 8. Flags

| Flag | Range | Default |
| --- | --- | --- |
| `--workers <n>` | 1 to 1024 | the machine's cores (`available_parallelism`), at most 1024 |
| `--task-depth <n>` | 1 to `--max-depth` | 1,000, at most `--max-depth` |
| `--io-threads <n>` | 1 to 4096 | 64 |
| `--file-threads <n>` | 1 to 256 | 4 |

A value that is not a number, or is out of range, is a usage error, exit code 2, before `main` starts. A `--task-depth` above `--max-depth` is checked after every flag is read, so the order of the flags does not matter. When `--task-depth` is not given and `--max-depth` is below 1,000, the default is `--max-depth`.

## 9. Stack use in the memory budget

- Each task records the highest point its stack has reached, in 16 KiB steps, and the run keeps the sum over its live tasks in one atomic counter. The budget checks compare `held + stacks` with the ceiling where they compared `held`.
- **No new cost on the fast path.** `enter` already compares the stack position with the task's budget (ADR 0102 rule 2). That comparison now uses the task's charged high point instead: while the stack stays below it, the fast path is unchanged; past it, the same check raises it to the next 16 KiB step, up to the budget, and charges the difference with one relaxed atomic add. A task's charge is released when it ends.
- **Decision D4: every task is charged, the root included,** as the amendment of ADR 0061 says ("every task"). The stack of the root is charged from its first call. A program near its memory budget can therefore stop with U6003 a little earlier than before, by at most its stack in use rounded up to 16 KiB. The conformance cases that measure memory are rerun on the native suite to confirm that none moves.

## 10. Size of `una-sys`

The crate had 363 lines of source before S1. `coroutine.rs` (the stack, the switch for two architectures, the interface) has about 400, and the `poll` helper a few more, which keeps the source under 800. Its tests are in `crates/una-sys/tests/coroutine.rs`, as the crate's other tests are.

## 11. Tests

- `una-sys`: switching back and forth with values carried through the body's captures; a recursion that hits the guard page, run in a child process, which ends by SIGSEGV or SIGBUS; dropping a suspended coroutine, in a child process, which aborts; dropping a finished and a never-started one, which returns the stack; 10,000 coroutines suspended at once, resumed in turn; a panic in a body, in a child process, which aborts.
- `una-rt`: the existing tests of `tasks.rs`, unchanged in what they check, with one worker and with several; a mailbox woken before and after its receiver parks; nested blocks three deep with one worker; a stalled helper of the I/O pool that does not delay the file pool.
- Conformance (native suite and `una run`): nested `together` three levels deep, and a `together` whose parent and children print, both edition 0.4. The conformance format has no directive for run flags, so the cases with `--workers 1` live in `crates/una-cli/tests/native.rs`.
- AddressSanitizer stress runs on Linux (rule 2) need a nightly toolchain and the Linux container, and are listed as a heavy run in the slice's report.

## 12. Open decisions

- D1 (drop of a suspended coroutine aborts rather than freeing the stack) reads the security review's "release its stack without running code on it" in the only sound way for a safe interface. It needs confirmation in the security review.
- D3 (the root keeps its own thread until S4) departs from the letter of rule 1 ("every task is a coroutine ... the root task") with no observable difference in S1.
- D4 (the root's stack is charged too) can move U6003 by up to 16 KiB per live task for programs at their memory budget.
- D5: the interpreter does not charge its evaluator's stack in S1. Its stack per call (about 13 KiB unoptimized) is far from native code's, so a charge would move its U6003 points by much more than the native one; the charge in the interpreter comes with its coroutines (S4), or by a decision to leave it native only.
