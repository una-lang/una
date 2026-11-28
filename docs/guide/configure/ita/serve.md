---
construct: "serve"
editions: ["0.4"]
spec: ["spec/31 §2", "spec/31 §6", "spec/31 §7", "spec/31 §1"]
codes: [U3033, U4003, U7008]
---

# `serve`

## What it does

The `serve` section of `ita.una` makes a package an HTTP server in the host-driven form (`spec/31 §7`). It names a module of the package. The host reads requests from the addresses that the package's `serve:` grants open, and it calls that module's `handle` once for each request it reads, each call in a task of its own. The host answers every request with the reply that `handle` returns. The package declares no `main`, and the module needs no `HttpServer`: the host owns every connection and every exchange, so `handle` is an ordinary function that the checker, the verifier and the tests see as any other.

`handle` has one exact shape:

```text
pub handle(request: internal Request) -> internal Reply
pub on_stop() -> Unit                 -- optional
```

`Request` and `Reply` come from `std.host`. `handle` may declare `uses` and `fails`, checked against the run's grants as `main`'s are. A failure of `handle` is answered `500` with an empty body. `on_stop`, when the module has one, runs once at every host stop, after the requests in flight are answered.

A server that takes its exchanges itself, with `next` and `reply` of `HttpServer`, is written in the language form instead (`spec/31 §1` rule 6): a `run entry` with a `serve:` grant and no `serve` section, as `examples/proxy` does.

## Forms

```text
serve
  module <module>                -- required: a module of the package
  within <milliseconds>          -- optional: 1 to 86,400,000; refused in this preview (U4003)
```

The section comes last in `ita.una`. It needs a `serve:` grant, in `run grants` or given with `--grant`, and stands in place of a `run entry` or a `drive` section, since a package runs one program. The grant opens the address and carries the server's limits, one per line below it (`spec/31 §2`):

```text
run
  grants
    serve "http://127.0.0.1:8080"
      at most 256 at once           -- exchanges delivered and not yet answered (/once)
      at most 600 per minute        -- exchanges delivered per period
      body at most 64 KiB
      idle within 5 seconds
      answer within 30 seconds
```

Every limit has a default, so a grant with no lines is a server with limits too. The table of `spec/31 §2` rule 2 lists each item, its default and its range.

## Examples

A health endpoint, as `examples/health` is:

`ita.una`

```una
ita health
  version 0.1.0
  edition 0.4

run
  grants
    serve "http://127.0.0.1:8080"

serve
  module health
```

`src/health.una`

```una
module health

import std.host (Method, Reply, Request)

--- Answers `GET /health` with 200, and every other request with 404.
pub handle(request: internal Request) -> internal Reply
=
  if request.method is Get and request.segments == ["health"]
    Reply(status: 200, headers: [], body: "ok")
  else
    Reply(status: 404, headers: [], body: "")
```

`una run .` serves it, and `una build .` makes an executable that serves the same way (see [Serving HTTP](../../serving-http.md)). `examples/orders` is a JSON API whose `handle` reaches its store over `Http`, through a capability that its tests replace.

A `serve` section beside a `run entry` is refused, since the package would run two programs:

`ita.una`

```una
-- fails with U7008
ita both
  version 0.1.0
  edition 0.4

run
  entry both
  grants
    serve "http://127.0.0.1:8080"

serve
  module both
```

`src/both.una`

```una
module both

import std.host (Reply, Request)

pub main() -> Unit = unit

pub handle(request: internal Request) -> internal Reply = Reply(status: 200, headers: [], body: "")
```

A `handle` that takes exchanges itself is refused: in this form the host owns them.

`ita.una`

```una
ita taker
  version 0.1.0
  edition 0.4

run
  grants
    serve "http://127.0.0.1:8080"

serve
  module taker
```

`src/taker.una`

```una
-- fails with U3033
module taker

import std.host (HttpServer, Reply, Request)

pub handle(request: internal Request) -> internal Reply
  uses @server: HttpServer[close]
=
  @server.close()
  Reply(status: 200, headers: [], body: "")
```

## Edge cases

- **`within`.** With `within w`, each call of `handle` would be cut at `w` milliseconds and answered `503`. In this preview the label of a server's timing, `L_srv`, is the top label, so every `within` of a `serve` section is U4003 at its value (`spec/31 §7` rule 3 item 7). Without it, the grant's `answer` bounds what each peer waits, and `handle` cannot run for ever, since it cannot say `diverges`.
- **No grant.** A run of a `serve` package with no `serve:` grant, in `run grants` or by `--grant`, is a usage error, exit code 2, before anything starts.
- **What `handle` may not do.** It may not say `diverges` (U3027), and `on_stop` may not declare `fails` (U3029). Neither may name `HttpServer` in its `uses`, or a capability whose handlers can perform one of its operations (U3033). Neither may have a `requires` or a refined parameter or result (U7008 at the section's `module`), since no call of the host establishes one.
- **The host's own answers.** A request the host refuses (a malformed head, a limit) is answered by the host with an empty body and never reaches `handle`. So is a request that waits past `answer` (`503` with `Retry-After: 1`). A connection idle past `idle` is closed (`spec/31 §6` rule 3 item 1). A peer that half-closes right after a complete request gets no reply (`spec/31 §6` rule 3 item 2).
- **A dependency's section** is not read: only the root package serves.
- **`una.toml`** has no `[serve]` table: the section is written in `ita.una` only, from edition `0.4`.

## Diagnostics

- [U7008](../../../../crates/una-explain/explain/U7008.md): an invalid `serve` section: no `module`, a module that is not the package's, a `within` out of range, a line of another form or given twice, or the section beside a `run entry` or a `drive` section; and a `handle` or `on_stop` of the wrong shape.
- [U4003](../../../../crates/una-explain/explain/U4003.md): a `within` of a `serve` section, whose cut the preview's `L_srv` refuses.
- [U3033](../../../../crates/una-explain/explain/U3033.md): a `handle` or `on_stop` that names `HttpServer`, or a capability whose handlers can perform its operations, in its `uses`.

## Edition

Edition `0.4` and later, a preview. Editions `0.1` to `0.3` have no servers.

## Normative source

- `spec/31 §7`: the section, the module's shape, its checks and the program the host runs.
- `spec/31 §2`: the `serve:` grant and its limits; `spec/31 §6`: connections and the host's own answers; `spec/31 §1`: the language form.
- Decision record: ADR 0123, rules 16 and 22.
