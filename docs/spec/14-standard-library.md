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

## 2. Built-in operations

`.len` on `Text` and `List`, arithmetic, comparisons, equality and text interpolation are built in (`spec/09 §2`). `Hashed` supports only `==` and `!=`; it cannot be interpolated or converted to `Text` (U2001).

## 3. Host capabilities (part of the prelude)

| Capability | Operation | Fails | Accepts | Notes |
| --- | --- | --- | --- | --- |
| `Console` | `print(line: Text) -> Unit` | | `public` | Writes the line and a newline to stdout |
| `Clock` | `now() -> Int` | | `public` | Milliseconds since the Unix epoch |
| `Env` | `get(name: Text) -> Option[internal Text]` | | `public` | Reads a process environment variable |
| `Files` | `read(path: Text) -> internal Text` | `FileError` | `public` | Path relative to the grant prefix |
| `Files` | `write(path: Text, content: Text) -> Unit` | `FileError` | `internal` | Path relative to the grant prefix |
| `Keys` | `hmac(key_name: Text, data: Text) -> internal Hashed` | `KeyError` | any label (host only) | HMAC-SHA-256 with a named key |

```una
kind FileError = NotFound | Denied | Io
kind KeyError = | UnknownKey
```

- `Files` resolves paths against the prefix fixed by the grant and rejects any path that normalizes outside it with `Denied`.
- `Keys` loads keys from the file given in the grant (`--grant keys:./keys.toml`, a table of `name = "<base64>"`). Key material never enters Una values.
- User code MAY write handlers for host capabilities (for example, a fake `Keys` in examples). Host implementations are used only by `una run`.

## 4. `std.testing`

Usable only inside handler bodies and example lines (U3015).

| Signature | Purpose |
| --- | --- |
| `fake_hash(seed: Int) -> internal Hashed` | Deterministic `Hashed` value for tests; equal seeds give equal values |
