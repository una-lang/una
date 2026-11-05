# Draft: ADR 0123, Amendment: the checked capability (slice S5, issue #1007)

- Status: Draft, for the project's decision. The accepted text of ADR 0123 does not change until the project accepts an amendment; `spec/31 §1` states what the implementation of the first part of slice S5 does meanwhile, and each point below is the most secure reading the records allow.
- Design: `docs/plan/0123-s5-design.md`.

## Why

Implementing ADR 0123 rules 0 to 5 and its Information flow in the checker met one conflict with another normative text and four points the record leaves open. Each is decided here the conservative way and recorded for acceptance.

## A. The field `query` of `Request`

ADR 0123 rule 1 declares `Request` with a field `query: internal List[Param]`. `spec/01` reserves `query` in every edition ("reserved for v0.4 (platform)", U0103), so a field of that name cannot be declared, and `request.query` could never be read. The reservation is older than the record, which states that it "adds no reserved or contextual keyword"; the clash went unnoticed.

**Decided in the implementation:** the field is `params: internal List[Param]`, with the same meaning, the query's parameters decoded in order. Rule 1 and its examples read `params` for `query` where they name the field; the trace's reduced form ("the values of query parameters, in `query` and in `target`") reads "in `params` and in `target`". The word `query` stays reserved.

## B. What `next` accepts

Rule 1 declares `accepts internal` on `reply` and, in its prose, `public` only on `close`; it writes no `accepts` on `next`. Without one, a call of `next` under any program counter would be accepted. Whether a task takes a request off the queue decides which task holds its reply right and takes a `/once` slot that other tasks' deliveries wait for, so a call under a `secret` program counter would let `secret` data decide what other tasks observe.

**Decided in the implementation:** `next` accepts `internal`. An accept loop runs under a `public` program counter (IF1), and a request task under an `internal` one (IF7), so no server of the record's examples changes; only a call of `next` under a `secret` condition is U4003.

## C. The function values of IF6

IF6 checks a `start` whose captures can hold a `ReplyRight`, by their types, "or include a function value, which may have captured one". Read alone, the second clause would check every `start` that captures a function value in every package of edition `0.4`, servers or not, and refuse ones under a `secret` program counter that hold no reply right and never could.

**Decided in the implementation:** the clause applies in a package that imports `HttpServer`, `Exchange` or `ReplyRight` from `std.host`, the only names through which a value of the package can hold a reply right (a `ReplyRight` comes only from `next`, and later from `test_exchange`, whose types name them). The first clause, a capture whose type can hold a `ReplyRight`, applies everywhere.

## D. U3033 through handlers

Rule 5 forbids an operation of `HttpServer` in the effect set of a branch. A branch that calls an operation of another capability, `@orders.find()`, runs whatever handler serves that binding, and a handler's parameters may hold a reply right, so two branches could still answer one right in an order that timing decides.

**Decided in the implementation:** an operation, in a branch, of a capability that has a handler in the package whose `uses` hold `HttpServer` (or such a capability, to a fixpoint) is U3033, whatever handler serves it at run time.

## E. `L_srv` with the preview's terms

The S4 amendment (IF-S1) defines `L_srv` with `sfx(s)`, the statements that can run after a hand-off site, and `Suf*`, its closure along the call graph. The checker of this slice computes `sfx(s) ⊔ Suf*(node(s))` as `τ⁰` of the bodies of every ancestor of the function that holds the site, the rule `L_lim` already uses (`spec/27 §9` rule 5), with the preview's `τ_conc`. It is sound and coarser: code that runs only before the first server operation raises the label too, which the amendment's section F.2 says it does not. Under the preview's `τ_conc`, a server of the language form has `L_srv = ⊤` in any case, since its `next` stands in a `tasks` block.

**Proposed:** keep this until #1155 (P11's `Suf`) or a statement-level `sfx` lands, as `spec/31 §1` rule 5 item 7 states it: a preview over-approximation that may become more precise and must not become stricter.

## F. Runs before the runtime

Slice S5 lands in two parts: the checks of this draft (#1007), then the operations in both engines (#1571) with the timing oracles that `spec/31 §6` requires before the serve threads meet a program. Between them a program can name `HttpServer` but not run it.

**Decided in the implementation:** `una run` and `una build` of a program whose entry function uses `HttpServer` are a usage error, exit code 2, before anything runs (`spec/31 §1` rule 10); `una check` and `una test`, whose tests install handlers with `given`, work as for any capability.

## G. Two readings added after review

- **`uses of` parameters in branches (rule 5).** A function that calls its `uses of` parameter in a branch runs code its callers choose, which the branch cannot see: in a program that imports a name of the server, such a call, or the passing on of the parameter, is U3033, failing closed.
- **Type parameters (IF6).** A capture of a type parameter's type counts as one that can hold a reply right only in a package that imports `HttpServer`, `Exchange` or `ReplyRight`, as for function values (C).

## H. The reduced trace form (#1571, part 3)

Rule 1 states the reduced form of the events of `next` and `reply` and that "their values are redacted by their labels, as every event's are". Four details are left to the implementation.

**Decided in the implementation** (`spec/31 §1` rule 7, `una_interp::serve_trace`, one implementation for both engines):

1. A header is a credential by its name in any letter case: a reply's `Set-Cookie` is one, as a request's `cookie` is (requests carry names in lower case already).
2. `<credential>` and `{"length": n, "hmac": "<hex>"}` are written bare in the value's Una form, as `<reply-right>` and `<redacted:level>` are, so they cannot be mistaken for a text of that content.
3. In `target`, which stays a quoted text, each value of a query part `name=value`, split at its first `=`, is replaced by the body form of the value as received, before its percent-decoding; a part without `=` is a name and stays, as names stay in `params`. In `params` the body form is of the decoded value.
4. The fail-closed rendering that the first part of the slice used until the reduced form landed, every argument of the host's `HttpServer` written as an `internal` value whatever its label, gives way to rule 1's own reading: redaction by labels, the reduced form beneath it. Since every field of `Request` and `Reply` is declared `internal`, a run's trace still shows no field of either; a `Reply` built from `public` data is written `Reply(status: <redacted:internal>, headers: <redacted:internal>, body: <redacted:internal>)` instead of `<redacted:internal>`, which shows nothing but its static label. A run without the engine keeps the fail-closed rendering.

The key is held by the engine of the run (`Port::trace_key`): all zeros under a script, and 32 bytes from the host's random source for the serve threads of a live run (part 4), where a host that gives none stops the run from starting, as for the placement key.

## I. `faults()` and the event of a contained fault (not implemented)

Decision 10 of the amendment of 2026-11-02 says "At S5 (#1007) the program reads the count of contained faults through `faults()`, and each contained fault is a trace event", and item 10 of the fault-policy draft (`docs/plan/0123-s4-faults-amendment-draft.md`) places both at `Shared::contain`. Neither text declares `faults()`, its type, its label, where it lives, or the event's form, and adding a name to `std.host` or an outcome kind to `spec/11 §5` without one is a change of the language (CLAUDE.md, "Scope guard"). The counts exist (`Shared::faults`, `GrantState::faults`), and the choke point passes its `fault::Site` already; nothing is shown to the program.

**Proposed, for the project's decision:**

1. `faults` is a fourth operation of `HttpServer`: `op faults() -> internal Int`, the run's count of contained faults, `accepts internal`, no failure, never parking. It is `internal` because the count depends on what peers send and when (IF5); an operation rather than a function, because it reads the host's state, so it is an event and is refused in a branch of `together` by U3033 as the other operations are. A `serve:` grant allows it with the others. The per-grant counts stay in the event of item 2.
2. Each contained fault appends a host event with no task and no operation, `{"seq": n, "outcome": {"kind": "fault", "site": "step" | "accept" | "peer-expiry" | "abandon", "listener": "<grant>"}}`, written at the program's next server operation after it (the events of serve threads are not in any task's order), before that operation's own event, so that a trace stays a function of a script under `--virtual-time`.
3. Until the project decides, contained faults stay counted and unseen, as `spec/31 §6` rule 13 item 5 states.

## J. The root's abort and `next` (`spec/31 §1` rule 3, Waiting)

The second part of the slice settled that an abort of the root does not lower the run's cancellation entry: the root unwinds through its `tasks` statements, which cancel their tasks. **Decided in the implementation:** a task waiting in `next` when the root aborts ends the operation `interrupted`, with its `cancelled` event, as a cancelled one does; only an abort by a started task ends a `next` with the run's abort. Both engines agree (`tests/serve_runs/aborted_root_ends_a_waiter`).
