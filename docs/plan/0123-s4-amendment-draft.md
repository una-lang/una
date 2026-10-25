# Draft: ADR 0123, Amendment (2026-10-25): the serve threads

- Status: Proposed. A draft for issue #1006 (ADR 0123 S4), kept here until the project decides it. Once accepted, it is appended to `docs/adr/0123-http-servers.md` as a dated amendment, and the accepted text above it stays unchanged.
- Design: `docs/plan/0123-s4-design.md`, whose §10.2 holds the soundness argument in full.

## Why

ADR 0123 rule 19 puts the server's work on the workers of ADR 0117, and has the worker that accepts a connection own it. Under ADR 0116's amendment of 2026-10-25, a precise `τ_conc`, that leaves two problems. The server's I/O then runs inside the turns of the workers, where its moments, set by the program's earlier hand-offs, shift the tasks beside every site, and the only label that covers it is P10's over every ancestor of a server operation, in effect `τ⁰` of the whole program. And which worker accepts depends on how busy each worker was, so the placement of request tasks carries the history of earlier code. This amendment moves the server to threads of its own, places connections by a keyed hash of their addresses, and labels what remains.

## A. Rule 19, native code

1. **Serve threads.** Dedicated serve threads serve the connections, `--serve-threads` of them (rule 20), and never the workers of ADR 0117 rule 2, which run tasks only. A serve thread runs no Una code. It has its own `kqueue` or `epoll`, owns the connections placed on it, and does their TLS, reading, parsing and writing.
2. **Accepting.** Each grant keeps one listening socket, without `SO_REUSEPORT`. Every listener is registered one-shot in one accept poller of the run, which is itself registered one-shot in each serve thread's poller; the thread that receives a listener's event accepts at most 16 connections and registers the listener again, so one thread accepts from a listener at a time. This replaces `EPOLLEXCLUSIVE` and `EV_DISPATCH`, and needs no new code with `unsafe`.
3. **Placement of connections.** A connection belongs to serve thread `H(listener, peer address, peer port) mod M`, with `M` the serve threads and `H` SipHash-2-4 under a 128-bit key of the run from the operating system's random source. The key is used for nothing else, and appears in no trace, report, message or error. It replaces "the worker that accepts a connection owns it". A run whose host gives no random bytes does not start (U6009).
4. **Placement of tasks.** ADR 0123 Appendix A, amendment of ADR 0117 rule 2 ("a started task whose captures hold a reply right runs on the worker that owns the reply right's connection") is withdrawn. Every task is placed by ADR 0116 P6.
5. **The `[serve]` path.** A request is read on its serve thread, handled in a task on a worker, and written on its serve thread: two hand-offs between threads per request, as for the language form. "With no hand-off of the request between threads" no longer holds.
6. **What is shared, and the locks.** Rule 19's atomic counters stay, without a lock per request. Three departures are declared:
   - the peer table is 64 shards of hash maps under mutexes per grant, locked at accept, close and expiry only; a connection holds its peer's entry by reference, so a request's shares are atomics;
   - a connection's pending reply sits under a mutex of that connection, uncontended except under reply pressure;
   - under reply pressure only, a mutex per grant keeps the heaps that choose the slowest connections.
7. **Reply pressure, bounded.** The thread whose reservation takes a grant over `replies` frees its victims' pending bytes at once, before it allocates its own reply; the victims' serve threads only close the descriptors later. Rule 13's worst case gains: the replies being reserved at one moment, at most one per worker, and, with TLS, the buffers of `rustls`, bounded per connection.
8. **Descriptors.** Before `main`, the run raises its soft limit to the sum of `connections`, plus one per listener, one for the accept poller, three per serve thread, 256 for the program's own `Http` requests and 64 for the rest of the runtime; a run that cannot is a usage error that names the number.

## B. Rule 20, the interpreter and the flag

`--serve-threads <n>`, from 1 to 256, applies to `una run` and to executables. Its default is the number of cores in the interpreter, and half the cores, at least 1, in native code. Under `--virtual-time` both engines use one serve thread. A run under `--virtual-time` that holds a `serve:` grant without `--serve-script` is a usage error before `main`.

## C. Information flow: the label of the serve threads

IF8. **`L_srv`.** The hand-off sites of a server are the calls of `next`, `reply` and `close`, each `start` whose captures can hold a `ReplyRight` or a function value, and the driver of `[serve]`. For a call site `c` in a body, `sfx(c)` is `τ⁰` of the statements that can run at or after `c` (at each level of nesting, the statement that holds `c` and every statement after it in its block, a loop around `c` whole), and `Suf*` joins `sfx` along every edge of ADR 0116 P1 that leads to a node, from the roots. Then

    L_srv = internal ⊔ ⨆ over hand-off sites s of ( sfx(s) ⊔ Suf*(node(s)) ⊔ τ_conc(s) ) ⊔ L_res

and `L_srv` joins `τ` of every timing site of a package whose program can perform an operation of `HttpServer`. It is `public` elsewhere. P11's `Suf`, once issue #1155 lands, MAY replace `sfx`.

- **Why it is sound.** The serve threads run no Una code, and what they do is a function of the request stream, of the program's hand-offs and of timers the two arm. The stream is input relative to the program's steps (Question 4) and `internal`. Every step between a hand-off and a later timing site is in the hand-off's suffix, or beside the site (`τ_conc`), or inside it (`τ⁰`). So the serve threads' work, and its moments relative to the site, are the same in two runs that differ only above the site's label. What remains is how the host shares its cores, caches and allocator between the serve threads and the workers, which ADR 0116 P12 already puts out of model, as for any program whose timing other threads of the host share.
- **Placement and turns.** P6 places every task, and no server work runs inside a worker's turn, so P7 is unchanged. P8's static condition gains an operation of `HttpServer` or a `[serve]` table: the serve threads, their pollers and the accept poller start before `main`.
- **Timers** of connections may fire up to 1 ms late where `epoll_wait` rounds its timeout; that phase is covered by `L_srv`.

**IF5, addition.** Which request a call of `next` receives depends on the timing of the serve threads and of every task that calls `next`, background tasks included, not only on the handlers. It is part of the request stream's dependence on the server's timing, out of model as Question 4 decided.

## D. Rule 22, the report

The `out_of_model` list gains, for packages with a `serve:` grant or a `[serve]` table: "which request a call of `next` receives when several serve threads hold requests, which the timing of every task of the run and of the serve threads decides".

## E. Questions for the project

1. **Dedicated serve threads (recommended), or the engine on the workers?**
   - **Cost of dedicated threads:** two hand-offs between threads per request on every path, about a microsecond each at saturation, and more threads than cores by default; rule 19's `[serve]` path no longer stays on one core.
   - **Cost of the engine on the workers:** a label of every timing site at `τ⁰` of the whole program, since every ancestor of a server operation counts; in a program that reads `secret` data anywhere, every `within` and `first of` becomes `secret`, and rule 15's `start ... within` and rule 16's `[serve] within` are refused in any server that touches `secret` data.
2. **The cost of `L_srv` to programmers** (with dedicated threads).
   - Every timing site of a server is at least `internal`, as IF7 makes request tasks already.
   - When code that can run after a server operation reads `secret` data (a handler, or code after the accept loop), `L_srv` is `secret`: inside the server's `tasks` statement this adds nothing to P2, which already gives its sites the statement's `κ`; outside it, an `on stop` or a later construct with a `within` or a `first of` becomes `secret` too.
   - Code that runs only before the first server operation, such as set-up that loads keys or computes on `secret` data, does not raise it.

   Recommended: accept, with P11's `Suf` (#1155) as the later relaxation.

## F. The measurement condition

Before ADR 0123's performance step, the project measures, on a quiet machine in release form: connections accepted per second, CPU time per request at saturation, the 50th and 99th percentile latency under uniform and skewed handler costs, and memory per idle connection, for (i) this design, (ii) a prototype with the engine on the workers and per-worker accept, and (iii) the `hyper` on `tokio` baseline of ADR 0123 Costs. When (i) is above the parity bar against (iii), CPU time per request at saturation above 1.1 times, and (ii) is within it, a further amendment weighs that gap against the label that (ii) needs.
