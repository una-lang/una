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

(v0.2, ADR 0056.) Core IR gains tuple types `(τ, ...)`, tuple construction and the binder `let (x, ...) = r in e`. Only the desugaring of the memory model uses them, and they are not surface syntax (`spec/17 §4`).

Refinements, `requires` and `ensures` clauses, the bodies of reflect predicates and the predicates of `checked` are terms of the refinement language (`spec/03 §3`): trees over binders, `it` and `result`, not A-normal expressions. A clause keeps its source text and span, and the contracts of the standard library are signatures in the program. Returned atoms, the value of each `let` (with whether its type is written), each argument, each division and each exhaustiveness check carry their source spans. A `when` over literals or ranges without `else` ends in an `unreachable` branch, whose `exhaustive` obligation proves it cannot be taken; reaching it at run time is a fault (ADR 0033).

## 3. Desugaring

| Surface | Core IR |
| --- | --- |
| Nested pure expression `f(g(x), y + 1)` | `let %t0 = g(x) in let %t1 = y + 1 in f(%t0, %t1)` |
| Labeled arguments, punning | Positional arguments in parameter order |
| `e.f(args)` method call | `f(e, args)` (`spec/04 §3`) |
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

Examples are not Core IR declarations. The test harness (`spec/11 §6`) runs their `do` expression, desugared, inside nested `with` bindings for their `given` lines.

## 4. Debug output

`una check --dump-core <module> [path]` prints the Core IR of the module's declarations on stdout, in the text syntax above, with every binder's type, label and refinements, deterministically (ADR 0032). The diagnostics of the package still go to stderr, and the exit code follows them. The dump is used by snapshot tests and is not a stable interface. (v0.3, ADR 0083.) `una --json core [path]` prints the Core IR of every module as one `una.core/1` document, the stable form (`spec/13 §8`).
