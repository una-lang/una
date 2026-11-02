# 14. Standard library (v0.1)

Status: Normative, v0.1. The standard library is part of the Trusted zone. Everything not listed here does not exist in v0.1.

## 1. Prelude

The prelude is imported into every module.

**Types:** `Int`, `Nat`, `Bool`, `Text`, `Unit`, `List[T]`, `Option[T]` (`Some(value)`, `None`), `Result[T, E]` (`Ok(value)`, `Err(failure)`), `Hashed`.

**Functions.** All are pure and cannot fail. Contracts shown are part of their signatures and are used as hypotheses by callers.

| Signature | Contract |
| --- | --- |
| `abs(x: Int) -> Int` | `ensures result >= 0 and (result == x or result == -x)` |
| `min(a: Int, b: Int) -> Int` | `ensures result <= a and result <= b and (result == a or result == b)` |
| `max(a: Int, b: Int) -> Int` | `ensures result >= a and result >= b and (result == a or result == b)` |
| `is_digits(t: Text) -> Bool` | none (use inside opaque predicates) |
| `starts_with(t: Text, prefix: Text) -> Bool` | none |
| `trim(t: Text) -> Text` | `ensures result.len <= t.len` |
| `to_int(t: Text) -> Option[Int]` | none |
| `is_empty[T](xs: List[T]) -> Bool` | `ensures result == (xs.len == 0)` |
| `first[T](xs: List[T]) -> Option[T]` | `ensures (result is None) == (xs.len == 0)` |
| `at[T](xs: List[T], index: Int) -> Option[T]` | `ensures (result is None) == (index < 0 or index >= xs.len)` |
| `append[T](xs: List[T], item: T) -> List[T]` | `ensures result.len == xs.len + 1` |
| `contains[T](xs: List[T], item: T) -> Bool` | none |
| `map[T, U](xs: List[T], each: fn(T) -> U) -> List[U]` | `ensures result.len == xs.len` |
| `filter[T](xs: List[T], keep: fn(T) -> Bool) -> List[T]` | `ensures result.len <= xs.len` |
| `first_where[T](xs: List[T], keep: fn(T) -> Bool) -> Option[T]` | none |
| `fold[T, U](xs: List[T], initial: U, step: fn(U, T) -> U) -> U` | none |
| `sum(xs: List[Int]) -> Int` | none |
| `is_some[T](o: Option[T]) -> Bool` | `ensures result == (o is Some)` |
| `or_else[T](o: Option[T], default: T) -> T` | none |
| `is_ok[T, E](r: Result[T, E]) -> Bool` | `ensures result == (r is Ok)` |

Higher-order prelude functions accept only pure, non-failing lambdas in v0.1 (effect polymorphism is deferred).

(v0.2, ADR 0062.) In edition `0.2`, `map`, `filter`, `fold` and `first_where` declare `uses of` and `fails of` their function parameter (`each`, `keep`, `step`), so they take effectful and failing functions.
- They call it left to right, once per element, except that `first_where` stops at the first accepted element.
- They stop at the first failure. Effects already performed stay performed (`spec/11 §5`).
- Their `ensures` hold on success. The program counter of the function's body is that of `spec/07 §7` rule 9.

(v0.2, ADR 0066.) In edition `0.2`, `min`, `max`, `contains` and `sum` are generic over constrained types, and the prelude gains `sort` (`spec/20 §6`). `abs` stays `Int`-only.

(v0.2, ADR 0069.) In edition `0.2`, the prelude gains `empty_map()` and the kind `Entry[K, V]`; the operations of a map are built-in methods of the receiver (`spec/21 §2`).

(v0.2, ADR 0070.) In edition `0.2`, the prelude gains the text functions of `spec/22 §2` and the kind `ParseError` (`spec/22 §4`).

(v0.2, ADR 0073.) In edition `0.2`, the prelude gains the kinds `Json`, `JsonNumber` and `JsonError`, the predicate `is_json_number`, and the functions `parse_json`, `to_json` and `quote_json` (`spec/23`).

(v0.2, ADR 0056.) The prelude gains `set[T](edit xs: List[T], index: Int, item: T) -> Unit`, with `requires 0 <= index and index < xs.len` and `ensures xs.len == old(xs).len` (`spec/17 §8`).

(v0.2, ADR 0057.) The conversions, wrapping, saturating and bit operations of fixed-width integers are built-in methods resolved by the receiver, not prelude functions (`spec/18 §4`, `§5`). `to_int` on `Text` keeps the meaning above.

**Result labels** (ADR 0038 D3, ADR 0051). The parameters of these functions that are not functions accept arguments of any label, and their results carry those labels as follows (`spec/07 §7` rule 8):

- **Structural functions** label each position of the result with what it holds. The element and accumulator positions carry the labels of their type variables, which every argument they come from joins (`spec/07 §7` rule 1). The top label is the top label of what decides the result's shape, so the labels of element values the function does not read never reach it:
  - `append`, `first` and `is_empty`: the top label of `xs`;
  - `at`: the top label of `xs` and every label of `index`;
  - `is_some`: the top label of `o`; `is_ok`: the top label of `r`;
  - `map`: the top labels of `xs` and `each`; `fold`: the label of the accumulator type, joined with the top labels of `xs` and `step`;
  - `filter` and `first_where`: the top labels of `xs` and `keep`, and the label of `keep`'s declared result. A predicate's result flows into that declared `Bool`, so a predicate over labeled element values is rejected there (U4001).
    (v0.2, ADR 0067.) In edition `0.2`, the predicate's result is not checked against `Bool`'s label: its label, symbolic labels included, joins the top label of the result instead, since which elements remain, or which one is found, depends on it. `first_where` calls `keep` only up to the first accepted element, so a predicate with effects or failures whose result is labeled is U4002 there (`spec/07 §3` rule 4).
  `.len` takes the top label of its operand (`spec/07 §7` rule 3).
- **The other functions** read element or argument values, and their result is labeled with the join of every label of their arguments, element types included: `contains`, `sum`, `or_else`, `min`, `max`, `abs` and the text functions `is_digits`, `starts_with`, `trim` and `to_int`. (v0.3, ADR 0114.) In edition `0.3`, so do `parse_f64`, `f64_from_bits`, `as_f64` and `json_number` (`spec/29`), and the failure of `parse_f64`, as `parse_int`'s.

The native implementations declare these labels explicitly.

Text works on Unicode scalar values: `.len` counts them; `is_digits` holds when every character is an ASCII digit, so it holds of the empty text; `trim` removes Unicode whitespace at both ends; `to_int` reads an optional `-` followed by ASCII digits; `starts_with` compares scalar values; `contains` and `==` are structural; `Text` orders by scalar values (ADR 0032).

**What the functions compute** (#692). Beyond their contracts, the prelude's functions compute the following, in the interpreter and in the native backend alike (`spec/16 §5`). Positions in a list count from 0.

- `first(xs)` is `Some` of the element at position 0, and `None` when `xs` is empty.
- `at(xs, index)` is `Some` of the element at position `index`, and `None` when `index` is negative or at least `xs.len`. It never fails.
- `append(xs, item)` is `xs` with `item` added after its last element.
- `map(xs, each)` is the list of `each(x)` for each element `x` of `xs`, in the order of `xs`.
- `filter(xs, keep)` is the list of the elements `x` of `xs` for which `keep(x)` is `true`, in the order of `xs`.
- `first_where(xs, keep)` is `Some` of the first element `x` of `xs` for which `keep(x)` is `true`, and `None` when there is none. `keep` is not called on the elements after that one.
- `fold(xs, initial, step)` starts from `initial` and, for each element `x` of `xs` from first to last, replaces the accumulator `acc` by `step(acc, x)`. The result is the last accumulator, so `initial` when `xs` is empty.
- `map`, `filter`, `first_where` and `fold` call their function parameter on the elements from first to last, at most once per element: `first_where` stops after the first element it accepts, and in edition `0.2` and later each stops at the first failure of its function (§1).
- `sum(xs)` is the exact sum of the elements as an `Int`, and `0` when `xs` is empty. `Int` has no bound, so the sum never overflows. Edition `0.2` keeps this meaning for its generic `sum` (`spec/20 §6` rule 1).
- `or_else(o, default)` is the value `v` when `o` is `Some(v)`, and `default` when `o` is `None`. `default` is an argument like any other, a value by the time of the call whatever `o` is (`spec/11 §4`); a call with effects is bound with `let` before it is given (U3013).
- `to_int(t)` is `Some(n)` when `t` is an optional `-` followed by one or more ASCII digits, leading zeros allowed, where `n` is the integer they write (`"-0"` is `0`). It is `None` for any other text, among them the empty text, `"-"`, a leading `+`, whitespace and digits outside ASCII. (Security fix, 2026-10-18, #914, in every edition.) A text of more than 4300 digits, the sign excluded and leading zeros included, is `None`, the bound of `parse_int` (`spec/22 §4`) and of a JSON integer (`spec/23 §3`): converting d digits takes O(d²), so a longer text is refused before any conversion.
- (ADR 0132.) Each function's time and space are bounded as §5 states.

The prelude's types mean:

- `Nat` is `Int where it >= 0` (`spec/03 §1`): an `Int` flows into `Nat` where the bound is proved (`spec/03 §2` rule 2).
- `Option[T]` is either `Some(value)`, which holds one value of `T`, or `None`, which holds none.
- `Result[T, E]` is either `Ok(value)`, which holds the success value of `T`, or `Err(failure)`, which holds the failure of `E`.

## 2. Built-in operations

`.len` on `Text` and `List`, arithmetic, comparisons, equality and text interpolation are built in (`spec/09 §2`). `Hashed` supports only `==` and `!=`; it cannot be interpolated or converted to `Text` (U2001).

## 3. Host capabilities (part of the prelude)

The host capabilities are declared in the module `std.host` and are in scope in every module through the prelude (`spec/10 §11` rule 8). A module MAY also import them, with their failure kinds, from `std.host` (`spec/10 §11` rule 10).

| Capability | Operation | Fails | Accepts | Notes |
| --- | --- | --- | --- | --- |
| `Console` | `print(line: Text) -> Unit` | | `public` | Writes the line and a newline to stdout |
| `Clock` | `now() -> Int` | | `public` | Milliseconds since the Unix epoch |
| `Env` | `get(name: Text) -> Option[internal Text]` | | `public` | Reads a process environment variable |
| `Files` | `read(path: Text) -> internal Text` | `FileError` | `public` | Path relative to the grant prefix |
| `Files` | `write(path: Text, content: Text) -> Unit` | `FileError` | `internal` | Path relative to the grant prefix |
| `Keys` | `hmac(key_name: Text, data: Text) -> internal Hashed` | `KeyError` | `public` for `key_name`, any label for `data` | HMAC-SHA-256 with a named key; the result carries the label of `data` |

```una
kind FileError = NotFound | Denied | Io
kind KeyError = | UnknownKey
```

- `Files` resolves a path against the prefix fixed by the grant (ADR 0038 D9):
  1. **Normalize without following links.** An absolute path, and a `..` segment that leaves the prefix, fail with `Denied`.
  2. **Resolve intermediate components.** Every intermediate directory is resolved with its symbolic links followed, and the resolved directory MUST lie inside the canonical grant prefix (`Denied` otherwise).
  3. **Never follow the final component.** The final component is opened without following a link: if it is a symbolic link, dangling or not, the operation fails with `Denied`. A write never creates a file through a link.
  4. **Races are out of the model.** A race between the check and the use of a path is outside the v0.1 model, since the interpreter is single-threaded and host capabilities are trusted; the build report lists it (`spec/13 §6`). (v0.2, ADR 0065.) Concurrent tasks may also operate on one path in any order (`spec/19 §3`), and the native runtime's host capabilities are safe to call from several tasks at once (`spec/19 §8`).
  5. **Hard links are out of the model.** A hard link inside the grant to a file outside it is an ordinary directory entry, so `Files` reads and writes it. Hard links are outside the v0.1 model, as races are: `Files` has no operation that creates a link, so only someone with other access to the granted directory can plant one, and the build report lists them (`spec/13 §6`, ADR 0048).
  6. **Resource limits are out of the model** (ADR 0086). A write can fail on what the environment sets: a full disk (`ENOSPC`), a quota, or a file size limit (`EFBIG`). Such a failure can depend on the length of the content, so it is outside the model, as races and hard links are.
  7. **Other processes are out of the model** (ADR 0086). Another process, a server on the same machine included, can change the granted directory between two operations of the run. The model treats the file system and the servers as separate, as races are outside it.
- `Keys` loads keys from the file given in the grant (`--grant keys:./keys.toml`, a table of `name = "<base64>"`). Key material never enters Una values.
- **Labels of `Keys.hmac`** (ADR 0039). The result of `@k.hmac(key_name: n, data: d)` is labeled `internal ⊔ ℓ*(d)`, where `ℓ*(d)` joins every label of the type of `d`; lowering it below that label needs a policy (`spec/07 §5`). `key_name` accepts only `public`, so a labeled key name, or a call under a raised program counter, is U4003, and a failure of `hmac` depends only on public data. `data` accepts every label. A user handler of `Keys` is checked against the worst caller (`spec/07 §7` rule 4): its `key_name` parameter is `public`, its `data` parameter carries every label, and its result may carry the label of `data`.
- **Labels of failures** (v0.2). Whether a host operation fails, and how, is labeled by the arguments that decide it (`spec/07 §7` rule 4): the path of `Files.read` and `Files.write`, not the content; the key name of `Keys.hmac`, not the data; for `Http`, `spec/24 §5` rule 3. `Files.write` accepts an `internal` path, so its failure is then `internal`: `attempt` gives an `internal` `Result`, and `if error pass` is U4002. The other paths and key names are `public`, and so are their failures, except that every `Files` failure also carries the package's file-state label: the labels that decide where the package's writes land and whether they run (S-14, ADR 0085, `spec/07 §7` rule 4). `Files` has two operations, `read` and `write`, and both carry it.
- **Host text** (ADR 0086, S-16). `Files.read` and `Env.get` make a `Text` from bytes the program does not control. When those bytes are not valid UTF-8, the run aborts with U6005 at the operation: `Files.read` does not fail with `Io`, and `Env.get` does not return `None`. Their content is `internal`, so it cannot decide a failure or an `Option` tag that a `public` branch sees; an abort is outside the noninterference model (`spec/07 §1`). Valid UTF-8 is read unchanged. In a task, the abort stops the task and its block reports it, as U6004 does.
- User code MAY write handlers for host capabilities (for example, a fake `Keys` in examples). Host implementations are used only by `una run`.

(v0.2, ADR 0074.) In edition `0.2`, `std.host` also declares the capability `Http` and the kinds `Header`, `Response` and `HttpError` (`spec/24`).

## 4. `std.testing`

Usable only in test code (ADR 0028, ADR 0043):

1. A function of `std.testing` is called only in example lines and in the operation bodies of handlers that are installed only by `given` lines of examples (tests, written `test` in edition `0.3`, `spec/08 §2`, ADR 0105; the messages name them by the edition's word). A call outside a handler operation body or an example is U3015.
2. A handler whose operation bodies use `std.testing` and that is installed by a `with` binding is U3015 at the `with` binding, with the use of `std.testing` as a related span.
3. A function of `std.testing` used as a value, not called, is U3015 everywhere, in handler bodies and examples included.

| Signature | Purpose |
| --- | --- |
| `fake_hash(seed: Int) -> internal Hashed` | Deterministic `Hashed` value for tests; equal seeds give equal values |
| `elapse(d: Int of Millisecond) -> Unit`, `requires d >= 0` | (Edition 0.4, ADR 0116 rule 19.) Moves the test's virtual clock forward by `d` at once, without waiting (`spec/26 §8`) |

## 5. Costs

(ADR 0132, in every edition.) Each item of the standard library has a stated cost: an upper bound on the time of one call and on the extra space it allocates, its result included, in the sizes of its arguments. A bound holds in the interpreter and in native code alike (`spec/16 §5`); constant factors are not part of it. A bound states what each engine MUST achieve: an engine that misses one has a defect, never a different specification.

**The model.**

1. **Steps.** One step is a constant amount of work: an instruction, a reference-count update, an allocation of a bounded size, a check of the memory budget (`spec/11 §7`), which adds O(1) per allocation and changes no bound.
2. **Sizes.** n is the length of a list `xs` or the number of entries of a map `m`; b is the length of a text `t` in UTF-8 bytes, and m the length in bytes of the other text argument, as in `spec/22 §2`; d is the number of 64-bit limbs of an `Int`, 1 for an `Int` that fits a machine word. s(x), the size of a value x, is 1 for a value of a fixed size (`Bool`, `Unit`, a fixed-width integer, `F64`, `F32`, a variant without fields), d for an `Int`, b for a `Text`, and one plus the sizes of its parts for a value with parts; size(xs) is n plus the sizes of the elements of `xs`.
3. **Passing values.** An argument or a result is passed by reference, in O(1), except an `Int`, which an engine MAY copy to read or return, in O(d). Every bound includes that cost.
4. **Comparisons.** `==` and the order of `spec/03 §1` on two values x and y take O(min(s(x), s(y))) steps. A bound counted in comparisons multiplies by that cost.
5. **Function parameters.** For `map`, `filter`, `first_where`, `fold` and a map's `update`, a bound counts the calls of the function parameter; each call costs what its body costs.
6. **Held once and given up.** An update of a value that no other binding holds, or that the caller gives up at the call, its last use, happens in place (`spec/16 §4` rule 7, `spec/17 §7` rule 3, ADR 0072); otherwise the value is copied first, in O(its length). The tables give both bounds.
7. **Amortized and expected.** A bound is for the worst case of one call unless it says otherwise. An **amortized** bound holds for the total of any sequence of k calls on one value, divided by k. An **expected** bound holds on average over the hashing key of a table (`spec/28 §1`), for every input, and applies to `HashMap` and `HashSet` only.
8. **Host calls.** An operation of a host capability is counted in host calls (a request to the operating system or the network) and in the time and space it spends on the program's side of them. The time the host takes for a call is outside the model. Each operation also records one trace event (`spec/11 §5`), in O(the size of its arguments and outcome). An operation that a user handler serves costs what the handler's body costs.

**The prelude** (§1, `spec/17 §8`, `spec/20 §6`, `spec/21 §2`, `spec/23 §2`):

| Call | Cost | Space |
| --- | --- | --- |
| `abs(x)` | O(d) | O(d) |
| `min(a, b)`, `max(a, b)` | one comparison | O(1) |
| `is_digits(t)` | O(b) | O(1) |
| `starts_with(t, prefix)` | O(m), m the bytes of `prefix` | O(1) |
| `trim(t)` | O(b) | O(b) |
| `to_int(t)` | O(b), the conversion being bounded by its 4300 digits (§1) | O(b) |
| `is_empty(xs)`, `first(xs)`, `at(xs, index)` | O(1) | O(1) |
| `append(xs, item)` | amortized O(1) when `xs` is held once or given up at the call, through any number of call levels (ADR 0072, ADR 0107); O(n) otherwise | O(1) amortized; O(n) for the copy |
| `set(xs, index, item)` | O(1) when `xs` is held once or given up at the call (ADR 0107); O(n) otherwise | O(1); O(n) for the copy |
| `contains(xs, item)` | O(n + Σ min(s(x), s(item))) over the elements x it compares, at most O(size(xs)); it stops at the first equal element | O(1) beyond one comparison |
| `map(xs, each)` | O(n) and n calls of `each` | O(n) |
| `filter(xs, keep)` | O(n) and n calls of `keep` | O(n) |
| `first_where(xs, keep)` | O(k) and k calls of `keep`, k the position of the element it finds plus one, or n | O(1) |
| `fold(xs, initial, step)` | O(n) and n calls of `step` | O(1) beyond the accumulators |
| `sum(xs)` | O(size(xs)) | O(d) of the result |
| `sort(xs)` | O(n log n) comparisons, and O(n) comparisons when `xs` is already in order | O(n) |
| `is_some(o)`, `is_ok(r)`, `or_else(o, default)` | O(1) | O(1) |
| `empty_map()` | O(1) | O(1) |
| `quote_json(t)` | O(b) | O(b), at most 6b + 2 bytes |
| `is_json_number(t)` | O(b) | O(1) |

The text functions of `spec/22 §2` cost what its table states (ADR 0070), `parse_json` and `to_json` what `spec/23 §6` states (ADR 0073), and the numeric functions what `spec/29` states.

**`std.testing`** (§4): `fake_hash(seed)` takes O(d) time for a seed of d limbs, and its result is 32 bytes; `elapse(d)` takes O(1), and a deadline it passes is found by what waits on it, not by the call.

**Built-in methods** (§2, `spec/18`, `spec/21 §2`):

| Call | Cost | Space |
| --- | --- | --- |
| `t.len`, `xs.len`, `m.len` | O(1) | O(1) |
| `m.has(key: k)`, `m.get(key: k)` | O(log n) comparisons (ADR 0069) | O(1) |
| `m.set(key: k, value: v)`, `m.remove(key: k)` | O(log n) comparisons when `m` is held once (ADR 0069); O(n) for a copy first otherwise | O(log n); O(n) for the copy |
| `m.with(key: k, value: v)`, `m.without(key: k)` | O(log n) comparisons when `m` is given up at the call; O(n) for a copy first otherwise | O(log n); O(n) for the copy |
| `m.update(key: k, initial: i, change: f)` | as `set`, and one call of `change` | as `set` |
| `m.keys()`, `m.values()`, `m.entries()` | O(n) | O(n) |
| `x.to_i8()` to `x.to_u64()`, `x.to_int()` | O(1) | O(1) |
| `a.wrapping_add(by: b)` and the other wrapping, saturating and bit operations of `spec/18 §5` | O(1) | O(1) |

A narrowing conversion whose `overflow` obligation holds reads a value of at most 64 bits, so rule 3 adds O(1). The methods of the collections cost what `spec/28` states (ADR 0119), and those of the numeric types what `spec/29` states (ADR 0120).

**Host capabilities** (§3, `spec/24`), with g the grants of the capability that allow the operation and c the components of a path after rule 1 of §3:

| Operation | Host calls | Cost and space on the program's side |
| --- | --- | --- |
| `Console.print(line)` | one write of b + 1 bytes to standard output | O(b) |
| `Clock.now()` | one read of the clock, none under virtual time (`spec/26 §8`) | O(1) |
| `Env.get(name)` | one lookup of the variable | O(m + r), m the bytes of `name` and r those of its value |
| `Files.read(path)` | O(g · c) lookups of directory entries, then one read of the file | O(p · g + r), p the bytes of `path` and r those of the file, its UTF-8 check included |
| `Files.write(path, content)` | O(g · c) lookups, then one write of the content | O(p · g + b), b the bytes of `content` |
| `Keys.hmac(key_name, data)` | none: the keys are read once, before `main` | O(log K) comparisons of key names, K the keys of the grant, and O(b) for HMAC-SHA-256 over `data`; the result is 32 bytes |
| `Http.get`, `Http.post`, `Http.put`, `Http.delete` | one request per response it reads, at most 6 with its redirects (`spec/24 §2` rule 3) | linear in the URL, headers and body it sends and in each response it reads (`spec/24 §4` rule 3), which `spec/24 §4` rule 1 bounds; O(G) to find its grants among G (`spec/27 §9` rule 7) |
