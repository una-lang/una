# health

A health endpoint: an HTTP server that answers `GET /health` with `200 ok`, and every other request with `404`.

This is the example of ADR 0123, "A health endpoint" ([HTTP servers](../../docs/adr/0123-http-servers.md)), in the host-driven form of [`spec/31 §7`](../../docs/spec/31-http-servers.md): the package declares no `main`. Its `ita.una` names, in a `serve` section, the module whose `handle` the host calls once for each request it reads, and grants the address it listens on. The host owns every connection and answers each request with the reply `handle` returns, so the module itself is one ordinary function, which the checker, the verifier and the tests see as any other.

What it demonstrates:

- a manifest in `ita.una`, edition `0.4` ([manifest](../../docs/guide/configure/ita/manifest.md)), with a `serve:` grant and a `serve` section;
- `handle(request: internal Request) -> internal Reply` of `std.host`, the shape the host calls;
- [tests](../../docs/guide/language/examples/test.md) that call `handle` directly with a request from `test_request` of `std.testing`, the request the host would read from a peer;
- an [`internal`](../../docs/guide/language/information-flow/labels.md) request and reply, the label the host gives what its peers send and read.

## Check and test it

From this directory:

```sh
una check .   # copy examples/health
una test .
una report .
```

`una test .` runs the three tests, each with a request built by `test_request`:

```text una="test ."
test health "the health endpoint answers ok" ... ok
test health "every other path is not found" ... ok
test health "a request of another method is not found" ... ok

3 tests: 3 passed, 0 failed
```

`una run .` serves: it listens on `127.0.0.1:8080`, answers each request with the reply of `handle`, and ends with exit code 0 when it is stopped (Ctrl-C or SIGTERM), after the requests in flight are answered (`spec/31 §1` rule 10). `una build` of a package that serves is still a usage error in this release. The toolchain's own tests run it over loopback in both engines.

## How it works

The `serve` section names the module `health`, and `run grants` holds the address:

```text file="ita.una"
run
  grants
    serve "http://127.0.0.1:8080"

serve
  module health
```

`handle` answers from the method and the decoded path segments of the request. Its parameter and its result are `internal`: a reply carries what the server's peers may read, and a `Reply` built from `secret` data is refused.

## Edition

Edition `0.4` ([edition](../../docs/guide/configure/package/edition.md)).
