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
