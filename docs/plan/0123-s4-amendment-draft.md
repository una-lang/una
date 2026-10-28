# Draft: ADR 0123, Amendment (2026-10-25): the serve threads

- Status: Proposed. A draft for issue #1006 (ADR 0123 S4), kept here until the project decides it. Once accepted, it is appended to `docs/adr/0123-http-servers.md` as a dated amendment, and the accepted text above it stays unchanged.
- Design: `docs/plan/0123-s4-design.md`, whose §10.2 holds the soundness argument in full.

## Why

ADR 0123 rule 19 puts the server's work on the workers of ADR 0117, and has the worker that accepts a connection own it. Under ADR 0116's amendment of 2026-10-25, a precise `τ_conc`, that leaves two problems. The server's I/O then runs inside the turns of the workers, where its moments, set by the program's earlier hand-offs, shift the tasks beside every site, and the only label that covers it is P10's over every ancestor of a server operation, in effect `τ⁰` of the whole program. And which worker accepts depends on how busy each worker was, so the placement of request tasks carries the history of earlier code. This amendment moves the server to threads of its own, begins accepting only when the program first serves, places connections by a keyed hash of their addresses, and labels what remains.

## A. A decision on the request stream

**The stream's phase is input.** Question 4 made the request stream input, with its dependence on the server's timing out of model. This amendment decides how that reads for timing: the bytes peers send, and their timing relative to the program's own steps, are input, labeled `internal`. Two runs compared under T6 receive the same stream relative to their steps.

## B. Rule 19, native code

1. **Serve threads.** Dedicated serve threads serve the connections, `--serve-threads` of them (rule 20), and never the workers of ADR 0117 rule 2, which run tasks only. A serve thread runs no Una code. It has its own `kqueue` or `epoll`, owns the connections placed on it, and does their TLS, reading, parsing and writing.
2. **When accepting begins.** The serve threads and their pollers start before `main` (ADR 0116 P8, whose static condition gains an operation of `HttpServer` or a `[serve]` table). No listener is registered, and nothing is accepted, until the run's first server operation (a call of `next`, `reply` or `close`, a `start` that moves a reply right, or the start of the `[serve]` driver), which sets this once. Before it, a peer's connection waits in the kernel's backlog, the network's. A `close` that comes first never begins accepting.
3. **Accepting.** Each grant keeps one listening socket, without `SO_REUSEPORT`. Every listener is registered one-shot in one accept poller of the run, which is itself registered one-shot in each serve thread's poller; the thread that receives a listener's event accepts at most 16 connections and registers the listener again, guarded by a flag so that one thread re-arms it at a time. A slot of `connections` is taken before each `accept`, so every accepted descriptor, one closed at once included, holds one. This replaces `EPOLLEXCLUSIVE` and `EV_DISPATCH`, and needs no new code with `unsafe`.
4. **Placement of connections.** A connection belongs to serve thread `H(listener, peer address, peer port) mod M`, with `M` the serve threads and `H` SipHash-2-4, with a domain tag, under a 128-bit key of the run from the operating system's random source. The key serves only this placement and the peer table's shards, under distinct tags, and appears in no trace, report, message or error. It replaces "the worker that accepts a connection owns it".
5. **Placement of tasks.** ADR 0123 Appendix A, amendment of ADR 0117 rule 2 ("a started task whose captures hold a reply right runs on the worker that owns the reply right's connection") is withdrawn. Every task is placed by ADR 0116 P6.
6. **The `[serve]` path.** A request is read on its serve thread, handled in a task on a worker, and written on its serve thread: two hand-offs between threads per request, as for the language form. "With no hand-off of the request between threads" no longer holds.
7. **What is shared, and the locks.** Rule 19's atomic counters stay, and a request's shares are atomics. The locks a request or a delivery may meet are declared:
   - the queue of requests waiting on each serve thread, under a mutex of that thread, held for O(1), taken by the thread and by `next`;
   - the list of tasks waiting in `next` for delivery, under a mutex per grant, taken only on the waiting path;
   - a connection's pending reply, under a mutex of that connection, uncontended except under reply pressure;
   - under reply pressure only, a mutex per grant that keeps the heaps choosing the slowest connections, taken by every reservation while pressure lasts;
   - the peer table, 64 shards of hash maps under mutexes per grant, locked at accept, close and expiry only.
8. **Reply pressure, bounded.** A pending reply is held in segments of at most 16 KiB, each dropped, and uncounted, as soon as it is written, so the counters of `replies` and `peer_replies` equal the reply bytes allocated. The thread whose reservation takes a grant over `replies` frees its victims' segments at once, before it allocates its own; the victims' serve threads only close the descriptors later. Rule 13's worst case gains: the replies being reserved at one moment, at most one per worker, and, with TLS, the buffers of `rustls`, bounded per connection. `reply` has no check point.
9. **Descriptors.** Before `main`, the run raises its soft limit, within the hard limit, to the sum of `connections`, plus one per listener, one for the accept poller, three per serve thread, 256 for the program's own `Http` requests, the `Files` helpers' open files (`--file-threads`, or 1 in the interpreter), and 64 for the rest of the runtime. Every socket of the program's `Http` counts against its 256, the I/O pool's fallback included.
10. **The `Costs` expectation.** ADR 0123 Costs expects the `[serve]` path "within the bar ... whose structure is that of a thread-per-core Rust server, less the shared counters". With two hand-offs per request it now reads: within the bar is the aim, to be confirmed by the measurement of section G; the hand-offs are the cost to measure, besides the shared counters.

## C. Rule 20, the interpreter and the flag

`--serve-threads <n>`, from 1 to 256, applies to `una run` and to executables. Its default is the number of cores in the interpreter, and half the cores, at least 1, in native code. `--workers` and its default do not change. Under `--virtual-time` both engines use one serve thread. A run under `--virtual-time` that holds a `serve:` grant without `--serve-script` is a usage error before `main`.

## D. Information flow

IF-S1. **The label of the serve threads, `L_srv`.** (Named so as not to collide with ADR 0116's IF8.)
- **Hand-off sites:** the calls of `next`, `reply` and `close`; each `start` whose captures can hold a `ReplyRight` or a function value; the driver of `[serve]`.
- **`sfx(c)`,** for a call site `c` in a body: `τ⁰` of the statements that can run at or after `c`: at each level of nesting, the statement that holds `c` and every statement after it in its block; a loop around `c` whole; the handling clauses of those statements and of every enclosing statement; the `on stop` block of every enclosing `for ever`; and everything these call.
- **`Suf*`** joins `sfx` along every edge of ADR 0116 P1 that leads to a node, from the roots: direct calls, `start` and the parts of constructs, calls through `Λ` and `uses of` parameters, operations through `K(c)` to every handler, and calls inside lambdas and handlers' operations.
- **The label:**

      L_srv = internal ⊔ ⨆ over hand-off sites s of ( sfx(s) ⊔ Suf*(node(s)) ⊔ τ_conc(s) ) ⊔ L_res

  `L_srv` joins `τ` of every timing site of a package whose program can perform an operation of `HttpServer`, and is `public` elsewhere. P11's `Suf`, once issue #1155 lands, MAY replace `sfx`.
- **Why it is sound.** Before the first hand-off, the serve threads hold no connection and do no work, so their state then is fixed, as P10's "every bucket starts full". From it on, they run no Una code, and what they do is a function of the request stream (input relative to the program's steps, section A, `internal`), of the program's hand-offs, and of timers the two arm. Every step between a hand-off and a later timing site is in the hand-off's suffix, or beside the site (`τ_conc`), or inside it (`τ⁰`). So the serve threads' work, and its moments relative to the site, are the same in two runs that differ only above the site's label.
- **The machine.** Given that, what remains is how the host shares its cores, caches and allocator between the serve threads and the workers. ADR 0116 P12 puts this out of model, as for any program whose timing other threads of the host share. P12 applies only because nothing is accepted before the first hand-off: no serve work exists whose moments set-up code could shift.
- **Placement and turns.** P6 places every task, and no server work runs inside a worker's turn, so P7 is unchanged.
- **Timers** of connections may fire up to 1 ms late where `epoll_wait` rounds its timeout; that phase is covered by `L_srv`.

**IF5, addition.** Which request a call of `next` receives depends on the timing of the serve threads and of every task that calls `next`, background tasks included, not only on the handlers. It is part of the request stream's dependence on the server's timing, out of model as Question 4 decided.

## E. Rule 22, Diagnostics, and `spec`

- **Rule 22.** The `out_of_model` list gains, for packages with a `serve:` grant or a `[serve]` table: "which request a call of `next` receives when several serve threads hold requests, which the timing of every task of the run and of the serve threads decides".
- **Diagnostics.** In ADR 0123's list of usage errors, "a descriptor limit below `connections` plus 64 that the run cannot raise" reads "a descriptor limit below the target of rule 19 (section B, item 9) that the run cannot raise"; the list gains "`--virtual-time` with a `serve:` grant and no `--serve-script`".
- **U6009** extends its scope: it is also reported at the start of a run that can serve when the host's random source gives no bytes for the placement key, before `main` and before any effect. `spec/13 §4` and its paragraph on U6009 in §6 change with it.
- **`spec/31 §2` rule 7, item 1** reads: "raises the run's limit of open descriptors to the target of `spec/31 §6`, within the hard limit; a run that cannot is a usage error that names the target", and `spec/31 §6` states the target of section B, item 9.

## F. Questions for the project

1. **Dedicated serve threads (recommended), or the engine on the workers?**
   - **Cost of dedicated threads:** two hand-offs between threads per request on every path, about a microsecond each at saturation, and about one and a half threads per core by default in native code; rule 19's `[serve]` path no longer stays on one core.
   - **Cost of the engine on the workers:** a label of every timing site at `τ⁰` of the whole program, since every ancestor of a server operation counts; in a program that reads `secret` data anywhere, every `within` and `first of` becomes `secret`, and rule 15's `start ... within` and rule 16's `[serve] within` are refused in any server that touches `secret` data.
2. **The cost of `L_srv` to programmers** (with dedicated threads).
   - Every timing site of a server is at least `internal`, as IF7 makes request tasks already.
   - When code that can run after a server operation reads `secret` data (a handler, code after the accept loop, a handling clause or an `on stop` block), `L_srv` is `secret`: inside the server's `tasks` statement this adds nothing to P2, which already gives its sites the statement's `κ`; outside it, a later construct with a `within` or a `first of` becomes `secret` too.
   - Code that runs only before the first server operation, such as set-up that loads keys or computes on `secret` data, does not raise it.
   - Peers that connect before the first server operation wait in the kernel's backlog, at most `min(connections, 65535)` per grant, until the program serves.

   Recommended: accept, with P11's `Suf` (#1155) as the later relaxation.

## G. The measurement condition

Before ADR 0123's performance step, the project measures, on a quiet machine in release form: connections accepted per second and the cost of each listener event (up to `M` wakes and `2M` `epoll_ctl` calls), CPU time per request at saturation, the 50th and 99th percentile latency under uniform and skewed handler costs, and memory per idle connection, for (i) this design, (ii) a prototype with the engine on the workers and per-worker accept, and (iii) the `hyper` on `tokio` baseline of ADR 0123 Costs. When (i) is above the parity bar against (iii), CPU time per request at saturation above 1.1 times, and (ii) is within it, a further amendment weighs that gap against the label that (ii) needs.
