# 0089. A `checked` predicate's other reads label its failure

- Status: Accepted (coordinator, 2026-10-10, under the standing security rule, for the owner's review)
- Date: 2026-10-10
- Deciders: the coordinator, for the owner's review; written by the builder session
- Resolves: #488 (S-17)
- Amends: `spec/10 §7` (T-Checked), by an appended correction (§13.2), and ADR 0031 by an appended amendment

## Context

`checked(a, φ) else fail C` evaluates the predicate φ at run time with `it` bound to `a`, and fails with `C` when φ is false (`spec/06 §6`). T-Checked (`spec/10 §7`, ADR 0031) requires $pc \sqcup \ell^{*}(a) = \bot$ (U4002 otherwise), because the failure reveals whether φ holds of `a`. The checker computed exactly that label, from the operand's type.

But φ is an expression in scope: it may read any value besides `a`. The interpreter evaluates it in the whole environment (`crates/una-interp/src/machine.rs`, `Rhs::Checked`), and so does native code.

**S-17.** Found by the builder session while widening the formal model with `checked` (#453). Confirmed in both editions on `main` and on the v0.3.4 release binary:

```una
pub kind LenError = | TooLong

pub short(raw: Text, limit: secret Int) -> Text
  fails LenError
=
  checked(raw, raw.len < limit) else fail TooLong
```

- `una check` accepted it, with `[escape_budget] checked_per_module = 1`. The failure was public, because `raw` is public.
- A caller that gave `short` an `internal` number from the environment, and branched on `attempt short("abc", limit: n)`, printed "PIN > 3" for one value of the number and "PIN <= 3" for another.
- That is a comparison with an `internal` value, reaching a `public` sink with no policy.

`checked` is an escape hatch, whose uses the budget counts and the build report lists. But an escape replaces a proof, not a label rule: it was never meant to bypass information flow.

**The audit (#488):** every construct whose predicate or condition is evaluated at run time and decides a success or a failure.

| Construct | What it reads | Labels counted, before this ADR |
| --- | --- | --- |
| `checked(a, φ) else fail C` | `a`, and every free variable of φ | `a` only: **the leak** |
| `if`, `when`, `if … is` conditions, `if error`/`if none` | their scrutinee | yes (T-If, T-Case, `spec/07 §7` rule 4) |
| A refinement (`where`), `requires`, `ensures` | static obligations, never evaluated at run time | not applicable |
| `unreachable` (a `when` without `else`) | it aborts, which is termination, outside the model | not applicable |
| `set` and native list operations out of range | they abort (#480), termination | not applicable |
| `expect` and examples | only `una test`, never a run of `main` | not applicable |

Edition 0.1 has the same `checked` and the same checker code, so it has the same leak.

## Decision

1. **The rule.** T-Checked's premise and the failure's label take every label of every value φ reads:

   $$pc \sqcup \ell^{*}(a) \sqcup \bigsqcup_{x \in \mathrm{fv}(\varphi) \setminus \{\mathtt{it}\}} \ell^{*}(x) = \bot$$

   - Otherwise it is U4002 at the `checked`, with the predicate as its related span when a read value is what makes the label non-public, and the operand otherwise.
   - $\ell^{*}(x)$ joins every label of the type of the local `x`, so a field path such as `limits.most` counts every label of `limits`, the field's included.
   - Names in φ that are not locals, such as predicates and functions, are pure, and their arguments are counted as reads.
2. **Both editions** get the rule: `main` for v0.3.6, and `release/v0.1` for v0.1.8.
3. **No other change.** An operand-only `checked`, and a predicate that reads only public values, are accepted as before.

**Complexity.** One pass over φ's syntax tree to collect its names, O(|φ|). Each name is looked up among the locals and its deep label computed, as at any other use. Linear in the predicate's size.

## Consequences

- A `checked` whose predicate reads a labeled value is now U4002, with a message that names the label. The program must make the comparison public first, through a policy, or return a labeled result instead of failing.
- The formal track's model leaves the predicate abstract, as a function of the operand's text (#453). Its re-check marks a `checked` whose formula reads another value as not re-checked, independently of this rule, as defense in depth.

## Alternatives considered

- **Forbid φ from reading anything but `it`.** This is simpler, but it breaks predicates that compare with a public bound passed as a parameter, `checked(raw, raw.len < limit)`, which are safe.
- **Label the result instead of the failure.** `checked` fails, so its failure is what reveals φ; a labeled result does not stop the failure from being observed. Rejected.
