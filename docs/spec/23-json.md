# 23. JSON

Status: Normative, v0.2 (ADR 0073). Not implemented yet; planned (#238).

The prelude reads and writes JSON (RFC 8259): a kind for JSON values, a parser that fails with a declared kind, and one canonical way of writing.

**Editions.** The constructs of this chapter exist only in packages of edition `0.2` (`spec/13 §7`). In edition `0.1`, `Json`, `JsonNumber`, `JsonError`, `parse_json`, `to_json`, `quote_json` and `is_json_number` are ordinary names.

## 1. The kind

1. **Values.** The prelude declares:

   ```una
   pub kind Json =
     | Null
     | Bool(value: Bool)
     | Int(value: Int)
     | Decimal(text: JsonNumber)
     | Str(value: Text)
     | Array(items: List[Json])
     | Object(fields: Map[Text, Json])

   pub kind JsonNumber = Text where is_json_number(it)
   ```

   `is_json_number(t: Text) -> Bool` is an opaque prelude predicate. It is true exactly when `t` matches the `number` production of RFC 8259 §6.
2. **Names.** A user kind named `Json`, `JsonNumber` or `JsonError` is U1002, as for `Map` (`spec/21 §1`).
3. **Numbers.** A JSON number with neither a fraction nor an exponent is `Int`, exact at any size. Every other number is `Decimal`, whose `text` is the number as written in the input.
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
   pub kind JsonError = Syntax(at: Int) | TooDeep(at: Int) | DuplicateKey(at: Int)
   ```

   - `at` is a position in scalar values, as in `spec/22 §1`.
   - The failure is the first error in reading order: `Syntax` at the first scalar value where no valid JSON text can continue, or at the end of `t` when it ends early.
6. **Size.** There is no limit beyond the memory budget (`spec/11 §7`).

## 4. Writing

1. **Canonical form.** `to_json(j)` writes `j` with no whitespace:
   - `Null` as `null`, and `Bool` as `true` or `false`;
   - `Int` in decimal, with a leading `-` when negative;
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

1. **Parsing.** The result of `parse_json(t)` and its failure are labeled with the labels of `t` (`spec/14 §1`). A value bound from the result by `when` takes that label as its top label (`spec/07 §7` rule 3).
2. **Constructing.** The positions of `Json` are unlabeled, so `public`. A labeled value given to a `Json` constructor is U4001 (`spec/07 §7` rule 2). A labeled text reaches JSON through a policy (`spec/07 §5`), or through `quote_json` and interpolation.
3. **Writing.** The result of `to_json(j)` carries the labels of `j`, and the result of `quote_json(t)` those of `t`.

## 6. Dynamic semantics

1. **Complexity.** For an input of n scalar values, with an object of k keys counted once for each object:
   - parsing takes O(n + Σ k log k) time, which is O(n) when every object's keys arrive in order, as `to_json` writes them;
   - writing takes O(n) time for an output of n scalar values;
   - both take O(n) space.
2. **No native recursion.** Parsing and writing keep their own stack on the heap, and never recurse on the native stack over a value's depth (ADR 0060).
3. **Equivalence.** Both engines use one implementation, and give the same value, text or failure for every input (`spec/16 §5`).
4. **Purity.** The functions of §2 perform no operation and record no trace event.

## 7. Verification

The functions are uninterpreted beyond their types (`spec/12 §2`). `is_json_number` is opaque, so a `JsonNumber` is made by parsing or by `checked` (`spec/11 §4`).
