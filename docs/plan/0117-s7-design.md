# ADR 0117 slice S7: the reactor (design note)

- Issue: #972 (part of #896, ADR 0117 rules 3 and 4, Security "A stalled upstream", Implementation plan step 6)
- Base: slice S4 (#969, `docs/plan/0117-s4-design.md`), on S3 (#968, interruptible waits), S2 (#967) and S1 (#966, the helper pools)
- Normative text: `spec/27 §2`; decisions in the ADR 0117 amendment of 2026-10-19 on slice S7
- Later work that builds on it: the listener of #897 (ADR 0123 rule 19)

This note fixes how S7 is built, and lists the decisions it takes where the ADRs leave a choice. Each decision takes the most secure option the ADRs allow.

## 1. What S7 delivers

1. `una_sys::poll`: a `kqueue` (macOS) or `epoll` (Linux) instance behind a safe interface, the only new `unsafe` code. It holds no logic: create, register, deregister, wait.
2. `una_rt::reactor` (safe code): one reactor thread that waits on that instance and wakes the tasks parked on sockets, by readiness, by their deadline, by a host stop, or by a cancellation.
3. `Http` in a task: the request runs on the task's own coroutine, and every wait on its socket parks the task on the reactor. No helper thread is held while a request waits for its server.
4. Name resolution and the TCP connection stay on the I/O pool (`--io-threads`), interruptibly (decision S7-D2).
5. Conformance: 64 requests to a server that never answers delay no `Files` or `Console` operation of another task, with `--io-threads 1`; the scale tests of S4 with every task parked on a request at once.

No construct, diagnostic code, flag or trace event is added, and no outcome changes: `spec/27 §2` rule 4 ("Meaning") holds, so the reactor changes when an operation's waits end, never what the operation does.

## 2. The `una-sys` interface

```rust
pub enum Interest { Read, Write }

pub struct Poller { /* an OwnedFd: the kqueue or the epoll instance, close-on-exec */ }
impl Poller {
    pub fn new() -> io::Result<Poller>;
    pub fn add(&self, fd: BorrowedFd<'_>, interest: Interest, token: u64) -> io::Result<()>;
    pub fn delete(&self, fd: BorrowedFd<'_>, interest: Interest) -> io::Result<()>;
    pub fn wait(&self, events: &mut Events, timeout: Option<Duration>) -> io::Result<usize>;
}
impl AsFd for Poller { .. }       // a poller may be nested in another one later (ADR 0123 rule 19)

// signals.rs
pub fn wake_pipe() -> Option<BorrowedFd<'static>>;  // the wake pipe's read end, once installed

pub struct Events { /* a buffer of kevent or epoll_event, zeroed once */ }
impl Events {
    pub fn with_capacity(capacity: usize) -> Events;
    pub fn tokens(&self) -> impl Iterator<Item = u64> + '_;   // of the last wait
}
```

- **One-shot registrations.** `add` registers `fd` for one event of `interest` (`EV_ADD | EV_ONESHOT` with `EVFILT_READ` or `EVFILT_WRITE`; `EPOLL_CTL_ADD` with `EPOLLIN | EPOLLRDHUP` or `EPOLLOUT`, and `EPOLLONESHOT`). After its event, a registration fires no more, so a descriptor that stays ready never makes the reactor spin, and a stale registration delivers at most one event, which the token's generation makes the reactor ignore (§3).
- **`delete`** removes a registration; one already gone (`ENOENT`, after its one-shot event on `kqueue`, or a descriptor already closed) is not an error.
- **Errors, end of file, hang-up** are delivered as an event of the token: the caller's next read or write sees the condition. A token is a plain integer: `una-sys` never reads it as an address, on `kqueue` too, where `udata` has a pointer type.
- **`wait`** returns at the first events, at the timeout (rounded up to whole milliseconds on `epoll`, capped at `i32::MAX` ms; `None` waits without a timeout), or with no event when a signal interrupts it (`EINTR`), so the caller loops.
- **Close-on-exec.** `epoll_create1(EPOLL_CLOEXEC)` opens the instance close-on-exec atomically. `kqueue()` has no such flag; its descriptor is set `FD_CLOEXEC` by `fcntl` right after, and a `kqueue` is never inherited by a child of `fork` anyway (kqueue(2)), so the window between the two calls exposes nothing. A poller whose flag cannot be set is closed and not used (fail closed).
- **No signal context.** Nothing of `poll` runs in a signal handler, and the handlers of `signals` are unchanged: they still allocate nothing.
- **What is not there.** The readiness table, wakes, timers, the stop pipe's handling and the reactor's loop are safe code in `una-rt`. The reactor wakes its own thread through a socket pair from `std` (`UnixStream::pair`, close-on-exec by `std`), registered like any socket, so `una-sys` needs no `eventfd` or `EVFILT_USER`.

### Size of `una-sys` (decision S7-D6)

`poll.rs` (180 lines), `signals::wake_pipe` (8 lines) and two lines of `lib.rs` fit under the cap of 1,200 lines (ADR 0116 amendment of 2026-10-19): the crate grows from 1,004 lines to 1,194, with the cap unchanged, so no amendment of the cap is proposed. The margin left is 6 lines: a later change to `una-sys`, such as the non-blocking `connect` of §5, needs the cap to be decided first.

## 3. The reactor (`una-rt`, safe code)

- **One reactor thread per process** (decision S7-D1), `una-reactor`, started at the first wait that needs it, as the timer thread of S2 is. It owns one `Poller` and a socket pair that wakes it.
- **The table.** A slab of waits behind one mutex: each wait holds its waker (the mailbox its task parks on, `sched::channel`), its timer in a timing wheel (`wheel.rs`), its descriptor and interest, and, when a host stop can interrupt it, its links in an intrusive list of such waits. A free list reuses slots. A wait's token is its slot and the slot's generation, so an event of a slot reused since is ignored.
- **A wait, on the task's coroutine** (`reactor::wait(fd, interest, timeout, stop, entries)`):
  1. Under the mutex: take a slot, arm the timer at `now + timeout` rounded up to the next millisecond, and link the wait in the stop list when a host stop can interrupt it. When the timer is due before the time the reactor sleeps to, wake the reactor (one byte on the socket pair; a full socket already holds one).
  2. `Poller::add(fd, interest, token)`. A failure removes the wait and returns `Failed`, which the client reports as `Unreachable`.
  3. Register the cancellation entries with `interrupt::register`, whose wake closes the same mailbox with `Cancelled`, then check them once (a cancellation that came before is seen there).
  4. Park on the mailbox. The first close counts: `Ready`, `TimedOut`, `Stopped` or `Cancelled`.
  5. Remove the wait: drop the interrupt registration, `Poller::delete`, cancel the timer, unlink it from the stop list, free the slot.
- **The reactor's loop.** Wait on the poller until the wheel's next expiration; for each token, take the wait's waker out of its slot, when the generation matches, and close it with `Ready`; on the stop pipe's token, close every wait of the stop list with `Stopped`; then advance the wheel and close the waits whose timers fired with `TimedOut`. Wakes are sent outside the mutex. The reactor runs no Una code and touches no task's state: a close pushes a resume job to the task's worker (`sched`).
- **Costs** (decision S7-D5). Register, wake and deregister are O(1): a slab slot, a wheel timer, a list link and two system calls of O(1) each in the kernel; the reactor's wake of one event is one slab lookup. A firing timer is O(1) amortized (`wheel.rs`). The interrupt registration of S3 keeps its O(d log w), for the d race entries of the task, which is 0 outside a race or a started task. A scaling test registers, wakes and removes 1,000, 10,000 and 100,000 waits and requires linear time.
- **The listener of #897.** The `Poller` is per instance, not a global: ADR 0123 rule 19 asks for one `kqueue` or `epoll` per worker, with `EPOLLEXCLUSIVE` and a nested `kqueue` with `EV_DISPATCH` for listening sockets. Those are new interests of `Poller::add` and a poller per worker in `una-rt`, added with #897; `AsFd` lets a poller be registered in another. The reactor's table, wheel and wakes do not depend on there being one reactor.

## 4. `Http` on the reactor

- **Where.** A task on a worker under the real clock: the same condition that sent `Http` to the I/O pool in S1 (`sched::in_task()`, not `--virtual-time`). The root on its own thread, a program without tasks, and every run under `--virtual-time` run the request inline, as before (S1, S4-D4).
- **The request runs on the task's coroutine** (decision S7-D3). `una_http::client::send_interruptible` takes an `Interrupt` whose `waits()` is true, so the socket is non-blocking and every read, write and step of the TLS handshake that would block calls `Interrupt::wait`, which parks the task on the reactor. The client is unchanged in what it sends and reads. Its CPU work, parsing and TLS, runs on the worker between waits, bounded per step by the response limits and one TLS record; like `sort`, it reaches no check point inside a step, so it joins the built-ins of the fairness bound (rule 2).
- **The `Interrupt` of a parked request** (`reactor::Parked`):
  - `stopped()`: the run's host stop when the request is in the body of a `for ever` (ADR 0116 rule 12), or a cancellation of its task (rule 4, first bullet), checked on the task's interruptible entries before each try;
  - `wait(fd, write, timeout)`: `reactor::wait`, mapping `Ready`, `TimedOut`, `Stopped` (the run's stop), and `Cancelled` (which `stopped()` then reports, so the client returns at once);
  - `open(host, port, deadline)`: §5.
- **After the client returns**, `host::http` reads whether the task was cancelled: the operation then ends `interrupted` and the task stops (S3, S4 amendment point 9), as with the pool. Otherwise its result is the client's, a host stop's `Interrupted` included.
- **What a cut releases** (rule 4: "With the reactor, an interrupted request is closed at once"). A cancellation closes the mailbox, the task resumes, the client returns `Interrupted` and drops its connection, which closes the socket, before the task goes on to its `cancelled` event. A deadline does the same with `TimedOut`. No thread is left behind with the request, and the slot of a concurrency limit (rule 8, slice S5) can be released when the client returns.
- **The stop pipe.** The reactor registers the read end of `signals`'s wake pipe. When it is readable, every wait that a host stop can interrupt is woken with `Stopped`, and each one reads its run's stop: when it has come, the request fails with `Interrupted`. The pipe stays readable for good once woken, so the reactor then deletes it, and a later wait that a host stop can interrupt rechecks its run's stop at least every 100 ms (the slice of `una_interp::stopping`), as `OpInterrupt` does after a wake of another run of the process.
- **Failing closed.** A reactor that cannot start (no poller, no thread, no socket pair) leaves `Http` on the I/O pool of S1, which is correct and only holds helpers; one start is tried per process.

## 5. Name resolution and connection (decision S7-D2)

`getaddrinfo` has no non-blocking form, and a non-blocking `connect` needs `socket` and `connect` calls that `std` does not offer, which would add `unsafe` code to `una-sys` beyond the minimal surface of §2.

- **Resolution and connection run as one job on the I/O pool** (`--io-threads`), through `sched::blocking_interruptible`, so the task parks and a cancellation releases it at once. The job is `una_http::client::open`, today's code: the resolver on a thread of its own bounded by the deadline, then `connect_timeout` over the addresses, within the request's deadline. The connected stream comes back to the task, which makes it non-blocking and goes on, on the reactor.
- **A literal address** (`127.0.0.1`, `[::1]`) is not resolved: no resolver thread starts for it.
- **What a helper can still hold.** A peer that drops connection attempts holds a helper for at most the time left of the request's deadline; a peer that accepts and never answers, the stalled upstream of the ADR's Security section, holds none. The I/O pool now serves only these two steps, so its default of 64 helpers bounds connections being made, not requests in flight.
- **Not taken:** a non-blocking `connect` in `una-sys` (about 40 lines: `socket` with `SOCK_NONBLOCK | SOCK_CLOEXEC`, `connect`, `getsockopt(SO_ERROR)`), which would free the helper on a dropped attempt too. It is left for the review to decide, since it grows the `unsafe` surface (decisions to confirm).

## 6. The interpreter

Unchanged: decision S4-D4 stands. Its `Http` stays on its one thread, synchronous, with `OpInterrupt` in a `for ever`, so its outputs, failures and traces do not change. The trait `Interrupt` gains `open`, with a default that is today's resolution and connection, so `OpInterrupt` keeps its behavior. Under the real clock, a started task's request still holds the interpreter's thread while it waits; that is liveness only, as S4-D4 records.

## 7. Descriptors

Each request parked on the reactor holds one socket, where the pool held at most `--io-threads` at once. A run that reaches the process's limit of open descriptors (`RLIMIT_NOFILE`) gets `Unreachable` for the connections it cannot open, the failure the client already gives (decision S7-D4): the runtime does not raise the limit, which is the operator's (`ulimit -n`). The scale tests raise it for their runs.

## 8. Tests

- `una-sys` (`tests/poll.rs`): a socket pair, readable and writable events with their tokens; one-shot (a second wait sees no event while the socket stays ready); `delete` of a registration that fired; the poller's descriptor is close-on-exec; a timeout with no event.
- `una-rt` (`reactor.rs`): a wait woken by readiness, by its deadline, by a cancellation and by the stop list; a stale token ignored; register, wake and remove at 1,000, 10,000 and 100,000 waits in linear time; a parked task on one worker woken while another task of that worker runs.
- `una-cli` (`tests/http.rs`):
  - 64 requests to a server that never answers, with `--io-threads 1`, beside a task that writes and reads a file and prints: the file and print finish long before the requests' `--http-timeout`, and the 64 requests time out together, in about one `--http-timeout`, not 64 of them (a stalled upstream holds no thread);
  - the S4 scale test with 1,000 and 4,000 started tasks all parked on a server that never answers at once, each failing at its `within`, time and memory linear; 10,000 and 15,000 for the release gates (`#[ignore]`).
- The existing tests of S2 to S4 over `Http` (`within` cutting a request, a loser interrupted at once, the `timeout` event of tasks) run unchanged on the reactor.

## 9. Decisions

- **S7-D1** one reactor thread per process, as the timer thread of S2; per-worker pollers come with #897.
- **S7-D2** resolution and connection on the I/O pool, interruptibly; literal addresses skip the resolver.
- **S7-D3** the request runs on the task's coroutine, and only its waits park.
- **S7-D4** no change of `RLIMIT_NOFILE`; a run out of descriptors gets `Unreachable`.
- **S7-D5** O(1) register, wake and deregister, one-shot registrations, generation tokens.
- **S7-D6** `una-sys` under its 1,200-line cap, unchanged.
- **S7-D7** a reactor that cannot start leaves `Http` on the I/O pool (fail closed to S1's behavior).
