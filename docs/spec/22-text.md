# 22. Text operations

Status: Accepted, not implemented in v0.2.0; planned (#237). Normative for edition `0.2` once implemented (ADR 0070).

These prelude functions take a text apart, search it, change it and parse it. Text stays a value of Unicode scalar values (`spec/03 §1`), and every function here is pure.

**Editions.** The functions and the kind of this chapter exist only in packages of edition `0.2` (`spec/13 §7`). In edition `0.1`, their names are ordinary names, and §6 does not apply.

## 1. Positions

1. **Scalar values.** A position in a text counts Unicode scalar values from 0, as `.len` does. `t.len` is the number of scalar values of `t`.
2. **Not bytes, not graphemes.** No function takes or returns a byte offset or a grapheme index. `bytes()` (§2) gives the UTF-8 encoding as a list.

## 2. Functions

They are prelude functions, called with method syntax, as `line.split(on: ",")` is `split(line, on: ",")` (`spec/04 §3`). In the table, n is the length of `t` in scalar values, b its length in UTF-8 bytes, and m the length in bytes of the other text argument.

| Function | Contract | Time |
| --- | --- | --- |
| `split(t: Text, on: Text) -> List[Text]` | `requires on.len > 0`; `ensures result.len >= 1` | O(b + m) |
| `join(parts: List[Text], with: Text) -> Text` | none | O(total bytes) |
| `ends_with(t: Text, suffix: Text) -> Bool` | none | O(m) |
| `contains_text(t: Text, part: Text) -> Bool` | none | O(b + m) |
| `find(t: Text, text: Text) -> Option[Int]` | `result` is `Some` exactly when `contains_text(t, part: text)` | O(b + m) |
| `replace(t: Text, old: Text, new: Text) -> Text` | `requires old.len > 0` | O(b + m + result) |
| `slice(t: Text, from: Int, to: Int) -> Text` | `requires 0 <= from and from <= to and to <= t.len`; `ensures result.len == to - from` | O(result) |
| `to_upper(t: Text) -> Text` | `ensures result.len == t.len` | O(b) |
| `to_lower(t: Text) -> Text` | `ensures result.len == t.len` | O(b) |
| `chars(t: Text) -> List[Text]` | `ensures result.len == t.len` | O(b) |
| `parse_int(t: Text) -> Int fails ParseError` | §4 | O(b) |
| `trim_start(t: Text) -> Text` | `ensures result.len <= t.len` | O(result + trimmed) |
| `trim_end(t: Text) -> Text` | `ensures result.len <= t.len` | O(result + trimmed) |
| `bytes(t: Text) -> List[U8]` | `ensures result.len >= t.len` | O(b) |

1. **`split`** returns the parts between non-overlapping occurrences of `on`, left to right, including empty parts: `"a,,b".split(on: ",")` is `["a", "", "b"]`, and a text without `on` is a list of itself. `join` is its inverse: `t.split(on: s).join(with: s) == t`.
2. **`find`** returns the position of the first occurrence of `text`, and `Some(0)` for an empty `text`.
3. **`replace`** replaces every non-overlapping occurrence of `old`, left to right.
4. **`slice`** returns the scalar values at positions `from` up to, not including, `to`. Its precondition is an obligation at each call (`spec/12 §3`).
5. **`chars`** returns one text of length 1 per scalar value, in order.
6. **`trim_start`** and **`trim_end`** remove Unicode `White_Space` at one end, as `trim` does at both (`spec/14 §1`).
7. **`bytes`** returns the UTF-8 encoding.
8. **The search algorithm** is linear in the worst case, such as the two-way algorithm. A naive quadratic search is not allowed.

## 3. Case

`to_upper` maps the ASCII letters `a` to `z` to `A` to `Z`, and `to_lower` maps them back. Every other scalar value is left as it is, so the length never changes and the result does not depend on a Unicode version.

## 4. Parsing integers

1. The prelude declares `kind ParseError = Empty | NotADigit(at: Int) | NoDigits`.
2. `parse_int(t)` reads an optional `-` followed by one or more ASCII digits, as `to_int` does (`spec/14 §1`), and returns the integer. Otherwise it fails:
   - `Empty` when `t` is empty;
   - `NoDigits` when `t` is `-` alone;
   - `NotADigit(at: i)` at the first position `i` that is neither the leading `-` nor a digit.
3. Space is not trimmed: `parse_int(trim(t))` reads a padded number.
4. The result is an unbounded `Int`. A fixed-width integer is parsed through a narrowing conversion, with its `overflow` obligation (`spec/18 §4`).
5. `to_int` keeps its meaning.

## 5. Information flow

1. **Results.** The result of every function in §2 carries the join of every label of every argument. For `split` and `chars`, that includes the list's top label and every element, since their shape depends on every scalar value (`spec/14 §1`, "the other functions").
2. **Failures.** `parse_int` fails depending on its argument's value, so its failure is labeled like its result. Handling it raises the program counter by that label (`spec/07 §3`).

## 6. Dynamic semantics

1. **Representation.** The interpreter and the native runtime keep a text as UTF-8 bytes together with its scalar count and whether every byte is ASCII, computed when the text is made. The first positional operation on a non-ASCII text builds an index of the byte offset of every 64th scalar value, kept with the value. These are not observable, but they bound the cost:
   - `.len` is O(1);
   - a position is O(1) on an ASCII text, and O(1) on any text once its index is built, in O(b) at most once.
2. **Appending in place.** An assignment that rebinds a text binding to an interpolation that begins with that binding, as `acc = "{acc}, {item}"`, appends to it.
   - When the binding's text is uniquely held, the append happens in place, in both engines, in amortized O(1) per scalar value appended. The scalar count and the ASCII flag are updated, not recomputed.
   - Otherwise the text is copied first.
   - This is the freedom of `spec/17 §7` rule 3: it changes no output, failure or trace.
   - So a loop of k such appends is O(total length), not O(k²).
3. **Equivalence.** Both engines give the same text for every operation, by the rules of §2 to §4 (`spec/16 §5`).

## 7. Verification

The contracts in §2 are hypotheses after each call, and `slice`'s precondition is an obligation. There is no theory of strings: beyond the contracts, the functions are uninterpreted (`spec/12 §2`).
