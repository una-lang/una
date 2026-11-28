# Serving HTTP

This page shows how a package of edition `0.4` answers HTTP/1.1 requests: how a run is granted an address to listen on, the two ways to write a server, what happens when the server is stopped, how its tests reach the request handling, and what the report says about it. The [specification](../spec/31-http-servers.md), `spec/31`, is normative, and this page only points to it. Edition `0.4` is a preview, and the limits of servers in this release are listed in [Known limits](known-limits.md#servers).

The worked example is [`examples/health`](../../examples/health/): a server that answers `GET /health` with `200 ok`, and every other request with `404`.

## Grants: an address and its limits

A run listens only on an address that a `serve:` grant names; there is no other way for a program to accept connections. In `ita.una` the grant is a `serve` line under `run grants`, with one item per line below it (`spec/31 §2` rule 3). This `run` section, in place of the one of `examples/health`, grants its address with four limits:

```text section="examples/health/ita.una"
run
  grants
    serve "http://127.0.0.1:8080"
      at most 256 at once
      body at most 64 KiB
      answer within 10 seconds
```

On the command line the same grant is `--grant 'serve:http://127.0.0.1:8080;256/once;body=64;answer=10'`, its items written as `;` suffixes and the whole value quoted, since a shell ends a command at an unquoted `;` (`spec/31 §2` rule 1). The address is an IPv4 address, or an IPv6 address in brackets, and always a port; a host name is refused, since nothing resolves a grant. `0.0.0.0` and `[::]` mean every interface (`spec/31 §2` rule 1).

Every grant has limits, whether it writes them or not. The defaults include 256 exchanges delivered and not yet answered (`/once`), 1024 requests read and waiting to be delivered (`waiting`), 1024 open connections, 64 of them from one peer at that default (`peer`), a body of at most 1024 KiB, and 30 seconds from the end of a request to the start of its reply (`answer`); the table of `spec/31 §2` rule 2 lists every item, its default and its range. When a limit is reached, the host answers for the program, with an empty body, as `spec/31 §6` rule 7 lists: `503` with `Retry-After: 1` when `waiting` is full or a request waits past `answer`, `408` for a request not read in time, `421` for a `Host` the grant does not allow.

`Host` is checked too: without a `hosts` item, a request's `Host` must be `localhost` or an IP literal, else the host answers `421`; with `hosts`, it must match a listed name (`spec/31 §2` rule 5).

## The `serve` section

In the host-driven form, the package declares no `main`. A `serve` section in `ita.una` names the module whose `handle` the host calls once for each request it reads (`spec/31 §7` rule 1). This is the manifest of `examples/health`:

```text file="examples/health/ita.una"
ita health
  version 0.1.0
  edition 0.4

run
  grants
    serve "http://127.0.0.1:8080"

serve
  module health
```

The module declares `handle`, and optionally `on_stop`, in exactly these shapes, with `Request` and `Reply` of `std.host` (`spec/31 §7` rule 2):

```text shapes="examples/health/src/health.una"
pub handle(request: internal Request) -> internal Reply
pub on_stop() -> Unit
```

The host owns every connection and answers each request with the reply `handle` returns, so `handle` is an ordinary function, which the checker, the verifier and the tests see as any other. This is `handle` of `examples/health`:

```text file="examples/health/src/health.una"
pub handle(request: internal Request) -> internal Reply
=
  if request.method is Get and request.segments == ["health"]
    Reply(status: 200, headers: [], body: "ok")
  else
    Reply(status: 404, headers: [], body: "")
```

The request and the reply are `internal`: a reply carries what the server's peers may read, and a `Reply` built from `secret` data is U4001 (`spec/31 §7` rule 3 item 5). `handle` may declare `uses` and `fails`; a failure of `handle` becomes `500` with an empty body, never the failure's payload (`spec/31 §7` rules 3 and 4). Neither function may say `diverges`, or name `HttpServer` in its `uses` (U3027, U3033; `spec/31 §7` rule 3).

## The language form

A program can also take requests itself, through the operations `next`, `reply` and `close` of the capability `HttpServer`. Its `main` is a `diverges` function with a `for ever` loop that calls `next` and starts a task per request (`spec/31 §1` rule 6). This server answers every request with `200 ok`; its `ita.una` names `run entry main` and a `serve:` grant, and no `serve` section:

`ita.una`
```text
ita server
  version 0.1.0
  edition 0.4

run
  entry main
  grants
    serve "http://127.0.0.1:8080"
```

`src/main.una`
```text
module main

import std.host (Exchange, HttpServer, Reply)

answer(exchange: internal Exchange) -> Unit
  uses @server: HttpServer[reply]
=
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "ok"))
    if error use unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  diverges
=
  tasks at most 1000 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange)
        is Err(failure) -> unit
    on stop
      @server.close()
```

`tasks at most N at once` bounds the program's tasks, and the grant's `/once` the exchanges open; a server that answers in its loop, with no `tasks`, is legal too (`spec/31 §1` rule 6). A package has one program: a `serve` section beside a `run entry` is U7008 (`spec/31 §7` rule 1).

## Running a server

`una run` serves a package with a `serve` section, and a program of the language form, in the interpreter over live listeners, and so does the executable that `una build` makes of it (below), after every check of the program and of its grants (`spec/31 §1` rule 10, `§7` rule 5). From `examples/health`:

```sh
una run .   # copy examples/health
```

Once the serve threads have started, and before the first request, the run writes one line on stderr for each `serve:` grant, in the order of the grants (`spec/31 §6` rule 14):

```text una="run ." in="examples/health"
serving on 127.0.0.1:8080 (granted by the package)
```

A grant given with `--grant` is written `` serving on 127.0.0.1:8080 (granted by a `--grant` flag) `` instead, so a run never serves an address unseen.

**A client that half-closes.** A peer that ends its side of the connection (a TCP FIN, `shutdown(SHUT_WR)`) right after a complete request gets no reply: while a request waits for its answer the host still reads, so it sees the end at once, takes the peer as gone, closes the connection and discards the reply (`spec/31 §6` rule 3 item 2). Some clients do this, such as `nc` in the modes that shut down the socket when their input ends; such a client sees an empty reply, and the server is unharmed. A client must keep its sending side open until the reply has arrived, as `curl` and browsers do.

**Stopping.** SIGINT (Ctrl-C) or SIGTERM is a host stop (`spec/26 §6` rule 1). The host closes every listener at once and closes idle connections; requests not yet delivered, or still being read, are answered `503`; delivered exchanges drain, each reply carrying `Connection: close` (`spec/31 §6` rule 9). In the host-driven form, `next` fails with `Interrupted`, the loop ends, the started tasks drain, `on_stop` runs, and the run ends with exit code 0 (`spec/31 §7` rule 4). The hard stop comes at the end of the grace period, `--stop-grace` seconds after the first request (10 by default), or at a second request: each open exchange is answered `503`, and the run ends with U6007, exit code 1 (`spec/26 §6` rule 6, `spec/31 §6` rule 9).

Only a run that can take a host stop, one with a `serve` section, a driver, or a `main` that says `diverges`, installs the handlers of SIGINT and SIGTERM. A server of the language form whose `main` does not say `diverges` keeps their default actions: SIGTERM ends its process at once, with no drain, and a shell reports exit code 143 (`spec/31 §6` rule 14).

**Building.** `una build` builds a program that serves, a package with a `serve` section or a program of the language form, into a native executable, with no test flag (`spec/31 §1` rule 10, `§7` rule 5). The executable checks its grants, those of `run grants` and of its own `--grant` flags, before `main`, writes the same `serving on ...` line for each address, serves as `una run` does, and drains at SIGINT or SIGTERM as above. `una build` of a package with a `serve` section needs no `serve:` grant, since the executable's may come from its flags; an executable started without one is a usage error, exit code 2 (`spec/31 §7` rule 1).

## Testing a server

Tests run in the interpreter (`spec/31 §8` rule 5). `std.testing` gives two functions for servers, which only tests and the handlers that only tests install may call (U3015; `spec/31 §8`):

- `test_request(method: Method, target: Text, body: Text) -> Request fails TestRequestError` gives the request the host would read from a peer that sent that method and target with `HTTP/1.1`, the one header `Host: test`, and `body`. The host's own reader reads it, so its `segments` and `params` are decoded as the host decodes them, and it fails with `Refused` where the host would answer `400`, as for a `..` segment or a `#` (`spec/31 §8` rule 1).
- `test_exchange(request: Request) -> Exchange` gives an exchange of that request with a fresh reply right, for a test of the language form: a handler of `HttpServer` that the test installs with `given` receives it in `reply` (`spec/31 §8` rule 3).

A test of the host-driven form calls `handle` directly. These are the three tests of `examples/health`, each with a request that `test_request` builds:

```text file="examples/health/src/health.una"
test "the health endpoint answers ok"
  do handle(test_request(method: Get, target: "/health", body: ""))
  expect result.status == 200

test "every other path is not found"
  do handle(test_request(method: Get, target: "/other", body: ""))
  expect result.status == 404

test "a request of another method is not found"
  do handle(test_request(method: Post, target: "/health", body: ""))
  expect result.status == 404
```

The first sends `GET /health`, whose segments are `["health"]`, and reads `200`. The second keeps the method and changes the path, and the third keeps the path and changes the method, so each branch of the condition in `handle` is taken by one test. `una test .` runs them:

```text una="test ." in="examples/health"
test health "the health endpoint answers ok" ... ok
test health "every other path is not found" ... ok
test health "a request of another method is not found" ... ok

3 tests: 3 passed, 0 failed
```

A test of the language form calls the function that answers an exchange with a handler installed, as `given @server = accepting()` with a handler `accepting` of `HttpServer`, and checks `expect performed @server.reply` (`spec/31 §8` rule 3).

## What the report says

`una report` of a package with a `serve:` grant or a `serve` section adds what it serves (`spec/31 §7` rule 6, `spec/13 §6`):

- `HttpServer` among the host capabilities, and `server: una-http/serve`, the server code of the host, in the `tcb` lines;
- a `servers:` section with one line per grant of `run grants` and its limits, the defaults applied, such as `http://127.0.0.1:8080: 256/once, rate none, waiting 1024, connections 1024, peer 64, ...`;
- three entries at the end of `out of model`: what a server's peers send in reaction to its timing, and which requests its full queues refuse; what a peer receives when the host's deadlines cut an exchange; and which request a call of `next` receives when several serve threads hold requests;
- a `warning:` line for each grant that is plain `http` on an address that is not loopback, a wildcard included, and for `hosts=*`, `peer=none`, `peer_buffers=none` and `tls12`.

With `--json`, these are the keys `tcb.host_capabilities`, `tcb.serve_grants`, `tcb.server`, `tcb.tls`, `out_of_model` and `warnings` of `una.report/2`.

## Refusals

- **`within` in a server.** Under the preview, the label `L_srv` of a server of the language form, and of a package with a `serve` section, is the top label, and so is every timing site of the package (`spec/31 §1` rule 5 item 7). So every `within` of a `serve` section is U4003 (`spec/31 §7` rule 3 item 7), and a `start ... within` in the block of a `tasks` statement whose expression performs an operation that accepts `internal` is refused: a server starts its request tasks without `within`, and the grant's `answer` bounds what each peer waits (`spec/31 §1` rule 5 item 8).
- **`https` grants fail closed.** An `https` grant needs `certificate` and `key` files, which the run loads and checks before `main` (`spec/31 §2` rules 7 and 8). The TLS server waits for the decision on the project's TLS provider (#1059), so a run with an `https` grant does not serve: it fails closed instead of serving plain bytes on an `https` address (`spec/31 §6` rule 10).
- **No `serve:` grant.** A run of a package with a `serve` section and no `serve:` grant, in `run grants` or by `--grant`, is a usage error, exit code 2, before anything starts (`spec/31 §7` rule 1).
- **A wrong `handle`.** A module without `handle` of exactly the shape above, or with an `on_stop` of another shape, is a usage error, exit code 2, of `una run` and `una build`, that names the first difference found (`spec/31 §7` rule 2).

## Residual limits

- **No share per peer for three limits.** A grant's `waiting`, `/once` and `delivered` have no share per peer, so one peer, or a few, can take the whole of each budget. At the defaults, `waiting` holds 1024 requests and one peer may hold 64 connections, so about 16 well-behaved addresses can fill `waiting` and hold back every other peer's requests until those are delivered or time out (`spec/31 §2` rule 2, `§6` rule 5). An operator facing it raises the grant's limits or puts a filter in front (ADR 0123 rule 7).
- **A grant that serves nothing.** A run that holds a `serve:` grant but does not serve, whose program never uses `HttpServer` and that has no `serve` section, still binds its listeners before `main`, accepts nothing on them and writes no `serving on` line (`spec/31 §6` rule 14). Read a package's `run grants`, or its report, before running it.
- **`--serve-script`.** The scripted connections of `--serve-script` run only in the toolchain's own test builds (`spec/31 §8` rule 6).
