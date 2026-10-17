---
construct: requirement and derived from
editions: ["0.1", "0.2"]
spec: ["spec/08 §1", "spec/02 §5", "spec/02 §3", "spec/13 §5", "spec/13 §6"]
codes: [U1002, U1006, U1007]
---

# `requirement` and `derived from`

## What it does

A `requirement` records a human or business statement, with its source and its owner, so contracts can be traced to intent. A function names the requirements it implements with `derived from`. `una trace` then lists, for one requirement, every function derived from it, with its contract and the status of each proof.

```una
module shop

requirement EVENTS_3
  text "An event with no remaining seats is always shown as sold out"
  source policy "Box Office Handbook 2.1"
  owner box_office
  status active

pub kind Availability = SoldOut | Few | Open

pub availability(remaining: Int) -> Availability
  requires remaining >= 0
  ensures remaining == 0 implies result is SoldOut
  derived from EVENTS_3
=
  if remaining == 0 then SoldOut else if remaining < 20 then Few else Open
```

## Forms

```ebnf
requirement_decl = "requirement" upper_ident NEWLINE INDENT
                     "text" text_lit NEWLINE
                     "source" source NEWLINE
                     "owner" lower_ident NEWLINE
                     "status" ( "proposed" | "active" ) NEWLINE
                   DEDENT ;
source          = "human" | ( "policy" | "regulation" | "generated" ) text_lit ;
```

The fields:

| Field | Values |
| --- | --- |
| `text` | The statement, as written by its source |
| `source` | `human`, `policy "<document>"`, `regulation "<document>"`, `generated "<task reference>"` |
| `owner` | Team or person accountable (lower snake case) |
| `status` | `proposed` or `active` |

**`derived from R1, R2`** is the last clause of a function header, at most once.

## Examples

The function above proves the requirement's intent as a contract: `ensures remaining == 0 implies result is SoldOut`. The requirement says what people want; the contract says what the code guarantees, and the verifier checks the contract.

A requirement written by an agent is only proposed. A person activates it by changing its source:

```una
-- fails with U1007
module shop

requirement GEN_1
  text "Seats are counted"
  source generated "task 42"
  owner agents
  status active
```

## Edge cases

**`derived from` names declared requirements**:

```una
-- fails with U1006
module shop

pub seats_left(remaining: Int) -> Int
  derived from NOPE_1
=
  remaining
```

**Identifiers are unique per package**:

```una
-- fails with U1002
module shop

requirement SHOP_1
  text "Prices include sales tax"
  source human
  owner team
  status active

requirement SHOP_1
  text "Prices show sales tax apart"
  source human
  owner team
  status active
```

Further rules:

- **Proposed requirements** do not block a build: functions derived from them compile normally, and the build report flags them.
- **`una trace R`** checks the package at tier 1, then prints the requirement and each function derived from it, with its `requires`, `ensures` and the status of each obligation. With `--json` it prints `una.trace/1`.
- **Evidence is separate.** A requirement states intent, a contract guarantees it, and examples show it running. See [`example`](../examples/example.md).

## Diagnostics

- [U1002](../../../../crates/una-explain/explain/U1002.md): a requirement identifier defined twice in a package.
- [U1006](../../../../crates/una-explain/explain/U1006.md): `derived from` names an undeclared requirement.
- [U1007](../../../../crates/una-explain/explain/U1007.md): a requirement with source `generated` and status `active`.

## Edition

Requirements are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/08 §1`: requirements, rules 1 to 4, and `una trace`.
- `spec/02 §5`: the grammar; `spec/02 §3`: the `derived from` clause.
- `spec/13 §5`: the command line; `spec/13 §6`: the build report.
