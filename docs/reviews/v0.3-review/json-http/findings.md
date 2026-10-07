# Cross-review of JSON and HTTP for v0.3.0: findings

Candidate `5495b61d65508fe1d26785f335d861799a58757f`. Severity and areas as in `plan.md`.

## H-1, Critical: whether an `Http` request fails reveals its `internal` arguments as `public`

**Area 6, failure labels; area 3, labels.** Case: `cases/6/http-failure-of-internal-url`.

```una
pub main() -> Unit
  uses @http: Http[get]
  uses @env: Env[get]
  uses @console: Console[print]
=
  let url = @env.get("SECRET_URL")
    if none use "http://127.0.0.1:8765/"
  let headers: List[Header] = []
  let r = attempt @http.get(url, headers: headers, key_name: None)
  when r
    is Ok -> @console.print("granted")
    is Err -> @console.print("not granted")
```

- `una check` accepts it with no diagnostic. `url` is `internal` (`Env.get`), and `Console.print` accepts only `public`.
- With a grant `http:http://127.0.0.1:8765:get` and a server there, `SECRET_URL=http://127.0.0.1:8765/` prints `granted` and `SECRET_URL=http://10.0.0.1:9/` prints `not granted`, in the interpreter and as a native executable. One bit of an `internal` value per request reaches a `public` sink, and a loop over guesses reads more.
- **Why it is a leak.** Every argument of an `Http` operation accepts `internal` (`spec/24 §5` rule 1), and its failure depends on them: `Denied` on the URL's origin and method and on whether `key_name` names a key, `BadUrl` on the URL, `BadRequest` on header names and values and on the key. But the failure is not labeled, so `attempt` gives a `public` `Result`, and `if error pass` and `if error fail` are allowed.
- **What the model says elsewhere.**
  - `spec/14 §3` makes `Keys.hmac`'s `key_name` accept only `public` for exactly this reason: "a failure of `hmac` depends only on public data".
  - Every v0.1 host operation that can fail takes the arguments its failure depends on at `public`: `Files` paths, `Keys` key names.
  - A handler of `Http` written in Una cannot do this: its body starts at `pc = ⊥` for failures and `fail` under a branch on an `internal` parameter is U4002 (`spec/07 §7`). Only the host implementation can.
  - N-27 labels the failure of a prelude function with every label of every argument.
- **Suggested fix.** Label the failure of an `Http` operation with the join of the labels of its arguments, as N-27 does for prelude functions: `attempt` gives a `Result` labeled that way, `if error use` runs under the raised program counter, and `if error pass` and `if error fail` are U4002 when the join is above `public`. A program whose URL, headers and key name are `public`, the common case, keeps `if error pass`. The alternative, making URL, headers and key name accept only `public` as `Keys` does, is simpler, but it forbids `internal` header values, which `Header.value` exists to carry.
- **Scope.** The same holds for `post`, `put` and `delete`, whose `body` does not affect the failure but whose URL, headers and key name do. It is a question for the spec owner whether the rule belongs in `spec/07 §7` for every operation that accepts more than `public`, since today only `Http` does.

**Fixed: #310, PR #317 (decisions N-39 and N-40), rechecked at `651062b` independently: Clean.**
- `cases/6/http-failure-of-internal-url` is now U4003 at both prints; the `if error pass` variants of area 3 are U4002 for an `internal` URL, and accepted for an `internal` header value, which N-40 takes out of the decision.
- `cases/recheck-h1/matrix.py`, 48 cases, 0 unexpected:
  - the URL decides, in `get`, `post`, `put` and `delete`, under `if error pass` and `if error fail` (U4002), with `if error use` accepted and a `public` URL as control;
  - the key name, and the header list's top label, decide; a header value and a body do not;
  - `attempt` on an `internal` URL makes the branch and the error payload U4003 at a `public` sink, while a `public` URL, an `internal` header value or an `internal` body keep it `public`;
  - the same in a loop over `internal` URLs, in a task of `together`, through a generic helper, and through a relay with `uses of` and `fails of`, each with a `public` control;
  - `Files.write` with an `internal` path is U4002 in editions 0.1 and 0.2, with `internal` content accepted; `Files.read` with an `internal` path and `Keys.hmac` with an `internal` key name stay U4003, and `Keys.hmac` with `secret` data stays accepted.
- `cases/recheck-h1/u6004`: a header value with U+0001, from the environment, in a task of `together`, aborts with U6004 at the operation, exit 1, in both engines, with identical traces (the task's `get` is `aborted`), and the server receives no request. With a tab instead, both engines send the requests and finish, with identical traces.
- Notes, not findings:
  - A `secret` URL or key name now gives U4002 next to its U4003, since the failure takes the argument's label. Both are true; the second is redundant.
  - The body stays out of the decision by design (N-39), but a server that receives an `internal` body can still choose whether the request fails. That is the behavior of an external system, which `spec/07 §1` leaves outside the model, as it does the response's content.
  - `Files.write` with an `internal` path changes in edition 0.1 too. Gate 1 pins v0.1.3, so its references are not affected.

## H-2, Minor: one large `parse_json` aborts the process instead of ending with U6003

**Area 5.** Case: `cases/5/scale` with a 26 MB array of short strings and `--max-memory 64`.

- Both engines print `memory allocation of … bytes failed` and exit with status 134 (an abort), with no diagnostic. With the default budget the same input parses.
- This is what `spec/11 §7` allows: the budget is checked at calls and loop iterations, and "an allocation beyond twice the budget between two checks aborts the process". `parse_json` and `to_json` are single calls that allocate in proportion to their input, with no check inside, so the guarantee of rule 141 ("a run over its budget ends with U6003") never applies to them.
- It matters because JSON is where untrusted size arrives: a response body of up to `--max-response` (16 MiB by default) expands several times when parsed. With the default budget of 4096 MiB that cannot reach the ceiling, but with a smaller `--max-memory` an input of a few MiB can.
- **Suggested fix.** Check the budget inside the parser and the writer, every so many values (say every 4096 nodes), as `for` loops do per iteration, and end with U6003 there. `spec/23 §3` rule 6 ("no limit beyond the memory budget") then holds in the sense rule 141 promises.

**Fixed: #314, PR #318 (decision N-41).** Una-1 wrote the fix, so Una-2, the author of JSON, reviewed it: approved at `1f461b5`. The case now ends with U6003 at the `parse_json` call in both engines; area 2's 10,119 inputs give 0 differences and area 5's doubling is unchanged.

## Clean

- **Area 2.** `parse_json` and `to_json` against an independent reference of `spec/23 §3` and `§4` (`cases/2/reference.py`), on 10,119 inputs (113 handcrafted, the rest generated and mutated): 0 differences in value, failure kind or position, every round trip a fixed point, and both engines identical (`cases/2/compare.py`). JSON built from `Map` and `Text` over 66 texts with every escape class: 0 differences, and `quote_json(t) == to_json(Str(value: t))` on all of them (`cases/2/gen_build.py`).
- **Area 3.** 23 label cases (`cases/3/matrix.py`): constructors (U4001 for a labeled `Str`, key or map), a parsed response body, a parse failure and its position, `quote_json`, interpolation, `m.has`, `find`, a status, and a `secret` URL, body or key name, all as specified. The two exceptions are H-1.
- **Area 1.** Against a two-origin logging server (`cases/1/server.py`, `cases/1/redirects`):
  - `key_name`'s bearer token, and the program's `Authorization`, `Cookie` and `Proxy-Authorization` (in lower case too), are sent to the request's origin, dropped once a redirect leaves it, and stay dropped when the chain comes back to it. Other headers follow.
  - `chunked` bodies, `Protocol` for a body that is not UTF-8, `TooLarge`, `Denied` for an origin and for an unknown key, `BadUrl`, and `BadRequest` for `key_name` with an `Authorization` header, all as specified.
  - The requests the server received and the program's results are identical in both engines.
- **Area 5.** By doubling on a quiet machine, best of three, both engines (`cases/5/scale.py`): flat arrays, objects of 50,000 to 400,000 keys in random order, 200 to 1,600 arrays nested 500 deep, and strings full of escapes, from 1 to 21 MB. Every step is ×1.85 to ×2.11, except the wide object at ×2.05 to ×2.34, which is its O(k log k) bound.
- **Area 1, time.** `--http-timeout 2` against a server that answers in 5 s fails with `TimedOut` after 2 to 3 s; `--http-timeout 8` gets the `200`, in both engines (`cases/1/timeout`).
- **Area 4.** The `--json run` trace and the executable's `--trace` of `cases/1/redirects` are identical: eleven `host:Http` events, header values redacted as `internal`, outcomes redacted by the binder's label, and the key in none of them.
- **Area 6, JSON.** The failure of `parse_json` on an `internal` text carries its label: `attempt` gives a labeled `Result`, a branch on it or on `at` is U4003 at a `public` sink, and `if error pass` is U4002 (area 3 cases). The open part of area 6 is H-1.
- **Area 7.** 13 cases (`cases/7/names.py`): in edition 0.1, `Json`, `Http`, `Response`, `parse_json` and `to_json` are ordinary names, and the prelude's are unreachable (U1001); in edition 0.2, a user `Json`, `JsonError`, `JsonNumber`, `Http`, `Header`, `Response` or `HttpError` is U1002, and a user variant named `Null` or `Str` shadows `Json`'s. `import std.host (Http)` is U1001 on the candidate, which predates #307; #307 is reviewed under #309.
- **Area 8.** The guide's JSON and HTTP sections are checked by the test suite and agree with `spec/23` and `spec/24`; the "not implemented yet" markers of `spec/13` and `spec/14` are gone; the CHANGELOG lines match.
