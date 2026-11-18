# 0087. `spec/10`'s rules carry the label that decides a failure

- Status: Accepted (coordinator, 2026-10-10, for the owner's review)
- Date: 2026-10-10
- Deciders: coordinator, for the owner's review; written by Owner (Una-1)
- Resolves: #479
- Amends: `spec/10`, by an appended correction section (§13); no accepted text is edited

## Context

`spec/07 §7` rule 4 labels a failure by what decides it: the arguments of a host operation that decide its failure, capped at what it accepts, as the failure of a labeled prelude function is (N-27, N-39, N-40), and the package's file-state label for `Files` (ADR 0085). Given that label, the rule says:
- `attempt` gives a `Result` with that label;
- `if error use` runs under it;
- `if error pass` and `if error fail` are U4002 when it is above `public`.

`spec/10`'s formal rules do not say this. Its judgment tracks a failure's error kind $F$, not the label of what decides it. So:
- **T-Let** types the body under $pc$, although the body runs only when the right-hand side did not fail, which that label decides.
- **T-Handle** types the handler under $pc$, and gives the value the type $\tau$, although which branch gives the value is decided by that label.
- **T-Attempt** gives $\mathrm{Result}[\tau, F]$ with no label.

The formal track's model, whose T6 is proven, has all three premises: `formal/UnaFormal/F1/LTyping.lean`, rules `letR`, `letE`, `handle` and `attempt`. M3 found the gap (`docs/plan/formal-track-m3.md` §2). It is not a soundness finding. The shipped checker is not laxer than the model, because the surface language cannot write a program where the two differ. Checked again with `una check` (a build of `main` with #471, 2026-10-10) in both editions:
- `if error pass` on a failure decided by an `internal` path is U4002, so a failing right-hand side whose failure is labeled above `public` can only reach the next statement through `attempt` or `if error use`;
- after `let r = attempt @files.write(p, ...)` with `p: internal Text`, a `@console.print` in a branch of `when r` is U4003: the `Result` carries the label;
- a handler expression with an effect is U3013, since an effectful call must be a whole statement (`spec/04 §4`).

## Decision

`spec/10` gains a correction section, §13, appended after §12, dated, citing M3. It states:

1. **Notation.** A failure $F$ carries the label of what decides it, written $F^{\ell_F}$, with $\ell_\emptyset = \bot$. That label is the one `spec/07 §7` rule 4 gives:
   - T-Op: the deciding arguments of the operation, capped at its `accepts`, joined with the file-state label for `Files`;
   - T-Call and T-App: $\bot$, since a function's own failure is decided at the public program counter (T-Fail, and $\mathrm{bound}(f)$ met with $\bot$);
   - a labeled prelude function: its arguments' labels (N-27).
2. **T-Let.** The body is typed under $pc \sqcup \ell_{F_1}$.
3. **T-Handle.** The handler is typed under $pc \sqcup \ell_{F_1}$, with $x : F_1^{\ell_{F_1}}$, and the expression has type $\tau \sqcup \ell_{F_1}$.
4. **T-Attempt.** The result is $\mathrm{Result}[\tau, F]^{\ell_F}$.
5. **What wins.** Where §4 and §5 differ from §13, §13 wins, as `spec/07 §7` rule 4 already states for the surface language.

**Complexity.** No change. The label is computed where the failure is (T-Op, T-Call, T-App), in O(1) per node, as the checker already does.

## Consequences

- The formal rules of `spec/10`, the model of the formal track, and the checker now agree on the three rules. M3's correction is closed.
- Nothing changes for programs: the checker already enforces `spec/07 §7` rule 4, and the evidence above shows it in both editions.
- `spec/10`'s accepted text stays as written. §13 names each rule it corrects, so a reader of §4 and §5 finds the correction by the rule's name.

## Alternatives considered

- **Edit §4 and §5 in place.** This is cleaner to read, but the correction is to be append-only, and the history of the rules stays visible.
- **Leave `spec/10` as it is,** since the surface cannot tell the difference. Rejected: `spec/10` is the normative statement of the type system, and the formal track proves T6 for the corrected rules, not for the ones written.
