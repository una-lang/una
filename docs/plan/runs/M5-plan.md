# M5 plan: capabilities, effects and failures

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/05`, `spec/06 §3 to §6`, `spec/04 §4, §5, §8`, `spec/10 §5, §6, §9`, `spec/14 §3, §4`, ADRs 0005 to 0007 and 0009; also `spec/02 §8` (context restrictions), `spec/08 §2` (examples) and `spec/13 §5` (`una contract`).

Codes: U3001 to U3015 and U7003.

## 1. Design

- **Two passes.** Pass 1 runs the M4 checker over every declaration and records authority facts per body: operations by reference name, calls with the `with` bindings of their statement, raised variants and `pass` clauses. Inference then computes the headers of private functions that omit `uses` or `fails` (ADR 0007), as least fixed points over each module's call graph, and the capability types of their references from the bindings their callers pass. Pass 2 runs the checker again with those headers, so the operations of private functions are typed, and reports every diagnostic at its site. Pass 1's diagnostics are discarded.
- **Capability contexts** (`spec/05 §1, §3, §4`, `spec/10 §5, §6`): a function's `uses`, declared or inferred; a handler's `uses` for its operation bodies; the `given` lines of an example; the `with` bindings of one statement, which rebind (`@x = @y`) or install a handler (`@x = h(args)`); the `uses` of the function type expected for a lambda.
- **Sufficiency** (`spec/10 §2`, Suff): at every call of a function, policy or function value, each requirement `@x: C[ops]` of the callee, after the statement's `with` bindings, is held by the caller's context under the same name, with the same capability and at least those operations.
- **Failures** (`spec/06`, `spec/10 §5`): the error kind of each call and operation; `fail` and the `fail` of handling clauses and `checked`; `pass` between equal kinds; one error kind per function (ADR 0006); `attempt` as a handling site.
- **Positions.** One effect per line (ADR 0009) is a flag set for the whole expression of a statement, `let` or final expression, through `attempt` and parentheses; example `do` lines are exempt.
- **Second-class rules** (`spec/05 §6`, `spec/10 §9`): a capability reference used as a value is U3005; an effectful or failing lambda, or a function value of a function with `uses` or `fails`, only appears directly as a call argument (U3006).
- **Host capabilities, examples, `std.testing` and budgets:** U3012 for `main`'s host `uses` against `[run] grants`; U3014 and U3001 for the `do` line's requirements against `given`; U3015 for `std.testing` outside handlers and examples; U7003 for `checked` beyond `[escape_budget] checked_per_module`.
- **`una contract <qualified.name>`** prints a function's full header, declared or inferred, with its doc comments and requirements; `--json` prints the shape of ADR 0029.

## 2. Spec gaps and ADRs 0028 and 0029

ADR 0028 (Provisional), "authority rules the specification leaves open", records:
1. **Which code reports a missing capability.** `@c` in a body whose `uses` are declared empty (a public function without `uses`, a lambda whose expected type has no `uses`) is U3004 (`spec/04 §2` rule 4); `@c` that no binding holds elsewhere is U3001 (`spec/05 §3`); a callee's requirement is U3001 when the caller holds no such name or holds another capability under it, and U3002 when it holds fewer operations (`spec/05 §4`); a call with effects from a body whose `uses` are declared empty is U3003, its effects exceeding its declared uses (`spec/04 §2, §5` rule 2).
2. **Inference.** A private function with no `uses` clause has inferred `uses`; one with any `uses` clause has exactly those. The same holds for `fails`. An inferred reference takes the capability that callers bind to its name; references without a caller, or bound to different capabilities by different callers, are U3001 with a hint to declare `uses`. The inferred error kind is the one kind of every variant the body raises and every `pass`; two kinds are U3009.
3. **Handlers.** `for` must name a capability (U3010); every operation of the capability is implemented (U3011, at the handler's name, listing the missing ones); an operation the capability lacks, or a wrong parameter count, is U3010; operation bodies fail only with the operation's error kind (U3009, U3008 for `pass`).
4. **Installing handlers.** In `with @x = h(args)` and `given @x = h(args)`, `h` names a handler (U3010) for the capability that the statement's expression, or the `do` line, needs at `@x` (U3010); the handler's own `uses` are held by the context of the installation (U3001, U3002).
5. **Examples.** A requirement of the `do` line that no `given` provides is U3014 for a host capability and U3001 otherwise; `expect performed @x.op` names a `given` binding (U3001) and an operation of its capability (U3002).
6. **Grants.** `una check` compares the host `uses` of `main` in the `[run] entry` module with `[run] grants` when `una.toml` lists grants (U3012 at the `uses` clause); `una run` (M7) compares them with its own grants.
7. **`std.testing`.** A call or reference to a function of `std.testing` outside a handler operation body or an example is U3015.
8. **Budgets.** Each module may use `checked` as many times as `[escape_budget] checked_per_module` allows (0 by default, and for a single file); the first use beyond the budget is U7003, with the other uses beyond it as related spans.
9. **Escaping lambdas.** A lambda whose expected type has `uses` or `fails`, or a function value of a function that declares or infers them, appears only directly as a call argument (U3006, at the lambda or the name); a field of a kind with such a function type is U3006 at the type.
10. **One effect per line.** A capability operation, or a call of a function, policy or function value with effects or failures, not in the positions of `spec/04 §4` is U3013 at the call; `checked` is not a call.

ADR 0029 (Provisional), "the output of `una contract`", defines the human output (the header in canonical form, inferred clauses followed by the comment `-- inferred`, then the requirements it derives from) and the JSON schema `una.contract/1`.

## 3. Tasks and commits

1. **Plan and ADRs** (`docs(plan)`).
2. **Authority facts, inference and contexts** (`feat(check)`): the two passes, inference, sufficiency (U3001 to U3004), `with` and handler instances, handler declarations (U3010, U3011).
3. **Failures and positions** (`feat(check)`): U3005 to U3009, U3013.
4. **Host capabilities, examples, `std.testing` and budgets** (`feat(check)`): U3012, U3014, U3015, U7003.
5. **`una contract`** (`feat(cli)`): human and JSON output, snapshot for the reference example.
6. **Conformance and explain** (`test(authority)`, `docs(diag)`): cases and explain texts for U3001 to U3015 and U7003.
7. **Self-review** (`docs(plan)`).

## 4. Conformance cases (`tests/conformance/authority/`)

At least one failing case per code, each otherwise valid through M10, and passing tours: inferred headers of private functions, rebinding and handler instances with `with`, handlers, examples with `given`, `attempt`, `pass`, `checked` within the budget. U7003 and U3012 are package cases, since they read `una.toml`.

## 5. Primary spans

U3001 at the reference, or at the call whose callee needs it; U3002 at the operation's name, or at the call; U3003 at the call; U3004 at the reference; U3005 at the reference; U3006 at the lambda or the function's name; U3007 at the fallible call; U3008 at `pass`; U3009 at the variant; U3010 at the handler's `for` name, at the operation, or at the installed handler's name; U3011 at the handler's name; U3012 at `main`'s `uses` clause; U3013 at the nested call; U3014 at the `do` line's call that needs the capability; U3015 at the call; U7003 at the first `checked` beyond the budget.

## 6. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Conformance cases for U3001 to U3015 and U7003 | runner summary |
| `una contract --json` snapshot for the reference example | a snapshot test of `una contract --json events.checkin.check_in` |

The whole reference example stays free of diagnostics.

## 7. Known gaps

- Labels, the program counter and sinks (`accepts`) are M6; `bound(f)` of `spec/10 §5` is M6.
- `una run` checks grants when it runs (M7); `una check` checks `[run] grants` only.
- Effect polymorphism is deferred (`spec/05 §8`): higher-order prelude functions take pure, non-failing lambdas.

## 8. Self-review checklist

Appended when the milestone is complete.
