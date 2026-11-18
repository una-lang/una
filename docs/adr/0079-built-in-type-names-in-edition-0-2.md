# 0079. Built-in type names cannot be declared in edition 0.2

- Status: Accepted
- Date: 2026-10-08
- Deciders: the coordinator under the owner's delegation (2026-10-08, N-56), for the owner's review; proposal by Owner (Una-1)
- Resolves: issue #130
- Amends: `spec/03 §1`

## Context

A declaration's name enters its module's namespace, where it wins over the prelude: a module may reuse a prelude name (`spec/10`, U1002's explanation). For the built-in types, this means that `pub kind Int = Small | Large` or a record named `Bool` silently replaces the built-in type in that module, with no diagnostic. Every later use of `Int` in the module then means the user's kind (#130, found while implementing #28).

Edition 0.2 already forbids this for its own built-in names:
- the fixed-width integer types (`spec/18 §1` rule 1);
- `Map` and `Entry` (`spec/21 §1` rule 1);
- the JSON and HTTP kinds of the prelude (`spec/23 §1`);
- the constraints `Eq`, `Ord` and `Integer` (`spec/20 §2`).

Each of these is U1002 at the declaration. The older built-in types were left as they were in edition 0.1.

Edition 0.1 is frozen, and Gate 1 pins it: all 55 packages of una-bench are edition 0.1, and none declares such a name.

## Decision

1. **Edition 0.2.** A kind, a variant or a capability named like a built-in type of `spec/03 §1` is U1002 at its name, as the fixed-width names are. The names are `Int`, `Nat`, `Bool`, `Text`, `Unit`, `Hashed`, `List`, `Option` and `Result`. Message: "`<name>` is a built-in type in edition 0.2, so a <kind | variant | capability> cannot take its name".
2. **What stays allowed.** The prelude's variants (`Some`, `None`, `Ok`, `Err`) and its error kinds (`FileError`, `KeyError`, `ParseError`) may still be declared in a module, where they win as any prelude name does. The guide itself declares its own `ParseError`. Host capability names (`Console`, `Env`, `Files`, `Keys`, `Clock`) are not types and are outside this decision.
3. **Edition 0.1** is unchanged: such a declaration still shadows the built-in in its module.

## Consequences

- An agent writing edition 0.2 gets a diagnostic, rather than a module in which `Int` silently means something else.
- One repository program changes: the conformance case `types/recursive_kinds_tour` (edition 0.2) declared `pub kind Nat = Int where it >= 0`, the same as the prelude's `Nat`. The line is removed, and the case uses the prelude's `Nat`. Nothing else in the conformance cases, the examples, the guide or the specification declares these names in edition 0.2; the edition 0.1 review files that do are unchanged.
- Gate 1 and v0.1.x are unaffected.
- The check is in the resolver, beside the fixed-width names. It costs O(1) per declaration.

## Alternatives considered

- **Both editions:** this would change edition 0.1's frozen behavior and need a una-bench amendment, for programs that Gate 1 does not contain.
- **A warning instead of U1002:** Una has no warnings (`spec/13`), and the fixed-width names are already errors.
- **Also forbidding the prelude's error kinds and host capability names:** user code legitimately declares its own `ParseError`, as the guide does, and capabilities are not types. They can be revisited separately.
