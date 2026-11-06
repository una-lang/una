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
2. what decides a call is the program counter at it, joined with the left operands of the `and`, `or` and `implies` whose right operand holds it;
3. `τ⁰(e)` (ADR 0116 IF5) as well, when `e` reaches code whose calls the checked package does not show with its own labels: a standard function or one of a dependency not checked with the package, a generic function, a function that applies a function argument such as `map`, a handler checked apart, or `Λ` in a package where a generic function, a dependency checked apart, or a standard function whose result is of a function type could make a lambda;
4. never `τ_conc`: each task has its own stack, so the work beside it does not change its depth.

The last sentence of IF6 ("a reply inside `e` already meets the same bound ...") is replaced by: "A reply inside `e` meets the bound of the program counter already; the depth label is what it does not cover."

**Why this label is sound.** The depth a task reaches, and whether it passes `--task-depth` or its stack, is fixed by which calls run, in which nesting: a frame is pushed only by a call. A call runs, or not, by the conditions around it, which raise the program counter, and by the left operand of a short-circuit operator. A function's parameters bound by their declared types what its conditions read (a flow above a parameter's label is U4001); `fail` and `if error pass` need a `public` program counter (`spec/07 §3` rule 3), so no failure skips a call on labeled data; a loop runs its body under the label of what it visits; and a lambda's body is checked where it is written, with its captures' labels. Two runs that differ only in data above `δ(e)` therefore make the same calls, in the same nesting, and reach the same depth. Item 3 falls back to `τ⁰(e)`, which bounds every step of `e`, wherever one of these premises is not visible to the checker.

**Why not `τ⁰(e)` alone.** It is sound too, and simpler, but it joins the labels of every value `e` reads and every operation's answer, whether or not anything branches on them. It would refuse a request task that reads a `secret` balance and replies with what a policy releases, the pattern IF7 describes and `tests/conformance/security/server_policy_reply` keeps. The depth label refuses only a task whose calls `secret` data decides.

**Why not a runtime fix.** The host cannot make a depth-aborted task's `500` look like the reply the task would have sent, so a uniform auto-answer does not close the channel (the hunt's option 2). Leaving it out of model (option 3) would weaken a public guarantee, against the project's rule to choose the most secure option.

## B. Out of scope: other effects after a depth abort

A task that ends alone also skips every effect it would have performed after the abort, which a public sink may see, and its `aborted` event and stderr line name it. Under `spec/07 §1` that is termination, out of model, as it is for an abort of the run. It is not observable by the peer through the reply right, which this amendment covers. A follow-up issue records it for a decision of the model.

## Cost

One call graph and one closure of `δ` over its strongly connected components per package, O(V + E), built at the first `start` that moves reply rights; then O(size of the seed of `e`) lookups per `start`, and `τ⁰(e)` only where item 3 applies.
