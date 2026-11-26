---
construct: module constants
editions: ["0.5"]
spec: ["spec/02 §1", "spec/02 §2", "spec/10 §8", "spec/10 §11", "spec/12 §3", "spec/13 §7", "spec/17 §2"]
codes: [U1011, U2041]
---

# Module constants: `const` at the top level

## What it does

From edition `0.5`, a module can name a value at the top level with `const`, give it a type, and export it with `pub`. The compiler computes the value once, when it checks the program, and every use sees exactly that value: the verifier proves contracts with it, and the interpreter and native code use it as a literal. A module still runs no code when it is loaded.

`ita.una`

```una
ita geometry
  version 0.1.0
  edition 0.5
```

`ita.lock`

```una
-- Written by `una ita`; do not edit.
lock 1

solver
  encoding "una-smt-1"
  name "z3"
  version "5.1.0"
  logic "QF_UFLIA"
  steps 5000000
  seed 0
```

`src/geometry.una`

```una
module geometry

pub kind Point
  x: Int
  y: Int

--- The ratio of a circle's circumference to its diameter.
pub const pi: F64 = 3.141592653589793

pub const max_items: Int in 1..1000 = 500

pub const origin: Point = Point(x: 0, y: 0)

pub area(radius: F64) -> F64 = pi * radius * radius

pub batch(n: Int in 1..max_items) -> Int
  requires n <= max_items
=
  n

pub moved(take p: Point) -> Point = Point(x: p.x + 1, y: p.y)

pub start() -> Point
=
  const p = origin
  moved(take p)
```

## Forms

```ebnf
constant_decl = [ "pub" ] "const" lower_ident ":" type "=" line_expr NEWLINE ;
```

- The type is required. The value is a **constant expression**: literals, `-`, the arithmetic of integers, of `F64` and of decimals of one scale, `and`, `or`, `not`, comparisons, constructors and lists of constant expressions, parentheses, and the names of other constants, declared before or after (`spec/10 §8`).
- A constant is a lower-case name of the module, like a function (`spec/10 §11`). `pub` exports it, and `import m (name)` brings it in.
- A constant of a scalar type (an integer, a float, a decimal, a `Bool`, a `Text` or `Unit`) may appear in `requires`, `ensures` and `where`, where it is its literal, and as a bound of a type's range, as in `Int in 1..max_items` (`spec/02 §2`). A record or list constant there is U2014, as the record or list it stands for is: the refinement language has no records or lists (`spec/12 §3`).

## Examples

A call is not a constant expression: compute such a value in a function, and call it where it is needed.

`ita.una`

```una
ita geometry
  version 0.1.0
  edition 0.5
```

`ita.lock`

```una
-- Written by `una ita`; do not edit.
lock 1

solver
  encoding "una-smt-1"
  name "z3"
  version "5.1.0"
  logic "QF_UFLIA"
  steps 5000000
  seed 0
```

`src/geometry.una`

```una
-- fails with U2041
module geometry

pub root(x: F64) -> F64 = x.sqrt()

pub const root_two: F64 = root(2.0)
```

Two constants that name each other have no order in which to compute them:

`ita.una`

```una
ita geometry
  version 0.1.0
  edition 0.5
```

`ita.lock`

```una
-- Written by `una ita`; do not edit.
lock 1

solver
  encoding "una-smt-1"
  name "z3"
  version "5.1.0"
  logic "QF_UFLIA"
  steps 5000000
  seed 0
```

`src/geometry.una`

```una
-- fails with U1011
module geometry

pub const width: Int = height + 1

pub const height: Int = width - 1
```

## Edge cases

- **Exact values.** `1 / 0`, a value that does not fit a fixed-width type (`const n: I32 = 3_000_000_000`), an integer of more than 4,300 digits and a value of more than 1,000,000 elements, fields and characters are U2041 when the program is checked, never at run time.
- **Refinements.** A refined type, as `Int in 1..1000`, is proven of the value once (`spec/12 §3`); `= 0` there is U5001.
- **Ownership.** A constant is not a binding of any function: assigning to it, or passing it with `edit`, is U2019, and `take` of it is U2020; copy it to a local `const` first (`spec/17 §2`).
- **Authority.** A constant's type holds no function type, and its value no capability reference (U3005): a constant never carries authority.

## Diagnostics

- [U1011](../../../../crates/una-explain/explain/U1011.md): a constant depends on itself, directly or through other constants.
- [U2041](../../../../crates/una-explain/explain/U2041.md): a value that is not a constant expression, divides by zero, does not fit its type or is too large, or a type that holds a function type.

## Edition

Edition `0.5`, a preview (`spec/13 §7`). In editions `0.1` to `0.4`, `const` at the top level is U0101, and in edition `0.4` `std.time`'s `millisecond`, `second` and `minute` are built-in names; edition `0.5` declares them as constants.

## Normative source

- `spec/02 §1`: the grammar of a module constant; `§2`: a range bound that names one.
- `spec/10 §8`: constant expressions, their evaluation and typing; `§11`: names and cycles.
- `spec/12 §3`: the obligation of a refined constant; `spec/17 §2`: ownership.
- Decision record: ADR 0126, with ADR 0137 for the word `const`.
