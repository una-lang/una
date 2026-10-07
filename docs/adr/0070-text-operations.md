# 0070. Text operations

- Status: Accepted
- Date: 2026-10-06
- Deciders: Leonardo (accepted on 2026-10-06, with in-place concatenation on the coordinator's proposal), on a proposal by Claude (Una-1)
- Resolves: issue #237 (Beta, epic #235)
- Amends: a new chapter `spec/22-text.md`; `spec/14 §1`, `spec/16 §4`; the guides with the implementation

## Context

The prelude has four text functions: `is_digits`, `starts_with`, `trim` and `to_int` (`spec/14 §1`), plus `.len` and interpolation. Agents write string processing constantly: split a line into fields, find a separator, replace, change case, parse a number with a useful error. Today they cannot do any of it, since `Text` has no indexing and no way to take it apart.

What a design must keep:

- **Text is Unicode scalar values.** `.len` counts them, and `Text` orders by them (`spec/03 §1`, ADR 0032). There is no `Char` type.
- **Calls.** Prelude functions are called with method syntax, as `t.trim()` is `trim(t)` (`spec/04 §3`). The arguments after the first are labeled. There is no overloading (charter §6).
- **Determinism.** Every engine gives the same result on every platform and build (`spec/16 §5`). A rule that depends on the Unicode version of the toolchain could change with a Rust upgrade.
- **Labels.** The text functions read their arguments' values, so their results carry the join of every label of every argument (ADR 0051, "the other functions").
- **Verification.** QF_UFLIA. `Text` is an uninterpreted sort with `len` (`spec/12 §4`), and a contract is a hypothesis after a call.
- **Edition 0.2 only.** Edition `0.1` keeps its prelude.

The proposal asked nine questions, X1 to X9, each with options and a recommendation A. Leonardo answered them on 2026-10-06, each with option A, and added X10 on the coordinator's proposal. The other options stay below, marked as not chosen.

## Decision

### X1. The form

**A (chosen): prelude functions, called with method syntax, as the four text functions are.**
- For example `line.split(on: ",")`, `name.find(text: "@")` and `t.replace(old: "-", new: " ")`.
- Unlike the methods of `Map` (ADR 0069) and the integer conversions (`spec/18 §4`), these need no receiver-specific resolution: their first parameter is a `Text`, and their names are new.

**B (not chosen):** built-in methods resolved by the receiver. That would allow names that other types also use, but no proposed name collides.

### X2. Positions and representation

Positions count something, and the runtime's representation decides what counting costs. Today `Text` is a Rust `String` in UTF-8, and `.len` counts scalar values on every call, in O(n). Below, n is the length of the text in scalar values and b its length in bytes (b ≤ 4n).

**A (chosen): positions count Unicode scalar values, as `.len` does, over UTF-8 with a cached count and an index built on demand.**
- **What is stored.** A text value stores its UTF-8 bytes, its scalar count, and whether every byte is ASCII, all computed once when the value is made. Every value is immutable, so these never go stale.
- **Costs.**
  - `.len` is O(1).
  - On an ASCII text, a scalar index is a byte index, so `slice` and every position are O(1) beyond the result's size.
  - On a non-ASCII text, the first positional call builds a checkpoint table: the byte offset of every 64th scalar, kept with the value. That is O(b) time and O(n/64) words of space, built at most once per value. Every later position is then O(1), at most 64 steps from a checkpoint.
- **Consequence.** A loop over `t.slice(from: i, to: i + 1)` for every `i` is O(n) in total, not O(n²), on every text. `chars()` (X3) gives all the one-scalar texts in O(b).

**B (not chosen): scalar positions over plain UTF-8, as today.** `.len` and every position cost O(n) per call, so the loop above is O(n²). This is the cost the performance rule rules out.

**C (not chosen):** byte offsets into UTF-8. Every position is O(1) with no cache, but `.len` and positions would disagree, and a slice could cut a scalar in half, which needs a new failure. Agents would have to reason about encodings.

**D (not chosen):** UTF-32, one word per scalar. Positions are O(1) with no index, but text takes about four times the memory of mostly-ASCII UTF-8, and every host operation would convert on the way in and out.

**E (not chosen):** grapheme clusters, which match what a person sees as a character. But they depend on the Unicode version's segmentation rules, which breaks determinism across toolchains, and indexing them is O(n) as well.

### X3. The functions

**A (chosen):**

| Function | Contract | Time, with X2 A |
| --- | --- | --- |
| `split(t: Text, on: Text) -> List[Text]` | `requires on.len > 0`; `ensures result.len >= 1` | O(b + m), two-way search |
| `join(parts: List[Text], with: Text) -> Text` | none | O(total bytes) |
| `ends_with(t: Text, suffix: Text) -> Bool` | none | O(m) |
| `contains_text(t: Text, part: Text) -> Bool` | none | O(b + m) |
| `find(t: Text, text: Text) -> Option[Int]` | `result` is `Some` exactly when `contains_text(t, part: text)` | O(b + m), plus O(1) for the scalar index from the checkpoints |
| `replace(t: Text, old: Text, new: Text) -> Text` | `requires old.len > 0`; every occurrence, left to right, without overlaps | O(b + m + result) |
| `slice(t: Text, from: Int, to: Int) -> Text` | `requires 0 <= from and from <= to and to <= t.len`; `ensures result.len == to - from` | O(result), after the index is built once |
| `to_upper(t: Text) -> Text`, `to_lower(t: Text) -> Text` | `ensures result.len == t.len` (X4) | O(b) |
| `chars(t: Text) -> List[Text]` | `ensures result.len == t.len`; each element is one scalar value | O(b) |
| `parse_int(t: Text) -> Int fails ParseError` | X5 | O(b) |
| `trim_start(t: Text) -> Text`, `trim_end(t: Text) -> Text` | `ensures result.len <= t.len` | O(result + trimmed) |
| `bytes(t: Text) -> List[U8]` (X7) | `ensures result.len >= t.len` | O(b) |
| `.len` | as today | O(1) |

m is the byte length of the other text argument. Space is O(result) for each function, and every result's own count and ASCII flag are computed as it is built, at no extra asymptotic cost. Rust's `str::find` and `str::split` use the two-way algorithm, linear in the worst case.

- **Names.** `contains` is already the prelude's generic list membership (`spec/20 §6`), so a text one needs its own name, here `contains_text`. `split(on:)` and `join(with:)` read as calls.
- **Empty separators.** An empty `on` or `old` would match everywhere, so it is a precondition, proven at the call, not a run-time rule.
- **No regular expressions.** Patterns are a library of their own, with no Beta need.

**B (not chosen):** the minimal set of the issue only (`split`, `join`, `find`, `replace`, case, `parse_int`, slicing), without `chars`, `trim_start`, `trim_end` and `contains_text`. Smaller, but agents would rebuild those from `find` and `slice`.

### X4. Case mapping

**A (chosen): ASCII only.** `to_upper` and `to_lower` map `a`–`z` to `A`–`Z` and back, and leave every other scalar value as it is. They keep the length, which the verifier can use, and their result never depends on a Unicode version.

**B (not chosen):** full Unicode case mapping. It is right for every script, but `ß` becomes `SS`, so the length changes, and the tables change with the Unicode version of the toolchain. It could come later under its own names, `to_upper_unicode` and `to_lower_unicode`. Both options are O(b).

### X5. Parsing integers

**A (chosen): `parse_int(t: Text) -> Int fails ParseError`, and `to_int` stays.**
- The prelude kind is `ParseError = Empty | NotADigit(at: Int) | NoDigits`.
  - The text is an optional `-` followed by ASCII digits, as `to_int` reads it (`spec/14 §1`).
  - Space is not trimmed: write `parse_int(trim(t))`.
  - `at` is the scalar index of the first character that is not a digit.
  - `NoDigits` is a lone `-`.
- It fails rather than returning a `Result`, so it composes with `if error` and `attempt` (`spec/06`). `attempt parse_int(t)` gives the `Result` when one is wanted.
- The fixed-width types parse through it and a narrowing conversion, with its `overflow` obligation (`spec/18 §4`), as `parse_int(t).to_u8()`, or through `checked`.

**B (not chosen):** return `Result[Int, ParseError]`. It is closer to the issue's wording, but every caller then writes `when`, and failures are the language's way to report errors.

**C (not chosen):** improve `to_int` alone, keeping `Option`. Agents would get no position for the error.

### X6. Labels

**A (chosen): the rule of the reading functions, unchanged.** Every result, including every element and the top of the lists that `split` and `chars` return, carries the join of every label of every argument (ADR 0051). The length of `split`'s result depends on the text's content, so it cannot be labeled more precisely than the whole.

- `parse_int` fails on the content of its argument, so a call on a labeled text is a failure under that label. Its failure is labeled like its result, and a caller under a public program counter handling it with `if error` raises the program counter by that label, as for any failing call (`spec/07 §3`).

**B (not chosen):** structural labels for `chars` and `split`, as ADR 0051 gives `map`. Not sound: their shape depends on every scalar value.

### X7. Bytes

**A (chosen): `bytes(t: Text) -> List[U8]`, the UTF-8 encoding, with `ensures result.len >= t.len`, and no conversion back for Beta.** Hashing and protocols get their bytes. A decoding function would need a failure for invalid UTF-8 and can come later.

**B (not chosen):** no bytes for Beta.

### X8. Verification

**A (chosen): only the contracts of X3, as hypotheses.** No theory of strings: the SMT logic stays QF_UFLIA, so `find`, `replace` and `split` are uninterpreted beyond their contracts. `slice`'s precondition is the obligation agents meet most, and its `ensures` gives the length the next call needs.

**B (not chosen):** the SMT theory of strings, which proves more about contents. Its decision procedures are far more expensive than QF_UFLIA's, and in the worst case undecidable with some extensions. It leaves QF_UFLIA and slows every query that touches `Text`, and ADR 0054 and the proof cache key would have to follow.

### X9. Native and edition

**A (chosen):**
- `una-rt` and the interpreter implement each function on Rust's `str`, walking scalar values. Whitespace for `trim`, `trim_start` and `trim_end` is Unicode `White_Space`, as `trim` is today, a property that has been stable across Unicode versions.
- The new names exist only in edition `0.2`. Edition `0.1`'s prelude is unchanged, so a user function named `split` there keeps its meaning.
- No new diagnostic code: a missing precondition is U5001, a wrong argument is U2001, and an unhandled `parse_int` failure is U3007.

**B (not chosen):** also add them to edition `0.1`. That would change the names in scope of every edition 0.1 package.

### X10. Appending in place

Added by Leonardo on 2026-10-06, on the coordinator's proposal.

- Una has no `+` on `Text`: a program builds a text with interpolation.
- An assignment that rebinds a text binding to an interpolation beginning with that binding, as `acc = "{acc}, {item}"`, appends to it. It does so in place when the binding's text is uniquely held, in both engines, in amortized O(1) per scalar value appended, and updates the cached count and ASCII flag as it goes. A shared text is copied first.
- This is the freedom of `spec/17 §7` rule 3, as for lists (#30), and is not observable.
- A loop of k appends is then O(total length) rather than O(k²).
- A scaling test of 10^4 to 10^6 appends in a loop, in both engines, fails on quadratic growth.

## Consequences

With X1 to X9 as A, and X10:
- Agents can split, join, search, replace, slice, change ASCII case and parse integers with a position for the error, in edition 0.2, with deterministic results on every engine.
- **Costs:**
  - fourteen prelude functions and one prelude kind;
  - their native and interpreter implementations;
  - their contracts in the verifier;
  - the cached scalar count, ASCII flag and checkpoint table of X2 in both engines, which also make today's `.len` O(1).
- **Tests first:**
  - conformance cases per function, including empty texts, multi-scalar characters and boundaries;
  - label cases for each result and for `parse_int`'s failure;
  - the preconditions proven and refuted;
  - interpreter and native parity on non-ASCII texts;
  - a scaling test that indexes every scalar of ASCII and non-ASCII texts of 10^4 to 10^6 scalars, in both engines, and fails on quadratic growth, as the performance rule requires.
- **An independent adversarial check before merge.**
- **Deferred:** Unicode case mapping (X4 B), grapheme clusters (X2 C), regular expressions, UTF-8 decoding (X7), and the SMT theory of strings (X8 B).

## Alternatives considered

The B and C options under each question. Also:
- **A `Char` type for scalar values.** `chars()` returns one-scalar texts instead. A `Char` would need its own literals, ordering and conversions, for a gain that `Text` of length 1 mostly gives.

## Amendment (2026-10-07)

Three parameters of X3 are renamed (the coordinator under the owner's delegation, N-20):
- `join(parts, separator: Text)`;
- `slice(t, start: Int, end: Int)`, with `requires 0 <= start and start <= end and end <= t.len` and `ensures result.len == end - start`;
- `replace(t, pattern: Text, by: Text)`, with `requires pattern.len > 0`.

**Why.** `with`, `from` and `old` are keywords of edition 0.2. A parameter with one of those names could not be named in its function's contracts or body: `requires old.len > 0` reads as `old(...)`. Reading a keyword as a label before `:` would allow the label, but not those uses, and would make the keyword depend on context for every tool. None of the new names is a keyword in any edition. The functions were not implemented when this was decided, so no program changes.
