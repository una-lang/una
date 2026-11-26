# 14. Standard library

Status: Normative, v0.1, with the additions of later editions marked by edition. The standard library is part of the Trusted zone. Everything not listed here does not exist in v0.1. This chapter covers the prelude, `std.host` and `std.testing`; the other standard modules are specified with their constructs: `std.time` and `std.drive` in `spec/26 §2` (edition `0.4`), `std.limit` in `spec/27 §10` (edition `0.4`), and `std.collections` in `spec/28 §2` (edition `0.5`, a preview).

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged: a call with effects is bound with `const` before it is given. A Core `let` (`spec/09`) stays. The chapter's Una blocks keep the words of edition `0.4` until issue #1630 moves those of edition `0.5`.

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
- (v0.4, ADR 0073 J5 as amended on 2026-11-09, accepted on 2026-11-10.) **The variant constructors of `Json`**, `Boolean`, `Number`, `Decimal`, `Str`, `Array` and `Object`, label their result's top with the join of every label of their arguments, element types included; the positions of `Json` stay `public` (`spec/23 §5` rule 2).

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
- `to_int(t)` is `Some(n)` when `t` is an optional `-` followed by one or more ASCII digits, leading zeros allowed, where `n` is the integer they write (`"-0"` is `0`). It is `None` for any other text, among them the empty text, `"-"`, a leading `+`, whitespace and digits outside ASCII. (Security fix, 2026-10-18, #914, in every edition.) A text of more than 4300 digits, the sign excluded and leading zeros included, is `None`, the bound of `parse_int` (`spec/22 §4`) and of a JSON integer (`spec/23 §3`): converting d digits takes more than linear time, O(d²) up to the bound and O(M(d) log d) beyond, so a longer text is refused before any conversion, and the work a text asks for stays linear in its length (the reason restated 2026-11-26, #1223). Writing has no such bound (`spec/11 §7`).

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
  2. **Resolve intermediate components.** Every intermediate directory is resolved with its symbolic links followed, and the resolved directory MUST lie inside the canonical grant prefix (`Denied` otherwise). When the resolution finds that an intermediate directory, the grant prefix included, does not exist, the operation fails with `NotFound` (with `Io` where a component of it is not a directory): the confinement decides this answer, and the operation MUST NOT give the operating system a path, so the answer does not depend on the host's limits on the links or the length of a path. The answer is decided when the directory is found missing: a directory created after that does not turn it into a success (rule 4). An intermediate directory, or a grant prefix, that does not resolve for any other reason fails the operation with `Denied`, and the operation gives the operating system no path either: a dangling symbolic link, a host limit on the links or the length of a path reached while resolving it, or a directory that may not be searched. The dangling symbolic link meant here is the intermediate directory or the grant prefix itself: a grant prefix whose path passes through a dangling symbolic link above it does not exist, and the operation fails with `NotFound`. Only when every intermediate directory resolves inside the canonical grant prefix is the operation given a path, the canonical form of the last directory joined with the final component (rule 3); a final component that does not exist there is left to the operation, so a read fails with `NotFound` and a write creates the file.
  3. **Never follow the final component.** The final component is opened without following a link: if it is a symbolic link, dangling or not, the operation fails with `Denied`. A write never creates a file through a link.
  4. **Races are out of the model.** A race between the check and the use of a path is outside the v0.1 model, since the interpreter is single-threaded and host capabilities are trusted; the build report lists it (`spec/13 §6`). (v0.2, ADR 0065.) Concurrent tasks may also operate on one path in any order (`spec/19 §3`), and the native runtime's host capabilities are safe to call from several tasks at once (`spec/19 §8`).
  5. **Hard links are out of the model.** A hard link inside the grant to a file outside it is an ordinary directory entry, so `Files` reads and writes it. Hard links are outside the v0.1 model, as races are: `Files` has no operation that creates a link, so only someone with other access to the granted directory can plant one, and the build report lists them (`spec/13 §6`, ADR 0048).
  6. **Resource limits are out of the model** (ADR 0086). A write can fail on what the environment sets: a full disk (`ENOSPC`), a quota, or a file size limit (`EFBIG`). Such a failure can depend on the length of the content, so it is outside the model, as races and hard links are.
  7. **Other processes are out of the model** (ADR 0086). Another process, a server on the same machine included, can change the granted directory between two operations of the run. The model treats the file system and the servers as separate, as races are outside it.
- `Keys` loads keys from the file given in the grant (`--grant keys:./keys.toml`, a table of `name = "<base64>"`). Key material never enters Una values.
- **Labels of `Keys.hmac`** (ADR 0039). The result of `@k.hmac(key_name: n, data: d)` is labeled `internal ⊔ ℓ*(d)`, where `ℓ*(d)` joins every label of the type of `d`; lowering it below that label needs a policy (`spec/07 §5`). `key_name` accepts only `public`, so a labeled key name, or a call under a raised program counter, is U4003, and a failure of `hmac` depends only on public data. `data` accepts every label. A user handler of `Keys` is checked against the worst caller (`spec/07 §7` rule 4): its `key_name` parameter is `public`, its `data` parameter carries every label, and its result may carry the label of `data`.
- **Labels of `Clock` readings** (ADR 0116 amendment of 2026-11-09, accepted, #898). In edition `0.4`, the result of `Clock.now` at a site is labeled with the timing label of the code that ran before it: `τ⁰` of the body of every ancestor of the site in the call graph, joined with `τ_conc` at it, `L_res`, and in a driven package `τ⁰` of `init`, `step` and `on_stop`, by the ancestors' rule of `spec/27 §9` rule 5. The reading then flows by the rules of `spec/07`: a `public` sink refuses one above `public` (U4003), and no release of timing is added. Its event in a run's trace is redacted at that label, and the report's `information.timing` list names each reading above `public`. The checker computes the label in the probe of `spec/27 §5` rule 8 and confirms it after checking (U8001 otherwise). In a package with a `serve` section, whose `L_srv` the preview makes `⊤` (`spec/31 §7` rule 6), every reading is `⊤`. A reading in a handler's operation is `⊤`, so a handler of `Clock` cannot forward the host's clock (U4001 at its answer). A reading that a policy's `via` function reaches, in its body or in what it calls, is U4001 when its label is above `public` (`spec/07 §5` rule 7), since no label is checked there. Editions `0.1` to `0.3` keep the reading unlabeled, with timing out of model (`spec/07 §1`).
- **Labels of failures** (v0.2). Whether a host operation fails, and how, is labeled by the arguments that decide it (`spec/07 §7` rule 4): the path of `Files.read` and `Files.write`, not the content; the key name of `Keys.hmac`, not the data; for `Http`, `spec/24 §5` rule 3. `Files.write` accepts an `internal` path, so its failure is then `internal`: `attempt` gives an `internal` `Result`, and `if error pass` is U4002. The other paths and key names are `public`, and so are their failures, except that every `Files` failure also carries the package's file-state label: the labels that decide where the package's writes land and whether they run (S-14, ADR 0085, `spec/07 §7` rule 4). `Files` has two operations, `read` and `write`, and both carry it.
- **Host text** (ADR 0086, S-16). `Files.read` and `Env.get` make a `Text` from bytes the program does not control. When those bytes are not valid UTF-8, the run aborts with U6005 at the operation: `Files.read` does not fail with `Io`, and `Env.get` does not return `None`. Their content is `internal`, so it cannot decide a failure or an `Option` tag that a `public` branch sees; an abort is outside the noninterference model (`spec/07 §1`). Valid UTF-8 is read unchanged. In a task, the abort stops the task and its block reports it, as U6004 does.
- User code MAY write handlers for host capabilities (for example, a fake `Keys` in examples). Host implementations are used only by `una run`.

(v0.2, ADR 0074.) In edition `0.2`, `std.host` also declares the capability `Http` and the kinds `Header`, `Response` and `HttpError` (`spec/24`).

(v0.4, ADR 0123.) In edition `0.4`, `std.host` also declares the capability `HttpServer`, the kinds `Method`, `Param`, `Request`, `Exchange`, `Status`, `Reply` and `ServeError`, and the opaque type `ReplyRight` (`spec/31 §1`). Unlike the other host capabilities, the prelude does not put these names in scope: a module imports them from `std.host`, and an import of one is never redundant (`spec/10 §11` rule 10).

## 4. `std.testing`

Usable only in test code (ADR 0028, ADR 0043):

1. A function of `std.testing` is called only in example lines and in the operation bodies of handlers that are installed only by `given` lines of examples (tests, written `test` in edition `0.3`, `spec/08 §2`, ADR 0105; the messages name them by the edition's word). A call outside a handler operation body or an example is U3015.
2. A handler whose operation bodies use `std.testing` and that is installed by a `with` binding is U3015 at the `with` binding, with the use of `std.testing` as a related span.
3. A function of `std.testing` used as a value, not called, is U3015 everywhere, in handler bodies and examples included.

| Signature | Purpose |
| --- | --- |
| `fake_hash(seed: Int) -> internal Hashed` | Deterministic `Hashed` value for tests; equal seeds give equal values |
| `elapse(d: Int of Millisecond) -> Unit`, `requires d >= 0` | (Edition 0.4, ADR 0116 rule 19.) Moves the test's virtual clock forward by `d` at once, without waiting (`spec/26 §8`) |
| `test_request(method: Method, target: Text, body: Text) -> Request`, `fails TestRequestError` | (Edition 0.4, ADR 0123 rule 21.) The request the host would read for `method`, `target` and `body`, with the header `host: test`; `Refused` where the host would answer `400` (`spec/31 §8`) |
| `test_exchange(request: Request) -> Exchange` | (Edition 0.4, ADR 0123 rule 21.) An exchange of `request` with a fresh reply right (`spec/31 §8`) |

(Edition 0.4, ADR 0123 rule 21.) `std.testing` also declares `pub kind TestRequestError = | Refused`.
