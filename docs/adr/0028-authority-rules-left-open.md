# 0028. Authority rules the specification leaves open

- Status: Provisional (implemented, pending human review)
- Date: 2026-09-29
- Deciders: Claude (overnight run), pending review by Leonardo

## Context

M5 implements capabilities, effects and failures (`spec/05`, `spec/06 §3 to §6`, `spec/04 §4, §5, §8`, `spec/10 §5, §6, §9`, `spec/14 §3, §4`, ADRs 0005 to 0007 and 0009) with the codes U3001 to U3015 and U7003. The specification gives the rules but leaves some open: `spec/05 §4` reports a capability that a callee needs and the caller lacks as U3001, while `spec/04 §2, §5` reports effects beyond a function's declared `uses` as U3003 and `@` in a function without `uses` as U3004; ADR 0007 infers the `uses` of private functions, but a reference's capability cannot be read from the body alone; handlers, `with` and `given` must "match" a capability without saying where each check points; examples must not use host capabilities (U3014) yet take every capability from `given` (U3001); `una check` knows `[run] grants` but not the grants of a later `una run`; and the escape budget, `std.testing` and escaping lambdas need a reporting site.

Decision log entries: D-071 to D-080 (`docs/reviews/open-decisions.md`).

## Decision

1. **Missing capabilities.** `@c` in a body whose `uses` are declared empty, a public function without `uses` or a lambda whose expected type has no `uses`, is U3004. `@c` that no binding in scope holds elsewhere is U3001. A callee's requirement `@x: C[ops]` is U3001 when the caller's context holds no `@x`, or holds another capability under that name, and U3002 when it holds fewer operations. A call of a function with effects from a body whose `uses` are declared empty is U3003: its effects exceed its declared uses.
2. **Inference.** A private function with no `uses` clause has inferred `uses`: the least set that covers its operations and the requirements of its callees, over the module's call graph. A private function with at least one `uses` clause has exactly the declared ones. `fails` is inferred the same way. An inferred reference `@x` has the capability that the callers bind to `@x`, after the `with` bindings of each call; a reference that no caller binds, or that callers bind to different capabilities, is U3001 with a hint to declare `uses`. The inferred error kind is the one kind of every variant the body raises and every `pass`; a second kind is U3009.
3. **Handlers.** `for` names a capability (U3010 otherwise). A handler implements every operation of its capability (U3011 at the handler's name, listing the missing ones); an operation that the capability lacks, or one with another parameter count, is U3010. An operation body fails only with the operation's error kind (U3009) and passes only an equal kind (U3008).
4. **Installing handlers.** In `with @x = h(args)` and `given @x = h(args)`, `h` names a handler (U3010) for the capability that the statement's expression, or the `do` line, needs at `@x` (U3010). The handler's own `uses` are held by the context where it is installed (rule 1).
5. **Examples.** A requirement of the `do` line that no `given` provides is U3014 for a host capability (`spec/14 §3`) and U3001 otherwise. `expect performed @x.op` and `expect not performed @x.op` name a `given` binding (U3001) and an operation of its capability (U3002).
6. **Grants.** When `una.toml` lists `[run] grants`, `una check` compares the host capabilities in the `uses` of `main` in the `[run] entry` module with them: a host operation that no grant allows is U3012 at its `uses` clause. `una run` compares `main` with its own grants (M7). The grants map to operations as `console` to `Console[print]`, `clock` to `Clock[now]`, `env` to `Env[get]`, `keys:<file>` to `Keys[hmac]`, and `files:<path>:<modes>` to the `Files` operations of the modes.
7. **`std.testing`.** A call of, or reference to, a function of `std.testing` outside a handler operation body or an example is U3015.
8. **Escape budget.** A module uses `checked` at most `[escape_budget] checked_per_module` times (0 by default, and for a single file). The first use beyond the budget is U7003, with the later uses as related spans.
9. **Escaping lambdas.** A lambda whose expected type has `uses` or `fails`, and a function value of a function with `uses` or `fails`, appear only directly as call arguments (U3006 at the lambda or the name). A field of a kind whose type is such a function type is U3006 at the type.
10. **One effect per line.** A capability operation, or a call of a function, policy or function value with `uses` or `fails`, declared or inferred, that is not the whole expression of a statement, `let` or final expression, optionally inside `attempt` and parentheses, is U3013 at the call. Example `do` lines are exempt; `checked` is not a call.

### Changes to other documents on acceptance

- `spec/05` gains a section "Reporting" that states rules 1 to 5 and 9; `spec/06 §6` states rule 8; `spec/14 §4` states rule 7; `spec/13 §5` states rule 6.

## Consequences

- Every authority diagnostic has one code and one site, so agents can repair them mechanically: most carry an edit that adds or widens a `uses` clause.
- Inference gives private functions typed operations without headers, at the cost of a second pass over the bodies.
- Rule 6 lets `una check` catch a missing grant before `una run`; a program run with extra `--grant` flags is checked again then.

## Alternatives considered

- **U3001 for every missing capability:** loses the distinction of `spec/04 §2, §5` between a missing reference and effects beyond declared uses.
- **Inferring a reference's capability from its operation's name:** ambiguous whenever two capabilities share an operation name, and it types a body without regard to what callers pass.
- **U3001 for host capabilities in examples:** leaves U3014 without a case.
- **Checking grants only in `una run`:** the manifest's grants are known statically, and checking them early is stricter.
