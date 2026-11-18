# Adversarial programs of the re-review

There are 89 programs, written against the closure head. Each file, or the main source file of each package, starts with a comment header giving:
- the attack;
- the expected diagnostic;
- the observed result;
- a verdict.

All pass `una fmt --check`. The "Codes at 85787d7" column lists the diagnostic codes `una --json check` reports at the final PR head (`-` means accepted). A program marked Critical or Major is one that is accepted but must be rejected, or one where the runtime departs from the spec, and `../findings.md` has the details. The link programs (`rv-link-*`) need the setup in their header and are judged by `una run`, so an accepted `una check` is expected for them.

Prefixes:
- `rv-gen`: generics with effects and labels
- `rv-hdl`: handler effect floors
- `rv-id`: declaration identity
- `rv-smt`: solver namespaces
- `rv-link`: file links
- `rv-auth` and `rv-flow`: variants of R-001 to R-006
- `rv-verify`, `rv-escape` and `rv-diag`: variants of R-007 to R-021

| Program | Attack | Codes at 85787d7 | Verdict |
| --- | --- | --- | --- |
| `rv-auth-field-fn-returning-effectful-fn.una` | record field of a pure function type whose result is an effectful function, filled by a lambda returning `note`, to carry `note` out of its `with` scope | U3006 | Held |
| `rv-auth-fn-param-contravariance.una` | function expecting a pure function parameter passed where the parameter is effectful, so it would receive `note` | U3003 | Held |
| `rv-auth-join-fn-values.una` | effectful or failing function joined with a pure one by `if` and `when`, then passed where a pure function is expected | U3003,U3006,U3009 | Held |
| `rv-auth-lambda-wraps-effect.una` | pure-typed lambda that calls an effectful function or parameter, returned as a pure function | U3003 | Held |
| `rv-auth-prelude-hof-second-class.una` | effectful function laundered through prelude `fold` initial value, `or_else` default and a `map` lambda result | U2004,U2010,U3006,U3016 | Held |
| `rv-auth-unify-pure-then-effectful.una` | a pure function fixes the type argument first, then an effectful or failing one is unified with it (generic call, `or_else`, `contains`) | U2001,U3007,U3016 | Held |
| `rv-auth-wrap-and-generic-value.una` | effectful and failing functions wrapped in `Ok`/`Err`, passed to a generic function value `let i = id`, and returned by `map` lambdas | U3006,U3016 | Held |
| `rv-diag-empty-counterexample-closed-goal.una` | a refuted refinement whose goal has no free variable but is not a literal, as `zero(1) > 0`. R-021 asked that a ground goal get no empty counterexample line... | U5001 | Minor (N-009) |
| `rv-escape-opaque-predicate-as-value.una` | an opaque predicate is used as a function value and called through it, in a `let`, as the `each` of `map` and as the `keep` of `first_where`. ADR 0042 rule... | - | Minor (N-008) |
| `rv-flow-field-label-equality-variants.una` | record with a `secret` field compared with `==`, through generic `same[T]`, prelude `contains` and a generic over `contains`; `flagged` is a control | U4001 | Critical (N-001) |
| `rv-flow-generic-effectful-param.una` | generic whose effectful function parameter prints its `T` argument, instantiated with labeled data in both argument orders, and prelude `map` with a... | U4001 | Held |
| `rv-flow-generic-fail-payload-use.una` | symbolic label of `a == b` placed in a failure payload of a generic function, recovered by `attempt` | U4001 | Held |
| `rv-flow-generic-variants.una` | generic `!=`, nested generic, field of a generic kind, `when` on `Option[T]`, two type parameters, argument order, lists and kinds as type arguments | U4001 | Held |
| `rv-flow-handler-floor-variants.una` | handlers for an op that `accepts secret`: nested `with` of a printing handler, helper with effects, handler parameter of effectful function type,... | U4001,U4002,U4003 | Held |
| `rv-flow-labeled-fn-variants.una` | calls of labeled function values: failing, chosen by a secret branch, record field call, field value, curried lambda, captured secret, list index | U3006,U4001,U4002 | Held |
| `rv-flow-record-field-label-equality.una` | equality on a record whose declared field is `internal` returns a public Bool, directly, through a generic `same[T]` and through prelude `contains`; runtime... | - | Critical (N-001) |
| `rv-flow-sum-field-label-equality.una` | sum kind with a `secret` payload field compared with `==`, alone and in a list; `opt` and `wrapped` are controls whose label sits in a type argument | U4001 | Critical (N-001) |
| `rv-flow-top-labeled-option-over-rejection.una` | positive side: `is_some`, `is_ok` and `append` on top-labeled values and a generic returning a function; spec/07 §7 rule 8 lets results carry labels | U4001 | Held at 85787d7 |
| `rv-gen-alias-effectful-fn.una` | U3016 bypass through a kind alias of an effectful function type (`kind Printer = fn(Text) -> Unit uses @c: Console[print]`) used as a list element type | U1001,U3016 | Minor (N-012) |
| `rv-gen-closure-over-effectful.una` | pure-typed lambdas that close over an effectful function parameter, passed to a pure parameter and to a generic `id`, and the parameter put in a list | U3003,U3006 | Held |
| `rv-gen-generic-fn-value-label.una` | a generic function bound with `let` and then called. ADR 0038 D3 raises the result of a direct generic call by the labels of its type arguments, but a... | ? | Critical (N-003) |
| `rv-gen-hof-contravariance.una` | pure lambdas and named functions with a public parameter passed where a generic HOF instantiates the parameter with a secret (`apply`, `fold`, `map`), and a... | U4001 | Held |
| `rv-gen-kind-contravariant-flows.una` | the covariant kind-argument flow of `Sink[Int]` into `Sink[secret Int]` at every flow site: annotated let, return, constructor field, list element, call... | - | Critical (N-002) |
| `rv-gen-kind-contravariant.una` | a user generic kind whose parameter sits in a function parameter (`eat: fn(T) -> Int`) is compared covariantly, so a `Sink[Int]` built from `fn(x: Int) ->... | - | Critical (N-002) |
| `rv-gen-kind-fn-field-step.una` | a generic kind with a field of type `fn(T) -> T`, built at `Step[Int]` from an identity lambda and read at `Step[secret Int]`; T also sits in the result, so... | U4001 | Held |
| `rv-gen-labeled-fn-values.una` | function values chosen under a secret branch passed through generics: into `fn(Int) -> T`, through `id`, picked by a secret index with `at`, and folded as... | U4001 | Held |
| `rv-gen-prelude-in-generic.una` | user generics that call prelude generics (`first_where`, `filter`) with a predicate over T, returning `Option[T]` and `List[T]`, then observed with `is` and... | U4001 | Held |
| `rv-gen-prelude-option-labels.una` | `is_some`, `is_ok` and `append` on secret data (D-136), directly and inside a generic under a symbolic condition | U4001,U4002 | Held |
| `rv-gen-refined-kind-arg.una` | the covariant kind-argument flow used for refinements instead of labels: a `Sink[Positive]` holding a function that divides by its argument, read as... | U2002 | Held |
| `rv-gen-result-without-t.una` | generic results whose declared type does not mention the type parameter (`Bool`, `Int`), through one and two levels of generic calls and a `fold` whose... | U4001 | Held |
| `rv-gen-second-class-nested.una` | U3016 and U3006 bypasses: a failing function wrapped in `Some` and in a generic `Box` list, an effectful function in a `Box` inside a list passed to a... | U3006,U3016 | Held |
| `rv-gen-sfun-branch-join.una` | S-Fun at an `if` branch join: a pure and an effectful function joined into one local, then passed to a pure parameter, and the same join under an internal... | U3003,U3006,U4001 | Held |
| `rv-gen-symbolic-pc-prelude.una` | effects under a symbolic program counter reached through prelude results in a generic body: `contains`, `first() is Some`, a field of `Box[T]`, a `when` on... | U4002 | Held |
| `rv-gen-symbolic-sinks.una` | a value labeled with a type parameter sent to a sink that accepts `secret` (not top), and into a public handler instance argument inside a generic body; `c`... | U4001,U4003 | Held |
| `rv-gen-two-params.una` | symbolic labels with two type parameters: the result type mentions only T, but the branch depends on U; option and list results chosen by U are then... | U4001 | Held |
| `rv-gen-unit-positions.una` | the "a Unit position never fails a flow" exemption stretched to `Option[Unit]`, `List[Unit]` lengths, a Unit laundered through a generic `id`, and a filter... | U4001 | Held |
| `rv-hdl-categories` | effect floor with categories: a handler of an op that accepts `internal[Location]` forwards its parameter to, or branches on it before, an op that accepts... | U4003 | Held |
| `rv-hdl-fail-op-under-pc.una` | an operation that can fail and accepts `secret` is performed under a secret branch, and its handler always fails; the performer passes the failure with `if... | U4002 | Held |
| `rv-hdl-files-floor.una` | a handler writes its parameter through the host `Files.write` (accepts `internal`): for an op that accepts `internal` (positive twin, performed under an... | U4001,U4002 | control |
| `rv-hdl-fn-value-param.una` | a handler receives an effectful function as an instance argument and calls it in the body of an operation that accepts `secret` (T-App inside the effect floor) | U4002 | Held |
| `rv-hdl-given-only-floor.una` | a handler that breaks the effect floor, installed only by `given` lines of an example with `expect performed` observing the inner effect | U4003 | Held |
| `rv-hdl-inferred-helper.una` | a handler operation that accepts `secret` calls a private helper whose `uses` are inferred and which prints | U4002 | Held |
| `rv-hdl-keys-data.una` | user handlers of the host `Keys` that print or fail depending on `data` (every label), and a control that prints only the public `key_name` | U4002,U4003 | Held |
| `rv-hdl-labeled-instance-args.una` | handler instance arguments that carry labels: a handler fails, prints and returns a value that depends on a `secret` instance parameter, and an `internal`... | U4001,U4002,U4003 | Held |
| `rv-hdl-nested-floor.una` | nested floors: a handler of an op that accepts `secret` performs an op of another user capability that accepts `public`, whose own handler prints; both... | U4003 | Held |
| `rv-hdl-result-kind-variance.una` | a handler operation result flows by the same covariant kind-argument rule: the op is declared to return `Sink[secret Int]`, the handler returns a... | - | Critical (N-002) |
| `rv-hdl-returns-lambda.una` | handler operation results that are lambdas: one closes over the `secret`-raised parameter, one is chosen by it, one is stored in a list and taken back | U4001 | Held |
| `rv-hdl-sfun-result.una` | S-Fun at a handler operation result and at lambda results: a handler returns an effectful named function for a pure function type, a lambda returns one, and... | U3003,U3006 | Held |
| `rv-hdl-testing-lambda.una` | `std.testing` hidden in a lambda inside a handler operation body, installed by `with` in `main`; and `fake_hash` passed as a handler instance argument in... | U3015 | Held |
| `rv-id-example-given` | an example installs the `lib.x.Gauge` handler `five` with `given` for `measure`, which needs `lib.y.Gauge`; examples are checked on a separate path from `with`. | U3010 | Held |
| `rv-id-field-vs-function.una` | a record `Tool` with a field `bump` of function type and a field `size`, next to functions `bump(t, x)` and `size(t)` that ensure `result == 1`. `t.bump(5)`... | U5001 | Held |
| `rv-id-fn-type-uses` | `lib.x.run_with` takes `fn() -> Int uses @g: Gauge[read]` with `lib.x.Gauge` (`>= 0`); `app` passes `lib.y.measure`, which uses `lib.y.Gauge` (`>= 10`) and... | U3003 | Held |
| `rv-id-handler-foreign` | `lib.x.Gauge` ensures `result >= 0`, `lib.y.Gauge` ensures `result >= 10`. The `lib.x` handler `five` (verified against `>= 0`) is installed for `measure`,... | U3010 | Held |
| `rv-id-host-rebind` | `lib.fake` declares its own `Env` whose `get` ensures `result is None`, so the verifier proves the `Some` branch of `probe` dead (`100 / zero`). `main`... | U3001 | Held |
| `rv-id-import-std-host.una` | a module that shadows the prelude `Files` with its own and still needs the host one imports it from `std.host`, the module spec/14 §3 says declares it. | U1001 | Minor (N-014) |
| `rv-id-imported-capability-handler` | a handler in `app` `for Gauge`, where `Gauge` is imported from `lib.y` (`ensures result >= 10`) and a same-named `lib.x.Gauge` (`>= 0`) exists. The... | U5001 | Held |
| `rv-id-nested-files` | a user capability `Files` in a nested module named like the standard library (`lib.host.std`), with `read` ensuring `result.len == 0` and no `fails`,... | U3012 | Held |
| `rv-id-opaque-reflect` | `lib.a.valid` is opaque (`x >= 0`), `lib.b.valid` is reflect (`x > 0`); both modules define `kind Good = Int where valid(it)`. `app` builds an `a.Good` with... | U5001 | Held |
| `rv-id-policy-twins` | `lib.a` and `lib.b` both declare a policy `release` with the same `from` and different `to` refinements (`1..10` and `0..10`), and via functions both named... | U5001 | Held |
| `rv-id-policy-twins-ok` | positive twin of rv-id-policy-twins, importing `lib.a.release` (`to public Int in 1..10`). | - | control |
| `rv-id-prelude-alias.una` | prelude kinds under other names (`Maybe[T] = Option[T]`, `Failure = FileError`, `Outcome = Result[Int, Failure]`), matched through the aliases. If an alias... | - | control |
| `rv-id-prelude-redeclared.una` | the module redeclares `Option` (variants swapped) and `FileError` (four variants). `first(xs)` returns the prelude `Option` into a `let` annotated with the... | U2001,U3008 | Held |
| `rv-id-reserved-std-host.una` | a single file that declares `module std.host` with its own `Files`, the module where the host capabilities live (spec/14 §3), to become the host declaration. | U1009 | Held |
| `rv-id-shadow-nat.una` | the module redeclares the prelude's `Nat` as `Int in -5..5`. If refinements were looked up by bare name, `n: Nat` would carry `n >= 0` and `100 / (n + 5)`... | U5001 | Held |
| `rv-id-std-spoof` | module paths that contain the reserved segments below the first one: `lib.std.host` declares `Clock` with `ensures result == 0` (this module's `main` uses... | U5001 | Held |
| `rv-id-twins` | `lib.a` and `lib.b` both declare a kind `Level`, a reflect predicate `nonneg` and a function `bump` with different contracts. `app` imports the `lib.a` ones... | U5001 | Held |
| `rv-id-variant-mix.una` | a user kind `Answer = None \| Some(value: Int) \| Ok` puts `None` and `Some` at the prelude's opposite tags, and a user `KeyError` puts `UnknownKey` last.... | U5001 | Held |
| `rv-link-case` | case differences on a case-insensitive file system (macOS APFS default). The grant names `./BOX` while the directory is `box`; paths spell links in other... | - | Held |
| `rv-link-chain` | links to links, loops, a link that leaves the grant and comes back, and a link to the grant's parent (ADR 0038 D9 rules 2 and 3). Setup (from this... | - | Held |
| `rv-link-dangling` | dangling links, final and intermediate, relative and absolute, pointing outside and inside the grant (R-011, ADR 0038 D9 rule 3: "dangling or not"). Setup... | - | Held |
| `rv-link-grant-prefix` | grant prefixes that are themselves links, and two `Files` grants at once. `[run] grants` holds `files:./gl:read` (gl links to `outside`), `files:./gd:write`... | - | Held |
| `rv-link-hardlink` | a hard link inside the grant to a file outside it. A hard link is not a symbolic link, so D9 does not apply; the grant confines paths, not inodes. Setup... | - | Minor (N-013) |
| `rv-link-intermediate` | intermediate directory links, relative and absolute, inside and outside the grant, and `..` after a link component (ADR 0038 D9 rules 1 and 2, spec/14 §3).... | - | Held |
| `rv-link-lexical` | lexical edges of rule 1 (ADR 0038 D9): empty path, `.`, absolute paths, `..` that leaves and re-enters the prefix, trailing slashes and `.` after a final... | - | Held |
| `rv-smt-callee-binders.una` | callee parameters and `result` that share names with caller binders: two calls of one function in a query (`inc(7)`, `inc(9)`, caller local `x`), a... | U5001 | Held |
| `rv-smt-generic-instances.una` | one query with `Box[Int]`, `Box[Text]`, `Box[Box[Int]]`, `Box[U]`, `Pair[Int, Text]` and `Pair[Text, Int]` (a sum whose variants share field names with... | U5001 | Held |
| `rv-smt-internal-names.una` | user kinds named `Text`, `Unknown`, `Fn`, `List`, variants `Unit`, `Hashed`, `Kind`, `Param`, `Len`, `Tag`, fields `len`, `tag`, `at`, `value`, `failure`,... | U5001 | Held |
| `rv-smt-lambda-binder.una` | a lambda parameter `x` after a sibling block that bound `x = 5`, and a lambda parameter `y` after a pattern binder `y = 5`. If lambda parameters kept their... | U5001 | Held |
| `rv-smt-long-names.una` | 1000-character names for a kind, a variant field, a predicate and a parameter. | U5001 | Held |
| `rv-smt-module-mangling` | modules `a1_b.c` and `a1.b_c` both declare a sum `K` (one variant and three). Joined with `_` without escaping, both sorts would be `u!Kind_a1_b_c_K` and... | U5001 | Held |
| `rv-smt-predicate-arg-sorts.una` | a reflect predicate's `define-fun` takes its parameter sorts from the arguments of its first use (generate.rs `predicate`). The uses pass a parameter, a... | - | Held |
| `rv-smt-predicate-function-same-name` | a reflect predicate `lib.p.big` (`x > 10`) and a function `lib.f.big` (`ensures result == (x > 0)`). `app` branches on the function and calls a function... | U5001 | Held |
| `rv-smt-reflect-calls-function.una` | reflect predicates whose bodies call a function (`double`) and the prelude `abs`. A `define-fun` body cannot carry the callee's contract per argument, so an... | U2015 | Held |
| `rv-smt-swapped-type-params.una` | generic callees `second[U, T]` and `boxed[U, T]` called from `flip[T, U]` with the parameters swapped, so a sequential substitution of type parameters would... | U5001 | Held |
| `rv-smt-underscore-kind.una` | a kind named `Box_Int` next to the generic `Box[Int]`; both would mangle to `u!Kind_t_Box_Int` if kind names could contain `_`, merging two field functions... | U1005 | Held |
| `rv-verify-fn-refinement-annotated-let` | a function value whose result is `Int` is bound by a `let` annotated `fn(Int) -> Pos` when the right-hand side is a call or a field access. ADR 0037 rule 2... | - | Critical (N-005) |
| `rv-verify-refined-fn-kind.una` | a refined kind whose base is a function type. ADR 0037 rule 10 says such a refinement is never dropped; ADR 0045 (Proposed) would make the form U2002. | ? | Major (N-006) |
| `rv-verify-via-param` | a policy says `from internal Int`, and its `via` function takes `internal Pos`. spec/07 §7 rule 5 compares only base types, so this is accepted. The body... | - | Critical (N-004) |
| `rv-verify-via-requires` | a policy's `via` function has a `requires` clause. Its body is verified under that precondition, so the D5 obligation that the result satisfies the `to`... | - | Critical (N-004) |

