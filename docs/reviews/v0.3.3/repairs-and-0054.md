# Deferred reviews: ADR 0054, the U0103 rename and the precondition repair

- **Issues:** #124 (ADR 0054, PR #117), #125 (the U0103 rename edit, PR #118) and #126 (a function value's precondition repair, PR #119). The owner deferred these three reviews on 2026-10-02.
- **Reviewer:** Una-1. **Code read on:** main `e2cf23f`.
- **Binary:** the release build of main `9b9af67`. The two later merges, #387 and #388, touch none of the code under review.
- **Method:**
  - Read the current code: `una-verify/src/run.rs` for ADR 0054, `una-syntax/src/lexer.rs` for the rename, and the repair text of the verifier diagnostics.
  - Run small packages at tier 1 with Z3 at the version in `verification.lock`.
  - For each repair, apply its text literally and check the result again.
- **Probes:** the packages are under `repairs-and-0054/`.
- **Question asked:**
  - Can the re-solve of ADR 0054 turn `refuted` or `unknown` into `proven`, or report a counterexample that is no execution?
  - Can a repair, applied mechanically, weaken a contract or change a capability or a label?

## Verdicts

| Item | Verdict | Findings |
| --- | --- | --- |
| #124, ADR 0054 | **Sound.** No path gives `proven` from the second query. | V-1 (minor): a counterexample can divide by zero |
| #125, the U0103 rename | **Not clean.** Applying the rename across two modules can bind a call to a different function. | R-1 (moderate), R-2 (minor) |
| #126, the precondition repair | **Safe.** No repair weakens a contract or changes authority, but one repair does not check once applied. | P-1 (minor) |

## #124: ADR 0054, solve again with the arithmetic interpreted

### What holds

- **Never `proven`.** `outcome` uses the second answer only when it is `refuted` and its model agrees (`run.rs`, `outcome`). In every other case it keeps the first answer, a refutation that disagrees, which becomes `unknown` with the `UNINTERPRETED` reason.
  - A second query that is unsatisfiable therefore gives `unknown`, as rule 1 says.
  - Probe `solve_again`: `a * a` with `ensures result >= 0` and `a / b` with `ensures result <= a` (where `a >= 0`) are both U5002, not proven.
- **Agreement is checked, never assumed.** `agrees` takes every application of `i!mul!Int`, `i!div!Int` and `i!mod!Int` in the assertions and compares it with exact arithmetic in 128 bits.
  - Products use `checked_mul`, and quotients and remainders are Euclidean.
  - A value that is not an integer, overflows, or comes from an application inside a definition (a `reflect` predicate) disagrees, which gives `unknown`.
- **The second query is a different query.** `interpreted` replaces the logic and the three declarations by `define-fun`s with the exact strings `smt.rs` prints, and its cache key is computed from its own text. If a replacement ever missed, the query would be the first one again, refuted with a disagreeing model, so `unknown`. That fails safe.
- **Text cannot reach the script.** Text literals are encoded as `i!lit!<hex>`, and the namespace test forbids user names under `i!`. So no program text can contain a declaration line that the replacement would rewrite.
- **Real counterexamples come back.** `a * a` as `Pos` is U5001 with `a = 0, result = 0`, and `if b == 0 then 0 else a / b` with `ensures result != 3` is U5001 with `a = 3, b = 1, result = 3`. Both are executions.

### V-1, minor: a refutation's counterexample can divide by zero

Rule 2 lets any value agree for a zero divisor. In probe `solve_again`, `divided(a, b) = a / b` with `ensures result == 7` is refuted with the counterexample `a = -1, b = 0, result = 0`. No execution gives that: at `b = 0` the program fails at the division.

- **Not unsound.** The program is rejected anyway: its division obligation `b != 0` is refuted on the same path, as the ADR expects. A path that reaches a division with a zero divisor also reaches that division's own obligation. Its assumptions are a prefix of the later obligation's, so a model with a zero divisor on its path refutes the division obligation too.
- **Misleading for agents.** The postcondition diagnostic shows the counterexample as an execution, and its help says to "add a guard or an arm so that the failing path returns a value for which `result == 7` holds". An agent that follows it works on a path that cannot run.
- **Suggested fix** (an amendment to ADR 0054, since the ADR is accepted): when the model's path divides by zero, keep the status but replace the counterexample by a note that the path divides by zero, pointing at the division's own diagnostic. Alternatively, report the obligation as `unknown` with that reason.

### Observation

- The first-iteration retry of ADR 0071 D3 does not get the second query of ADR 0054. A retry whose model disagrees stays `unknown`. This loses only completeness.

## #125: the U0103 rename

### What holds

- **Within one file, the rename preserves meaning.**
  - The lexer chooses one name per word after reading the whole file, skips every name the file already has, and gives the same name to every occurrence.
  - Text contents are not tokens, so they are not renamed. Interpolations are tokens, so they are renamed with the rest.
  - Probe `rename_one_file`:
    - `capability shared` becomes `Shared`, its use `shared[put]` becomes `Shared[put]`, and the operation parameter and the labeled argument `table:` both become `the_table_2`, since the file has `the_table`;
    - applied, the file checks clean (`rename_one_file_applied`).
- **Labels cannot change.** Categories are `upper_ident` (`spec/02`), and every word reserved in edition 0.1 is lower case.
  - So no category is ever renamed.
  - No capitalized new name, such as `Shared`, `Type` or `Var`, is a prelude type or capability.
- **Contracts stay the same.** A rename inside `requires`, `ensures` or `where` renames the binder and its uses alike.

### R-1, moderate: across modules, the rename can bind a call to a different function

PR #118 states the known limit: "the name is unique within the file". The consequence is worse than a name that fails to resolve. Probe `rename_cross_module`:

```una
module a

--- A different function, which already has the name.
pub the_table(x: Int) -> Int = x + 100

pub table(x: Int) -> Int = x
```

```una
module b

import a (table)

pub main() -> Unit
  uses @console: Console[print]
=
  let n = table(1)
  @console.print("{n}")
```

- In `a`, the file has `the_table`, so `table` becomes `the_table_2`.
- In `b`, the file has no clash, so the call `table(1)` becomes `the_table(1)`.
- The import line gets advice instead of an edit (R-2), so the agent must rename that name by hand. The name the rename chose for every other occurrence in the file is `the_table`.
- With that rename, the package checks clean at tier 1 and prints `101`. The program as written meant `a.table(1)`, which is `1`.

So `b` now calls a different function with the same signature. That function can have:
- other contracts, so the call is verified against them;
- other labels, so flows change;
- other `uses`, in which case the call fails to check unless `b` already declares them.

This is a capability or label change that the review was asked to rule out, reached through one mechanical step.

**Suggested fix:** choose the name of a `pub` declaration and of an imported name across the package, not per file. Every file that names it must get the same new name, one that no file in that set already uses. Where the lexer cannot see other files, the checker can.

The smallest safe alternative:
- for a name that is imported, or `pub` and used from another module, give advice without an edit;
- or rename it to a name that no module of the package declares.

### R-2, minor: an imported name gets the module-path advice

`reserved` in `lexer.rs` treats every word on a line that starts with `module` or `import` as part of a module path. So in `import a (table)`, the imported name `table` gets the advice "rename the module so that no part of its path is `table`, and move its file to match". It gets no edit.
- That advice is wrong: `table` is not part of the module path.
- Leaving it without an edit is what makes R-1 a manual step.

**Fix:** treat only the words before `(` as the path.

## #126: a function value's precondition repair

### What holds

- **The guarded lambda never weakens a contract.** It calls the callee only under the conjunction of every `requires`, so each obligation of the call holds by the guard. The verifier still checks the lambda, so a wrong guard cannot pass.
- **Labels are kept.** The lambda's parameters take the expected function type, labels included. Probe `precondition_labeled`: for `same(a: secret Int, b: Int) -> secret Int`, the lambda `fn(a, b) -> if a == b then same(a, b) else 0` checks clean against `fn(secret Int, Int) -> secret Int`.
- **No capability is added.** The helper of a failing callee omits `uses`, so as a private function its effects are inferred from its body: the callee's, no more.

### P-1, minor: the helper of a failing callee drops the callee's labels

For `risky(a: secret Int, b: Int) -> secret Int requires a == b fails Bad`, the repair proposes `risky_where_defined(a: Int, b: Int) -> Int fails Bad`, without the labels. Applied as written, it does not check: `precondition_labeled_applied` reports U4001 at the result (`secret Int` returned as `Int`) and U4001 where it is passed (`fn(Int, Int) -> Int` given for `fn(secret Int, Int) -> secret Int`).
- It is rejected, so nothing leaks, but the repair is not one an agent can apply as written.
- The same printing would drop the callee's parameter refinements. A dropped refinement is a dropped precondition, but the verifier catches it at the call inside the helper, so it is also rejected, not weakened.

**Fix:** print the helper's parameters and result with the callee's labels and refinements, as written in its header.

## Summary for triage

| Finding | Severity | Soundness | Fix |
| --- | --- | --- | --- |
| V-1 | Minor | No: the program is rejected by the division obligation | Note instead of the counterexample, by an amendment to ADR 0054 |
| R-1 | Moderate | No leak by itself, but one mechanical step makes a call reach another function, which can change labels and contracts | Choose renames across the package, or advice only for cross-module names |
| R-2 | Minor | No | Path words only before `(` |
| P-1 | Minor | No: the repair is rejected | Print labels and refinements in the helper's header |
