# ADR 0123 S4: the serve threads (design note)

- Issue: #1006, part of #897. Implementation plan step 4 of ADR 0123: one listening socket per grant with exclusive wake-ups, per-thread readiness, the shared counters, the peer table, the reserves and shares of requests, replies and delivered requests, the closing of the slowest connections under reply pressure, the handshake rate and share, timers, `Host` checks, the host's own answers, the closing after a refusal, and graceful shutdown (rules 12, 14, 17 to 19); the interpreter's serve threads and `--serve-threads` (rule 20).
- Slices of ADR 0123 named here: S5 is #1007 (the `HttpServer` capability, implementation plan step 5), S6 is #1008 (`[serve]`, step 6). They are not ADR 0117's slices S5 (#970, limits on `Http` grants) and S6 (#971, `limiter`), which this note always names with "ADR 0117".
- Base: ADR 0117 S7 (#972, the reactor, `docs/plan/0117-s7-design.md`) with ADR 0123 S3 (#1005, `docs/plan/0123-s3-design.md`) merged in. S7 sits on an older ADR 0117 S4. The branch is restacked onto ADR 0117 S5 (#970) before any code of this slice is written: that base holds the fixes of #1145 (placement, threads up front) and #1146 (turns), `crates/una-cli/tests/timing_channels.rs` and `crates/una-cli/tests/common/rank.rs`, none of which this base has.
- Normative text: ADR 0123 rules 12, 14, 17 to 20, Security, Costs; ADR 0116, amendment of 2026-10-25 "a precise `τ_conc`" (P0 to P13) and amendment of 2026-10-25 "the timer wheel of P13"; `spec/31`, which gains §6 with this slice.
- Revision: this is the third and final version, after two rounds of independent review (the first: four High, ten Medium and several Low findings; the second: sound after fixes, with findings A to M listed in §17). §17 lists each finding and where this text answers it; §19 lists the points left to follow-up issues. The amendment of ADR 0123 that the departures below need is drafted in `docs/plan/0123-s4-amendment-draft.md`, status Proposed.

This note fixes how S4 is built, and lists the decisions it takes where the records leave a choice. Each decision takes the most secure option the records allow. Where a decision departs from the text of an accepted record, it says so, and the departure waits for the amendment.

## 1. What this slice does, and what it leaves

| Part | This slice | Later |
| --- | --- | --- |
| The serve threads | dedicated threads in both engines, never the task workers (§3, decision S4-D1) | |
| Accepting on the listeners of S3 | from the run's first server operation only; every serve thread may accept, one at a time, through one shared accept poller; each connection goes to the thread its keyed hash names (§4) | |
| Connections | the state machine of rule 12: handshaking, idle, reading, waiting, delivered, writing, closing after a refusal; at most one timer each | |
| Shared counters and reserves | `/once`, the rate, `waiting`, `connections`, `buffers`, `delivered`, `replies`, with their shares per peer (§6) | |
| The peer table | per grant, IPv4 or IPv6 /64, bounded (§6.3) | |
| Reply pressure | the slowest connection of the peer furthest over its share, else the slowest of all, freed at once (§6.5) | |
| Handshakes | the rates per grant and per peer at accept, and the half of the serve threads that may run a step (§6.6) | the TLS server itself waits for #1059 |
| `Host` and the host's own answers | `421`, `408`, `503`, `100 Continue`, the auto-answers' bytes (§7) | the auto-answers' triggers (ADR 0123 S5, #1007) |
| Shutdown | the host's side of rule 18, and `close` (§8) | `Interrupted` and `Closed` of `next` (#1007) |
| `--serve-threads` | both engines (§3) | |
| The label `L_srv` | specified (§10); implemented in the checker with #1007 | #1007 |
| `HttpServer`, `next`, `reply`, reply rights, U3033, IF6, traces | none: no program can name `HttpServer` yet | #1007 |
| `[serve]` and its driver | none | #1008 |
| `--serve-script`, `--serve-out` | none; the conformance scripts of §11.1 use their format through a test harness | ADR 0123 step 7 |

**Dormant until #1007.** The serve threads start only in a run whose program can perform an operation of `HttpServer` (§10, P8). No program can until #1007, so every run of this slice keeps the behavior of S3: listeners bound and held, nothing accepted. The slice is exercised through the engine's own tests (§11), with a consumer written in Rust in place of the program.

## 2. Layout

- `crates/una-http/src/serve/engine.rs` and submodules (safe code, no `unsafe`): the serve thread's loop, the connection state machine, the grant state, the peer table, the reserves, the reply-pressure heaps, the handshake gate, the delivery queues, the inboxes. It is driven through a trait `Io` (sockets, clock, wakes), implemented by real sockets in both engines and by scripted sockets with a clock the test drives. Both engines run this one code (rule 20).
- `crates/una-http/src/wheel.rs`: the timer wheel, moved from `crates/una-rt/src/wheel.rs` without change after #1151 lands, and re-exported by `una-rt`; `una-interp` cannot depend on `una-rt`.
- `crates/una-http` gains a dependency on `una-hash` (SipHash-2-4, no `unsafe`, `getrandom` only) for the keyed placement and the peer table (§4).
- `crates/una-rt` and `crates/una-interp`: start the serve threads before `main` (§10, P8), deliver requests to tasks, take replies from them. The workers do not change, nor does `--workers` and its default (§3).
- `crates/una-hash` gains `SipKey::from_host() -> Result<SipKey, KeyError>`, 128 bits from `getrandom`, safe code, beside `ProcessKey::from_host` (§4).
- `crates/una-sys`: no change (§13).

## 3. Dedicated serve threads (H1, decision S4-D1)

**The choice.** The engine runs on serve threads of its own in both engines, `--serve-threads` of them, and never on the task workers. A serve thread runs no Una code and no task. The workers run tasks only, as in ADR 0117.

**The options weighed.**
- **(a) Dedicated serve threads**, with a label computed over the code that runs after the program's server operations (§10). Chosen.
- **(b) The engine on the workers**, as rule 19 writes it. Its I/O shares each worker's turns, so P7 must count pending engine work as a second task (finding H2), timers fire between turns (finding M2), and every pass of I/O shifts the turns of the tasks on that worker. The engine's work then sits inside the scheduler that the `τ_conc` argument reasons about, and the only label that covers it is P10's over every ancestor of a server operation, which is in effect `τ⁰` of the whole program (finding H1). Not chosen.
- **(c) Others considered.** A single acceptor thread (the first version of this note) adds a hand-off for every connection and a single point of failure, without closing more. A per-worker engine with one listener socket per worker needs `SO_REUSEPORT`, which rule 19 forbids. Neither is better than (a).

**What changes in ADR 0123** (drafted in the amendment):
- **Rule 19.** "The workers of ADR 0117 rule 2 serve the connections" becomes: dedicated serve threads serve them, `--serve-threads` in both engines. The placement of a task that captures a reply right on its connection's worker (ADR 0123 Appendix A, amendment of ADR 0117 rule 2) is withdrawn: every task is placed by P6. A connection belongs to the serve thread its keyed hash names (§4). The `[serve]` path no longer reads, handles and answers a request on one core: a request is read on its serve thread, handled on a worker, and written on its serve thread.
- **Rule 20.** `--serve-threads <n>`, from 1 to 256, applies to native executables too. Its default is the number of cores in the interpreter, as rule 20 says, and half the cores, at least 1, in native code. `--workers` and its default, a worker per core (ADR 0117), do not change, so native code runs about one and a half threads per core by default; the measurement of §12 decides whether the split changes.

**The performance cost.** Two hand-offs between threads per request on every path, where rule 19 promised none on the `[serve]` path: the request goes from its serve thread to a task on a worker, and the reply back to the serve thread's inbox. Each is one lock-free push, and a wake only when the receiving thread sleeps (§5). At saturation threads rarely sleep, so the cost is about two queue operations and the cache misses of moving the request's bytes, on the order of a microsecond per request. Native code also runs more threads than cores by default (workers plus serve threads). The measurement of §12 decides whether this stays within the parity bar, and is the condition of a further amendment if it does not.

## 4. Listening, accepting and placement (M1, decided)

- **The socket.** Each grant keeps the one socket that S3 binds before `main`: `SO_REUSEADDR`, never `SO_REUSEPORT`, `IPV6_V6ONLY` on IPv6 (rule 19, Security).
- **Accepting begins at the first server operation (finding A, decision S4-D12).** The serve threads, their pollers and the accept poller start before `main` (P8), but no listener is registered and nothing is accepted until the run's first server operation, the first call of a site of `S_srv` (§10.1). That operation sets a flag once, by compare-and-swap, and pushes a begin message to every serve thread; the thread that receives it first registers the listeners. Before it, a peer's connection waits in the kernel's backlog (S3's backlog, `min(connections, 65535)`), which is the network's, outside the host under Question 4. So the serve threads hold no connection and do no work before the first hand-off, and their state then is fixed, as P10's "every bucket starts full". A `close` that comes first never begins accepting.
- **One accept poller per run.** Every listener is registered, one-shot, in one shared `Poller` (S7's `una_sys::poll`), and that poller's descriptor is registered, one-shot, in each serve thread's own poller. When a listener is ready, the serve threads whose registration is armed wake; the first to wait on the accept poller with a zero timeout receives the listener's one-shot event, accepts up to 16 connections, then registers the listener again; the others find no event and register the accept poller again. Exactly one thread accepts from a listener at a time, with no new `una-sys` code. On Linux, a registration that fired stays, disabled, until it is deleted, so re-arming is a delete followed by an add, as S7 does (finding L3). Each listener has an atomic `armed` flag: a thread re-arms it only when it changes the flag from false to true, and the thread that takes the listener's event sets it false, so two threads never re-arm one listener at once (M7). Each listener event wakes up to `M` serve threads and costs up to `2M` `epoll_ctl` calls on Linux to re-arm the accept poller in each; the measurement of §12 records it.
- **The owner: a keyed hash (decision S4-D2).** A connection belongs to serve thread `H("una.serve.place", listener, peer address, peer port) mod M`, with `M` the number of serve threads and `H` SipHash-2-4 (`una_hash::sip24`) under a 128-bit key drawn for the run from the operating system's random source by the new `SipKey::from_host` (§2). The key is separate from the process key of hash tables, used only for placement and the peer table's shards, never written to a trace, the report, a message or an error, and has no `Debug`. A domain tag at the start of each input, `una.serve.place` here and `una.serve.peer` for the peer table's shards, keeps the two uses of the key apart. A run whose host gives no random bytes does not start: U6009, whose text in `spec/13 §4` and §6 extends to the placement key of a run that serves (the amendment). The thread that accepts hands a connection that is not its own to the owner's inbox.
  - **Why keyed, and why these inputs.** The owner depends only on the connection's own addresses, which are input: not on the accept order, nor on which thread happened to accept, nor on how busy any thread was. Without the key a peer could choose its source ports to put every connection on one serve thread; with it, it cannot predict the thread, and throwaway connections cannot move later ones, since each connection's place is computed alone.
  - **Cost.** One SipHash of at most 38 bytes per connection, and one push when the acceptor is not the owner.
- **Checks at accept, in this order, each O(1):** the grant's `connections`, taken before `accept` is called: the thread reserves a slot by an atomic increment below the cap, and gives it back when `accept` returns `EAGAIN`, so every accepted descriptor, a connection closed at once included, holds a slot and stays within the descriptor budget of §6.8 (at the cap, the accepting thread does not re-arm the listener; a close of a connection of the grant re-arms it, so the kernel's backlog holds the rest, rule 12); the peer's `peer` count, over which the connection is closed at once; for `https`, the grant's and the peer's handshake rates (§6.6), over which the connection is closed at once, before any handshake work. A connection that passes takes its head reserve (§6.4), its place in the peer table, and the `read` timer from the accept.
- **Errors of `accept`.** `ECONNABORTED` and `EINTR` are skipped. `EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM` and any other error stop accepting on that listener for 100 ms, by a timer of the thread that saw it, so a full descriptor table never makes a thread spin.
- **Close-on-exec (finding L4).** std's `TcpListener::accept` uses `accept4` with `SOCK_CLOEXEC` on Linux, and `accept` followed by `ioctl(FIOCLEX)` on macOS; non-blocking mode and `TCP_NODELAY` are std calls. No `fcntl` of this crate is involved.

## 5. A serve thread's loop (M6, M7)

- **Its state.** A `Poller`, a wake pair (`UnixStream::pair`, as S7's reactor uses), an inbox, its connection slab, its timer wheel, its queue of handshakes and its queue of requests read and waiting.
- **The loop.** Drain the inbox; handle at most 256 ready events and at most 1,024 due timers per pass, so neither starves the other; if there is more, pass again at once; otherwise block in the poller until the earliest live timer.
- **Try first, register on `EAGAIN` (M6).** Every read and write is tried at once; a connection registers its interest only when the call returns `EAGAIN`, one-shot, and deletes it when the wait ends.
- **Noticing a departing peer (finding L4).** While a connection waits for its exchange to end, its read interest stays registered (`EPOLLRDHUP` on Linux, `EV_EOF` on macOS), so a peer that leaves is seen at once and its exchange ends with its reply discarded (rule 14). Bytes that arrive meanwhile are kept within the connection's head reserve and read no further.
- **The inbox.** A `std::sync::mpsc` channel per serve thread: unbounded and lock-free on the sending side, so a task that hands off a reply never waits on a lock a serve thread holds. Each message names its connection by slot and generation, and a message for a slot reused since is dropped (finding L4).
- **Wakes without loss, only of sleeping threads (M6, M7).** A serve thread stores `sleeping = true`, issues `fence(SeqCst)`, checks its inbox once more, and only then blocks in its poller; on waking it stores `false`. A sender pushes its message, issues `fence(SeqCst)`, then loads `sleeping`, and writes one byte to the wake pair only when it is true. The two fences order each side's store before its load, so either the sender sees `true` and writes, or the sleeper sees the message in its last check: no wake is lost, and a busy thread is never woken. A model-checking test with `loom` is planned (§19); until then a test drives the two sides through every interleaving of their four steps by a test hook (§11.2).
- **Every path that pushes to a serve thread,** each followed by the wake rule above:
  1. a connection handed to its owner by the thread that accepted it;
  2. a reply handed off by `reply`, from a task on a worker or from the interpreter's thread;
  3. the end of an exchange by the program (auto-answers, `close`) and by the host's own transitions on another thread (§6.5's victims);
  4. a host stop or `close` (§8): the stop path pushes a stop message to every serve thread;
  5. a re-arm of the accept poller after a close below `connections`;
  6. the begin message of the first server operation.
- **Every path from a serve thread to a task:** a request delivered to a task parked in `next`, and a slot of `/once` or room in `delivered` freed for one.
- **The rate's token (M7).** A `next` that waits only for the grant's rate arms a timer on the run's timer thread (ADR 0117 S2's `timer.rs`, in nanoseconds after #1151) at the token's exact moment; when it fires, the timer thread sends to the task's mailbox, which resumes the task on its worker; the task then takes the token by compare-and-swap, or, when another delivery took it first, arms again for the next one. No serve thread's wheel holds it. Each goes through S7's `sched::channel` send, which pushes a resume job to the task's worker and wakes it by the scheduler's own condition variable, unchanged. S7's reactor and the timer thread push only to workers, as today, and never to a serve thread. Joins and task placement never involve a serve thread.

## 6. Shared state

All shared state is per grant, except the handshake gate, which is per run. Counters sit each on its own cache line (`#[repr(align(128))]`). Sizes and counts are `u64`, with checked or saturating arithmetic (rule 7).

### 6.1 Counters

| Counter | Charged | Released | Operation |
| --- | --- | --- | --- |
| `connections` open | at accept | at close | atomic increment below the cap |
| `waiting` requests | when a request is read and queued | at delivery or refusal | atomic increment below the cap; at the cap, `503` |
| `/once` open exchanges | at delivery | when the exchange ends | atomic increment below the cap; else delivery waits |
| the rate | at delivery | never: a token | GCRA timestamp, §6.2 |
| `buffers` bytes | at a body's admission | at delivery or refusal | atomic add below the cap, with the peer's share |
| `delivered` bytes | at delivery | when the exchange ends | atomic add below the cap; else delivery waits, within `answer` |
| `replies` bytes | when `reply` reserves them (§6.5) | as bytes are written, or when freed under pressure | atomic add, never refused |

A request costs a few atomic operations, O(1), and no lock (rule 19). The peer's shares are atomics of the peer's entry, which the connection holds by reference from its accept, so no request touches the peer table's locks (M5). A delivery that must wait parks its task on the grant's list of waiters under a mutex that only the waiting path takes (declared in the amendment).

### 6.2 The rate (GCRA)

One `AtomicU64` of nanoseconds of the monotonic clock per grant, updated by a compare-and-swap loop, O(1), lock-free. The interval `T / n` is rounded up to a whole nanosecond, never to zero, as rule 7 allows; rounding up only ever delivers fewer than the grant says. A delivery that waits for a token wakes at the token's moment, a timer of the wheel. The period arithmetic is shared with ADR 0117 S5's governor, which keeps its own exact form for `Http`.

### 6.3 The peer table (M4)

- **Keys.** An IPv4 address, or the /64 prefix of an IPv6 address; an IPv4-mapped IPv6 address counts as its IPv4 address (rule 12).
- **Entries.** Open connections, bytes of bodies admitted, bytes of replies pending, and the peer's handshake timestamp (§6.6), each an atomic; an entry is held by reference by each of its connections.
- **Structure (decision S4-D3).** 64 shards per grant, each a hash map behind its own mutex, hashed by SipHash under the run's placement key with the tag `una.serve.peer`, so a peer cannot make its addresses collide. Locks are taken at accept, at close and at expiry only, never per request. O(1) on average, and O(1) expected under any choice of addresses.
- **Life and bound.** An entry lives while it has an open connection, or while its next allowed handshake time is in the future; then a timer of the serve thread that last updated it removes it, unless it was used again. The GCRA timestamp of a peer is at most one burst ahead of the clock, and a burst of `max(4, handshakes / 16)` at that rate lasts one second, so an entry with no connection lives at most one second. Entries with no connection are created only by handshakes the grant's rate admitted, at most `handshakes` per second. So the table of a grant holds at most `connections + handshakes` entries: 1,536 with the defaults. A plain `http` grant keeps no handshake timestamps, and holds at most `connections`.

### 6.4 Reserves of request bytes (rule 13)

1. **Heads.** Each connection's reserve, `line + header`, is accounting counted at accept and released at close, never allocated ahead.
2. **Bodies.** A declared `Content-Length` reserves its whole length before the body is read; a chunked body reserves 64 KiB before each step. A reservation must fit the peer's `peer_buffers` and the grant's `buffers`: the grant's atomic add first, then the peer's; a failure of the second gives the first back. When it does not fit, `503` with `Retry-After: 1` at once, and the connection closes after it (§7). `100 Continue` is sent only after the admission.
3. **Delivered.** From delivery until the exchange ends, the request's bytes are charged to `delivered`; delivery waits for room, within `answer`, then `503`.

### 6.5 Reply pressure (H4, M5)

- **Segments, counted until dropped (H4).** A pending reply is a queue of segments of at most 16 KiB each. The owner writes from the front segment, and drops each segment as soon as its last byte is written, giving its bytes back to the grant's and the peer's counters at that moment. So the counters always equal the bytes of reply segments allocated and not yet dropped, and the victims of pressure are chosen by that count.
- **Reserve first, free victims, then allocate.** `reply` reserves its serialized size `r` on the grant's `replies` and the peer's `peer_replies`, with atomic adds. When the grant is then over `replies`, the same thread, before it allocates its own segments, frees victims until the grant is back under `replies` or no other reply is pending:
  1. it takes the grant's pressure mutex and picks the victim: the connection with the most bytes left of the peer furthest over its share; when no peer is over its share, the connection with the most bytes left of all; ties to the lower connection id;
  2. it marks the victim discarded in its outbox (below), removes it from the heaps, takes its pending segments and drops them, and gives their bytes back to both counters, at once;
  3. it pushes a close of the victim to the victim's owner (§5). The owner only closes the descriptor later.

  Then it allocates and serializes its reply into segments, and pushes it to its connection's owner. `reply` never waits on a peer, and never fails for this (rule 13). `reply` has no check point: the freeing loop, at O(log c) per victim, runs inside it, and each victim is freed once, so its cost is amortized over the replies that were charged; under ADR 0117 rule 2's fairness bound, `reply` counts among the built-ins without a check point.
- **Outboxes.** A connection's pending segments sit in an outbox under a mutex of that connection alone: the owner takes it for one non-blocking write of one segment, the victim's freer for the take above. It is uncontended except under pressure, and it is the only lock a request's reply meets (declared in the amendment).
- **Heaps, lazily.** Outside pressure there are no heaps, and nothing but atomics is touched. When a grant first goes over `replies`, its pressure mutex builds three indexed max-heaps from its connections with pending replies, O(c): the peers by how far they are over their share, each peer's connections by bytes left, and all connections by bytes left. A connection's key is the bytes its segments hold when last read; a connection has one reply pending at most (rule 12), so its key, set when the reply is inserted, only ever decreases as segments are dropped, and it is an upper bound. A pick refreshes the top connection from its atomic count, and keeps it when it still beats the next key, which is then a true maximum; otherwise it reinserts it lower and tries again. Each refresh lowers a key, so picks cost O(log c) amortized. A peer's key, how far it is over its share, can grow: every reservation made while the heaps exist updates the peer's key, or inserts the peer, and inserts the new connection, under the pressure mutex, O(log c). That is a lock per reply during pressure only (declared in the amendment). The heaps are kept, under the mutex, while the grant is above three quarters of `replies`, and dropped below. A new episode starts only after a quarter of `replies` has been written or freed, so the O(c) build is amortized over that many bytes.
- **One reply larger than `replies`** is kept only while the grant has no other reply pending, else its connection is closed at once (rule 13).
- **The bound outside `--max-memory`.** For a grant, at any moment: the bytes of reply segments allocated and not dropped are exactly what the counters hold, at most `replies` plus the replies being reserved at that moment, at most one per worker (or the interpreter's thread), each no larger than the `Text` it was made from, which `--max-memory` bounded; plus the head bytes held, at most `connections × (line + header)`, which rule 13 counts; plus, with #1059, the TLS buffers of `rustls`, bounded per connection by its buffer limit (one record of about 16.5 KiB each way, set with `set_buffer_limit`), which rule 13's statement of the worst case gains: about 34 MiB for 1,024 connections (finding L4).

### 6.6 Handshakes

- **Rates at accept.** Per grant, `handshakes` per second, and per peer `max(4, handshakes / 16)` per second, each a GCRA timestamp as in §6.2. Over either, the connection is closed at its accept, before any handshake work (rule 9).
- **The share.** A per-run count of serve threads inside a handshake step. A thread takes a place before each step and gives it back after, only while fewer than `ceil(M / 2)` are taken; otherwise the connection stays in the thread's queue of handshakes, within its `read` timer.
- **One serve thread (finding L4).** With `M = 1`, `ceil(1 / 2)` is 1: the one thread may run handshakes, and does at most 8 handshake steps per pass of its loop, so requests on its open connections still progress between them.
- **The TLS server waits for #1059.** Until then the handshake step is a trait whose test implementation is compiled only under `cfg(test)` (finding L4), and a run whose program can serve and that holds an `https` grant is refused before `main`, a usage error: it fails closed rather than serve plain bytes on an `https` address.

### 6.7 Delivery (M5)

- **Per serve thread, first come, first served** (rule 19): each thread's queue of requests read and waiting, in the order it finished reading them, under a mutex of that thread held for O(1), taken by the thread and by `next` (declared in the amendment).
- **Across serve threads.** Each thread has a ready flag, an atomic set when its queue becomes non-empty. A `next` scans the flags from a per-run rotating cursor, an atomic advanced at each delivery, so no thread's requests wait behind another's indefinitely (finding L4). O(1) expected under load, O(M) at worst.
- **For `[serve]`** (#1008), each worker's driver loop scans from its own index.

### 6.8 Descriptors (H3)

- **The target.** Before `main`, S3's `prepare` raises the soft limit of open descriptors, within the hard limit, to

  `T = Σ connections + L + 1 + 3M + C + F + R`

  with `L` the listeners (one per grant), 1 the accept poller, `3M` a poller and a wake pair per serve thread, `C = 256` sockets kept for the program's own `Http` requests, `F` the files that `Files` helpers may hold open at once (`--file-threads` in native code, 1 in the interpreter), and `R = 64` for `Console`, the reactor, its pair, the wake pipe and the runtime. A run that cannot reach `T` is a usage error naming `T`, before anything runs, as S3 does for its smaller target. `prepare` therefore takes `M` and `F`.
- **S7's cap** on the sockets of `Http` requests, in a run that serves, is the soft limit less `Σ connections + L + 1 + 3M + F + R`, so at least `C`, and never 1.
- **The I/O pool's fallback.** When S7's reactor cannot start, `Http` runs on the I/O pool, whose requests today take no place from S7's cap: up to `--io-threads` sockets, outside `C`. In a run that serves, the pool's path takes a place from the same gate before it connects (`una_rt::sockets`), so every socket of the program's `Http`, on either path, counts against `C`.
- **Accepting within the budget.** A connection's slot of `connections` is taken before `accept` (§4), so the server's descriptors never exceed `Σ connections`.
- **Test.** Under a hard limit of `T - 1` the run is refused, and its message names `T`; at `T`, the cap of `una_rt::sockets` is exactly `C`; with every connection of the grant open, 256 requests of the program's own `Http` all get a socket, on the reactor and with the reactor disabled; and a flood of connections over `peer`, each closed at once, never takes a descriptor past `Σ connections`.

## 7. `Host`, the host's own answers, and the closing after a refusal

- **`Host`.** The syntax check of S2, then `Hosts::allows` of S3 with the grant's `hosts`, else `421`. Over TLS, the server name of the handshake MUST equal the `Host` name, else `421` (with #1059). O(names).
- **The host's answers** are written by `una_http::reply::write_answer` (S2): the refusals of rules 8, 10 and 11; `408` past `read`; `503` for a full `waiting`, a body that does not fit, a wait past `answer` for delivery, an exchange past `answer` without a reply, and a stop or `close` that finds a request undelivered or being read. The auto-answers of rule 4.5 use the same writer; their triggers come with #1007.
- **The closing after a refusal** (rule 10). After a `400`, `413`, `431` or `503` that closes the connection, the owner writes the answer, shuts down the sending side, reads and drops what the peer still sends, at most 64 KiB or for one second, by the connection's one timer, and closes. The connection keeps its place in `connections` and in the peer table until it closes.

## 8. Graceful shutdown

At a host stop (ADR 0116 rule 14), or at `close`: every serve thread deregisters and closes the listeners and its idle keep-alive connections; requests read but not delivered, or being read, get `503` with `Connection: close`, as soon as their head is read or at once when it is not complete; open exchanges drain, each reply with `Connection: close`. At the hard stop (ADR 0116 rule 16), every auto-answer is written with one non-blocking write, then the connection closes, without waiting on its peer. O(connections + queued requests).

## 9. The interpreter and `--serve-threads`

- **`--serve-threads <n>`,** 1 to 256, in `una run` and in executables (§3). Out of range or not a number is a usage error, exit code 2. Accepted and unused in a run that does not serve.
- **What crosses** between a serve thread and the interpreter's thread: only plain owned data, a request's bytes and fields, a serialized reply (ADR 0117 rule 11).
- **`--virtual-time` (M9).** A run under `--virtual-time` that holds a `serve:` grant and has no `--serve-script` is refused before `main`, a usage error: live peers send in real time, which virtual time cannot place. Until step 7 adds `--serve-script`, every such run is refused. Under `--serve-script`, both engines use one serve thread, so they read and deliver in the same order (rule 21).

## 10. Timing channels

ADR 0116's amendment of 2026-10-25 requires that every runtime decision beside a site depend only on the tasks that run and on time measured from points inside them (D, step 3), or carry a label. Question 4 of ADR 0123 makes the request stream input, with its dependence on the server's timing out of model (IF5). This note reads that as: the stream's bytes, and their timing relative to the program's own steps, are input labeled `internal`, as requests are.

### 10.1 The serve threads' sites, and `L_srv`

The amendment states this label as IF-S1, a name that does not collide with ADR 0116's IF8.

**The server's hand-off sites,** `S_srv` (finding L4):
- each call of `next`, `reply` and `close` of `HttpServer`;
- each `start` whose captures can hold a `ReplyRight`, by their types, or a function value (IF6's sites): the started task's end may send an auto-answer;
- the driver of a `[serve]` table (#1008), as a node of the graph whose body is its loop, `handle` and `on_stop`.

**The suffix of a call site.** For a call site `c` in the body of `m`, `sfx(c)` is `τ⁰` of the statements of `m`'s body that can run at or after `c`: at each level of nesting, the statement that holds `c` and every statement after it in its block, and, when `c` is inside a loop, the whole loop; the handling clauses of every statement it holds, and of every enclosing statement, since a failure after `c` runs them; the `on stop` block of every enclosing `for ever`, which runs after the loop's last iteration; with `τ⁰` of everything those statements call. For a node `n`, `Suf*(n)` is the least label with `Suf*(n) ≥ sfx(c) ⊔ Suf*(m)` for every edge from `m` to `n` at a site `c`, over every edge of P1: direct calls, `start` and the parts of every construct, calls of function values and `uses of` parameters through `Λ`, operations through `K(c)` to every handler of the capability, and calls inside lambdas and handlers' operations, which leave from `Λ` and from the handler's node. Roots (`main`, the driver) have no incoming edge. Within a strongly connected component every member's `Suf*` includes `τ⁰` of every member's body, as P11 does.

**The label.**

    L_srv = internal ⊔ ⨆ over s in S_srv of ( sfx(s) ⊔ Suf*(node(s)) ⊔ τ_conc(s) ) ⊔ L_res

It joins `τ` of every timing site of a package whose program can perform an operation of `HttpServer`, as `L_res` does (C of the amendment), and it is `public` in every other package, which starts no serve thread. It costs one backward sweep per body for `sfx`, and one pass over the components of the graph for `Suf*`: O(V + E + size of the bodies), with the join's O(k).

**What it leaves out:** the code that can only run before every server operation: a program's set-up, its configuration, the keys it loads and the secrets it computes before it serves. That is the narrowing that H1 asks for. It holds because nothing is accepted before the first server operation (§4, finding A): before it, the serve threads do no work at all. P10's ancestor rule, which the first version used, kept all of it, since `main` is an ancestor of every site.

**When #1155 lands,** P11's `Suf` over the graph of expressions MAY replace `sfx`, which is its statement-level over-approximation: a statement holds everything evaluated after the call in it, and more.

### 10.2 Soundness

**The claim.** Take a timing site `e`, an observer at `l` with `τ(e) ≤ l`, so `L_srv ≤ l`, and two runs that differ only in data above `l`, with the same stop schedule and the same request stream relative to the program's steps. That the stream's phase relative to the program's steps is input is a decision of the amendment, which reads Question 4 so. Then the work the serve threads do, and the moments they do it, relative to `e`'s start, are the same in both runs; and what remains of their effect on `e` is the sharing of the machine, which P12 puts out of model.

1. **The serve threads are a function of three inputs, from the first hand-off on.** They run no Una code, and before the run's first server operation they hold no connection and do nothing but wait on their inboxes (§4): their state at the first hand-off is fixed, the same in every run. From it on, what a serve thread does, and when, is a deterministic function of: the events of its sockets (the request stream); the messages of its inbox (the program's hand-offs, §5's paths 2, 3, 4 and 6, and other serve threads' messages, which are functions of the same inputs); and its timers, armed by the two. No choice of a serve thread reads a worker's state, a task's state, a turn or the scheduler's queues.
2. **The stream** is the same in both runs relative to the program's steps (Question 4), and is labeled `internal`, the floor of `L_srv`.
3. **The hand-offs.** A hand-off `h` happens in a task at a site of `S_srv`. Its effect on `e` depends on when `h` happens relative to `e`'s start. Every step that runs after `h` and before `e`'s start is one of:
   - a step of `h`'s task after `h`, in the body that holds `h` or in a caller after the call that led to it: in `sfx(h) ⊔ Suf*(node(h))`, by the definition, since each caller's call site is an edge into the chain;
   - a step of another part of a construct that holds `h`'s task, which runs until that construct ends: the construct is a statement of a caller's body that holds the edge into `h`'s chain, so `sfx` of that edge holds it whole;
   - a step of code after `h`'s task has ended, in the task that started it: again in `sfx` of the `start` edge or the construct's statement;
   - a step of a task that runs beside `e`: covered by `τ_conc(e)`, as P4 states;
   - a step of `e`'s own expression, when `h` is inside it: covered by `τ⁰(e)`.

   So the time from `h` to `e` depends only on data at most `L_srv ⊔ τ_conc(e) ⊔ τ⁰(e) ≤ τ(e) ≤ l`. A hand-off that comes after `e`'s start is inside `e` or beside it, the last two cases. Code before the first hand-off of the run plays no part: before it the serve threads do no work, and the connections that peers open wait in the kernel's backlog, the network's, so the code before the first hand-off sets neither the serve threads' state nor their work. The kernel's own handling of those connections is the host's, as for any process that listens.
4. **State left by earlier hand-offs** (counters, timestamps, heaps, the peer table, pending replies): a function of the stream and of earlier hand-offs `h'`. Each `h'` lies before `e`, and the time from `h'` to `e` is covered by step 3, applied to `h'`. So the state at any moment of `e` is the same function of the same data in both runs.
5. **Timers.** Each is armed at a stream event or a hand-off, and fires at its exact moment, or up to 1 ms later where `epoll_wait` rounds its timeout (M2). The rounding depends on the phase of the arming moment, which steps 2 and 3 fix. So firing moments are the same in both runs. This channel is covered by `L_srv`, not closed by the wheel, and no `una-sys` line is spent on `timerfd` or `epoll_pwait2` (§13).
6. **What the serve threads do to the workers.** Besides the machine, three things only, each a function of steps 2 to 5:
   - **wakes of tasks** (§5's paths from a serve thread): a resume job pushed to a task's worker at the moment a request is delivered or a slot is freed;
   - **locks shared with tasks:** the delivery queues and the waiters' list (taken by `next`), the outboxes and the pressure mutex (taken by `reply`), each held for O(1), or O(log c) under pressure. A task meets them only at a site of `S_srv`, and how long it waits is a function of the serve threads' activity (steps 1 to 5); the time after that site is in its `sfx`. The inboxes are lock-free on the sending side, so `reply` never waits on a serve thread;
   - **the descriptor table:** disjoint budgets (§6.8), so the serve threads never make a task's `Files` or `Http` operation fail; descriptor numbers are never values of the program.
7. **What is left: the machine.** P12 applies here because of step 1's fix: with nothing accepted before the first hand-off, no serve work exists whose moments the set-up code could shift. With steps 1 to 6, the serve threads run the same work at the same moments relative to `e` in both runs. What remains is how the operating system shares cores, caches and memory bandwidth between them and the workers, and the allocator's state that their buffers leave. That is P12's "the host's load", "the processor's caches" and "the allocator's state", exactly as for a program without tasks, whose timing other threads of the host also share: the effect of a work pattern that does not differ between the two runs. P9's helpers are not a counterexample: their pattern differs with `secret` data unless labeled, and `L_res` labels it; here `L_srv` labels the pattern, and only the sharing of the machine is left to P12.
8. **Conclusion.** The outcome at `e` is the same in both runs, and it carries at least `τ(e) ⊇ L_srv`. The argument holds per fixed stop schedule, as T6 does.

### 10.3 Every shared resource

| Resource | What it can depend on | Handling |
| --- | --- | --- |
| Which serve thread owns a connection | the connection's addresses and the run's key | **SC1, closed**: keyed hash of input (§4) |
| Placement of tasks | the constructs around them | **closed**: P6 for every task; the reply-right placement is withdrawn (§3) |
| The serve threads' I/O, handshakes, parsing, writing, closes | the stream; hand-offs | **SC2, labeled `L_srv`** (§10.2 steps 1 to 4, 7) |
| `/once`, `waiting`, `delivered`, the rate's timestamp | the stream; earlier deliveries and exchange ends | **SC3, labeled `L_srv`**: when `next` returns, and its `TimedOut` under a deadline (§10.2 step 4) |
| Which request `next` returns, across serve threads | the serve threads' progress (stream, hand-offs) and the order in which tasks on all workers, background tasks included, call `next` | **SC4, out of model, stated**: new text in the amendment and a new `out_of_model` entry of rule 22 (§10.4) |
| The stream's phase against the program | when peers send | **SC5, input**, `internal` (§10.2 step 2) |
| Threads of the server | none: all start before `main` | **SC6, closed** (P8, extended below) |
| Accepting before the program serves | with accepting before `main`: the set-up code's length, against the peers' arrivals | **SC9, closed**: accepting begins at the first server operation (§4) |
| Timers | their arming moments; `epoll_wait`'s rounding | **SC7, labeled `L_srv`** (§10.2 step 5) |
| I/O inside a worker's turns | none: no engine on workers | **SC8, closed** (§3) |
| Wakes of tasks; locks met at `next` and `reply`; the descriptor table | as above | **labeled `L_srv`**, or disjoint (§10.2 step 6) |
| Reply buffers and the allocator | the replies' sizes and moments | pattern under `L_srv`; the allocator's state is P12 |
| The placement key | the host's random source | none: no program data reaches it, and it is never shown |
| Delivered requests in the heap | requests and the program's speed | `--max-memory` exhaustion is termination, out of model (P12) |

**P6.** Every task is placed by P6's `(w + i) mod N`. **P7.** The workers' turns are unchanged; no engine work runs on a worker. **P8.** Its static condition gains "an operation of `HttpServer`, or a `[serve]` table"; such a run starts, before `main`, every serve thread with its poller and wake pair, and the accept poller. It registers the listeners at the first server operation, not before (§4). **P9.** `L_srv` joins `L_res` at every site. **P10.** The counters' history is `L_srv`'s. **P12.** §10.2 step 7. **P13.** Every timer is armed in nanoseconds on the wheel of the P13 amendment; `epoll_wait`'s rounding is labeled.

### 10.4 SC4, in the amendment

IF5 of ADR 0123 says what `internal` code may learn "about the timing of the program's own handlers from which requests arrive". Which request `next` returns depends also on the timing of tasks that are not handlers, through when each task calls `next`, and on the serve threads' progress. The amendment states it under IF5 and adds to rule 22's `out_of_model` list, for packages that serve: "which request a call of `next` receives when several serve threads hold requests, which the timing of every task of the run and of the serve threads decides".

### 10.5 The cost to programmers

Stated as a Question of the amendment:
- A server's timing sites are at least `internal`, as IF7 already makes them in request tasks.
- `L_srv` is `secret` when code that can run after a server operation reads `secret` data: a handler that reads it, or code after the accept loop. Inside the server's `tasks` statement this adds nothing to P2, which already gives every site the statement's `κ`, so rule 15's `start answer(exchange) within d` and rule 16's `[serve] within` keep the status IF7 gives them. Outside it, such as an `on stop` with a `within`, or a later construct, those sites become `secret` too.
- Code that runs only before the first server operation, such as set-up that loads keys or computes on `secret` data, leaves `L_srv` unchanged.

## 11. Tests

### 11.1 Conformance first

The cases are written before the engine, in the format of rule 21: `tests/conformance/serve/<case>/` holds `grant.txt` (the `serve:` spec), `script.jsonl` (connections, bytes, paced reads and closes at virtual milliseconds), `consumer.jsonl` (the replies a Rust consumer gives each delivered request and when, standing for the program until #1007) and `expected.jsonl` (the bytes each connection receives, with the virtual time each leaves). A harness in `crates/una-http/tests/serve_scripts.rs` runs each through the engine with one serve thread and a clock the test drives; step 7 runs the same scripts through both engines. After each case, every counter, reserve, share and heap is zero and the peer table empty, or holds only entries whose handshake time is still ahead.

Cases: keep-alive and `requests`; `Connection: close`; `HTTP/1.0`; pipelined requests in order; each timer passing (`idle`; `read` with `408`; `write`; `answer` with `503` and a late reply discarded); a peer at `peer`, and two IPv6 peers in one /64; `connections` reached, the next connection held until one closes; a body over `buffers` or `peer_buffers`; `Expect: 100-continue` after admission only; `waiting` full; `/once`, the rate and `delivered` delaying delivery; `Host` without and with `hosts`; the closing after a refusal with a peer that sends past 64 KiB and one that stays past one second; a peer that leaves while its exchange is open; 64 paced connections that never read 1 MiB replies, 64 that never read 3 MiB, and 64 that trickle 1 MiB bodies, while a fresh client is answered and the slowest connections close, with the pending bytes never above the bound of §6.5; one reply larger than `replies`; a stop with requests queued, in flight, being read and idle; the hard stop.

### 11.2 Unit and engine tests (`una-http`)

GCRA rounded up; the peer table's keys, its expiry, and its bound of `connections + handshakes`; the reserves given back on every path, a refused second charge included; the victim's buffer freed by the thread that crossed the cap, before its own allocation; the lazy heaps' picks against a brute-force maximum over random histories; the handshake gate never above `ceil(M / 2)`, and at `M = 1`; inbox messages for a reused slot dropped; the wake rule never losing a message under a test that interleaves sender and sleeper at every step; the first server operation beginning accepting once, whatever operations race for it; two threads never re-arming one listener at once; a reply's segments dropped as written, with the counters equal to the bytes allocated at every step; a peer's key raised by a reservation during pressure; `--virtual-time` without `--serve-script` refused; the descriptor target of §6.8 and its three tests.

### 11.3 Deterministic guards (ordinary `cargo test`)

- **SC1, placement spread (M1).** With a fixed test key, the 65,535 valid source ports (1 to 65,535) of one peer address, and 65,535 addresses of one /64 with one port, each spread over `M` of 2, 7 and 64: the chi-square statistic of the counts stays below the 0.999 quantile for `M - 1` degrees of freedom, and the largest count is below 1.2 times the mean. A second guard checks that the owner never depends on which thread accepted, nor on the accept order.
- **SC6.** A run that can serve has every serve thread, poller and wake pair in place before its first job, and no listener registered until its first server operation; one that cannot serve starts none of them.
- **SC9.** A peer that connects and sends before the first server operation is not accepted until it: the engine's accept count stays 0, and its connection is still in the backlog.
- **SC7.** After any history of arming and cancelling connection timers, each wheel holds only live timers.
- **SC8.** The workers' loop has no path into the engine: a test that runs the engine while counting the engine's entries from worker threads, which must be zero.

### 11.4 Native and interpreter

`una-rt` and `una-interp`: the engine on real serve threads with the Rust consumer, over loopback, requests answered and shutdown draining; only owned data crossing in the interpreter. `una-cli`: `--serve-threads` 0, 257 and `x` refused with exit code 2 by `una run` and by an executable; `--virtual-time` with a `serve:` grant refused before `main`.

### 11.5 Real-clock oracles

In `crates/una-cli/tests/timing_channels.rs` (on the ADR 0117 S5 base), each with exactly `#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]`, run with `--ignored --test-threads=1` in release form, 20 runs of each input alternating, judged by the Mann-Whitney U test of `crates/una-cli/tests/common/rank.rs` at a two-sided p below 0.001, with a control of one input against itself. They need a program that names `HttpServer`, so they land in #1007's pull request, before it merges. Each program is accepted by `una check` under `L_srv`, so each builds:

- `serve_prefix_secret` (the narrowing of §10.1): a `secret` decides 0 or 300 ms of work before the program's first server operation. The program then prints a ready line (`public`, before any server operation), and only then does the harness connect, so the stream is fixed relative to the program's steps. A request task over `public` work under `within` replies with whether its deadline passed. The two inputs' outcomes and times MUST not differ.
- `serve_owner_spread` (SC1): two runs, whose `secret` decides 0 or 300 ms of set-up work, each served by 8 connections from crafted ports of one address; the request tasks' outcomes MUST not differ.
- `serve_handoff_residue` and `serve_delivery_history` (SC2, SC3): positive controls. Their `public` twins, in which a `public` key decides how many reply bytes are pending to a paced client, or how long the code between two deliveries under a rate takes, MUST show a difference, so the channel that `L_srv` labels exists on the machine. Their `secret` versions are refused by U4003 in #1007's checker tests, as P9's and P10's are.

- `serve_prefix_early_peers` (SC9, finding A): the variant of `serve_prefix_secret` whose harness connects 512 connections and sends a request with a 64 KiB body on each as soon as the process starts, before the set-up code ends. On a design that accepts before the first server operation, the serve threads read and parse those requests during the set-up of 0 ms but not during the one of 300 ms, so the measured `within` overlaps that work for one input only, and the oracle fails. With accepting at the first server operation, every connection waits in the backlog for both inputs, and the oracle passes. The pull request measures the parsing work on the gate machine and sets the deadline so that both outcomes before the fix lie at least 25 ms from it.

SC6 and SC8 have no oracle: their channel has no switch a `secret` can turn, since the threads start before `main` and no engine runs on a worker. Their guards are those of §11.3.

### 11.6 Scaling

Each doubling MUST take less than three times as long as the size before: engine with scripted sockets, 1,000 to 50,000 connections accepted and closed, 10,000 to 100,000 reply charges under pressure, 10,000 to 100,000 timers, headers of 1 to 64 KiB and 1 to 1,000 lines, bodies in 1-byte chunks; live sockets, 1,000 and 10,000 idle keep-alive connections, memory linear and no time growth per request, 50,000 for the release gates.

## 12. The measurement (M8)

On the measuring machine, quiet, in release form, before the amendment is accepted and again before ADR 0123's performance step:
- **What.** Connections accepted per second, and the cost of each listener event (up to `M` wakes and `2M` `epoll_ctl` calls, §4); CPU time per request at saturation; the 50th and 99th percentile latency at fixed loads with uniform handler costs and with skewed ones (one request in a hundred a hundred times slower); memory per idle connection.
- **Against.** (i) this design; (ii) a prototype with the engine on the workers and per-worker exclusive accept (rule 19 as written); (iii) the `hyper` on `tokio` baseline of ADR 0123 Costs, with the same limits.
- **The condition.** When (i) is above the parity bar against (iii), CPU time per request at saturation above 1.1 times, while (ii) is within it, a further amendment is proposed that weighs the gap against the label (ii) needs. The records are kept with the runs' provenance.

## 13. `una-sys`

**No new binding.** The accept poller and every serve thread's poller use S7's `Poller` with its one-shot interests; std does `accept`, non-blocking mode, `TCP_NODELAY`, `shutdown` and peer addresses; the wake pairs are std's `UnixStream::pair`. Exact timers (`timerfd`, `epoll_pwait2`) are not added: SC7 is labeled (§10.2 step 5). Every libc function stays bound once: `poll` and `getrlimit` in `io.rs`, `setrlimit` in `server.rs`, `fcntl` once in `signals.rs`, `epoll_ctl` and `kevent` in `poll.rs`.

- **Counting rule.** Before and after any change to the crate: `find crates/una-sys/src -name '*.rs' -exec cat {} + | wc -l`, recorded in the commit message.
- **The count, for the cap's decision (finding L1).** 1,199 on S7; 1,382 at `4a39956f`, after S3 was merged in with the folds of one `poll`, one `getrlimit`, one conversion of `errno` and one `fcntl`; 1,384 at `7f59d354`, after `rustfmt` of the merged files. Over the cap of 1,200 (ADR 0116 amendment of 2026-10-19); this slice adds none.

## 14. Implementation plan

Each sub-step is a commit, with fmt, clippy and the workspace's tests run in the check slot before it is pushed.

0. **Docs.** This note; the amendment draft (`docs/plan/0123-s4-amendment-draft.md`), which goes to ADR 0123 as a dated amendment once the project accepts it.
1. **Restack** onto ADR 0117 S5 (#970), then #1151 and #1148 as they land.
2. **Conformance first.** The script cases of §11.1 and their harness, failing.
3. **The wheel** moves to `una-http`, unchanged; `una-http` depends on `una-hash`, which gains `SipKey::from_host`.
4. **The engine's state.** Counters, GCRA, the peer table, reserves, pressure, the handshake gate, delivery, the placement hash, with their unit tests, guards and scaling tests.
5. **The serve thread.** Its loop, inbox and wakes, connections, timers, `Host`, the host's answers, the closing after a refusal, shutdown: the cases of step 2 pass.
6. **Both engines.** The serve threads started before `main`, accepting from the first server operation, delivery to tasks and replies back, `--serve-threads`, the refusal under `--virtual-time`, the descriptor target, S7's cap, and the I/O pool's fallback taking a place from it; U6009's text in `spec/13` for the placement key.
7. **Live and scaling tests.**
8. **Spec and changelog.** `spec/31 §6`, "Connections and serve threads", with rules 12, 14, 17, 18 (host side), 19 and 20 as amended; `changelog.d/1006-serve-threads.md`; `docs/security/tcb.md` lists the engine.
9. **Security review** of the engine, then the measurement of §12.

## 15. Dependencies

- **ADR 0117 S5 (#970):** the restack base, with #1145, #1146, the timing oracles' file and `rank.rs`.
- **#1151:** the wheel in nanoseconds with removal at once.
- **#1148:** `L_res`, which `L_srv` joins; needed by #1007 for the label.
- **#1087:** the analysis, whose graph (P1), `τ_conc` (P4) and construct tree `L_srv` uses.
- **#1147:** the ancestor machinery of P10 (`limit_labels`), which `Suf*`'s pass over the components reuses in the forward direction.
- **#1155:** P11; its `Suf` MAY replace `sfx` (§10.1).
- **#1059:** the TLS provider; until it is decided, `https` serving fails closed (§6.6).
- **#1128:** `identity()` in `una-sys/src/server.rs` keeps its `geteuid` until `dir::effective_uid` lands.
- **ADR 0117 S6 (#971, `limiter` and `std.limit`): not needed.** The handshake rates and the delivery rate are limits of the grant (rule 7), kept by the host, of the kind of ADR 0117 rule 8, not the program's `limiter` (rule 9). Nothing waits for it.

## 16. Decisions

- **S4-D1** dedicated serve threads in both engines; P6 places every task (§3).
- **S4-D2** a connection belongs to the serve thread of a keyed SipHash of (listener, peer address, peer port), under a key of the run that is never shown; every serve thread may accept, through one accept poller (§4).
- **S4-D3** the peer table: 64 shards per grant under mutexes, locked at accept, close and expiry only; entries bounded by `connections + handshakes`.
- **S4-D4** reply pressure: replies in segments of at most 16 KiB, each dropped and uncounted once written; reserve, free victims at once on the thread that crosses the cap, then allocate; lazy heaps only under pressure, peers' keys updated at every reservation during it.
- **S4-D5** delivery across serve threads by ready flags and a rotating cursor.
- **S4-D6** wake only sleeping threads, by the store-check protocol of §5; inboxes lock-free on the sending side.
- **S4-D7** the descriptor target `Σ connections + L + 1 + 3M + 256 + F + 64`; a `connections` slot taken before `accept`; the I/O pool's fallback gated by the same cap.
- **S4-D8** the serve threads start only in a run that can serve, before `main`; dormant until #1007.
- **S4-D12** listeners are registered, and accepting begins, at the run's first server operation, set once; before it, connections wait in the kernel's backlog.
- **S4-D9** `https` serving fails closed until #1059.
- **S4-D10** `L_srv` over the suffixes of the server's hand-off sites, in the checker with #1007, after the amendment is accepted.
- **S4-D11** `--virtual-time` with a `serve:` grant and no `--serve-script` is refused before `main`.

## 17. The review's findings

| Finding | Where this text answers it |
| --- | --- |
| H1, `L_srv` was `τ⁰` of the whole program | §3 (option (a), dedicated serve threads), §10.1 (`L_srv` over the suffixes of the hand-off sites, which leaves set-up code out), §10.2 (the soundness argument, with P12 for the machine), §10.5 and the amendment's Question (the cost); the oracles of §11.5 are accepted programs |
| H2, a worker with one task never does its I/O | moot: no engine runs on a worker (§3); a serve thread caps events, timers and handshake steps per pass (§5, §6.6) |
| H3, the descriptor budget | §6.8: the target, S7's cap at least 256, and three tests |
| H4, memory outside `--max-memory` | §6.5: reserve, free the victims at once on the detecting thread, then allocate; the bound, TLS buffers included |
| M1, keyed placement | §4: SipHash-2-4 of `una-hash` under a key of the run, never shown; every serve thread may accept; §11.3's spread test |
| M2, timers between turns, `epoll_wait`'s rounding | SC7 labeled under `L_srv` (§10.2 step 5); no `una-sys` cost (§13) |
| M3, SC4 | §10.4 and the amendment: IF5 text and a new `out_of_model` entry |
| M4, the peer table's size | §6.3: expiry at the end of the handshake burst; at most `connections + handshakes` entries |
| M5, locks per request | §6.1 (peer shares by reference), §6.5 (lazy heaps only under pressure; the outbox mutex declared), §6.7 (ready flags) |
| M6, try first, wake only sleepers | §5 |
| M7, every push path, no lost wakeup | §5's lists and the store-check protocol, with its test (§11.2) |
| M8, the measurement | §12, with the 1.1 parity bar as the condition of a further amendment |
| M9, `--virtual-time` with live listeners | §9 and S4-D11 |
| M10, dependencies | §15: #1087, #1147, #1148 for #1007's label, #1155 |
| L1, `una-sys` at 1,384 | §13 |
| L2, slice numbers | the header: #1007 and #1008, distinct from ADR 0117's #970 and #971 |
| L3, re-arming on Linux | §4: delete, then add |
| L4, small gaps | handshake share at one thread (§6.6); `rustls` buffers in rule 13's bound (§6.5); inbox messages by slot and generation (§5); read interest to notice a departing peer (§5); the handshake double under `cfg(test)` (§6.6); the ready cursor's rotation (§6.7); `accept4` and `ioctl(FIOCLEX)` (§4); `L_srv` names its sites (§10.1) |

The second round's findings:

| Finding | Where this text answers it |
| --- | --- |
| A (High), accepting before the first server operation | §4 (S4-D12): listeners registered and accepting begun at the first server operation, set once; §10.1, §10.2 steps 1, 3 and 7 (P12 valid given the fix); SC9 in §10.3; the oracle `serve_prefix_early_peers` and the guard SC9 (§11.3, §11.5); the amendment states the stream's phase as a decision |
| A, `sfx` | §10.1: handling clauses, `on stop`, and the edges through `Λ` and `K(c)` |
| Descriptors | §6.8: `F` in the target; a slot taken before `accept`; the I/O pool's fallback gated by `C` |
| H4 (High), memory outside `--max-memory` | §6.5: segments of 16 KiB dropped as written and counted until dropped; peers' keys updated at every reservation during pressure; `reply` has no check point |
| M1 | §2 and §4: `SipKey::from_host`; domain tags; U6009 extended (amendment); §11.3 within the 65,535 valid ports |
| M5 | §6.1, §6.7 and the amendment: the delivery-queue mutex per serve thread and the waiters' list mutex declared |
| M6 | §5: `fence(SeqCst)` on both sides; a `loom` test (§19); §4 and §12: the accept poller's wake cost |
| M7 | §5: the rate's token path through the run's timer thread; §4: the listener's `armed` flag |
| Amendment gaps | the amendment: ADR 0123's Diagnostics line and `spec/31 §2` rule 7 replaced, U6009's scope, Costs' parity expectation for `[serve]`, the label renamed IF-S1, `--workers` unchanged (§3) |

## 18. Open questions, with the answers this slice takes

1. **Dedicated serve threads, or the engine on the workers?** *Answer:* dedicated (S4-D1, §3), subject to the measurement of §12.
2. **Does CPU sharing between serve threads and workers need a label?** *Answer:* the pattern of the serve threads' work does, `L_srv`; the sharing of the machine by a pattern that does not differ is P12's (§10.2 step 7).
3. **Exact timers in `una-sys`?** *Answer:* no; labeled (§10.2 step 5), at no `una-sys` cost.
4. **The `una-sys` cap.** *Answer:* this slice adds no line; the cap's decision stays with the amendment S3 proposed, and §13 records the count.
5. **Native `--serve-threads` default?** *Answer:* half the cores, at least 1, with `--workers` unchanged; the measurement may change it.
6. **Does a server need ADR 0117 S6?** *Answer:* no (§15).
7. **Does accepting start before `main`?** *Answer:* no (reversing the first version's answer): the serve threads start before `main`, and accepting begins at the first server operation (S4-D12).

## 19. Follow-up issues

Points this slice does not settle, to be filed:
1. **A `loom` dev-dependency** for the model-checking test of the wake protocol (§5): `loom` would join `Cargo.lock` as a test-only crate, which the project decides; until then the interleaving test of §11.2 stands.
2. **The `una-sys` cap** (§13): 1,384 lines against 1,200; the amendment that S3 proposed, with this count.
3. **`rustls` buffer limits** with #1059: measure the per-connection TLS memory and fix the limit that rule 13's bound states (§6.5).
4. **`sfx` from P11** once #1155 lands (§10.1).
5. **The native thread split** (`--workers` against `--serve-threads`) after the measurement of §12.
