# orders

A JSON API: an HTTP server that reads and stores the orders of a shop, kept in a document database with an HTTP interface.

This is the example of ADR 0123, "A JSON API over a database-like capability" (a decision record of the HTTP servers), in the host-driven form of [`spec/31 §7`](../../docs/spec/31-http-servers.md). The package declares no `main`: its `ita.una` names, in a `serve` section, the module whose `handle` the host calls for each request. `handle` routes the request over a capability of its own, `Orders`. In the program, a handler implements `Orders` over `Http`, and the tests install handlers that keep the orders in memory.

What it demonstrates:

- a `serve` section and a `serve:` grant with a delivery rate (6000 a minute) and a body limit (64 KiB), beside an `http` grant for the database ([manifest](../../docs/guide/configure/ita/manifest.md), [serve](../../docs/guide/configure/ita/serve.md));
- a [capability](../../docs/guide/language/capabilities/capability.md) that the server needs and two [handlers](../../docs/guide/language/capabilities/handler.md) of it: one over `Http`, and in-memory ones for the tests;
- a failure of `Http`, which is `internal`, reported as a value (`Unavailable`, `Refused`) and answered `503`;
- JSON read with `parse_json`, and written by interpolation, since the fields of `Json` are `public` and cannot hold `internal` data;
- [tests](../../docs/guide/language/examples/test.md) of the routes with `test_request` of `std.testing`, each with a handler of `Orders` installed by `given`.

## Check and test it

From this directory:

```sh
una check .   # copy examples/orders
una test .
una report .
```

`una test .` runs the six tests, each with an in-memory store:

```text una="test ."
test api "an unknown order is 404" ... ok
test api "a stored order is read back as JSON" ... ok
test api "an order is created from its JSON" ... ok
test api "a quantity that is not positive is 422" ... ok
test api "an id that is not a number is 400" ... ok
test api "another method is 405" ... ok

6 tests: 6 passed, 0 failed
```

`una run .` serves on `127.0.0.1:8081`. With a document database on `127.0.0.1:5984` holding the database `shop`, `GET /orders/7` reads the document `/shop/orders/7`, and `POST /orders/7` with a body such as `{"item": "apples", "quantity": 2}` writes it. Without one, every request that needs the store is answered `503`. The toolchain's own tests run the server over loopback in both engines, with requests that never reach the store.

## How it works

The `serve` section names the module `api`, and `run grants` holds both addresses:

```text file="ita.una"
run
  grants
    serve "http://127.0.0.1:8081"
      at most 6000 per minute
      body at most 64 KiB
    http "http://127.0.0.1:5984" get, put

serve
  module api
```

`handle` installs `document_store`, the handler of `Orders` over `Http`, around `route`, which sees only `Orders`. So `route` and the functions it calls are tested without a database or the network. The request and every reply are `internal`, and so is everything the store reads.

## Edition

Edition `0.4`, a preview ([edition](../../docs/guide/configure/package/edition.md)).
