# Draft: ADR 0123, Amendment: the depth of a request task in IF6 (issue #1674)

- Status: Draft, for the project's decision. The accepted text of ADR 0123 does not change until the project accepts an amendment; `spec/31 §1` rule 5 item 6 states what the checker does meanwhile, which is the most secure reading the records allow.
- Found by: the security hunt of the serve engine, finding F1 (Medium, both engines).

## Why

IF6 checks the auto-answer of a `start` that moves reply rights "under `pc ⊔ τ(e)` when it has `within`, and under `pc` otherwise, against `internal`", and says that "a reply inside `e` already meets the same bound under ADR 0117's timing floor, so this rejects only a `start` whose expression never replies".

That leaves out how the task ends. ADR 0117 rule 7 makes a started task that runs past `--task-depth` end alone, and rule 4.5 answers its rights `500`. Whether a peer gets the program's reply or the host's `500` then depends on how deep `e` recursed. A `reply` reached after a recursion on `secret` data runs under a `public` or `internal` program counter, so it is accepted, and the recursion only computes `secret` values, so it is legal too. The hunt's probe, a task that recursed `key - body.len` deep, recovered a secret integer exactly by a binary search on the body's length, in 14 requests, in both engines.

`spec/07 §1` puts termination and stack exhaustion out of model, under termination-insensitive noninterference, which limits such a leak to about one bit per run because the run ends. A task that ends alone does not end the run: every request turns termination into a status the peer reads, as often as it likes. The model's reason for leaving termination out does not hold here.

## A. The depth label

**Proposed text for IF6:** a `start e` that moves reply rights is checked as a `reply` under `pc ⊔ δ(e)`, joined with `τ(e)` when it has `within`, against `internal` (U4003 at the `start`), where `δ(e)`, the depth label, bounds what decides which calls `e` makes:

1. the join of what decides each call, operation and install that `e` can run: in `e`, in the functions and handlers it reaches along the call graph of ADR 0116 P1, with an edge from each policy to its `via` function, and in the lambdas (`Λ`) that a call of a function value can run;
2. what decides a call is the program counter at it, joined with the left operands of the `and`, `or` and `implies` whose right operand holds it, with the label of the function value at a call of one (T-App, `spec/07 §3` rule 4), and, at a call of a prelude function that applies a function argument such as `map`, with the deep labels of its other arguments, which decide how often the lambda runs;
3. `τ⁰(e)` (ADR 0116 IF5) as well, when `e` reaches code whose calls the checked package does not show with its own labels: a function of a standard module other than the prelude or of a dependency not checked with the package, a generic function of the package, a handler checked apart, a function that applies a function argument whose code is not known at the call, or `Λ` in a package where a generic function, a dependency checked apart, or a standard function whose result is of a function type could make a lambda;
4. never `τ_conc`: each task has its own stack, so the work beside it does not change its depth.

The prelude's functions do not bring item 3: both engines run each as host code (the interpreter's `Machine::prelude`, the runtime's `una_rt_*` intrinsics), which calls Una code only through a function argument, so a call of one pushes no frame that data could multiply. This assumes that host code does not exhaust a task's stack on data of its own; the engines check the depth and the stack budget at each call of Una code.

The last sentence of IF6 ("a reply inside `e` already meets the same bound ...") is replaced by: "A reply inside `e` meets the bound of the program counter already; the depth label is what it does not cover."

**Why this label is sound.** The depth a task reaches, and whether it passes `--task-depth` or its stack, is fixed by which calls of Una code run, in which nesting: a frame is pushed only by such a call. A call runs, or not, by the conditions around it, which raise the program counter, by the left operand of a short-circuit operator, by the function value it calls, which chooses the code, and, inside a lambda that `map` and its kin apply, by the list they walk. A function's parameters bound by their declared types what its conditions read (a flow above a parameter's label is U4001); `fail` and `if error pass` need a `public` program counter (`spec/07 §3` rule 3), so no failure passed on skips a call on labeled data; the default of `if error use` runs under the label that decides the failure; a loop runs its body under the label of what it visits; and a lambda's body is checked where it is written, with its captures' labels, while which lambda runs is the value's label. Two runs that differ only in data above `δ(e)` therefore make the same calls, in the same nesting, and reach the same depth. Item 3 falls back to `τ⁰(e)`, which bounds every step of `e`, wherever one of these premises is not visible to the checker.

**A correction found on the way.** The checker ran the default of `if error use` under the program counter from before the clause, not under the label that decides the failure. Its calls and effects were therefore checked as if they ran whatever the failure, a gap of `spec/07 §3` beyond this record. The implementation raises the program counter before the clause's context is taken, so the default's effects, and its calls for the depth label, see that label. No accepted text changes for it.

**What the fallback refuses.** `τ⁰(e)` joins every value `e` reads, whether or not anything branches on it. So a request task that reads `secret` data and reaches a function of a standard module other than the prelude, a generic function of the package or a dependency checked apart is refused even when no `secret` value decides a call. The message says that the checker cannot see what decides the depth. Making those precise needs their bodies' calls in the findings, a later refinement that MAY only accept more.

**Why not `τ⁰(e)` alone.** It is sound too, and simpler, but it would refuse every request task that reads a `secret` balance and replies with what a policy releases, the pattern IF7 describes and `tests/conformance/security/server_policy_reply` keeps, and every such task that calls a prelude function. The depth label refuses those only when `secret` data decides a call, or when the fallback above applies.

**Why not a runtime fix.** The host cannot make a depth-aborted task's `500` look like the reply the task would have sent, so a uniform auto-answer does not close the channel (the hunt's option 2). Leaving it out of model (option 3) would weaken a public guarantee, against the project's rule to choose the most secure option.

## B. Out of scope: other effects after a depth abort

A task that ends alone also skips every effect it would have performed after the abort, which a public sink may see, and its `aborted` event and stderr line name it. Under `spec/07 §1` that is termination, out of model, as it is for an abort of the run. It is not observable by the peer through the reply right, which this amendment covers. A follow-up issue records it for a decision of the model.

## Cost

One call graph and one closure of `δ` over its strongly connected components per package, O(V + E), built at the first `start` that moves reply rights; then O(size of the seed of `e`) lookups per `start`, and `τ⁰(e)` only where item 3 applies. Recording what decides a call costs O(open sites + log n) per call site.
