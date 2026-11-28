# Una, compact

Editions: 0.1, 0.2 stable; 0.3–0.5 preview. This reference teaches 0.2; see una-for-agents.md for the others.

**Loop.** `una fmt .`, `una fix .`, `una --json check .`; read `code` and `repairs`, `una explain <code>`; then `una --json test .`.

**Package.** `una.toml` (`[package]` `name`, `version`, `edition = "0.2"`; `[run]` `entry`, `grants`; `[trusted] modules`), `verification.lock` (pins Z3), `src/<module>.una`.

**Syntax.**
- Two-space indentation; each call on one line.
- There is no `return`, `match`, `var`, `while` or `type`. Use the last expression, `when`, `let edit`, `for` and `kind`.
- `result` is reserved. A name is bound once per scope, with no shadowing.
- Arguments after the first are labeled, as in `f(a, b: 2)`. Constructors with two or more fields label them all.
- Text escapes are `\{ \} \" \\ \n \u{D}`, and `{e}` interpolates.

**Functions.** Clauses in order `requires`, `ensures`, `uses`, `fails`, then `=` and the body below. No clauses: `f(x: Int) -> Int = x * 2`. `kind P = A(x: Int) | B` is a sum; indented `name: Type` lines make a record. `when v` has arms `is A(x) -> ...` and `else -> ...`.

**Failures.** Declare `fails E` (one sum kind); fail with `fail V(...)`. Handle a failing call on the lines below it with `if error pass`, `if error fail V`, `if error use v`, `if none fail V` or `if none use v`; `attempt f(x)` gives a `Result`. An effect or fallible call is a whole statement or `let` (U3013).

**Capabilities.** There is no ambient authority.
- Declare `uses @log: Log[write]` in the header, then call `@log.write(x)`.
- `with @x = @y` renames a capability; `with @x = h()` installs a handler.
- A `pub` function declares its callees' capabilities.
- Only `main` gets host capabilities: `Console[print]`, `Env[get]`, `Clock[now]`, `Files[read, write]`, `Keys[hmac]`, `Http[get, post, put, delete]`, granted in `[run] grants` (`console`, `env`, `http:https://host`).
- `pub handler h(...) for Cap` implements every operation, as `op name(params) = body`.

**Labels.** `public < internal < confidential < secret < restricted`.
- Data only rises. `Env.get`, `Files.read` and Http responses are `internal`, so printing them is U4003.
- To lower a label, write a `policy` in a trusted module.
- A label covers one position: `internal Option[internal Int]`.

`una.toml`
```toml
[package]
name = "g"
version = "0.1.0"
edition = "0.2"

[trusted]
modules = ["release"]
```

`src/release.una`
```una
trusted module release

pub policy shown
  from internal Text
  to public Text
  via copy
  purpose "show a name"
  owner ops

copy(t: internal Text) -> public Text = t
```

**Contracts.**
- `requires` and `ensures` use comparisons, `+ -`, `*` by a literal, `.len`, `m.has(key: k)`, `is`, `and or not implies`, `old(p)`. The solver knows a call only by its contract.
- A loop forgets every fact except the refinements of what it updates, so put the contract on a helper with an `edit` parameter.
- Never weaken a contract or add `checked` just to pass.

**Loops.**
- `let edit x = e`, then `x = e2`; `for v in xs`, `for i in 0..<n`.
- An `edit p` parameter is in-out; the caller writes `edit x`.
- `edit xs.set(index: i, item: v)` replaces an element. `xs.append(item: v)` returns a new list.

```una
module tally

add(edit counts: Map[Text, Int], key: Text) -> Unit
  ensures counts.has(key: key)
=
  let n = counts.get(key: key)
    if none use 0
  edit counts.set(key: key, value: n + 1)

pub tally(keys: List[Text]) -> Map[Text, Int]
=
  let edit counts: Map[Text, Int] = empty_map()
  for key in keys
    add(edit counts, key)
  counts
```

**Library.**
- **Maps:** `get`, `set`, `remove`, `has`, `len`, `keys()` and `entries()`. A type argument is never refined, so `Map[Text, Nat]` is U2002.
- **JSON:** `t.parse_json()` (fails `JsonError`), `j.to_json()`, `t.quote_json()`; `Null`, `Boolean`, `Number(value)`, `Str`, `Array(items)`, `Object(fields)`.
- **Text:** `split(on:)`, `join(separator:)`, `find(text:)`, `replace(pattern:, by:)`, `slice(start:, end:)`, `trim()` and `parse_int()`.
- **Http:** `get(url, headers: [Header(name:, value:)], key_name: None)` gives a `Response` (`status`, `body`), failing with `HttpError`.
  A `secret` header value needs a policy to `internal`; no control characters in values.
- **Together:** `together` over `let` branches, or `together for x in xs`; the first failure in list order wins.

**Examples.** `given @x = handler(...)` lines (no host capabilities), one `do expr`, `expect` lines over `result`, `fails V` or `performed @x.op`.
