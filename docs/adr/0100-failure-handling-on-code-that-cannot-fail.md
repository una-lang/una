# 0100. Failure handling on code that cannot fail

- Status: Accepted
- Date: 2026-10-13
- Deciders: Una project (accepted on 2026-10-13)
- Resolves: issue #602
- Target: edition `0.3` only (a preview, `spec/13 §7`). Editions `0.1` and `0.2` are unchanged: they accept the same programs as today. The rule only rejects, so it makes edition `0.3` stricter, never looser. The Gate 1 pin does not move.
- Implementation: after the edition `0.3` preview lands (#564).
- Amends: `spec/06 §3`, `spec/06 §5`, `spec/13 §4` (two codes)

## Context

`spec/06 §3` requires a handling clause on a statement whose expression can fail (U3007). `spec/06 §5` makes `attempt f(args)` turn the failure of "a single fallible call" into a `Result`. The specification is silent about the converse: a clause, or an `attempt`, on code that cannot fail. The checker accepts both without a word (probe with v0.3.8):

```una
double(n: Int) -> Int = n * 2

pub add_one(n: Int) -> Int
  fails E
=
  let x = n + 1
    if error use 0
  let y = double(x)
    if error pass
  y

pub tried(n: Int) -> Result[Int, Text] = attempt double(n)
```

All of it checks:

- The `if error use 0` and the `if error pass` never run.
- `tried` declares an error type, `Text`, that no run can produce: the error type of an `attempt` whose call cannot fail is left unconstrained, so any type is accepted.

A clause after `attempt` is accepted too, though `attempt` already turned the failure into a value.

**Why it matters.** A dead clause states a belief about failure that the code does not hold:

- A reader, human or agent, takes `if error fail Unavailable` to mean that the line can be unavailable.
- An agent that sees it accepted learns that clauses are harmless to add, and adds them by habit. Then a clause that matters looks no different from one that does not.
- The issue reports this as a common confusion, found while writing the per-construct docs (#585).
- A dead clause spreads. In a private function, a dead `if error fail Bad` is the failure from which its `fails` is inferred (`spec/04 §5` rule 3). Every caller must then handle a failure that cannot happen: `pub caller(n: Int) -> Int = helper(n)` is U3007, "`helper` can fail with `E`" (probe with v0.3.8).
- `attempt` of a call that cannot fail goes further: it gives a `Result` whose `Err` arm is unreachable, and an error type the program never produces.

## Decision

1. **A dead handling clause is an error in edition `0.3` (U3025).** An `if error` clause is dead when the statement's expression has no failure type, judged once the inferred kinds are final (`spec/04 §5` rule 3). The judgment reads the failure type itself, not whether U3007 would be reported: U3007 is not reported in some places, such as the lines of an example's `do`, and a clause there is judged the same way. The cases are:
   - an expression with no call or operation that can fail;
   - a call of a function whose `fails` is declared absent or inferred as absent;
   - an operation whose capability declares no `fails` for it;
   - any expression wrapped in `attempt`, whose failure is already a value.

   The diagnostic is at the `if error` keyword, and names the expression's construct. An `if none` clause on the same statement is not affected. A dead clause never runs, so its own content is not judged as failure handling. The U3008 at a dead `if error pass` is not reported, and neither is the U3009 at the variant of a dead `if error fail V`: U3025 is, at the same clause. Otherwise a dead `if error fail V` in code without `fails` would ask to declare `fails`, the opposite of the fix. Inference still reads the clause: a dead `fail V` records its failure as any other does, and gives a private function its inferred kind (`spec/04 §5` rule 3). Rule 4's root case depends on this.
2. **`attempt` of a call that cannot fail is an error in edition `0.3` (U3026).** The operand of `attempt` must be a call that can fail, in the sense of rule 1. The diagnostic is at `attempt`.
3. **What stays live.** A call whose failure is not known to be absent is never dead:
   - the call of a parameter of function type that declares `fails`, or of a parameter `p` under `fails of p` (ADR 0062), whose failure depends on the argument;
   - a call whose callee or kind is already in error. An error type decides nothing, so no second diagnostic follows a first;
   - a call of a poisoned function (below), judged as rule 4 says.

   **Poisoned functions.** A function, declared or inferred, is poisoned when:
   - it contains a reported U3007, U3008 or U3009 site. Fixing it may give the function a failure it does not have now: a call left unhandled (U3007) records no failure, and a `fail` or a pass in code without `fails` (U3009, U3008) has no failure type until `fails` is declared;
   - or it is in error: a private function inferred as "cannot fail" that a failure of error type, or an operation whose capability does not resolve, reaches through its own facts or a chain of passes. Its "cannot fail" rests on code already reported. The implementation marks an unresolved capability on its own: the inferred kinds of ADR 0099 do not tell an operation with no `fails` from one whose capability does not resolve;
   - or it mentions a poisoned function in any way other than a direct call handled by `if error use`, by `if error fail V` or by `attempt`. That covers a call with `if error pass` or with no handling clause, and a mention as a value: bound with `let`, passed as an argument, stored in a field, or given to a `fails of` parameter. A failure that reaches the function through a local, a field or a lambda follows the poisoned function's, which a name does not reveal.

   A direct call handled by `if error use`, by `if error fail V` or by `attempt` does not spread poisoning: the failure stops there. Poisoning is a search over the reversed reference graph, linear in the references.
4. **When they are judged.** The family is U3007, U3008, U3009, U3025 and U3026. U3025 and U3026 are judged by the first of these cases that applies:
   - **An error outside the family.** U3025 and U3026 are not reported, and neither are the U3008 and U3009 of dead clauses (rule 1). That error may have made a type an error or left a capability unresolved, in ways rule 3 does not enumerate. It also rejects the package, so nothing dropped here can lead to an acceptance.
   - **Family errors only.** U3025 and U3026 are judged only at plain sites:
     - an expression with no call or operation;
     - a direct call of a named function or an operation, where the callee is not poisoned, no argument is a function value (a lambda or a named function), and no argument contains a family site.

     A call of a function value (a parameter, a local, a field), or a call given a function-valued argument, is not judged. Its failure may come from code with a family error, which a name does not reveal: a lambda with an unhandled call infers no failure, and so does a poisoned function passed as a value or bound first with `let f = c`.
   - **No error.** U3025 and U3026 are judged at every site.

   U3025 and U3026 never count against each other. A package with one of each reports both and is rejected. A wording that suppressed each because the other exists would accept it: that is the fail-open case, and a test pins it.

   The family's own errors do not suppress the plain sites, and that matters:
   - A dead `if error fail V` in a private function gives it a kind of its own (see Context), so its callers report U3007. If those U3007 suppressed U3025, the dead clause would never be reported. The agent would be sent to add a clause at every caller, and `una fix` would never reach the root.
   - Instead U3025 is reported at the clause, a plain site, together with the callers' U3007. The helper contains no reported family site (its clause's own content is replaced by U3025, rule 1), so it is not poisoned. U3007 has no edit, so `una fix` deletes the dead clause and checks again, and the callers' U3007 are gone.
5. **Repairs.**
   - **U3025** has one repair: delete the clause, every line from `if error` to the end of the clause. The edit is exact under ADR 0096 rule 2: it is computed from the declaration's tokens, it leaves a file that parses, and it is deterministic. Its `escape_cost` is 0 and it widens nothing, so `una fix` applies it (ADR 0084). Deleting the clause changes no run, since the clause never runs.
   - **U3026** has one repair, with `edit: null`: "remove `attempt` and use the value of the call directly, then unwrap each use of the `Result`: a `when ... is Ok(v)` becomes the value `v`, its `is Err` arm goes, and a declared `Result[T, E]` becomes `T`". Removing `attempt` changes the expression's type, so the uses of the `Result` must change with it. That is a rewrite outside the expression, and a cascade is not an exact edit (ADR 0096 rule 2), so the repair describes the change and carries no edit.
6. **Editions.** Editions `0.1` and `0.2` keep today's behavior: no diagnostic, and for `attempt`, an unconstrained error type. `spec/06 §3` and `§5` state the two rules for edition `0.3` and later.

**Complexity.** Both judgments reuse the failure type the checker already computes for U3007, so each costs O(1) per statement and per `attempt`. U3025 and U3026 need the final kinds and every family site, so they are computed after the checking pass has reported U3007, U3008 and U3009. A post-pass reads the recorded clause and `attempt` sites. Poisoning is one search over the reversed reference graph. Both are linear in the size of the package.

## Options considered

For both shapes:

- **(A) An error in edition `0.3`, with the repairs of rule 5 (chosen).** It is the strictest choice, and it is confined to the preview, where a rule may still add errors (`spec/13 §7`). The repair of U3025 is applied by `una fix`, so the cost to an agent is one `una fix` round.
- **(B) A warning.** Una reports errors only (`spec/13 §1`: "v0.1 defines errors only"), and a warning would not change the exit code of `una check`, which is `1` only for an error (`spec/13 §5`). An agent's loop stops at a successful check, so a warning would be read rarely and acted on less. This option also adds a severity that the toolchain does not use today.
- **(C) A defined meaning, accepted silently.** A dead clause would be specified as allowed and without effect; `attempt` of a call that cannot fail would have a fixed error type (an empty kind). This settles the specification gap but keeps the confusion the issue describes. An empty error type is also a construct that Una does not have.
- **(D) An error in every edition.** Editions `0.1` and `0.2` promise to change what they accept only to fix a soundness or security error (`spec/13 §7`). A dead clause is neither: it is misleading, not unsound. Option (D) would break existing edition `0.2` programs for no safety gain.

For the repair of U3026:

- **An edit that removes `attempt` (not chosen).** `una fix` would apply it and then check again, so a wrong context surfaces as a diagnostic (ADR 0096). But each use of the `Result` would then fail to check: one error would become several, at sites the agent did not touch. A description is the honest repair here.

## Consequences

- Edition `0.3` programs carry handling clauses exactly where a failure can happen. A clause there is a fact about the code.
- U3007 and U3025 together make the clause a function of the code: one is required where the expression can fail, and the other forbids one where it cannot.
- `una fix` removes dead clauses. One removal can expose another: once a dead `if error fail V` goes, a private function whose `fails` was inferred from it may no longer fail. Its callers' clauses then become dead in the next round, which `una fix`'s rounds cover. Such a removal can also expose an `attempt` of that function (U3026), which needs the agent's edit. A clause whose body performs an operation, or calls a function that uses a capability, adds to the inferred `uses` of a private function. Deleting it can then narrow those `uses` too, which only removes authority.
- Tests:
  - conformance cases for each case of rules 1 to 4, in edition `0.3`, and unchanged in edition `0.2`;
  - a clause inside an example's `do`;
  - a broken callee whose caller's live clause is neither reported nor touched by `una fix`;
  - after the root error is fixed, no U3007 appears at that caller;
  - the root case: a dead `if error fail V` in a private helper whose callers report U3007. U3025 is reported at the clause together with the U3007, and after `una fix` no U3007 remains;
  - shape (a): a private `c` with an unhandled fallible call (U3007), and a caller's `if error` on `c()`. No U3025; after `una fix` and the U3007 repair in `c`, no U3007 appears at the caller;
  - shape (b): a `fail V`, or an `if error pass`, in a function without `fails` (U3009, U3008), and a caller's `if error` on it. No U3025; after `una fix` and the family repair (declaring `fails`), no U3007 appears at the caller;
  - a dead `if error fail V` in a `pub` function without `fails`: U3025, and no U3009;
  - a call that takes a failing function value where the expected type drops its failure (U3009), with a clause on that call: no U3025;
  - a package with exactly one U3025 and one U3026: both are reported, and it is rejected;
  - a lambda argument with an unhandled fallible call (U3007), as in `xs.map(each: (x) -> ...)`, and an `if error` on that call: no U3025;
  - `let f = c` then `f(x)` with an `if error`, where `c` is poisoned: no U3025;
  - a private `f` that does `let h = c` then passes the failure of `h(x)`, where `c` is poisoned, and a caller's `if error` on `f(n)`: no U3025, since `f` is poisoned through the reference;
  - a dead `if error fail V` together with an unrelated error (U2001): neither U3025 nor the clause's U3009 is reported, and the check exits with `1`.
- Two codes join `spec/13 §4`: U3025 and U3026, after the codes that ADRs 0092 to 0094 assign. Each code gets an explain text.
- The formal track is unaffected: the rule only rejects programs.
- Until this ADR is accepted and implemented, the docs describe today's behavior and link to #602.
