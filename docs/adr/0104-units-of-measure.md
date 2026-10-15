# 0104. Units of measure

- Status: Accepted
- Date: 2026-10-14
- Deciders: Una project (accepted on 2026-10-14)
- Resolves: issue #34
- Target: edition `0.3`, as a preview construct (`spec/13 §7`). Editions `0.1` and `0.2` do not change, and the Gate 1 pin does not move.
- Amends, once accepted: `spec/02 §2` (grammar), `spec/03` (types), `spec/10` (static semantics), `spec/13 §4` (codes) and `§8` (the Core export); a new chapter, `spec/25`, holds the rules.

## Context

Programs written for Una count things in units: milliseconds since 1970 (`now()`), grams and milliliters (`examples/pantry`), degrees Celsius and percent (`examples/greenhouse`), days (`examples/toolshelf`). Today a unit lives in a name or a comment. `kind Celsius = Int in -40..80` documents the unit, but the type is still `Int` (`spec/03 §2` rule 2: aliases and refinements are structural over their base), so a value in Celsius adds to a value in percent without complaint, and a function that takes milliseconds accepts seconds. These are the mistakes that generated code makes and that a reader does not catch: every value is an `Int`, and nothing says which `Int`.

The constraints on a design:

- **No implicit conversions** (charter §6, ADR 0057 §4). A value changes unit only where the code says so.
- **No floating-point type** (ADR 0068 D5, ADR 0073). Every magnitude is an integer: `Int` or a fixed-width integer (`spec/18`). A conversion between units must be exact.
- **Una's own syntax.** No angle brackets and no annotation syntax borrowed from TypeScript or similar languages. Brackets already mean type arguments (`List[Int]`).
- **The verifier stays decidable** (`spec/12 §2`): queries stay in QF_UFLIA, with the fallback of ADR 0053 and ADR 0054.
- **Edition `0.3` is a preview that only narrows** (`spec/13 §7`): a construct it adds, once erased, must give a program that edition `0.2` accepts, with no fewer allowed behaviors.
- **The security model is unchanged.** Labels (`spec/07`) and capabilities (`spec/05`) apply to a measured value exactly as to its magnitude.

## Decision

### 1. Declaring units

A unit is declared at the top level of a module by the contextual keyword `measure`, in edition `0.3`. `measure` is the keyword only at the start of a top-level declaration, after an optional `pub`, and only when an upper identifier follows it; so a function `pub measure(x: Int) -> Int` keeps its meaning:

```
pub measure Meter
pub measure Second
pub measure Kelvin
pub measure Gram
pub measure Kilogram = 1000 * Gram
measure Kilometer = 1000 * Meter
measure Millisecond = Second / 1000
pub measure Newton = Kilogram * Meter / (Second * Second)
```

- A declaration without `=` declares a **base unit**, a dimension of its own. A base unit is identified by its declaration, its module's path and its name, as a capability is (ADR 0092): `a.Meter` and `b.Meter`, declared in two modules, are two dimensions, and never compare equal.
- A declaration with `=` declares a **derived unit**: a unit expression over declared units, with optional factors. A factor is a positive integer literal; one that divides (`Second / 1000`) makes the unit's scale a positive rational, reduced.
- Unit names are upper identifiers, in their own namespace, and follow the rules of kind names for visibility (`pub`), imports (`use`) and qualification. Each of these is U1002:
  - a module that declares a unit and a kind with the same name;
  - a module that imports a unit and a kind with the same name, from two modules;
  - a unit named as a built-in type, a prelude kind or a constraint (`Int`, `I64`, `Text`, `List`, `Map`, `Option`, `Result`, `Eq`, `Ord`, `Integer`), as ADR 0079 does for kinds.
- Each of these is U2029, at the declaration:
  - a cycle among derived units;
  - a unit expression that names an unknown unit;
  - a factor that is not a positive integer literal, as `Meter * (0)` or `Second / 0`;
  - a derived unit whose normal form (rule 2) has no base unit and scale 1, as `measure Ratio = Meter / Meter`, since `Int of Ratio` would silently be `Int`.
- `measure` stays an identifier everywhere else: a field, a variable or a variant named `measure` keeps its meaning, as in `examples/pantry`.

### 2. Measured types

A numeric type takes a unit with `of`, which is already contextual (`spec/02 §3`), here recognized right after `Int` or a fixed-width integer type in a type position. It does not meet `uses of` or `fails of`, which follow a clause keyword:

```
distance: Int of Meter
elapsed: I64 of Millisecond
pub kind Speed = Int of Meter / Second in 0..300
pub kind Absolute = Int of Kelvin where it >= 0
```

- The grammar becomes `type = [ label ] type_core [ "of" unit_expr ] [ "in" range ] ;` with `unit_expr = unit_term { ( "*" | "/" ) unit_term } ;` and `unit_term = upper_ident | "(" unit_expr ")" | integer_literal ;`. Only `Int`, `I8` to `I64` and `U8` to `U64` take `of`; any other base with `of` is U2029.
- A unit has a **normal form**: a positive rational scale, and a map from base units, keyed by their declarations (rule 1), to nonzero integer exponents, sorted by module path and then name. Two units are equal exactly when their normal forms are equal: `Kilometer` and `1000 * Meter` are the same unit; `Kilometer` and `Meter` are not.
- A unit whose normal form has no base unit and scale 1, as `Meter / Meter` written in a type, is no unit: the type is the plain numeric type.
- A unit with no base unit and a scale other than 1, as `Kilometer / Meter` (scale 1000), is a **scaled dimensionless** unit. It stays a unit: `Int of Kilometer / Meter` is not `Int`. `convert()` takes it to the plain numeric type when the ratio is an integer (rule 4), here multiplying by 1000. Its `magnitude()` is the value counted in steps of its scale, 1000 here, which is easy to misread; the `explain` text of U2028 says so.
- A measured type is a type of its own: `Int of Meter` is neither `Int` nor `Int of Second`, and no value moves between them without an explicit operation (rule 4). Where an `Int` is expected, an `Int of Meter` is a mismatch, U2028. This is what makes units useful, and it is why the unit is part of the type and not an alias of `Int`.
- A refinement of a measured type, by `in` or `where`, constrains its magnitude. In a clause, a measured type reads as its numeric type (`spec/18 §2.4`), and the unit rules of rule 3 still apply to the clause's operands (rule 4 says how a clause writes a conversion).
- A measured type may be a type argument: `List[Int of Meter]`, `Option[I32 of Gram]`, `Map[Text, Int of Second]`. Since a measured type is not a refinement, ADR 0040's rule on refined type arguments does not apply to it. A refined measured kind, such as `Speed` above, is treated as any refined kind is: refused as a type argument today (U2002), and under whatever rule replaces ADR 0040's for refined elements.

### 3. Arithmetic and comparison

The operators of `spec/10 §3` and `spec/18 §2.2` keep their meaning on magnitudes. Their units follow these rules, checked on every expression, clause included:

- `+`, `-`, `mod`, unary `-`, `==`, `!=`, `<`, `<=`, `>`, `>=` and `in range`: both operands have the same unit, and the result of `+`, `-`, `mod` and unary `-` has it. Different units are U2028, whose message names both normal forms.
- `*`: the units multiply. `Int of Meter * Int of Meter` is `Int of Meter * Meter`.
- `/`: the units divide, and the division is Euclidean, as on `Int` (`spec/10 §3`). `Int of Meter / Int of Second` is `Int of Meter / Second`. Its `division` obligation is unchanged.
- **Constant operands.** A constant expression, made only of integer literals and operators, takes the unit its context expects, as one literal does and as a literal takes a fixed-width type (`spec/18 §2.1`). So in `d + 5` and in `d + 2 * 3`, with `d: Int of Meter`, the constant is `Int of Meter`, and so is it in `requires d > 2 * 1000`. A constant that is itself an operand of `*` or `/` with a measured value is unitless: `2 * d` and `d / 2` keep `d`'s unit. Without a context, a constant is unitless.
- **Fixed-width magnitudes** follow `spec/18`: the operands of an operator have the same integer type and the same unit, and the `overflow` obligations are those of the integer type.
- **Ranges.** In `for i in a..b` (`spec/17 §4`), both bounds have the same unit, and `i` has it: in `for i in 0..d` with `d: Int of Meter`, `0` takes `Meter`, and `i` is `Int of Meter`.
- There is no unit inference across declarations: a function's parameters and result declare their units, as they declare their types. Inside a body, a `let` without a type takes its value's unit.
- There are no unit variables: a function is not generic over units.

**Constraints and built-in methods.** A measured type satisfies `Eq` and `Ord` when its numeric type does (`spec/20`), and never `Integer`: `Integer` gives `to_int()`, a unitless `Int`, which would remove the unit without rule 4. So `sum[T: Integer]` on a `List[Int of Gram]` is U2025, and a sum of measured values is written `xs.map(each: fn(x) -> x.magnitude()).sum().measured()`. `abs`, defined on `Int`, takes a unitless argument: `abs(d)` is U2028, and `abs(d.magnitude()).measured()` keeps the unit. The built-in methods of fixed-width integers (`spec/18 §4`, `§5`) on a measured receiver:
- the conversions between integer types, `to_i8()` to `to_u64()` and `to_int()`, keep the unit: `d.to_i64()` on `I32 of Gram` is `I64 of Gram`;
- `wrapping_add`, `wrapping_sub`, `saturating_add` and `saturating_sub` take an argument of the receiver's unit and keep it;
- `wrapping_mul` and `saturating_mul` multiply the units of the receiver and the argument, as `*` does;
- the shifts keep the receiver's unit and take a unitless shift amount;
- `bit_and`, `bit_or`, `bit_xor` and `bit_not` need a unitless receiver and argument: on a measured value they are U2028;
- any other built-in method of an integer type needs a unitless receiver (U2028), so a method that a later specification adds does not carry a unit by accident.

### 4. Explicit operations

Three built-in methods move a value between units. Each is explicit, and each takes the unit it produces from the type its context expects, as a constant does:

- `n.measured()`: from a unitless integer to the same integer type with the expected unit. `let d: Int of Meter = n.measured()`.
- `d.magnitude()`: from a measured type to its numeric type, unitless.
- `d.convert()`: from one unit to another of the same dimension, the same base units with the same exponents, where the ratio of the scales, source over target, is a positive integer `k`: the magnitude is multiplied by `k`. `let m: Int of Meter = km.convert()` multiplies by 1000. On a fixed-width magnitude, the multiplication has an `overflow` obligation (`spec/18 §3`). A conversion whose ratio is not an integer, as from `Meter` to `Kilometer`, or between different dimensions, is U2030: the exact way down is written out, `(d.magnitude() / 1000).measured()`, so the truncation of the division shows in the code.

Each of these is U2030:
- `measured()` or `convert()` with no expected unit;
- `measured()` on a measured value, and `convert()` on a unitless value or to a unitless type that is not a scaled dimensionless unit's conversion (rule 2);
- a `convert()` whose `k` does not fit the magnitude's integer type, as `k = 3,600,000` into `I16 of Millisecond` from `Hour`, since the erased literal would not fit (`spec/18 §2.1`).

`magnitude()` on a unitless value is U2028. There is no conversion between a measured type and `Text` other than through `magnitude()`.

**Method resolution.** In edition `0.3`, `x.measured()`, `x.magnitude()` and `x.convert()`, with no arguments, are these built-ins, ahead of a function of the same name in scope (`spec/10 §5` rule 4), as the conversions of `spec/18 §4` are. A function of the package named `measured`, `magnitude` or `convert` is still called as a function, `convert(x)`. A call in method form where neither the receiver nor the expected type has a unit is then U2030 or U2028, never a silent change of meaning. Where the expected type has a unit, a package function of the same name, which could not return a measured type before this record, would have been U2028; the built-in only turns that error into an acceptance with the built-in meaning, which is new edition `0.3` code. So a package of edition `0.3` that called its own function in method form gains an error and keeps no other result.

**Clauses.** `magnitude()`, `measured()` and `convert()` are terms of the refinement language (`spec/03 §3`): the first two as the identity on the magnitude, `convert()` as the multiplication by its literal `k`, which the refinement language admits. So a clause that compares a measured parameter with a unitless one writes `requires d.magnitude() > n`, and a reflect predicate over `Int` applied to a measured value takes `d.magnitude()`. A unit error in a lock condition of `[contracts]` (ADR 0092) is U7002 at its entry, as any lock condition that does not check is.

### 5. Erasure

Units are compile-time only. The **erasure** `E` maps an edition `0.3` program to one that edition `0.2` reads. It is defined once, here, and every other rule refers to it:
- `E` deletes every `measure` declaration and every `of unit_expr` of a type;
- `E(x.measured())` and `E(x.magnitude())` are `E(x)`;
- `E(x.convert())` is `E(x)` when `k = 1`, and otherwise `E(x) * k`, in that operand order, with `k` a literal of the magnitude's integer type;
- `E` leaves everything else as it is.

Elaboration applies `E` before A-normalization (`spec/09`), so the three methods introduce no temporary of their own. Then:

- **Runtime.** Values carry no unit. The interpreter and the native backend run the erased program, so a measured value costs what its magnitude costs.
- **Text and JSON.** `to_text`, interpolation and the Una form of a measured value are its magnitude's (`spec/11 §5`, `spec/18 §2.3`): `"{d}"` writes `5`, not `5 Meter`. `parse_int` (`spec/22 §4`) and `Json.Number` (`spec/23 §1`) give unitless integers, which `measured()` makes measured. Traces and `--json` outputs write magnitudes. So a program's output does not depend on its units, which is what the preview rule asks.
- **Verification.** Obligations are generated from the erased Core (`spec/12 §3`). A measured type encodes as its numeric type's sort, `Int`, with the range hypotheses of a fixed-width type (`spec/18 §6`). No sort, symbol or logic is added: the query texts of a program are byte-identical to those of `E` of it, and stay in QF_UFLIA. Obligation ids and goal texts are not: they are hashed from, and print, the source's text, so `km.convert()` and `km * 1000` have different ids. The unit check is entirely the type checker's, before the solver runs.
- **Core export.** `una core` (`spec/13 §8`) adds an optional `unit` field, the normal form, to a numeric type. It is additive under `una.core/1` (ADR 0076 rule 1). With every `unit` key removed, the export is byte-identical to that of `E` of the program, except for the source text of goals.
- **Information flow.** A measured value's labels are its magnitude's; `spec/07` is unchanged.

### 6. The preview rule

The construct satisfies `spec/13 §7` for edition `0.3`, stated against `E`: for a program that edition `0.3` accepts, `E` of it is a program that edition `0.2` accepts, with the same behavior, and units add only errors (U2028 to U2030, and U2025 and U7002 where rule 3 and rule 4 say so). The three methods are not syntax errors in edition `0.2`, where a call `x.convert()` resolves to a function in scope or is U1001, so the preview rule's clause on new constructs needs one line for them, which this record proposes with it: "A later `0.3.x` release MAY also give the methods `measured()`, `magnitude()` and `convert()` of ADR 0104 their meaning; every use of them where neither the receiver nor the expected type carries a unit is an error."

### 7. Codes

Three codes join the type range of `spec/13 §4`, each with an `explain` text and ranked repairs:

| Code | Meaning | Repairs, ranked |
| --- | --- | --- |
| U2028 | Operands, an argument, a result or a binding have different units, or a measured and a unitless value meet, including `magnitude()` on a unitless value and a unitless-only method on a measured value | `convert()` when the dimensions match and the ratio is an integer; `measured()` or `magnitude()` when one side is unitless; change the declared unit |
| U2029 | An invalid unit declaration or unit expression: an unknown unit, a cycle, a factor that is not a positive integer literal, a declared unit with no dimension and scale 1, `of` on a type that is not an integer | name a declared unit; break the cycle; write the factor as a positive integer; declare a base unit |
| U2030 | `measured()` or `convert()` without an expected unit, `measured()` on a measured value, `convert()` on a unitless value, a `convert()` whose ratio is not an integer, whose dimensions differ, or whose ratio does not fit the magnitude's type | annotate the expected type; write the division out with `magnitude()` and `measured()`; widen the magnitude's integer type |

In editions `0.1` and `0.2`, a `measure` declaration is a syntax error (U0101), as any unknown top-level declaration is, and `of` after a type is U0101.

### 8. The formal model

F1 imports Core through `una.core/1`, which keeps magnitudes only: the `unit` field is optional and the importer ignores it. Since erasure is the identity on values and labels (rule 5), every theorem of `formal/Theorems.lean` holds for the erased program, which is the program that runs. The formal work is one importer test that a document with `unit` fields gives the same verdicts as the same document without them. A future proof that the unit check is sound, that a checked program never adds magnitudes of different units, is a type-system property about the source, outside F1, and is left for later.

### 9. Cost

- **Checker.** Each arithmetic node computes a unit normal form: O(k log k) for k base units in the operands' units, which is small and does not grow with the program. The check stays linear in the program's size. The normal forms of declared units are computed once per module, with a depth-first walk that finds cycles: O(u + e) for u units and e references.
- **Verifier.** Nothing: the queries are those of the erased program.
- **Runtime.** Nothing: erasure leaves no trace in values, and `convert()` is one multiplication by a literal.
- **Implementation.** The parser (the contextual `measure`, `of` after an integer type, unit expressions), name resolution (the unit namespace, `pub` and `use`, declarations as identities), the type checker (measured types, rules 3 and 4, the constraints and built-in methods, the three codes), the refinement language's three terms, elaboration (`E`, before A-normalization), the canonical form (`spec/15`), the Core export field, the explain texts, and about sixty conformance cases: each operator with matching and mismatching units; constants in context; two same-named base units from two modules; `Integer`, `sum` and `abs` on measured values; each fixed-width method on a measured receiver; the three methods with each failure, method resolution against a function of the same name, a `convert()` whose ratio does not fit; scaled dimensionless units; range loops; clauses with `magnitude()`; a lock condition with a unit error; measured type arguments; refinements of measured kinds; the erasure test (`una core --json` with the `unit` keys removed against `E`, and the query texts identical); and editions `0.1` and `0.2` rejecting the syntax. The guide gets a section. The estimate is one milestone item of medium size.

## Consequences

- A unit becomes part of a value's type. Mixing units, passing seconds where milliseconds are expected, or comparing grams with milliliters is a type error with a repair, before anything runs.
- Units cost nothing at run time and nothing in verification, and they do not change any output. A team may add units to a working program and only gain errors.
- Units are an edition `0.3` construct, so packages of edition `0.1` and `0.2` are unaffected, and the Gate 1 pin does not move.
- `measure` and `of` are contextual, so no identifier of an existing program changes meaning.
- Conversions are exact by construction: up a scale by an integer factor, or written out where they truncate.
- Deferred: unit variables (functions generic over units), exponents written as `^n`, units on decimal text (`Json.Decimal`), and a soundness proof of the unit check.

## Alternatives considered

- **A record per unit**, `kind Meters = Meters(value: Int)`: possible today, and nominal. Rejected as the design: it has no algebra (a product of two `Meters` is no area), every operation unwraps and rewraps, and generated code does it inconsistently.
- **Making aliases nominal**, so that `kind Celsius = Int` is distinct from `Int`: rejected. It changes the meaning of every existing alias (`spec/03 §2` rule 2) in every edition and breaks the structural reading of refinements on which the verifier relies.
- **Unit annotations in angle brackets after literals and types**, as some languages write `5<m>`: rejected. It is the syntax the language avoids, and brackets already mean type arguments.
- **Units as values carried at run time**: rejected. It costs memory and time on every value, changes the Text and JSON forms, and would break the preview rule.
- **Implicit conversion between scales of a dimension**, adding kilometers to meters directly: rejected. It is an implicit conversion (charter §6), and down a scale it truncates silently.
- **Rational or decimal magnitudes**: out of scope. Una has no such type, and units do not add one.
- **Unit variables now**: deferred. They need unit unification in the checker and a syntax for unit parameters; the monomorphic design covers the cases of the examples and leaves room for them.
- **Rendering the unit in `to_text`**: rejected for this design. It changes a program's output when units are added, which the preview rule forbids, and a unit name in output is a presentation choice that a program can make itself.

Note (2026-10-15): the Core export carries the erased program E, with no `unit` field; units never reach Core or the verifier.
