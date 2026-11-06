# 0098. A policy call keeps a public failure when its `via` function's failure reveals nothing

- Status: Accepted
- Date: 2026-10-13
- Deciders: Una project (accepted on 2026-10-13)
- Resolves: issue #497 (S-18 option B, narrowed)
- Target: editions `0.1` and `0.2`. This relaxes information flow: it accepts programs that ADR 0090 rejects. So no release may carry it until its formal statement is merged (rule 6).
- Implementation: #497. Formal statement: #587, which blocks any release that carries #497.
- Amends: `spec/07 §5` (a policy call's failure label) and ADR 0090 rule 1, by a narrowing of its label for one shape of `via` function

## Context

ADR 0090 labels the failure of every policy call $pc \sqcup \ell(\mathit{to})$, where $\ell(\mathit{to})$ is the top label of the policy's `to` type. That label is sound: the policy authorizes releasing what its `via` function decides at $\ell(\mathit{to})$, and the failure is released no lower than that. It is also conservative: it ignores what actually decides the failure.

**The false positive.** A `via` function that only computes an HMAC:

```una
hmac_phone(phone: secret[PII] Phone) -> internal Hashed
  uses @keys: Keys[hmac]
  fails KeyError
=
  @keys.hmac(key_name: "phone_pepper", data: phone)
    if error pass
```

Whether `@keys.hmac` fails does not depend on `phone`. The failure of `Keys.hmac` is decided by its key name, never by its data (`spec/07 §4`), and the key name here is a literal. Yet under ADR 0090, `attendee_key(phone) if error fail ...` in public code is U4002. `examples/events` and the declassification tour work around it (#496).

**What decides a `via` function's failure.** The failure points of a `via` body, and what decides each:

| Failure point | Decided by |
| --- | --- |
| `fail V` | the program counter at that point |
| `checked(v, P) else fail V` | the program counter, `v`, and the other inputs of `P` |
| a failing host operation, passed on (`if error pass`, `if error fail V`) | the program counter, and the arguments that decide that operation's failure (`spec/07 §4`) |
| a failing user function or function value, passed on | the program counter, and that call's own failure label |
| a nested policy call, passed on | the program counter, and ADR 0090 or this ADR for that call |

Aborts (a division by zero, an overflow) are not failures and lie outside T6 (ADR 0057). A failure handled inside the body (`attempt`, `if error use`) is not a failure point.

**Why a general rule is not decided here.** No label is checked inside a `via` function (`spec/07 §5` rule 3). So the labels that ordinary typing computes there can be laundered:
- `let y: public Int = x`, where `x` is secret, types `y` as public;
- a helper `h(p: public Int)` called with a secret argument checks its own failure as if `p` were public;
- an annotated lambda parameter does the same.

Labeling a `via` function's failure "by what decides it" with the ordinary label machinery would then label `if y > 3 then fail TooBig` public, which is unsound. A sound general rule needs an analysis of its own, in which labels only rise inside `via` bodies, together with its own formal statement. That is an open direction (below), not this decision.

## Decision

1. **The failure-decision label.** Each `via` function `f` has a failure-decision label $\ell_f$, computed once from its body:
   - $\ell_f = \mathit{public}$ when `f`'s body has the shape of rule 2;
   - otherwise $\ell_f = \top$.
2. **The shape.** A `via` function `f` has the shape when all of these hold:
   - **No handler of `Keys` is installed by the program's code.** A handler of `Keys` declared in the package may appear only in its own declaration and in examples, where `given` installs it. A handler is installed only by `with` or `given`, and is not a value (U2001), so such a handler is never installed in a run of the program: examples are not runs, since `una run` and `una build` never execute them. Then every binding of `Keys` that a run reaches, whoever supplies it and wherever a `with` rebinds it, is the host's `Keys`. `std.testing` declares no handler. A `Keys` handler whose name appears in any other declaration gives every `via` function $\ell_f = \top$. The check counts words, so an occurrence in a comment or a text literal also excludes. The premise covers every handler of `Keys` that the package declares, in any of its modules, vendored and boundary modules included (ADR 0093): those are modules of the package, and a package imports no code from outside itself. So a handler that wraps or delegates to another is covered twice over. It is a handler of `Keys` itself, and it names the other in its own declaration, which already puts every `via` outside the shape. A handler whose capability does not resolve to a capability, such as one written `for` an alias, counts as a handler of `Keys`, so the check fails closed.

   **The install paths are a closed list.** At this edition `with` and `given` are the only ways to install a handler: a handler is not a value (U2001), and a capability is second-class (U3005), so neither can be stored, passed, or put in a field. Any later construct that installs or rebinds a handler, such as ADR 0093's narrowed rebinding `@y[ops]` (edition `0.3`), must extend this premise and its check, or the premise no longer holds. A test enforces the review: it matches the syntax's statements, expressions, declarations and binding forms with no wildcard, so adding one fails to compile until the check is revisited.
   - **`f`'s body has no `with`**, in a statement or in a handling clause.
   - **Every failure point of the body is a call `@k.hmac(key_name: L, data: e)`**, where `@k` is a binding of `Keys` declared in `f`'s header, and where:
     - `L` is a plain text literal, read on its token as written: no interpolation and no escape;
     - its failure is handled with `if error pass`, or with `if error fail V` where `V` takes no argument. A payload could carry what the unchecked body computes from the `from` value;
     - the call is a statement, or the value of a `let`, in the body's top-level sequence. It is not inside an `if`, `when` or `if none` branch, a block `if` or `when` as the value of a `let`, a lambda, or the handling of another call.
   - **The body has no other failure point.** Una has no implicit propagation (`spec/06`): every failure leaves a body through a `fail` (`fail V`, `checked … else fail V`, `if error fail V`, `if none fail V`) or a `pass` (`if error pass`), and every failing call, whatever its syntax, is handled by one of them. That includes a method-syntax call, a call of a function value, a prelude function whose `fails of` lambda fails, and a policy call. So the check counts the words `fail` and `pass` in the body's text, and they must be exactly the handling clauses of the allowed calls. A word anywhere else, even in a text literal or a comment, puts the body outside the shape. This is stricter than inferred failability, and it fails closed.

   The body's other expressions are unrestricted: they may branch, compute and call functions that cannot fail. They decide nothing about the failure.
3. **The label of a policy call's failure.** For a call `p(v)` of a policy `p : from ⇝ to via f`, at program counter $pc$, the failure is labeled $pc \sqcup \lambda$, where:

   $$\lambda = \begin{cases} \ell_f & \text{if } \ell_f \sqsubseteq \ell(\mathit{to}) \\ \ell(\mathit{to}) & \text{otherwise} \end{cases}$$

   - When `f` has the shape of rule 2, $\lambda = \mathit{public}$, and the failure is labeled $pc$, as for any call of an ordinary function that fails publicly.
   - Otherwise $\lambda = \ell(\mathit{to})$, which is ADR 0090's label, unchanged.
   - ADR 0090's other rules stand: `attempt`, `if error use`, `if error pass` and `if error fail` read this label as they read any failure label (`spec/07 §7` rule 4, `spec/10 §13.1`).
4. **Soundness.** The failure of `p(v)` (whether it occurs, and its variant) must be visible only to observers allowed to see what decides it, or what the policy authorizes releasing.
   - **When $\lambda = \ell(\mathit{to})$:** this is ADR 0090's argument. The policy authorizes releasing at $\ell(\mathit{to})$ whatever its `via` function decides, and #453 Phase B proves it in the model (T-Declassify, `pc ⊔ dst.top`).
   - **When $\lambda = \ell_f = \mathit{public}$:** the failure of `p(v)` is a function of nothing secret. By rule 2, the body's only failure points are `Keys.hmac` calls, reached on every run that reaches them (none is under a branch), and the variant of each is fixed by its handling.
     - A `Keys.hmac` call fails exactly when its key name is not among the keys the run was granted. That is the native runtime's `keys_hmac` and the interpreter's `Keys`/`hmac`.
     - The key name is a plain literal, the same on every run.
     - The granted keys are the run's configuration (`[run] grants`, `--grant`), which the runs that T6 compares share, as they share every grant.
     - So two runs that reach the call agree on whether it fails. Neither the `from` value nor any other input reaches the decision: no flow check inside the body matters, because no value of the body reaches a failure point.
     - The variant is fixed by the handling, and it carries no payload.
     - Reaching the call at all is decided by the program counter at the policy call, and $pc$ is joined into the label.
     - **The termination channel.** An earlier statement of the body that aborts (a division by zero, an overflow) or does not end, on a value derived from the `from` value, separates the runs by termination, not by failure. T6 is termination-insensitive and does not claim it (ADR 0057). This is the same residual class as the security hunt's family F8 (#553).
     - **`@k` is the host's `Keys`.** The policy's caller supplies `@k` by name, and could otherwise install a handler whose failure depends on its own parameters, its state or a captured value, all of which are checked against the handler's labels, not the call's. Rule 2's package-level premise, no handler of `Keys` installed by the program's code, makes every binding of `Keys` that a run reaches the host's. A `with` can then only rebind it to another binding of the host's `Keys`. The body has no `with` either.
   - **Why a choice and not a meet.** Both candidates are sound on their own: $\ell_f$ by noninterference of the decision, and $\ell(\mathit{to})$ by the policy's authority. Their meet need not be. With label categories, $\ell_f \sqcap \ell(\mathit{to})$ can lie strictly below both, for $\ell_f = \mathit{secret}[A]$ and $\ell(\mathit{to}) = \mathit{secret}[B]$. An observer at the meet would then see a failure that neither noninterference nor the policy lets them see. So rule 3 picks one of the two sound labels, and never a label below both.
5. **What does not change.**
   - The `via` body's value stays unchecked (`spec/07 §5` rule 3): this ADR reads its shape, never its labels.
   - S-18's `gate` (`if x > 3 then fail TooBig else x`) has a `fail` under a branch, so $\ell_f = \top$. Its failure keeps the label `internal`, and the program of S-18 stays rejected.
   - A `via` function with `checked` (R-007) keeps ADR 0090's label.
   - A policy whose `to` is `public` keeps a public failure, as before.
6. **The formal statement comes before any release.**
   - The model's B1 asks a `via` body to be `PureOrFails`: no call and no host operation (#453 Phase B). So no `via` function of rule 2's shape is re-checked today. The re-check reports such a declaration as not re-checked, never as accepted, and the corpus has no flip.
   - The theorem for rule 2 extends B1 to `via` bodies whose only failure points are `Keys.hmac` calls with a literal key name, and labels their failure $pc$. It is required before any release carries this change.
   - `main` may carry the change. The release gate must not tag a release that contains it until that theorem (#587) is merged: #587 blocks the next release milestone.
7. **Complexity.** $\ell_f$ is one pass over each `via` body, linear in its size, computed once per `via` function. Each policy call adds one $\sqsubseteq$ test and one join, $O(1)$. A check stays linear in the source.

8. **A stated limitation: examples.** `una test` outcomes are not an observer of T6, which is about the program's runs. A fake handler of `Keys` installed by an example's `given` line can make that example's result depend on its inputs, for instance a fake whose `hmac` fails on the data. That is a test fixture, not a run of the program, and this decision makes no claim about it.

## Consequences

- The `Keys.hmac`-only `via` keeps a public failure, in the `examples/events` shape as well, where a fake handler of `Keys` is installed only by `given`. `examples/events` and the declassification tour drop their #496 workarounds.
- Conformance cases pin both directions:
  - the shape of rule 2 keeps a public failure;
  - S-18's `gate` stays rejected.
- Rule 2 refuses to lower, and the failure stays at $\ell(\mathit{to})$, for each of these:
  - `Keys.hmac` under a branch;
  - an interpolated key name;
  - `Keys.hmac` after a `checked`;
  - `Keys.hmac` with a failing user call;
  - a nested policy call;
  - a handler of `Keys` installed by a `with` in the program's code;
  - a handler of `Keys` named in a comment of the program's code;
  - a second handler of `Keys` beside one that a `with` installs;
  - a handler written `for` an alias of `Keys`, installed by a `with`;
  - a `with` in the `via` body;
  - `if error fail V` with a payload;
  - a failing method-syntax call;
  - a prelude function with a failing lambda.
- A release cannot ship this until the formal statement of rule 6 is merged.
- Other host operations are not lowered. Extending rule 2 to one needs that operation's own evidence and soundness argument.

## Alternatives considered

- **The meet $\ell_f \sqcap \ell(\mathit{to})$.** Rejected: it can be below both sound labels (rule 4).
- **Option B in full:** label every `via` function's failure by what decides it. Deferred, as an open direction. It needs an analysis in which labels only rise inside `via` bodies: an unchecked flow raises its target to the source's label, and a call whose argument exceeds its parameter's label takes the join of its arguments as its failure label. It also needs its own formal statement. This ADR covers the one false positive measured (#496) with the smallest trusted surface.
- **Every host operation whose deciding arguments are literals.** Deferred, for the same reason, one operation at a time: `Files` adds the package's file-state label, and `Http` has every argument as a decider.
- **Keep ADR 0090 alone.** Rejected by #497: it rejects the documented `Keys.hmac` pattern and forces a workaround in the examples.

## Amendment (2026-10-14): the narrowed rebinding, and edition `0.3`

The rebase of the implementation (#497) onto ADR 0093's narrowed rebinding reviewed it as rule 2 requires.

- **Rule 2: the narrowed rebinding installs nothing.** `with @x = @y[ops]` (ADR 0093 rule 4) binds `@x` to the binding `@y` already holds, restricted to `ops`. It installs no handler, so the handler behind `@x` is the one behind `@y`, and a binding of `Keys` that a run reaches stays the host's. The premise of rule 2 needs no extension for it, and the check counts a narrowed rebinding in a `via` body as a `with`, which already puts that body outside the shape. The test that matches the binding forms with no wildcard now lists it.
- **A handler `for` an alias of `Keys` cannot be written.** At editions `0.1` to `0.3`, an import names a capability by its own name, and an alias declaration (`kind K = ...`) names a type, which a capability is not. A handler whose `for` names no capability is rejected (U1001 or U3010), and the check counts it as a handler of `Keys` (rule 2), so it fails closed. The Consequences case for an alias is therefore stated here rather than tested; the case of a second handler of `Keys`, declared beside the one a `with` installs, is tested.
- **Edition `0.3`** (accepted 2026-10-14). The relaxation also applies in edition `0.3`, which is cumulative over `0.2` (`spec/13 §7`): the implementation lowers the failure label there too. This widens the Target line to edition `0.3`.

## Amendment (2026-11-06): the timing sites of a `via` body

- Status: Accepted, by the project's decision of 2026-11-06 on the draft `docs/plan/0098-via-timing-amendment-draft.md` (its option 2). This amendment adds to the accepted text above, which is unchanged; where the two differ, this amendment governs.
- Issue: #1673, the residual of the review of PR #1670. Target: edition `0.4`, the only edition with timing sites (ADR 0116, ADR 0117). Editions `0.1` to `0.3` do not change.
- Amends: `spec/07 §5`, rule 3 and a new rule 7.

**The problem.** Rule 5 keeps a `via` body's value unchecked (`spec/07 §5` rule 3). In edition `0.4` a `via` body can hold a timing site, a `within` or a `first of`, whose outcome becomes the `via`'s result. Whether a deadline or a race passes depends on the program's other work that runs beside the policy call (ADR 0116 IF6), data the policy never receives and does not authorize releasing. Called in a branch of `together` beside `crunch(key)`, such a `via` releases the timing of `crunch(key)` at the label of `to`. The same body in a plain function is refused, since its timing label is checked there.

**The amendment.** Rule 5's first item reads: "The `via` body's value stays unchecked (`spec/07 §5` rule 3), except for the concurrent label of its timing sites (below): this ADR reads its shape, never its labels." And:

> **The timing sites of a `via` body** (edition `0.4`). A deadline or race outcome may reach the result of a `via` function only if its concurrent label is `public`. The concurrent label of a timing site is its `τ_conc` (ADR 0116 IF6) joined with its residue label `L_res` (ADR 0116, amendment P9): what the program's other work, beside the policy call, can make of the site's timing. Since no label is checked inside a `via` body, every outcome of a timing site that the body reaches counts as reaching the result: each `within`, `first of`, `for ever within`, and statement that an installation of `limiter` scopes over, in the `via` body or in code it reaches along the call graph of ADR 0116 P1 (with the edge of each policy call to its `via` function), is U4001 at the site when its concurrent label is above `public`. The other items of the site's timing label are the `via`'s own data and stay unchecked.

**Why it is sound.** The trust of rule 3 covers what the `via` computes from its `from` value and its own inputs: the policy releases it at the label of `to`. With the concurrent label `public`, no other task of the program can run while the site runs, and no residue of other tasks' requests can slow it, so whether its deadline or race passes is decided by the `via`'s own work, which the policy already releases. A site reached from the body but checked with labels counts too, since its result comes back into code where nothing is checked.

**What it refuses.** A `via` with a timing site, called where a part of a task construct can run beside it (`together`, `together for`, `first of`, `tasks`), as the repro of the review of PR #1670. Called from sequential code, as in the conformance case `via_hmac_within`, the concurrent label is `public` and nothing changes. With the preview's `τ_conc`, which is `⊤` wherever a part of a task construct can reach the site's function, the rule is coarse; the precise `τ_conc` (ADR 0116, amendment of 2026-10-25) makes it more precise with no change of its own.

**Alternatives.** Keeping the trusted-code rule and documenting the risk would leave a `via` releasing data of tasks its author cannot see. Refusing `within` and `first of` in every `via` body would also refuse sound programs, such as `via_hmac_within` from sequential code.

**Costs.** The concurrent label of each site is computed by the timing graph already; the check adds O(1) per timing site that a `via` function reaches, after one reach of the `via` functions, O(V + E) per package.

**Implementation.** Issue #1673 implements the check in the checker, with refused conformance cases and their accepted twins. Until it lands, `main` accepts the shape; no released line is affected, since `release/v0.3` and `release/v0.1` have no timing sites.
