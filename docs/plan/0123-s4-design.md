# ADR 0123 S4: the serve threads (design note)

- Issue: #1006, part of #897. Implementation plan step 4 of ADR 0123: one listening socket per grant with exclusive wake-ups, per-worker readiness, the shared counters, the peer table, the reserves and shares of requests, replies and delivered requests, the closing of the slowest connections under reply pressure, the handshake rate and share, timers, `Host` checks, the host's own answers, the closing after a refusal, and graceful shutdown (rules 12, 14, 17 to 19); the interpreter's serve threads and `--serve-threads` (rule 20).
- Base: ADR 0117 S7 (#972, the reactor, `docs/plan/0117-s7-design.md`) with ADR 0123 S3 (#1005, `docs/plan/0123-s3-design.md`) merged in. S7 sits on an older ADR 0117 S4. The branch is restacked onto ADR 0117 S5 (#970) before any code of this slice is written: S5's base holds the fixes of #1145 (placement, threads up front) and #1146 (turns), `crates/una-cli/tests/timing_channels.rs` and `crates/una-cli/tests/common/rank.rs`, none of which this base has. The code of this slice also builds on #1151 (deadlines in nanoseconds and the timer wheel of the amendment of 2026-10-25) and #1148 (`L_res`), once they land.
- Normative text: ADR 0123 rules 12, 14, 17 to 20, Security, Costs; ADR 0116, amendment of 2026-10-25 "a precise `τ_conc`" (P0 to P13) and amendment of 2026-10-25 "the timer wheel of P13"; `spec/31`, which gains §6 with this slice.

This note fixes how S4 is built, and lists the decisions it takes where the records leave a choice. Each decision takes the most secure option the records allow. Where a decision departs from the text of an accepted record, it says so, and the departure waits for a dated amendment (§11).

## 1. What this slice does, and what it leaves

| Part | This slice | Later |
| --- | --- | --- |
| Accepting on the listeners of S3 | one acceptor thread per run, one-shot readiness, connections handed to their owner (§3) | |
| Per-worker readiness | a poller per worker; I/O between turns and while idle (§4) | |
| Connections | the state machine of rule 12: idle, reading, waiting, writing, closing after a refusal; at most one timer each | |
| Shared counters and reserves | `/once`, the rate, `waiting`, `connections`, `buffers`, `delivered`, `replies`, with their shares per peer (§5) | |
| The peer table | per grant, IPv4 or IPv6 /64, O(1) per accept and close (§5.3) | |
| Reply pressure | the slowest connection of the peer furthest over its share, else the slowest of all (§5.5) | |
| Handshakes | the rate per grant and per peer at accept, the half of the workers that may run one (§5.6) | the TLS server itself waits for #1059 |
| `Host` and the host's own answers | `421` by the grant's `hosts`, `408`, `503`, `100 Continue`, the auto-answers' bytes (§6) | the auto-answers' triggers (step 5) |
| Shutdown | the host's side of rule 18, and `close` (§7) | `Interrupted` and `Closed` of `next` (step 5) |
| The interpreter | serve threads of its own, `--serve-threads` (§8) | |
| `HttpServer`, `next`, `reply`, reply rights, U3033, IF6, traces | none: no program can name `HttpServer` yet | step 5 |
| `[serve]` and its driver | none | step 6 |
| `--serve-script`, `--serve-out` | none; the conformance scripts of §10 use their format and a test harness | step 7 |

**Dormant until step 5.** The serve threads start only in a run whose program can perform an operation of `HttpServer` (§9, P8). No program can until step 5, so every run of this slice keeps the behavior of S3: listeners bound and held, nothing accepted. The slice is exercised through the engine's own tests (§10), with a consumer written in Rust in place of the program.

## 2. Layout

- `crates/una-http/src/serve/engine.rs` and submodules (safe code, no `unsafe`): the connection state machine, the grant state shared by the workers, the peer table, the reserves, the reply-pressure heaps, the handshake gate, and the delivery queues. It is driven through a trait `Io` (sockets, clock, wakes), implemented by real sockets in both engines and by scripted sockets with a clock the test drives. Both engines run this one code (rule 20).
- `crates/una-http/src/wheel.rs`: the timer wheel, moved from `crates/una-rt/src/wheel.rs` without change after #1151 lands, and re-exported by `una-rt`. The engine needs it, and `una-interp` cannot depend on `una-rt`.
- `crates/una-rt`: the workers own their pollers and run the engine between turns (§4); the acceptor thread; the placement of rule 19; the start before `main` (§9).
- `crates/una-interp`: the serve threads and `--serve-threads` (§8).
- `crates/una-sys`: no change (§12).

## 3. Listening and accepting

**One acceptor thread per run, and an owner chosen by the accept order.**

- **The socket.** Each grant keeps the one socket that S3 binds before `main`: `SO_REUSEADDR`, never `SO_REUSEPORT`, `IPV6_V6ONLY` on IPv6. No other process can bind the same address and port and take a share of the connections (rule 19, Security).
- **The acceptor.** One thread, `una-accept`, started before `main` (§9), registers each listener in a poller of its own with the one-shot read interest that S7's `Poller::add` already has. At an event it accepts up to 16 connections from that listener, then registers it again. One thread waits on each listener, so each connection wakes exactly one thread: the exclusive wake-ups of rule 19, without `EPOLLEXCLUSIVE` or a nested `kqueue` with `EV_DISPATCH`.
- **The owner (decision S4-D1).** The `k`-th connection the run accepts, counting every descriptor the kernel hands over, those closed at once included, is owned by worker `k mod N`, with `N` the number of workers (native) or of serve threads (interpreter). The acceptor hands the descriptor to its owner's inbox and wakes the owner. The owner sets it non-blocking and `TCP_NODELAY`, registers it in its own poller, and serves it for its whole life.
  - **Why.** Rule 19 has the worker that accepts own the connection. Which worker wakes first then depends on how busy each worker is, which the tasks of earlier constructs decide, possibly on `secret` data, and the connections stay with that worker for their keep-alive life. Rule 19's placement then puts request tasks on those workers: a placement set by the history of earlier code, the channel that P6 closes for task placement (SC1 in §9). The accept order is the kernel's backlog order, a function of the request stream alone, which Question 4 makes input.
  - **Cost.** One atomic increment and one push into an inbox per connection, not per request; keep-alive amortizes it. The acceptor's own work per connection is `accept`, two `fcntl` calls inside std, and the push: far below a TLS handshake or a request's parse, which the owner does.
  - **Departure from rule 19's text.** Rule 19 describes per-worker waits with `EPOLLEXCLUSIVE` and `EV_DISPATCH`, and the acceptor as the owner. This choice needs no new `una-sys` code (§12) and closes SC1. It waits for the amendment of §11.
- **Checks at accept, in this order, each O(1):** the grant's `connections` (at the cap, the acceptor stops registering the listener until a connection of the grant closes, so the kernel's backlog holds the rest, rule 12); the peer's `peer` count (a connection over it is closed at once); for `https`, the grant's and the peer's handshake rates (§5.6), a connection over either closed at once, before any handshake work. A connection that passes takes its head reserve (§5.4), its place in the peer table, and the `read` timer from the accept.
- **Errors of `accept`.** `ECONNABORTED` and `EINTR` are skipped. `EMFILE`, `ENFILE`, `ENOBUFS` and `ENOMEM` stop accepting on that listener for 100 ms, by a timer of the acceptor, so a full descriptor table never makes the acceptor spin; the connection waits in the backlog. Any other error is treated the same way, and counted for the report of a later slice.
- **The descriptor budget (decision S4-D2).** S3 raises the soft limit to the sum of `connections` plus 64. S7's cap on the sockets of `Http` requests (`una_rt::sockets`) is computed from the soft limit at its first use, so on a server it would claim the descriptors meant for connections. The cap becomes the soft limit, less the sum of `connections` over the `serve:` grants, less S7's reserve, and at least 1. A server's connections and its own requests then cannot starve each other of descriptors.

## 4. Per-worker readiness

- **A poller per worker.** Each worker owns a `Poller` (S7's `una_sys::poll`), its connection slab, its timer wheel, its queue of handshakes, and its queue of requests read and waiting (rule 19, "Per worker"). A wake socket pair (`UnixStream::pair`, as S7's reactor uses) lets other threads wake it: the acceptor's hand-off, a reply handed off by a task on another worker, a stop.
- **When the worker does I/O (decision S4-D3).** Only between turns, never inside one:
  - **between two turns,** one poll with a zero timeout, then at most 64 events and the due timers of its wheel;
  - **while it has no ready task,** a blocking poll until the earliest live timer of its wheel, which replaces the condition variable a worker waits on today.

  The budget is a count of events, never a span of time, and no tick shared by the run decides when a pass happens. The work of a pass depends only on the events ready, which the request stream and the replies decide (§9, SC8).
- **Connections and their state.** A slab of connections with a free list; a token is the slot and its generation, so a stale event is ignored, as in S7. A connection is in one of: handshaking, idle (between requests), reading, waiting for delivery, delivered (an open exchange), writing, closing after a refusal (rule 10). Registrations are one-shot, added when the connection waits for readiness and deleted when the wait ends, as S7 does.
- **One request at a time** (rule 12). A connection's next request is read only after the reply to the current one is written, so a pipelined client is served in order. Bytes that arrive past the current request stay in the connection's buffer, within its head reserve.
- **Timers** (rule 12, P13). At most one per connection, `idle`, `read` or `write`, and one per open exchange, `answer`, on the owner's wheel. The wheel is that of the amendment of 2026-10-25 on P13: exact deadlines in nanoseconds of the monotonic clock, cancellation that removes the timer at once, no dead entries left by code that has ended. `read` counts from the accept for a connection's first request, the handshake included; a request past `read` is answered `408` and closed; an idle connection past `idle`, or a reply not written within `write`, is closed.

## 5. Shared state

All shared state is per grant, except the handshake gate and the accept counter, which are per run. Counters sit each on its own cache line (`#[repr(align(128))]`, which covers the adjacent-line prefetch of x86 and the 128-byte lines of Apple's cores). Sizes and counts are `u64`, compared with checked or saturating arithmetic (rule 7).

### 5.1 Counters

| Counter | Charged | Released | Operation |
| --- | --- | --- | --- |
| `connections` open | at accept | at close | atomic increment below the cap, O(1) |
| `waiting` requests | when a request is read and queued | at delivery or refusal | atomic increment below the cap; at the cap, `503` (rule 14) |
| `/once` open exchanges | at delivery | when the exchange ends | atomic increment below the cap; at the cap, delivery waits |
| the rate | at delivery | never: a token | GCRA timestamp, §5.2 |
| `buffers` bytes | at a body's admission (§5.4) | at delivery or refusal | atomic add below the cap, together with the peer's share |
| `delivered` bytes | at delivery | when the exchange ends | atomic add below the cap; else delivery waits, within `answer` |
| `replies` bytes | at `reply`'s hand-off | as bytes are written, or at close | atomic add, never refused (rule 13); over the cap, §5.5 |

A request costs a few atomic operations on these, O(1), and no lock (rule 19). Delivery that must wait parks the calling task (step 5) on a list of the grant's waiters under a mutex, which only the waiting path takes.

### 5.2 The rate (GCRA)

- **One timestamp per grant**, an `AtomicU64` of nanoseconds of the monotonic clock, updated by a compare-and-swap loop: O(1), lock-free. The interval `T / n` is rounded up to a whole nanosecond, never to zero, as rule 7 allows ("A token interval of a rate is rounded up, never to zero"). S5's governor counts in `1/n` nanoseconds in 128-bit integers under a mutex, which a lock-free update cannot do on the targets' stable atomics; rounding up only ever delivers fewer, never more, than the grant says.
- **Exact deadlines.** A delivery that waits for a token wakes at the token's exact moment, a timer of the wheel (P13).
- **Not S5's governor.** The governor charges a request to several `Http` grants at once and orders waiters by ticket. A delivery is charged to one grant, so the timestamp alone serves. The arithmetic of periods is shared.

### 5.3 The peer table

- **Keys.** An IPv4 address, or the /64 prefix of an IPv6 address; an IPv4-mapped IPv6 address counts as its IPv4 address (rule 12).
- **Entries.** Open connections, bytes of bodies admitted (`peer_buffers`), bytes of replies pending (`peer_replies`), and the peer's handshake timestamp (§5.6). An entry exists while any of them is nonzero, so the table holds at most `connections` entries.
- **Structure (decision S4-D4).** 64 shards per grant, each a hash map behind its own mutex, with the hasher of std (`RandomState`, keyed at random per run), so a peer cannot choose addresses that collide. O(1) on average per accept, close and share charge; O(1) expected under any choice of addresses, since the keys are secret.
- **Departure from rule 19's wording.** Rule 19 lists the peer table among the atomic counters. A lock-free map is far more code for no gain: the table is touched at accept and close, and when a body or a reply is charged, not on each byte. Requests on an open connection read no peer entry. The mutexes are per shard and held for O(1) work.

### 5.4 Reserves of request bytes (rule 13)

1. **Heads.** Each connection's reserve, `line + header`, is accounting counted at accept and released at close, never allocated ahead. Reading a head never waits for a shared budget.
2. **Bodies.** A declared `Content-Length` reserves its whole length before the body is read; a chunked body reserves 64 KiB before each step. A reservation must fit the peer's `peer_buffers` and the grant's `buffers`, charged together: the grant's atomic add first, then the peer's entry; a failure of the second gives back the first, so the counters never hold a charge that was refused. When it does not fit, `503` with `Retry-After: 1` at once, and the connection closes after it (§6). `100 Continue` is sent only after the admission.
3. **Delivered.** From delivery until the exchange ends, the request's head and body bytes are charged to `delivered`. Delivery waits for room, within `answer`, after which the request is refused with `503`.

### 5.5 Reply pressure

- **Charges.** A reply's serialized bytes are charged to the grant's `replies` and the peer's `peer_replies` when it is handed off. The bytes are allocated outside `--max-memory` (rule 13), so `reply` never waits or fails for them.
- **Heaps (decision S4-D5).** Per grant, under one mutex that only this accounting takes:
  - an indexed max-heap of the peers by how far they are over their share, `bytes - peer_replies`;
  - per peer, an indexed max-heap of its connections by the bytes their pending reply has left;
  - an indexed max-heap of all connections with a pending reply by bytes left.

  Each holds positions in its entries, so an update or a removal is O(log c). A connection's key is updated when its reply is charged, after each write step of at most 16 KiB, and when the reply ends.
- **Closing.** While the grant is over `replies`, the owner of the connection whose charge crossed the cap closes, in order: the connection with the most bytes left of the peer furthest over its share; when no peer is over its share, the connection with the most bytes left of all. Ties go to the lower connection id, so the choice is a function of the state. A closed connection's reply is discarded, and the program never sees it (rule 14, IF4). A close of a connection that another worker owns is a message to that worker's inbox; its bytes stay charged until it closes, so the loop never closes two for one excess.
- **One reply larger than `replies`** is written only while the grant has no other reply pending, else its connection is closed at once (rule 13).

### 5.6 Handshakes

- **Rates at accept.** Per grant, `handshakes` per second, and per peer `max(4, handshakes / 16)` per second, each a GCRA timestamp as in §5.2, the peer's in its entry. Over either, the connection is closed at its accept, before any handshake work (rule 9).
- **The share.** A per-run counter of the workers inside a handshake step. A worker takes a place before each step and gives it back after, and takes one only while fewer than `ceil(N / 2)` are taken; otherwise the connection stays in the worker's queue of handshakes, within its `read` timer. Request handling always keeps the other workers (rule 19).
- **The TLS server itself waits for #1059** (S3, "TLS provider"). Until then the engine's handshake step is a trait whose only implementation is a test double, and a run whose program can serve and that holds an `https` grant is refused before `main`, a usage error: it fails closed rather than serve plain bytes on an `https` address.

### 5.7 Delivery

- **Per worker, first come, first served** (rule 19): each worker's queue of requests read and waiting, in the order it finished reading them.
- **Across workers, for the language form (decision S4-D6).** A run-level ready list of workers whose queue holds a request, each worker in it at most once, under one mutex: `next` takes the first worker of the list and the oldest request of that worker's queue. O(1). Order across workers is not kept, as rule 19 says.
- **For `[serve]`** (step 6), each worker's driver takes from its own queue only.

## 6. `Host`, the host's own answers, and the closing after a refusal

- **`Host`.** The syntax check of S2 (`una_http::request::host`) on every grant, then `Hosts::allows` of S3 with the grant's `hosts`: `421` otherwise. Over TLS, the server name of the handshake MUST equal the `Host` name, else `421` (with #1059). O(names).
- **The host's answers** are written by `una_http::reply::write_answer` (S2): an empty body, `Content-Length: 0`, `Retry-After: 1` on `503`, `Connection: close` when the connection closes. The engine sends them for: the refusals of rules 8, 10 and 11; `408` past `read`; `503` for a full `waiting`, a body that does not fit, a wait past `answer` for delivery, an exchange past `answer` without a reply, and a stop or `close` that finds a request undelivered or still being read. The `500` and `503` auto-answers of rule 4.5 use the same writer; their triggers come with step 5.
- **The closing after a refusal** (rule 10). After a `400`, `413`, `431` or `503` that closes the connection, the owner writes the answer, shuts down the sending side (`TcpStream::shutdown(Write)`), reads and drops what the peer still sends, at most 64 KiB or for one second, whichever comes first, by the connection's one timer, and closes. The connection keeps its place in `connections` and in the peer table until it closes, so lingering connections cannot be used to exceed either.

## 7. Graceful shutdown

At a host stop (ADR 0116 rule 14), or at `close` (rule 1):

1. the acceptor deregisters and closes every listener, and each worker closes its idle keep-alive connections;
2. requests read but not delivered get `503` with `Connection: close`; a request still being read gets it as soon as its head is read, or at once when its head is not complete;
3. open exchanges drain: their replies carry `Connection: close`, and each connection closes after its reply;
4. at the hard stop (ADR 0116 rule 16), every open exchange whose task is cancelled gets its auto-answer (step 5) written without waiting on its peer: one non-blocking write, then close.

The cost is O(connections + queued requests). Workers do this from their own inboxes; nothing waits on a peer. `Interrupted` and `Closed` of `next` come with step 5.

## 8. The interpreter's serve threads (rule 20)

- **`--serve-threads <n>`,** from 1 to 256, by default the number of cores, at most 256. Out of range, or not a number, is a usage error, exit code 2. It is a flag of `una run` only: executables serve on their `--workers`, and refuse it as an unknown flag (decision S4-D7). It is accepted and unused in a run that does not serve.
- **The threads** run the same engine as native workers, each with its poller, slab, wheel and queues, and the same acceptor, owner rule and shared state. They run no Una code.
- **What crosses.** Only plain owned data, as for the interpreter's other waiting operations (ADR 0117 rule 11): a request's bytes and fields, owned `String`s and `Vec`s, go to the interpreter's thread through a delivery mailbox; a serialized reply goes back to the owner's inbox. No value of the interpreter crosses.
- **Under `--virtual-time`** the interpreter uses one serve thread whatever `--serve-threads` says, as the native run uses one worker (rule 19), so the two engines read and deliver in the same order (rule 21).
- **Up front.** All `n` threads and the acceptor start before `main`, in a run whose program can serve (§9, P8).

## 9. Timing channels

ADR 0116's amendment of 2026-10-25 requires that every runtime decision beside a site depend only on the tasks that run and on time measured from points inside them (D, step 3), or carry a label. Question 4 of ADR 0123 makes the request stream input, with its dependence on the server's timing out of model (IF5). This section reads that as: the stream's bytes, and their timing relative to the program's own steps, are input labeled `internal`, as requests are. Every shared resource of the serve threads is listed below with what its behavior can depend on, and how P6 to P13 apply.

| Resource | What its behavior can depend on | Effect | Handling |
| --- | --- | --- | --- |
| Accept, and which worker owns a connection | with rule 19's text: which worker woke first, set by the load of earlier tasks; with S4-D1: the accept order only | placement of request tasks (rule 19) | **SC1, closed structurally**: owner `k mod N` (§3) |
| Per-worker readiness, and the I/O of the serve threads (handshakes, reads, parses, writes, closes) | the request stream; replies, whose content is at most `internal` and whose moment is the handler's | time taken on the workers' cores beside any site | **SC2, labeled `L_srv`** (below) |
| When the I/O runs on a worker | only the events ready, between turns or while idle, by an event budget | the order of task turns | **SC8, closed structurally**: no I/O inside a turn, no time budget, no shared tick (§4, P7) |
| Counters `/once`, `waiting`, `delivered`, the rate's timestamp | how many exchanges are open and when earlier deliveries happened, which the program's earlier code timed | how long `next` waits; which requests are refused | **SC3, labeled `L_srv`** for waits; refusals are the stream's (IF5) |
| Which request `next` returns, across workers | the workers' load and the stream | the content of an `internal` value | **SC4, out of model, stated**: Question 4 and IF5 of ADR 0123, accepted, "what `internal` code may learn ... from which requests arrive"; the report's `out_of_model` entry of rule 22 names it |
| The stream's phase against the program | when peers send, against where the program is | which code the host's reading work lands beside | **SC5**: input, `internal`; its work is under `L_srv`'s floor |
| The peer table, reserves and shares, reply-pressure heaps, handshake rates | the stream; reply sizes (`internal`); the program's timing in answering | which connections are closed or refused (the network only, IF4); host work | network effects out of model (IF4, IF5); work under `L_srv` |
| The handshake share | other workers' handshake steps | when a handshake step runs | host work, under `L_srv` |
| Timers (`idle`, `read`, `write`, `answer`, linger, accept back-off) | the exact moments they were armed, by input events | network only; their firing work | **SC7, closed structurally**: exact nanoseconds, removal at once, no dead entries (P13 and its wheel amendment) |
| Threads of the server (acceptor, pollers, interpreter serve threads) | with a lazy start: whether earlier code started them | time taken inside the construct that would start them | **SC6, closed structurally**: all start before `main` (P8) |
| The peer table's hash keys | a random key of the run and the peers' addresses | lookup time | none: no program data reaches it |
| Delivered requests in the heap | requests and the program's speed | `--max-memory` exhaustion | termination, out of model (P12) |
| Descriptors (§3, S4-D2) | the program's own files and requests | accept back-off | host work and network, under `L_srv` |

**P6, placement.** Rule 19 places a started task that holds a reply right on the worker that owns its connection. With S4-D1 that worker is a function of the accept order, which is input, so the placement depends only on the constructs around the task and on input labeled `internal`. Every site beside a request task already has `τ_conc` at least `internal`, since the construct that delivers requests reaches `next`, whose `ρ` is `internal` (IF1). Other started tasks keep P6's `(w + i) mod N`.

**P7, turns.** The engine runs between turns and while a worker is idle, never inside one, so a turn's count is unchanged: from the later of its start and the moment its worker gained a second task. The engine's handlers are not tasks and do not count as one.

**P8, threads up front.** The static condition of P8 gains "an operation of `HttpServer`, or a `[serve]` table". A run that meets it starts, before `main`: the acceptor, every worker's poller and wake pair, and in the interpreter every serve thread. The listeners are registered and accepting begins before `main`. Nothing of the server starts inside a construct.

**P9, residue, and the new label `L_srv` (SC2, SC3).** The work of the serve threads is not inside any construct, as a helper's residue is not, and it runs beside every site of the run on the same cores. Its amount and moments depend on the stream (`internal`) and on when the program handed off its replies and called `next`, which code before those sites decides, possibly on `secret` data. A program without tasks shows the channel already: a loop that delivers a request, computes for a `secret`-dependent time, replies, and later runs a `within` over `public` work, whose deadline the writing of earlier replies may or may not overlap. So:

- **`L_srv`** is the join of `internal` and, over every site `s` that performs an operation of `HttpServer`, `τ⁰` of the body of every ancestor of `s`'s node, `s`'s own node included, `τ_conc(s)` and `L_res`: P10's rule, applied to the server's operations. Code between two hand-offs of one task lies in an ancestor's body, as P10 argues for requests; the work of other tasks is under their own `τ_conc`.
- **It joins `τ` of every timing site** of a package whose program can perform an operation of `HttpServer`, as `L_res` does (C of the amendment). It is `public` in every other package, which starts no serve thread.
- **It covers SC3.** A `next` under a deadline whose wait at the rate, a `/once` slot or `delivered` depends on earlier deliveries is P10's example with `next` in place of a request; its `TimedOut` carries `t`, which includes `L_srv`.
- **The cost to programs.** A server's timing sites are at least `internal`, which IF7 already makes them in request tasks. A server whose request handling reads `secret` data, or whose ancestors do, has `L_srv = secret`, and no `within` or `first of` anywhere in it can then take `internal` operations: IF7 already says so for request tasks, and this extends it to the rest of such a program. P11's code between hand-offs MAY relax it later, as for requests.
- **Where it lands.** In the checker with step 5, where the first sites exist, after the amendment of §11 is accepted. Step 5 MUST NOT merge without it, and no release ships `HttpServer` before it.

**P10, history.** The history that earlier code leaves in the grant's counters and timestamps is covered by `L_srv`'s ancestor term (above). The handshake rates' timestamps are driven by accept times, which are input; they decide only closes at accept.

**P12, out of model.** The memory budget (delivered requests count toward `--max-memory`, whose exhaustion ends the run); the allocator's state; caches; the host's load from other processes. Nothing else of this slice is added to P12.

**P13, exact deadlines.** Every timer of the engine is armed and fired at its exact deadline in nanoseconds of the monotonic clock, on the wheel of the amendment on P13's wheel. The `Date` header is written from the clock in seconds and is never a value of the program.

**Named channels, and their status:**

| Channel | Status | Reason |
| --- | --- | --- |
| SC1, connection placement history | closed | owner by accept order (S4-D1) |
| SC2, serve work beside sites | labeled, `L_srv` | the work cannot be removed from the cores it shares; its moments follow the program's hand-offs |
| SC3, delivery history | labeled, `L_srv` | rule 7's rate counts deliveries, which the program times |
| SC4, cross-worker delivery order | out of model, stated | ADR 0123 Question 4 and IF5, accepted |
| SC5, the stream's phase | input, `internal`; work under `L_srv` | Question 4 |
| SC6, lazy start of the server's threads | closed | P8, extended |
| SC7, timer state | closed | P13 and its wheel |
| SC8, I/O inside turns | closed | I/O only between turns and while idle (S4-D3) |

## 10. Tests

### 10.1 Conformance first

The cases are written before the engine, as script cases in the format of rule 21: `tests/conformance/serve/<case>/` holds `grant.txt` (the `serve:` spec), `script.jsonl` (connections, bytes, paced reads and closes at virtual milliseconds), `consumer.jsonl` (the replies a Rust consumer gives each delivered request and when, standing for the program until step 5) and `expected.jsonl` (the bytes each connection receives, with the virtual time each leaves). A harness in `crates/una-http/tests/serve_scripts.rs` runs each through the engine with one worker and a clock the test drives. Step 7 runs the same scripts through `una run --serve-script` and the executable, and compares the engines byte for byte. After each case the harness asserts that every counter, reserve, share and heap is back to zero, and the peer table empty.

Cases: keep-alive and `requests`; `Connection: close`; `HTTP/1.0`; pipelined requests answered in order; each timer passing (`idle`, `read` with `408`, `write`, `answer` with `503` and a late reply discarded); a peer at `peer`, and two IPv6 peers in one /64 counted as one; `connections` reached, the next connection held until one closes; a body over `buffers` or `peer_buffers`, `503` at once; `Expect: 100-continue` answered after admission only; `waiting` full; `/once`, the rate and `delivered` delaying delivery; `Host` that is a DNS name without `hosts` (`421`), and with `hosts`; the closing after a refusal, with a peer that keeps sending past 64 KiB and one that keeps the connection open past one second; 64 paced connections that never read 1 MiB replies, 64 that never read 3 MiB, and 64 that trickle 1 MiB bodies, while a fresh client is answered within its own time and the slowest connections close; one reply larger than `replies`; a stop with requests queued, in flight, being read and idle; the hard stop.

### 10.2 Unit and engine tests (`una-http`)

GCRA with the interval rounded up; the peer table's keys (/64, IPv4-mapped); the reserves charged and given back on every path, including a refused second charge; the heaps' choice and its tie-break; the handshake gate never above `ceil(N / 2)`; the owner rule `k mod N` whatever thread accepts; accept back-off on `EMFILE`, without spinning; S7's socket cap less the sum of `connections`.

### 10.3 Deterministic guards (ordinary `cargo test`)

- **SC1.** With a test acceptor that hands connections from a chosen thread, the owners follow the accept order alone.
- **SC6.** A scheduler made for a program that can serve has the acceptor, every poller and every listener registration in place before its first job; one made for a program that cannot serve starts none of them (beside #1145's guard of P8).
- **SC7.** After any history of arming and cancelling connection timers, each worker's wheel holds only live timers, and no wakeup beyond what they need (the test of the wheel amendment, run on the engine's timers).
- **SC8.** A worker's engine pass never runs while a task's turn is open: a counter the test reads, under a scheduler driven by the test.

### 10.4 Native and interpreter

- `una-rt`: the engine on real workers with the Rust consumer, over loopback: requests answered on their connection's worker; a reply handed off from another worker; shutdown draining.
- `una-interp`: the same on serve threads, and requests and replies crossing as owned data only.
- `una-cli`: `--serve-threads` 0, 257 and `x` refused with exit code 2; accepted on `una run`; refused by an executable; an `https` grant in a run that can serve refused before `main` until #1059 (with step 5, when such a run exists).

### 10.5 Real-clock oracles

In `crates/una-cli/tests/timing_channels.rs`, each with exactly `#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]`, run with `--ignored --test-threads=1` in release form, 20 runs of each input alternating, judged by the Mann-Whitney U test of `crates/una-cli/tests/common/rank.rs` (on the S5 base) at a two-sided p below 0.001, with a control of one input against itself. Each needs a program that names `HttpServer`, so each lands in step 5's pull request, before it merges:

- `serve_owner_history` (SC1): a `secret` decides whether an earlier construct kept worker 0 busy while 8 keep-alive connections were accepted; the measured `within` over `public` work then runs beside request tasks on those connections. Closed by S4-D1: the two inputs' times do not differ.
- `serve_io_between_turns` (SC8): a request flood on worker 0's connections beside a `public` `within` on worker 0, with a `secret` deciding the phase of the root's turn at the fork. Closed: no difference.
- `serve_up_front` (SC6): a `secret` decides whether an earlier construct performed a `next`; the measured construct performs the first one. Closed: no difference.
- `serve_residue` (SC2) and `serve_delivery_history` (SC3): the differentials of the leaks before `L_srv`, whose check after the fix is U4001 from `una check`, as P9's and P10's are; the oracle runs the `public` twins only, as positive controls that the channel exists on the machine.

### 10.6 Scaling

Each doubling MUST take less than three times as long as the size before:

- engine, scripted sockets: 1,000 to 50,000 connections accepted and closed (peer table, slab), 10,000 to 100,000 reply charges under pressure (heaps, O(log c)), 10,000 to 100,000 timers armed and cancelled, 1,000 to 64,000 bytes of headers and 1 to 1,000 lines (linear parse), bodies in 1-byte chunks up to the limit;
- live sockets: 1,000 and 10,000 idle keep-alive connections, memory linear and no time growth per request; 50,000 for the release gates, `#[ignore]` with the reason of the gates.

## 11. Records this slice needs amended

The departures below change the text of ADR 0123, which is accepted, so each is proposed as a dated amendment appended to it, in a pull request of its own with status Proposed, before the code that relies on it merges:

1. **Rule 19, accepting.** One acceptor thread per run with one-shot readiness, in place of per-worker waits with `EPOLLEXCLUSIVE` or `EV_DISPATCH`; and the owner of a connection chosen by the accept order, `k mod N`, in place of the worker that accepts (S4-D1, SC1).
2. **Rule 19, the peer table** under sharded mutexes, touched per connection, not per request (S4-D4).
3. **Information flow, `L_srv`** (§9), joined into `τ` of every timing site of a package that can serve, and P8's static condition extended to `HttpServer` and `[serve]`, as the amendment of ADR 0116 frames its own rules.
4. **Rule 20, `--serve-threads`** on `una run` only (S4-D7).

## 12. `una-sys`

**No new binding.** The acceptor uses S7's `Poller::add` with the one-shot read interest, and std for `accept`, non-blocking mode, `TCP_NODELAY`, `shutdown` and peer addresses. The wake pairs are `UnixStream::pair` from std. Every libc function stays bound once: `poll` and `getrlimit` in `io.rs`, `setrlimit` in `server.rs`, `fcntl` once in `signals.rs`, `epoll_ctl` and `kevent` in `poll.rs`.

- **Counting rule.** Before and after any change to the crate: `find crates/una-sys/src -name '*.rs' -exec cat {} + | wc -l`, recorded in the commit message; any change that adds lines waits for the decision of the cap (below).
- **The count now.** 1,199 lines on S7, 1,382 after S3 was merged in, with the folds of the merge (one `poll`, one `getrlimit`, one conversion of `errno`, one `fcntl`). The crate is over its cap of 1,200 lines (ADR 0116 amendment of 2026-10-19). This slice adds none.

## 13. Implementation plan

Each sub-step is a commit, with fmt, clippy and the workspace's tests run in the check slot before it is pushed.

0. **Docs.** This note; the amendment of §11 as Proposed, in its own pull request.
1. **Restack** onto ADR 0117 S5 (#970) and, as they land, #1151 and #1148.
2. **Conformance first.** The script cases of §10.1, their harness and the zero-state assertions, failing.
3. **The wheel** moves to `una-http`, unchanged.
4. **The engine's state.** Counters, GCRA, the peer table, reserves, heaps, the handshake gate, delivery queues, with their unit and scaling tests.
5. **The engine's connections.** The state machine, timers, `Host`, the host's answers, the closing after a refusal, shutdown, against scripted sockets: the cases of step 2 pass.
6. **Native.** Pollers per worker, I/O between turns and while idle, the acceptor and the owner rule, the start before `main`, S7's socket cap less the sum of `connections`, the guards of §10.3.
7. **Interpreter.** Serve threads, `--serve-threads`, owned data across, one thread under `--virtual-time`.
8. **Live and scaling tests** of §10.4 and §10.6.
9. **Spec and changelog.** `spec/31 §6`, "Connections and serve threads", with rules 12, 14, 17, 18 (host side), 19 and 20 as they land; `changelog.d/1006-serve-threads.md`; `docs/security/tcb.md` lists the engine.
10. **Security review** of the engine before the pull request merges.

## 14. Dependencies

- **ADR 0117 S5 (#970):** the restack base, with #1145, #1146, the timing oracles' file and `rank.rs`.
- **#1151:** the wheel in nanoseconds with removal at once, which the engine's timers use (SC7).
- **#1148:** `L_res`, which `L_srv` joins.
- **#1059:** the TLS provider; until it is decided, `https` serving fails closed (§5.6).
- **ADR 0117 S6 (#971, `limiter` and `std.limit`): not needed.** The handshake rates and the delivery rate are limits of the grant (rule 7), kept by the host, of the kind of ADR 0117 rule 8, not the program's own `limiter` (rule 9). Their GCRA is §5.2's, with the period arithmetic shared with S5's governor. Nothing waits for S6, and nothing is done for it. A later wish to pace `[serve]` requests in code is `limiter`'s, after S6, and step 6's or later.
- **#1128:** `identity()` in `una-sys/src/server.rs` keeps its `geteuid` until #1128's `dir::effective_uid` lands (a TODO there names it).

## 15. Decisions

- **S4-D1** one acceptor thread per run; a connection's owner is `k mod N` by the accept order (§3; amendment §11.1).
- **S4-D2** S7's request-socket cap leaves the sum of `connections` to the server.
- **S4-D3** the engine runs between turns and while idle, with an event budget of 64, never inside a turn.
- **S4-D4** the peer table: 64 shards per grant under mutexes, with a random key per run.
- **S4-D5** reply pressure: indexed heaps per grant and per peer, O(log c), ties to the lower connection id.
- **S4-D6** delivery across workers through a ready list of workers, O(1).
- **S4-D7** `--serve-threads` on `una run` only; one serve thread under `--virtual-time`.
- **S4-D8** the serve threads start only in a run whose program can serve, before `main`; dormant until step 5.
- **S4-D9** `https` serving fails closed until #1059.
- **S4-D10** `L_srv` (§9), in the checker with step 5, after its amendment is accepted.

## 16. Open questions, with the answers this slice takes

1. **The `una-sys` cap.** The merged crate has 1,382 lines against a cap of 1,200; S3's note proposed 1,400. *Answer:* S4 adds no line (§12), so the cap amendment stays the S3 proposal; nothing of this slice waits for it, and any later `una-sys` change waits for it.
2. **Owner by accept order, or by the worker that accepts, as rule 19 says?** *Answer:* by accept order (S4-D1), since the other leaves a placement set by earlier code (SC1); proposed as an amendment.
3. **Atomic GCRA with the interval rounded up, or S5's exact `1/n` arithmetic under a mutex?** *Answer:* atomic, rounded up, which rule 7 allows and which only delivers fewer.
4. **A lock-free peer table?** *Answer:* no; sharded mutexes, touched per connection (S4-D4).
5. **Does accepting start before `main`?** *Answer:* yes, in a run that can serve (P8); requests that arrive before the program's first `next` wait in `waiting`, within `answer`.
6. **Does a server need ADR 0117 S6?** *Answer:* no (§14).
7. **May `L_srv` be avoided by a structural fix?** *Answer:* not in this slice. The serve threads' work shares the cores with every site, and its moments follow the program's hand-offs; separate threads would not change that (P9 counts helpers' work for the same reason). The label is the sound option now; P11's relaxation MAY narrow it later.
8. **What does an `https` grant do before #1059?** *Answer:* a run that can serve with one is refused before `main` (S4-D9); S3's behavior, binding with nothing accepting, stays for runs that cannot serve.
9. **`--serve-threads` on executables?** *Answer:* refused, since native code serves on `--workers` (S4-D7).
