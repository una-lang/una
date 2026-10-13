# 0072. Arguments that move into the callee

- Status: Accepted
- Date: 2026-10-07
- Deciders: the coordinator under the owner's delegation (2026-10-07), pending Leonardo's review; proposal by Claude (Una-2)
- Resolves: issue #262 (pre-tag review v0.2.0, area 3d, finding 3d-2)
- Amends: `spec/16 §4` rule 7, `spec/17 §7` rule 3

## Context

A list built through a call is copied at every call, in both engines. `push(edit acc, i)` in a loop, or `acc = pushed(acc, i)`, takes O(n) per append and O(n²) for the loop. Measured on v0.2.0 (debug build), 10,000 appends become 40,000:

| Shape | Interpreter | Native (with the build) |
| --- | --- | --- |
| `push_one(edit a, i: i)` | 1.2 s to 28.5 s | 6.7 s to 76.8 s |
| `a = pushed(a, i: i)` | 1.2 s to 18.4 s | 3.5 s to 46.0 s |

The native runtime reports one list copy per append.

The cause is the calling convention. `spec/16 §4` rule 7 says a value passed to a function is never moved, and native calls borrow every argument. So the caller's binding still holds the list during the call, and the callee's `append` sees two references and copies. The interpreter does the same: the caller's environment keeps the value. Before this ADR, only the prelude's `append` and `set` took a list over at its last use (#30, #201).

The language already allows the update in place. `spec/17 §7` rule 3 lets a backend update a uniquely held value in place, since that is not observable. An `edit` argument is the only use of its binding in the call (`spec/17 §3` rule 4), and the binding takes the callee's final value when the call returns. A binding at its last use is never read again. What the language does not allow is losing the caller's value when the call fails. By `spec/17 §7` rule 1, a call that fails leaves every `edit` argument unchanged, and the code that runs after the failure may read it: a handling clause, or what follows an `attempt`.

## Decision

**D1. Which arguments move.** An argument that is a binding moves into the callee when nothing the caller runs afterwards reads that binding: neither the code after the call, nor the code its failure reaches (a handling clause, or what follows an `attempt`). That covers an `edit` argument, and a binding at its last use, such as the `acc` of `acc = pushed(acc, i)`.
- Any other argument is borrowed, as before. The callee then copies the value when it updates it, so the caller keeps its value.
- This keeps `spec/17 §7` rule 1 exactly: a binding that the failure path reads never moves.
- A binding passed twice in one call does not move: the callee would see one parameter change under the other.

**D2. Native calling convention: owned parameters with borrow inference**, as Lean 4 does.
- **Owned.** A parameter of a function of the package is owned when the function consumes it. A consumed name reaches one of these:
  - the list of `append` or `set`;
  - the text a concatenation begins with (ADR 0070 X10);
  - an owned parameter of another function;
  - the initial state of a loop whose body consumes the state;
  - a binding that aliases one of these.

  The owned parameters of mutually recursive functions are a least fixpoint over the package.
- **Borrowed.** Every other parameter is borrowed, with no reference counting at the call. That is the common read-only case, and it keeps today's cost.
- **At a call,** for each owned parameter:
  - an argument that moves by D1 leaves the caller's scope;
  - any other argument is retained.
- **In the callee,** an owned parameter belongs to the body's scope. The body releases it at the end, on success and on failure, or moves it at its last use, like a `let` binding.
- **Not affected.** Closures, a handler's operations and the type descriptors of `spec/20 §7` keep borrowed parameters.

**D3. Interpreter.** The interpreter applies the same moves. A binding that moves by D1 gives up its value before the call, so the callee holds the only reference, and `Rc::make_mut` updates in place. No inference is needed: an interpreted callee does not release its parameters.

**Complexity.** Appending through any number of call levels takes amortized O(1) per append, in both engines: O(n) for n appends instead of O(n²).
- Space stays O(n), and nothing is copied when no alias exists.
- The inference costs one linear pass over the package per change of the fixpoint, so at most one pass per parameter. In practice it takes two or three passes.

## Consequences

- Building a list or a text through helper functions is linear. That is the common shape for an agent: a `render(edit out)` or `push_row(acc, row)` called in a loop.
- `spec/16 §4` rule 7 no longer lists "a value passed to a function" among the values never moved, and names arguments among the updates in place. Its text-append line now says that #261 implemented it.
- A read-only parameter costs nothing more. A consumed parameter costs one retain at a call that still needs its argument, which was a full copy before.
- The ABI between functions of one package changes. It is internal: executables are built whole, and nothing links against a function of the package from outside.
- Deferred:
  - moving a value into a closure's parameter, or into a handler's operation;
  - moving out of a nested handled block, where the binding is owned by an enclosing scope. Those still copy, as before.

## Alternatives considered

- **Every parameter owned.** Simple, but every read-only call would pay a retain and a release per argument. Borrow inference gives the best average constant factor, as Perceus (Koka) and Lean show.
- **A flag per call telling the callee whether it owns each argument.** That costs a branch per parameter in every callee, and the callee would need both code paths. Rejected.
- **Only `take` and `edit` parameters owned.** The convention is then visible in the signature, but `acc = pushed(acc, i)` stays quadratic unless the author writes `take`. The language should not make a performance trap the default.
- **Copy-on-write in the callee, with no move.** The caller's reference is what forces the copy, so this changes nothing.

## Amendment of 2026-10-07: function values (N-42)

Decided by the coordinator under the owner's delegation (N-42), pending Leonardo's review; found by the in-place review of v0.3.0 (#311, IP-1 and IP-2), fixed by #312.

"Not affected: closures" (D2) left a call through a function value copying a list or a text at every call, in both engines: `acc = f(acc, i)` with `f` a named function passed as a value, or a lambda, and `fold` with a list or text state natively. It is replaced by:

1. **A lambda owns every parameter.** Its body moves each one at its last use or releases it at its end, as a function's body does with its owned parameters.
2. **A function used as a value is the lambda that calls it** (ADR 0037 rule 1). That lambda is the wrapper the convention needs: its call moves the arguments into the function's owned parameters, and releases those the function borrows. So every function value has the same convention, whatever its target.
3. **A call through a function value** moves each argument at its last use, by D1, and retains it otherwise, as a call of a function does with an owned parameter. Borrow inference (D2) counts it as consuming every argument.
4. **The prelude.** `fold` moves its state into the step, its initial state included when it is at its last use and is not the list folded over; `map`, `filter`, `first_where` and `fold` retain each item they give the function.
5. **The interpreter** moves a binding at its last use into a call through a function value, as into a call of a function (D3).

**Complexity.** Appending through any number of function values and higher-order layers is amortized O(1) per append, in both engines: O(n) for n appends, in O(n) space, copying nothing that no other name holds. A call through a function value whose target only borrows costs at most one retain at the caller and one release in the lambda, a constant per call; a call whose argument moves costs nothing more. A `fold` into a text that starts from a literal copies that literal once.

IP-3, `acc = f(acc) if error use acc`, still copies at each call: the failure path reads `acc`, so it never moves (D1). The guide gives its linear form.

## Implementation note (2026-10-13): the borrowed entry (#284)

Approved by the coordinator (N-248). D2 left a cost: a call that still needs its argument retains it for an owned parameter, and the callee releases it, even when the path that consumes it never runs. `cons(xs, i) = if i < 0 then xs.append(item: i).len else xs.len`, called 10^7 times with `xs` live, took 28% longer than before this ADR.

1. **Two entries.** A function with an owned parameter may have a second entry, compiled from the same body as if every parameter were borrowed. Each entry is compiled only when a call uses it, so a function has at most two, and the code stays linear in the size of the package.
2. **The choice at a direct call.** When no argument of an owned parameter moves (each would be retained: a binding still live, a literal, a borrowed word, or a word given twice), the call takes the borrowed entry and gives nothing. Otherwise it takes the owned entry, and D2 applies unchanged: arguments at their last use move, the others are retained.
3. **Why it costs nothing elsewhere.** On the borrowed entry, a path that consumes the parameter copies it, as the copying runtime entry does for any borrowed word. On the owned entry with a retained argument, that path copied too, since the caller's reference was still there. The copies are the same; only the retain and release of the calls that consume nothing are gone. A call that moves takes the owned entry, so appending through calls stays amortized O(1) per append. A value copied on the borrowed entry is owned, and moves into the calls after it.
4. **Not affected.** Function values, the lambdas that wrap a named function (N-42), and the interpreter (D3), which counts no references.

This is not the rejected "flag per call": nothing is decided at run time, and no callee branches on how it was called.

**Complexity.** Choosing an entry costs O(parameters) per call site at compile time. Run time changes by a constant per call, and no copy is added.
