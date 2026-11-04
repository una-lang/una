# Draft: ADR 0116, Amendment: the iteration budget of rule 19 is per loop instance

- Status: Accepted by the project on 2026-11-03; its text is now ADR 0116's amendment of 2026-11-03, at the end of `docs/adr/0116-long-running-programs.md`, which governs. This draft is kept as the record of the proposal.
- Issue: #1472 (S-36). Found by the review of the formal model of ADR 0116 (#946, PR #1470), question 5.
- Implementation: the pull request that closes #1472 implements this draft in both engines and in native code, with `spec/26 §5` rule 7, §6 rules 1 and 8, §8 rule 1, `spec/13 §5` and `spec/08 §2`, under the rule that security fixes are made without waiting (the fix never weakens a guarantee). If the project rejects the draft, that pull request is the change to revert.

## The problem

Rule 19 says of `stop after N`: "A host stop request then arrives once `N` iterations of `for ever` loops have started in the run, counted in the order the interpreter starts them", and gives `--stop-after-iterations <n>` "the same meaning". Rule 14 makes every host stop request sticky. Together they make the stop point of each loop depend on how many iterations every earlier loop of the run ran.

A `for ever` under a `secret` branch is legal (IF4: its body runs under the enclosing program counter; `stop` there is legal under IF2), so how many iterations it runs is `secret`. Under the run-wide count it moves where every later `public` loop stops, and the output reveals the secret. The reproducer of #1472 prints `public loop ran 8 - (s + 1) iterations` under `--stop-after-iterations 8` for a secret `s` of 0, 2 or 5: the whole value up to `N` in one terminating run, not the one bit that a termination-insensitive guarantee concedes. A secret `Bool` that guards a loop only the host stops gives 0 public iterations against 4.

This contradicts the ADR's own reasoning. "Where this sits in the model" states T6 for schedules "that agree at every loop instance ... an observer at `l` can see", and says "Deadlines are not inputs: when one passes depends on the program's own speed". A schedule computed from the program's own iteration count is not an input of the host either: two runs that differ only in `secret` data get different schedules at a `public` loop, so T6 says nothing of them. `spec/26 §6` rule 8 called `--stop-after-iterations` "a fixed stop schedule", which it was not.

No released version is affected: `for ever`, the driver, `stop after N` and `--stop-after-iterations` exist only in edition `0.4`, which only `main` builds; `release/v0.3` (v0.3.17) and `release/v0.1` do not have the flag. In editions `0.1` to `0.3` no loop counts, so the old behaviour is not observable there.

## The amendment

Rule 19, the item **`stop after N`**, reads instead:

> **`stop after N`.** A test (`spec/08 §2`) MAY end with the line `stop after N`, `N` an integer literal of at least 1. It gives each **loop instance** of the test's run a budget of `N` iterations: each entry into a `for ever`, and the driver's loop, counts its own iteration starts from 0, and that instance ends at the start of its iteration `N + 1`, as at a stop point after a host stop request (rule 14 item 2): it appends its `stop` event, and then its `on stop` runs. The budget is not a host stop request: the end of one instance's budget interrupts no operation (rule 14 item 1), does not reach any other loop, a loop entered later included, and starts no grace period (rule 16). `una run` and executables take `--stop-after-iterations <n>` with the same meaning, so the equivalence tests (`spec/16 §5`) can run long-running programs.

Rule 14 drops "in tests, the `stop after` line (rule 19)" from its list of host stop requests, and the definition "Host stop request" at the head of the ADR drops "or the `stop after` line of a test".

"Where this sits in the model" gains one sentence after "T6 holds for every fixed schedule.":

> The iteration budget of rule 19 is such a schedule: the stop point of each loop instance is its iteration `N + 1`, the same for every instance and every entry, whatever the run did elsewhere.

## Why this option

The review named two fixes. This draft takes the first.

1. **A budget per loop instance (chosen).** It is the shape ADR 0116 already states ("for each loop instance, the iteration at whose start it stops") and the shape the formal model proves (`Host.stopAt site i`, `i` counted from 0 at each entry, the same at every entry of a site, PR #1470). A secret loop can still run its own budget out, but that ends only itself, which ran under the raised program counter already. Cost: O(1) per iteration start, one counter per loop instance on the stack of the code that runs it, so threads no longer share an atomic count (the compare-exchange loop of the run-wide count goes away).
2. **Keep the run-wide count and reject, or label to `⊤`, a `for ever` reachable under a raised program counter when the flag is in use.** Rejected: a flag of the host would change which programs are accepted, or the labels of their bindings, and the check could not see the flag at `una check` time. It also keeps a schedule that depends on scheduling across tasks, which the equivalence tests cannot fix.

Behaviour that changes, in edition `0.4` only:

- A sequence of loops under `--stop-after-iterations n` runs `n` iterations in each loop, not `n` in all; a loop entered again runs `n` again at each entry. A program that relied on the old count stops later. The budget bounds each loop instance, not the run: a `for ever` body can start a task that runs another `for ever`, so the number of instances, and with it the work of a run, can grow with the depth of started tasks. `--deadline` and the host's stop requests bound the run. A run-wide cap that stops loops must not return: the program can observe it, which is the leak of S-36. ADR 0117 rule 7, which calls the budget a stop request, is amended in the same way.
- The end of a budget no longer interrupts a waiting `Http` operation of another task's loop, and no longer starts the grace period, so an `on stop` that waits on such an operation is no longer cut by the hard stop after a budget. SIGINT, SIGTERM and `--deadline` keep rules 14 and 16 unchanged: they stay sticky host stop requests.

## Open points, outside this amendment

- The `stop` event of a loop under a raised program counter (review of #946, the first Medium) is still emitted only on the path that entered that loop, and the trace gives it no label. This amendment does not change it: it is the separate finding that the model drops the `stop` event, to settle with #946.
- `spec/26 §9` item 6, added by PR #1470, lists "schedules that count iterations across loops, as `stop after N` and `--stop-after-iterations` do" as outside the model. Under this amendment these flags give a schedule of the modelled shape, so whichever of the two pull requests lands second updates that item: the iteration budget is inside the model, and only schedules whose stop points differ between entries of one loop (a sticky request that ends a later loop at its first iteration) stay outside.
