# Limited review of the v0.1.2 fixes: summary

- **Issue:** #97. Branch `review/97-v0.1.2`, from `main` at `37a25a8`, 2026-10-01.
- **Reviewed:**
  - PR #102 (#84, ADR 0052), head `ca54a15`;
  - PR #103 (#72, ADR 0053), head `2dee6fb`;
  - #99, #100 and #101, already merged on `main`.
- **Binaries:**
  - `main` (`37a25a8`);
  - each PR head on its own;
  - both PRs merged locally on top of `main`, with no conflicts.
- **Evidence:** only what this reviewer reproduced, with cold proof caches.

## Verdicts

| Item | Verdict | Findings |
| --- | --- | --- |
| PR #102 (ADR 0052) | **Ready** | none |
| PR #103 (ADR 0053) | **Ready** | V12-001 (Minor): many real refutations through `*`, `/` or `mod` become `unknown` |
| #99, #100, #101 on `main` | Ready | V12-002 (Minor): the rename edit can collide with an existing name, or leave a module file misnamed. V12-003 (Minor): the lambda repair does not check for several `requires` or a callee that can fail |
| **v0.1.2** | **Ready** | 0 Critical, 0 Major, 3 Minor |

No finding affects soundness: nothing that `main` rejects is accepted with #102, #103 or both.

## PR #102: the operation sink bounds only `ℓ⁺`

**Soundness.** I looked for any position that `ℓ⁺` drops but that still carries caller data to the handler.

`ℓ⁺` drops exactly the contravariant positions of the argument's type, such as the parameters of a function argument. Those carry data from the handler to the caller's function. The caller's function is checked against its own parameter labels, and ADR 0050's flow still compares them with the declared type.

`adversarial/v12-sink-channels.una` tries every other channel with `let`-bound values, so the sink and not the eager lambda check (R3-N1) is what decides:
- a result of a result, an `Option` element, a `Box` field, and the parameter of a function parameter: each carries a secret and is U4003;
- a function that fails depending on a secret: U4002 in its own body, so failure is no channel;
- effectful function parameters of operations cannot be written (U1001), and lambdas cannot use capabilities, so that channel does not exist.

A function whose parameter label is higher than declared is accepted, as ADR 0052 intends. The handler can pass it only data at the declared label.

**Reruns:**
- The F-001 programs (`fr-op-arg-variance`, `fr-op-arg-fn-components`) and all the round-3 programs (`r3-oparg-forms`, `-sites`, `-data-accepted`, `-categories`) give the same U4001s.
- `r3-oparg-categories` loses its two U4003s: the exact function, and the one whose `internal` parameter lacks the category. That second one keeps its U4001. R3-001 is fixed.
- Of the 259 earlier programs, this is the only change.

**Spec:** `spec/07 §7` rule 4 states `ℓ⁺`. ADR 0050 has the "Amended by" note, and D-164 is marked superseded in the decision log.

## PR #103: refutations through uninterpreted arithmetic

**Soundness.** `unknown` and `refuted` are both errors, and the rerun of 259 programs plus the new ones shows no program newly accepted. Two earlier programs move from U5001 to U5002 (`verify-negative-literal-operands`, `rv-smt-internal-names`), and no status moves toward `proven`.

**Both outcomes.** `adversarial/v12-arith-outcomes.una`:
- `product_pos` (`a * b` as `Pos`) stays U5001 with the real counterexample `a = 0, b = 38, result = 0`;
- `product_nat` and `guarded`, which are true, are U5002 with no counterexample and the same repairs;
- the linear controls are U5001.

**Determinism.** `una --json check` is byte-identical cold, warm, by default and with `RAYON_NUM_THREADS=1`. `unknown` results are cached as such.

**Spec:** `spec/12 §6` states the rule, including the cases that count as disagreeing.

**V12-001 (Minor).** ADR 0053 says "real counterexamples through products keep their status", but the model is checked and never repaired, so many real refutations become `unknown` with no counterexample:
- `square_pos` (`a * a` as `Pos`, false at `a = 0`);
- `small` (`a * b < 4` with `a, b` in `1..2`, false at `2, 2`);
- `fixed` (`7 / -2` is -3, so `ensures result == 3` is false);
- `negarith.e` of the first review;
- `rv-smt-internal-names`. Its division is refuted at a real point, but an unrelated `100 / zero` in an arm that its `requires` makes unreachable has a zero divisor, which counts as disagreeing.

This is the cost that alternative B was rejected for, in part of the cases. Agents lose the counterexample, but nothing unsound passes.

Possible fixes for v0.1.x:
- when the model disagrees, pin each application to its real value and solve again, a bounded number of times;
- check only the applications on the refuted path.

## #99, #100 and #101 on `main`: messages and repairs

| Item | Checked | Result |
| --- | --- | --- |
| #99 (#73), literal and nested patterns | `is Some(0)`, `Some(-1)`, `Some(true)`, a text field, `Some(Some(x))`, `Some(Two(...))`, `Two(left: 1, ...)` | The messages are accurate. The described second `when` with a literal arm and `else`, and the comparison form, check (a probe of my own, and the conformance case `literal_pattern_repaired`) |
| #100 (#75), syntax hints | the `fail` of a stored value, a `when` after `=`, `if ... then fail` without `else` | Each shape the hints describe passes a full `una check` |
| #100 (#75), reserved-word rename | parameters, fields with punning, functions and their uses, a kind, a module | The edits rename every occurrence consistently. **V12-002 (Minor):** the new name is not checked against existing names. `table` next to `the_table` becomes a duplicate binding (U1002), and `kind service` next to `kind Service` a duplicate kind (U1002). `module table` in `src/table.una` becomes `module the_table` in a file that must hold `table` (U1008). Fix: choose a free name, and for a module, say that the file moves too |
| #101 (#76), precondition of a function value | one `requires`, two `requires`, a callee that can fail, an annotated `let` | The message names the function and the clause. The lambda shape checks for one clause. **V12-003 (Minor):** with two `requires`, each diagnostic's lambda guards only its own clause, so applying either leaves the other refuted (`b > a`). For a callee that can fail, `if a > 0 then failing(a) else ...` is U3013. Fix: guard the conjunction of every `requires`, and for a failing callee describe a named helper function instead of a lambda |

## Counts

| Severity | Count | Findings |
| --- | --- | --- |
| Critical | 0 | |
| Major | 0 | |
| Minor | 3 | V12-001 (#103), V12-002 (#100), V12-003 (#101) |
| Note | 0 | |

**Adversarial programs:** 4 new, in `adversarial/`. `v12-rename-collisions.una` fails at the lexer by design (U0103), so `una fmt --check` cannot format it; the other three pass it.

## Baseline

| Command | `main` (`37a25a8`) | `main` with #102 and #103 merged |
| --- | --- | --- |
| build, `fmt --check`, clippy `-D warnings` (`--locked`), rustdoc, release build | clean | clean |
| `cargo test --workspace --locked` | 360 passed, 0 failed, 0 ignored | 362 passed, 0 failed, 0 ignored |
| `RAYON_NUM_THREADS=1` | 360 passed | 362 passed |
| Conformance | 659 of 659 | 661 of 661 |
| Without Z3, cold caches | 139 fail, each with U5003 only; none skipped | 140 fail, each with U5003 only; none skipped |
| Examples, cold: `una check`, `una test` | all 7 pass; 106 proven, deterministic across thread counts, all cached when warm | same |
| CI | `main` push run 36932059850 green | #102: 36932225857 and 36932233876 green; #103: 36932823493 and 36932855586 green |

## Rerun of the 259 programs of the earlier reviews

`../v0.1-final-review/scripts/rerun-adversarial.sh` ran every program cold with each of the four binaries; the outputs are in `rerun/`.

| Binary | Changed codes against `main` |
| --- | --- |
| #102 | `round3/r3-oparg-categories`: U4001,U4003 to U4001 (R3-001 fixed) |
| #103 | `v0.1-review/verify-negative-literal-operands`: U5001 to U5001,U5002; `v0.1-rereview/rv-smt-internal-names`: U5001 to U5002 (V12-001) |
| Both merged | exactly the union of the two rows above |

## Files

- `summary.md`: this file.
- `adversarial/`: 4 programs, each with a header (attack, expected, observed, verdict).
- `rerun/`: codes of the 259 earlier programs and the 4 new ones, on each of the four binaries.
