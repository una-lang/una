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

**Result labels** (ADR 0038 D3, ADR 0051). The parameters of these functions that are not functions accept arguments of any label, and their results carry those labels as follows (`spec/07 §7` rule 8):

- **Structural functions** label each position of the result with what it holds. The element and accumulator positions carry the labels of their type variables, which every argument they come from joins (`spec/07 §7` rule 1). The top label is the top label of what decides the result's shape, so the labels of element values the function does not read never reach it:
  - `append`, `first` and `is_empty`: the top label of `xs`;
  - `at`: the top label of `xs` and every label of `index`;
  - `is_some`: the top label of `o`; `is_ok`: the top label of `r`;
  - `map`: the top labels of `xs` and `each`; `fold`: the label of the accumulator type, joined with the top labels of `xs` and `step`;
  - `filter` and `first_where`: the top labels of `xs` and `keep`, and the label of `keep`'s declared result. A predicate's result flows into that declared `Bool`, so a predicate over labeled element values is rejected there (U4001).
  `.len` takes the top label of its operand (`spec/07 §7` rule 3).
- **The other functions** read element or argument values, and their result is labeled with the join of every label of their arguments, element types included: `contains`, `sum`, `or_else`, `min`, `max`, `abs` and the text functions `is_digits`, `starts_with`, `trim` and `to_int`.

The native implementations declare these labels explicitly.

Text works on Unicode scalar values: `.len` counts them; `is_digits` holds when every character is an ASCII digit, so it holds of the empty text; `trim` removes Unicode whitespace at both ends; `to_int` reads an optional `-` followed by ASCII digits; `starts_with` compares scalar values; `contains` and `==` are structural; `Text` orders by scalar values (ADR 0032).

## 2. Built-in operations

`.len` on `Text` and `List`, arithmetic, comparisons, equality and text interpolation are built in (`spec/09 §2`). `Hashed` supports only `==` and `!=`; it cannot be interpolated or converted to `Text` (U2001).

## 3. Host capabilities (part of the prelude)

The host capabilities are declared in the module `std.host` and are in scope in every module through the prelude (`spec/10 §11` rule 8).

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
  4. **Races are out of the model.** A race between the check and the use of a path is outside the v0.1 model, since the interpreter is single-threaded and host capabilities are trusted; the build report lists it (`spec/13 §6`).
  5. **Hard links are out of the model.** A hard link inside the grant to a file outside it is an ordinary directory entry, so `Files` reads and writes it. Hard links are outside the v0.1 model, as races are: `Files` has no operation that creates a link, so only someone with other access to the granted directory can plant one, and the build report lists them (`spec/13 §6`, ADR 0048).
- `Keys` loads keys from the file given in the grant (`--grant keys:./keys.toml`, a table of `name = "<base64>"`). Key material never enters Una values.
- **Labels of `Keys.hmac`** (ADR 0039). The result of `@k.hmac(key_name: n, data: d)` is labeled `internal ⊔ ℓ*(d)`, where `ℓ*(d)` joins every label of the type of `d`; lowering it below that label needs a policy (`spec/07 §5`). `key_name` accepts only `public`, so a labeled key name, or a call under a raised program counter, is U4003, and a failure of `hmac` depends only on public data. `data` accepts every label. A user handler of `Keys` is checked against the worst caller (`spec/07 §7` rule 4): its `key_name` parameter is `public`, its `data` parameter carries every label, and its result may carry the label of `data`.
- **Labels of failures** (v0.1.4, N-39). Whether a host operation fails, and how, is labeled by the arguments that decide it (`spec/07 §7` rule 4): the path of `Files.read` and `Files.write`, not the content; the key name of `Keys.hmac`, not the data. `Files.write` accepts an `internal` path, so its failure is then `internal`: `attempt` gives an `internal` `Result`, and `if error pass` is U4002. The other paths and key names are `public`, and so are their failures.
- User code MAY write handlers for host capabilities (for example, a fake `Keys` in examples). Host implementations are used only by `una run`.

## 4. `std.testing`

Usable only in test code (ADR 0028, ADR 0043):

1. A function of `std.testing` is called only in example lines and in the operation bodies of handlers that are installed only by `given` lines of examples. A call outside a handler operation body or an example is U3015.
2. A handler whose operation bodies use `std.testing` and that is installed by a `with` binding is U3015 at the `with` binding, with the use of `std.testing` as a related span.
3. A function of `std.testing` used as a value, not called, is U3015 everywhere, in handler bodies and examples included.

| Signature | Purpose |
| --- | --- |
| `fake_hash(seed: Int) -> internal Hashed` | Deterministic `Hashed` value for tests; equal seeds give equal values |
