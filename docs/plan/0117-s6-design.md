# ADR 0117 slice S6: `std.limit` (design note)

- Issue: #971 (part of #896, ADR 0117 rule 9, IF5, Verification, Costs, Implementation plan step 5)
- Base: slice S5 (#970, `docs/plan/0117-s5-design.md`), on S4 (#969), S3 (#968), S2 (#967) and S1 (#966)
- Normative text: `spec/27 §10`, `spec/10 §11` rule 10; decisions in the ADR 0117 amendment of 2026-10-27 on slice S6

This note fixes how S6 is built, and lists the decisions it takes where the ADRs leave a choice. Each decision takes the most secure option the ADRs allow.

## 1. What S6 delivers

1. The standard module `std.limit`, in edition `0.4`: the capability `Limit` with `op acquire() -> Unit fails Limited accepts internal`, its failure kind `Limited = | RateLimited`, and the handler `limiter(at_most: Int, per: Int of Millisecond, waiting: Int) for Limit`, which the runtime provides (rule 9).
2. Its enforcement in both engines through the host layer of slice S5: one bucket per installation, shared by every task that uses the binding, started tasks included.
3. The obligations `at_most >= 1`, `per >= 1` and `waiting >= 0` at the arguments of each installation (Verification).
4. IF5: U4001 at a `with` binding, or a `given` line, whose statement's timing label does not flow to `internal`.

Not in S6: the reactor (step 6), which moves the native wait off the helper; a precise `τ_conc` (ADR 0116 IF6), which would accept a `limiter` installed inside a task construct.

## 2. The module

- **Read only when imported** (decision S6-D1). `std.limit` joins `std.collections` as a module the checker loads only for a package of edition `0.4` that imports it (ADR 0119 rule 0): the names pass lists its exports (`Limit`, `Limited`, `limiter`), the type environment registers its source after `std.collections`, and `standard_kinds` puts its kind and its capability in Core IR, in the module `std.limit`. So no other package's checks or Core IR change, and the `una core` snapshots of the suite are untouched.
- **The handler's source** (decision S6-D2). The module's source declares `limiter` with a placeholder body, `op acquire() = unit`, so that the checker resolves `with @x = limiter(...)` by the one rule for handlers (`spec/05 §9` rule 4, U3010), checks its arguments against its parameters, records it as a handler of `Limit` for the timing labels, and resolves `@x.acquire()` as an operation of `Limit`. The elaboration of standard modules keeps only kinds and capabilities, so Core IR holds no handler declaration, and each engine recognizes the installation by its path, `std.limit.limiter`, before looking the handler up. The interpreter's walker for hash tables enters no body for it. An engine that met a `Decl::Handler` of it would have no code to run; none can.
- **The constants.** `una_core::LIMIT_MODULE` and `una_core::LIMITER` name the path in every crate that recognizes it.

## 3. Enforcement (`una-http`, `limit.rs`)

- **The governor's slot, generalized** (decision S6-D3). The per-grant state of slice S5 (`Slot`) keeps its rate as a count and a period in nanoseconds rather than a `Period` word, so a `limiter`'s `per`, any number of milliseconds, is a rate like `per second` is; GCRA's units of `1/n` nanoseconds keep `T/n` exact for every count. A grant's slot is built from its `Limits` as before.
- **`Limiter`** is a governor of one slot with a rate, no count in flight, and `waiting` as its queue, over the run's clock. `acquire` takes a permit over that slot and drops it at once: no slot is held, so nothing waits for a holder (rule 8, No deadlock, item 2). Everything else is the governor's: the ticket queue, the full-queue failure, the deadline on the wall clock and on the run's clock, the host stop and the abandon, and the move of the virtual clock to a refill or to the covering deadline.
- **Saturation.** The obligations prove `at_most >= 1`, `per >= 1` and `waiting >= 0`. A count the engines receive beyond what a slot holds saturates (`u64`, and `u32` for the queue), so a larger count never wraps into a smaller limit (decision S6-D4).

## 4. The interpreter

- A third kind of instance, `Instance::Limiter`, holds the bucket (`Rc<Limiter>`) and the id, `with:limiter#<n>` or `given:<name>`, numbered as a handler's. `install` makes it from the arguments on the run's clock; `serve` routes `acquire` to it.
- `acquire` waits as a request does at a limit (slice S5): the covering deadline is the earliest of the `within`s in progress and the `for ever within` iteration, passed as `until` on the run's clock and as the wall-clock deadline under the real clock; a host stop interrupts it in the body of a `for ever`. Nothing abandons it: the interpreter runs one task at a time.
- **A cut wait fails with `RateLimited`** (decision S6-D5). `Limited` has one variant, and rule 9 makes the wait interruptible: past the covering deadline, or on a host stop, no token is taken and the call fails with `RateLimited`. The `for ever within` iteration records its time-out at the operation, as for a request; a `within` observes its own deadline at the next check point or when its expression returns (`spec/27 §5`).

## 5. Native code

- `una_rt_limiter_instance(at_most, per, waiting)` makes an instance with no operation code, as a host instance, so an operation on it takes the runtime's path; the bucket is kept run-wide in the state the tasks share, and the instance holds its index as its one cell, so a copy of the instance on another worker shares the bucket (decision S6-D6). Its id counts as a handler's.
- `una_rt_limit_acquire(site, instance)` is lowered as a host operation of the module `std.limit`: it waits on a helper of the I/O pool while the task parks (slice S5, decision S5-D6), with the deadlines, the host stop and the abandon of a request's wait; an abandoned wait ends `interrupted` and stops the task through the sited path. A cut wait fails with `RateLimited` as in the interpreter, and a `for ever within` iteration records its time-out.
- The embedded description names `Limited` of `std.limit` as the failure kind of the operation, so a native trace renders it.

## 6. The checker

- **The timing seed** (IF5). A statement with a `with` binding that installs `limiter` opens a timing seed before its bindings are checked, so the handlers they install count, as the seed of `e within d` does, and closes it after the statement's expression; one `LimiterSite` per such binding carries the seed. An example whose `given` line installs it opens the seed over the `do` line, with every `given` handler recorded; the `do` line is placed, as `main` is, since a test runs alone and nothing runs beside its root (a task or a lambda inside it stays unplaced by its own sites). `authority::limiter_timing` computes `τ` with every handler of each capability counted (`HostHandlers::All`) and the preview's `τ_conc`, and reports U4001 at the binding, or the `given` line, when the label does not flow to `internal`, naming the first source that raised it.
- **Unplaced installations** (decision S6-D7). A statement checked inside a task, a lambda or the block of a `tasks` statement is unplaced, so its `τ_conc` is `⊤` and the installation is U4001 until `τ_conc` becomes precise: the safe side of IF5. The ADR's own form, a `limiter` installed around the call of the function that holds the `tasks` statement, is placed and accepted when that statement is timed by `internal` data at most.
- **The obligations.** `install` records a `refinement` obligation at each argument of `limiter`; the verifier generates `a >= 1`, `p >= 1` and `w >= 0` for the installation's path, as it generates the bound of `tasks`, since Core IR declares no handler.

## 7. Costs

| Operation | Time | Space |
| --- | --- | --- |
| An installation | O(1): one slot | one bucket, and natively one entry of the run's table |
| A call served at once | O(1) under the bucket's lock | none |
| A call that waits | O(1) to join the queue, O(1) amortized to leave it | one ticket, and one parker |
| The checker | one timing label per installation, O(functions + call edges + operations + handlers) each | the sites |

A package that does not import `std.limit` pays nothing: the module is not read, and no Core IR changes.

## 8. Tests

- Conformance (`tests/conformance`, `ita.una` packages, edition `0.4`): `runtime/limiter_acquire` (a burst, a refill, and a full queue with `waiting: 0`, both engines), `runtime/limiter_tasks` (three started tasks share three tokens, the block finds none), `security/limiter_secret` (U4001 at the binding), `security/limiter_internal_task` (accepted: `acquire` under an `internal` program counter in a started task, the `limiter` around the function that holds the `tasks` statement), `verification/limiter_bounds` (U5001 at `at_most: 0` and `waiting: -1`), `names/limit_0_3` (U1001 in edition `0.3`).
- Unit (`una-http`, `limit.rs`): a limiter's burst, refill and a deadline that comes first; a full queue; under a virtual clock, five calls at 0, 0, 500, 1000 and 1500 ms in no wall time, and a call cut by its deadline on the run's clock; a wait ended by a host stop and by an abandon.
- Both engines (`crates/una-cli/tests/limiter.rs`): under `--virtual-time`, five calls at two per second print the same `Clock.now` readings in both engines, with the same trace, five `acquire` events under `with:limiter#1`, in no wall time; and a `given` line installs `limiter` for an example whose `elapse` refills the bucket.
