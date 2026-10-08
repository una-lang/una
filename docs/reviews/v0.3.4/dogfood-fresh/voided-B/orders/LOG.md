# orders: log

DOC: una-0.3.3/docs is missing, so the JSON API (`parse_json`, the variants of `Json`, `to_json`) and the list API (`append`) are described nowhere I could read. `una explain` mentions `Json` only in passing (U4001) and never names its functions. I found them through deliberate compiler errors (U1001, U2007, U2001).
Before writing: una explain U0007 (how to write `{` and `}` in text, so JSON in a literal is `"\{\"id\": 7\}"`), una explain U3015 (example syntax: `example`, `do`, `expect result == ...`).

## Run 1
- U3007: `parse_json` can fail with `JsonError`; handle the failure with an `if error` clause, or wrap the c
- U1001: `Json` has no field or function `to_text`, and no function `to_text` in scope applies
- Consulted: message only.
- Enough: partly. U3007 was clear. U1001 confirmed that `parse_json` exists but gave no hint for the writer's name.
- Changed: handled the error with `if error use`, and added a deliberately incomplete `when` over `Json` so that U2007 lists the variants.

## Run 2
- U1001: cannot find name `JNull` in this module, its imports or the prelude
- U1001: cannot find name `write_json` in this module, its imports or the prelude
- U2007: `when` does not match variants `Boolean`, `Number`, `Decimal`, `Str`, `Array`, `Object` of `Json`
- Consulted: message only.
- Enough: partly. U2007 gave every variant. The writer's name was still unknown.
- Changed: used `Null`; bound each variant's payload; tried six candidate writer names at once.

## Run 3
- U1001: cannot find name `json_text` in this module, its imports or the prelude
- U1001: cannot find name `render_json` in this module, its imports or the prelude
- U1001: `Json` has no field or function `write`, and no function `write` in scope applies
- U1001: cannot find name `show_json` in this module, its imports or the prelude
- U1001: cannot find name `format_json` in this module, its imports or the prelude
- Consulted: message only.
- Enough: yes. `to_json` was the only candidate it did not reject, so that name exists.
- Changed: returned each payload where an `Int` is expected, and bound `to_json(doc)` to an `Int`, so that U2001 names the types.

## Run 4
- U2001: `b` is declared as `Int`, found `Text`
- U2001: `kind_of` returns `Int`, found `Bool`
- U2001: `kind_of` returns `Int`, found `Text` (Decimal)
- U2001: `kind_of` returns `Int`, found `Text` (Str)
- U2001: `kind_of` returns `Int`, found `List[Json]`
- U2001: `kind_of` returns `Int`, found `Map[Text, Json]`
- Consulted: message only.
- Enough: yes. Json = Null | Boolean(Bool) | Number(Int) | Decimal(Text) | Str(Text) | Array(List[Json]) | Object(Map[Text, Json]), and to_json returns Text.
- Changed: wrote the real implementation, with `OrderError`, small Option helpers, a `for` loop, and `Map` built with `empty_map`/`set`. I guessed `edit names.push(...)` for lists.

## Run 5
- U1001: `List[Json]` has no field or function `push`, and no function `push` in scope applies
- Consulted: message only.
- Enough: partly. No suggestion of the right name.
- Changed: tried `edit names.append(...)`.

## Run 6
- U2011: this statement has type `List[Json]`, and only `Unit` values can be discarded; bind the value with `
- U2020: parameter `xs` of `append` does not take its argument by `edit`
- U2004: argument 2 of `append` needs a label; only the first argument may go without one
- Consulted: message only.
- Enough: yes. Together they show `append(xs, item:)` returns a new list.
- Changed: `names = names.append(item: Str(name))`.
DOC: README.md says "lists and texts updated in place", which made me expect an `edit` method for lists. `append` is a pure function returning a new list. Nothing I could read says which list operations exist.

## Run 7
- Diagnostics: none (exit 0). PASS.

## una test
- `example orders "two lines" ... ok`: 1 passed, 0 failed, exit 0. The example checks exactly the input and output the task gives.
