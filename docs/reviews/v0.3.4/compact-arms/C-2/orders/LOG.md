# orders LOG

## Check run 1
- U0101: nested patterns are not supported in v0.1
- U0007 (x4): unescaped `}` in text; write `\}`
- Consulted: message only (repairs given); una explain U0007 to confirm
- Enough: yes
- Change: escaped every `}` in the JSON text literals; split the nested `Some(Array(items))` pattern into two `when`s via helper functions.
- DOC: una-compact.md §Syntax lists `\}` as an escape but does not say a literal `}` must always be escaped in text.

## Check run 2
- U1001 (x6): cannot find type `Malformed` / variant `Malformed` in this module, its imports or the prelude
- U2002 (x2): `fails` names a sum kind, and `OrderError` is not one
- Consulted: una explain U2002 (says `fails` needs a sum kind, does not say a one-variant kind is an alias)
- Enough: partly; inferred that `kind OrderError = Malformed` is read as an alias to a type named Malformed
- Change: gave the kind two variants, `Malformed | Missing`.
- DOC: una-compact.md §Functions shows `kind P = A(x: Int) | B` but does not say that `kind E = A` with one bare variant is an alias, not a sum.

## Check run 3
- No diagnostics; exit 0.
- Consulted: n/a
- Enough: yes
- Change: none. (Learned by acceptance: `Object` holds a `Map[Text, Json]`, `Number` holds an `Int`.)
- DOC: una-compact.md §Library lists the Json variants but not their payload types (`Object(Map[Text, Json])`, `Number(Int)`, `Array(List[Json])`).

## una test (after run 3)
- `example orders "two lines" ... ok`, 1 passed, exit 0.

## Check run 4
- No diagnostics; exit 0.
- Consulted: n/a
- Enough: yes
- Change: a `qty` that is not a number now fails `Malformed` instead of counting as 0.

## una test (after run 4)
- `example orders "two lines" ... ok`, 1 passed, exit 0.
