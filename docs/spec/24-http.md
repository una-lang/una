# 24. HTTP

Status: Normative, v0.2 (ADR 0074 and its amendment of 2026-10-07).

The host capability `Http` sends HTTP/1.1 requests to the origins that grants allow, and returns their responses.

**Editions.** The constructs of this chapter exist only in packages of edition `0.2` and later (`spec/13 §7`). In edition `0.1`, `Http`, `Header`, `Response` and `HttpError` are ordinary names.

## 1. The capability

1. **Declarations.** `std.host` declares, in the prelude (`spec/14 §3`):

   ```una
   pub kind Header
     name: Text
     value: internal Text

   pub kind Response
     status: Int
     headers: List[Header]
     body: Text

   pub kind HttpError = Denied | BadUrl | BadRequest | Unreachable | TimedOut | TooLarge | Tls | Protocol

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

2. **Names.** A user kind or capability named `Http`, `Header`, `Response` or `HttpError` is U1002 in edition 0.2. A module MAY import the four names from `std.host`, as `import std.host (Http, Response)` (`spec/10 §11` rule 10, #307).
3. **Statuses.** Every HTTP response is a `Response`, whatever its status. A request fails only when it gets no response it may return:
   - `Denied`: no grant allows the URL's origin with the method, or `key_name` names no key of the grants;
   - `BadUrl`: the URL is not an absolute `http` or `https` URL, or it has user info;
   - `BadRequest`: a header's name is not a token, or it names `Host`, `Content-Length`, `Transfer-Encoding` or `Connection`, which the client sets itself; or the request names a key by `key_name` and also has an `Authorization` header, or the key has a control character other than a tab;
   - `Unreachable`: name resolution or the connection failed;
   - `TimedOut`: the request passed its deadline (§4 rule 2);
   - `TooLarge`: the response passed a limit of §4 rule 1;
   - `Tls`: the TLS handshake or the certificate failed;
   - `Protocol`: the response is not valid HTTP/1.1 as §3 reads it, or its body is not UTF-8.
   - (Edition 0.4 preview, ADR 0116 rule 12.) `Interrupted`, a ninth variant after `Protocol`, in edition `0.4` only: a host stop request came while the operation waited, or before it started, in the body of a `for ever` (`spec/26 §6`). The request may or may not have reached its server.
   - (Edition 0.4, ADR 0117 rule 8.) `RateLimited`, a tenth variant after `Interrupted`, in edition `0.4` only: the request had to wait at a limit of a grant whose queue was full (`spec/27 §9`). It was not sent.

   A header value with a control character other than a tab is no failure: the run aborts with U6004 at the operation, before anything is sent. A header's value is `internal` (§5), so a failure the program could handle would reveal whether it holds one; an abort is outside the noninterference model (`spec/07 §1`).

## 2. Grants

1. **Syntax.** `http:<origin>` or `http:<origin>:<methods>`, in `[run] grants` and in `--grant` (`spec/13 §5`):
   - `<origin>` is `http://` or `https://`, a host, and an optional `:<port>`; an IPv6 host is in brackets, as `http://[::1]:8080`;
   - the host's leftmost label MAY be `*`, which matches exactly one label;
   - `<methods>` is a comma-separated list of `get`, `post`, `put` and `delete`, by default `get`.
2. **Deny by default.** A request is allowed when a grant matches its scheme, its host and its port (the scheme's default when absent), and allows its method. Otherwise it fails with `Denied`, before any connection.
3. **Redirects.** For `301`, `302`, `303`, `307` and `308`, the client follows the `Location` when the method is `get`, or the status is `303` (which it follows as `get`), and a grant allows the target with that method. It follows at most 5 redirects in a row. Otherwise the redirect response is returned.
4. **Checks.** `una check` and `una run` report a `uses` of an `Http` operation that no grant allows as U3012, as for every host capability.
5. **Limits** (edition `0.4`, ADR 0117 rule 8). A grant MAY carry limits on the rate of its requests, on how many are in flight at once, and on how many wait, as limit lines under it in `ita.una` or suffixes of its spec, as in `http:https://api.example.com:get;100/minute;8/once;waiting=1000` (`spec/27 §9`). A request is charged to every grant that allows it, and each redirect to the grants that allow its target; one over a limit waits, and fails with `RateLimited` when the queue it must join is full.

## 3. Requests and responses

1. **HTTP/1.1, directly.** The client speaks HTTP/1.1 to the origin of each request, with no proxy. It MUST ignore `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY`, in any letter case.
2. **Requests.** A request carries the method, the URL's path and query, `Host`, `Content-Length` for a body, `Accept-Encoding: identity`, `Connection: close`, and the program's headers in order. With `key_name: Some(n)` it also carries `Authorization: Bearer <key>`, with the key named `n` of the `keys:` grants (`spec/13 §5`), to the origin of the request only. A redirect to another origin carries neither that header nor the program's `Authorization`, `Cookie` and `Proxy-Authorization` headers, in any letter case; its other headers follow. Once a redirect has left the request's origin, those credentials stay dropped for the rest of the chain, even at a later redirect back to that origin.
3. **Responses.**
   - The client reads the status line and the headers.
   - It reads the body by `Content-Length`, by `chunked` transfer coding (trailers are read and dropped), or to the end of the connection when neither is present. A chunk size is hex digits alone, with no whitespace around them, else `Protocol`.
   - A `204` or `304` response has no body, whatever its headers say.
   - At most 10 `1xx` responses MAY precede the final one; one more, or a `101`, is `Protocol`.
   - Header names are kept as received, in order.
4. **TLS** verifies the server's certificate chain against the Mozilla root store that the implementation compiles in, and the host name. There is no option to skip verification.
   - **Settings up front** (ADR 0116 amendment, P8, #1148). In edition `0.4`, a run whose grants include `Http` builds its TLS client settings, the root store included, before `main` starts, in both engines, and a run without such a grant never builds them. Built at the first `https` request instead, they would be state left by earlier code: a request after earlier `secret` code that made one would save the time of building them, which a deadline could turn into a value.

## 4. Limits and time

1. **Size.** A response's headers are at most 100, and at most 64 KiB together, else `Protocol`. Its body is at most `--max-response <MiB>` (default 16), else `TooLarge`, and the client reads no further.
2. **Time.** Each request has one deadline, `--http-timeout <seconds>` (default 30, at most 86400, a day): the total wall-clock time of the request, covering name resolution, connecting, the TLS handshake, writing and reading, across its redirects, and in edition `0.4` its waits at the limits of grants (`spec/27 §9`). Past it the request fails with `TimedOut`. Name resolution and the TCP connection run on two bounded pools of threads, since the host's resolver has no timeout of its own, nothing in the process can cut a lookup, and a connection attempt cannot be woken without code of its own (#1148): the request hands each to its pool and waits only until its deadline, an instant, or until its task's cancellation abandons it, and then fails with `TimedOut` or `Interrupted` whatever the step does; the step in progress runs to its end on the pool's thread, which closes the connection, if one came, without using it.
   - **The lookup pool** has `max(2, --io-threads / 4)` threads (64 helpers give 16): a lookup normally takes milliseconds against an exchange's tens to hundreds, so a quarter of the helpers serve their lookups without a queue, and lookups that hang hold at most that many threads. An address literal is not looked up and skips this pool.
   - **The connect pool** has `--io-threads` threads, one per helper, so connecting is never capped below what the helpers could do themselves. A connection job looks at its request only between two addresses: an attempt in progress runs until it connects or the request's deadline passes.
   - **Up front.** Every thread of both pools starts before `main` in a run whose grants include any `Http` grant, in every edition and in both engines, and none in another run (ADR 0116 amendment, P8); their stacks, 256 KiB each, are reserved and committed only as they are used. A thread that cannot start stops the run before `main` with U6003, and a request made without the pools fails with `Unreachable`. With the helpers, a run's `Http` requests take at most `2 × --io-threads + max(2, --io-threads / 4)` threads, and the run as a whole that number, `--workers` and `--file-threads`, all started before `main`. The pools are not capped below that, so a large `--io-threads` can pass the system's limit on a process's threads: at 4096, about 9,200 threads with the defaults, beyond macOS's usual limit (`kern.num_taskthreads`, often 8192) and many containers' limit on processes and threads. The run then stops before `main` with U6003, exit code 1, no position and the message `the lookup and connect threads of this run's requests could not be made`, or that of `spec/27 §1` rule 4 when the helpers are the ones refused; a smaller `--io-threads` runs.
   - **The hand-off.** Each step costs one push and one pop under its pool's mutex, the wake of one pool thread and a reply on the request's own channel: O(1), about two thread wakes, against the milliseconds the step takes.
   - **Hangs.** When every thread of a pool holds a step that hangs, new steps queue, and each of their requests still fails at its own deadline; a step still queued when its request gives up never runs. A lookup or a connection left running after its request gave up, its task cancelled or its deadline passed, is residue that the timing label `L_res` covers (`spec/26 §7` rule 5), at most the pools' sizes. The reactor of ADR 0117 step 6 replaces both pools.
   - **Out of model** (ADR 0116 amendment, P12). The host's resolver may keep state of its own across lookups: glibc's name service switch loads its modules and reads its configuration at the process's first lookup, which can cost a few milliseconds once, and its caches, like the host's, keep answers of earlier lookups. Which lookups earlier code made can so change how long a later one takes. That state is the host's, as the processor's caches are, and stays out of model; a lookup's time reaches a value only through a deadline, whose label covers the request's operation.
3. **Complexity.** A request takes time and space linear in the URL, headers and body it sends. A response takes time and space linear in what it reads, which rule 1 bounds.
4. **Tasks.** A request blocks only its task's thread (`spec/19 §8`). In native code, from edition `0.4`, it parks its task while it runs on a helper thread (`spec/27 §2`), where it also waits at the limits of its grants (`spec/27 §9`).

## 5. Information flow

1. Every argument of every operation is a sink with `accepts internal` (`spec/07 §7` rule 4): a `secret` URL, header, body or key name is U4003.
2. Every response is `internal`.
3. **Failures** (S-15, ADR 0085). Whether an operation fails, and how, is labeled by every argument: the URL, the key name, the headers with their values, and the body (`spec/07 §7` rule 4). The server receives them all and may decide the failure by them, as `TooLarge`, `Protocol` and `TimedOut` are. Since `Header` declares its value `internal`, the failure of every request is `internal`, as its response is. So `attempt` gives an `internal` `Result`, and `if error pass` and `if error fail` are U4002.
4. A key sent by `key_name` never becomes a value of the program, so it has no label in it. It appears in no trace, outcome or diagnostic.

## 6. Traces

An operation of `Http` is a trace event of `spec/11 §5`, with `instance` `host:Http` and `op` the method's name. Its arguments are written and redacted as any operation's are, and so is its outcome, by the label of the binder of its result.

## 7. Examples and determinism

1. Examples cannot use `Http` (U3014). A program's tests install handlers for it (`spec/05 §5`).
2. The network is outside the model: two runs MAY get different responses. Grants, redirects, the limits of §4 rule 1 and the requests the client writes are deterministic. Whether a request waits at, or is refused by, a limit of a grant (`spec/27 §9`) depends on the timing of the run's requests, which is outside the model; `RateLimited` carries the label of that timing.
3. Both engines use one client, and give the same requests, responses and failures for the same server (`spec/16 §5`).
