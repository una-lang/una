# 0074. An HTTP client

- Status: Accepted
- Date: 2026-10-07
- Deciders: the coordinator under the owner's delegation (2026-10-07, N-26), pending Leonardo's review; proposal by Claude (Una-2)
- Resolves: issue #239 (Beta)
- Amends: a new chapter `spec/24-http.md`; `spec/14 §3` (host capabilities), `spec/13 §5` (grants and flags), `spec/11 §5` (traces), `spec/16 §3` (executables); `docs/compiler/architecture.md §7` (dependencies)

## Context

Programs need the network: calling an API, fetching a document, posting a result. Una has no network capability. The JSON of ADR 0073 makes the bodies usable.

The constraints:
- **Authority is per operation and granted.** A function lists the operations it may perform (`uses @files: Files[read]`), and `una run` grants host capabilities by prefix and mode (`files:<path>:read`, `spec/13 §5`), checked before `main` runs (U3012). A request outside a grant fails with `Denied`, as a path outside a `Files` grant does (`spec/14 §3`).
- **Labels.** Every operation argument is a sink with its operation's `accepts` label, and its result keeps its declared label (`spec/07 §7` rule 4). `Files.read` is `internal`, and `Files.write` accepts `internal`.
- **Credentials.** `Keys.hmac` already uses a key from the `keys:<file>` grant by name, and the key never enters the program (ADR 0039).
- **Examples.** They cannot reach host capabilities (U3014). A program's tests use handlers (`spec/05 §5`, `spec/14 §3`).
- **Traces.** Every host operation is a trace event. Arguments above `public` are redacted, and outcomes are redacted by the label of their binder (`spec/11 §5`).
- **Limits.** Memory is bounded by `--max-memory` (`spec/11 §7`). Timing is outside the model (charter §3).
- **Dependencies.** No network or TLS crate is in Cargo.lock, and every crate outside the allowed list needs an ADR saying why the standard library or an allowed crate is not enough (`architecture.md §7`). The host implementations are in the trusted base (charter §3).
- **Performance rule.** The ADR states the complexity, and bodies are streamed or bounded (CLAUDE.md, N-22 guidance).

## Decision

### H1. The capability

A host capability `Http` in `std.host`, with one operation per method, so authority over methods is static: `uses @http: Http[get]` can never post.

```una
pub kind Header
  name: Text
  value: Text

pub kind Response
  status: Int
  headers: List[Header]
  body: Text

pub kind HttpError = Denied | BadUrl | Unreachable | TimedOut | TooLarge | Tls | Protocol

pub capability Http
  op get(url: Text, headers: List[Header], key_name: Option[Text]) -> internal Response
    fails HttpError
    accepts internal
  op post(url: Text, headers: List[Header], body: Text, key_name: Option[Text]) -> internal Response
    fails HttpError
    accepts internal
  op put(url: Text, headers: List[Header], body: Text, key_name: Option[Text]) -> internal Response
    fails HttpError
    accepts internal
  op delete(url: Text, headers: List[Header], key_name: Option[Text]) -> internal Response
    fails HttpError
    accepts internal
```

- **Statuses are not failures.** A 404 or a 500 is a `Response` with that `status`. The program decides what it means. `HttpError` is for requests that got no HTTP response:
  - `Denied`: outside the grants, or a method the grant does not allow;
  - `BadUrl`: not an absolute `http` or `https` URL, or one with user info;
  - `Unreachable`: DNS or connection failure;
  - `TimedOut`;
  - `TooLarge`: over the response limit of H4;
  - `Tls`: a handshake or certificate failure;
  - `Protocol`: a malformed response.
- **The body is `Text`.** A response body that is not valid UTF-8 is `Protocol`: Una has no byte strings in v0.2. A body in another encoding is out of scope.
- **Not in scope:** cookies, caches, HTTP/2, compression (requests send `Accept-Encoding: identity`), and redirects that leave the grants (H2).
- **HTTP/1.1 only, and no proxies** (N-26). The client speaks HTTP/1.1, and connects directly to the origin of each request. It ignores the variables `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY`, in any case, for determinism and explicit authority: a grant names where requests go, and nothing in the environment changes that.
- Alternatives: one `send(request)` operation (C), or `get` and `post` only (D).

### H2. Grants

- **Syntax.** `http:<origin>` or `http:<origin>:<methods>`, in `[run] grants` and `--grant`:
  - `<origin>` is a scheme, a host and an optional port: `https://api.example.com`, or `http://127.0.0.1:8080`.
  - A leftmost `*.` matches exactly one more label (`https://*.example.com` matches `a.example.com`, not `example.com` or `a.b.example.com`).
  - `<methods>` is a comma list among `get`, `post`, `put` and `delete`, by default `get`.
- **Deny by default.** A request is allowed only when its URL's origin matches a grant that allows its method. Otherwise it fails with `Denied` before any connection. Ports must match exactly; a missing port means the scheme's default.
- **`https` unless granted otherwise.** A grant names its scheme, so plain `http` is possible only where a grant says `http://`, as for local services in tests.
- **Redirects.** At most 5 are followed, and each one only when its target matches a grant for the same method, after a `303`, or for `get`. Anything else is the redirect response itself, returned to the program as a `Response`.
- **Checks.** `uses` of `Http` operations that no grant allows are U3012 before `main`, as for the other capabilities. A malformed grant is a usage error, or U7002 in `una.toml`.
- **Name resolution** is the system's. A grant names hosts, not addresses, and the program cannot see addresses. DNS rebinding is out of the model, and listed in the build report as path races are for `Files`.

### H3. Labels and credentials

- **What a request carries.** Every argument of every operation is a sink that `accepts internal`, as `Files.write` does: `internal` data may go to a granted origin. A `secret` value reaches a request only through a policy (`spec/07 §5`), whose `via` function documents why. A `secret` URL, header or body is U4003 otherwise.
- **Responses** are `internal`, as `Files.read` is. A program that knows a response is public declassifies it through a policy.
- **Credentials without the secret.** `key_name: Some(name)` sends `Authorization: Bearer <key>`, with the key named `name` taken from the `keys:<file>` grant (ADR 0039). The key never enters the program's values, its traces or its labels. An unknown name fails with `Denied`, and the trace shows the name only.
  - This covers the common case of an API token without a policy.
  - Other schemes (Basic, custom headers) use a policy.
- Alternatives: a per-parameter `accepts` (E); `public` sinks only (F).

### H4. Bounds, streaming and time

- **Bounded, not streamed.** Una v0.2 has no streams, so bodies are whole `Text` values, within bounds:
  - A response body over `--max-response <MiB>` (default 16) fails with `TooLarge`, without reading past the limit. The connection is closed.
  - The headers of a response are bounded: at most 100 headers and 64 KiB in all, else `Protocol`.
  - A request body is a `Text` the program already holds, so the memory budget bounds it (`spec/11 §7`).
- **Time.** Each request has one deadline, `--http-timeout <seconds>` (default 30): the total wall-clock time of the request (N-26), covering name resolution, connecting, the TLS handshake, writing the request and reading the response, across redirects. It is not a per-read timeout, so a server that trickles bytes (slowloris) cannot stretch a request past it. Past it, the request fails with `TimedOut`. Timing is outside the model, so a `TimedOut` is an outcome the program must handle, like any failure of the outside world.
- **Complexity.** A request takes O(u + h + b) time and space for the URL, headers and body it sends. A response takes O(h' + b') for what it receives, with b' ≤ the limit, so it is bounded. Parsing is single-pass and linear: the status line, the headers, then `Content-Length` or chunked framing. No step is superlinear in the bytes received.
- **Tasks.** A request blocks only its own task's thread (`spec/19 §8`), as the other host operations do.

### H5. Traces, replay and redaction

- **The event** is that of `spec/11 §5`, with `instance` `host:Http` and `op` the method:
  - `url` and `key_name` are shown when `public`, and redacted otherwise;
  - `headers` and `body` are redacted by their labels, as any argument is;
  - the outcome, an `internal Response`, is redacted by its binder's label (`<redacted:internal>`), as `Files.read` is.
- **Replay** (`una replay`, v0.3, #57) needs full results, which it gets from a recording made by the run, not from the trace's redacted outcome. That is the same arrangement as for `Files.read`, and this ADR adds nothing to it.
- The credential of H3 appears nowhere: not in the trace, the outcome, or any diagnostic.

### H6. `una test` and offline programs

- **Examples cannot use `Http`** (U3014), as for every host capability. They are deterministic.
- **Programs are tested with handlers.** A test package or an example's `given` installs a handler `for Http` that answers from a table, a recorded response or a fixture (`spec/05 §5`). This needs nothing new.
- A recording mode for `una run` (`--http-record`) belongs with `una replay` (#57). It is not part of this ADR.

### H7. The implementation and the trusted base

One client serves both engines: a new crate `una-http`, called by the interpreter's `World` and by `una-rt`'s host functions, so parity of everything but the network itself holds by construction.

- **HTTP/1.1: written here**, about 700 lines of safe Rust over `std::net::TcpStream`:
  - writing the request line, headers and body;
  - parsing the status line and headers, with `Content-Length` and `chunked` framing;
  - redirects (H2), the limits of H4, and the deadline, through the stream's read and write timeouts.

  Writing it ourselves is simpler to audit than a general client (alternative B), since the language needs a small, strict subset.
- **TLS: `rustls`**, with the `ring` crypto provider and `webpki-roots`, the Mozilla root store compiled in.
  - **Why a new dependency.** TLS cannot be written safely in-house, and the standard library has none. `rustls` is memory-safe Rust, widely audited, and needs no system library, so both engines and every platform behave the same.
  - **Why compiled-in roots.** They are reproducible and independent of the machine. The root store changes with the crate's version, which `Cargo.lock` pins and the build report lists.
  - **The cost.** `ring` holds assembly and C for its primitives, so the trusted base grows by `rustls`, `ring`, `webpki-roots`, `rustls-pki-types`, `untrusted` and their small dependencies. `architecture.md §7` gains this ADR as their justification.
  - **Binary size** (N-26). A native executable that does not use `Http` MUST NOT grow: the TLS stack is linked only into executables that use it, and the implementation checks that the linker drops it. The pull request reports the size of `examples/hello` built natively before and after, on macOS and on Linux.
- **Fuzzing the parser** (N-26). The response parser is new trusted code, so property or fuzz tests cover:
  - the status line and headers;
  - `Content-Length`;
  - chunked encoding with trailers, and malformed chunk sizes;
  - every limit of H4;
  - every response split at every byte boundary, which must parse the same as in one piece.
  - Alternatives: OpenSSL or the platform's TLS (G), `aws-lc-rs` (H), or the system's `curl` (I).
- **The build report** lists `Http` among the host capabilities and its dependencies in the trusted base (`spec/12`).

### H8. Determinism and parity

- **Determinism.** The network is outside the model: two runs may get different responses. Everything the client decides is deterministic: grants, redirects, limits and the order of headers it sends.
- **Parity.** Tests use a local server on `127.0.0.1`, granted as `http://127.0.0.1:<port>`, with canned responses, so the differential test compares both engines without the network. TLS is tested against a local `rustls` server with a test certificate authority, kept in the repository's test fixtures. The test certificate authority is installed for those tests only, through a test hook, never in release builds.

### H9. Scope

- Edition 0.2 only.
- New diagnostics: none. U3012, U3014, U4003 and U7002 apply as they do.
- New flags: `--max-response` and `--http-timeout`, for `una run` and executables alike (`spec/16 §3`).

## Consequences

- Programs can call granted APIs with static method authority, deny-by-default origins, bounded memory and time, labeled responses, and tokens that never enter their values.
- Programs cannot handle binary bodies, streaming, or other encodings until the language has bytes and streams.
- The trusted base grows by the TLS stack. That is the main cost, and it is stated and justified here.

## Alternatives considered

- **A. A capability per origin**, declared by the program (`capability Weather` bound to a host). This needs parameterized capabilities (backlog #59, v0.4). The grant patterns of H2 give most of the benefit now.
- **B. `ureq` or `reqwest`**, to avoid writing HTTP/1.1. They pull in many more crates (URL parsing, compression, cookies, async runtimes for `reqwest`), whose behavior we would have to pin and restrict. That is a larger trusted base for features the language does not use.
- **C. One `send(request)` operation** with a method field. It is more flexible, but method authority would move from the static `uses` list to run-time grants only. H1 keeps both.
- **D. `get` and `post` only**, as the issue's title suggests. `put` and `delete` cost nothing extra, and REST APIs need them.
- **E. A per-parameter `accepts`** (a public URL, internal bodies). It is more precise, but it needs a new per-parameter form, as `Keys` has today only as an exception. H3 keeps one label per operation, as the rest of `std.host` does.
- **F. Sinks that accept only `public`.** This is safer by default, but every program that sends a file's contents would need a policy. `internal` matches `Files.write`, where the user also chose the destination.
- **G. OpenSSL, `native-tls` or the platform's TLS.** These are system libraries, which vary across machines and are harder to pin, and Linux would need `libssl` at run time. They are rejected for reproducibility.
- **H. `rustls` with `aws-lc-rs`.** It is FIPS-oriented, but a larger C code base with a CMake build. `ring` is smaller. This can be revisited if FIPS matters.
- **I. Running the system's `curl`.** No crate, but behavior and version vary by machine, it needs an external program at run time, and argument handling becomes a new attack surface. Rejected.

## Amendment of 2026-10-07: implementation decisions (N-28)

Decided by the coordinator under the owner's delegation (N-28), pending Leonardo's review; raised by Claude (Una-2) while implementing.

1. **The credential stays with its origin.** `key_name` sends its key only to the origin of the request the program made. A redirect to another origin, granted or not, carries no `Authorization`.
2. **`HttpError` gains `BadRequest`**, for a request header the client must refuse:
   - a name that is not a token;
   - a value with a control character, which would allow header injection;
   - a header the client sets itself (`Host`, `Content-Length`, `Transfer-Encoding`, `Connection`).

   `BadUrl` stays for URLs. The kind is now `Denied | BadUrl | BadRequest | Unreachable | TimedOut | TooLarge | Tls | Protocol`.
3. **The test certificate authority is a debug-build hook.** It is gated by `debug_assertions` inside `una-http`, never by a cargo feature, so no release build can trust it. A release-build test, which `scripts/local-ci.sh` runs, shows the test authority rejected.
4. **Grants are read by one parser.** `una_core::http` holds URLs and `http:` grants, so the checker, the manifest and both engines read them alike. `una-http` holds the client, and is the only crate that links the TLS stack.
5. **Executables keep only the runtime they call.** `una build` links with `-dead_strip` (macOS) or `--gc-sections` (Linux), so an executable that never uses `Http` carries no TLS code (H7). A test checks it.

## Amendment of 2026-10-07: independent check (N-30)

Decided by the coordinator under the owner's delegation (N-30), pending Leonardo's review; found by the coordinator's independent check of the implementation.

1. **Traces redact the prelude's labeled fields in both engines.** The interpreter reads the field types of the prelude's kinds from their declarations, as the executable does, so `Header.value` is `<redacted:internal>` in every trace.
2. **The program's credentials stay with their origin too.** A redirect to another origin drops `Authorization`, `Cookie` and `Proxy-Authorization`, in any letter case, besides the key of `key_name` (amendment N-28, item 1). A request with both `key_name` and an `Authorization` header is `BadRequest`.
3. **A key with a control character is `BadRequest`**, as a header value is, since it is written into a header.
4. **`--http-timeout` is at most a day** (86400 seconds); more is a usage error, and the client computes its deadline without overflow.
5. **An IPv6 grant names its host in brackets**, `http:http://[::1]:8080`; the colons inside the brackets are the host's.
6. **RFC 9112 details.** A `204` or `304` has no body, whatever its headers say. At most 10 `1xx` responses precede the final one. A chunk size has no whitespace around its digits.
7. **The residual of H7 on macOS.** An executable that never uses `Http` carries no `rustls` or `webpki` code. On Linux it carries no `ring` code either. On macOS it keeps about 39 KB of `ring`'s assembly kernels (SHA-2, AES, ChaCha20, P-256), which no call reaches. `ld64` cannot dead-strip an object assembled without `.subsections_via_symbols`, and the loading of the runtime archive, transitive by object file, pulls those objects in. The size test bounds this residual at 64 KiB on macOS and at zero on Linux.

## Amendment of 2026-10-07: no private keys in the repository (N-31)

Decided by the coordinator under the owner's delegation (N-31), pending Leonardo's review.

The test certificate authority of H8 is not kept in the repository's test fixtures. The TLS tests make it, and the server certificates it signs, once per test process with `rcgen`, a dev-dependency of `una-http` outside the trusted base. They write only the authority's certificate to a temporary file for the debug-only hook. The repository and its history hold no private key, which a secret scanner would flag.

## Amendment of 2026-10-07: the residual of H7 (N-32)

Decided by the coordinator under the owner's delegation (N-32), pending Leonardo's review.

H7 reads: an executable that never uses `Http` carries no TLS logic. On macOS up to 64 KiB of the TLS library's assembly primitives may remain, unreachable (amendment N-30, item 7). The size test enforces at most 64 KiB on macOS and none on Linux.

## Amendment of 2026-10-07: credentials across a chain (N-35)

Decided by the coordinator under the owner's delegation (N-35), pending Leonardo's review; found by the coordinator's recheck.

Once a redirect leaves the request's origin, the key of `key_name` and the program's `Authorization`, `Cookie` and `Proxy-Authorization` stay dropped for the rest of the chain, as browser `fetch` does. A chain A → B → A does not send them to A at its last hop, whose path B chose.

## Amendment of 2026-10-07: failures labeled by their arguments (N-39, N-40)

Decided by the coordinator under the owner's delegation (N-39, N-40), pending Leonardo's review; found by the cross-review of v0.3.0 (#308, H-1), fixed by #310.

1. **Whether a request fails reveals its arguments** (N-39). `Denied`, `BadUrl` and `BadRequest` depend on the URL, the headers and the key name, which accept `internal`. So the failure of an `Http` operation, its tag and payload, is labeled by the join of those arguments' labels, as N-27 labels the failure of a prelude function. `attempt` gives a `Result` with that label, `if error use` runs under it, and `if error pass` and `if error fail` are U4002 when it is above `public`. The body decides nothing and does not count. The rule is general (`spec/07 §7` rule 4): the path of `Files` and the key name of `Keys.hmac` decide their failures, each capped at what the operation accepts, so only `Files.write`, which accepts an `internal` path, changes.
2. **Header values abort instead of failing** (N-40). `Header.value` is declared `internal`, so the checker cannot tell a public value from an `internal` one: counting it would label every request's failure. The headers decide through the list's labels and the names only. A value with a control character other than a tab is no longer `BadRequest`: the run aborts with U6004 at the operation, before the URL is read and before anything is sent, in both engines, with the exit status, trace and redaction of U6003. An abort is outside the noninterference model (`spec/07 §1`), which is termination-insensitive: it reveals at most one bit per run, not one per request. A full disk failing a write is resource exhaustion, outside the model too.
