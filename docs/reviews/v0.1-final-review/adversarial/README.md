# Adversarial programs of the limited re-review

There are 18 programs, written against `879c26a`. Each file, or the main source file of a package, starts with a comment header that gives the attack, the expected diagnostic, the observed result and a verdict. All of them pass `una fmt --check`.

The "Codes at 879c26a" column lists the codes `una --json check` reports, from `../rerun/codes-new-at-879c26a.tsv` (`-` means accepted). `../scripts/rerun-adversarial.sh` reproduces it: it runs each program in a fresh temporary copy, so the proof cache is cold.

Prefixes:
- `fr-arg`: argument binding and punning (ADR 0049, #68)
- `fr-repair`: repair ranking with attenuated bindings (#69)
- `fr-lambda`: lambda parameter refinements (#71)
- `fr-flow`, `fr-fn`, `fr-op-arg` and `fr-handler-arg`: flow sites against value sources, including sites outside the closure's matrix
- `fr-variance`, `fr-generic`, `fr-label`: label computation and variance (ADR 0046, D-160 to D-168)
- `fr-via`: ADR 0047 and D-169
- `fr-expect`: ADR 0044, ADR 0045 rule 2 and D-170

| Program | Attack | Codes at 879c26a | Verdict |
| --- | --- | --- | --- |
| `fr-arg-binding-forms.una` | ADR 0049 across call forms: method call `b.minus(a)`, a generic `pair(b, a)`, an operation `@calc.sub(b, a)`, a `let`-bound function value `f(b, a)`, a lambda whose parameters shadow, and a labeled argument to a function value | U1002,U2005,U2006 | Held |
| `fr-arg-binding-prelude-handler.una` | ADR 0049 for prelude functions (`starts_with`, `contains`, `at`) whose first argument is named like their second parameter, and for handler instance arguments in `with` and `given` | U2006 | Held |
| `fr-expect-fails-do.una` | ADR 0044 and D-170: `expect fails` after a `do` that cannot fail, one whose callee absorbs the failure, and one wrapped in `attempt`; ADR 0045 rule 2 for an alias | U1001,U2002 | Minor (F-003) |
| `fr-flow-extra-sites-label.una` | label flow sites outside the closure matrix: handler instance argument, failure payload field, `if error use` default | U4001 | Held |
| `fr-flow-extra-sites-refinement.una` | refinement flow sites outside the closure matrix: operation argument, handler instance argument, failure payload, `attempt` then a pattern on the payload, `if error use` default, and `given` | U5001 | Held |
| `fr-fn-refinement-lets.una` | N-005 class: annotated `let` of `fn(Int) -> Pos` from a `when`, an `if none use` default, an operation result and a field; a range-sugar function kind (ADR 0045 rule 1) at a parameter and a `let` | U5001 | Held |
| `fr-generic-invariant.una` | D-161 and D-166: generic functions over an invariant kind, unified in both argument orders, through an identity, a generic `feed`, an `Option` and `let`-bound generic function values | U4001 | Held |
| `fr-generic-return-strip.una` | D-166: generic functions returning a function, a kind with a function field, a `Box` and a list derived from `x == y`, then read as public at the call | U4001 | Held |
| `fr-handler-arg-contravariance.una` | control for F-001: the same values passed as handler instance arguments | U4001 | Held |
| `fr-label-fn-field.una` | D-168: a function value chosen under a secret branch stored in a record field, in a generic `Box`, next to a data field, and sent to an operation; a lambda capturing the secret | U4001 | Held |
| `fr-lambda-param-refinement.una` | UI-008 soundness: a lambda whose parameter is annotated `Pos` flows where `fn(Int) -> Int` is expected, at an argument, annotated `let`, `map`, `fold`, a generic, a function value, a return, a branch join and an `Option` default | U5001 | Held |
| `fr-op-arg-fn-components.una` | F-001 under `List` and a generic `Box` (`List[fn(secret Int) -> Int]`, `Box[fn(secret Int) -> Int]`) at operation arguments, with a refinement control | U5001 | Critical (F-001) |
| `fr-op-arg-fn-fails.una` | control for F-001: a failing function passed where an operation declares a pure `fn() -> Int` | U3009 | Held |
| `fr-op-arg-fn-refinement.una` | control for F-001: a function whose result is 0 passed where an operation declares `fn(Int) -> Pos` | U5001 | Held |
| `fr-op-arg-variance.una` | an operation argument is not a label flow into the operation's parameter type: a `Sink[Int]` built from an identity passes where `op open(s: Sink[secret Int])` is declared, and a named `fn(Int) -> Int` where `fn(secret Int) -> Int` is declared; the handler, holding a secret, feeds it to the argument and returns the result as a public `Int` | - | Critical (F-001) |
| `fr-repair-attenuated-call.una` | issue #69 (UI-005) beyond the operation site: U3002 at a call whose callee needs an operation the attenuated `@front` lacks while `@back` permits it; two permitting bindings; a lambda | U3002,U3004 | Minor (F-002) |
| `fr-variance-forms.una` | ADR 0046 variance through a generic alias of a function type, a nested invariant kind, `Option` of a function, a sum variant, a recursive sum, the second of two parameters and mutually recursive kinds; each value is built at `K[Int]` by a factory and flows into `K[secret Int]` | U4001 | Held |
| `fr-via-shapes` | ADR 0047 and D-169: a `via` whose result is an alias of `to` with no `where`, whose parameter is an alias with its own `where`, a different refined kind, and a generic `via` | U4005 | Held |
