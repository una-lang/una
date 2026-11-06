# Draft: ADR 0098, Amendment (2026-11-06): the timing sites of a `via` body (issue #1673)

- Status: Accepted, by the project's decision of 2026-11-06 (option 2 below). The amendment text is "Amendment (2026-11-06): the timing sites of a `via` body" of `docs/adr/0098-a-via-function-whose-failure-reveals-nothing.md`, and its spec text is `spec/07 §5` rule 7; this file keeps the options and the reasoning.
- Origin: the review of PR #1670 (finding F1, High, on `main` only). PR #1670 fixed the missing edge of the timing graph for a policy call's `via` function; this draft settles the residual.
- Implementation: issue #1673 (the check, its conformance cases and their accepted twins).

## Why

A policy call runs its `via` function. Since PR #1670 the timing graph of ADR 0116 P1 has an edge from the call to the `via` function, so a `within`, `first of`, `for ever within` or task construct in a `via` body counts for the sites around the call. One shape stays accepted: a timing site inside the `via` body itself, whose outcome flows into the `via`'s result.

```una
hmac_it(t: secret Text) -> internal Hashed
  uses @keys: Keys[hmac]
  fails ViaError
=
  @keys.hmac(key_name: "pepper", data: t) within 1 * millisecond
    if error fail Gone
```

Called in a branch of `together` beside `crunch(key)`, whether this `within` passes depends on how long `crunch(key)` runs, so `key`, data the policy never received, reaches the policy's result and the code that reads it. `spec/07 §5` rule 3 performs no information-flow check inside a `via` body: that body is trusted to release what it computes from its `from` value at the label of `to`. The same body in a plain function is refused, since there the site's timing label, `τ_conc` included, is checked.

The trust that rule 3 gives a `via` body covers the data the policy receives. It does not cover the program's other tasks, which a `via`'s author cannot see: whether a deadline or a race in the body passes depends on what runs beside the call, which the policy does not authorize releasing.

## Options

1. **Keep the trusted-code rule** and document in `spec/07 §5` that a `via` that turns a deadline or a race into its result also releases the timing of whatever runs beside the call. No code; the trusted module's author carries the risk.
2. **Bound the timing sites of a `via`.** A deadline or race outcome may reach the result of a `via` only if its concurrent label is `public`. Since no label is checked inside a `via` body, every outcome of a timing site the body reaches counts as reaching the result, so the check is at the site: U4001 when the site's concurrent label is above `public`. One check, an amendment of `spec/07 §5` and of ADR 0098, and conformance cases. It refuses the repro, and shapes such as `via_hmac_within` only when the policy is called where other work of the program can run beside it.
3. **Refuse `within` and `first of` in every `via` body.** The simplest rule, but it also refuses `via_hmac_within` called from sequential code, which is sound today.

## Decision

Option 2, the most secure option that keeps the sound programs. The normative text is in ADR 0098's amendment and in `spec/07 §5` rule 7.

## Choices made in writing the rule

- **The concurrent label** of a site is `τ_conc` (ADR 0116 IF6, `spec/26 §7` rule 5 item 5) joined with `L_res` (item 6), the residue that other tasks' requests leave. Both come from work outside the `via` body; the other items of `τ` (the program counter, what the body reads, the operations it performs, item 7's values) describe the `via`'s own data, which rule 3 trusts it to release.
- **Every timing site the body reaches counts,** not only the sites written in it: a function the body calls is checked with labels, but its result comes back into a body where nothing is checked, so a `secret` result that a deadline decided would be released at the label of `to`.
- **Sites:** every site with a timing label of `spec/26` or `spec/27`: `within` (with `start ... within`), `first of`, `for ever within`, and the statement that an installation of `limiter` scopes over.
- **Precision.** With the preview's `τ_conc`, `public` means that no part of a task construct can reach the site's function, so a `via` with a timing site works only for a policy called outside every part of a task construct. The precise `τ_conc` of ADR 0116's amendment of 2026-10-25 makes the rule more precise with no change of its own.

## Costs

One look-up of the concurrent label per timing site that a `via` function reaches, which the timing graph computes already: O(1) per site after the graph, and the reach of the `via` functions, O(V + E) once per package.
