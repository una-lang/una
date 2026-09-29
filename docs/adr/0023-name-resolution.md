# 0023. Name resolution

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo (accepted on 2026-09-29), on a proposal by Claude (overnight run)

## Context

M3 implements "imports, visibility, cycles, duplicates, unresolved names (U1001 to U1004)" and naming conventions (U1005). The specification gives pieces of the rules: all top-level names of a module are visible throughout it (`spec/02 §1`); declarations are private unless `pub` (`spec/04 §5`); the prelude is imported into every module (`spec/14 §1`); requirement identifiers are unique per package and example names per module (`spec/08`); the conventions of `spec/01 §4`. It says nothing about namespaces, shadowing, what an import of a kind brings, or where U1001 ends and U2002 ("Unknown or ill-formed type") begins. The reference example `events` fixes one constraint: `main.una` matches `SoldOut`, `Few` and `Open`, the variants of `Availability`, without importing `Availability` or its variants. Programs also declare names that the prelude declares too, such as a variant `NotFound`, which the prelude kind `FileError` (`spec/14 §3`) has, and later editions of the prelude may add names that programs already use.

Decision log entries: D-037 to D-042 and D-046 (`docs/reviews/open-decisions.md`).

## Decision

1. **Namespaces.** Each module has a lower-case namespace (functions, predicates, handlers, policies) and an upper-case one (kinds, their variants, capabilities). Requirement identifiers form one namespace per package, example names one per module, and module paths one per package plus `std.testing`.
2. **Scopes.** From innermost to outermost: locals (parameters, type parameters, `let`, pattern bindings, lambda parameters, handler and handler-operation parameters); the module's own declarations; its explicit imports; the variants of the sum kinds it imports, since importing a sum kind brings its variants; the prelude (`spec/14 §1, §3`). A name resolves if some scope holds it, and the innermost holder wins. When one scope holds several candidates, such as variants of two imported kinds with the same name, the type checker chooses by type (M4); a name that stays ambiguous is U1001.
3. **Variants in context.** A variant whose kind the context gives is resolved by that kind, in M4, whether or not any scope holds it: in a `when` pattern or an `is` test (the kind of the value tested), after `fail` in an expression, a handling clause or `checked` (the error kind of the function), and after `expect fails` (the error kind of the `do` expression).
4. **U1001** reports a name that no scope holds: in expressions, callees, constructors outside the contexts of rule 3, type names and type parameters, capability names in `uses` and `for`, handler names in `with` and `given`, the `via` function of a policy, and imports (a module of neither the package nor `std`, or a name the module does not declare). U2002 (M4) reports a name that resolves to something that is not a type, or an ill-formed type. Field names, method names and argument labels depend on types (M4), capability references `@x` on capability contexts (M5), and label categories on `una.toml` (U4007, M6).
5. **U1002** reports: two declarations of a module with the same name in one namespace, variants included; an explicit import with the name of a declaration of the module or of another explicit import; a local that rebinds a local of an enclosing scope; two parameters, type parameters, fields, variant fields, operations, handler operations, pattern bindings or lambda parameters of one construct with the same name; two requirements of a package or two examples of a module with the same name; two label categories in `una.toml`. A declaration or import may reuse a prelude name, and a local may reuse any non-local name: the inner one wins.
6. **Imports.** An imported name must be public (U1004, with an edit that adds `pub` to the declaration). An import cycle is U1003, reported once per cycle at the first import of the cycle's first module, in path order.
7. **Syntax errors.** In a module with syntax errors, declarations that parsed are registered and checked for U1002, U1005 and U1008, but bodies are not resolved, and imports from that module are not checked, so no diagnostic cascades from a declaration that failed to parse.
8. **Conventions (U1005)**, checked where a name is defined: lower-case names (declarations, parameters, locals, fields, operations, module path segments, owners) contain no `__` and do not end with `_`; kinds, variants, capabilities, type parameters and label categories are UpperCamelCase (`[A-Z][a-zA-Z0-9]*`); requirement identifiers match `[A-Z][A-Z0-9]*(_[A-Z0-9]+)*_[0-9]+`.

### Changes to other documents on acceptance (made on 2026-09-29)

- `spec/10` gains a section "Names" stating rules 1 to 8: `spec/10 §11`.

## Consequences

- The constraint of the reference example holds: `main.una` matches the variants of a kind it never names. A module's own `NotFound` shadows the prelude's.
- M3 never reports a name that the type checker could still resolve; M4 adds the type-directed choice.
- Forbidding a local from rebinding another local is strict: code reads the same wherever a name appears inside a function. Relaxing it later breaks no program.

## Alternatives considered

- **Reject every reuse of a prelude name:** a module could not declare its own `NotFound`, and every prelude addition in a later edition would break programs.
- **Import only the names listed:** a module that imports a sum kind would also need to import every variant it constructs.
- **Resolve every variant by type only:** leaves bare constructors such as `SoldOut` without a meaning outside a typed context.
- **Resolve every variant through scopes only:** `main.una` would have to import every variant it matches.
- **Allow locals to rebind locals:** common in functional languages, but a reader of a contract must then track which binding each use means.
