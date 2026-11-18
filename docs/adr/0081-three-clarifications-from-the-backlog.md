# 0081. Three clarifications from the backlog: constructor argument order, handlers in the report, and inline lambdas at operations

- Status: Accepted
- Date: 2026-10-08
- Deciders: the coordinator session, on Leonardo's behalf, under the delegation of 2026-10-06 (decisions N-60 and N-61, on the backlog triage of 2026-10-08), on a proposal by Claude (Una-1)
- Resolves: issues #8 (v0.1 review R-023), #14 (v0.1 review R-045) and #85 (round-3 re-review note R3-N1)

## Context

Three items left from the v0.1 reviews each ask for one sentence of spec, and one of them for a small change to the report:

1. **#8, R-023.** `spec/04 §3` rule 5 says arguments are evaluated "left to right in source order", and rule 4 says the formatter orders a constructor's fields by declaration. The review showed `do sum(Pair(y: one(), x: two()))` passing as written and failing with another variant after `una fmt`. The spec does not say which order is the program's.
2. **#14, R-045.** The report's `authority` section lists "every function of the package with `uses`" (`spec/13 §6`). A handler with `uses` clauses (`spec/02 §4`) holds authority too, and is not listed. `tcb.host_capabilities` is computed from the same function headers, so a host capability that only a handler uses is not listed either. The issue held this back because extending the report changed its version; ADR 0076 has since made added keys additive.
3. **#85, R3-N1.** With `op grab(w: fn() -> Int) accepts secret` and `pin: secret Int`, `@p.grab(fn() -> pin)` is U4001 at the lambda, and the same lambda bound with `let w = fn() -> pin` and passed as `@p.grab(w)` is accepted. Both are sound. `spec/07 §7` rule 4 compares an operation argument with the declared parameter type, its covariant positions taken as `⊤`, and `accepts` bounds the data (ADR 0050). An inline lambda is first typed against its expected type, the declared parameter type, as any lambda is (`spec/03 §7`). So its result must flow into the declared `Int`, and `⊤` never replaces that. The spec does not say which comparison an inline lambda gets.

What the implementation does today:
- Elaboration evaluates a call's arguments as written and then places them by parameter (`ordered_values` in `elab.rs`). So a constructor's arguments are evaluated in source order.
- A program that is not in canonical form is U7001, and `una run`, `una test` and `una build` check it first. Every program that runs therefore writes a constructor's arguments in field declaration order.

## Decision

1. **Constructor arguments (#8).** A constructor's arguments are evaluated in the order its kind declares its fields. In canonical form that is their source order (`spec/04 §3` rule 4, `spec/15`), so the two rules agree on every program that checks. `spec/04 §3` rule 5 gains that sentence. Nothing in the implementation changes.
   - A source that writes the fields in another order is U7001.
   - `una fmt` reorders it to declaration order, and that is the order the program means.
2. **Handlers in the report (#14).** The report gains the key `handler_authority`, after `authority`. It lists every handler of the package with `uses` clauses, by qualified name, with:
   - `capability`, the capability it implements, qualified;
   - `public`;
   - `uses`, as written in its header, in the form `authority` uses.

   A handler's `uses` are always written, so an entry has no `inferred`. `tcb.host_capabilities` also counts the host capabilities that handlers use.
   - Under ADR 0076, both are additive: a new key, and an existing list that now also counts what it was meant to count, the host capabilities the package's code reaches.
   - The schema stays `una.report/2`.
   - `authority` keeps its meaning, functions only, so a consumer that reads its entries as functions stays right.
3. **Inline lambdas at operations (#85).** The stricter check stays.
   - An inline lambda given as an operation argument is typed against the operation's declared parameter type, as a lambda is against any expected type (`spec/03 §7`). So its result flows into the declared result type, labels included (U4001), before rule 4 compares the argument.
   - A function value passed by name, or a lambda first bound with `let`, has its own type, and rule 4 compares it as it states.
   - `spec/07 §7` rule 4 gains that sentence. Two conformance cases, one for each form, record it.

## Consequences

- **#8.** No program changes meaning. The rule removes the doubt R-023 raised: `una fmt` cannot change what a program does, since a program whose order differs is not yet one that runs.
- **#14.**
  - A reader of the report sees every handler that holds authority, and every host capability the package reaches.
  - Counting the host capabilities that handlers use in `tcb.host_capabilities` corrects what the field already promises, the host capabilities the package's code reaches. It does not change the field's meaning, so under ADR 0076 it needs no new version.
  - It costs one pass over the handlers: `O(handlers + their uses)`.
  - The text form of the report gains a `handler authority:` section after `authority:`.
- **#85.**
  - Writing an inline lambda against an operation that accepts more than its parameter type declares is rejected where the bound form is accepted.
  - The check stays sound and simple. A lambda keeps a single typing rule, whatever it is passed to.

## Alternatives considered

- **#8: source order, with `una fmt` forbidden to reorder constructor arguments.** Not chosen: canonical form orders them by declaration (ADR 0020), and the two orders already coincide on every program that runs.
- **#14: handler entries in `authority`, with a key that tells them apart.** Not chosen: a consumer that reads every entry of `authority` as a function would misread them. That changes what an existing key means, which ADR 0076 versions.
- **#85: type an inline lambda at an operation argument by ADR 0050's view** of the parameter type: covariant data positions bounded by `accepts`. Not chosen: it gives lambdas a second typing rule that depends on where they are passed, for no program that the bound form cannot already write.
