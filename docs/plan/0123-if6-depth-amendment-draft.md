# Draft: ADR 0123, Amendment: the depth of a request task in IF6 (issue #1674)

- Status: Draft, for the project's decision. The accepted text of ADR 0123 does not change until the project accepts an amendment; `spec/31 §1` rule 5 item 6 states what the checker does meanwhile, which is the most secure reading the records allow.
- Found by: the security hunt of the serve engine, finding F1 (Medium, both engines).

## Why

IF6 checks the auto-answer of a `start` that moves reply rights "under `pc ⊔ τ(e)` when it has `within`, and under `pc` otherwise, against `internal`", and says that "a reply inside `e` already meets the same bound under ADR 0117's timing floor, so this rejects only a `start` whose expression never replies".

That leaves out how the task ends. ADR 0117 rule 7 makes a started task that runs past `--task-depth` end alone, and rule 4.5 answers its rights `500`. Whether a peer gets the program's reply or the host's `500` then depends on how deep `e` recursed. A `reply` reached after a recursion on `secret` data runs under a `public` or `internal` program counter, so it is accepted, and the recursion only computes `secret` values, so it is legal too. The hunt's probe, a task that recursed `key - body.len` deep, recovered a secret integer exactly by a binary search on the body's length, in 14 requests, in both engines.

`spec/07 §1` puts termination and stack exhaustion out of model, under termination-insensitive noninterference, which limits such a leak to about one bit per run because the run ends. A task that ends alone does not end the run: every request turns termination into a status the peer reads, as often as it likes. The model's reason for leaving termination out does not hold here.

## A. The rule

**Proposed text for IF6, added:** a `start e` that moves reply rights is also U4003 at the `start` when both:

1. **its task can push frames without a bound its code shows:** what `e` runs, along the call graph of ADR 0116 P1 with an edge from each policy to its `via` function, reaches a cycle (a function, a handler, a `via` or a lambda that can call itself again), a call of a function value or of a `uses of` parameter, or code whose calls the checker does not hold (a standard module other than the prelude, a dependency not checked with the package);
2. **it can hold data above `internal`:** the program counter at the `start`, what `e` reads from outside, binds or receives from an operation, or what any node it reaches declares in its signature, binds, receives or, in a lambda, captures. Symbolic labels of type parameters do not count, since the values of a type parameter come from the callers, which count them where they hold them.

The last sentence of IF6 ("a reply inside `e` already meets the same bound ...") is replaced by: "A reply inside `e` meets the bound of the program counter already; the depth of its task is bounded by the rule above."

**Why it is sound.** A frame is pushed only by a call of Una code. If item 2 fails, nothing the task runs holds data above `internal`, so whatever decides its depth is at most `internal`, which the answer accepts. If item 1 fails, the task's calls form no cycle, call no function value and stay in code the checker holds, so its depth is bounded by the longest chain of calls that code shows. The prelude's functions are host code in both engines (the interpreter's `Machine::prelude`, the runtime's `una_rt_*`), which push no frame of Una code but through a function argument, itself a call of a function value; this assumes that host code does not exhaust a task's stack on data of its own.

**What it does not cover yet.** With item 1 false the task still ends alone when the operator sets `--task-depth` below that longest chain, and which chain runs may depend on data above `internal`. The default `--task-depth` is 1000. Issue #1681 proposes that the engines refuse such a setting.

**What it refuses.** The rule does not ask whether the data decides the recursion: a task that recurses on the request's length and also reads a `secret` key for a policy is refused. That is the price of a rule that is simple enough to be clearly sound and linear. Two rounds of a precise label (what decides each call) were sound only after each premise was patched (a `via` checked without labels, the symbolic labels of generic bodies, a quadratic fallback), and were withdrawn. Issue #1682 records the precise rule as a later refinement, which MAY only accept more.

**Why not a runtime fix.** The host cannot make a depth-aborted task's `500` look like the reply the task would have sent, so a uniform auto-answer does not close the channel (the hunt's option 2). Leaving it out of model (option 3) would weaken a public guarantee, against the project's rule to choose the most secure option.

## B. A correction found on the way

The checker ran the default of `if error use` under the program counter from before the clause, not under the label that decides the failure. Its calls and effects were therefore checked as if they ran whatever the failure, a gap of `spec/07 §3` beyond this record. The implementation raises the program counter before the clause's context is taken. No accepted text changes for it.

## C. Out of scope: other effects after a depth abort

A task that ends alone also skips every effect it would have performed after the abort, which a public sink may see, and its `aborted` event and stderr line name it. Under `spec/07 §1` that is termination, out of model, as it is for an abort of the run. It is not observable by the peer through the reply right, which this amendment covers. Issue #1677 records it for a decision of the model.

## Cost

One graph of the package, one search for its cycles and two closures over its strongly connected components, O(V + E), at the first `start` that moves reply rights; then O(size of the seed of `e`) lookups per `start`. Recording what a node holds costs O(open sites + log n) per binding and per operation.
