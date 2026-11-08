# Draft: ADR 0117, Amendment: a task switch that work under a raised label does not move (issue #1684)

- Status: Draft, not accepted. The accepted text of ADR 0117 does not change until the project accepts an amendment. By its decision of 2026-11-06 the project chose, for issue #1684, option 2 of the card now, a conservative checker rule that refuses started tasks whose loop bounds depend on `secret` data, and this amendment later (section C), which stays a draft for a later decision. Section B shows that the rule as worded is not enough and proposes the shape a sound rule needs; that shape is **proposed, not decided**, and the choice of the exact rule is open.
- Implemented: section B's conservative rule, in the shape below with `internal` as its bound, in PR #1709 (issue #1684, `spec/27 §8` rule 5); section C stays a draft. The check points that data at `internal` decide, the residual of issue #1708, are listed in the report's `information.timing` as `task switch` (`spec/13 §6`); a rule at `public` and section C stay open for the project's decision.
- Origin: finding T1 of the security hunt of #973 (Medium, confirmed in both engines, on `main` only; no released line has `tasks`).
- Concerns: ADR 0117 rules 11 and 12 (the interpreter's schedule and native code under `--virtual-time`), `spec/27 §8` rules 2 and 3.

## A. The problem

In a run whose program has a `tasks` statement, the interpreter runs the root task and each started task on coroutines, one at a time, and the task that runs goes to the back of the queue once it "has passed 1024 check points in its turn while another task is ready" (`spec/27 §8` rule 2). Native code under `--virtual-time` follows the same schedule (rule 3). Check points are each call, each operation, each loop iteration and each element of a prelude function over a list.

So the number of check points a task passes decides when it yields, and with it the order in which the tasks' later effects happen. When that number depends on `secret` data, the order of later `public` prints does too. The hunt's probe, a 30-line program accepted today, runs a pure loop whose length a `secret` byte decides in a started task, beside a task that prints `public` lines; the interleaving of the prints spells the byte, exactly, under `una run` and under native `--virtual-time`, and native code with 8 workers shows it too.

`spec/27 §8` rule 1 puts "the order of real time across started tasks" out of model, as the order of effects across tasks is (`spec/07 §1`). The interpreter's schedule is not real time: it is a deterministic function of the check points each task passes, so the reason for leaving it out of model does not hold here, as for the run-wide iteration count of ADR 0116 S-36.

The number of check points depends on data above `public` in more ways than a loop's bound: a branch under a raised program counter whose two sides pass different numbers of check points, a loop whose condition or collection is labeled, a prelude function over a labeled list, and a recursion whose depth labeled data decides.

## B. The conservative rule, now (proposed shape for the decided option 2; the exact rule is open)

**What was decided.** Option 2 of the card: a conservative rule of the checker, with no change of the engines, that refuses started tasks whose loop bounds depend on `secret` data. Its spec text lands in `spec/27 §8` with its code and conformance cases (#1684).

**Why loop bounds are not enough.** A loop's bound is one way in which `secret` data decides how many check points a task passes, but not the only one. A branch on `secret` data whose two sides pass different numbers of check points does it too: a loop of `public` length whose body calls a function on one side of a `secret` branch passes a `secret` number of check points, and so moves the switch, with no labeled bound anywhere. So does a loop whose condition or collection is labeled, a prelude function over a labeled list, and a recursion whose depth labeled data decides. And the root task switches with the started tasks (`spec/27 §8` rule 2), so the root's code moves the switch as a started task's does. A rule that refuses only labeled loop bounds in started tasks leaves the channel open.

**What a sound rule must therefore cover.** Every way in which data above `public` can decide the number of check points that the root task or a started task passes. The shape proposed here, **for the project to confirm before #1684 implements it**, is:

In a package whose code holds a `tasks` statement, U4001 at each construct whose number of check points can depend on data above `public`, in code that the root task or a started task can run (the call graph of ADR 0116 P1):
1. a check point under a program counter above `public` (a call, an operation, a loop, or a prelude function over a list, inside a branch on labeled data);
2. a loop whose number of iterations a value above `public` can decide (its bound, its condition, the collection it walks);
3. a prelude function over a list whose label is above `public` (`map`, `filter`, `first_where`, `fold`);
4. a recursion, a cycle of the call graph, entered under a program counter above `public` or with an argument above `public`.

**Cost to programmers.** This is much more than option 2 as worded refuses. In a program with started tasks, code that branches on `secret` data must pass no check point inside the branch: no call, no operation, no loop. That refuses much code that is sound today in programs without `tasks`, and it is the reason for section C.

**Open.** The choice of the exact rule: option 2 as worded (labeled loop bounds in started tasks), which is not sound alone; the shape above; or a shape between them, such as items 1 to 4 for started tasks only, which leaves the root's switches open. Each refuses less than the one after it and closes less of the channel.

**Cost of the check.** One pass over the bodies that the program counter analysis visits already, plus the reach from the `tasks` statements and the cycles of the call graph, O(V + E) per package.

## C. The amendment of ADR 0117, later (draft, not accepted)

**Proposed:** the switch of rules 11 and 12 counts only the check points whose occurrence `public` data decides. Each check point that the checker places under a program counter above `public`, or inside a loop, a prelude function over a list or a recursion that section B's items 2 to 4 name, is marked in Core IR as not counted; the interpreter and native code under `--virtual-time` pass it without advancing the count of the turn. The moment a task yields is then a function of the `public` check points it passed, and the order of effects across tasks no longer depends on data above `public`.

Two alternatives were considered for the same rule: padding the count of a labeled region to a bound its code shows (not available for a loop whose bound is labeled), and switching at a labeled region's end only (a turn may then last as long as the region, the same fairness cost as below with more state).

**What it costs.**
- *Fairness.* A task that runs a long loop under a raised label does not yield during it, so other ready tasks wait until it ends or parks. Under `--virtual-time` this is a property of the schedule, not of the clock; under the real clock (rule 2) turns end by time, and that order is out of model already.
- *Core IR.* One bit per check point, set by the checker from the program counter it computes; the engines read it at each check point, O(1).
- *Equivalence.* Both engines read the same bit, so the equivalence tests of `spec/27 §8` rule 3 keep one schedule.

**What it relaxes.** Section B's rule is withdrawn: a program with started tasks may branch and loop on `secret` data again, as programs without `tasks` do.

**What it must keep (#1677).** Section B's rule also closes the depth channel of started tasks above `internal`: a recursion that data above `internal` decide can run a task past `--task-depth`, which then ends alone while the run goes on, so whether its later effects happen, and the order of other tasks' effects, would follow that data (`spec/27 §8` rule 5, `security/task_switch_recursion`). Uncounted check points do not close it, since the task still ends alone. The amendment MUST therefore keep a depth rule for started tasks when it withdraws section B's: for instance, refuse a recursion, a call of a function value or code the checker does not see, under a program counter above the bound, in a started task; or a depth floor over the task's effects. Otherwise #1677 reopens above `internal`. The same rule closes #1681: a request task with no recursion ends alone only under an operator's lowered `--task-depth`, and which chain of calls runs is a branch that section B's rule refuses above `internal` (`spec/31 §1`, `security/server_start_static_depth_secret`), so withdrawing it without a replacement reopens #1681 too. Both are tied to the decision on this amendment (DECISIONS-OPEN item 18). The precise item 2 of IF6's depth rule (#1682, `spec/31 §1`) reads the check points that section B's rule records (`Checker::switch_site`, in function, predicate and handler bodies), whether or not that rule reports them: an amendment that withdraws the rule MUST keep recording those sites with their labels, or #1682's item 2 silently accepts what the coarse test of #1674 refuses, and #1681 reopens.

**Open for the decision.** Whether a quiet region (`spec/26 §6` rule 4) counts as labeled for this rule, and whether the native real-clock schedule needs the same bit for a turn that ends inside a labeled region (its order is out of model today).
