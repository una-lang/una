# 23. JSON

Status: Normative, v0.2 (ADR 0073 and its amendments of 2026-10-07).

The prelude reads and writes JSON (RFC 8259): a kind for JSON values, a parser that fails with a declared kind, and one canonical way of writing.

**Editions.** The constructs of this chapter exist only in packages of edition `0.2` and later (`spec/13 §7`). In edition `0.1`, `Json`, `JsonNumber`, `JsonError`, `parse_json`, `to_json`, `quote_json` and `is_json_number` are ordinary names.

## 1. The kind

1. **Values.** The prelude declares:

   ```una
   pub kind Json =
     | Null
     | Boolean(value: Bool)
     --- An exact integer: a JSON number with neither a fraction nor an exponent. Other numbers are `Decimal`.
     | Number(value: Int)
     | Decimal(text: JsonNumber)
     | Str(value: Text)
     | Array(items: List[Json])
     | Object(fields: Map[Text, Json])

   pub kind JsonNumber = Text where is_json_number(it)
   ```

   `is_json_number(t: Text) -> Bool` is an opaque prelude predicate. It is true exactly when `t` matches the `number` production of RFC 8259 §6 and has a fraction or an exponent, so `Number` and `Decimal` never hold the same number (ADR 0073 amendment).
2. **Names.** A user kind named `Json`, `JsonNumber` or `JsonError` is U1002, as for `Map` (`spec/21 §1`). A user variant named like a variant of `Json` or `JsonError` shadows it in its module, as a user `Some` shadows `Option`'s (ADR 0073, amendment of 2026-10-07).
3. **Numbers.** A JSON number with neither a fraction nor an exponent is `Number`, exact at any size. Every other number is `Decimal`, whose `text` is the number as written in the input.
4. **Objects** are maps ordered by their keys (`spec/21 §1` rule 3), so the order of the input is not kept.
5. **Equality** is structural, as for any kind. Two `Decimal`s are equal when their texts are: `1.0` and `1.00` differ.

## 2. Functions

| Function | Type | Contract |
| --- | --- | --- |
| `parse_json(t: Text)` | `Json fails JsonError` | §3 |
| `to_json(j: Json)` | `Text` | §4 |
| `quote_json(t: Text)` | `Text` | the string literal of §4 rule 2 |

They are prelude functions, called with method syntax as those of `spec/22 §2` are: `t.parse_json()`, `j.to_json()`, `t.quote_json()`.

## 3. Parsing

1. **Grammar.** `parse_json(t)` reads exactly one JSON value (RFC 8259 §2 to §7), with optional whitespace (U+0020, U+0009, U+000A, U+000D) before and after it, and nothing else. There are no comments, no trailing commas, no `NaN` or `Infinity`, no single quotes, and no byte order mark. A number has no leading zero before other digits, and no leading `+`.
2. **Strings.**
   - The escapes are `\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t` and `\uXXXX`, with hex digits in either case. A high surrogate escape followed by a low one gives one scalar value.
   - A lone surrogate, a raw scalar value below U+0020, or any other escape is a syntax error.
3. **Duplicate keys.** An object with two equal keys, compared after unescaping, fails with `DuplicateKey` at the second one.
4. **Depth.** Arrays and objects nest at most 512 deep. The bracket that opens the 513th level fails with `TooDeep` at that bracket.
5. **Failures.** The prelude declares:

   ```una
   pub kind JsonError = Syntax(at: Int) | TooDeep(at: Int) | DuplicateKey(at: Int) | NumberTooLong(at: Int)
   ```

   - `at` is a position in scalar values, as in `spec/22 §1`.
   - The failure is the first error in reading order: `Syntax` at the first scalar value where no valid JSON text can continue, or at the end of `t` when it ends early. In a `\u` escape, that is the first character that cannot continue a valid escape: `"\uD800` fails at 7, `"\uDC00` at 4, the `C` that makes it a lone low surrogate.
   - `NumberTooLong` at the first character of an integer of more than 4300 digits. A number with a fraction or an exponent has no such limit. (2026-10-18, #914.) The same bound, 4300 digits, holds for `parse_int` (`spec/22 §4`) and `to_int` (`spec/14 §1`). It bounds reading only: `to_json` writes an `Int` of any size, whose text `parse_json` then refuses (§6 rule 1, 2026-11-26, #1223).
6. **Size.** There is no limit beyond the memory budget (`spec/11 §7`). `parse_json` and `to_json` check the budget as they go, every 4096 values and every 4096 scalar values of a string, so a run over its budget ends with U6003 at the call (#314).

## 4. Writing

1. **Canonical form.** `to_json(j)` writes `j` with no whitespace:
   - `Null` as `null`, and `Boolean` as `true` or `false`;
   - `Number` in decimal, with a leading `-` when negative;
   - `Decimal` as its `text`;
   - `Str` as the string literal of rule 2;
   - `Array` as `[`, its items separated by `,`, then `]`;
   - `Object` as `{`, its entries in key order (`"key":value`, with the key written as in rule 2) separated by `,`, then `}`.
2. **String literals.**
   - A string is written between `"` characters.
   - `"` and `\` are escaped with a backslash.
   - U+0008, U+000C, U+000A, U+000D and U+0009 are written as `\b`, `\f`, `\n`, `\r` and `\t`.
   - Every other scalar value below U+0020, and U+007F, U+2028, U+2029, U+202A to U+202E and U+2066 to U+2069, are written as `\u` followed by four lowercase hex digits.
   - Every other scalar value is written as itself.
3. **Round trip.** `parse_json(to_json(j)) == j` for every `j` whose arrays and objects nest at most 512 deep; `to_json` writes deeper values, which `parse_json` then refuses with `TooDeep`. For every `t` that `parse_json` accepts, `to_json(parse_json(t))` is a fixed point of `to_json ∘ parse_json`.
4. **Interpolation** of a `Json` writes its Una form (`spec/11 §5`), not JSON.

## 5. Information flow

1. **Parsing.** The result of `parse_json(t)` and its failure are labeled with the labels of `t` (`spec/14 §1`). A value bound from the result by `when` takes that label as its top label (`spec/07 §7` rule 3). Whether the call fails, and where, depends on `t`:
   - `attempt` gives a `Result` labeled like `t`;
   - `if error use` runs under a program counter raised by those labels, and its value carries them;
   - `if error pass` and `if error fail` are U4002 when those labels are not `public` (`spec/07 §3` rule 3).
2. **Constructing.** In editions `0.2` and `0.3`, the positions of `Json` are unlabeled, so `public`. A labeled value given to a `Json` constructor is U4001 (`spec/07 §7` rule 2). A labeled text reaches JSON through a policy (`spec/07 §5`), or through `quote_json` and interpolation.
   (v0.4, ADR 0073 J5 as amended on 2026-11-09, accepted on 2026-11-10, #917.) In edition `0.4` and later, a `Json` value has one label, its top label; its positions stay `public` in its type, so no signature changes. A call of `Boolean`, `Number`, `Decimal`, `Str`, `Array` or `Object` accepts arguments at any label, and its result is labeled with the join of every label of every argument (`spec/07 §7` rule 3: list elements, map keys and values, the top labels of nested `Json` values) and of nothing else. `Null`, `Array([])` and `Object` of an empty map are `public`; a constructor under a raised program counter is raised by the branch join (`spec/07 §7` rule 2), as any expression is. Refinements are kept: the text of `Decimal` still proves `is_json_number`. A value read back from a labeled `Json` is raised by its top label (`spec/07 §7` rule 3) and `to_json` carries it (rule 3), so the U4001 or U4003 of a labeled `Json` is where it reaches a lower declared type or a sink, never at the constructor. The rule is coarser than a label per position: a `public` field beside a `secret` one reads as `secret`. User kinds and the other kinds of the standard library keep their fields' declared labels.
3. **Writing.** The result of `to_json(j)` carries the labels of `j`, and the result of `quote_json(t)` those of `t`.

## 6. Dynamic semantics

1. **Complexity.** For an input of n scalar values, with an object of k keys counted once for each object:
   - parsing takes O(n + Σ k log k) time, which is O(n) when every object's keys arrive in order, as `to_json` writes them;
   - writing takes O(n) time for an output of n scalar values, except that writing a `Number` of d digits takes O(M(d) log d), M(d) the cost of a product of d-digit integers, for an `Int` of any size (`spec/11 §7`, 2026-11-26, #1223; from #923 to #1223 an `Int` of more than 4300 digits was never written);
   - both take O(n) space.
2. **No native recursion.** Parsing and writing keep their own stack on the heap, and never recurse on the native stack over a value's depth (ADR 0060).
3. **Equivalence.** Both engines use one implementation, and give the same value, text or failure for every input (`spec/16 §5`).
4. **Purity.** The functions of §2 perform no operation and record no trace event.

## 7. Verification

The functions are uninterpreted beyond their types (`spec/12 §2`). `is_json_number` is opaque, so a `JsonNumber` is made by parsing or by `checked` (`spec/11 §4`).
