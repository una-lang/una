# proxy

A rate-limited proxy: an HTTP server that forwards each `GET` to an upstream and relays its answer, at most 600 requests a minute.

This is the example of ADR 0123, "A rate-limited proxy", in the language form of [`spec/31 §1`](../../docs/spec/31-http-servers.md) rule 6. The program's `main` takes each exchange with `next` and starts a task per request, and the task forwards the request upstream and answers with `reply`. The rate is the serve grant's: the host delivers at most 600 exchanges a minute and at most 256 open at once, and answers the others `503` with `Retry-After: 1`.

What it demonstrates:

- a `run entry` with a `serve:` grant that limits delivery, and an `http` grant for the upstream ([manifest](../../docs/guide/configure/ita/manifest.md), [serve](../../docs/guide/configure/ita/serve.md));
- `HttpServer` taken by `main`: `next` in a `for ever` loop, a `tasks` statement that starts a task per exchange, `reply` from that task, and `close` at the host stop;
- headers forwarded and relayed by allowlists, so a peer's `forwarded`, `x-forwarded-*`, credentials and `expect` never reach the upstream, and a status the host would not send becomes `502`;
- [tests](../../docs/guide/language/examples/test.md) of the pure parts, the allowlists and the relay.

ADR 0123 sketched two more limits that this example leaves out, since the preview refuses them in a server. The preview's label of the serve threads, `L_srv`, is the top label, so every timing site of a server is too (`spec/31 §1` rule 5 item 7):

- a `start ... within 5 * second` per request (U4003); here the grant's `answer` (30 seconds by default) bounds what each peer waits;
- the `limiter` of `std.limit` and limits on the `http` grant, whose label `L_lim` must be at most `internal`.

Once `L_srv` becomes precise (#1155), they can come back.

## Check and test it

From this directory:

```sh
una check .   # copy examples/proxy
una test .
una report .
```

`una test .` runs the three tests:

```text una="test ."
test proxy "only allowed headers go upstream" ... ok
test proxy "a status the host would not send becomes 502" ... ok
test proxy "an upstream's 304 is relayed without a body" ... ok

3 tests: 3 passed, 0 failed
```

`una run .` serves on `127.0.0.1:8082` and forwards each `GET` to `https://upstream.example`, which stands for a real upstream: change the URL in `src/proxy.una`, and the `http` grant with it, to serve another. A request of any other method is answered `405` without a request upstream. The toolchain's own tests run the server over loopback in both engines with such a request. A host stop (Ctrl-C or SIGTERM) ends the `for ever` loop, and its `on stop` closes the server.

## How it works

`run grants` holds the listener with its limits, and the upstream:

```text file="ita.una"
run
  entry proxy
  grants
    serve "http://127.0.0.1:8082"
      at most 600 per minute
      at most 256 at once
    http "https://upstream.example" get
```

`main` says `diverges`, so the run installs the handlers of SIGINT and SIGTERM, and a stop drains the started tasks before `on stop` runs.

## Edition

Edition `0.4`, a preview ([edition](../../docs/guide/configure/package/edition.md)).
