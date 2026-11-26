# 09. Core IR

Status: Normative, v0.1.

Core IR is the only language the checker's formal rules (`spec/10`), the verifier (`spec/12`) and the interpreter (`spec/11`) are defined over. Every surface construct desugars into it. A surface feature that cannot be desugared into Core IR does not exist.

## 1. Properties

1. **A-normal form.** Every operand is an atom; every intermediate result is bound by `let`.
2. **Explicit.** Every binder carries its full type, including refinement and label. Generic instantiations are explicit. Capability contexts are explicit on every function and handler.
3. **Unique names.** All binders are alpha-renamed to be unique within a function. Compiler temporaries use the prefix `%` (not legal surface syntax).
4. **Positional.** Labels, punning and method-call syntax are resolved away.

## 2. Syntax

```text
program     P  ::= D*
decl        D  ::= type T[α*] = τ                                    alias or refinement
                 | data T[α*] = C(f: τ, ...) | ...                    records are single-constructor data
                 | pred reflect p(x: τ, ...) = φ
                 | pred opaque  p(x: τ, ...) = e
                 | cap K { op o(x: τ, ...) : τ fails E requires φ ensures ψ accepts ℓ }
                 | handler h(x: τ, ...) for K uses Κ { op o(y, ...) = e }
                 | fun f[α*](x: τ, ...) : τ uses Κ fails E requires φ ensures ψ = e
                 | policy p : τ ⇝ τ via f

types       τ  ::= {ν: b | φ}^ℓ                                       refined, labeled base
                 | (fn(τ, ...) -> τ uses Κ fails E)^ℓ
base        b  ::= Int | Bool | Text | Unit | Hashed | T[τ*] | α
cap context Κ  ::= { c ↦ K[O], ... }
atoms       a  ::= x | n | true | false | "text" | unit
rhs         r  ::= a | prim(a, ...) | f[τ*](a, ...) | a(a, ...) | c.o(a, ...)
                 | C(a, ...) | a.f | [a, ...] | λ(x: τ, ...). e
                 | checked(a, φ, C(a, ...)) | declassify_p(a)
expr        e  ::= a
                 | let x: τ = r in e
                 | if a then e else e
                 | case a of { C(x, ...) ⇒ e; ... } [ else ⇒ e ]
                 | fail C(a, ...)
                 | handle e with x ⇒ e
                 | attempt e
                 | with c = s in e                                      s ::= d | h(a, ...)
prim            ::= + | - | * | div | mod | neg | == | != | < | <= | > | >= | not
                 | len | concat | to_text
```

`and`, `or` and `implies` are not primitives: they desugar to `if` so that short-circuit evaluation and path conditions coincide. (v0.2, ADR 0069.) The built-in operations of the prelude's types, such as a map's methods (`spec/21 §2`), are calls `f[τ*](a, ...)` of prelude functions in this model; an implementation may represent them as primitives.

(v0.2, ADR 0065.) Core IR gains `together` and `together_for` (`spec/19 §5`).

(Edition 0.4, ADR 0117 rule 6.) Core IR gains `first_of(e₁ | ... | eₙ)`, n ≥ 2: the branches of a race, each closed over the bindings before it, whose value is that of the first branch to return; when every branch fails, it fails with the failure of the lowest-indexed one (`spec/27 §6`).

(Edition 0.4, ADR 0117 rule 7.) Core IR gains `tasks(b) e` and `start(e)`. `tasks(b) e` runs `e`, the block of a `tasks` statement, in the current task, with the bound `b`, an atom bound before it or none for the default bound; its value is `e`'s, the tuple of the outer bindings the block updates, and it ends once `e` has ended and every task started in it has ended, after canceling them when `e` failed or stopped; an `exit_k` in `e` crosses it. `start(e)`, of type `Unit`, occurs in the `e` of a `tasks`, outside the tasks in it: it starts a task for `e`, closed over the bindings before it, which cannot fail, and goes on (`spec/27 §7`).

(S-39, ADR 0116 amendment of 2026-11-06.) Core IR gains `quiet e`, in every edition: the loops, deadlines and task constructs below exist in edition `0.4` and later only, but calls, operations and `with` exist in every edition, and elaboration marks them in every edition (`spec/26 §6` rule 4). The form `quiet e` is written where `e` is a `for ever` loop, a `within`, a `first of`, a `tasks`, `together` or `together for` expression, a `with` that installs a handler (S-41), a `start` (S-40, #1512), or a `let` of a call or an operation, ending in its result. Elaboration writes it when `e` stands under a program counter above `public`: for a call, when its callee can run one of those constructs, install a handler or perform an operation, and for an operation, always (S-42, `spec/26 §6` rule 4). Its value, failures and exits are `e`'s, and it changes only the trace: while `e` runs, in its callees and in the tasks it starts included, the runtime appends no `stop`, `timeout` or `cancelled` event of its own, an operation appends no event to a run's trace (S-42, an example's trace keeps it) unless the mark is `quiet timing`, written when the timing floor of a `within` or a `first of` alone raises `e`'s program counter, whose operations keep their events (`spec/27 §5` rule 7), the `tasks` statements and `first of` blocks that run, and the tasks it starts, are not counted, so their path parts are `qs`, `qf` and `q`, with no number (S-40, S-44, `spec/27 §6` rule 5, `§7` rule 5), and the handlers installed take no number, `with:<h>#q` (S-41), and the operations those handlers serve append no event in any trace (#1544). (2026-11-26, #1223.) The event of an operation that is not recorded, by S-42 or #1544, is not rendered, so it holds no text and reserves none against the memory budget (`spec/11 §7`), which is outside the noninterference model; until #1223 this was an exception to "its failures and exits are `e`'s", for the abort of an integer of more than 4300 digits, which no longer exists. The text form prints `quiet`, or `quiet timing`, on its own line, `e` below it, and `una.core/1` gives it the kind `quiet`, with `body`, `timing` (a Boolean) and `span` (`spec/13 §8`).

(Edition 0.4, ADR 0117 rule 5.) Core IR gains `within(a, C_t, [C_w]) e`: `e` bounded by the deadline `a`, in milliseconds. Its value is `e`'s when `e` returns before the deadline; otherwise it fails with `C_t()`, a variant without fields, and a failure `x` of `e` in time fails as `C_w(x)`, or as `x` without `C_w`. `e within d` with `if error pass` is `within(d, F.TimedOut) e`, with `F` the function's error kind; with another clause it is `within(d, Timed.TimedOut, Timed.Failed) e`, or `within(d, Deadline.Expired) e` when `e` cannot fail, under the clause's own `handle` (`spec/27 §5`).

(Edition 0.4, ADR 0116 rule 20.) Core IR gains `exit_k(a, ...)`, with the tag `k`, `next` or `stop`, and the values of the innermost loop's state: it ends the current iteration of that loop, which goes on to the next one with that state (`next`, from `skip`) or ends with it as its value (`stop`). It occurs only in a loop's body, outside the tasks in it (`spec/26 §3`).

(v0.2, ADR 0056.) Core IR gains tuple types `(τ, ...)`, tuple construction and the binder `let (x, ...) = r in e`. Only the desugaring of the memory model uses them, and they are not surface syntax (`spec/17 §4`).

Refinements, `requires` and `ensures` clauses, the bodies of reflect predicates and the predicates of `checked` are terms of the refinement language (`spec/03 §3`): trees over binders, `it` and `result`, not A-normal expressions. A clause keeps its source text and span, and the contracts of the standard library are signatures in the program. Returned atoms, the value of each `let` (with whether its type is written), each argument, each division and each exhaustiveness check carry their source spans. A `when` over literals or ranges without `else` ends in an `unreachable` branch, whose `exhaustive` obligation proves it cannot be taken; reaching it at run time is a fault (ADR 0033).

## 3. Desugaring

| Surface | Core IR |
| --- | --- |
| Nested pure expression `f(g(x), y + 1)` | `let %t0 = g(x) in let %t1 = y + 1 in f(%t0, %t1)` |
| Labeled arguments, punning | Positional arguments in parameter order |
| `e.f(args)` method call | `f(e, args)` (`spec/04 §3`) |
| A reference to a module constant (edition `0.5`, ADR 0126 rule 8) | Its value, evaluated at check time: a literal, or a constructor or list of such; Core IR declares no constant |
| `a and b` | `if a then b else false` |
| `a or b` | `if a then true else b` |
| `a implies b` | `if a then b else true` |
| `x in lo..hi` / `x in lo..<hi` | `lo <= x and x <= hi` / `lo <= x and x < hi`, then as above |
| `when x` with `is C(..)`, `is literal`, `in range`, `else` arms | `case` for variant arms; `if` chains for literal and range arms; `else` as the final branch, or `unreachable` without one |
| `if error fail V(args)` | `handle e with error ⇒ fail V(args)` |
| `if error pass` | `e` (typing requires equal error kinds) |
| `if error use u` | `handle e with error ⇒ u` |
| `if none fail V(args)` | `let %t = e in case %t of { Some(v) ⇒ v; None ⇒ fail V(args) }` |
| `if none use u` | `let %t = e in case %t of { Some(v) ⇒ v; None ⇒ u }` |
| `stmt with @x = s1, @y = s2` | `with x = s1 in with y = s2 in stmt` |
| `"a {x} b"` | `concat("a ", to_text(x), " b")`; `to_text` is defined for `Int`, `Bool` and `Text` only (U2001 otherwise) |
| policy call `p(v)` | `declassify_p(v)` |
| Short and long function forms | The same `fun` declaration |
| Statement sequence `s1; s2; e` | `let _: Unit = s1 in ...` for Unit statements |

A Core `let` binds a right-hand side or an expression, so the value of a handled statement or of a branching construct can be bound. The checker records its resolutions by the span of each expression (callees and their type arguments, the parameter of each argument, fields, operations, variants, capability bindings, and the labeled type of each expression and binder), and the desugaring applies to the syntax tree with these records; only declarations without diagnostics are elaborated (ADR 0032).

Elaboration fails closed (#1050). Every declaration of the package that type-checked is in Core IR: its types, functions, predicates, policies, capabilities and handlers among the declarations of its module, and its examples among the module's examples. So are the contracts of the standard library, the host capabilities and the kinds of the prelude that Core IR declares. A declaration missing from Core IR would have no obligations, so the compiler checks this once Core IR is built. Unless another error already rejects the package, a declaration that is missing is U8001 and the package is rejected.

Examples are not Core IR declarations. The test harness (`spec/11 §6`) runs their `do` expression, desugared, inside nested `with` bindings for their `given` lines.

## 4. Debug output

`una check --dump-core <module> [path]` prints the Core IR of the module's declarations on stdout, in the text syntax above, with every binder's type, label and refinements, deterministically (ADR 0032). The diagnostics of the package still go to stderr, and the exit code follows them. The dump is used by snapshot tests and is not a stable interface. (S-45, #1552.) An operation `c.o(a, ...)` carries, with its arguments' static labels, the static label its outcome is rendered at in a run's trace (`spec/11 §5`); the dump prints it after the call, `c.o(a, ...) outcome internal`, when it is above `public`. (v0.3, ADR 0083.) `una --json core [path]` prints the Core IR of every module as one `una.core/1` document, the stable form (`spec/13 §8`).
