# ADR 0117 slice S7: the reactor (design note)

- Issue: #972 (part of #896, ADR 0117 rules 3 and 4, Security "A stalled upstream", Implementation plan step 6)
- Base: slice S4 (#969, `docs/plan/0117-s4-design.md`), on S3 (#968, interruptible waits), S2 (#967) and S1 (#966, the helper pools)
- Normative text: `spec/27 §2`; decisions in the ADR 0117 amendment of 2026-10-19 on slice S7
- Later work that builds on it: the listener of #897 (ADR 0123 rule 19)
- Review: the independent review of S7 (findings R7-1 to R7-8) and its rechecks (R7-9 to R7-14) are folded in; §10 lists where each one landed.

This note fixes how S7 is built, and lists the decisions it takes where the ADRs leave a choice. Each decision takes the most secure option the ADRs allow.

## 1. What S7 delivers

1. `una_sys::poll`: a `kqueue` (macOS) or `epoll` (Linux) instance behind a safe interface. It holds no logic: create, register, deregister, wait. And `una_sys::io::descriptor_limit`, one read of the soft limit of open descriptors.
2. `una_rt::reactor` (safe code): one reactor thread that waits on that instance and wakes the tasks parked on it, by readiness, by their deadline, by their run's host stop, by a cancellation, or by a party that hands them something.
3. `Http` in a task: the request runs on the task's own coroutine, and every wait on its socket parks the task on the reactor. No helper thread is held while a request waits for its server.
4. `una_rt::sockets` (safe code): a process-wide cap on the sockets of parked requests, derived from the limit of open descriptors, with a first-come, first-served line (decision S7-D4).
5. Name resolution and the TCP connection stay on the I/O pool (`--io-threads`), interruptibly, with their wait in the pool covered by the request's deadline, and resolver threads bounded (decision S7-D2).
6. Conformance: 64 requests to a server that never answers delay no `Files` or `Console` operation of another task, with `--io-threads 1`; 1,000 requests under `ulimit -n 256` never make a `Files` operation fail; the scale tests of S4 with every task parked on a request at once.

No construct, diagnostic code, flag or trace event is added, and no outcome changes: `spec/27 §2` rule 4 ("Meaning") holds, so the reactor changes when an operation's waits end, never what the operation does.

## 2. The `una-sys` interface

```rust
// poll.rs
pub enum Interest { Read, Write }

pub struct Poller(/* an OwnedFd: the kqueue or the epoll instance, close-on-exec */);
impl Poller {
    pub fn new() -> io::Result<Poller>;
    pub fn add(&self, fd: BorrowedFd<'_>, interest: Interest, token: u64) -> io::Result<()>;
    pub fn delete(&self, fd: BorrowedFd<'_>, interest: Interest) -> io::Result<()>;
    pub fn wait(&self, events: &mut Events, timeout: Option<Duration>) -> io::Result<usize>;
}
impl AsFd for Poller { .. }       // a poller may be nested in another one later (ADR 0123 rule 19)

pub struct Events { /* a buffer of kevent or epoll_event, zeroed once */ }
impl Events {
    pub fn with_capacity(capacity: usize) -> Events;
    pub fn tokens(&self) -> impl Iterator<Item = u64> + '_;   // of the last wait
}

// signals.rs
pub fn wake_pipe() -> Option<BorrowedFd<'static>>;  // the wake pipe's read end, once installed

// io.rs
pub fn descriptor_limit() -> Option<u64>;           // the soft RLIMIT_NOFILE
```

- **One-shot registrations.** `add` registers `fd` for one event of `interest` (`EV_ADD | EV_ONESHOT` with `EVFILT_READ` or `EVFILT_WRITE`; `EPOLL_CTL_ADD` with `EPOLLIN | EPOLLRDHUP` or `EPOLLOUT`, and `EPOLLONESHOT`). After its event, a registration fires no more, so a descriptor that stays ready never makes the reactor spin, and a stale registration delivers at most one event, which the token's generation makes the reactor ignore (§3). On `epoll` a registration that fired stays, disabled, until it is deleted, so every wait deletes its registration when it ends, and the reactor deletes its own nudge registration before it adds it again.
- **`delete`** removes a registration; one already gone (`ENOENT`, after its one-shot event on `kqueue`) is not an error.
- **Errors, end of file, hang-up** are delivered as an event of the token: the caller's next read or write sees the condition. A token is a plain integer: `una-sys` never reads it as an address, on `kqueue` too, where `udata` has a pointer type.
- **`wait`** returns at the first events, at the timeout (rounded up to whole milliseconds on `epoll`, capped at `i32::MAX` ms; `None` waits without a timeout), or with no event when a signal interrupts it (`EINTR`), so the caller loops.
- **Close-on-exec.** `epoll_create1(EPOLL_CLOEXEC)` opens the instance close-on-exec atomically. `kqueue()` has no such flag; its descriptor is set `FD_CLOEXEC` by `fcntl` right after, and a `kqueue` is never inherited by a child of `fork` anyway (kqueue(2)), so the window between the two calls exposes nothing. A poller whose flag cannot be set is closed and not used (fail closed).
- **`wake_pipe`** borrows the read end of the wake pipe for the life of the process, and only once `install` has succeeded: only a failed install closes the pipe.
- **`descriptor_limit`** reads the soft `RLIMIT_NOFILE` once with `getrlimit`. The getrlimit binding of #1005 (`server.rs`, which also raises the limit for listeners) is a second reader of the same value; when both land, one binding serves both, and this one goes.
- **No signal context.** Nothing of `poll` runs in a signal handler, and the handlers of `signals` are unchanged: they still allocate nothing.
- **What is not there.** The readiness table, wakes, timers, the stop pipe's handling, the socket cap and the reactor's loop are safe code in `una-rt`. The reactor wakes its own thread through a socket pair from `std` (`UnixStream::pair`, close-on-exec by `std`), registered like any socket, so `una-sys` needs no `eventfd` or `EVFILT_USER`.

### Size of `una-sys` (decision S7-D6)

Counted with `wc -l` on each source file:

| File | Before S7 | After S7 | Change |
| --- | --- | --- | --- |
| `attention.rs` | 115 | 115 | |
| `coroutine.rs` | 539 | 539 | |
| `io.rs` | 16 | 27 | `descriptor_limit` (+11) |
| `lib.rs` | 19 | 21 | `pub mod poll` and the documentation (+2) |
| `poll.rs` | | 174 | new |
| `signals.rs` | 315 | 323 | `wake_pipe` (+8); the private pipe opener renamed `open_pipe` |
| **Total** | **1,004** | **1,199** | **+195** |

The crate stays under its cap of 1,200 lines (ADR 0116 amendment of 2026-10-19), with one line of margin: no amendment of the cap is proposed. Any later change to `una-sys`, such as the non-blocking `connect` left open in §5, or a second reader of the descriptor limit, needs the cap decided first.

## 3. The reactor (`una-rt`, safe code)

- **One reactor thread per process** (decision S7-D1), `una-reactor`, started at the first wait that needs it, as the timer thread of S2 is. It owns one `Poller` and a socket pair that wakes it.
- **The table.** A slab of waits behind one mutex: each wait holds its wake (the mailbox its task parks on, `sched::channel`), its timer in a timing wheel (`wheel.rs`), and, when its run's host stop interrupts it, its run and its links in an intrusive list of such waits. A free list reuses slots. A wait's token is its slot and the slot's generation, so an event of a slot reused since is ignored.
- **A wait, on the task's coroutine** (`Reactor::wait_on(fd, timeout, stop, entries, arm)`):
  1. Under the mutex: register the stop pipe when the wait has a run and the pipe is not watched yet; take a slot; arm the timer at `now + timeout` rounded up to the next millisecond, plus one; list the wait when it has a run. When the timer is due before the time the reactor sleeps to, or a listed wait needs the reactor's look sooner, wake the reactor (one byte on the socket pair; a full socket already holds one).
  2. When the wait has a descriptor: `Poller::add(fd, interest, token)`. A failure removes the wait and returns `Failed`, which the client reports as `Unreachable`.
  3. Register the cancellation entries with `interrupt::register`, whose wake closes the same mailbox with `Cancelled`, then check them once (a cancellation that came before is seen there).
  4. `arm(ready)`: hand a wake that closes the mailbox with `Ready` to the party the wait is for (the socket cap, a connection job); when what the wait is for has come already, it returns true and the task does not park.
  5. Park on the mailbox. The first close counts: `Ready`, `TimedOut`, `Stopped` or `Cancelled`.
  6. Remove the wait: drop the interrupt registration, `Poller::delete`, cancel the timer, unlist it, free the slot.
- **The reactor's loop.** Wait on the poller until the wheel's next expiration; for each token, take the wait's wake out of its slot, when the generation matches, and close it with `Ready`; then look at the listed waits when the pipe tells of a stop or no longer watches; then advance the wheel and close the waits whose timers fired with `TimedOut`. Wakes are sent outside the mutex. The reactor runs no Una code and touches no task's state: a close pushes a resume job to the task's worker (`sched`).
- **Host stops.** The pipe of `signals` is registered once, at the first wait that a host stop can interrupt. Its event makes the reactor look at the listed waits and wake those whose run has stopped (`RunStop::stopped`, atomic loads), and no other. Every request of a run's stop writes the pipe: a signal, the deadline's, and the run's own request at `--stop-after-iterations` (`RunStop::request_local` wakes the pipe since review R7-11). The pipe stays readable for good after its first write, so its registration is then deleted, as one-shot already disabled it, and from then on the reactor looks at the listed waits every 100 ms, the slice of `una_interp::stopping`, on its own thread, keeping the time of its last look (review R7-12): no wait is woken, and none registers again, unless its run has stopped. A run whose stop comes is therefore seen at once while the pipe watches, and within 100 ms after another run of the process has woken it, as `OpInterrupt` sees it.
- **Costs** (decision S7-D5). Register, wake and deregister are O(1): a slab slot, a wheel timer, a list link and one system call each; the reactor's wake of one event is one slab lookup. A firing timer is O(1) amortized (`wheel.rs`). A look at the listed waits is O(k) for their k, on the reactor's thread, at a pipe event and, once the pipe has fired, every 100 ms. The interrupt registration of S3 keeps its O(d log w), for the d race entries of the task, which is 0 outside a race or a started task. Scaling tests register, wake and remove 1,000, 10,000 and 100,000 waits and require linear time.
- **Failing closed** (review R7-4). A reactor that cannot start (no poller, no socket pair, no thread) leaves `Http` on the I/O pool of S1, which is correct and only holds helpers; its start is tried again at most once a second, so a transient shortage of descriptors or threads does not turn the reactor off for the life of the process.
- **The listener of #897.** The `Poller` is per instance, not a global: ADR 0123 rule 19 asks for one `kqueue` or `epoll` per worker, with `EPOLLEXCLUSIVE` and a nested `kqueue` with `EV_DISPATCH` for listening sockets. Those are new interests of `Poller::add` and a poller per worker in `una-rt`, added with #897; `AsFd` lets a poller be registered in another. The reactor's table, wheel and wakes do not depend on there being one reactor.

## 4. `Http` on the reactor

- **Where.** A task on a worker under the real clock: the same condition that sent `Http` to the I/O pool in S1 (`sched::in_task()`, not `--virtual-time`). The root on its own thread, a program without tasks, and every run under `--virtual-time` run the request inline, as before (S1, S4-D4).
- **The request runs on the task's coroutine** (decision S7-D3). `una_http::client::send_interruptible` takes an `Interrupt` whose `waits()` is true, so the socket is non-blocking and every read, write and step of the TLS handshake that would block calls `Interrupt::wait`, which parks the task on the reactor. The client is unchanged in what it sends and reads. Its CPU work, parsing and TLS, runs on the worker between waits, bounded per step by the response limits and one TLS record; like `sort`, it reaches no check point inside a step, so it joins the built-ins of the fairness bound (rule 2).
- **The `Interrupt` of a parked request** (`reactor::Parked`):
  - `stopped()`: the run's host stop when the request is in the body of a `for ever` (ADR 0116 rule 12), or a cancellation of its task (rule 4, first bullet), checked on the task's interruptible entries before each try;
  - `wait(fd, write, timeout)`: a reactor wait on the socket, listed with the run when the request is in a `for ever`; `Cancelled` is recorded, so the client returns at once;
  - `open(host, port, deadline)`: §5, after the request's place among the sockets (§7).
- **After the client returns**, `host::http` reads whether a cancellation interrupted the request: the operation then ends `interrupted` and the task stops (S3, S4 amendment point 9), as with the pool. A request that completed before it saw the cancellation completed (`spec/19 §6` rule 4). Otherwise its result is the client's, a host stop's `Interrupted` included.
- **What a cut releases** (rule 4: "With the reactor, an interrupted request is closed at once"). A cancellation closes the mailbox, the task resumes, the client returns `Interrupted` and drops its connection, which closes the socket, before the task goes on to its `cancelled` event; the request's place among the sockets is given back with it. A deadline does the same with `TimedOut`. No thread is left behind with the request.

## 5. Name resolution and connection (decision S7-D2, as decided after the review)

`getaddrinfo` has no non-blocking form, and a non-blocking `connect` needs `socket` and `connect` calls that `std` does not offer, which would add `unsafe` code to `una-sys` beyond its cap.

- **Resolution and connection run as one job on the I/O pool** (`--io-threads`). The task parks on the reactor until the job hands back its connection (`Ready`), until the request's deadline passes, which covers the job's wait in the pool's queue, or until a cancellation. The job is `una_http::client::open`: the resolver on a thread of its own bounded by the deadline, then `connect_timeout` over the addresses, within the request's deadline. The connected stream comes back to the task, which makes it non-blocking and goes on, on the reactor.
- **A job whose task gave up ends at once** (review R7-3). The job carries clones of the task's cancellation entries and reads them, with the run's stop and the deadline, before it resolves, and before each address. So a job of a cancelled or expired request that a helper takes from the queue costs O(1) and opens nothing.
- **Name resolution** (`una-http`, `resolve.rs`, review R7-10):
  - **One lookup per name at a time.** Callers that ask for a name while its lookup is in flight wait for that lookup's answer, each until its own deadline. A lookup whose callers all gave up keeps its place until the system answers it, but later callers of the name join it rather than start another. So a name whose lookups stall holds one resolver place, whatever the rate of its requests: the bound per name is 1, below a quarter of any bound of the process.
  - **Bounded threads.** Resolver threads are bounded process-wide by `--io-threads` (`una_http::client::limit_resolvers`, set when the run's scheduler is made; 64 in the interpreter), with a first-come, first-served line for a place whose wait the deadline also covers. No thread starts past a deadline.
  - **At most a quarter of the threads per zone** (the name without its first label: `a.example.net` is in `example.net`), and at least 1 (review R7-13). A zone whose many distinct names never resolve, such as a wildcard zone, holds at most that share, and leaves the other places to other zones; a caller whose zone has its share waits out of the line, so it holds no one behind it, and joins the line's end again once the zone has room.
  - **A lookup that panics** ends as a failure through a guard in its thread (review R7-14): its place is given back and its callers get `Unreachable`, and a thread that cannot start ends its lookup the same way.
  - **A failed name is remembered** for 5 seconds: it answers `Unreachable` at once, without a new lookup. The time is the monotonic clock of the process, since a run under virtual time has no real network (ADR 0116 rule 19).
  - **Costs.** O(1) per call on average: hash maps by name (never iterated into anything a program sees), a line whose callers that gave up are skipped lazily, and a cache of failures pruned when it has doubled. A place given back wakes the callers waiting for one, at most the threads that resolve.
- **A literal address** (`127.0.0.1`, `[::1]`) is not resolved: no resolver thread starts for it.
- **A connection made for a task that left** is handed back together with the request's place among the sockets, so it is closed before its place is given back.
- **What a helper can still hold.** A peer that drops connection attempts holds a helper for at most the time left of the request's deadline; a peer that accepts and never answers, the stalled upstream of the ADR's Security section, holds none. The I/O pool now serves only these two steps, so its default of 64 helpers bounds connections being made, not requests in flight. The pool's queue holds the jobs of requests whose deadlines have not all passed; a job whose request is gone costs O(1) when it is taken.
- **Not taken:** a non-blocking `connect` in `una-sys` (about 40 lines), which would free the helper on a dropped attempt too. It does not fit under the cap (§2), and the steps above bound what it would save.

## 6. The interpreter

Unchanged: decision S4-D4 stands. Its `Http` stays on its one thread, synchronous, with `OpInterrupt` in a `for ever`, so its outputs, failures and traces do not change. The trait `Interrupt` gains `open`, with a default that is today's resolution and connection, so `OpInterrupt` keeps its behavior, and its `wait` takes a `BorrowedFd` instead of a raw descriptor. Its name resolution is that of §5: one lookup per name at a time, at most 64 resolver threads, none started past a request's deadline, and a failed name remembered for 5 seconds. `RunStop::request_local`, the run's own stop request, now also writes the wake pipe, as the deadline's request does; in the interpreter, which runs one task at a time, that changes no outcome. Under the real clock, a started task's request still holds the interpreter's thread while it waits; that is liveness only, as S4-D4 records.

## 7. Descriptors (decision S7-D4, as decided after the review)

Each request parked on the reactor holds one socket, where the pool held at most `--io-threads` at once. The runtime does not raise the process's limit of open descriptors (`RLIMIT_NOFILE`), which stays the operator's (`ulimit -n`). Instead, so that a server that never answers cannot take every descriptor and make the `Files` operations of other tasks fail (review R7-2):

- **A process-wide cap on request sockets** (`una_rt::sockets`): the soft limit read once at the first parked request, minus a reserve of a quarter of it and at least 64 for `Files`, `Console`, the runtime and its pipes, and at least 1 place: 192 under the macOS default of 256, 768 under the usual Linux default of 1,024. A limit that cannot be read counts as 256, and one above 2^20 as 2^20.
- **A place per request,** taken before its first connection and kept until the request ends, its redirects included, so a request holds at most one place and one socket at a time.
- **First come, first served.** A request that finds no place free waits in a line, parked on the reactor, an interruptible wait: a cancellation ends it, and the request's deadline (`--http-timeout`, or a `within` that covers it) covers it, so the request then fails with `TimedOut`. A host stop takes effect when the connection step ends, as for resolution (ADR 0116, implementation plan, item 6).
- **Costs.** Taking and giving back a place are O(1) under one mutex. A waiter that leaves is marked with one compare-and-swap, not removed, and skipped when a place is given; the line is compacted once more than half of it has left, so each waiter costs O(1) amortized. A place given to a waiter is marked on the waiter's state, so a waiter that wakes for another reason still finds it, and gives it back when it does not use it: no place is lost.
- **No stranded place** (review R7-9). A waiter that leaves takes the gate's lock before it marks itself gone and leaves the count of waiters, the same lock under which a place is handed over. So a place given back meanwhile goes to the next waiter or to the free places, and free places and waiters never coexist: a newcomer never queues behind a line that holds no one while a place is free. A unit test drives the interleaving, the give-back on another thread while the waiter's leaving holds the lock, and checks that the place is free and taken at once.
- A connection that cannot be opened for another reason still fails with `Unreachable`, as before.

## 8. Tests

- `una-sys` (`tests/poll.rs`): a socket pair, readable and writable events with their tokens; one-shot (a second wait sees no event while the socket stays ready); `delete` of a registration that fired; a peer that closes; the poller's descriptor is close-on-exec; add and delete at 1,000 and 10,000 in linear time.
- `una-rt`:
  - `reactor.rs`: a stale token ignored; a host stop that wakes the waits of its run only; register, wake and remove at 1,000, 10,000 and 100,000 waits in linear time; a parked task woken by readiness, by its time and by a cancellation; a wait of a run that stops woken with no pipe event; a wait without a descriptor woken by its party; a parked task leaving its worker to the others.
  - `reactor.rs`, with the handlers installed, in a process of its own (the test runs its scenario through `current_exe`, so the handlers of the shared test process stay untouched): the pipe is watched, and a run's own stop wakes a wait listed with its run at once, through the pipe.
  - `sockets.rs`: the cap for several limits; places given first come, first served and never lost; a place given back while a waiter leaves is never stranded (the interleaving driven by a test hook); waiters that leave in O(1) each, at 10,000 and 100,000.
- `una-http` (`resolve.rs`): concurrent callers of a name share one lookup; a stalled name holds one place and another name still resolves beside it under a bound of 2; sixteen distinct names of a stalled zone hold one place between them while a healthy host resolves; a lookup that panics gives its place back and answers its callers; a failed name is remembered without a new lookup; the line skips callers that left.
- `una-cli` (`tests/http.rs`):
  - 64 requests to a server that never answers, with `--io-threads 1`, beside a task that writes and reads a file and prints: the print comes long before the requests' `--http-timeout`, and the 64 requests time out together, in about one `--http-timeout`, not 64 of them (a stalled upstream holds no thread);
  - 1,000 requests to a server that never answers under `ulimit -n 256`, beside a task that writes, reads and writes again a file: the file's copy holds its text, so no `Files` operation failed;
  - the S4 scale test with 1,000 and 4,000 started tasks all parked on a server that never answers at once, time and memory linear; 10,000 and 15,000 for the release gates (`#[ignore]`). The silent server of these runs is a process of its own with a raised descriptor limit, and each run raises its own limit so that every task holds its socket under the cap.
- The existing tests of S2 to S4 over `Http` (`within` cutting a request, a loser interrupted at once, the `timeout` event of tasks) run unchanged on the reactor.

## 9. Decisions

- **S7-D1** one reactor thread per process, as the timer thread of S2; per-worker pollers come with #897.
- **S7-D2** resolution and connection on the I/O pool, as one job that the request's deadline and cancellations cover; the job reads its task's cancellation entries and the deadline before it starts anything; one lookup per name at a time, shared by its callers; resolver threads bounded by `--io-threads`, first come, first served, none started past a deadline; a failed name remembered for 5 seconds; literal addresses skip the resolver.
- **S7-D3** the request runs on the task's coroutine, and only its waits park.
- **S7-D4** no change of `RLIMIT_NOFILE`; a process-wide cap on request sockets, from the soft limit with a reserve, with a first-come, first-served line covered by the deadline.
- **S7-D5** O(1) register, wake and deregister, one-shot registrations, generation tokens.
- **S7-D6** `una-sys` under its 1,200-line cap, unchanged: 1,199 lines.
- **S7-D7** a reactor that cannot start leaves `Http` on the I/O pool (fail closed to S1's behavior), and is tried again at most once a second.

## 10. The review's findings

| Finding | Where it landed |
| --- | --- |
| R7-1 (`wake_pipe` defined twice) | the private pipe opener of `signals.rs` is `open_pipe` |
| R7-2 (descriptors without bound) | §7, `una_rt::sockets`, `una_sys::io::descriptor_limit`, the test under `ulimit -n 256` |
| R7-3 (orphaned connection jobs, resolver threads) | §5: the job reads its task's entries and the deadline first; the deadline covers the pool's queue; `resolve` checks the deadline first; resolver threads bounded, first come, first served |
| R7-4 (a failed start lasts) | §3, Failing closed: tried again at most once a second |
| R7-5 (slices after the pipe fired) | §3, Host stops: the pipe's registration is deleted, and the reactor looks at the listed waits' runs itself; no wait registers again unless its run stopped |
| R7-6 (close-on-exec gaps on macOS) | §2; no process API exists in a run, so nothing to change |
| R7-7 (`client::open` checks no grant) | its documentation says a caller must have checked the grants |
| R7-9 (a place stranded while a waiter leaves) | §7, No stranded place: `settle` takes the lock before its compare-and-swap; a test drives the interleaving |
| R7-10 (a stalled name fills the resolver places) | §5, Name resolution: one lookup per name at a time, so at most one place per name; failed names remembered for 5 seconds; tests in `resolve.rs` |
| R7-11 (a run's own stop does not write the pipe) | §3, Host stops: `RunStop::request_local` wakes the pipe; a test with the handlers installed, in a process of its own |
| R7-12 (the look at every pass) | §3, Host stops: the reactor keeps the time of its last look, and looks every 100 ms or at a pipe event |
| R7-13 (a stalled wildcard zone) | §5, Name resolution: at most a quarter of the resolver threads per zone; a test with sixteen names of one stalled zone |
| R7-14 (a lookup that panics) | §5, Name resolution: a guard in the lookup's thread ends it as a failure; a test with a panicking lookup |
| R7-8 (count of `unsafe` blocks) | `poll.rs` has eight `unsafe` blocks, `io.rs` two, and `signals.rs` one more, each with its `// SAFETY:` comment |
